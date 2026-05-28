//! Strong Alteration Verification Module
//! Combines local pdfium-render + perceptual hashing for maximum confidence.

use crate::engine::balance::{process_and_reconcile, BalanceError};
use crate::engine::model::Transaction;
use image::RgbaImage;
use image_hasher::{HashAlg, HasherConfig};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub math_valid: bool,
    pub visual_diff_score: f64,
    pub only_intended_changes: bool,
    pub report_files: Vec<String>,
    pub message: String,
}

#[derive(Error, Debug)]
pub enum VerificationError {
    #[error("Failed to load PDF: {0}")]
    PdfiumLoad(String),
    #[error("Failed to render page: {0}")]
    PdfiumRender(String),
    #[error("Page count mismatch: original {original}, edited {edited}")]
    PageCountMismatch { original: usize, edited: usize },
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Image encoding error: {0}")]
    ImageEncode(String),
    #[error("Hashing error: {0}")]
    Hash(String),
    #[error("Balance error: {0}")]
    Balance(#[from] BalanceError),
}

pub struct MathInputs {
    pub transactions: Vec<Transaction>,
    pub opening_balance: f64,
    pub expected_final_balance: Option<f64>,
}

const VISUAL_DIFF_THRESHOLD: f64 = 0.02;

pub async fn verify_edit(
    original: &Path,
    edited: &Path,
    output_dir: &Path,
    intended_bboxes: &[(usize, [f32; 4])],
    math_inputs: MathInputs,
    use_pdfrest: bool,
    pdfrest_key: Option<String>,
) -> Result<VerificationReport, VerificationError> {
    verify_edit_pages(
        original,
        edited,
        output_dir,
        intended_bboxes,
        math_inputs,
        use_pdfrest,
        pdfrest_key,
        None,
    )
    .await
}

/// Same as [`verify_edit`] but with the option to restrict the visual diff
/// to a specific set of pages (Stage 2 / Item #2). Useful for the workflow's
/// visual-validation loop, which knows from [`crate::engine::workflow::BalancePreview::changed_pages`]
/// which pages were actually edited and can avoid re-rendering the rest.
///
/// `only_pages = None` is identical to `verify_edit`.
pub async fn verify_edit_pages(
    original: &Path,
    edited: &Path,
    output_dir: &Path,
    intended_bboxes: &[(usize, [f32; 4])],
    math_inputs: MathInputs,
    use_pdfrest: bool,
    pdfrest_key: Option<String>,
    only_pages: Option<&[usize]>,
) -> Result<VerificationReport, VerificationError> {
    std::fs::create_dir_all(output_dir)?;

    let mut pdfrest_warning: Option<String> = None;
    let mut pdfrest_images: Option<(Vec<std::path::PathBuf>, Vec<std::path::PathBuf>)> = None;

    if use_pdfrest {
        if let Some(key) = pdfrest_key {
            let client = crate::ai::pdfrest::PdfRestClient::new(key);
            let orig_out = output_dir.join("pdfrest/original");
            let edit_out = output_dir.join("pdfrest/edited");

            let res = tokio::join!(
                client.render_pdf_to_images(original, &orig_out, 300),
                client.render_pdf_to_images(edited, &edit_out, 300)
            );

            match res {
                (Ok(orig_paths), Ok(edit_paths)) => {
                    pdfrest_images = Some((orig_paths, edit_paths));
                }
                (Err(e), _) | (_, Err(e)) => {
                    let label = match e {
                        crate::ai::pdfrest::PdfRestError::Auth => "Auth Failure",
                        crate::ai::pdfrest::PdfRestError::Timeout { .. } => "Timeout",
                        _ => "API Error",
                    };
                    pdfrest_warning = Some(format!(
                        "⚠️ pdfRest unavailable ({}); using local rendering.",
                        label
                    ));
                }
            }
        } else {
            pdfrest_warning = Some(
                "⚠️ pdfRest requested but PDFREST_API_KEY missing; using local rendering.".into(),
            );
        }
    }

    let pdfium = Pdfium::default();
    let original_doc = pdfium
        .load_pdf_from_file(original, None)
        .map_err(|e| VerificationError::PdfiumLoad(e.to_string()))?;
    let edited_doc = pdfium
        .load_pdf_from_file(edited, None)
        .map_err(|e| VerificationError::PdfiumLoad(e.to_string()))?;

    let original_len = original_doc.pages().len() as usize;
    let edited_len = edited_doc.pages().len() as usize;

    if original_len != edited_len {
        return Err(VerificationError::PageCountMismatch {
            original: original_len,
            edited: edited_len,
        });
    }

    let mut report_files = Vec::new();
    let mut max_visual_score: f64 = 0.0;

    let hasher = HasherConfig::new()
        .hash_size(16, 16)
        .hash_alg(HashAlg::DoubleGradient)
        .to_hasher();

    for i in 0..original_len {
        // Stage 2 / Item #2: skip pages the caller hasn't asked us to check.
        // This makes the visual-validation loop cheap on multi-page edits.
        if let Some(pages) = only_pages {
            if !pages.contains(&i) {
                continue;
            }
        }
        let page_idx = i as u16;

        let (mut original_img, mut edited_img) =
            if let Some((orig_paths, edit_paths)) = &pdfrest_images {
                if i < orig_paths.len() && i < edit_paths.len() {
                    let o = image::open(&orig_paths[i]).map(|img| img.to_rgba8());
                    let e = image::open(&edit_paths[i]).map(|img| img.to_rgba8());

                    match (o, e) {
                        (Ok(o_img), Ok(e_img)) => {
                            report_files.push(orig_paths[i].to_string_lossy().to_string());
                            report_files.push(edit_paths[i].to_string_lossy().to_string());
                            (o_img, e_img)
                        }
                        _ => {
                            // Fallback to pdfium
                            let original_page = original_doc
                                .pages()
                                .get(page_idx)
                                .map_err(|e| VerificationError::PdfiumRender(e.to_string()))?;
                            let edited_page = edited_doc
                                .pages()
                                .get(page_idx)
                                .map_err(|e| VerificationError::PdfiumRender(e.to_string()))?;

                            let target_width = (original_page.width().value * 300.0 / 72.0) as i32;
                            let render_config = PdfRenderConfig::new()
                                .set_target_width(target_width)
                                .set_clear_color(PdfColor::WHITE);

                            let o_img = original_page
                                .render_with_config(&render_config)
                                .map_err(|e| VerificationError::PdfiumRender(e.to_string()))?
                                .as_image()
                                .to_rgba8();
                            let e_img = edited_page
                                .render_with_config(&render_config)
                                .map_err(|e| VerificationError::PdfiumRender(e.to_string()))?
                                .as_image()
                                .to_rgba8();
                            (o_img, e_img)
                        }
                    }
                } else {
                    (image::RgbaImage::new(1, 1), image::RgbaImage::new(1, 1)) // Should not happen
                }
            } else {
                let original_page = original_doc
                    .pages()
                    .get(page_idx)
                    .map_err(|e| VerificationError::PdfiumRender(e.to_string()))?;
                let edited_page = edited_doc
                    .pages()
                    .get(page_idx)
                    .map_err(|e| VerificationError::PdfiumRender(e.to_string()))?;

                let width_pts = original_page.width().value;
                let target_width = (width_pts * 300.0 / 72.0) as i32;

                let render_config = PdfRenderConfig::new()
                    .set_target_width(target_width)
                    .set_clear_color(PdfColor::WHITE);

                let o_img = original_page
                    .render_with_config(&render_config)
                    .map_err(|e| VerificationError::PdfiumRender(e.to_string()))?
                    .as_image()
                    .to_rgba8();

                let e_img = edited_page
                    .render_with_config(&render_config)
                    .map_err(|e| VerificationError::PdfiumRender(e.to_string()))?
                    .as_image()
                    .to_rgba8();

                let orig_png_path = output_dir.join(format!("original_p{}_300dpi.png", i + 1));
                let edit_png_path = output_dir.join(format!("edited_p{}_300dpi.png", i + 1));

                o_img
                    .save(&orig_png_path)
                    .map_err(|e| VerificationError::ImageEncode(e.to_string()))?;
                e_img
                    .save(&edit_png_path)
                    .map_err(|e| VerificationError::ImageEncode(e.to_string()))?;

                report_files.push(orig_png_path.to_string_lossy().to_string());
                report_files.push(edit_png_path.to_string_lossy().to_string());
                (o_img, e_img)
            };

        // Apply masks
        let scale = 300.0 / 72.0;
        let img_w = original_img.width() as f32;
        let img_h = original_img.height() as f32;

        for (page, bbox) in intended_bboxes {
            if *page == i {
                let x0 = (bbox[0] * scale).max(0.0).min(img_w) as u32;
                let y0 = (bbox[1] * scale).max(0.0).min(img_h) as u32;
                let x1 = (bbox[2] * scale).max(0.0).min(img_w) as u32;
                let y1 = (bbox[3] * scale).max(0.0).min(img_h) as u32;

                for y in y0..y1 {
                    for x in x0..x1 {
                        original_img.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
                        edited_img.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
                    }
                }
            }
        }

        // Perceptual hash diff
        let hash1 = hasher.hash_image(&original_img);
        let hash2 = hasher.hash_image(&edited_img);
        let dist = hash1.dist(&hash2);
        let normalised_hamming = dist as f64 / 256.0; // 16*16 bits

        // Per-pixel diff
        let mut total_diff: u64 = 0;
        let mut diff_img = RgbaImage::new(original_img.width(), original_img.height());
        for (x, y, p1) in original_img.enumerate_pixels() {
            let p2 = edited_img.get_pixel(x, y);
            let r_diff = (p1[0] as i16 - p2[0] as i16).abs() as u8;
            let g_diff = (p1[1] as i16 - p2[1] as i16).abs() as u8;
            let b_diff = (p1[2] as i16 - p2[2] as i16).abs() as u8;
            total_diff += (r_diff as u64) + (g_diff as u64) + (b_diff as u64);
            diff_img.put_pixel(x, y, image::Rgba([r_diff, g_diff, b_diff, 255]));
        }
        let pixel_count = original_img.width() as u64 * original_img.height() as u64;
        let normalised_pixel_diff = total_diff as f64 / (255.0 * 3.0 * pixel_count as f64);

        let page_score = normalised_hamming.max(normalised_pixel_diff);
        max_visual_score = max_visual_score.max(page_score);

        let diff_png_path = output_dir.join(format!("visual_diff_p{}_300dpi.png", i + 1));
        diff_img
            .save(&diff_png_path)
            .map_err(|e| VerificationError::ImageEncode(e.to_string()))?;
        report_files.push(diff_png_path.to_string_lossy().to_string());
    }

    let only_intended_changes = max_visual_score < VISUAL_DIFF_THRESHOLD;

    // 5. Math validity
    let (math_valid, math_message) = match process_and_reconcile(
        math_inputs.transactions,
        math_inputs.opening_balance,
        math_inputs.expected_final_balance,
    ) {
        Ok((_, None)) => (true, "✅ Mathematical integrity verified.".to_string()),
        Ok((_, Some(msg))) => (false, format!("⚠️ Mathematical mismatch: {}", msg)),
        Err(e) => (false, format!("❌ Balance Engine error: {}", e)),
    };

    let mut final_message = format!(
        "Verification Result:\nMath: {}\nVisual: {:.4} (Threshold: {})\nOnly Intended: {}",
        if math_valid { "✅" } else { "❌" },
        max_visual_score,
        VISUAL_DIFF_THRESHOLD,
        if only_intended_changes { "✅" } else { "❌" }
    );
    final_message.push_str(&format!("\n{}", math_message));

    if let Some(warn) = pdfrest_warning {
        final_message.push_str(&format!("\n{}", warn));
    }

    Ok(VerificationReport {
        math_valid,
        visual_diff_score: max_visual_score,
        only_intended_changes,
        report_files,
        message: final_message,
    })
}

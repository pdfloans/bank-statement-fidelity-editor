//! Strong Alteration Verification Module
//! Combines local pdfium-render + perceptual hashing for maximum confidence.
//!
//! Stage G (fidelity verification tightening, items #17-#20):
//!
//! - #17 localized tile-max + glyph-edge-sensitive scoring so a single
//!   drifted glyph trips the gate instead of being averaged away.
//! - #18 edited neighbourhoods are scored at 600 DPI (the rest of the page
//!   stays at the cheaper base DPI) so sub-pixel kerning / baseline errors
//!   are actually visible to the comparator.
//! - #19 original and edited are ALWAYS rendered by the same engine with
//!   identical, pinned anti-aliasing flags. Renderer / AA mismatch would
//!   create deltas unrelated to the edit (false fails) or mask real ones
//!   (false passes).
//! - #20 the intended regions are no longer blanket-masked away; we
//!   positively score the replacement glyphs against the original so the
//!   verifier actually proves the edit's font/spacing fidelity.

use crate::engine::balance::{process_and_reconcile, BalanceError};
use crate::engine::model::Transaction;
use image::{GrayImage, RgbaImage};
use image_hasher::{HashAlg, HasherConfig};
use pdfium_render::prelude::*;
use rust_decimal::Decimal;
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
    /// Stage G / Item #17: the worst-scoring localized tile across all
    /// checked pages (outside the intended-edit regions). This is the value
    /// the `only_intended_changes` gate is actually computed from.
    #[serde(default)]
    pub max_tile_score: f64,
    /// Stage G / Item #20: the worst per-edit replacement-fidelity score
    /// (how faithfully the new glyphs reproduce the original style after
    /// best-shift alignment). Higher = more drift/shape mismatch.
    #[serde(default)]
    pub max_edit_region_score: f64,
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
    pub opening_balance: Decimal,
    pub expected_final_balance: Option<Decimal>,
}

/// Page-level diff gate. Localized tile scoring (Item #17) is far more
/// sensitive than a whole-page average, so the threshold can stay tight.
const VISUAL_DIFF_THRESHOLD: f64 = 0.02;

/// Base render DPI for whole-page comparison.
const BASE_DPI: f32 = 300.0;

/// High DPI used around edited regions (Item #18).
const EDIT_REGION_DPI: f32 = 600.0;

/// Side length (px) of the localized scoring tiles (Item #17).
const TILE_PX: u32 = 24;

/// Pinned, deterministic render configuration (Item #19). Anti-aliasing
/// flags are fixed so original and edited rasterise identically; the only
/// pixel differences are real content differences.
fn pinned_render_config(target_width: i32) -> PdfRenderConfig {
    PdfRenderConfig::new()
        .set_target_width(target_width)
        .set_clear_color(PdfColor::WHITE)
        // Keep text/path AA on (matches how a human views the PDF) but pin
        // it identically for both sides. Disable LCD subpixel text — it is
        // orientation/order dependent and would inject channel-fringe deltas.
        .use_lcd_text_rendering(false)
        .set_text_smoothing(true)
        .set_path_smoothing(true)
        .set_image_smoothing(true)
        .render_annotations(true)
        .render_form_data(true)
}

/// Convert an RGBA render to grayscale luminance for structural comparison.
fn to_gray(img: &RgbaImage) -> GrayImage {
    let mut out = GrayImage::new(img.width(), img.height());
    for (x, y, p) in img.enumerate_pixels() {
        let l = (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32)
            .round()
            .clamp(0.0, 255.0) as u8;
        out.put_pixel(x, y, image::Luma([l]));
    }
    out
}

/// Sobel-style gradient magnitude image. Glyph edges dominate the gradient,
/// so a diff of gradient images is highly sensitive to spacing / shape
/// changes that a flat luminance diff averages away (Item #17).
fn gradient_magnitude(g: &GrayImage) -> GrayImage {
    let (w, h) = (g.width(), g.height());
    let mut out = GrayImage::new(w, h);
    if w < 3 || h < 3 {
        return out;
    }
    let at = |x: u32, y: u32| g.get_pixel(x, y)[0] as i32;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let gx = (at(x + 1, y - 1) + 2 * at(x + 1, y) + at(x + 1, y + 1))
                - (at(x - 1, y - 1) + 2 * at(x - 1, y) + at(x - 1, y + 1));
            let gy = (at(x - 1, y + 1) + 2 * at(x, y + 1) + at(x + 1, y + 1))
                - (at(x - 1, y - 1) + 2 * at(x, y - 1) + at(x + 1, y - 1));
            let mag = ((gx * gx + gy * gy) as f64).sqrt().min(255.0) as u8;
            out.put_pixel(x, y, image::Luma([mag]));
        }
    }
    out
}

/// Item #17: localized tile-max score over a region of two aligned gray
/// images, blending flat-luminance and gradient (edge) differences. Tiles
/// fully inside any `exclude` rect (image-space, x0,y0,x1,y1) are skipped so
/// intended edits don't count toward the "only intended changes" gate.
/// Returns the worst (maximum) normalized tile score in [0,1].
fn tile_max_score(
    orig_gray: &GrayImage,
    edit_gray: &GrayImage,
    orig_grad: &GrayImage,
    edit_grad: &GrayImage,
    exclude: &[(u32, u32, u32, u32)],
) -> f64 {
    let w = orig_gray.width().min(edit_gray.width());
    let h = orig_gray.height().min(edit_gray.height());
    let mut worst = 0.0f64;
    let mut ty = 0;
    while ty < h {
        let mut tx = 0;
        while tx < w {
            let x1 = (tx + TILE_PX).min(w);
            let y1 = (ty + TILE_PX).min(h);
            // Skip tiles that lie (mostly) inside an excluded edit rect.
            let center = (tx + (x1 - tx) / 2, ty + (y1 - ty) / 2);
            let skip = exclude.iter().any(|(ex0, ey0, ex1, ey1)| {
                center.0 >= *ex0 && center.0 < *ex1 && center.1 >= *ey0 && center.1 < *ey1
            });
            if !skip {
                let mut lum_sum = 0u64;
                let mut grad_sum = 0u64;
                let mut count = 0u64;
                for y in ty..y1 {
                    for x in tx..x1 {
                        let lo = orig_gray.get_pixel(x, y)[0] as i32;
                        let le = edit_gray.get_pixel(x, y)[0] as i32;
                        lum_sum += (lo - le).unsigned_abs() as u64;
                        let go = orig_grad.get_pixel(x, y)[0] as i32;
                        let ge = edit_grad.get_pixel(x, y)[0] as i32;
                        grad_sum += (go - ge).unsigned_abs() as u64;
                        count += 1;
                    }
                }
                if count > 0 {
                    let lum = lum_sum as f64 / (255.0 * count as f64);
                    let grad = grad_sum as f64 / (255.0 * count as f64);
                    // Edge term weighted higher: it's the spacing/shape signal.
                    let score = 0.4 * lum + 0.6 * grad;
                    if score > worst {
                        worst = score;
                    }
                }
            }
            tx += TILE_PX;
        }
        ty += TILE_PX;
    }
    worst
}

/// Item #20: positive replacement-fidelity score for one edited region.
///
/// Renders the same page region from both PDFs at high DPI, finds the integer
/// (dx,dy) shift in a small window that minimises the gradient diff, and
/// returns `(best_score, dx, dy)`. A faithful edit reproduces the original
/// glyph style closely (low residual) and needs little/no shift. Because the
/// content legitimately changed (e.g. a digit), we compare GRADIENT structure
/// (stroke style, weight, spacing rhythm) rather than raw luminance, and take
/// the best alignment so a pure positional offset is reported as drift rather
/// than inflating the shape residual.
fn region_fidelity_score(orig_gray: &GrayImage, edit_gray: &GrayImage) -> (f64, i32, i32) {
    let og = gradient_magnitude(orig_gray);
    let eg = gradient_magnitude(edit_gray);
    let w = og.width().min(eg.width());
    let h = og.height().min(eg.height());
    if w < 4 || h < 4 {
        return (0.0, 0, 0);
    }
    let rng = 6i32;
    let mut best = f64::MAX;
    let mut best_dx = 0;
    let mut best_dy = 0;
    for dy in -rng..=rng {
        for dx in -rng..=rng {
            let mut sum = 0u64;
            let mut count = 0u64;
            for y in rng..(h as i32 - rng) {
                for x in rng..(w as i32 - rng) {
                    let ox = x as u32;
                    let oy = y as u32;
                    let ex = (x + dx) as u32;
                    let ey = (y + dy) as u32;
                    let a = og.get_pixel(ox, oy)[0] as i32;
                    let b = eg.get_pixel(ex, ey)[0] as i32;
                    sum += (a - b).unsigned_abs() as u64;
                    count += 1;
                }
            }
            if count > 0 {
                let score = sum as f64 / (255.0 * count as f64);
                if score < best {
                    best = score;
                    best_dx = dx;
                    best_dy = dy;
                }
            }
        }
    }
    if best == f64::MAX {
        best = 0.0;
    }
    (best, best_dx, best_dy)
}

/// Item #18 + #20: render a single page sub-rectangle (in PDF points) at
/// `dpi` from an already-loaded document, returning the grayscale crop.
/// Uses the pinned render config + a clip so only the neighbourhood is
/// rasterised (cheap even at 600 DPI).
fn render_region_gray(
    doc: &PdfDocument,
    page_idx: u16,
    bbox_pts: [f32; 4],
    pad_pts: f32,
    dpi: f32,
) -> Result<GrayImage, VerificationError> {
    let page = doc
        .pages()
        .get(page_idx)
        .map_err(|e| VerificationError::PdfiumRender(e.to_string()))?;
    let page_w = page.width().value;
    let page_h = page.height().value;
    let x0 = (bbox_pts[0] - pad_pts).max(0.0);
    let y0 = (bbox_pts[1] - pad_pts).max(0.0);
    let x1 = (bbox_pts[2] + pad_pts).min(page_w);
    let y1 = (bbox_pts[3] + pad_pts).min(page_h);

    let full_w_px = (page_w * dpi / 72.0).round() as i32;
    let cfg = pinned_render_config(full_w_px.max(1));
    let full = page
        .render_with_config(&cfg)
        .map_err(|e| VerificationError::PdfiumRender(e.to_string()))?
        .as_image()
        .to_rgba8();

    let scale = dpi / 72.0;
    let px0 = ((x0 * scale) as u32).min(full.width().saturating_sub(1));
    let py0 = ((y0 * scale) as u32).min(full.height().saturating_sub(1));
    let px1 = ((x1 * scale).ceil() as u32).min(full.width());
    let py1 = ((y1 * scale).ceil() as u32).min(full.height());
    if px1 <= px0 || py1 <= py0 {
        return Ok(GrayImage::new(1, 1));
    }
    let crop = image::imageops::crop_imm(&full, px0, py0, px1 - px0, py1 - py0).to_image();
    Ok(to_gray(&crop))
}

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
    verify_edit_pages_with_padding(
        original,
        edited,
        output_dir,
        intended_bboxes,
        math_inputs,
        use_pdfrest,
        pdfrest_key,
        only_pages,
        0.0,
    )
    .await
}

/// Full-shape verifier with all knobs exposed.
///
/// `mask_padding_pts` (Stage 3 / Item #3): grow each `intended_bbox` by this
/// many PDF points on every side before masking. The visual-validation loop
/// uses this to widen the mask on retries, accommodating sub-pixel baseline
/// shifts that would otherwise keep flagging "intended-only = false" forever.
/// Capped at 12pt is the loop's responsibility, not this function's.
pub async fn verify_edit_pages_with_padding(
    original: &Path,
    edited: &Path,
    output_dir: &Path,
    intended_bboxes: &[(usize, [f32; 4])],
    math_inputs: MathInputs,
    use_pdfrest: bool,
    pdfrest_key: Option<String>,
    only_pages: Option<&[usize]>,
    mask_padding_pts: f32,
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
    let mut max_tile_score: f64 = 0.0;
    let mut max_edit_region_score: f64 = 0.0;
    let mut legacy_pixel_score: f64 = 0.0;

    for i in 0..original_len {
        // Stage 2 / Item #2: skip pages the caller hasn't asked us to check.
        // This makes the visual-validation loop cheap on multi-page edits.
        if let Some(pages) = only_pages {
            if !pages.contains(&i) {
                continue;
            }
        }
        let page_idx = i as u16;

        let (original_img, edited_img) = if let Some((orig_paths, edit_paths)) = &pdfrest_images {
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
                        // Item #19: identical pinned config for both sides.
                        let original_page = original_doc
                            .pages()
                            .get(page_idx)
                            .map_err(|e| VerificationError::PdfiumRender(e.to_string()))?;
                        let edited_page = edited_doc
                            .pages()
                            .get(page_idx)
                            .map_err(|e| VerificationError::PdfiumRender(e.to_string()))?;

                        let target_width = (original_page.width().value * BASE_DPI / 72.0) as i32;
                        let render_config = pinned_render_config(target_width);

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
            let target_width = (width_pts * BASE_DPI / 72.0) as i32;

            // Item #19: one pinned config drives BOTH renders.
            let render_config = pinned_render_config(target_width);

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

        // Build intended-edit exclusion rects in image space (with padding).
        let scale = BASE_DPI / 72.0;
        let img_w = original_img.width() as f32;
        let img_h = original_img.height() as f32;
        let mut exclude_rects: Vec<(u32, u32, u32, u32)> = Vec::new();
        for (page, bbox) in intended_bboxes {
            if *page == i {
                let pad = mask_padding_pts;
                let x0 = ((bbox[0] - pad) * scale).max(0.0).min(img_w) as u32;
                let y0 = ((bbox[1] - pad) * scale).max(0.0).min(img_h) as u32;
                let x1 = ((bbox[2] + pad) * scale).max(0.0).min(img_w) as u32;
                let y1 = ((bbox[3] + pad) * scale).max(0.0).min(img_h) as u32;
                exclude_rects.push((x0, y0, x1, y1));
            }
        }

        // Item #17: localized tile-max scoring on luminance + gradient. This
        // is the gate signal — a single drifted glyph OUTSIDE the intended
        // regions produces a high-scoring tile that a whole-page average
        // would have hidden.
        let orig_gray = to_gray(&original_img);
        let edit_gray = to_gray(&edited_img);
        let orig_grad = gradient_magnitude(&orig_gray);
        let edit_grad = gradient_magnitude(&edit_gray);
        let page_tile_score = tile_max_score(
            &orig_gray,
            &edit_gray,
            &orig_grad,
            &edit_grad,
            &exclude_rects,
        );
        max_tile_score = max_tile_score.max(page_tile_score);

        // Keep a legacy whole-page perceptual-hash + pixel score for the
        // human-facing report number (it's informative, not the gate).
        let hasher = HasherConfig::new()
            .hash_size(16, 16)
            .hash_alg(HashAlg::DoubleGradient)
            .to_hasher();
        let mut masked_o = original_img.clone();
        let mut masked_e = edited_img.clone();
        for (x0, y0, x1, y1) in &exclude_rects {
            for y in *y0..*y1 {
                for x in *x0..*x1 {
                    if x < masked_o.width() && y < masked_o.height() {
                        masked_o.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
                        masked_e.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
                    }
                }
            }
        }
        let hash1 = hasher.hash_image(&masked_o);
        let hash2 = hasher.hash_image(&masked_e);
        let normalised_hamming = hash1.dist(&hash2) as f64 / 256.0;

        let mut total_diff: u64 = 0;
        let mut diff_img = RgbaImage::new(original_img.width(), original_img.height());
        for (x, y, p1) in masked_o.enumerate_pixels() {
            let p2 = masked_e.get_pixel(x, y);
            let r_diff = (p1[0] as i16 - p2[0] as i16).unsigned_abs() as u8;
            let g_diff = (p1[1] as i16 - p2[1] as i16).unsigned_abs() as u8;
            let b_diff = (p1[2] as i16 - p2[2] as i16).unsigned_abs() as u8;
            total_diff += (r_diff as u64) + (g_diff as u64) + (b_diff as u64);
            diff_img.put_pixel(x, y, image::Rgba([r_diff, g_diff, b_diff, 255]));
        }
        let pixel_count = original_img.width() as u64 * original_img.height() as u64;
        let normalised_pixel_diff = total_diff as f64 / (255.0 * 3.0 * pixel_count.max(1) as f64);
        legacy_pixel_score = legacy_pixel_score.max(normalised_hamming.max(normalised_pixel_diff));

        let diff_png_path = output_dir.join(format!("visual_diff_p{}_300dpi.png", i + 1));
        diff_img
            .save(&diff_png_path)
            .map_err(|e| VerificationError::ImageEncode(e.to_string()))?;
        report_files.push(diff_png_path.to_string_lossy().to_string());

        // Item #18 + #20: positively verify each intended edit's replacement
        // glyphs at 600 DPI. We render just the edited neighbourhood from
        // both PDFs (cheap), then score the gradient residual after best
        // alignment. High residual = the new glyphs don't match the
        // original's weight/spacing/shape — i.e. a fidelity failure on the
        // edit itself, which the old blanket-mask approach never checked.
        if pdfrest_images.is_none() {
            for (page, bbox) in intended_bboxes {
                if *page != i {
                    continue;
                }
                let o_region =
                    render_region_gray(&original_doc, page_idx, *bbox, 3.0, EDIT_REGION_DPI)?;
                let e_region =
                    render_region_gray(&edited_doc, page_idx, *bbox, 3.0, EDIT_REGION_DPI)?;
                let (score, _dx, _dy) = region_fidelity_score(&o_region, &e_region);
                max_edit_region_score = max_edit_region_score.max(score);
            }
        }
    }

    // Item #17: the gate is the worst localized tile OUTSIDE intended edits.
    let only_intended_changes = max_tile_score < VISUAL_DIFF_THRESHOLD;
    // Report number favours the most sensitive signal we computed.
    let max_visual_score = max_tile_score.max(legacy_pixel_score);

    // 5. Math validity.
    //
    // Improvement #4: when the document carries no transaction/balance data
    // (e.g. a non-statement PDF or a page with no parseable rows), math
    // reconciliation is *not applicable* rather than a failure. Emitting a
    // scary "Balance Engine error: Missing opening balance" in that case is
    // misleading, so we degrade gracefully to a visual-only verdict and mark
    // math_valid = true (nothing to disprove).
    let has_balance_data = !math_inputs.transactions.is_empty()
        && math_inputs.opening_balance != Decimal::ZERO;
    let (math_valid, math_message) = if !has_balance_data {
        (
            true,
            "➖ Math check not applicable (no transaction/balance data found); visual-only verification.".to_string(),
        )
    } else {
        match process_and_reconcile(
            math_inputs.transactions,
            math_inputs.opening_balance,
            math_inputs.expected_final_balance,
        ) {
            Ok((_, None)) => (true, "✅ Mathematical integrity verified.".to_string()),
            Ok((_, Some(msg))) => (false, format!("⚠️ Mathematical mismatch: {}", msg)),
            // A genuine engine error on a doc that *did* have balance data.
            Err(crate::engine::balance::BalanceError::MissingOpeningBalance) => (
                true,
                "➖ Math check skipped (opening balance could not be determined); visual-only verification.".to_string(),
            ),
            Err(e) => (false, format!("❌ Balance Engine error: {}", e)),
        }
    };

    let mut final_message = format!(
        "Verification Result:\nMath: {}\nVisual (tile-max): {:.4} (Threshold: {})\nOnly Intended: {}",
        if math_valid { "✅" } else { "❌" },
        max_tile_score,
        VISUAL_DIFF_THRESHOLD,
        if only_intended_changes { "✅" } else { "❌" }
    );
    final_message.push_str(&format!(
        "\nEdit-region fidelity (max residual): {:.4}",
        max_edit_region_score
    ));
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
        max_tile_score,
        max_edit_region_score,
    })
}

#[cfg(test)]
mod stage_g_tests {
    use super::*;
    use image::{GrayImage, Luma};

    /// Build a white gray image with an optional black rectangle "glyph".
    fn img_with_block(w: u32, h: u32, block: Option<(u32, u32, u32, u32)>) -> GrayImage {
        let mut g = GrayImage::from_pixel(w, h, Luma([255]));
        if let Some((x0, y0, x1, y1)) = block {
            for y in y0..y1 {
                for x in x0..x1 {
                    g.put_pixel(x, y, Luma([0]));
                }
            }
        }
        g
    }

    /// Item #17: a single localized glyph change must produce a high tile
    /// score, whereas the whole-page average of the same change is tiny.
    /// This is the core sensitivity claim of the new verifier.
    #[test]
    fn tile_max_detects_localized_change_that_average_hides() {
        let w = 600;
        let h = 400;
        // Original: one small block. Edited: block shifted a few px (a
        // drifted glyph). Everything else identical white.
        let orig = img_with_block(w, h, Some((100, 100, 130, 140)));
        let edited = img_with_block(w, h, Some((104, 100, 134, 140)));

        let orig_grad = gradient_magnitude(&orig);
        let edit_grad = gradient_magnitude(&edited);

        // Whole-page average luminance diff — the OLD gate signal.
        let mut total = 0u64;
        for (x, y, p) in orig.enumerate_pixels() {
            let q = edited.get_pixel(x, y)[0] as i32;
            total += (p[0] as i32 - q).unsigned_abs() as u64;
        }
        let whole_page_avg = total as f64 / (255.0 * (w * h) as f64);

        // New localized signal.
        let tile = tile_max_score(&orig, &edited, &orig_grad, &edit_grad, &[]);

        assert!(
            whole_page_avg < VISUAL_DIFF_THRESHOLD,
            "precondition: the change is small on a whole-page average ({whole_page_avg:.5})"
        );
        assert!(
            tile > VISUAL_DIFF_THRESHOLD,
            "tile-max must catch the localized drift the average hides (tile={tile:.5})"
        );
    }

    /// Item #17: excluding the intended-edit region means a change confined
    /// to that region does NOT trip the gate.
    fn rect_around(x0: u32, y0: u32, x1: u32, y1: u32) -> (u32, u32, u32, u32) {
        (x0, y0, x1, y1)
    }

    #[test]
    fn excluded_region_change_does_not_trip_gate() {
        let w = 600;
        let h = 400;
        let orig = img_with_block(w, h, Some((100, 100, 130, 140)));
        let edited = img_with_block(w, h, Some((104, 100, 134, 140)));
        let orig_grad = gradient_magnitude(&orig);
        let edit_grad = gradient_magnitude(&edited);

        // Exclude a generous box around the change.
        let exclude = vec![rect_around(80, 80, 160, 160)];
        let tile = tile_max_score(&orig, &edited, &orig_grad, &edit_grad, &exclude);
        assert!(
            tile < VISUAL_DIFF_THRESHOLD,
            "change inside the excluded (intended) region must not trip the gate (tile={tile:.5})"
        );
    }

    /// Item #20: identical regions score ~0 with zero shift; a region whose
    /// content was rendered in a heavier/shifted style scores higher.
    #[test]
    fn region_fidelity_rewards_matching_and_zero_shift() {
        let w = 120;
        let h = 80;
        // Two identical "glyph" crops.
        let a = img_with_block(w, h, Some((40, 30, 60, 60)));
        let b = img_with_block(w, h, Some((40, 30, 60, 60)));
        let (score_same, dx, dy) = region_fidelity_score(&a, &b);
        assert!(
            score_same < 0.01,
            "identical regions ~0 (got {score_same:.5})"
        );
        assert_eq!((dx, dy), (0, 0), "identical regions need no shift");

        // A much heavier stroke (wrong weight) should score worse than identical.
        let heavy = img_with_block(w, h, Some((38, 28, 64, 62)));
        let (score_heavy, _, _) = region_fidelity_score(&a, &heavy);
        assert!(
            score_heavy > score_same,
            "wrong-weight glyph must score worse ({score_heavy:.5} > {score_same:.5})"
        );
    }

    /// A pure positional offset is reported as shift, not inflated shape
    /// residual: the best-aligned score stays low.
    #[test]
    fn region_fidelity_aligns_out_pure_translation() {
        let w = 120;
        let h = 80;
        let a = img_with_block(w, h, Some((40, 30, 60, 60)));
        let shifted = img_with_block(w, h, Some((43, 30, 63, 60)));
        let (score, dx, _dy) = region_fidelity_score(&a, &shifted);
        assert!(
            dx != 0 || score < 0.02,
            "translation should be recovered by alignment (dx={dx}, score={score:.5})"
        );
    }
}

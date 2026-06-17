//! Native PDF Engine — oxidize-pdf AST traversal + pdf-writer serialization.
//!
//! Phase 2 of the architecture rewrite. This replaces all FFI-based PDF
//! engines (MuPDF, PyMuPDF, pdfium-render) with pure Rust implementations.
//!
//! ## Design
//!
//! - **Read path:** Uses `oxidize_pdf::Document` for non-destructive PDF
//!   parsing. Content streams are walked operator-by-operator to extract
//!   text blocks with their positions, fonts, and sizes.
//!
//! - **Write path:** Uses `lopdf` (already in the dep tree) for surgical
//!   content stream edits. `pdf-writer` is used for full-page serialization
//!   when needed.
//!
//! - **Rendering:** Fallback native renderer drawing bounding boxes using `imageproc`.

use crate::engine::layout::{DocumentLayout, PageLayout};
use crate::pdf::engine::*;
use std::path::Path;

/// Pure-Rust PDF engine backed by `oxidize-pdf` + `lopdf`.
#[derive(Debug, Default)]
pub struct OxidizePdfEngine;

impl OxidizePdfEngine {
    pub fn new() -> Self {
        Self
    }

    /// Load a PDF document via lopdf (which is already a dependency) and
    /// count pages.
    fn page_count(&self, path: &Path) -> Result<usize, EngineError> {
        let doc =
            lopdf::Document::load(path).map_err(|e| EngineError::LoadFailed(format!("{e}")))?;
        Ok(doc.get_pages().len())
    }

    /// Extract text blocks from a single page by walking the content stream.
    ///
    /// This parses the raw PDF operators (Tj, TJ, Tm, Tf, Td, TD, T*, etc.)
    /// to reconstruct positioned text spans. Each span becomes a `TextBlock`
    /// with its bounding box estimated from the text matrix and font metrics.
    fn extract_text_blocks_from_page(
        &self,
        path: &Path,
        page_num: usize,
    ) -> Result<Vec<TextBlock>, EngineError> {
        let doc =
            lopdf::Document::load(path).map_err(|e| EngineError::LoadFailed(format!("{e}")))?;

        let pages = doc.get_pages();
        let page_id = pages
            .get(&(page_num as u32 + 1)) // lopdf uses 1-indexed pages
            .ok_or_else(|| {
                EngineError::ExtractFailed(format!(
                    "Page {} not found (document has {} pages)",
                    page_num,
                    pages.len()
                ))
            })?;

        let content = doc
            .get_page_content(*page_id)
            .map_err(|e| EngineError::ExtractFailed(format!("Failed to get page content: {e}")))?;

        let operations = lopdf::content::Content::decode(&content)
            .map_err(|e| {
                EngineError::ExtractFailed(format!("Failed to decode content stream: {e}"))
            })?
            .operations;

        let mut blocks: Vec<TextBlock> = Vec::new();
        let mut current_font = String::from("Unknown");
        let mut font_size: f32 = 12.0;
        // Text matrix tracking: [a, b, c, d, tx, ty]
        let mut tm = [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0];
        // Text line matrix (set by Td/TD/T*)
        let mut tlm = [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0];
        let mut in_text = false;

        for op in &operations {
            match op.operator.as_str() {
                "BT" => {
                    in_text = true;
                    tm = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                    tlm = tm;
                }
                "ET" => {
                    in_text = false;
                }
                "Tf" if in_text => {
                    // Set font: Tf <font-name> <size>
                    if op.operands.len() >= 2 {
                        if let lopdf::Object::Name(ref name) = op.operands[0] {
                            current_font = String::from_utf8_lossy(name).to_string();
                        }
                        font_size = operand_to_f32(&op.operands[1]).unwrap_or(12.0);
                    }
                }
                "Tm" if in_text => {
                    // Set text matrix directly: Tm a b c d tx ty
                    if op.operands.len() >= 6 {
                        for (i, operand) in op.operands.iter().enumerate().take(6) {
                            tm[i] = operand_to_f32(operand).unwrap_or(0.0);
                        }
                        tlm = tm;
                    }
                }
                "Td" | "TD" if in_text => {
                    // Move text position: Td tx ty
                    if op.operands.len() >= 2 {
                        let tx = operand_to_f32(&op.operands[0]).unwrap_or(0.0);
                        let ty = operand_to_f32(&op.operands[1]).unwrap_or(0.0);
                        tlm[4] += tx;
                        tlm[5] += ty;
                        tm = tlm;
                    }
                }
                "T*" if in_text => {
                    // Move to start of next line (uses TL — leading)
                    // Approximate as moving down by font_size
                    tlm[5] -= font_size;
                    tm = tlm;
                }
                "Tj" if in_text => {
                    // Show string: Tj <string>
                    if let Some(text) = extract_string_operand(&op.operands) {
                        if !text.trim().is_empty() {
                            let x = tm[4];
                            let y = tm[5];
                            let estimated_width = text.len() as f32 * font_size * 0.5;
                            blocks.push(TextBlock {
                                page: page_num,
                                text: text.clone(),
                                bbox: [x, y, x + estimated_width, y + font_size],
                                font: current_font.clone(),
                                size: font_size,
                                obj_id: Some(format!("ObjId({}, {})", page_id.0, page_id.1)),
                            });
                        }
                    }
                }
                "TJ" if in_text => {
                    // Show array of strings with kerning adjustments
                    if let Some(lopdf::Object::Array(ref arr)) = op.operands.first() {
                        let mut combined_text = String::new();
                        for item in arr {
                            match item {
                                lopdf::Object::String(bytes, _) => {
                                    combined_text.push_str(&String::from_utf8_lossy(bytes));
                                }
                                lopdf::Object::Integer(_) | lopdf::Object::Real(_) => {
                                    // Kerning adjustment — skip
                                }
                                _ => {}
                            }
                        }
                        if !combined_text.trim().is_empty() {
                            let x = tm[4];
                            let y = tm[5];
                            let estimated_width = combined_text.len() as f32 * font_size * 0.5;
                            blocks.push(TextBlock {
                                page: page_num,
                                text: combined_text,
                                bbox: [x, y, x + estimated_width, y + font_size],
                                font: current_font.clone(),
                                size: font_size,
                                obj_id: Some(format!("ObjId({}, {})", page_id.0, page_id.1)),
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(blocks)
    }
}

/// Helper: extract f32 from a lopdf Object (Integer or Real).
fn operand_to_f32(obj: &lopdf::Object) -> Option<f32> {
    match obj {
        lopdf::Object::Integer(i) => Some(*i as f32),
        lopdf::Object::Real(f) => Some(*f),
        _ => None,
    }
}

/// Helper: extract a String from the first string operand.
fn extract_string_operand(operands: &[lopdf::Object]) -> Option<String> {
    for op in operands {
        if let lopdf::Object::String(bytes, _) = op {
            return Some(String::from_utf8_lossy(bytes).to_string());
        }
    }
    None
}

/// Recommendation #1 — faithful page rasterisation using `pdfium-render`.
///
/// Binds to a local `pdfium` library first (so a shipped binary wins) and
/// falls back to the system library. Renders the requested page at `dpi`
/// using anti-aliasing flags pinned identically to the fidelity verifier
/// (`use_lcd_text_rendering(false)` + smoothing on) so previews match what
/// the verifier scores. Returns PNG bytes plus the page size in points.
fn render_page_with_pdfium(
    path: &Path,
    page: usize,
    dpi: f32,
) -> Result<RenderedPage, EngineError> {
    use pdfium_render::prelude::*;

    let bindings = Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./"))
        .or_else(|_| Pdfium::bind_to_system_library())
        .map_err(|e| EngineError::RenderFailed(format!("Failed to bind pdfium: {e}")))?;
    let pdfium = Pdfium::new(bindings);

    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| EngineError::RenderFailed(format!("Failed to load PDF: {e}")))?;

    let pages = document.pages();
    let page_count = pages.len() as usize;
    if page >= page_count {
        return Err(EngineError::RenderFailed(format!(
            "Page {page} out of range (document has {page_count} pages)"
        )));
    }

    let pdf_page = pages
        .get(page as u16)
        .map_err(|e| EngineError::RenderFailed(format!("Failed to get page {page}: {e}")))?;

    let width_pts = pdf_page.width().value;
    let height_pts = pdf_page.height().value;

    let dpi = if dpi.is_finite() && dpi > 0.0 {
        dpi
    } else {
        150.0
    };
    let target_width = ((width_pts * dpi / 72.0).round() as i32).max(1);

    let config = PdfRenderConfig::new()
        .set_target_width(target_width)
        .set_clear_color(PdfColor::WHITE)
        .use_lcd_text_rendering(false)
        .set_text_smoothing(true)
        .set_path_smoothing(true)
        .set_image_smoothing(true)
        .render_annotations(true)
        .render_form_data(true);

    let image = pdf_page
        .render_with_config(&config)
        .map_err(|e| EngineError::RenderFailed(format!("pdfium render failed: {e}")))?
        .as_image()
        .into_rgba8();

    let mut png_bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )
        .map_err(|e| EngineError::RenderFailed(format!("Failed to encode PNG: {e}")))?;

    Ok(RenderedPage {
        png_bytes,
        width_pts,
        height_pts,
    })
}

impl PdfEngine for OxidizePdfEngine {
    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            supports_redaction: true,
            supports_cjk: false, // Phase 3 — needs skrifa CID font mapping
            supports_embedded_fonts: true,
            estimated_fidelity: 0.85,
        }
    }

    fn render_page(&self, path: &Path, page: usize, dpi: f32) -> Result<RenderedPage, EngineError> {
        // Recommendation #1: faithful, pure-Rust(ish) rasterisation via
        // `pdfium-render` (already a dependency, already used by the fidelity
        // verifier). This makes the native engine the primary preview path so
        // previews no longer depend on the GIL-locked Python actor, while
        // PyMuPDF stays as the automatic fallback in the selector.
        render_page_with_pdfium(path, page, dpi)
    }

    fn get_text_blocks(&self, path: &Path, page: usize) -> Result<Vec<TextBlock>, EngineError> {
        self.extract_text_blocks_from_page(path, page)
    }

    fn find_text_block_at_click(
        &self,
        path: &Path,
        page: usize,
        x: f32,
        y: f32,
    ) -> Result<Option<TextBlock>, EngineError> {
        let blocks = self.get_text_blocks(path, page)?;
        Ok(blocks
            .into_iter()
            .find(|b| x >= b.bbox[0] && x <= b.bbox[2] && y >= b.bbox[1] && y <= b.bbox[3]))
    }

    fn apply_change(
        &self,
        input: &Path,
        output: &Path,
        page: usize,
        bbox: [f32; 4],
        new_text: &str,
        old_text: &str,
        _font_path: Option<&Path>,
    ) -> Result<ReplaceOutcome, EngineError> {
        // Stage 1 Strict Font Guard:
        // The native engine blindly writes into the content stream. If the embedded
        // font subset is missing glyphs, the PDF will show boxes. As a robust heuristic,
        // if the new text contains non-ASCII characters (e.g. symbols, CJK), we refuse
        // to edit so the Selector auto-falls back to PyMuPDF Pro which has font replication.
        if !new_text.is_ascii() {
            return Err(EngineError::FontCoverageMissing(
                "Native engine requires ASCII for safe subset coverage; complex chars detected"
                    .into(),
            ));
        }

        // Load the document
        let mut doc =
            lopdf::Document::load(input).map_err(|e| EngineError::LoadFailed(format!("{e}")))?;

        let pages = doc.get_pages();
        let page_id = *pages.get(&(page as u32 + 1)).ok_or_else(|| {
            EngineError::ApplyFailed(format!(
                "Page {} not found (document has {} pages)",
                page,
                pages.len()
            ))
        })?;

        // Get the page content
        let content_bytes = doc
            .get_page_content(page_id)
            .map_err(|e| EngineError::ApplyFailed(format!("Failed to get page content: {e}")))?;

        let mut content = lopdf::content::Content::decode(&content_bytes)
            .map_err(|e| EngineError::ApplyFailed(format!("Failed to decode content: {e}")))?;

        // Walk the content stream and find the text span that overlaps `bbox`.
        // Replace it with `new_text`.
        let mut tm = [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0];
        let mut tlm = tm;
        let mut font_size: f32 = 12.0;
        let mut in_text = false;
        let mut replaced = false;

        for op in &mut content.operations {
            match op.operator.as_str() {
                "BT" => {
                    in_text = true;
                    tm = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                    tlm = tm;
                }
                "ET" => {
                    in_text = false;
                }
                "Tf" if in_text => {
                    if op.operands.len() >= 2 {
                        font_size = operand_to_f32(&op.operands[1]).unwrap_or(12.0);
                    }
                }
                "Tm" if in_text => {
                    if op.operands.len() >= 6 {
                        for (i, operand) in op.operands.iter().enumerate().take(6) {
                            tm[i] = operand_to_f32(operand).unwrap_or(0.0);
                        }
                        tlm = tm;
                    }
                }
                "Td" | "TD" if in_text => {
                    if op.operands.len() >= 2 {
                        let tx = operand_to_f32(&op.operands[0]).unwrap_or(0.0);
                        let ty = operand_to_f32(&op.operands[1]).unwrap_or(0.0);
                        tlm[4] += tx;
                        tlm[5] += ty;
                        tm = tlm;
                    }
                }
                "T*" if in_text => {
                    tlm[5] -= font_size;
                    tm = tlm;
                }
                "Tj" if in_text && !replaced => {
                    let x = tm[4];
                    let y = tm[5];
                    let mut text_matches = false;
                    let mut found_text = String::new();
                    if let Some(text) = extract_string_operand(&op.operands) {
                        found_text = text.clone();
                        if !text.trim().is_empty() && text.trim() == old_text.trim() {
                            text_matches = true;
                        }
                    }
                    let x_matches = x >= bbox[0] - 5.0 && x <= bbox[2] + 5.0;
                    if x_matches {
                        println!(
                            "[DEBUG Tj] Found text '{found_text}' at x={x}, target='{old_text}'"
                        );
                    }
                    let y_matches = y >= bbox[1] - 1.0 && y <= bbox[3] + 1.0;

                    if text_matches || (x_matches && y_matches) {
                        // Replace the string operand
                        if !op.operands.is_empty() {
                            op.operands[0] = lopdf::Object::String(
                                new_text.as_bytes().to_vec(),
                                lopdf::StringFormat::Literal,
                            );
                            replaced = true;
                        }
                    }
                }
                "TJ" if in_text && !replaced => {
                    let x = tm[4];
                    let y = tm[5];
                    let mut text_matches = false;
                    let mut found_text = String::new();
                    if let Some(lopdf::Object::Array(ref arr)) = op.operands.first() {
                        let mut combined = String::new();
                        for item in arr {
                            if let lopdf::Object::String(bytes, _) = item {
                                combined.push_str(&String::from_utf8_lossy(bytes));
                            }
                        }
                        found_text = combined.clone();
                        if !combined.trim().is_empty() && combined.trim() == old_text.trim() {
                            text_matches = true;
                        }
                    }
                    let x_matches = x >= bbox[0] - 5.0 && x <= bbox[2] + 5.0;
                    if x_matches {
                        println!(
                            "[DEBUG TJ] Found text '{found_text}' at x={x}, target='{old_text}'"
                        );
                    }
                    let y_matches = y >= bbox[1] - 1.0 && y <= bbox[3] + 1.0;

                    if text_matches || (x_matches && y_matches) {
                        // Replace the entire TJ array with a single Tj string
                        op.operator = "Tj".to_string();
                        op.operands = vec![lopdf::Object::String(
                            new_text.as_bytes().to_vec(),
                            lopdf::StringFormat::Literal,
                        )];
                        replaced = true;
                    }
                }
                _ => {}
            }
        }

        if !replaced {
            return Err(EngineError::ApplyFailed(
                "No matching text span found at the specified bbox".into(),
            ));
        }

        // Re-encode the content stream and set it back on the page
        let new_content_bytes = content
            .encode()
            .map_err(|e| EngineError::ApplyFailed(format!("Failed to encode content: {e}")))?;

        doc.change_page_content(page_id, new_content_bytes)
            .map_err(|e| EngineError::ApplyFailed(format!("Failed to update page: {e}")))?;

        doc.save(output)
            .map_err(|e| EngineError::ApplyFailed(format!("Failed to save: {e}")))?;

        Ok(ReplaceOutcome {
            success: true,
            font_used: "original".to_string(),
            overflow: false,
            obj_id: Some(format!("ObjId({}, {})", page_id.0, page_id.1)),
        })
    }

    fn analyze_layout(&self, path: &Path) -> Result<DocumentLayout, EngineError> {
        let page_count = self.page_count(path)?;

        let mut pages = Vec::with_capacity(page_count);
        for i in 0..page_count {
            let blocks = self
                .extract_text_blocks_from_page(path, i)
                .unwrap_or_default();

            // Simple heuristic: check for header/footer by position
            let has_header = blocks.iter().any(|b| b.bbox[1] < 72.0); // top inch
            let has_footer = blocks.iter().any(|b| b.bbox[1] > 720.0); // bottom inch

            let dominant_font = blocks
                .first()
                .map(|b| b.font.clone())
                .unwrap_or_else(|| "Unknown".to_string());

            pages.push(PageLayout {
                page_number: i + 1,
                has_header,
                has_footer,
                has_page_number: false,
                table_columns: 0,
                main_text_style: "normal".to_string(),
                dominant_font,
            });
        }

        let has_consistent_headers = pages.iter().all(|p| p.has_header);
        let has_consistent_footers = pages.iter().all(|p| p.has_footer);

        Ok(DocumentLayout {
            total_pages: page_count,
            pages,
            has_consistent_headers,
            has_consistent_footers,
            overall_style: "standard".to_string(),
            layout_confidence: 0.7,
        })
    }

    fn apply_many_edits(
        &self,
        input: &std::path::Path,
        output: &std::path::Path,
        edits_json: &str,
        _font_path: Option<&std::path::Path>,
    ) -> Result<usize, EngineError> {
        let edits: Vec<serde_json::Value> = serde_json::from_str(edits_json)
            .map_err(|e| EngineError::ApplyFailed(format!("Invalid edits JSON: {e}")))?;

        // Stage 1 Strict Font Guard for batch edits
        for edit in &edits {
            if let Some(new_text) = edit["new_text"].as_str() {
                if !new_text.is_ascii() {
                    return Err(EngineError::FontCoverageMissing(
                        "Native engine requires ASCII for safe subset coverage; complex chars detected".into()
                    ));
                }
            }
        }

        let mut doc =
            lopdf::Document::load(input).map_err(|e| EngineError::LoadFailed(format!("{e}")))?;

        let mut applied_count = 0;
        let mut modified_pages = std::collections::HashSet::new();

        let mut edits_by_page: std::collections::HashMap<usize, Vec<&serde_json::Value>> =
            std::collections::HashMap::new();
        for edit in &edits {
            if let Some(page) = edit["page"].as_u64() {
                edits_by_page.entry(page as usize).or_default().push(edit);
            }
        }

        let pages = doc.get_pages();

        for (page_idx, page_edits) in edits_by_page {
            let page_id = *pages
                .get(&(page_idx as u32 + 1))
                .ok_or_else(|| EngineError::ApplyFailed(format!("Page {page_idx} not found")))?;

            let content_bytes = doc.get_page_content(page_id).unwrap_or_default();
            if content_bytes.is_empty() {
                continue;
            }

            let mut content = match lopdf::content::Content::decode(&content_bytes) {
                Ok(c) => c,
                Err(e) => {
                    return Err(EngineError::ApplyFailed(format!(
                        "Failed to decode content: {e}"
                    )))
                }
            };

            let mut tm = [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0];
            let mut tlm = tm;
            let mut font_size: f32 = 12.0;
            let mut in_text = false;

            for op in &mut content.operations {
                match op.operator.as_str() {
                    "BT" => {
                        in_text = true;
                        tm = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                        tlm = tm;
                    }
                    "ET" => {
                        in_text = false;
                    }
                    "Tf" if in_text => {
                        if op.operands.len() >= 2 {
                            font_size = operand_to_f32(&op.operands[1]).unwrap_or(12.0);
                        }
                    }
                    "Tm" if in_text => {
                        if op.operands.len() >= 6 {
                            for (i, operand) in op.operands.iter().enumerate().take(6) {
                                tm[i] = operand_to_f32(operand).unwrap_or(0.0);
                            }
                            tlm = tm;
                        }
                    }
                    "Td" | "TD" if in_text => {
                        if op.operands.len() >= 2 {
                            let tx = operand_to_f32(&op.operands[0]).unwrap_or(0.0);
                            let ty = operand_to_f32(&op.operands[1]).unwrap_or(0.0);
                            tlm[4] += tx;
                            tlm[5] += ty;
                            tm = tlm;
                        }
                    }
                    "T*" if in_text => {
                        tlm[5] -= font_size;
                        tm = tlm;
                    }
                    "Tj" | "TJ" if in_text => {
                        let x = tm[4];
                        let y = tm[5];
                        for edit in &page_edits {
                            if let Some(rect) = edit["rect"].as_array() {
                                if rect.len() == 4 {
                                    let bbox = [
                                        rect[0].as_f64().unwrap_or(0.0) as f32,
                                        rect[1].as_f64().unwrap_or(0.0) as f32,
                                        rect[2].as_f64().unwrap_or(0.0) as f32,
                                        rect[3].as_f64().unwrap_or(0.0) as f32,
                                    ];
                                    if x >= bbox[0] - 1.0
                                        && y >= bbox[1] - 1.0
                                        && x <= bbox[2] + 1.0
                                        && y <= bbox[3] + 1.0
                                    {
                                        if let Some(new_text) = edit["new_text"].as_str() {
                                            op.operator = "Tj".to_string();
                                            op.operands = vec![lopdf::Object::String(
                                                new_text.as_bytes().to_vec(),
                                                lopdf::StringFormat::Literal,
                                            )];
                                            applied_count += 1;
                                            modified_pages.insert(page_id);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }

            if modified_pages.contains(&page_id) {
                let new_content_bytes = content.encode().map_err(|e| {
                    EngineError::ApplyFailed(format!("Failed to encode content: {e}"))
                })?;

                doc.change_page_content(page_id, new_content_bytes)
                    .map_err(|e| EngineError::ApplyFailed(format!("Failed to update page: {e}")))?;
            }
        }

        doc.save(output)
            .map_err(|e| EngineError::ApplyFailed(format!("Failed to save: {e}")))?;

        Ok(applied_count)
    }

    fn clone_pages(
        &self,
        input: &std::path::Path,
        output: &std::path::Path,
        page_indices: Vec<usize>,
    ) -> Result<usize, EngineError> {
        let mut doc =
            lopdf::Document::load(input).map_err(|e| EngineError::LoadFailed(format!("{e}")))?;

        let pages = doc.get_pages();
        let mut cloned = 0;

        for &idx in &page_indices {
            if let Some(&page_id) = pages.get(&(idx as u32 + 1)) {
                if let Ok(page_dict) = doc.get_object(page_id) {
                    let page_dict_clone = page_dict.clone();
                    let new_page_id = doc.add_object(page_dict_clone);

                    // Manually append the new page to the Pages tree
                    if let Ok(catalog) = doc.catalog() {
                        if let Ok(pages_ref) = catalog.get(b"Pages") {
                            if let Ok(pages_id) = pages_ref.as_reference() {
                                if let Ok(pages_dict) = doc.get_dictionary_mut(pages_id) {
                                    if let Ok(kids) = pages_dict.get_mut(b"Kids") {
                                        if let Ok(kids_array) = kids.as_array_mut() {
                                            kids_array.push(lopdf::Object::Reference(new_page_id));

                                            // Update Count
                                            if let Ok(count_obj) = pages_dict.get_mut(b"Count") {
                                                if let Ok(count) = count_obj.as_i64() {
                                                    *count_obj = lopdf::Object::Integer(count + 1);
                                                    cloned += 1;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        doc.save(output)
            .map_err(|e| EngineError::ApplyFailed(format!("Failed to save: {e}")))?;
        Ok(cloned)
    }

    fn remove_pages(
        &self,
        input: &std::path::Path,
        output: &std::path::Path,
        page_indices: Vec<usize>,
    ) -> Result<usize, EngineError> {
        let mut doc =
            lopdf::Document::load(input).map_err(|e| EngineError::LoadFailed(format!("{e}")))?;

        let mut page_nums = Vec::new();
        for &idx in &page_indices {
            page_nums.push(idx as u32 + 1);
        }

        doc.delete_pages(&page_nums);
        doc.save(output)
            .map_err(|e| EngineError::ApplyFailed(format!("Failed to save: {e}")))?;
        Ok(page_nums.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capabilities() {
        let engine = OxidizePdfEngine::new();
        let caps = engine.capabilities();
        assert!(caps.supports_redaction);
        assert!(caps.supports_embedded_fonts);
        assert!(!caps.supports_cjk); // Not yet
    }

    #[test]
    fn operand_to_f32_converts_correctly() {
        assert_eq!(operand_to_f32(&lopdf::Object::Integer(42)), Some(42.0));
        assert_eq!(operand_to_f32(&lopdf::Object::Real(2.5)), Some(2.5));
        assert_eq!(operand_to_f32(&lopdf::Object::Boolean(true)), None);
    }
}

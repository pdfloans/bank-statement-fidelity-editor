use std::path::Path;
use std::sync::Arc;
use crate::pdf::PdfEngine;
use super::geometry::*;

pub struct TesseractProvider {
    pub engine: Arc<dyn PdfEngine>,
}

impl GeometryProvider for TesseractProvider {
    fn extract_line_geometry(&self, pdf_path: &Path) -> Result<Vec<LineGeometry>, ExtractorError> {
        let mut geometries = Vec::new();

        let layout = self.engine.analyze_layout(pdf_path)
            .map_err(|e| ExtractorError::ExtractionFailed(e.to_string()))?;

        for page in 0..layout.total_pages {
            let blocks = self.engine.get_text_blocks(pdf_path, page).unwrap_or_default();
            
            // Tesseract provider activates only when text-layer line count < 10 (scanned document)
            if blocks.len() < 10 {
                tracing::info!("Page {} text block count ({}) < 10. Activating Tesseract OCR.", page, blocks.len());
                
                // IMPLEMENTATION PATH:
                // 1. Add `leptess = "0.14"` to Cargo.toml
                // 2. Ensure Tesseract binaries are in PATH
                // 3. Render page to image: self.engine.render_page(pdf_path, page, 300.0)
                // 4. Run LepTess OCR on the rendered image
                
                tracing::warn!("Tesseract OCR is disabled in this build (leptess dependency not present). Skipping page {}.", page);
            }
        }

        Ok(geometries)
    }
}

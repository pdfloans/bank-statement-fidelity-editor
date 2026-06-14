use super::engine::*;
use pdfium_render::prelude::*;
use std::path::Path;

#[derive(Debug)]
pub struct MuPdfEngine;

impl Default for MuPdfEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl MuPdfEngine {
    pub fn new() -> Self {
        Self
    }
}

impl PdfEngine for MuPdfEngine {
    fn capabilities(&self) -> EngineCapabilities {
        EngineCapabilities {
            supports_redaction: true,
            supports_cjk: false,
            supports_embedded_fonts: true,
            estimated_fidelity: 0.85,
        }
    }

    fn render_page(&self, path: &Path, page: usize, dpi: f32) -> Result<RenderedPage, EngineError> {
        tracing::info!("[pdfium] render_page called: {:?} page={} dpi={}", path, page, dpi);
        let bindings = Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./"))
            .or_else(|e| {
                tracing::warn!("[pdfium] bind_to_library(./) failed: {e}, trying '.'");
                Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("."))
            })
            .or_else(|e| {
                tracing::warn!("[pdfium] bind_to_library(.) failed: {e}, trying system");
                Pdfium::bind_to_system_library()
            })
            .map_err(|e| {
                tracing::error!("[pdfium] all binding attempts failed: {e}");
                EngineError::LoadFailed(format!("Pdfium library not found: {e}"))
            })?;
        tracing::info!("[pdfium] library bound successfully");
        let pdfium = Pdfium::new(bindings);
        let doc = pdfium
            .load_pdf_from_file(path, None)
            .map_err(|e| EngineError::LoadFailed(e.to_string()))?;

        let p = doc
            .pages()
            .get(page as u16)
            .map_err(|e| EngineError::RenderFailed(e.to_string()))?;

        let width_pts = p.width().value;
        let height_pts = p.height().value;
        let render_config =
            PdfRenderConfig::new().set_target_width((width_pts * dpi / 72.0) as i32);

        let bitmap = p
            .render_with_config(&render_config)
            .map_err(|e| EngineError::RenderFailed(e.to_string()))?;

        let mut png_bytes = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut png_bytes);
        if bitmap
            .as_image()
            .write_to(&mut cursor, image::ImageFormat::Png)
            .is_ok()
        {
            Ok(RenderedPage {
                png_bytes,
                width_pts,
                height_pts,
            })
        } else {
            Err(EngineError::RenderFailed(
                "Failed to write PNG bytes".into(),
            ))
        }
    }

    fn get_text_blocks(&self, _path: &Path, _page: usize) -> Result<Vec<TextBlock>, EngineError> {
        Err(EngineError::Unsupported)
    }

    fn find_text_block_at_click(
        &self,
        _path: &Path,
        _page: usize,
        _x: f32,
        _y: f32,
    ) -> Result<Option<TextBlock>, EngineError> {
        Err(EngineError::Unsupported)
    }

    fn apply_change(
        &self,
        _input: &Path,
        _output: &Path,
        _page: usize,
        _bbox: [f32; 4],
        _new_text: &str,
        _font_path: Option<&Path>,
    ) -> Result<ReplaceOutcome, EngineError> {
        Err(EngineError::Unsupported)
    }

    fn analyze_layout(&self, _path: &Path) -> Result<DocumentLayout, EngineError> {
        Err(EngineError::Unsupported)
    }
}

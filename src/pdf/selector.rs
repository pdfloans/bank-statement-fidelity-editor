use std::sync::Arc;
use super::engine::*;
use std::path::Path;

#[derive(Debug)]
pub struct PdfEngineSelector {
    primary: Arc<dyn PdfEngine>,
    fallback: Arc<dyn PdfEngine>,
}

impl PdfEngineSelector {
    pub fn new(primary: Arc<dyn PdfEngine>, fallback: Arc<dyn PdfEngine>) -> Self {
        Self { primary, fallback }
    }
    
    fn try_primary_or_fallback<T, F>(&self, operation: F) -> Result<T, EngineError>
    where
        F: Fn(&dyn PdfEngine) -> Result<T, EngineError>,
    {
        match operation(&*self.primary) {
            Ok(result) => Ok(result),
            Err(EngineError::Unsupported) => {
                tracing::warn!(
                    engine.fallback_triggered = true,
                    primary_error = "Unsupported",
                    "Primary engine unsupported, falling back"
                );
                operation(&*self.fallback)
            }
            Err(e) => {
                // If it's a shaping error or something that mupdf fails on but python handles:
                // we'll just fall back on ANY error for this alpha.
                tracing::warn!(
                    engine.fallback_triggered = true,
                    primary_error = %e,
                    "Primary engine failed, falling back"
                );
                operation(&*self.fallback)
            }
        }
    }
}

impl PdfEngine for PdfEngineSelector {
    fn capabilities(&self) -> EngineCapabilities {
        let p_cap = self.primary.capabilities();
        let f_cap = self.fallback.capabilities();
        EngineCapabilities {
            supports_redaction: p_cap.supports_redaction || f_cap.supports_redaction,
            supports_cjk: p_cap.supports_cjk || f_cap.supports_cjk,
            supports_embedded_fonts: p_cap.supports_embedded_fonts || f_cap.supports_embedded_fonts,
            estimated_fidelity: p_cap.estimated_fidelity.max(f_cap.estimated_fidelity),
        }
    }

    fn render_page(&self, path: &Path, page: usize, dpi: f32) -> Result<RenderedPage, EngineError> {
        self.try_primary_or_fallback(|engine| engine.render_page(path, page, dpi))
    }

    fn get_text_blocks(&self, path: &Path, page: usize) -> Result<Vec<TextBlock>, EngineError> {
        self.try_primary_or_fallback(|engine| engine.get_text_blocks(path, page))
    }

    fn find_text_block_at_click(&self, path: &Path, page: usize, x: f32, y: f32) -> Result<Option<TextBlock>, EngineError> {
        self.try_primary_or_fallback(|engine| engine.find_text_block_at_click(path, page, x, y))
    }

    fn apply_change(
        &self, 
        input: &Path, 
        output: &Path, 
        page: usize, 
        bbox: [f32; 4], 
        new_text: &str,
        font_path: Option<&Path>
    ) -> Result<ReplaceOutcome, EngineError> {
        self.try_primary_or_fallback(|engine| engine.apply_change(input, output, page, bbox, new_text, font_path))
    }

    fn analyze_layout(&self, path: &Path) -> Result<DocumentLayout, EngineError> {
        self.try_primary_or_fallback(|engine| engine.analyze_layout(path))
    }
}

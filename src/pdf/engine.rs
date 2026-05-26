use std::path::{Path, PathBuf};
use thiserror::Error;
use serde::{Serialize, Deserialize};

#[derive(Error, Debug)]
pub enum EngineError {
    #[error("Unsupported operation for this engine")]
    Unsupported,
    #[error("Failed to load document: {0}")]
    LoadFailed(String),
    #[error("Failed to render page: {0}")]
    RenderFailed(String),
    #[error("Failed to extract text: {0}")]
    ExtractFailed(String),
    #[error("Failed to apply change: {0}")]
    ApplyFailed(String),
    #[error("Layout analysis failed: {0}")]
    LayoutFailed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineCapabilities {
    pub supports_redaction: bool,
    pub supports_cjk: bool,
    pub supports_embedded_fonts: bool,
    pub estimated_fidelity: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplaceOutcome {
    pub success: bool,
    pub font_used: String,
    pub overflow: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextBlock {
    pub page: usize,
    pub text: String,
    pub bbox: [f32; 4],
    pub font: String,
    pub size: f32,
}

#[derive(Debug, Clone)]
pub struct RenderedPage {
    pub png_bytes: Vec<u8>,
    pub width_pts: f32,
    pub height_pts: f32,
}

// Re-export DocumentLayout from engine layer
pub use crate::engine::layout::DocumentLayout;

/// The core trait for PDF rendering and manipulation.
pub trait PdfEngine: Send + Sync + std::fmt::Debug {
    fn capabilities(&self) -> EngineCapabilities;
    
    fn render_page(&self, path: &Path, page: usize, dpi: f32) -> Result<RenderedPage, EngineError>;
    
    fn get_text_blocks(&self, path: &Path, page: usize) -> Result<Vec<TextBlock>, EngineError>;
    
    fn find_text_block_at_click(&self, path: &Path, page: usize, x: f32, y: f32) -> Result<Option<TextBlock>, EngineError>;
    
    fn apply_change(
        &self, 
        input: &Path, 
        output: &Path, 
        page: usize, 
        bbox: [f32; 4], 
        new_text: &str
    ) -> Result<ReplaceOutcome, EngineError>;
    
    fn analyze_layout(&self, path: &Path) -> Result<DocumentLayout, EngineError>;
}

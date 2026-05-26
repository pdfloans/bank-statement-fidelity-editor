use std::path::Path;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GeometrySource {
    TextLayer,
    Ocr,
    BankTemplate { template_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineGeometry {
    pub page: usize,
    pub line_on_page: usize,
    pub text: String,
    pub bbox: [f32; 4],
    pub confidence: f32,
    pub source: GeometrySource,
}

#[derive(Error, Debug)]
pub enum ExtractorError {
    #[error("Failed to extract geometry: {0}")]
    ExtractionFailed(String),
}

pub trait GeometryProvider: Send + Sync {
    fn extract_line_geometry(&self, pdf_path: &Path) -> Result<Vec<LineGeometry>, ExtractorError>;
}

//! Font Replication
//!
//! High-level orchestration for the deep font replication pipeline. The
//! heavy-lifting (extraction, glyph rasterisation, optional AI extrapolation)
//! lives in `python/font_replicator.py` and is invoked through the PyO3
//! bridge. This Rust module exposes serialisable data types so the rest of
//! the engine can consume the result without touching Python directly.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontMetrics {
    pub upm: u16,
    pub ascender: i16,
    pub descender: i16,
    pub font_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlyphImage {
    pub char: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepFontReplicationResult {
    pub success: bool,
    pub metrics: Option<FontMetrics>,
    pub images: Vec<GlyphImage>,
    pub baseline_y: Option<i32>,
    pub error: Option<String>,
}

impl DeepFontReplicationResult {
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn font_path(&self) -> Option<PathBuf> {
        self.metrics.as_ref().map(|m| PathBuf::from(&m.font_path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_successful_replication_payload() {
        let json = r#"{
            "success": true,
            "metrics": {
                "upm": 1000,
                "ascender": 800,
                "descender": -200,
                "font_path": "out/extracted_subset.ttf"
            },
            "images": [{"char": "A", "path": "out/glyph_65.png"}],
            "baseline_y": 200
        }"#;
        let parsed = DeepFontReplicationResult::from_json(json).unwrap();
        assert!(parsed.success);
        assert_eq!(
            parsed.font_path().unwrap().to_string_lossy(),
            "out/extracted_subset.ttf"
        );
        assert_eq!(parsed.images.len(), 1);
    }

    #[test]
    fn parses_a_failure_payload() {
        let json = r#"{"success": false, "error": "Font not found", "images": []}"#;
        let parsed = DeepFontReplicationResult::from_json(json).unwrap();
        assert!(!parsed.success);
        assert_eq!(parsed.error.as_deref(), Some("Font not found"));
    }
}

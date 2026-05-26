//! Text-Only PDF Fidelity Editor v0.1
//! The ONLY job: Replace text while preserving 100% visual fidelity (kerning, font, size, color, position).

use thiserror::Error;

#[derive(Error, Debug)]
pub enum TextEditError {
    #[error("Text replacement failed: {0}")]
    ReplacementFailed(String),
}

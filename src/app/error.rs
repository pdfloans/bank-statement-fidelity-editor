use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum AppError {
    #[error("API configuration is missing: {0}")]
    ApiConfigMissing(String),

    #[error("Required font '{0}' is missing from the system")]
    FontMissing(String),

    #[error("Failed to load PDF document: {0}")]
    PdfLoadError(String),

    #[error("I/O error occurred: {0}")]
    IoError(std::sync::Arc<std::io::Error>),

    #[error("API failure or rate limit: {0}")]
    ApiFailure(String),

    #[error("Visual verify failed: {0}")]
    VisualDrift(String),

    #[error("Parse failure or malformed PDF: {0}")]
    ParseFailure(String),

    #[error("Unknown error: {0}")]
    Unknown(String),
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        Self::IoError(std::sync::Arc::new(err))
    }
}

impl AppError {
    /// Parses string errors back into AppError for autofix interception
    pub fn parse_msg(msg: &str) -> Option<Self> {
        let lower = msg.to_lowercase();
        if lower.contains("api_key_invalid") || lower.contains("api key") || lower.contains("credentials") {
            Some(Self::ApiConfigMissing(msg.to_string()))
        } else if lower.contains("429") || lower.contains("quota") || lower.contains("rate limit") || lower.contains("api error") {
            Some(Self::ApiFailure(msg.to_string()))
        } else if lower.contains("font missing") || lower.contains("coverage missing chars") || lower.contains("fontconfig") {
            Some(Self::FontMissing(msg.to_string()))
        } else if lower.contains("visual verify failed") || lower.contains("drifted") || lower.contains("didn't converge") {
            Some(Self::VisualDrift(msg.to_string()))
        } else if lower.contains("parse") || lower.contains("corrupt") || lower.contains("invalid pdf") || lower.contains("0 transactions") {
            Some(Self::ParseFailure(msg.to_string()))
        } else {
            None
        }
    }

    /// Suggests an actionable UI fix for the specific error
    pub fn suggested_action(&self) -> Option<&'static str> {
        match self {
            Self::ApiConfigMissing(_) => Some("Open Settings to configure API Keys"),
            Self::ApiFailure(_) => Some("Retry with a different AI Provider (e.g. Gemini, OpenRouter) or wait for quota reset"),
            Self::FontMissing(_) => Some("Synthesize Font via Typst Reconstruction (Slower but 100% Fidelity)"),
            Self::VisualDrift(_) => Some("Proceed anyway, or Retry with Typst Reconstruction"),
            Self::ParseFailure(_) => Some("Retry with Offline OCR / LlamaParse fallback"),
            Self::Unknown(_) => Some("Retry the last action or check logs"),
            _ => None,
        }
    }
}

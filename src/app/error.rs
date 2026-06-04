use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("API configuration is missing: {0}")]
    ApiConfigMissing(String),

    #[error("Required font '{0}' is missing from the system")]
    FontMissing(String),

    #[error("Failed to load PDF document: {0}")]
    PdfLoadError(String),

    #[error("I/O error occurred: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Workflow failed: {0}")]
    WorkflowError(String),

    #[error("Runtime engine error: {0}")]
    EngineError(String),
    
    #[error("Unknown error: {0}")]
    Unknown(String),
}

impl AppError {
    /// Parses string errors back into AppError for autofix interception
    pub fn from_str(msg: &str) -> Option<Self> {
        let lower = msg.to_lowercase();
        if lower.contains("api_key_invalid") || lower.contains("api key") || lower.contains("document ai configuration") {
            Some(Self::ApiConfigMissing(msg.to_string()))
        } else if lower.contains("font missing") || lower.contains("coverage missing chars") {
            Some(Self::FontMissing(msg.to_string()))
        } else if lower.contains("drifted") || lower.contains("visual didn't converge") {
            Some(Self::WorkflowError(msg.to_string()))
        } else {
            None
        }
    }

    /// Suggests an actionable UI fix for the specific error
    pub fn suggested_action(&self) -> Option<&'static str> {
        match self {
            Self::ApiConfigMissing(_) => Some("Open Settings to configure API Keys"),
            Self::FontMissing(_) => Some("Synthesize Font via Deep Replication"),
            _ => None,
        }
    }
}

use std::path::PathBuf;
use std::env;

#[derive(Debug, Clone, Default)]
pub struct DocumentAiConfig {
    pub project_id: String,
    pub location: String,
    pub processor_id: String,
    pub service_account_path: String,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub gemini_api_key: Option<String>,
    pub pdfrest_api_key: Option<String>,
    pub document_ai: Option<DocumentAiConfig>,
    pub pymupdf_pro_key: String,
    pub passphrase: String,
    pub otel_endpoint: Option<String>,
    pub otel_service_name: String,
    pub log_dir: PathBuf,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            gemini_api_key: None,
            pdfrest_api_key: None,
            document_ai: None,
            pymupdf_pro_key: "hFKt4hca03GCFLAFLEGz5Bd3".into(),
            passphrase: "DEV_PASSPHRASE".into(),
            otel_endpoint: None,
            otel_service_name: "dual-core-pdf-pipeline".into(),
            log_dir: PathBuf::from("./logs"),
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ConfigError {
    #[error("Missing required environment variable: {0}")]
    MissingRequired(String),
}

impl AppConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let gemini_api_key = env::var("GEMINI_API_KEY").ok().filter(|s| !s.is_empty());
        let pdfrest_api_key = env::var("PDFREST_API_KEY").ok().filter(|s| !s.is_empty());
        
        let document_ai = match (
            env::var("DOCUMENT_AI_PROJECT_ID").ok().filter(|s| !s.is_empty()),
            env::var("DOCUMENT_AI_LOCATION").ok().filter(|s| !s.is_empty()),
            env::var("DOCUMENT_AI_PROCESSOR_ID").ok().filter(|s| !s.is_empty()),
            env::var("GOOGLE_APPLICATION_CREDENTIALS").ok().filter(|s| !s.is_empty()),
        ) {
            (Some(project_id), Some(location), Some(processor_id), Some(service_account_path)) => {
                Some(DocumentAiConfig { project_id, location, processor_id, service_account_path })
            }
            _ => None,
        };

        let pymupdf_pro_key = env::var("PYMUPDF_PRO_KEY")
            .unwrap_or_else(|_| "hFKt4hca03GCFLAFLEGz5Bd3".to_string());
            
        let passphrase = env::var("DUAL_CORE_PASSPHRASE")
            .map_err(|_| ConfigError::MissingRequired("DUAL_CORE_PASSPHRASE".to_string()))?;

        let otel_endpoint = env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok().filter(|s| !s.is_empty());
        let otel_service_name = env::var("OTEL_SERVICE_NAME")
            .unwrap_or_else(|_| "dual-core-pdf-pipeline".to_string());

        let log_dir = env::var("LOG_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("./logs"));

        Ok(Self {
            gemini_api_key,
            pdfrest_api_key,
            document_ai,
            pymupdf_pro_key,
            passphrase,
            otel_endpoint,
            otel_service_name,
            log_dir,
        })
    }
}

use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct DocumentAiConfig {
    pub project_id: String,
    pub location: String,
    pub processor_id: String,
    /// Optional path to a Google Cloud Service Account JSON key (legacy auth).
    /// If empty, the client falls back to API-key auth (`api_key` field).
    pub service_account_path: String,
    /// Optional Document AI API key (Beta). Takes precedence over OAuth when set.
    pub api_key: String,
    /// Optional path to Application Default Credentials JSON (set by
    /// `gcloud auth application-default login`). Auto-detected from the
    /// platform's well-known location when not set explicitly.
    pub adc_path: String,
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
    pub webhook_url: Option<String>,
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
            webhook_url: None,
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
        let webhook_url = env::var("WEBHOOK_URL").ok().filter(|s| !s.is_empty());

        // Document AI: the project/location/processor identify the processor.
        // Auth can be (in priority order):
        //  1. API key (Beta, preferred): DOCUMENT_AI_API_KEY
        //  2. Application Default Credentials: auto-detected, or override via
        //     CLOUDSDK_CONFIG / explicit ADC path
        //  3. Service-account JSON: GOOGLE_APPLICATION_CREDENTIALS
        // At least one of those must be present alongside the processor ids
        // for Document AI to be considered configured.
        let proj = env::var("DOCUMENT_AI_PROJECT_ID")
            .ok()
            .filter(|s| !s.is_empty());
        let loc = env::var("DOCUMENT_AI_LOCATION")
            .ok()
            .filter(|s| !s.is_empty());
        let proc_id = env::var("DOCUMENT_AI_PROCESSOR_ID")
            .ok()
            .filter(|s| !s.is_empty());
        let sa_path = env::var("GOOGLE_APPLICATION_CREDENTIALS")
            .ok()
            .filter(|s| !s.is_empty());
        let api_key = env::var("DOCUMENT_AI_API_KEY")
            .ok()
            .filter(|s| !s.is_empty());
        let adc_path = detect_adc_path();

        let document_ai = match (proj, loc, proc_id) {
            (Some(project_id), Some(location), Some(processor_id))
                if api_key.is_some() || sa_path.is_some() || adc_path.is_some() =>
            {
                Some(DocumentAiConfig {
                    project_id,
                    location,
                    processor_id,
                    service_account_path: sa_path.unwrap_or_default(),
                    api_key: api_key.unwrap_or_default(),
                    adc_path: adc_path.unwrap_or_default(),
                })
            }
            _ => None,
        };

        let pymupdf_pro_key =
            env::var("PYMUPDF_PRO_KEY").unwrap_or_else(|_| "hFKt4hca03GCFLAFLEGz5Bd3".to_string());

        let passphrase = env::var("DUAL_CORE_PASSPHRASE")
            .map_err(|_| ConfigError::MissingRequired("DUAL_CORE_PASSPHRASE".to_string()))?;

        let otel_endpoint = env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
            .ok()
            .filter(|s| !s.is_empty());
        let otel_service_name =
            env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "dual-core-pdf-pipeline".to_string());

        let log_dir = env::var("LOG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./logs"));

        Ok(Self {
            gemini_api_key,
            pdfrest_api_key,
            document_ai,
            pymupdf_pro_key,
            passphrase,
            otel_endpoint,
            otel_service_name,
            log_dir,
            webhook_url,
        })
    }
}

/// Locate the Application Default Credentials file written by
/// `gcloud auth application-default login`. Returns `None` if no ADC file
/// can be found at any of the standard locations.
///
/// Priority order:
///  1. `GOOGLE_APPLICATION_CREDENTIALS_ADC` (custom override, opt-in)
///  2. `CLOUDSDK_CONFIG` env var (gcloud-supported override) +
///     `application_default_credentials.json`
///  3. Platform default:
///     - Windows:  `%APPDATA%\gcloud\application_default_credentials.json`
///     - Unix:     `$HOME/.config/gcloud/application_default_credentials.json`
fn detect_adc_path() -> Option<String> {
    if let Ok(p) = env::var("GOOGLE_APPLICATION_CREDENTIALS_ADC") {
        if !p.is_empty() && std::path::Path::new(&p).exists() {
            return Some(p);
        }
    }
    if let Ok(cfg) = env::var("CLOUDSDK_CONFIG") {
        let candidate = PathBuf::from(cfg).join("application_default_credentials.json");
        if candidate.exists() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    let candidate = if cfg!(windows) {
        env::var("APPDATA").ok().map(|d| {
            PathBuf::from(d)
                .join("gcloud")
                .join("application_default_credentials.json")
        })
    } else {
        env::var("HOME").ok().map(|d| {
            PathBuf::from(d)
                .join(".config")
                .join("gcloud")
                .join("application_default_credentials.json")
        })
    };
    candidate
        .filter(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_adc_path_returns_string_or_none_without_panicking() {
        // Whatever the platform, this must not crash.
        let _ = detect_adc_path();
    }
}

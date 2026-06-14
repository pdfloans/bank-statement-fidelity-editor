use std::env;
use std::path::PathBuf;
use std::sync::OnceLock;

use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use serde::{Deserialize, Serialize};

use crate::app::env_spec::is_well_formed_pro_key;
use crate::error::{ConfigError, ConfigResult};

/// Minimum passphrase length for security (16 characters)
const MIN_PASSPHRASE_LENGTH: usize = 16;

/// Minimum passphrase length for development mode
const DEV_PASSPHRASE_MIN_LENGTH: usize = 8;

pub static HTTP_CLIENT: OnceLock<ClientWithMiddleware> = OnceLock::new();

pub fn global_http_client() -> ClientWithMiddleware {
    HTTP_CLIENT.get_or_init(|| {
        let retry_policy = ExponentialBackoff::builder().build_with_max_retries(3);
        let reqwest_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .tcp_keepalive(std::time::Duration::from_secs(60))
            .build();
        
        let reqwest_client = match reqwest_client {
            Ok(client) => client,
            Err(e) => {
                tracing::error!("[config] Failed to build HTTP client: {}. Using default client as fallback.", e);
                reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(60))
                    .build()
                    .unwrap_or_else(|e| {
                        tracing::error!("[config] Failed to build fallback HTTP client: {}. This is critical.", e);
                        std::process::exit(1);
                    })
            }
        };
            
        ClientBuilder::new(reqwest_client)
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build()
    }).clone()
}

/// Availability of PyMuPDF Pro per-segment editing/rendering (Subsystem B),
/// derived solely from the `PYMUPDF_PRO_KEY` value the application read.
///
/// This status governs **only** the high-fidelity per-segment edit/render
/// path. It deliberately has **no bearing** on `lopdf` split/merge
/// (Subsystem A), which runs in every runtime environment regardless of key
/// state (see [`AppConfig::pro_editing_available`]).
///
/// # Offline expiry caveat
/// A Pro key's expiry can only be confirmed by PyMuPDF at unlock time. There
/// is no offline expiry check, so a present, well-formed key is reported as
/// [`ProKeyStatus::Available`]; absence or a malformed value is reported as
/// [`ProKeyStatus::Unavailable`]. A well-formed-but-expired key cannot be
/// distinguished here and will surface its failure later, at unlock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProKeyStatus {
    /// A present, well-formed `PYMUPDF_PRO_KEY` was found; per-segment editing
    /// is expected to work (subject to unlock-time expiry verification).
    Available,
    /// No key, or a malformed key, was found; per-segment editing is
    /// unavailable. Splitting and merging remain available regardless.
    Unavailable,
}

impl ProKeyStatus {
    /// Returns `true` only when per-segment editing is available.
    pub fn is_available(&self) -> bool {
        matches!(self, ProKeyStatus::Available)
    }

    /// A human-readable, GUI/`serve`-friendly explanation of the status.
    pub fn reason(&self) -> &'static str {
        match self {
            ProKeyStatus::Available => {
                "Per-segment editing is available: a well-formed PYMUPDF_PRO_KEY was found \
                 (expiry is verified by PyMuPDF at unlock time)."
            }
            ProKeyStatus::Unavailable => {
                "Per-segment editing is unavailable: PYMUPDF_PRO_KEY is absent or malformed. \
                 Splitting and merging remain available."
            }
        }
    }
}

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
    /// GCS URI for batch process outputs (e.g. gs://my-bucket/outputs/).
    pub gcs_output_uri: String,
    /// Passphrase for encrypting the local Document AI cache
    pub passphrase: String,
}

/// How the Gemini calls authenticate.
///
/// `ApiKey` is the simplest (AI Studio `AIza...` key, default) and `Vertex`
/// is the enterprise option that authenticates with a Google Cloud service
/// account (or ADC) and calls the Vertex AI Gemini endpoint. Vertex keeps
/// data inside your GCP project and does not require an API key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeminiAuthMode {
    /// AI Studio API key (`generativelanguage.googleapis.com?key=...`).
    #[default]
    ApiKey,
    /// Vertex AI (`{location}-aiplatform.googleapis.com`) authenticated via a
    /// Google Cloud service account / ADC token. No API key used.
    Vertex,
}

impl DocumentAiConfig {
    /// Returns true if the Document AI configuration has valid authentication.
    pub fn has_auth(&self) -> bool {
        !self.api_key.is_empty() || !self.adc_path.is_empty() || !self.service_account_path.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionMode {
    Local,
    Remote { url: String },
}

impl Default for ConnectionMode {
    fn default() -> Self {
        Self::Local
    }
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub gemini_api_key: Option<String>,
    pub pdfrest_api_key: Option<String>,
    pub lipi_api_key: Option<String>,
    pub document_ai: Option<DocumentAiConfig>,
    pub pymupdf_pro_key: Option<String>, // Changed to Option - must come from env
    pub passphrase: String,
    pub otel_endpoint: Option<String>,
    pub otel_service_name: String,
    pub log_dir: PathBuf,
    pub webhook_url: Option<String>,
    /// How Gemini authenticates: AI Studio API key (default) or Vertex AI
    /// (service-account / ADC token). Set in-app via the Credentials panel
    /// or by `GEMINI_AUTH_MODE=vertex` in the environment.
    pub gemini_auth_mode: GeminiAuthMode,
    /// Whether we're in development mode (relaxed security requirements)
    pub is_dev_mode: bool,
    /// The connection mode (Local vs Remote Engine)
    pub connection_mode: ConnectionMode,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            gemini_api_key: None,
            pdfrest_api_key: None,
            lipi_api_key: None,
            document_ai: None,
            pymupdf_pro_key: None,
            passphrase: "DEV_PASSPHRASE".into(),
            otel_endpoint: None,
            otel_service_name: "dual-core-pdf-pipeline".into(),
            log_dir: PathBuf::from("./logs"),
            webhook_url: None,
            gemini_auth_mode: GeminiAuthMode::ApiKey,
            is_dev_mode: cfg!(debug_assertions),
            connection_mode: ConnectionMode::Local,
        }
    }
}

impl AppConfig {
    /// Loads configuration from environment variables.
    ///
    /// # Errors
    /// Returns `ConfigError` if required variables are missing or invalid.
    pub fn from_env() -> ConfigResult<Self> {
        let is_dev_mode = cfg!(debug_assertions);

        let clean_key = |key: Result<String, env::VarError>| -> Option<String> {
            key.ok()
                .map(|s| s.trim_matches('"').trim_matches('\'').trim().to_string())
                .filter(|s| !s.is_empty())
        };

        // Optional API keys
        let gemini_api_key = clean_key(env::var("GEMINI_API_KEY"));
        let pdfrest_api_key = clean_key(env::var("PDFREST_API_KEY"));
        let lipi_api_key = clean_key(env::var("LIPI_API_KEY"));
        let webhook_url = clean_key(env::var("WEBHOOK_URL"));

        // Document AI configuration
        let proj = clean_key(env::var("DOCUMENT_AI_PROJECT_ID"));
        let loc = clean_key(env::var("DOCUMENT_AI_LOCATION"));
        let proc_id = clean_key(env::var("DOCUMENT_AI_PROCESSOR_ID"));
        let sa_path = clean_key(env::var("GOOGLE_APPLICATION_CREDENTIALS"));
        let api_key = clean_key(env::var("DOCUMENT_AI_API_KEY"));
        let adc_path = detect_adc_path();
        let gcs_output_uri = clean_key(env::var("DOCUMENT_AI_GCS_URI")).unwrap_or_default();

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
                    gcs_output_uri,
                    passphrase: String::new(), // Filled in by AppConfig later
                })
            }
            _ => None,
        };

        // PyMuPDF Pro key - required in production
        let pymupdf_pro_key = clean_key(env::var("PYMUPDF_PRO_KEY"));

        // Gemini auth mode: default API key, opt into Vertex via env.
        let gemini_auth_mode = match env::var("GEMINI_AUTH_MODE")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "vertex" | "vertex_ai" | "vertexai" => GeminiAuthMode::Vertex,
            _ => GeminiAuthMode::ApiKey,
        };

        // Passphrase - required
        let passphrase = env::var("DUAL_CORE_PASSPHRASE")
            .map(|s| s.trim_matches('"').trim_matches('\'').trim().to_string())
            .map_err(|_| ConfigError::MissingRequired("DUAL_CORE_PASSPHRASE".to_string()))?;

        // Validate passphrase length
        let min_length = if is_dev_mode {
            DEV_PASSPHRASE_MIN_LENGTH
        } else {
            MIN_PASSPHRASE_LENGTH
        };

        if passphrase.len() < min_length {
            return Err(ConfigError::invalid_value(
                "DUAL_CORE_PASSPHRASE",
                format!(
                    "must be at least {} characters (got {})",
                    min_length,
                    passphrase.len()
                ),
            ));
        }

        // Optional OTEL configuration
        let otel_endpoint = env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
            .ok()
            .filter(|s| !s.is_empty());
        let otel_service_name =
            env::var("OTEL_SERVICE_NAME").unwrap_or_else(|_| "dual-core-pdf-pipeline".to_string());

        // Log directory - validate it can be created
        let log_dir = env::var("LOG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("./logs"));

        // Try to create the log directory to catch permission issues early
        if let Err(e) = std::fs::create_dir_all(&log_dir) {
            return Err(ConfigError::invalid_value(
                "LOG_DIR",
                format!("cannot create directory: {e}"),
            ));
        }

        let mut doc_ai = document_ai;
        if let Some(ref mut d) = doc_ai {
            d.passphrase = passphrase.clone();
        }

        Ok(Self {
            gemini_api_key,
            pdfrest_api_key,
            lipi_api_key,
            document_ai: doc_ai,
            pymupdf_pro_key,
            passphrase,
            otel_endpoint,
            otel_service_name,
            log_dir,
            webhook_url,
            gemini_auth_mode,
            is_dev_mode,
            connection_mode: ConnectionMode::Local,
        })
    }

    /// Validates the configuration and returns errors for any missing required items.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        if self.pymupdf_pro_key.is_none() && !self.is_dev_mode {
            errors.push("PYMUPDF_PRO_KEY environment variable is not set".to_string());
        }

        if self.passphrase.len() < MIN_PASSPHRASE_LENGTH && !self.is_dev_mode {
            errors.push(format!(
                "DUAL_CORE_PASSPHRASE must be at least {MIN_PASSPHRASE_LENGTH} characters"
            ));
        }

        errors
    }

    /// Reports whether PyMuPDF Pro per-segment editing/rendering (Subsystem B)
    /// is available, based on the `PYMUPDF_PRO_KEY` this config holds.
    ///
    /// A key is considered available when it is present and well-formed — a
    /// 24-character value with the `hFKt` trial-key prefix (Requirement 21.4).
    /// Because expiry cannot be verified offline, a well-formed key is treated
    /// as available and any expiry failure surfaces later at PyMuPDF unlock
    /// time. Absence or a malformed value yields [`ProKeyStatus::Unavailable`]
    /// (Requirements 11.1, 11.3, 21.1, 21.2, 21.3, 21.6).
    ///
    /// # Subsystem isolation
    /// This status governs **only** per-segment editing/rendering. The
    /// `lopdf` split/merge engine (Subsystem A) does **not** consult this and
    /// runs regardless of key state in every runtime environment — local GUI,
    /// local `serve`, and the Railway `pdfsitch` deployment (Requirements
    /// 11.2, 21.5). Nothing in this method or its callers should be used to
    /// gate splitting or merging.
    pub fn pro_key_status(&self) -> ProKeyStatus {
        match self.pymupdf_pro_key.as_deref() {
            Some(key) if is_well_formed_pro_key(key) => ProKeyStatus::Available,
            _ => ProKeyStatus::Unavailable,
        }
    }

    /// Convenience boolean form of [`AppConfig::pro_key_status`]: `true` when
    /// per-segment editing is available.
    ///
    /// This MUST NOT be used to gate splitting or merging — those run
    /// regardless of Pro-key state (Requirements 11.2, 21.5).
    pub fn pro_editing_available(&self) -> bool {
        self.pro_key_status().is_available()
    }

    /// A human-readable reason describing the current Pro-key availability,
    /// suitable for GUI status display or a headless `serve` return value
    /// (Requirements 11.3, 21.6).
    pub fn pro_editing_status_reason(&self) -> &'static str {
        self.pro_key_status().reason()
    }

    /// Returns true if the application has valid AI configuration for balancing.
    pub fn has_ai_for_balancing(&self) -> bool {
        self.gemini_api_key.is_some()
            && (self.document_ai.is_some() || self.pdfrest_api_key.is_some())
    }

    /// Returns true if the application has valid AI configuration for extraction.
    pub fn has_ai_for_extraction(&self) -> bool {
        self.document_ai.is_some()
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

    #[test]
    fn pro_editing_unavailable_when_key_absent() {
        let cfg = AppConfig {
            pymupdf_pro_key: None,
            ..AppConfig::default()
        };
        assert_eq!(cfg.pro_key_status(), ProKeyStatus::Unavailable);
        assert!(!cfg.pro_editing_available());
        assert!(cfg.pro_editing_status_reason().contains("unavailable"));
    }

    #[test]
    fn pro_editing_available_with_well_formed_trial_key() {
        let cfg = AppConfig {
            pymupdf_pro_key: Some("hFKt4hca03GCFLAFLEGz5Bd3".to_string()),
            ..AppConfig::default()
        };
        assert_eq!(cfg.pro_key_status(), ProKeyStatus::Available);
        assert!(cfg.pro_editing_available());
    }

    #[test]
    fn pro_editing_unavailable_with_malformed_key() {
        let cfg = AppConfig {
            pymupdf_pro_key: Some("not-a-valid-key".to_string()),
            ..AppConfig::default()
        };
        assert_eq!(cfg.pro_key_status(), ProKeyStatus::Unavailable);
        assert!(!cfg.pro_editing_available());
    }

    #[test]
    fn pro_key_status_does_not_affect_validate_for_split_merge() {
        // A missing Pro key must never cause additional validation beyond the
        // existing dev-mode-gated PYMUPDF_PRO_KEY check; split/merge are not
        // gated by Pro-key state.
        let cfg = AppConfig {
            pymupdf_pro_key: None,
            is_dev_mode: true,
            ..AppConfig::default()
        };
        // In dev mode the missing key is not reported as an error.
        assert!(!cfg
            .validate()
            .iter()
            .any(|e| e.contains("PYMUPDF_PRO_KEY")));
        // And availability is independent of validate().
        assert!(!cfg.pro_editing_available());
    }

    #[test]
    fn pro_key_status_serializes_to_json() -> anyhow::Result<()> {
        let json = serde_json::to_string(&ProKeyStatus::Available)?;
        assert!(json.contains("available"));
        Ok(())
    }
}

use crate::app::config::{global_http_client, AppConfig, GeminiAuthMode};
use crate::engine::layout::DocumentLayout;
use crate::engine::model::Transaction;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceAdjustment {
    pub page: usize,
    pub line_on_page: usize,
    pub old_running_balance: f64,
    pub new_running_balance: f64,
    pub reason: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiBalancePlan {
    pub adjustments: Vec<BalanceAdjustment>,
    pub overall_strategy: String,
    pub confidence: f32,
}

impl GeminiBalancePlan {
    pub fn validate(&mut self) -> Result<(), GeminiError> {
        if self.confidence < 0.0 || self.confidence > 1.0 {
            return Err(GeminiError::Format(format!(
                "BalancePlan confidence {} out of range",
                self.confidence
            )));
        }
        for adj in &mut self.adjustments {
            if adj.confidence < 0.0 || adj.confidence > 1.0 {
                return Err(GeminiError::Format(format!(
                    "Adjustment confidence {} out of range",
                    adj.confidence
                )));
            }
        }
        Ok(())
    }
}

/// Result of asking Gemini "did Document AI capture every transaction on the
/// page, and does the data look internally consistent?"
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiCompletenessReport {
    /// 0..1 — Gemini's confidence that the parse is complete.
    pub completeness_score: f32,
    /// Free-text explanation Gemini provided.
    pub notes: String,
    /// Rows or fields Gemini suspects were missed by Document AI.
    pub missing_rows: Vec<String>,
    /// True when the math (running balances, totals, opening/closing) is
    /// internally consistent.
    pub math_consistent: bool,
}

impl GeminiCompletenessReport {
    pub fn validate(&mut self) -> Result<(), GeminiError> {
        if self.completeness_score < 0.0 || self.completeness_score > 1.0 {
            return Err(GeminiError::Format(format!(
                "Completeness score {} out of range",
                self.completeness_score
            )));
        }
        Ok(())
    }
}

/// Result of a vision-based anomaly check on a rendered page.
///
/// Stage 4 / Item #10: after the workflow renders an edited PDF, we send the
/// page back to Gemini Vision and ask "does anything look off?" — kerning
/// drift, baseline misalignment, font-weight mismatch, colour shift,
/// hallucinated text. Hotspots that overlap an *intended* bbox are
/// expected (we just edited there); hotspots elsewhere on the page mean
/// the renderer collateral-damaged something and the loop should retry or
/// fail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiVisionReport {
    /// 0..1 — overall anomaly score. 0 = looks pristine, 1 = clearly broken.
    pub anomaly_score: f32,
    /// Rectangles (in PDF points) where Gemini saw something off.
    pub hotspots: Vec<VisionHotspot>,
    /// Free-text reasoning.
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionHotspot {
    pub bbox: [f32; 4],
    pub kind: String,
    /// Per-hotspot confidence the anomaly is real.
    pub confidence: f32,
}

impl GeminiVisionReport {
    /// Whether this report should reject the render.
    ///
    /// Rejects when overall `anomaly_score >= reject_threshold` OR when any
    /// hotspot lies outside every intended bbox (i.e. the renderer changed
    /// something we didn't ask it to).
    pub fn should_reject(&self, intended_bboxes: &[[f32; 4]], reject_threshold: f32) -> bool {
        if self.anomaly_score >= reject_threshold {
            return true;
        }
        for h in &self.hotspots {
            if !intended_bboxes
                .iter()
                .any(|b| crate::pdf::bbox_overlap_fraction(h.bbox, *b) > 0.0)
            {
                return true;
            }
        }
        false
    }

    pub fn validate(&mut self) -> Result<(), GeminiError> {
        if self.anomaly_score < 0.0 || self.anomaly_score > 1.0 {
            return Err(GeminiError::Format(format!(
                "Vision anomaly score {} out of range",
                self.anomaly_score
            )));
        }
        for h in &mut self.hotspots {
            if h.confidence < 0.0 || h.confidence > 1.0 {
                return Err(GeminiError::Format(format!(
                    "Hotspot confidence {} out of range",
                    h.confidence
                )));
            }
        }
        Ok(())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum GeminiError {
    #[error("Missing Configuration: GEMINI_API_KEY")]
    MissingKey,
    #[error("Missing Vertex AI configuration: DOCUMENT_AI_PROJECT_ID (+ location) and a service-account/ADC credential are required for Vertex mode")]
    MissingVertexConfig,
    #[error("Vertex AI auth error: {0}")]
    Vertex(String),
    #[error("Network Error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Middleware Error: {0}")]
    Middleware(#[from] reqwest_middleware::Error),
    #[error("API Error (HTTP {0}): {1}")]
    Api(StatusCode, String),
    #[error("Invalid Response: {0}")]
    InvalidResponse(String),
    #[error("Low Confidence: {0:.2}")]
    LowConfidence(f32),
    #[error("Format error: {0}")]
    Format(String),
}

pub struct GeminiClient {
    pub api_key: String,
    pub http: reqwest_middleware::ClientWithMiddleware,
    pub base_url: String,
    /// How this client authenticates and which endpoint family it targets.
    auth: GeminiAuth,
}

/// The best available Gemini **Pro** model id, tried first for all reasoning
/// and vision calls.
///
/// `gemini-2.5-pro` is Google's most advanced generally available reasoning model.
const GEMINI_PRO_MODEL: &str = "gemini-2.5-pro";

/// GA Pro fallback if the preview frontier model isn't enabled for a given
/// project/key (some projects must allowlist preview models). Still a top-tier
/// reasoning model and generally available on Vertex AI + the AI Studio API.
const GEMINI_PRO_FALLBACK: &str = "gemini-1.5-pro";

/// Last-resort flash fallback when neither Pro model is available for this
/// key/project (403/404).
const GEMINI_FLASH_FALLBACK: &str = "gemini-2.5-flash";

/// Resolved authentication strategy for a `GeminiClient`.
#[derive(Clone)]
enum GeminiAuth {
    /// AI Studio API key, appended as `?key=...` to the public endpoint.
    ApiKey,
    /// Vertex AI: a Google Cloud OAuth bearer token (minted from a service
    /// account or ADC) sent as `Authorization: Bearer`, targeting the
    /// regional `{location}-aiplatform.googleapis.com` endpoint for
    /// `projects/{project}/locations/{location}/publishers/google/models`.
    Vertex {
        project_id: String,
        location: String,
        access_token: String,
    },
}

impl GeminiClient {
    pub fn from_app_config(cfg: &AppConfig) -> Result<Self, GeminiError> {
        match cfg.gemini_auth_mode {
            GeminiAuthMode::Vertex => {
                // Vertex needs a GCP project + location and an OAuth token,
                // which we mint from the same Document AI service-account / ADC
                // credentials. This keeps a single GCP identity for both APIs.
                let doc_ai = cfg
                    .document_ai
                    .clone()
                    .ok_or(GeminiError::MissingVertexConfig)?;
                if doc_ai.project_id.is_empty() {
                    return Err(GeminiError::MissingVertexConfig);
                }
                let location = if doc_ai.location.is_empty() || doc_ai.location == "us" {
                    "us-central1".to_string()
                } else if doc_ai.location == "eu" {
                    "europe-west1".to_string()
                } else {
                    doc_ai.location.clone()
                };
                let access_token = mint_gcp_access_token(&doc_ai)
                    .map_err(|e| GeminiError::Vertex(format!("token mint failed: {e}")))?;
                let base_url = format!("https://{location}-aiplatform.googleapis.com");
                Ok(Self {
                    api_key: String::new(),
                    http: global_http_client(),
                    base_url,
                    auth: GeminiAuth::Vertex {
                        project_id: doc_ai.project_id,
                        location,
                        access_token,
                    },
                })
            }
            GeminiAuthMode::ApiKey => {
                let api_key = cfg.gemini_api_key.clone().ok_or(GeminiError::MissingKey)?;
                Ok(Self {
                    api_key,
                    http: global_http_client(),
                    base_url: "https://generativelanguage.googleapis.com".into(),
                    auth: GeminiAuth::ApiKey,
                })
            }
        }
    }

    pub async fn from_app_config_async(cfg: &AppConfig) -> Result<Self, GeminiError> {
        match cfg.gemini_auth_mode {
            GeminiAuthMode::Vertex => {
                let doc_ai = cfg
                    .document_ai
                    .clone()
                    .ok_or(GeminiError::MissingVertexConfig)?;
                if doc_ai.project_id.is_empty() {
                    return Err(GeminiError::MissingVertexConfig);
                }
                let location = if doc_ai.location.is_empty() || doc_ai.location == "us" {
                    "us-central1".to_string()
                } else if doc_ai.location == "eu" {
                    "europe-west1".to_string()
                } else {
                    doc_ai.location.clone()
                };
                let access_token = mint_gcp_access_token_async(&doc_ai)
                    .await
                    .map_err(|e| GeminiError::Vertex(format!("token mint failed: {e}")))?;
                let base_url = format!("https://{location}-aiplatform.googleapis.com");
                Ok(Self {
                    api_key: String::new(),
                    http: global_http_client(),
                    base_url,
                    auth: GeminiAuth::Vertex {
                        project_id: doc_ai.project_id,
                        location,
                        access_token,
                    },
                })
            }
            GeminiAuthMode::ApiKey => {
                let api_key = cfg.gemini_api_key.clone().ok_or(GeminiError::MissingKey)?;
                Ok(Self {
                    api_key,
                    http: global_http_client(),
                    base_url: "https://generativelanguage.googleapis.com".into(),
                    auth: GeminiAuth::ApiKey,
                })
            }
        }
    }

    /// Build the `generateContent` URL for `model` and return it alongside an
    /// optional bearer token to attach as an `Authorization` header.
    ///
    /// This is the single place that differs between the AI Studio API-key
    /// endpoint and the Vertex AI endpoint, so every request method routes
    /// through it and stays auth-agnostic.
    fn endpoint(&self, model: &str) -> (String, Option<String>) {
        match &self.auth {
            GeminiAuth::ApiKey => (
                format!(
                    "{}/v1beta/models/{}:generateContent?key={}",
                    self.base_url,
                    model,
                    self.api_key.trim()
                ),
                None,
            ),
            GeminiAuth::Vertex {
                project_id,
                location,
                access_token,
            } => (
                format!(
                    "{}/v1/projects/{}/locations/{}/publishers/google/models/{}:generateContent",
                    self.base_url, project_id, location, model
                ),
                Some(access_token.clone()),
            ),
        }
    }

    /// POST `body` to `model`'s endpoint, attaching the right auth for the
    /// active mode (`X-goog-api-key` header or bearer token).
    async fn post_generate(
        &self,
        model: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response, GeminiError> {
        let (url, bearer) = self.endpoint(model);

        let mut attempts = 0;
        let max_attempts = 4; // 1 initial + 3 retries
        loop {
            attempts += 1;
            let mut req = self.http.post(&url).json(body);
            match &self.auth {
                GeminiAuth::ApiKey => {
                    // Pass the API key in the header AND the URL query parameter
                    req = req.header("x-goog-api-key", self.api_key.trim());
                }
                GeminiAuth::Vertex { .. } => {
                    if let Some(ref token) = bearer {
                        req = req.bearer_auth(token);
                    }
                }
            }

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if (status.is_server_error() || status == 429) && attempts < max_attempts {
                        // If it's a 429 Quota/Rate Limit, don't sleep excessively, bail out after 1 retry
                        // to let the fallback chain proceed to the next tier immediately.
                        if status == 429 && attempts >= 2 {
                            tracing::warn!("Gemini 429 Too Many Requests for {}, aborting retries to allow fallback.", model);
                            return Ok(resp);
                        }
                        let delay = std::time::Duration::from_millis(500 * (1 << (attempts - 1)));
                        tracing::warn!(
                            "Gemini {} error for model {}, retrying in {:?}...",
                            status,
                            model,
                            delay
                        );
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    if status == reqwest::StatusCode::BAD_REQUEST
                        || status == reqwest::StatusCode::UNAUTHORIZED
                        || status == reqwest::StatusCode::FORBIDDEN
                    {
                        tracing::error!("Gemini API rejected request with {} for model {}! Please verify your GEMINI_API_KEY and quota.", status, model);
                    }
                    return Ok(resp);
                }
                Err(e) => {
                    if attempts < max_attempts {
                        let delay = std::time::Duration::from_millis(500 * (1 << (attempts - 1)));
                        tracing::warn!("Gemini network error {}, retrying in {:?}...", e, delay);
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    return Err(e.into());
                }
            }
        }
    }

    /// POST to the best available **Pro** model, with a graceful fallback
    /// chain. Tries the frontier preview model first, then the GA Pro model
    /// (for projects/keys that haven't allowlisted preview models, or whose
    /// free tier has no quota for the preview model — HTTP 429), then flash
    /// as a last resort. All reasoning and vision calls go through here so they
    /// always prefer Pro — never the old flash-by-default behavior.
    async fn post_generate_pro(
        &self,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response, GeminiError> {
        // A model is "unavailable for this key/project" when access is denied
        // (403), the model id isn't served (404), or the key has no quota for
        // it (429 — common on the AI Studio free tier for preview models).
        fn should_fall_back(status: StatusCode) -> bool {
            status == StatusCode::FORBIDDEN
                || status == StatusCode::NOT_FOUND
                || status == StatusCode::TOO_MANY_REQUESTS
        }

        // Tier 1: frontier preview Pro (best, finance-tuned).
        let response = self.post_generate(GEMINI_PRO_MODEL, body).await?;
        if !should_fall_back(response.status()) {
            return Ok(response);
        }
        tracing::warn!(
            "[gemini] Pro model '{}' unavailable ({}); falling back to GA '{}'",
            GEMINI_PRO_MODEL,
            response.status(),
            GEMINI_PRO_FALLBACK
        );

        // Tier 2: GA Pro.
        let response = self.post_generate(GEMINI_PRO_FALLBACK, body).await?;
        if !should_fall_back(response.status()) {
            return Ok(response);
        }
        tracing::warn!(
            "[gemini] GA Pro '{}' unavailable ({}); falling back to flash '{}'",
            GEMINI_PRO_FALLBACK,
            response.status(),
            GEMINI_FLASH_FALLBACK
        );

        // Tier 3: flash, last resort.
        self.post_generate(GEMINI_FLASH_FALLBACK, body).await
    }

    // Internal method for testing
    
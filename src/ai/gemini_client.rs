use crate::app::config::{AppConfig, GeminiAuthMode};
use crate::engine::layout::DocumentLayout;
use crate::engine::model::Transaction;
use reqwest::{Client, StatusCode};
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
    pub fn should_reject(
        &self,
        intended_bboxes: &[[f32; 4]],
        reject_threshold: f32,
    ) -> bool {
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
}

#[derive(thiserror::Error, Debug)]
pub enum GeminiError {
    #[error("Missing Configuration: GEMINI_API_KEY")]
    MissingKey,
    #[error("Missing Vertex AI configuration: DOCUMENT_AI_PROJECT_ID (+ location) and a service-account/ADC credential are required for Vertex mode")]
    MissingVertexConfig,
    #[error("Vertex AI auth error: {0}")]
    Vertex(String),
    #[error("HTTP Error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API Error (HTTP {0}): {1}")]
    Api(StatusCode, String),
    #[error("Invalid Response: {0}")]
    InvalidResponse(String),
    #[error("Low Confidence: {0:.2}")]
    LowConfidence(f32),
}

pub struct GeminiClient {
    api_key: String,
    http: Client,
    base_url: String,
    /// How this client authenticates and which endpoint family it targets.
    auth: GeminiAuth,
}

/// The best available Gemini **Pro** model id, tried first for all reasoning
/// and vision calls.
///
/// `gemini-3.1-pro-preview` is Google's most advanced reasoning model and the
/// designated replacement for the now-shutdown `gemini-3-pro-preview`
/// (retired Mar 9, 2026). Google specifically calls out its agentic
/// improvements "in domains like finance and spreadsheet applications", which
/// is exactly this statement-balancing workload — so it's both the most
/// capable and the most specialized fit. There is no separate finance-specific
/// `generateContent` model; this is it.
const GEMINI_PRO_MODEL: &str = "gemini-3.1-pro-preview";

/// GA Pro fallback if the preview frontier model isn't enabled for a given
/// project/key (some projects must allowlist preview models). Still a top-tier
/// reasoning model and generally available on Vertex AI + the AI Studio API.
const GEMINI_PRO_FALLBACK: &str = "gemini-2.5-pro";

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
                let location = if doc_ai.location.is_empty() {
                    "us-central1".to_string()
                } else {
                    doc_ai.location.clone()
                };
                let access_token = mint_gcp_access_token(&doc_ai)
                    .map_err(|e| GeminiError::Vertex(format!("token mint failed: {e}")))?;
                let base_url = format!("https://{location}-aiplatform.googleapis.com");
                Ok(Self {
                    api_key: String::new(),
                    http: Client::new(),
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
                    http: Client::new(),
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
                    self.base_url, model, self.api_key
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
    /// active mode (query-param API key or bearer token).
    async fn post_generate(
        &self,
        model: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response, GeminiError> {
        let (url, bearer) = self.endpoint(model);
        let mut req = self.http.post(&url).json(body);
        if let Some(token) = bearer {
            req = req.bearer_auth(token);
        }
        Ok(req.send().await?)
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
        Ok(self.post_generate(GEMINI_FLASH_FALLBACK, body).await?)
    }

    // Internal method for testing
    #[cfg(test)]
    fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            api_key,
            http: Client::new(),
            base_url,
            auth: GeminiAuth::ApiKey,
        }
    }

    pub async fn propose_balance_adjustments(
        &self,
        transactions: &[Transaction],
        imbalance: f64,
        layout: &DocumentLayout,
    ) -> Result<GeminiBalancePlan, GeminiError> {
        let prompt = format!(
            "You are an expert financial auditor.\n\
             A bank statement has a mathematical imbalance of ${:.2}.\n\
             Analyze the transactions and propose the minimal cascading adjustments to the running balances to fix it.\n\
             Transactions: {}\n\
             Document layout summary: {} pages.",
            imbalance,
            serde_json::to_string(transactions).unwrap_or_default(),
            layout.total_pages
        );

        let schema = json!({
            "type": "object",
            "properties": {
                "adjustments": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "page": { "type": "integer" },
                            "line_on_page": { "type": "integer" },
                            "old_running_balance": { "type": "number" },
                            "new_running_balance": { "type": "number" },
                            "reason": { "type": "string" },
                            "confidence": { "type": "number" }
                        },
                        "required": ["page", "line_on_page", "old_running_balance", "new_running_balance", "reason", "confidence"]
                    }
                },
                "overall_strategy": { "type": "string" },
                "confidence": { "type": "number" }
            },
            "required": ["adjustments", "overall_strategy", "confidence"]
        });

        let body = json!({
            "contents": [{
                "parts": [{ "text": prompt }]
            }],
            "generationConfig": {
                "responseMimeType": "application/json",
                "responseSchema": schema
            }
        });

        let response = self.post_generate_pro(&body).await?;

        if !response.status().is_success() {
            return Err(GeminiError::Api(
                response.status(),
                response.text().await.unwrap_or_default(),
            ));
        }

        let json_resp: serde_json::Value = response.json().await?;
        let plan_text = json_resp["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .ok_or_else(|| GeminiError::InvalidResponse("Missing or invalid text field".into()))?;

        let plan: GeminiBalancePlan = serde_json::from_str(plan_text)
            .map_err(|e| GeminiError::InvalidResponse(format!("JSON parse error: {}", e)))?;

        if plan.confidence < 0.7 {
            return Err(GeminiError::LowConfidence(plan.confidence));
        }

        Ok(plan)
    }

    /// Ask Gemini to validate that Document AI captured every transaction on
    /// the page and that the resulting numbers are internally consistent.
    /// This is stage 1 of the user-facing workflow.
    pub async fn validate_parse_completeness(
        &self,
        transactions: &[Transaction],
        opening_balance: f64,
        closing_balance: f64,
        total_pages: usize,
    ) -> Result<GeminiCompletenessReport, GeminiError> {
        let prompt = format!(
            "You are a bank-statement auditor. Document AI extracted the \
             following transactions from a {} page statement.\n\n\
             Opening balance: ${:.2}\nClosing balance: ${:.2}\n\
             Transactions: {}\n\n\
             Confirm: (a) does the running ledger balance to the closing? \
             (b) is anything obviously missing (e.g. fee rows skipped, gap in \
             dates suggesting a row was not captured)? Reply ONLY in the \
             configured JSON schema.",
            total_pages,
            opening_balance,
            closing_balance,
            serde_json::to_string(transactions).unwrap_or_default(),
        );

        let schema = json!({
            "type": "object",
            "properties": {
                "completeness_score": { "type": "number" },
                "notes":              { "type": "string" },
                "missing_rows":       { "type": "array", "items": { "type": "string" } },
                "math_consistent":    { "type": "boolean" }
            },
            "required": ["completeness_score", "notes", "missing_rows", "math_consistent"]
        });

        let body = json!({
            "contents": [{ "parts": [{ "text": prompt }] }],
            "generationConfig": {
                "responseMimeType": "application/json",
                "responseSchema": schema
            }
        });

        let response = self.post_generate_pro(&body).await?;
        if !response.status().is_success() {
            return Err(GeminiError::Api(
                response.status(),
                response.text().await.unwrap_or_default(),
            ));
        }
        let json_resp: serde_json::Value = response.json().await?;
        let plan_text = json_resp["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .ok_or_else(|| GeminiError::InvalidResponse("Missing or invalid text field".into()))?;
        serde_json::from_str(plan_text)
            .map_err(|e| GeminiError::InvalidResponse(format!("JSON parse error: {}", e)))
    }

    /// Vision-based anomaly check on a rendered page.
    ///
    /// Stage 4 / Item #10. `page_png` is the PNG bytes of the rendered
    /// edited page; `intended_bboxes` is the list of bounding boxes (in PDF
    /// points) the user actually intended to change. Returns
    /// [`GeminiVisionReport`] with `anomaly_score` and any hotspots Gemini
    /// flagged.
    pub async fn validate_render_visually(
        &self,
        page_png: &[u8],
        intended_bboxes: &[[f32; 4]],
    ) -> Result<GeminiVisionReport, GeminiError> {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(page_png);

        let prompt = format!(
            "You are a forensic-document analyst examining a rendered bank \
             statement page that has been programmatically edited. \
             Examine the image for any visual anomalies: kerning drift, \
             baseline misalignment, font-weight or colour mismatch \
             vs neighbouring text, ghosted glyphs, redaction edge \
             artifacts, hallucinated or duplicated numbers. \n\n\
             The user *intended* to edit text inside these bounding boxes \
             (in PDF points, [x0, y0, x1, y1]): {}.\n\n\
             Return ONLY the configured JSON. anomaly_score should be 0.0 \
             when the page looks pristine and 1.0 when it's clearly broken. \
             Each hotspot is a region of concern; the bbox should be in PDF \
             points (the page is letter or A4, ~612x792pt). 'kind' is one \
             of: kerning, baseline, weight, color, ghost, edge, hallucinated. \
             Only flag genuinely suspicious regions; sub-pixel rendering \
             noise or expected redactions inside the intended bboxes are \
             not anomalies.",
            serde_json::to_string(intended_bboxes).unwrap_or_default()
        );

        let schema = json!({
            "type": "object",
            "properties": {
                "anomaly_score": { "type": "number" },
                "hotspots": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "bbox": {
                                "type": "array",
                                "items": { "type": "number" }
                            },
                            "kind": { "type": "string" },
                            "confidence": { "type": "number" }
                        },
                        "required": ["bbox", "kind", "confidence"]
                    }
                },
                "notes": { "type": "string" }
            },
            "required": ["anomaly_score", "hotspots", "notes"]
        });

        let body = json!({
            "contents": [{
                "parts": [
                    { "text": prompt },
                    {
                        "inline_data": {
                            "mime_type": "image/png",
                            "data": b64,
                        }
                    }
                ]
            }],
            "generationConfig": {
                "responseMimeType": "application/json",
                "responseSchema": schema
            }
        });

        // Vision benefits from the most capable Pro model; the helper falls
        // back to flash automatically if Pro is unavailable for this key.
        let response = self.post_generate_pro(&body).await?;

        if !response.status().is_success() {
            return Err(GeminiError::Api(
                response.status(),
                response.text().await.unwrap_or_default(),
            ));
        }
        let json_resp: serde_json::Value = response.json().await?;
        let report_text = json_resp["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .ok_or_else(|| GeminiError::InvalidResponse("Missing vision text field".into()))?;
        serde_json::from_str(report_text)
            .map_err(|e| GeminiError::InvalidResponse(format!("vision JSON parse: {}", e)))
    }
}

/// Mint a short-lived Google Cloud OAuth access token (scope
/// `cloud-platform`) for Vertex AI, from the same credentials the Document AI
/// client uses. Prefers an explicit service-account JSON key
/// (`service_account_path`); falls back to an ADC `authorized_user` file
/// (`adc_path`). Returns the bearer token string.
///
/// This is a synchronous, one-shot exchange (used at client construction)
/// implemented with `reqwest::blocking` so `from_app_config` stays non-async.
fn mint_gcp_access_token(
    doc_ai: &crate::app::config::DocumentAiConfig,
) -> Result<String, String> {
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use std::time::{SystemTime, UNIX_EPOCH};

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|e| format!("clock error: {e}"))?;

    let http = reqwest::blocking::Client::new();

    // Preferred: service-account JWT-bearer grant.
    if !doc_ai.service_account_path.is_empty() {
        let key_content = std::fs::read_to_string(&doc_ai.service_account_path)
            .map_err(|e| format!("read service account: {e}"))?;
        let sa: serde_json::Value =
            serde_json::from_str(&key_content).map_err(|e| format!("parse SA json: {e}"))?;
        let client_email = sa["client_email"]
            .as_str()
            .ok_or("service account missing client_email")?;
        let private_key = sa["private_key"]
            .as_str()
            .ok_or("service account missing private_key")?;

        #[derive(Serialize)]
        struct Claims {
            iss: String,
            scope: String,
            aud: String,
            iat: u64,
            exp: u64,
        }
        let claims = Claims {
            iss: client_email.to_string(),
            scope: "https://www.googleapis.com/auth/cloud-platform".to_string(),
            aud: "https://oauth2.googleapis.com/token".to_string(),
            iat: now,
            exp: now + 3600,
        };
        let mut header = Header::new(Algorithm::RS256);
        header.typ = Some("JWT".to_string());
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes())
            .map_err(|e| format!("bad private key: {e}"))?;
        let signed = encode(&header, &claims, &encoding_key)
            .map_err(|e| format!("jwt sign: {e}"))?;

        let resp = http
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &signed),
            ])
            .send()
            .map_err(|e| format!("token request: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!(
                "token endpoint {}: {}",
                resp.status(),
                resp.text().unwrap_or_default()
            ));
        }
        let v: serde_json::Value = resp.json().map_err(|e| format!("token json: {e}"))?;
        return v["access_token"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "token response missing access_token".to_string());
    }

    // Fallback: ADC authorized_user refresh-token grant.
    if !doc_ai.adc_path.is_empty() {
        let raw = std::fs::read_to_string(&doc_ai.adc_path)
            .map_err(|e| format!("read ADC: {e}"))?;
        let adc: serde_json::Value =
            serde_json::from_str(&raw).map_err(|e| format!("parse ADC json: {e}"))?;
        let client_id = adc["client_id"].as_str().ok_or("ADC missing client_id")?;
        let client_secret = adc["client_secret"]
            .as_str()
            .ok_or("ADC missing client_secret")?;
        let refresh_token = adc["refresh_token"]
            .as_str()
            .ok_or("ADC missing refresh_token")?;
        let resp = http
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("refresh_token", refresh_token),
            ])
            .send()
            .map_err(|e| format!("ADC token request: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!(
                "ADC token endpoint {}: {}",
                resp.status(),
                resp.text().unwrap_or_default()
            ));
        }
        let v: serde_json::Value = resp.json().map_err(|e| format!("ADC token json: {e}"))?;
        return v["access_token"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "ADC token response missing access_token".to_string());
    }

    Err("Vertex mode needs a service-account JSON (GOOGLE_APPLICATION_CREDENTIALS) or ADC".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn request_body_uses_camelcase_and_object_schema() {
        let _transactions: Vec<Transaction> = vec![];
        let _layout = DocumentLayout {
            total_pages: 1,
            pages: vec![],
            has_consistent_headers: true,
            has_consistent_footers: true,
            overall_style: "".into(),
            layout_confidence: 1.0,
        };

        // We simulate the request building logic
        let schema = json!({
            "type": "object",
            "properties": {
                "adjustments": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "page": { "type": "integer" },
                            "line_on_page": { "type": "integer" },
                            "old_running_balance": { "type": "number" },
                            "new_running_balance": { "type": "number" },
                            "reason": { "type": "string" },
                            "confidence": { "type": "number" }
                        },
                        "required": ["page", "line_on_page", "old_running_balance", "new_running_balance", "reason", "confidence"]
                    }
                },
                "overall_strategy": { "type": "string" },
                "confidence": { "type": "number" }
            },
            "required": ["adjustments", "overall_strategy", "confidence"]
        });

        let body = json!({
            "contents": [{
                "parts": [{ "text": "prompt" }]
            }],
            "generationConfig": {
                "responseMimeType": "application/json",
                "responseSchema": schema
            }
        });

        assert_eq!(
            body["generationConfig"]["responseMimeType"]
                .as_str()
                .unwrap(),
            "application/json"
        );
        assert!(body["generationConfig"]["responseSchema"].is_object());
    }

    #[tokio::test]
    async fn low_confidence_response_returns_error() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1beta/models/gemini-2.5-pro:generateContent"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "candidates": [{
                    "content": {
                        "parts": [{
                            "text": "{\"adjustments\": [], \"overall_strategy\": \"Unsure\", \"confidence\": 0.5}"
                        }]
                    }
                }]
            })))
            .mount(&server)
            .await;

        let client = GeminiClient::with_base_url("fake".into(), server.uri());

        let plan = client
            .propose_balance_adjustments(
                &[],
                0.0,
                &DocumentLayout {
                    total_pages: 1,
                    pages: vec![],
                    has_consistent_headers: true,
                    has_consistent_footers: true,
                    overall_style: "".into(),
                    layout_confidence: 1.0,
                },
            )
            .await;

        match plan {
            Err(GeminiError::LowConfidence(conf)) => assert_eq!(conf, 0.5),
            _ => panic!("Expected LowConfidence error"),
        }
    }

    fn hot(bbox: [f32; 4], kind: &str) -> VisionHotspot {
        VisionHotspot {
            bbox,
            kind: kind.into(),
            confidence: 0.9,
        }
    }

    #[test]
    fn vision_should_reject_when_overall_score_too_high() {
        let report = GeminiVisionReport {
            anomaly_score: 0.5,
            hotspots: vec![],
            notes: String::new(),
        };
        // No hotspots; threshold 0.15 -> reject by score alone.
        assert!(report.should_reject(&[], 0.15));
        assert!(!report.should_reject(&[], 0.6));
    }

    #[test]
    fn vision_should_reject_when_hotspot_outside_intended_bboxes() {
        let report = GeminiVisionReport {
            anomaly_score: 0.05,
            hotspots: vec![hot([200.0, 300.0, 250.0, 320.0], "kerning")],
            notes: String::new(),
        };
        let intended = vec![[100.0, 100.0, 150.0, 120.0]];
        // Score is fine but the hotspot is unintended -> reject.
        assert!(report.should_reject(&intended, 0.15));
    }

    #[test]
    fn vision_should_accept_when_hotspots_overlap_intended() {
        let report = GeminiVisionReport {
            anomaly_score: 0.04,
            hotspots: vec![hot([105.0, 102.0, 145.0, 118.0], "edge")],
            notes: String::new(),
        };
        let intended = vec![[100.0, 100.0, 150.0, 120.0]];
        // Both checks pass: low score, hotspot inside the intended bbox.
        assert!(!report.should_reject(&intended, 0.15));
    }

    #[test]
    fn vision_accepts_pristine_render_with_no_hotspots() {
        let report = GeminiVisionReport {
            anomaly_score: 0.0,
            hotspots: vec![],
            notes: "looks clean".into(),
        };
        assert!(!report.should_reject(&[], 0.15));
        assert!(!report.should_reject(&[[0.0, 0.0, 100.0, 100.0]], 0.15));
    }
}

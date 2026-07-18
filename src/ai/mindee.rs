//! Mindee Financial Document API client.
//!
//! Provides bank statement extraction via the Mindee REST API as a fallback
//! or alternative to Google Document AI. Uses the `financial_document` model
//! which returns structured transaction line items with per-field polygon
//! bounding boxes - a near-direct mapping to the existing `BankStatement` /
//! `Transaction` / `FieldBboxes` types.
//!
//! # Auth
//!
//! A single API key passed in the `Authorization: Token <key>` header.
//! No OAuth, JWT, or AWS-style signing required.
//!
//! # Flow
//!
//! 1. **Enqueue**: `POST /v1/financial_document/v1/predict_async` with the
//!    PDF as a multipart form upload.
//! 2. **Poll**: `GET /v1/documents/queue/{job_id}` until status is
//!    `completed` (or `failed`).
//! 3. **Parse**: Map the Mindee response JSON into `BankStatement`.
//!
//! # Cache
//!
//! Reuses [`crate::ai::docai_cache::DocAiCache`] for encrypted on-disk
//! caching, keyed by `(pdf_hash, "mindee", model_name)`.

use reqwest::StatusCode;
use reqwest_middleware::ClientWithMiddleware;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::path::Path;

use crate::ai::document_ai::BankStatement;
use crate::app::config::AppConfig;
use crate::engine::model::{f64_to_dec, FieldBboxes, Provenance, Transaction};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(thiserror::Error, Debug)]
pub enum MindeeError {
    #[error("Missing Configuration: {0}")]
    MissingConfig(&'static str),
    #[error("I/O Error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Network Error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Middleware Error: {0}")]
    Middleware(#[from] reqwest_middleware::Error),
    #[error("Parse Error: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("API Error (HTTP {0}): {1}")]
    Api(StatusCode, String),
    #[error("Polling timeout after {0} attempts")]
    PollTimeout(u32),
    #[error("Extraction failed: {0}")]
    ExtractionFailed(String),
}

// ---------------------------------------------------------------------------
// Strongly-typed Mindee response models (serde)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct MindeeV2EnqueueResponse {
    pub job: Option<MindeeV2Job>,
}

#[derive(Debug, Deserialize)]
pub struct MindeeV2Job {
    pub id: String,
    #[serde(default)]
    pub status: String,
    pub result_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MindeeV2JobResponse {
    pub job: Option<MindeeV2Job>,
}

#[derive(Debug, Deserialize)]
pub struct MindeeV2ResultResponse {
    pub inference: Option<MindeeV2Inference>,
}

#[derive(Debug, Deserialize)]
pub struct MindeeV2Inference {
    pub id: Option<String>,
    pub result: Option<MindeeV2Result>,
}

#[derive(Debug, Deserialize)]
pub struct MindeeV2Result {
    #[serde(default)]
    pub fields: std::collections::HashMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Maximum number of poll iterations before giving up.
const MAX_POLL_ATTEMPTS: u32 = 60;

/// Initial delay between poll attempts (milliseconds).
const INITIAL_POLL_DELAY_MS: u64 = 2000;

/// Maximum delay between poll attempts (milliseconds).
const MAX_POLL_DELAY_MS: u64 = 10_000;

/// The Mindee product name for the Financial Document model.

/// The Mindee API v1 base URL.

pub struct MindeeClient {
    api_key: String,
    http: ClientWithMiddleware,
    /// Product path, e.g. "mindee/financial_document".
    model_id: String,
    /// Passphrase for the encrypted cache.
    passphrase: String,
}

const MINDEE_API_V2_BASE: &str = "https://api-v2.mindee.net/v2";

impl MindeeClient {
    pub fn from_app_config(cfg: &AppConfig) -> Result<Self, MindeeError> {
        let api_key = cfg
            .mindee_api_key
            .clone()
            .filter(|k| !k.is_empty())
            .ok_or(MindeeError::MissingConfig("MINDEE_API_KEY"))?;

        let model_id = cfg
            .mindee_model_id
            .clone()
            .filter(|k| !k.is_empty())
            .ok_or(MindeeError::MissingConfig("MINDEE_MODEL_ID"))?;

        Ok(Self {
            api_key,
            http: crate::app::config::global_http_client(),
            model_id,
            passphrase: cfg.passphrase.clone(),
        })
    }

    pub async fn ping(&self) -> Result<(), MindeeError> {
        let url = format!("{}/products/extraction/enqueue", MINDEE_API_V2_BASE);
        let resp = self
            .http
            .post(&url)
            .header("Authorization", &self.api_key)
            .send()
            .await?;

        match resp.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                Err(MindeeError::Api(
                    StatusCode::UNAUTHORIZED,
                    "Invalid MINDEE_API_KEY - check your key at https://platform.mindee.com/".into(),
                ))
            }
            _ => Ok(()), // 400 Bad Request means key is valid but body missing, which is fine
        }
    }

    pub async fn parse_statement(&self, pdf_path: &Path) -> Result<BankStatement, MindeeError> {
        let cache = match crate::ai::docai_cache::DocAiCache::open_default(&self.passphrase) {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!("[mindee] cache disabled (open failed): {}", e);
                None
            }
        };
        let cache_key = if cache.is_some() {
            crate::ai::docai_cache::DocAiCache::make_key(
                pdf_path,
                "mindee",
                "api",
                &self.model_id,
                "v2",
            )
            .ok()
        } else {
            None
        };
        if let (Some(c), Some(k)) = (cache.as_ref(), cache_key.as_ref()) {
            if let Some(hit) = c.get(k) {
                tracing::info!("[mindee] cache HIT (skipping network) key={}", &k[..16]);
                return Ok(hit);
            }
        }

        let real_dims = get_real_page_dims(pdf_path);

        let job_id = self.enqueue(pdf_path).await?;
        tracing::info!("[mindee] enqueued job {}", job_id);

        let result_url = self.poll_until_complete(&job_id).await?;
        let response = self.get_results(&result_url).await?;

        let stmt = parse_mindee_response(&response, &real_dims)?;

        if let (Some(c), Some(k)) = (cache.as_ref(), cache_key.as_ref()) {
            if let Err(e) = c.put(k, &stmt) {
                tracing::warn!("[mindee] cache write failed: {}", e);
            }
        }

        Ok(stmt)
    }

    async fn enqueue(&self, pdf_path: &Path) -> Result<String, MindeeError> {
        let url = format!("{}/products/extraction/enqueue", MINDEE_API_V2_BASE);
        let pdf_bytes = tokio::fs::read(pdf_path).await?;
        let filename = pdf_path.file_name().unwrap_or_default().to_string_lossy().into_owned();

        let mut attempts = 0;
        let max_attempts = 4;
        loop {
            attempts += 1;
            let part = reqwest::multipart::Part::bytes(pdf_bytes.clone())
                .file_name(filename.clone())
                .mime_str("application/pdf")
                .unwrap_or_else(|_| reqwest::multipart::Part::bytes(Vec::new()));
            
            let form = reqwest::multipart::Form::new()
                .part("file", part)
                .text("model_id", self.model_id.clone());

            match self.http.post(&url).header("Authorization", &self.api_key).multipart(form).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if (status.is_server_error() || status == 429) && attempts < max_attempts {
                        let delay = std::time::Duration::from_millis(500 * (1 << (attempts - 1)));
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    if !status.is_success() {
                        return Err(MindeeError::Api(status, resp.text().await.unwrap_or_default()));
                    }
                    let enqueue_resp: MindeeV2EnqueueResponse = resp.json().await?;
                    let job = enqueue_resp.job.ok_or(MindeeError::ExtractionFailed("No job in enqueue response".into()))?;
                    return Ok(job.id);
                }
                Err(e) => {
                    if attempts < max_attempts {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        continue;
                    }
                    return Err(e.into());
                }
            }
        }
    }

    async fn poll_until_complete(&self, job_id: &str) -> Result<String, MindeeError> {
        let url = format!("{}/jobs/{}", MINDEE_API_V2_BASE, job_id);
        let mut delay_ms = INITIAL_POLL_DELAY_MS;

        for _ in 1..=MAX_POLL_ATTEMPTS {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            
            let resp = match self.http.get(&url).header("Authorization", &self.api_key).send().await {
                Ok(r) => r,
                Err(_) => {
                    delay_ms = (delay_ms * 3 / 2).min(MAX_POLL_DELAY_MS);
                    continue;
                }
            };

            if !resp.status().is_success() {
                let status = resp.status();
                if status.is_server_error() || status == 429 {
                    delay_ms = (delay_ms * 2).min(MAX_POLL_DELAY_MS);
                    continue;
                }
                return Err(MindeeError::Api(status, resp.text().await.unwrap_or_default()));
            }

            let queue_resp: MindeeV2JobResponse = resp.json().await?;
            if let Some(ref job_status) = queue_resp.job {
                match job_status.status.to_lowercase().as_str() {
                    "completed" => {
                        return job_status.result_url.clone().ok_or(MindeeError::ExtractionFailed("No result_url returned".into()));
                    }
                    "failed" => {
                        return Err(MindeeError::ExtractionFailed("Job failed".into()));
                    }
                    _ => { }
                }
            }
            delay_ms = (delay_ms * 3 / 2).min(MAX_POLL_DELAY_MS);
        }
        Err(MindeeError::PollTimeout(MAX_POLL_ATTEMPTS))
    }

    async fn get_results(&self, result_url: &str) -> Result<MindeeV2ResultResponse, MindeeError> {
        let resp = self.http.get(result_url).header("Authorization", &self.api_key).send().await?;
        if !resp.status().is_success() {
            return Err(MindeeError::Api(resp.status(), resp.text().await.unwrap_or_default()));
        }
        Ok(resp.json().await?)
    }
}

// Response -> BankStatement mapping
// ---------------------------------------------------------------------------

pub fn parse_mindee_response(
    response: &MindeeV2ResultResponse,
    real_page_dims: &std::collections::HashMap<usize, (f32, f32)>,
) -> Result<BankStatement, MindeeError> {
    let inference = response
        .inference
        .as_ref()
        .ok_or_else(|| MindeeError::ExtractionFailed("No inference in response".into()))?;

    let result = inference
        .result
        .as_ref()
        .ok_or_else(|| MindeeError::ExtractionFailed("No result in response".into()))?;

    let fields = &result.fields;

    let get_f64 = |name: &str| -> Option<f64> {
        fields.get(name)
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_f64())
    };

    let get_str = |name: &str| -> Option<String> {
        fields.get(name)
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };

    let opening_balance = get_f64("opening_balance").map(f64_to_dec).unwrap_or(Decimal::ZERO);
    let closing_balance = get_f64("closing_balance").map(f64_to_dec).unwrap_or(Decimal::ZERO);
    let account_number = get_str("account_number").or_else(|| get_str("customer_account_details"));

    let mut transactions = Vec::new();
    let mut total_pages = 1;

    // Look for transactions or line_items array
    let lines_arr = fields.get("transactions")
        .or_else(|| fields.get("line_items"))
        .and_then(|v| v.get("values").or_else(|| Some(v))) // Sometimes arrays are wrapped in values
        .and_then(|v| v.as_array());

    if let Some(lines) = lines_arr {
        for (idx, item) in lines.iter().enumerate() {
            let page_idx = item.get("page_id").and_then(|v| v.as_u64()).map(|v| v as usize).unwrap_or(0);
            if page_idx + 1 > total_pages {
                total_pages = page_idx + 1;
            }

            let (page_w, page_h) = real_page_dims
                .get(&page_idx)
                .copied()
                .unwrap_or((612.0, 792.0));

            let description = item.get("description").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let date = item.get("date").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            
            let amount = item.get("total_amount").or_else(|| item.get("amount")).and_then(|v| v.as_f64());
            let confidence = item.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;

            let polygon_val = item.get("polygon").and_then(|v| v.as_array());
            let mut polygon = Vec::new();
            if let Some(arr) = polygon_val {
                for pt in arr {
                    if let Some(coords) = pt.as_array() {
                        if coords.len() >= 2 {
                            if let (Some(x), Some(y)) = (coords[0].as_f64(), coords[1].as_f64()) {
                                polygon.push([x, y]);
                            }
                        }
                    }
                }
            }

            let row_bbox = polygon_to_bbox(&polygon, page_w, page_h);
            let field_bboxes = FieldBboxes::default();

            let mut debit = None;
            let mut credit = None;

            if let Some(amt) = amount {
                let dec_amt = f64_to_dec(amt);
                if dec_amt < Decimal::ZERO {
                    credit = Some(dec_amt.abs());
                } else {
                    debit = Some(dec_amt);
                }
            }

            transactions.push(Transaction {
                page: page_idx,
                line_on_page: idx,
                date,
                raw_text: description,
                debit,
                credit,
                running_balance: None,
                bbox: row_bbox,
                field_bboxes,
                provenance: Provenance::Mindee { confidence },
            });
        }
    }

    Ok(BankStatement {
        total_pages,
        transactions,
        opening_balance,
        closing_balance,
        account_number,
    })
}

pub fn polygon_to_bbox(
    polygon: &[[f64; 2]],
    page_width: f32,
    page_height: f32,
) -> Option<[f32; 4]> {
    if polygon.is_empty() {
        return None;
    }

    let mut x0 = f32::MAX;
    let mut y0 = f32::MAX;
    let mut x1 = f32::MIN;
    let mut y1 = f32::MIN;

    for point in polygon {
        let x = point[0] as f32 * page_width;
        let y = point[1] as f32 * page_height;
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }

    Some([x0, y0, x1, y1])
}

fn get_real_page_dims(pdf_path: &Path) -> std::collections::HashMap<usize, (f32, f32)> {
    let mut dims = std::collections::HashMap::new();
    if let Ok(doc) = lopdf::Document::load(pdf_path) {
        for (i, (_, page_id)) in doc.get_pages().into_iter().enumerate() {
            if let Ok(page_dict) = doc.get_dictionary(page_id) {
                if let Ok(rect) = page_dict.get(b"MediaBox").and_then(lopdf::Object::as_array) {
                    if rect.len() == 4 {
                        let get_num = |obj: &lopdf::Object| -> f32 {
                            match obj {
                                lopdf::Object::Real(r) => *r,
                                lopdf::Object::Integer(i) => *i as f32,
                                _ => 0.0,
                            }
                        };
                        let w = (get_num(&rect[2]) - get_num(&rect[0])).abs();
                        let h = (get_num(&rect[3]) - get_num(&rect[1])).abs();
                        dims.insert(i, (w, h));
                    }
                }
            }
        }
    }
    dims
}

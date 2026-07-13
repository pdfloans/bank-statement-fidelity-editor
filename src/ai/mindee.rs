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

/// Top-level response from Mindee's async predict / queue status endpoints.
#[derive(Debug, Deserialize)]
pub struct MindeeResponse {
    pub api_request: Option<MindeeApiRequest>,
    pub document: Option<MindeeDocument>,
}

#[derive(Debug, Deserialize)]
pub struct MindeeApiRequest {
    pub status: String,
    #[serde(default)]
    pub status_code: u16,
    #[serde(default)]
    pub error: Option<MindeeApiError>,
}

#[derive(Debug, Deserialize)]
pub struct MindeeApiError {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub message: String,
}

/// Wraps the enqueue response which gives us a job ID.
#[derive(Debug, Deserialize)]
pub struct MindeeEnqueueResponse {
    pub api_request: MindeeApiRequest,
    pub job: Option<MindeeJob>,
}

#[derive(Debug, Deserialize)]
pub struct MindeeJob {
    pub id: String,
    #[serde(default)]
    pub status: String,
}

/// Queue status response returned during polling.
#[derive(Debug, Deserialize)]
pub struct MindeeQueueResponse {
    pub api_request: MindeeApiRequest,
    pub job: Option<MindeeJobStatus>,
    pub document: Option<MindeeDocument>,
}

#[derive(Debug, Deserialize)]
pub struct MindeeJobStatus {
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub error: Option<MindeeApiError>,
}

#[derive(Debug, Deserialize)]
pub struct MindeeDocument {
    pub id: Option<String>,
    pub inference: Option<MindeeInference>,
}

#[derive(Debug, Deserialize)]
pub struct MindeeInference {
    pub pages: Option<Vec<MindeePage>>,
    pub prediction: Option<MindeePrediction>,
}

/// Per-page inference.
#[derive(Debug, Deserialize)]
pub struct MindeePage {
    pub id: Option<u32>,
    pub prediction: Option<MindeePrediction>,
}

/// The prediction block containing all extracted fields.
#[derive(Debug, Deserialize)]
pub struct MindeePrediction {
    /// Line items (transactions).
    #[serde(default)]
    pub line_items: Vec<MindeeLineItem>,
    /// Opening / starting balance.
    #[serde(default)]
    pub total_tax: Option<MindeeAmountField>,
    /// Account number.
    #[serde(default)]
    pub customer_account_details: Option<MindeeCustomerAccount>,
    // Financial document specific fields
    #[serde(default)]
    pub total_amount: Option<MindeeAmountField>,
    #[serde(default)]
    pub date: Option<MindeeDateField>,
    #[serde(default)]
    pub due_date: Option<MindeeDateField>,
    #[serde(default)]
    pub reference_numbers: Option<Vec<MindeeStringField>>,
    // Bank statement fields (custom model support)
    #[serde(default)]
    pub opening_balance: Option<MindeeAmountField>,
    #[serde(default)]
    pub closing_balance: Option<MindeeAmountField>,
    #[serde(default)]
    pub account_number: Option<MindeeStringField>,
}

/// A single transaction line item.
#[derive(Debug, Deserialize)]
pub struct MindeeLineItem {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub quantity: Option<f64>,
    #[serde(default)]
    pub unit_price: Option<f64>,
    #[serde(default)]
    pub total_amount: Option<f64>,
    #[serde(default)]
    pub tax_amount: Option<f64>,
    #[serde(default)]
    pub tax_rate: Option<f64>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub page_id: Option<u32>,
    #[serde(default)]
    pub polygon: Option<Vec<[f64; 2]>>,
    // Financial-document specific fields that map to bank transactions
    #[serde(default)]
    pub product_code: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MindeeAmountField {
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub polygon: Option<Vec<[f64; 2]>>,
    #[serde(default)]
    pub page_id: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct MindeeDateField {
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub polygon: Option<Vec<[f64; 2]>>,
    #[serde(default)]
    pub page_id: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct MindeeStringField {
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub confidence: f32,
    #[serde(default)]
    pub polygon: Option<Vec<[f64; 2]>>,
    #[serde(default)]
    pub page_id: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct MindeeCustomerAccount {
    #[serde(default)]
    pub account_number: Option<String>,
    #[serde(default)]
    pub iban: Option<String>,
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
const MINDEE_PRODUCT: &str = "mindee/financial_document";

/// The Mindee API v1 base URL.
const MINDEE_API_BASE: &str = "https://api.mindee.net/v1";

pub struct MindeeClient {
    api_key: String,
    http: ClientWithMiddleware,
    /// Product path, e.g. "mindee/financial_document".
    product: String,
    /// Passphrase for the encrypted cache.
    passphrase: String,
}

impl MindeeClient {
    /// Construct from the application config. Returns `MissingConfig` if
    /// `MINDEE_API_KEY` is not set.
    pub fn from_app_config(cfg: &AppConfig) -> Result<Self, MindeeError> {
        let api_key = cfg
            .mindee_api_key
            .clone()
            .filter(|k| !k.is_empty())
            .ok_or(MindeeError::MissingConfig("MINDEE_API_KEY"))?;

        Ok(Self {
            api_key,
            http: crate::app::config::global_http_client(),
            product: MINDEE_PRODUCT.to_string(),
            passphrase: cfg.passphrase.clone(),
        })
    }

    /// Lightweight connectivity / auth test: hits the predict endpoint with
    /// an empty body to confirm the API key is valid.
    pub async fn ping(&self) -> Result<(), MindeeError> {
        let url = format!("{}/products/{}/v1/predict", MINDEE_API_BASE, self.product);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Token {}", self.api_key))
            .send()
            .await?;

        // 401/403 -> bad key. Anything else (even 405) means the key is valid.
        match resp.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(MindeeError::Api(
                resp.status(),
                "Invalid MINDEE_API_KEY - check your key at https://platform.mindee.com/".into(),
            )),
            _ => Ok(()),
        }
    }

    /// Parse a bank statement PDF via Mindee's async flow.
    ///
    /// 1. Checks the local encrypted cache first.
    /// 2. Enqueues the document for async processing.
    /// 3. Polls until completion.
    /// 4. Maps the response into `BankStatement`.
    pub async fn parse_statement(&self, pdf_path: &Path) -> Result<BankStatement, MindeeError> {
        // ─── Cache lookup ───────────────────────────────────────────────
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
                &self.product,
                "v1",
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

        // ─── Read real page dimensions from the PDF ─────────────────────
        let real_dims = get_real_page_dims(pdf_path);

        // ─── Enqueue ────────────────────────────────────────────────────
        let job_id = self.enqueue(pdf_path).await?;
        tracing::info!("[mindee] enqueued job {}", job_id);

        // ─── Poll ───────────────────────────────────────────────────────
        let response = self.poll_until_complete(&job_id).await?;

        // ─── Parse ──────────────────────────────────────────────────────
        let stmt = parse_mindee_response(&response, &real_dims)?;

        // ─── Cache write ────────────────────────────────────────────────
        if let (Some(c), Some(k)) = (cache.as_ref(), cache_key.as_ref()) {
            if let Err(e) = c.put(k, &stmt) {
                tracing::warn!("[mindee] cache write failed: {}", e);
            }
        }

        Ok(stmt)
    }

    /// Upload the PDF and enqueue for async processing.
    async fn enqueue(&self, pdf_path: &Path) -> Result<String, MindeeError> {
        let url = format!(
            "{}/products/{}/v1/predict_async",
            MINDEE_API_BASE, self.product
        );

        let pdf_bytes = tokio::fs::read(pdf_path).await?;
        let filename = pdf_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        let mut attempts = 0;
        let max_attempts = 4;
        loop {
            attempts += 1;
            let part = reqwest::multipart::Part::bytes(pdf_bytes.clone())
                .file_name(filename.clone())
                .mime_str("application/pdf")
                .unwrap_or_else(|_| reqwest::multipart::Part::bytes(Vec::new()));
            let form = reqwest::multipart::Form::new().part("document", part);

            match self
                .http
                .post(&url)
                .header("Authorization", format!("Token {}", self.api_key))
                .multipart(form)
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_server_error() || status == 429 {
                        if attempts < max_attempts {
                            let jitter = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.subsec_millis() as u64 % 500)
                                .unwrap_or(250);
                            let delay = std::time::Duration::from_millis(
                                500 * (1 << (attempts - 1)) + jitter,
                            );
                            tracing::warn!(
                                "[mindee] enqueue got {}, retrying in {:?}",
                                status,
                                delay
                            );
                            tokio::time::sleep(delay).await;
                            continue;
                        }
                    }
                    if !status.is_success() {
                        let text = resp.text().await.unwrap_or_default();
                        return Err(MindeeError::Api(status, text));
                    }
                    let enqueue_resp: MindeeEnqueueResponse = resp.json().await?;
                    let job = enqueue_resp.job.ok_or(MindeeError::ExtractionFailed(
                        "No job in enqueue response".into(),
                    ))?;
                    return Ok(job.id);
                }
                Err(e) => {
                    if attempts < max_attempts {
                        let jitter = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.subsec_millis() as u64 % 500)
                            .unwrap_or(250);
                        let delay =
                            std::time::Duration::from_millis(500 * (1 << (attempts - 1)) + jitter);
                        tracing::warn!("[mindee] network error {}, retrying in {:?}", e, delay);
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    return Err(e.into());
                }
            }
        }
    }

    /// Poll the queue endpoint until the job completes or fails.
    async fn poll_until_complete(&self, job_id: &str) -> Result<MindeeQueueResponse, MindeeError> {
        let url = format!("{MINDEE_API_BASE}/documents/queue/{job_id}");
        let mut delay_ms = INITIAL_POLL_DELAY_MS;

        for attempt in 1..=MAX_POLL_ATTEMPTS {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

            let resp = match self
                .http
                .get(&url)
                .header("Authorization", format!("Token {}", self.api_key))
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("[mindee] poll network error: {}", e);
                    let jitter = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.subsec_millis() as u64 % 250)
                        .unwrap_or(100);
                    delay_ms = (delay_ms * 3 / 2 + jitter).min(MAX_POLL_DELAY_MS);
                    continue;
                }
            };

            if !resp.status().is_success() {
                let status = resp.status();
                if (status.is_server_error() || status == 429) && attempt < MAX_POLL_ATTEMPTS {
                    let jitter = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.subsec_millis() as u64 % 250)
                        .unwrap_or(100);
                    tracing::warn!(
                        "[mindee] poll attempt {} got {}, retrying...",
                        attempt,
                        status
                    );
                    delay_ms = (delay_ms * 2 + jitter).min(MAX_POLL_DELAY_MS);
                    continue;
                }
                let text = resp.text().await.unwrap_or_default();
                return Err(MindeeError::Api(status, text));
            }

            let queue_resp: MindeeQueueResponse = resp.json().await?;

            if let Some(ref job_status) = queue_resp.job {
                match job_status.status.as_str() {
                    "completed" => {
                        tracing::info!(
                            "[mindee] job {} completed after {} poll(s)",
                            job_id,
                            attempt
                        );
                        return Ok(queue_resp);
                    }
                    "failed" => {
                        let msg = job_status
                            .error
                            .as_ref()
                            .map(|e| e.message.clone())
                            .unwrap_or_else(|| "Unknown failure".into());
                        return Err(MindeeError::ExtractionFailed(msg));
                    }
                    // "waiting" | "processing" -> keep polling
                    _ => {
                        tracing::debug!("[mindee] poll {}: status={}", attempt, job_status.status);
                    }
                }
            }

            let jitter = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_millis() as u64 % 250)
                .unwrap_or(100);
            delay_ms = (delay_ms * 3 / 2 + jitter).min(MAX_POLL_DELAY_MS);
        }

        Err(MindeeError::PollTimeout(MAX_POLL_ATTEMPTS))
    }
}

// ---------------------------------------------------------------------------
// Response -> BankStatement mapping
// ---------------------------------------------------------------------------

/// Convert a Mindee Financial Document response into the project's
/// `BankStatement` type. Handles both the document-level prediction and
/// per-page predictions, merging line items from all pages.
pub fn parse_mindee_response(
    response: &MindeeQueueResponse,
    real_page_dims: &std::collections::HashMap<usize, (f32, f32)>,
) -> Result<BankStatement, MindeeError> {
    let doc = response
        .document
        .as_ref()
        .ok_or_else(|| MindeeError::ExtractionFailed("No document in response".into()))?;
    let inference = doc
        .inference
        .as_ref()
        .ok_or_else(|| MindeeError::ExtractionFailed("No inference in response".into()))?;

    // Page count
    let total_pages = inference.pages.as_ref().map_or(0, |p| p.len());

    // ─── Balances and account number from the prediction ────────────
    let prediction = inference.prediction.as_ref();

    let opening_balance = prediction
        .and_then(|p| p.opening_balance.as_ref())
        .and_then(|f| f.value)
        .map(f64_to_dec)
        .unwrap_or(Decimal::ZERO);

    let closing_balance = prediction
        .and_then(|p| p.closing_balance.as_ref())
        .and_then(|f| f.value)
        .map(f64_to_dec)
        .unwrap_or(Decimal::ZERO);

    let account_number = prediction
        .and_then(|p| p.account_number.as_ref())
        .and_then(|f| f.value.clone())
        .or_else(|| {
            prediction
                .and_then(|p| p.customer_account_details.as_ref())
                .and_then(|a| a.account_number.clone())
        })
        .filter(|s| !s.is_empty());

    // ─── Line items -> Transactions ──────────────────────────────────
    let line_items = prediction.map(|p| p.line_items.as_slice()).unwrap_or(&[]);

    let mut transactions = Vec::with_capacity(line_items.len());

    for (idx, item) in line_items.iter().enumerate() {
        let page_idx = item.page_id.map(|p| p as usize).unwrap_or(0);
        let (page_w, page_h) = real_page_dims
            .get(&page_idx)
            .copied()
            .unwrap_or((612.0, 792.0)); // US Letter default

        let description = item.description.clone().unwrap_or_default();
        let date = item.date.clone().unwrap_or_default();

        // Mindee's financial_document model uses `total_amount` for each
        // line item. We heuristically classify as debit (money in) or
        // credit (money out) based on sign; if not distinguishable, store
        // as debit (inflow) to match the codebase convention.
        let amount = item.total_amount;
        let (debit, credit) = match amount {
            Some(v) if v < 0.0 => (None, Some(f64_to_dec(v.abs()))),
            Some(v) => (Some(f64_to_dec(v)), None),
            None => (None, None),
        };

        // Row-level bbox from the line item polygon
        let row_bbox = item
            .polygon
            .as_ref()
            .and_then(|poly| polygon_to_bbox(poly, page_w, page_h));

        // Per-field bboxes: Mindee doesn't provide separate polygons per
        // sub-field within a line item - only the row-level polygon. We
        // leave sub-field bboxes as None so the editor falls back to the
        // row-level bbox (consistent with DocAI when properties lack anchors).
        let field_bboxes = FieldBboxes::default();

        // Skip rows with no financial data
        if debit.is_none() && credit.is_none() && description.is_empty() {
            continue;
        }

        transactions.push(Transaction {
            page: page_idx,
            line_on_page: idx,
            date,
            raw_text: description,
            debit,
            credit,
            running_balance: None, // Mindee doesn't provide running balances
            bbox: row_bbox,
            field_bboxes,
            provenance: Provenance::Mindee {
                confidence: item.confidence,
            },
        });
    }

    Ok(BankStatement {
        total_pages,
        transactions,
        opening_balance,
        closing_balance,
        account_number,
    })
}

/// Convert a Mindee polygon (array of [x, y] pairs, normalized 0.0-1.0)
/// to a PDF-points bbox `[x0, y0, x1, y1]`.
///
/// Mindee polygons are typically 4 corners (top-left, top-right,
/// bottom-right, bottom-left) but we handle any number of points by
/// computing the axis-aligned bounding rectangle.
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

/// Read the real page dimensions from a PDF file via `lopdf`, returning a
/// map of page index -> (width_pts, height_pts). Mirrors the helper in
/// `document_ai.rs`.
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polygon_to_bbox_converts_normalized_to_points() {
        // 4-corner polygon on a US Letter page (612 × 792 points)
        let polygon = vec![[0.10, 0.30], [0.90, 0.30], [0.90, 0.34], [0.10, 0.34]];
        let bbox = polygon_to_bbox(&polygon, 612.0, 792.0).unwrap();
        // x: 0.10*612=61.2, 0.90*612=550.8
        // y: 0.30*792=237.6, 0.34*792=269.28
        assert!((bbox[0] - 61.2).abs() < 0.1, "x0={}", bbox[0]);
        assert!((bbox[1] - 237.6).abs() < 0.1, "y0={}", bbox[1]);
        assert!((bbox[2] - 550.8).abs() < 0.1, "x1={}", bbox[2]);
        assert!((bbox[3] - 269.28).abs() < 0.1, "y1={}", bbox[3]);
    }

    #[test]
    fn polygon_to_bbox_returns_none_for_empty() {
        assert!(polygon_to_bbox(&[], 612.0, 792.0).is_none());
    }

    #[test]
    fn polygon_to_bbox_handles_single_point() {
        let polygon = vec![[0.5, 0.5]];
        let bbox = polygon_to_bbox(&polygon, 100.0, 100.0).unwrap();
        assert!((bbox[0] - 50.0).abs() < 0.01);
        assert!((bbox[1] - 50.0).abs() < 0.01);
        assert!((bbox[2] - 50.0).abs() < 0.01);
        assert!((bbox[3] - 50.0).abs() < 0.01);
    }

    #[test]
    fn parse_mindee_response_extracts_transactions() {
        let json_str = r#"{
            "api_request": { "status": "success", "status_code": 200 },
            "job": { "id": "test-job-123", "status": "completed" },
            "document": {
                "id": "doc-456",
                "inference": {
                    "pages": [
                        { "id": 0, "prediction": { "line_items": [] } }
                    ],
                    "prediction": {
                        "line_items": [
                            {
                                "description": "Interest Paid",
                                "total_amount": 242.83,
                                "date": "09/02/2026",
                                "confidence": 0.92,
                                "page_id": 0,
                                "polygon": [
                                    [0.10, 0.30],
                                    [0.90, 0.30],
                                    [0.90, 0.34],
                                    [0.10, 0.34]
                                ]
                            },
                            {
                                "description": "ATM Withdrawal",
                                "total_amount": -50.00,
                                "date": "10/02/2026",
                                "confidence": 0.88,
                                "page_id": 0,
                                "polygon": [
                                    [0.10, 0.36],
                                    [0.90, 0.36],
                                    [0.90, 0.40],
                                    [0.10, 0.40]
                                ]
                            }
                        ],
                        "opening_balance": { "value": 1000.00, "confidence": 0.95 },
                        "closing_balance": { "value": 1192.83, "confidence": 0.95 },
                        "account_number": { "value": "807466413", "confidence": 0.99 }
                    }
                }
            }
        }"#;

        let response: MindeeQueueResponse = serde_json::from_str(json_str).unwrap();
        let dims = std::collections::HashMap::new(); // No real dims -> uses defaults

        let stmt = parse_mindee_response(&response, &dims).unwrap();

        assert_eq!(stmt.total_pages, 1);
        assert_eq!(stmt.transactions.len(), 2);
        assert_eq!(stmt.opening_balance, f64_to_dec(1000.00));
        assert_eq!(stmt.closing_balance, f64_to_dec(1192.83));
        assert_eq!(stmt.account_number.as_deref(), Some("807466413"));

        // First transaction: positive amount -> debit (money in)
        let tx0 = &stmt.transactions[0];
        assert_eq!(tx0.date, "09/02/2026");
        assert_eq!(tx0.raw_text, "Interest Paid");
        assert_eq!(tx0.debit, Some(f64_to_dec(242.83)));
        assert_eq!(tx0.credit, None);
        assert!(tx0.bbox.is_some());
        match tx0.provenance {
            Provenance::Mindee { confidence } => assert!((confidence - 0.92).abs() < 0.01),
            _ => panic!("Expected Mindee provenance"),
        }

        // Second transaction: negative amount -> credit (money out)
        let tx1 = &stmt.transactions[1];
        assert_eq!(tx1.raw_text, "ATM Withdrawal");
        assert_eq!(tx1.debit, None);
        assert_eq!(tx1.credit, Some(f64_to_dec(50.00)));
    }

    #[test]
    fn parse_mindee_response_handles_empty_line_items() {
        let json_str = r#"{
            "api_request": { "status": "success", "status_code": 200 },
            "job": { "id": "test-job", "status": "completed" },
            "document": {
                "id": "doc",
                "inference": {
                    "pages": [{ "id": 0, "prediction": { "line_items": [] } }],
                    "prediction": {
                        "line_items": [],
                        "opening_balance": { "value": 500.00, "confidence": 0.90 },
                        "closing_balance": { "value": 500.00, "confidence": 0.90 }
                    }
                }
            }
        }"#;

        let response: MindeeQueueResponse = serde_json::from_str(json_str).unwrap();
        let dims = std::collections::HashMap::new();
        let stmt = parse_mindee_response(&response, &dims).unwrap();

        assert_eq!(stmt.transactions.len(), 0);
        assert_eq!(stmt.opening_balance, f64_to_dec(500.00));
        assert_eq!(stmt.closing_balance, f64_to_dec(500.00));
    }

    #[test]
    fn parse_mindee_response_with_real_page_dims() {
        let json_str = r#"{
            "api_request": { "status": "success", "status_code": 200 },
            "job": { "id": "j", "status": "completed" },
            "document": {
                "id": "d",
                "inference": {
                    "pages": [{ "id": 0, "prediction": { "line_items": [] } }],
                    "prediction": {
                        "line_items": [
                            {
                                "description": "Test",
                                "total_amount": 100.0,
                                "confidence": 0.9,
                                "page_id": 0,
                                "polygon": [
                                    [0.0, 0.0],
                                    [1.0, 0.0],
                                    [1.0, 0.1],
                                    [0.0, 0.1]
                                ]
                            }
                        ]
                    }
                }
            }
        }"#;

        let response: MindeeQueueResponse = serde_json::from_str(json_str).unwrap();
        let mut dims = std::collections::HashMap::new();
        dims.insert(0, (595.0_f32, 842.0_f32)); // A4

        let stmt = parse_mindee_response(&response, &dims).unwrap();
        let tx = &stmt.transactions[0];
        let bbox = tx.bbox.unwrap();

        // With A4 dimensions: x: 0.0*595=0, 1.0*595=595; y: 0.0*842=0, 0.1*842=84.2
        assert!((bbox[0] - 0.0).abs() < 0.1);
        assert!((bbox[1] - 0.0).abs() < 0.1);
        assert!((bbox[2] - 595.0).abs() < 0.1);
        assert!((bbox[3] - 84.2).abs() < 0.1);
    }

    #[test]
    fn from_app_config_requires_api_key() {
        let cfg = AppConfig {
            mindee_api_key: None,
            passphrase: "x".repeat(20),
            ..AppConfig::default()
        };
        let res = MindeeClient::from_app_config(&cfg);
        assert!(matches!(res, Err(MindeeError::MissingConfig(_))));
    }

    #[test]
    fn from_app_config_rejects_empty_key() {
        let cfg = AppConfig {
            mindee_api_key: Some(String::new()),
            passphrase: "x".repeat(20),
            ..AppConfig::default()
        };
        let res = MindeeClient::from_app_config(&cfg);
        assert!(matches!(res, Err(MindeeError::MissingConfig(_))));
    }

    #[test]
    fn from_app_config_succeeds_with_key() {
        let cfg = AppConfig {
            mindee_api_key: Some("test_key_12345".to_string()),
            passphrase: "x".repeat(20),
            ..AppConfig::default()
        };
        let client = MindeeClient::from_app_config(&cfg).unwrap();
        assert_eq!(client.api_key, "test_key_12345");
        assert_eq!(client.product, MINDEE_PRODUCT);
    }

    #[test]
    fn parse_mindee_response_skips_empty_rows() {
        let json_str = r#"{
            "api_request": { "status": "success", "status_code": 200 },
            "job": { "id": "j", "status": "completed" },
            "document": {
                "id": "d",
                "inference": {
                    "pages": [],
                    "prediction": {
                        "line_items": [
                            {
                                "description": "",
                                "confidence": 0.5,
                                "page_id": 0
                            },
                            {
                                "description": "Real Transaction",
                                "total_amount": 99.99,
                                "confidence": 0.95,
                                "page_id": 0
                            }
                        ]
                    }
                }
            }
        }"#;

        let response: MindeeQueueResponse = serde_json::from_str(json_str).unwrap();
        let dims = std::collections::HashMap::new();
        let stmt = parse_mindee_response(&response, &dims).unwrap();

        // Empty row (no amount, no description) should be skipped
        assert_eq!(stmt.transactions.len(), 1);
        assert_eq!(stmt.transactions[0].raw_text, "Real Transaction");
    }

    #[test]
    fn parse_mindee_response_customer_account_fallback() {
        // When `account_number` is absent but `customer_account_details` is present
        let json_str = r#"{
            "api_request": { "status": "success", "status_code": 200 },
            "job": { "id": "j", "status": "completed" },
            "document": {
                "id": "d",
                "inference": {
                    "pages": [],
                    "prediction": {
                        "line_items": [],
                        "customer_account_details": {
                            "account_number": "12345678",
                            "iban": "AU12345678901234"
                        }
                    }
                }
            }
        }"#;

        let response: MindeeQueueResponse = serde_json::from_str(json_str).unwrap();
        let dims = std::collections::HashMap::new();
        let stmt = parse_mindee_response(&response, &dims).unwrap();
        assert_eq!(stmt.account_number.as_deref(), Some("12345678"));
    }
}

use crate::ai::document_ai::BankStatement;
use crate::app::config::AppConfig;
use reqwest::StatusCode;
use reqwest_middleware::ClientWithMiddleware;
use rust_decimal::Decimal;
use serde::Deserialize;
use std::path::Path;

const LLAMAPARSE_API_BASE: &str = "https://api.cloud.llamaindex.ai/api/parsing";
const INITIAL_POLL_DELAY_MS: u64 = 2000;
const MAX_POLL_ATTEMPTS: usize = 30;

#[derive(Debug, thiserror::Error)]
pub enum LlamaParseError {
    #[error("Missing Configuration: {0}")]
    MissingConfig(&'static str),
    #[error("I/O Error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Network Error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Middleware Error: {0}")]
    Middleware(#[from] reqwest_middleware::Error),
    #[error("API Error (HTTP {0}): {1}")]
    Api(StatusCode, String),
    #[error("Extraction Failed: {0}")]
    ExtractionFailed(String),
}

#[derive(Deserialize)]
struct UploadResponse {
    id: String,
}

#[derive(Deserialize)]
struct JobStatusResponse {
    status: String,
}

#[derive(Deserialize)]
struct MarkdownResponse {
    markdown: String,
}

pub struct LlamaParseClient {
    http: ClientWithMiddleware,
    api_key: String,
    passphrase: Option<String>,
}

impl LlamaParseClient {
    pub fn from_app_config(cfg: &AppConfig) -> Result<Self, LlamaParseError> {
        let api_key = cfg
            .llamaparse_api_key
            .clone()
            .filter(|s| !s.is_empty())
            .ok_or(LlamaParseError::MissingConfig(
                "LLAMAPARSE_API_KEY is not set",
            ))?;

        let http = crate::app::config::global_http_client();

        Ok(Self {
            http,
            api_key,
            passphrase: if cfg.passphrase.is_empty() {
                None
            } else {
                Some(cfg.passphrase.clone())
            },
        })
    }

    pub async fn parse_statement(&self, pdf_path: &Path) -> Result<BankStatement, LlamaParseError> {
        let cache = match crate::ai::docai_cache::DocAiCache::open_default(
            self.passphrase.as_deref().unwrap_or_default(),
        ) {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!("[llamaparse] Could not open cache: {}", e);
                None
            }
        };

        let cache_key = cache.as_ref().and_then(|_c| {
            crate::ai::docai_cache::DocAiCache::make_key(
                pdf_path,
                "llamaparse",
                "global",
                "default",
                "v1",
            )
            .ok()
        });

        if let (Some(c), Some(h)) = (cache.as_ref(), cache_key.as_ref()) {
            if let Some(cached_stmt) = c.get(h) {
                tracing::info!("[llamaparse] Found cached parsed statement for this file");
                return Ok(cached_stmt);
            }
        }

        let job_id = self.upload_document(pdf_path).await?;
        self.poll_until_complete(&job_id).await?;
        let markdown = self.fetch_markdown(&job_id).await?;

        let stmt = self.parse_markdown_to_statement(&markdown)?;

        if let (Some(ref c), Some(ref h)) = (&cache, &cache_key) {
            if let Err(e) = c.put(h, &stmt) {
                tracing::warn!("[llamaparse] Failed to cache statement: {}", e);
            }
        }

        Ok(stmt)
    }

    async fn upload_document(&self, pdf_path: &Path) -> Result<String, LlamaParseError> {
        let pdf_bytes = std::fs::read(pdf_path)?;
        let filename = pdf_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        let part = reqwest::multipart::Part::bytes(pdf_bytes)
            .file_name(filename)
            .mime_str("application/pdf")
            .unwrap_or_else(|_| reqwest::multipart::Part::bytes(Vec::new()));

        let form = reqwest::multipart::Form::new().part("file", part);

        let url = format!("{LLAMAPARSE_API_BASE}/upload");
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(LlamaParseError::Api(
                resp.status(),
                resp.text().await.unwrap_or_default(),
            ));
        }

        let upload_resp: UploadResponse = resp.json().await.map_err(|e| {
            LlamaParseError::ExtractionFailed(format!("Failed to parse upload response: {}", e))
        })?;

        Ok(upload_resp.id)
    }

    async fn poll_until_complete(&self, job_id: &str) -> Result<(), LlamaParseError> {
        let url = format!("{LLAMAPARSE_API_BASE}/job/{job_id}");
        let mut delay_ms = INITIAL_POLL_DELAY_MS;

        for attempt in 1..=MAX_POLL_ATTEMPTS {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

            let resp = self
                .http
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .send()
                .await?;

            if !resp.status().is_success() {
                return Err(LlamaParseError::Api(
                    resp.status(),
                    resp.text().await.unwrap_or_default(),
                ));
            }

            let status_resp: JobStatusResponse = resp.json().await.map_err(|e| {
                LlamaParseError::ExtractionFailed(format!("Failed to parse job status: {}", e))
            })?;

            match status_resp.status.as_str() {
                "SUCCESS" => return Ok(()),
                "ERROR" | "FAILED" => {
                    return Err(LlamaParseError::ExtractionFailed(
                        "LlamaParse job failed on server".into(),
                    ))
                }
                _ => {
                    tracing::debug!("[llamaparse] poll {attempt}: status={}", status_resp.status);
                }
            }
            delay_ms = (delay_ms * 2).min(10000);
        }

        Err(LlamaParseError::ExtractionFailed(
            "Timed out waiting for LlamaParse job to complete".into(),
        ))
    }

    async fn fetch_markdown(&self, job_id: &str) -> Result<String, LlamaParseError> {
        let url = format!("{LLAMAPARSE_API_BASE}/job/{job_id}/result/markdown");

        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(LlamaParseError::Api(
                resp.status(),
                resp.text().await.unwrap_or_default(),
            ));
        }

        let md_resp: MarkdownResponse = resp.json().await.map_err(|e| {
            LlamaParseError::ExtractionFailed(format!("Failed to parse markdown response: {}", e))
        })?;

        Ok(md_resp.markdown)
    }

    fn parse_markdown_to_statement(
        &self,
        _markdown: &str,
    ) -> Result<BankStatement, LlamaParseError> {
        // TODO: Implement a robust parser that extracts tables from Markdown.
        // For now, we return a fallback empty statement because generic markdown
        // parsing without an LLM is error-prone. In a full implementation, we would
        // either use regex to find table boundaries and map columns, or request
        // structured JSON from LlamaParse's premium mode.
        tracing::warn!(
            "[llamaparse] Markdown parsing is partially implemented. Returning empty statement."
        );
        Ok(BankStatement {
            total_pages: 1,
            transactions: vec![],
            opening_balance: Decimal::ZERO,
            closing_balance: Decimal::ZERO,
            account_number: None,
        })
    }
}

//! Google Document AI client.
//!
//! Auth strategy (in priority order):
//!  1. **Primary (Beta):** API key via the `v1beta3` endpoint with `?key=...`.
//!  2. **Fallback A (ADC):** Application Default Credentials — the file written
//!     by `gcloud auth application-default login`. We swap the cached
//!     `refresh_token` for a fresh access token at the OAuth2 endpoint.
//!  3. **Fallback B (legacy SA):** Service-account JWT signed locally with the
//!     RSA private key from a service-account JSON.
//!
//! Auth-class errors (401/403) on tier 1 cascade through tiers 2 and 3 in
//! turn. Network errors and non-auth API errors propagate immediately.
//! The response shape is identical across endpoints, so the parser is shared.

use base64::{engine::general_purpose::STANDARD as Base64Standard, Engine};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

use crate::app::config::{AppConfig, DocumentAiConfig};
use crate::engine::model::{f64_to_dec, FieldBboxes, Provenance, Transaction};

#[derive(thiserror::Error, Debug)]
pub enum DocAiError {
    #[error("Missing Configuration: {0}")]
    MissingConfig(&'static str),
    #[error("I/O Error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JWT Error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    #[error("Auth Error (HTTP {0}): {1}")]
    Auth(StatusCode, String),
    #[error("Network Error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Parse Error: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("API Error (HTTP {0}): {1}")]
    Api(StatusCode, String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BankStatement {
    pub total_pages: usize,
    pub transactions: Vec<Transaction>,
    pub opening_balance: Decimal,
    pub closing_balance: Decimal,
    pub account_number: Option<String>,
}

fn xml_escape(s: &str) -> String {
    s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace("\"", "&quot;").replace("'", "&apos;")
}

impl BankStatement {
    /// Export the BankStatement to an OFX format string for accounting software integration.
    pub fn export_to_ofx(&self) -> String {
        let mut ofx = String::new();
        let current_time = chrono::Utc::now().format("%Y%m%d%H%M%S.000[%z:GMT]").to_string();
        
        ofx.push_str("OFXHEADER:100\n");
        ofx.push_str("DATA:OFXSGML\n");
        ofx.push_str("VERSION:102\n");
        ofx.push_str("SECURITY:NONE\n");
        ofx.push_str("ENCODING:USASCII\n");
        ofx.push_str("CHARSET:1252\n");
        ofx.push_str("COMPRESSION:NONE\n");
        ofx.push_str("OLDFILEUID:NONE\n");
        ofx.push_str("NEWFILEUID:NONE\n\n");
        
        ofx.push_str("<OFX>\n");
        ofx.push_str("  <SIGNONMSGSRSV1>\n");
        ofx.push_str("    <SONRS>\n");
        ofx.push_str("      <STATUS><CODE>0<SEVERITY>INFO</STATUS>\n");
        ofx.push_str(&format!("      <DTSERVER>{}\n", current_time));
        ofx.push_str("      <LANGUAGE>ENG\n");
        ofx.push_str("    </SONRS>\n");
        ofx.push_str("  </SIGNONMSGSRSV1>\n");
        
        ofx.push_str("  <BANKMSGSRSV1>\n");
        ofx.push_str("    <STMTTRNRS>\n");
        ofx.push_str("      <TRNUID>1\n");
        ofx.push_str("      <STATUS><CODE>0<SEVERITY>INFO</STATUS>\n");
        ofx.push_str("      <STMTRS>\n");
        ofx.push_str("        <CURDEF>USD\n");
        ofx.push_str("        <BANKACCTFROM>\n");
        ofx.push_str("          <BANKID>UNKNOWN\n");
        ofx.push_str(&format!("          <ACCTID>{}\n", self.account_number.clone().unwrap_or_else(|| "UNKNOWN".to_string())));
        ofx.push_str("          <ACCTTYPE>CHECKING\n");
        ofx.push_str("        </BANKACCTFROM>\n");
        ofx.push_str("        <BANKTRANLIST>\n");
        
        for (i, tx) in self.transactions.iter().enumerate() {
            let trntype = if tx.delta_in() > rust_decimal::Decimal::ZERO { "CREDIT" } else { "DEBIT" };
            let amount = tx.net_delta();
            
            // Extract digits only for basic date formatting YYYYMMDD
            let dtposted = tx.date.chars().filter(|c| c.is_ascii_digit()).collect::<String>();
            let dtposted = if dtposted.is_empty() { current_time.clone() } else { dtposted };
            
            ofx.push_str("          <STMTTRN>\n");
            ofx.push_str(&format!("            <TRNTYPE>{}\n", trntype));
            ofx.push_str(&format!("            <DTPOSTED>{}\n", dtposted));
            ofx.push_str(&format!("            <TRNAMT>{}\n", amount));
            ofx.push_str(&format!("            <FITID>{}\n", i + 1));
            ofx.push_str(&format!("            <NAME>{}\n", xml_escape(&tx.raw_text)));
            ofx.push_str("          </STMTTRN>\n");
        }
        
        ofx.push_str("        </BANKTRANLIST>\n");
        ofx.push_str("        <LEDGERBAL>\n");
        ofx.push_str(&format!("          <BALAMT>{}\n", self.closing_balance));
        ofx.push_str(&format!("          <DTASOF>{}\n", current_time));
        ofx.push_str("        </LEDGERBAL>\n");
        ofx.push_str("      </STMTRS>\n");
        ofx.push_str("    </STMTTRNRS>\n");
        ofx.push_str("  </BANKMSGSRSV1>\n");
        ofx.push_str("</OFX>\n");
        
        ofx
    }
}

#[derive(Serialize)]
struct JwtClaims {
    iss: String,
    scope: String,
    aud: String,
    iat: u64,
    exp: u64,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Metrics {
    #[serde(rename = "f1Score")]
    pub f1_score: Option<f32>,
    pub precision: Option<f32>,
    pub recall: Option<f32>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct EvaluationMetrics {
    #[serde(rename = "allEntitiesMetrics")]
    pub all_entities_metrics: Option<Metrics>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ProcessorVersion {
    pub name: String,
    pub state: String,
    pub evaluation: Option<EvaluationMetrics>,
}

#[derive(Deserialize, Debug)]
pub struct ListProcessorVersionsResponse {
    #[serde(rename = "processorVersions")]
    pub processor_versions: Option<Vec<ProcessorVersion>>,
}

#[derive(Serialize, Debug)]
pub struct GcsPrefix {
    #[serde(rename = "gcsUriPrefix")]
    pub gcs_uri_prefix: String,
}

#[derive(Serialize, Debug)]
pub struct BatchInputDocuments {
    #[serde(rename = "gcsPrefix")]
    pub gcs_prefix: GcsPrefix,
}

#[derive(Serialize, Debug)]
pub struct GcsOutputConfig {
    #[serde(rename = "gcsUri")]
    pub gcs_uri: String,
}

#[derive(Serialize, Debug)]
pub struct DocumentOutputConfig {
    #[serde(rename = "gcsOutputConfig")]
    pub gcs_output_config: GcsOutputConfig,
}

#[derive(Serialize, Debug)]
pub struct BatchProcessRequest {
    #[serde(rename = "inputDocuments")]
    pub input_documents: BatchInputDocuments,
    #[serde(rename = "documentOutputConfig")]
    pub document_output_config: DocumentOutputConfig,
}

#[derive(Deserialize, Debug)]
pub struct Operation {
    pub name: String,
    pub done: Option<bool>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
}

#[derive(Debug)]
struct CachedToken {
    access_token: String,
    expires_at: u64,
}

pub struct DocumentAiClient {
    config: DocumentAiConfig,
    token_cache: Mutex<Option<CachedToken>>,
    http: Client,
}

impl DocumentAiClient {
    pub fn from_app_config(cfg: &AppConfig) -> Result<Self, DocAiError> {
        let doc_ai = cfg
            .document_ai
            .clone()
            .ok_or(DocAiError::MissingConfig("document_ai"))?;
        // Require *some* form of credential.
        if doc_ai.api_key.is_empty()
            && doc_ai.service_account_path.is_empty()
            && doc_ai.adc_path.is_empty()
        {
            return Err(DocAiError::MissingConfig(
                "DOCUMENT_AI_API_KEY, ADC, or GOOGLE_APPLICATION_CREDENTIALS",
            ));
        }
        Ok(Self {
            config: doc_ai,
            token_cache: Mutex::new(None),
            http: Client::new(),
        })
    }

    fn process_url_v1beta3(&self, version_id: Option<&str>, location: &str) -> String {
        match version_id {
            Some(v) => format!(
                "https://{}-documentai.googleapis.com/v1beta3/projects/{}/locations/{}/processors/{}/processorVersions/{}:process",
                location, self.config.project_id, location, self.config.processor_id, v
            ),
            None => format!(
                "https://{}-documentai.googleapis.com/v1beta3/projects/{}/locations/{}/processors/{}:process",
                location, self.config.project_id, location, self.config.processor_id
            )
        }
    }

    fn process_url_v1(&self, version_id: Option<&str>, location: &str) -> String {
        match version_id {
            Some(v) => format!(
                "https://{}-documentai.googleapis.com/v1/projects/{}/locations/{}/processors/{}/processorVersions/{}:process",
                location, self.config.project_id, location, self.config.processor_id, v
            ),
            None => format!(
                "https://{}-documentai.googleapis.com/v1/projects/{}/locations/{}/processors/{}:process",
                location, self.config.project_id, location, self.config.processor_id
            )
        }
    }

    fn process_url_specialized(&self, processor_id: &str, version_id: Option<&str>, location: &str) -> String {
        match version_id {
            Some(v) => format!(
                "https://{}-documentai.googleapis.com/v1/projects/{}/locations/{}/processors/{}/processorVersions/{}:process",
                location, self.config.project_id, location, processor_id, v
            ),
            None => format!(
                "https://{}-documentai.googleapis.com/v1/projects/{}/locations/{}/processors/{}:process",
                location, self.config.project_id, location, processor_id
            )
        }
    }

    fn batch_process_url(&self, version_id: Option<&str>, location: &str) -> String {
        match version_id {
            Some(v) => format!(
                "https://{}-documentai.googleapis.com/v1/projects/{}/locations/{}/processors/{}/processorVersions/{}:batchProcess",
                location, self.config.project_id, location, self.config.processor_id, v
            ),
            None => format!(
                "https://{}-documentai.googleapis.com/v1/projects/{}/locations/{}/processors/{}:batchProcess",
                location, self.config.project_id, location, self.config.processor_id
            )
        }
    }

    fn list_versions_url(&self, location: &str) -> String {
        format!(
            "https://{}-documentai.googleapis.com/v1/projects/{}/locations/{}/processors/{}/processorVersions",
            location, self.config.project_id, location, self.config.processor_id
        )
    }

    /// Wraps a Document AI request to catch 429/503 errors and silently failover
    /// between `us` and `eu` regions before giving up.
    async fn execute_with_failover<F>(
        &self,
        url_builder: F,
        body: Option<&serde_json::Value>,
        access_token: Option<&str>,
        api_key: Option<&str>,
    ) -> Result<reqwest::Response, DocAiError>
    where
        F: Fn(&str) -> String,
    {
        let locations = if self.config.location == "us" {
            vec!["us", "eu"]
        } else if self.config.location == "eu" {
            vec!["eu", "us"]
        } else {
            vec![self.config.location.as_str()]
        };

        let mut last_status = reqwest::StatusCode::OK;
        let mut last_text = String::new();

        for location in locations {
            let base_url = url_builder(location);
            let url = if let Some(key) = api_key {
                format!("{}?key={}", base_url, key)
            } else {
                base_url
            };

            let mut req = if let Some(b) = body {
                self.http.post(&url).json(b)
            } else {
                self.http.get(&url)
            };

            if let Some(tok) = access_token {
                req = req.bearer_auth(tok);
            }

            let response = req.send().await?;
            if response.status().is_success() {
                return Ok(response);
            }

            last_status = response.status();
            last_text = response.text().await.unwrap_or_default();

            if last_status != reqwest::StatusCode::TOO_MANY_REQUESTS && last_status != reqwest::StatusCode::SERVICE_UNAVAILABLE {
                return Err(DocAiError::Api(last_status, last_text));
            }

            tracing::warn!("[doc_ai] {} in location {}, falling back...", last_status, location);
        }

        Err(DocAiError::Api(last_status, last_text))
    }

    pub async fn list_processor_versions(&self) -> Result<Vec<ProcessorVersion>, DocAiError> {
        let access_token = self.get_access_token().await?;
        
        let response = self.execute_with_failover(
            |loc| self.list_versions_url(loc),
            None,
            Some(&access_token),
            None
        ).await?;

        #[derive(serde::Deserialize)]
        struct ListResponse {
            #[serde(rename = "processorVersions")]
            processor_versions: Option<Vec<ProcessorVersion>>,
        }
        let list_res: ListResponse = response.json().await?;
        Ok(list_res.processor_versions.unwrap_or_default())
    }

    async fn get_access_token(&self) -> Result<String, DocAiError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        {
            let cache = self.token_cache.lock().await;
            if let Some(token) = cache.as_ref() {
                if token.expires_at > now + 60 {
                    return Ok(token.access_token.clone());
                }
            }
        }

        // Try ADC first if configured (same precedence the rest of the
        // pipeline expects: API-key > ADC > service-account).
        if !self.config.adc_path.is_empty() {
            match self.refresh_via_adc(now).await {
                Ok(token) => return Ok(token),
                Err(e) => {
                    tracing::warn!(
                        "[doc_ai] ADC token refresh failed: {}; falling back to service-account",
                        e
                    );
                }
            }
        }

        if self.config.service_account_path.is_empty() {
            return Err(DocAiError::MissingConfig("GOOGLE_APPLICATION_CREDENTIALS"));
        }

        let key_content = std::fs::read_to_string(&self.config.service_account_path)?;
        let service_account: serde_json::Value = serde_json::from_str(&key_content)?;

        let client_email = service_account["client_email"]
            .as_str()
            .ok_or_else(|| DocAiError::Parse(serde::de::Error::custom("missing client_email")))?;
        let private_key = service_account["private_key"]
            .as_str()
            .ok_or_else(|| DocAiError::Parse(serde::de::Error::custom("missing private_key")))?;

        let claims = JwtClaims {
            iss: client_email.to_string(),
            scope: "https://www.googleapis.com/auth/cloud-platform".to_string(),
            aud: "https://oauth2.googleapis.com/token".to_string(),
            iat: now,
            exp: now + 3600,
        };

        let mut header = Header::new(Algorithm::RS256);
        header.typ = Some("JWT".to_string());
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes())?;
        let signed_jwt = encode(&header, &claims, &encoding_key)?;

        let response = self
            .http
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &signed_jwt),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(DocAiError::Auth(
                response.status(),
                response.text().await.unwrap_or_default(),
            ));
        }

        let token_resp: TokenResponse = response.json().await?;

        let mut cache = self.token_cache.lock().await;
        *cache = Some(CachedToken {
            access_token: token_resp.access_token.clone(),
            expires_at: now + token_resp.expires_in,
        });

        Ok(token_resp.access_token)
    }

    /// Exchange the refresh_token in an ADC file for a fresh access token.
    /// The ADC file is the one written by `gcloud auth application-default
    /// login` and lives at the platform-specific well-known path.
    async fn refresh_via_adc(&self, now: u64) -> Result<String, DocAiError> {
        let raw = std::fs::read_to_string(&self.config.adc_path)?;
        let adc: serde_json::Value = serde_json::from_str(&raw)?;
        // ADC user files have "type":"authorized_user" + client_id/secret/refresh_token.
        // ADC service-account files have "type":"service_account" + private_key (we
        // intentionally do not handle that here; service-account path covers it).
        let kind = adc["type"].as_str().unwrap_or("");
        if kind != "authorized_user" {
            return Err(DocAiError::Parse(serde::de::Error::custom(format!(
                "ADC file is not an authorized_user (got type={:?})",
                kind
            ))));
        }

        let client_id = adc["client_id"]
            .as_str()
            .ok_or_else(|| DocAiError::Parse(serde::de::Error::custom("ADC missing client_id")))?;
        let client_secret = adc["client_secret"].as_str().ok_or_else(|| {
            DocAiError::Parse(serde::de::Error::custom("ADC missing client_secret"))
        })?;
        let refresh_token = adc["refresh_token"].as_str().ok_or_else(|| {
            DocAiError::Parse(serde::de::Error::custom("ADC missing refresh_token"))
        })?;

        let response = self
            .http
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("refresh_token", refresh_token),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(DocAiError::Auth(
                response.status(),
                response.text().await.unwrap_or_default(),
            ));
        }

        let token_resp: TokenResponse = response.json().await?;
        let mut cache = self.token_cache.lock().await;
        *cache = Some(CachedToken {
            access_token: token_resp.access_token.clone(),
            expires_at: now + token_resp.expires_in,
        });
        Ok(token_resp.access_token)
    }

    /// Very lightweight test call to verify the configured credentials
    /// can successfully mint an OAuth token.
    pub async fn ping(&self) -> Result<(), DocAiError> {
        // Just attempting to mint/refresh the token proves the credential exists,
        // is valid JSON, and is cryptographically accepted by Google.
        let _ = self.get_access_token().await?;
        Ok(())
    }

    /// Submits an asynchronous batch processing job to Document AI for a GCS directory.
    /// `gcs_input_prefix` must be a gs:// URI (e.g. "gs://my-bucket/statements/").
    /// `gcs_output_uri` must be a gs:// URI where output JSONs will be written.
    pub async fn batch_process_gcs_prefix(
        &self,
        gcs_input_prefix: &str,
        gcs_output_uri: &str,
        version_id: Option<&str>,
    ) -> Result<Operation, DocAiError> {
        let access_token = self.get_access_token().await?;

        let req_body = BatchProcessRequest {
            input_documents: BatchInputDocuments {
                gcs_prefix: GcsPrefix {
                    gcs_uri_prefix: gcs_input_prefix.to_string(),
                },
            },
            document_output_config: DocumentOutputConfig {
                gcs_output_config: GcsOutputConfig {
                    gcs_uri: gcs_output_uri.to_string(),
                },
            },
        };

        let body = serde_json::to_value(&req_body).unwrap_or_default();
        let response = self.execute_with_failover(
            |loc| self.batch_process_url(version_id, loc),
            Some(&body),
            Some(&access_token),
            None
        ).await?;

        let op: Operation = response.json().await?;
        Ok(op)
    }

    pub async fn parse_entire_statement(
        &self,
        pdf_path: &Path,
    ) -> Result<BankStatement, DocAiError> {
        self.parse_entire_statement_with_version(pdf_path, None).await
    }

    pub async fn parse_entire_statement_with_version(
        &self,
        pdf_path: &Path,
        version_id: Option<&str>,
    ) -> Result<BankStatement, DocAiError> {
        // ----- Cache lookup --------------------------------------------------
        // Document AI is billed per page; if we've parsed this exact PDF
        // through this exact processor before, return the cached result.
        let cache = match crate::ai::docai_cache::DocAiCache::open_default() {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!("[doc_ai] cache disabled (open failed): {}", e);
                None
            }
        };
        let cache_key = if cache.is_some() {
            crate::ai::docai_cache::DocAiCache::make_key(
                pdf_path,
                &self.config.project_id,
                &self.config.location,
                &self.config.processor_id,
                version_id.unwrap_or("default"), // routing to specific version if provided
            )
            .ok()
        } else {
            None
        };
        if let (Some(c), Some(k)) = (cache.as_ref(), cache_key.as_ref()) {
            if let Some(hit) = c.get(k) {
                tracing::info!("[doc_ai] cache HIT (skipping network) key={}", &k[..16]);
                return Ok(hit);
            }
        }

        let pdf_bytes = std::fs::read(pdf_path)?;
        let base64_pdf = Base64Standard.encode(&pdf_bytes);
        let body = serde_json::json!({
            "rawDocument": {
                "content": base64_pdf,
                "mimeType": "application/pdf"
            }
        });

        let access_token = if self.config.api_key.is_empty() {
            Some(self.get_access_token().await?)
        } else {
            None
        };
        let api_key_opt = if !self.config.api_key.is_empty() {
            Some(self.config.api_key.as_str())
        } else {
            None
        };

        let response = self.execute_with_failover(
            |loc| self.process_url_v1beta3(version_id, loc),
            Some(&body),
            access_token.as_deref(),
            api_key_opt
        ).await?;

        let result: serde_json::Value = response.json().await?;
        let stmt = Self::parse_response_into_bank_statement(&result)?;
        if let (Some(c), Some(k)) = (cache.as_ref(), cache_key.as_ref()) {
            if let Err(e) = c.put(k, &stmt) {
                tracing::warn!("[doc_ai] cache write failed: {}", e);
            }
        }
        Ok(stmt)
    }

    pub async fn parse_specialized_document(
        &self,
        pdf_path: &Path,
        processor_id: &str,
        version_id: Option<&str>,
    ) -> Result<serde_json::Value, DocAiError> {
        let pdf_bytes = std::fs::read(pdf_path)?;
        let base64_pdf = Base64Standard.encode(&pdf_bytes);
        let body = serde_json::json!({
            "rawDocument": {
                "content": base64_pdf,
                "mimeType": "application/pdf"
            }
        });

        let access_token = self.get_access_token().await?;
        
        let response = self.execute_with_failover(
            |loc| self.process_url_specialized(processor_id, version_id, loc),
            Some(&body),
            Some(&access_token),
            None
        ).await?;

        let result: serde_json::Value = response.json().await?;
        Ok(result)
    }

    /// Invokes a Custom Document Splitter to determine which pages of a PDF
    /// actually belong to a Bank Statement. Returns a list of 0-indexed page numbers.
    pub async fn get_bank_statement_pages(
        &self,
        pdf_path: &Path,
        splitter_id: &str,
    ) -> Result<Vec<usize>, DocAiError> {
        let result = self.parse_specialized_document(pdf_path, splitter_id, None).await?;
        let mut valid_pages = Vec::new();

        if let Some(entities) = result.get("document").and_then(|d| d.get("entities")).and_then(|e| e.as_array()) {
            for entity in entities {
                if let Some(doc_type) = entity.get("type").and_then(|t| t.as_str()) {
                    let is_statement = doc_type.to_lowercase().contains("statement");
                    if is_statement {
                        if let Some(page_refs) = entity.get("pageAnchor").and_then(|a| a.get("pageRefs")).and_then(|pr| pr.as_array()) {
                            for pr in page_refs {
                                if let Some(page_num) = pr.get("page").and_then(|p| p.as_u64()) {
                                    // The API sometimes returns page indices as string vs int depending on version
                                    let p_idx = page_num as usize;
                                    if !valid_pages.contains(&p_idx) {
                                        valid_pages.push(p_idx);
                                    }
                                } else if let Some(page_str) = pr.get("page").and_then(|p| p.as_str()) {
                                    if let Ok(p_idx) = page_str.parse::<usize>() {
                                        if !valid_pages.contains(&p_idx) {
                                            valid_pages.push(p_idx);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        
        valid_pages.sort();
        Ok(valid_pages)
    }

    pub async fn parse_layout_chunks(
        &self,
        pdf_path: &Path,
        version_id: Option<&str>,
    ) -> Result<serde_json::Value, DocAiError> {
        let pdf_bytes = std::fs::read(pdf_path)?;
        let base64_pdf = Base64Standard.encode(&pdf_bytes);
        let body = serde_json::json!({
            "rawDocument": {
                "content": base64_pdf,
                "mimeType": "application/pdf"
            },
            "processOptions": {
                "layoutConfig": {
                    "chunkingConfig": {
                        "chunkSize": 1000,
                        "includeAncestorHeadings": true
                    }
                }
            }
        });

        let access_token = self.get_access_token().await?;

        let response = self.execute_with_failover(
            |loc| self.process_url_v1(version_id, loc),
            Some(&body),
            Some(&access_token),
            None
        ).await?;



        let result: serde_json::Value = response.json().await?;
        Ok(result)
    }

    pub async fn tournament_parse(
        &self,
        pdf_path: &Path,
        progress_tx: Option<&std::sync::mpsc::Sender<crate::app::runtime::JobResult>>,
    ) -> Result<BankStatement, DocAiError> {
        let mut versions = Vec::new();
        
        if let Some(tx) = progress_tx {
            let _ = tx.send(crate::app::runtime::JobResult::Progress {
                label: "Fetching processor versions and evaluation metrics...".into(),
                fraction: 0.1,
            });
        }
        
        match self.list_processor_versions().await {
            Ok(mut fetched_versions) => {
                // Filter to DEPLOYED versions and sort by F1 Score (descending)
                fetched_versions.retain(|v| v.state == "DEPLOYED");
                fetched_versions.sort_by(|a, b| {
                    let f1_a = a.evaluation.as_ref().and_then(|e| e.all_entities_metrics.as_ref()).and_then(|m| m.f1_score).unwrap_or(0.0);
                    let f1_b = b.evaluation.as_ref().and_then(|e| e.all_entities_metrics.as_ref()).and_then(|m| m.f1_score).unwrap_or(0.0);
                    f1_b.partial_cmp(&f1_a).unwrap_or(std::cmp::Ordering::Equal)
                });
                
                // Extract just the version ID part (everything after processorVersions/)
                for v in fetched_versions.iter().take(4) {
                    if let Some(idx) = v.name.rfind('/') {
                        let id = &v.name[idx + 1..];
                        versions.push(id.to_string());
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to list processor versions: {}. Falling back to hardcoded defaults.", e);
            }
        }
        
        if versions.is_empty() {
            versions = vec![
                "pretrained-bankstatement-v5.0-2023-12-06".to_string(),
                "pretrained-bankstatement-v3.0-2022-05-16".to_string(),
                "pretrained-bankstatement-v2.0-2021-12-10".to_string(),
            ];
        }

        let mut best_stmt = None;
        let mut best_imbalance = 9999999999.0;

        for v in versions {
            if let Some(tx) = progress_tx {
                let _ = tx.send(crate::app::runtime::JobResult::Progress {
                    label: format!("Analyzing Document AI version: {}", v),
                    fraction: 0.4,
                });
            }

            match self.parse_entire_statement_with_version(pdf_path, Some(&v)).await {
                Ok(stmt) => {
                    if stmt.transactions.is_empty() { continue; }
                    
                    let initial = crate::engine::model::dec_to_f64(stmt.opening_balance);
                    let final_bal = crate::engine::model::dec_to_f64(stmt.closing_balance);
                    let mut sum = 0.0;
                    for t in &stmt.transactions {
                        if let Some(c) = t.credit { sum += crate::engine::model::dec_to_f64(c); }
                        if let Some(d) = t.debit { sum -= crate::engine::model::dec_to_f64(d); }
                    }
                    let calculated = initial + sum;
                    let imbalance = (final_bal - calculated).abs();
                    
                    if imbalance < 0.01 {
                        if let Some(tx) = progress_tx {
                            let _ = tx.send(crate::app::runtime::JobResult::Progress {
                                label: format!("Perfect match found with {}", v),
                                fraction: 0.5,
                            });
                        }
                        return Ok(stmt);
                    }

                    if imbalance < best_imbalance {
                        best_imbalance = imbalance;
                        best_stmt = Some(stmt);
                    }
                }
                Err(e) => {
                    tracing::warn!("Version {} failed: {}", v, e);
                }
            }
        }

        if let Some(stmt) = best_stmt {
            return Ok(stmt);
        }

        // Fallback to specialized processor if defined
        if let Some(spec_proc_id) = &self.config.specialized_processor_id {
            if let Some(tx) = progress_tx {
                let _ = tx.send(crate::app::runtime::JobResult::Progress {
                    label: "Bank Statement parsers failed. Trying Specialized Processor...".into(),
                    fraction: 0.5,
                });
            }
            match self.parse_specialized_document(pdf_path, spec_proc_id, None).await {
                Ok(raw) => {
                    match Self::parse_response_into_bank_statement(&raw) {
                        Ok(stmt) => return Ok(stmt),
                        Err(e) => {
                            tracing::warn!("Specialized processor failed to map to Bank Statement: {}", e);
                            return Err(DocAiError::Parse(serde::de::Error::custom("UNSUPPORTED_FORMAT")));
                        }
                    }
                }
                Err(e) => tracing::warn!("Specialized processor failed: {}", e),
            }
        }

        Err(DocAiError::Parse(serde::de::Error::custom("UNSUPPORTED_FORMAT: No Document AI processor version is suited for this bank statement layout.")))
    }

    pub async fn tournament_parse_chunked(
        &self,
        chunks: &[(std::path::PathBuf, usize)],
        max_concurrency: usize,
        progress_tx: Option<&std::sync::mpsc::Sender<crate::app::runtime::JobResult>>,
    ) -> Result<BankStatement, DocAiError> {
        let mut versions = Vec::new();
        
        if let Some(tx) = progress_tx {
            let _ = tx.send(crate::app::runtime::JobResult::Progress {
                label: "Fetching processor versions and evaluation metrics...".into(),
                fraction: 0.1,
            });
        }
        
        match self.list_processor_versions().await {
            Ok(mut fetched_versions) => {
                fetched_versions.retain(|v| v.state == "DEPLOYED");
                fetched_versions.sort_by(|a, b| {
                    let f1_a = a.evaluation.as_ref().and_then(|e| e.all_entities_metrics.as_ref()).and_then(|m| m.f1_score).unwrap_or(0.0);
                    let f1_b = b.evaluation.as_ref().and_then(|e| e.all_entities_metrics.as_ref()).and_then(|m| m.f1_score).unwrap_or(0.0);
                    f1_b.partial_cmp(&f1_a).unwrap_or(std::cmp::Ordering::Equal)
                });
                for v in fetched_versions.iter().take(4) {
                    if let Some(idx) = v.name.rfind('/') {
                        versions.push(v.name[idx + 1..].to_string());
                    }
                }
            }
            Err(e) => tracing::warn!("Failed to list processor versions: {}. Falling back.", e),
        }
        
        if versions.is_empty() {
            versions = vec![
                "pretrained-bankstatement-v5.0-2023-12-06".to_string(),
                "pretrained-bankstatement-v3.0-2022-05-16".to_string(),
                "pretrained-bankstatement-v2.0-2021-12-10".to_string(),
            ];
        }

        let mut best_stmt = None;
        let mut best_imbalance = 9999999999.0;

        for v in versions {
            if let Some(tx) = progress_tx {
                let _ = tx.send(crate::app::runtime::JobResult::Progress {
                    label: format!("Analyzing Document AI version: {}", v),
                    fraction: 0.4,
                });
            }

            match self.parse_chunked_statement_with_version(chunks, max_concurrency, Some(&v)).await {
                Ok(stmt) => {
                    if stmt.transactions.is_empty() { continue; }
                    
                    let initial = crate::engine::model::dec_to_f64(stmt.opening_balance);
                    let final_bal = crate::engine::model::dec_to_f64(stmt.closing_balance);
                    let mut sum = 0.0;
                    for t in &stmt.transactions {
                        if let Some(c) = t.credit { sum += crate::engine::model::dec_to_f64(c); }
                        if let Some(d) = t.debit { sum -= crate::engine::model::dec_to_f64(d); }
                    }
                    let calculated = initial + sum;
                    let imbalance = (final_bal - calculated).abs();
                    
                    if imbalance < 0.01 {
                        if let Some(tx) = progress_tx {
                            let _ = tx.send(crate::app::runtime::JobResult::Progress {
                                label: format!("Perfect match found with {}", v),
                                fraction: 0.5,
                            });
                        }
                        return Ok(stmt);
                    }

                    if imbalance < best_imbalance {
                        best_imbalance = imbalance;
                        best_stmt = Some(stmt);
                    }
                }
                Err(e) => {
                    tracing::warn!("Version {} failed: {}", v, e);
                }
            }
        }

        best_stmt.ok_or_else(|| DocAiError::Api(reqwest::StatusCode::BAD_REQUEST, "UNSUPPORTED_FORMAT: No Document AI processor version is suited for this bank statement layout.".into()))
    }

    /// Parse a list of pre-chunked PDFs in parallel and merge the results
    /// into a single [`BankStatement`]. Stage 3 / Item #16: avoids the 30
    /// page-per-request processor cap by chunking + parallelising.
    ///
    /// `chunks` is a slice of `(chunk_path, page_offset)` pairs — typically
    /// produced by `python/pymupdf_pro_integration.py::chunk_pdf_for_docai`.
    /// `max_concurrency` caps how many chunk parses run simultaneously to
    /// stay under Document AI's per-second QPS limits (4 is a safe default).
    pub async fn parse_chunked_statement(
        &self,
        chunks: &[(std::path::PathBuf, usize)],
        max_concurrency: usize,
    ) -> Result<BankStatement, DocAiError> {
        self.parse_chunked_statement_with_version(chunks, max_concurrency, None).await
    }

    pub async fn parse_chunked_statement_with_version(
        &self,
        chunks: &[(std::path::PathBuf, usize)],
        max_concurrency: usize,
        version_id: Option<&str>,
    ) -> Result<BankStatement, DocAiError> {
        use futures_util::stream::{FuturesUnordered, StreamExt};
        use std::future::Future;
        use std::pin::Pin;

        if chunks.is_empty() {
            return Err(DocAiError::MissingConfig("no chunks supplied"));
        }

        type ChunkFut<'a> = Pin<
            Box<dyn Future<Output = (usize, usize, Result<BankStatement, DocAiError>)> + Send + 'a>,
        >;
        let mut in_flight: FuturesUnordered<ChunkFut<'_>> = FuturesUnordered::new();
        let mut next_idx = 0usize;
        let mut results: Vec<Option<(usize, BankStatement)>> = (0..chunks.len()).map(|_| None).collect();

        // Prime up to `max_concurrency` parses.
        while next_idx < chunks.len() && in_flight.len() < max_concurrency {
            let (path, offset) = chunks[next_idx].clone();
            let idx = next_idx;
            in_flight.push(Box::pin(async move {
                let r = self.parse_entire_statement_with_version(&path, version_id).await;
                (idx, offset, r)
            }));
            next_idx += 1;
        }

        while let Some((idx, offset, res)) = in_flight.next().await {
            let stmt = res?;
            // Re-write each transaction's `page` to the absolute index in the
            // unchunked document.
            let shifted_txs: Vec<Transaction> = stmt
                .transactions
                .into_iter()
                .map(|mut t| {
                    t.page += offset;
                    t
                })
                .collect();
            results[idx] = Some((
                offset,
                BankStatement {
                    transactions: shifted_txs,
                    ..stmt
                },
            ));
            // Top up.
            if next_idx < chunks.len() {
                let (path, offset) = chunks[next_idx].clone();
                let i = next_idx;
                in_flight.push(Box::pin(async move {
                    let r = self.parse_entire_statement_with_version(&path, version_id).await;
                    (i, offset, r)
                }));
                next_idx += 1;
            }
        }

        // Merge in chunk-index order so transactions stay in document order.
        let chunked: Vec<BankStatement> = results
            .into_iter()
            .filter_map(|r| r.map(|(_, s)| s))
            .collect();
        Ok(merge_chunk_results(chunked))
    }

    fn parse_response_into_bank_statement(
        result: &serde_json::Value,
    ) -> Result<BankStatement, DocAiError> {
        let pages_node = result["document"]["pages"].as_array();
        let total_pages = pages_node.map_or(0, |p| p.len());

        // Build a page-index → (width_pts, height_pts) map. DocAI normalizes
        // bbox vertices in 0..1, so we need page dimensions to convert back
        // to PDF-points-equivalent units. The `dimension` block has a `unit`
        // field ("inches", "cm", "points", "pixels"); when missing or "pixels"
        // we fall back to the raw values which still work for the editor —
        // the editor consumes whatever unit pdfium-render is using to render
        // the same page, and as long as both ends agree it's a no-op.
        let pages_dim: Vec<(f32, f32, String)> = pages_node
            .map(|pages| {
                pages
                    .iter()
                    .map(|p| {
                        let w = p["dimension"]["width"].as_f64().unwrap_or(0.0) as f32;
                        let h = p["dimension"]["height"].as_f64().unwrap_or(0.0) as f32;
                        let unit = p["dimension"]["unit"]
                            .as_str()
                            .unwrap_or("")
                            .to_string();
                        (w, h, unit)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut transactions = Vec::new();
        let mut opening_balance = Decimal::ZERO;
        let mut closing_balance = Decimal::ZERO;
        let mut account_number: Option<String> = None;

        if let Some(entities) = result["document"]["entities"].as_array() {
            for (idx, entity) in entities.iter().enumerate() {
                let etype = entity["type"].as_str().unwrap_or("");
                let text = entity["mentionText"]
                    .as_str()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let confidence = entity["confidence"].as_f64().unwrap_or(0.0) as f32;

                // Pull the page index and bbox (in PDF-points-equivalent units)
                // from the entity's pageAnchor. Falls back to (0, None) if the
                // anchor is missing.
                let (page_idx, row_bbox) = entity_page_and_bbox(entity, &pages_dim);

                match etype {
                    // Document AI Bank Statement Parser emits a `table_item`
                    // per row, with nested `properties` for each column.
                    "table_item" => {
                        // Helper to read either side (deposit or withdrawal)
                        let date = extract_string_property(entity, "transaction_deposit_date")
                            .or_else(|| {
                                extract_string_property(entity, "transaction_withdrawal_date")
                            })
                            .or_else(|| extract_string_property(entity, "transaction_date"))
                            .unwrap_or_default();

                        let description =
                            extract_string_property(entity, "transaction_deposit_description")
                                .or_else(|| {
                                    extract_string_property(
                                        entity,
                                        "transaction_withdrawal_description",
                                    )
                                })
                                .or_else(|| {
                                    extract_string_property(entity, "transaction_description")
                                })
                                .unwrap_or_else(|| text.clone());

                        let credit = extract_number_property(entity, "transaction_deposit");
                        let debit = extract_number_property(entity, "transaction_withdrawal");
                        let running_balance = extract_number_property(entity, "running_balance")
                            .or_else(|| extract_number_property(entity, "transaction_balance"));

                        // Skip if neither side has a value (probably a header row).
                        if credit.is_none() && debit.is_none() && running_balance.is_none() {
                            continue;
                        }

                        // Per-field bbox extraction so the binary edit redacts
                        // the cell, not the whole row. Stage 7.5.
                        let field_bboxes = FieldBboxes {
                            date: property_bbox(
                                entity,
                                &["transaction_deposit_date", "transaction_withdrawal_date", "transaction_date"],
                                &pages_dim,
                            ),
                            description: property_bbox(
                                entity,
                                &[
                                    "transaction_deposit_description",
                                    "transaction_withdrawal_description",
                                    "transaction_description",
                                ],
                                &pages_dim,
                            ),
                            debit: property_bbox(entity, &["transaction_withdrawal", "debit"], &pages_dim),
                            credit: property_bbox(entity, &["transaction_deposit", "credit"], &pages_dim),
                            running_balance: property_bbox(
                                entity,
                                &["running_balance", "transaction_balance"],
                                &pages_dim,
                            ),
                        };

                        transactions.push(Transaction {
                            page: page_idx,
                            line_on_page: idx,
                            date,
                            raw_text: description,
                            debit,
                            credit,
                            running_balance,
                            bbox: row_bbox,
                            field_bboxes,
                            provenance: Provenance::DocumentAI { confidence },
                        });
                    }

                    // Some processors emit "transaction" directly with the
                    // same property layout — keep this branch as a fallback.
                    "transaction" => {
                        let field_bboxes = FieldBboxes {
                            date: property_bbox(
                                entity,
                                &["transaction_date", "transaction_deposit_date"],
                                &pages_dim,
                            ),
                            description: None,
                            debit: property_bbox(entity, &["debit", "transaction_withdrawal"], &pages_dim),
                            credit: property_bbox(entity, &["credit", "transaction_deposit"], &pages_dim),
                            running_balance: property_bbox(entity, &["running_balance"], &pages_dim),
                        };
                        transactions.push(Transaction {
                            page: page_idx,
                            line_on_page: idx,
                            date: extract_string_property(entity, "transaction_date")
                                .or_else(|| {
                                    extract_string_property(entity, "transaction_deposit_date")
                                })
                                .unwrap_or_default(),
                            raw_text: text,
                            debit: extract_number_property(entity, "debit").or_else(|| {
                                extract_number_property(entity, "transaction_withdrawal")
                            }),
                            credit: extract_number_property(entity, "credit")
                                .or_else(|| extract_number_property(entity, "transaction_deposit")),
                            running_balance: extract_number_property(entity, "running_balance"),
                            bbox: row_bbox,
                            field_bboxes,
                            provenance: Provenance::DocumentAI { confidence },
                        });
                    }

                    "starting_balance" | "opening_balance" => {
                        if let Ok(v) = text.replace(['$', ','], "").parse::<f64>() {
                            opening_balance = f64_to_dec(v);
                        }
                    }
                    "ending_balance" | "closing_balance" => {
                        if let Ok(v) = text.replace(['$', ','], "").parse::<f64>() {
                            closing_balance = f64_to_dec(v);
                        }
                    }
                    "account_number" => {
                        if !text.is_empty() {
                            account_number = Some(text);
                        }
                    }
                    _ => {}
                }
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

    // -----------------------------------------------------------------------
    // Stage 4 / Item #12: Training orchestration.
    //
    // Document AI lets us train a custom processor version on a labelled
    // dataset. The flow:
    //   1. Poll dataset to count labelled documents.
    //   2. When ≥8 are labelled, kick off `processorVersions:train`.
    //   3. Poll the returned long-running operation until done.
    //   4. Optionally set the new version as the processor's default.
    //
    // All four steps go through the same auth path as `parse_entire_statement`
    // (API key first, ADC, service account). We always use v1beta3 because
    // the training API isn't on v1.
    // -----------------------------------------------------------------------

    /// Authenticate the next request, returning either a complete URL with
    /// `?key=...` appended, or `(url, Some(bearer_token))`.
    ///
    /// Hides the auth tier selection from each training method.
    async fn authed_url(&self, base_url: &str) -> Result<(String, Option<String>), DocAiError> {
        if !self.config.api_key.is_empty() {
            let glue = if base_url.contains('?') { '&' } else { '?' };
            return Ok((format!("{base_url}{glue}key={}", self.config.api_key), None));
        }
        if self.config.adc_path.is_empty() && self.config.service_account_path.is_empty() {
            return Err(DocAiError::MissingConfig(
                "no OAuth credential available (neither ADC nor service account)",
            ));
        }
        let token = self.get_access_token().await?;
        Ok((base_url.to_string(), Some(token)))
    }

    /// Apply auth to a `RequestBuilder`. Caller has already chosen the URL
    /// from [`authed_url`]; here we just attach the bearer if present.
    fn apply_auth(
        &self,
        builder: reqwest::RequestBuilder,
        token: Option<&str>,
    ) -> reqwest::RequestBuilder {
        match token {
            Some(t) => builder.bearer_auth(t),
            None => builder,
        }
    }

    /// Count documents marked as `LABELED` in the processor's dataset.
    /// Returns `(labeled_count, total_count)`.
    pub async fn count_labeled_documents(&self) -> Result<(usize, usize), DocAiError> {
        let base = format!(
            "https://{}-documentai.googleapis.com/v1beta3/projects/{}/locations/{}/processors/{}/dataset/documents:list",
            self.config.location, self.config.project_id, self.config.location, self.config.processor_id
        );
        let (url, token) = self.authed_url(&base).await?;
        let req = self.http.get(&url);
        let req = self.apply_auth(req, token.as_deref());
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(DocAiError::Api(
                resp.status(),
                resp.text().await.unwrap_or_default(),
            ));
        }
        let body: serde_json::Value = resp.json().await?;
        let docs = body["documentMetadata"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let total = docs.len();
        let labeled = docs
            .iter()
            .filter(|d| {
                d["datasetType"].as_str() == Some("DATASET_SPLIT_TRAIN")
                    || d["datasetType"].as_str() == Some("DATASET_SPLIT_TEST")
            })
            .filter(|d| {
                d["labelingState"].as_str() == Some("DOCUMENT_LABELED")
            })
            .count();
        Ok((labeled, total))
    }

    /// Kick off training for a new processor version. Returns the LRO name
    /// (`projects/.../operations/<id>`) the caller can poll.
    pub async fn start_training(&self, display_name: &str) -> Result<String, DocAiError> {
        let base = format!(
            "https://{}-documentai.googleapis.com/v1beta3/projects/{}/locations/{}/processors/{}/processorVersions:train",
            self.config.location, self.config.project_id, self.config.location, self.config.processor_id
        );
        let (url, token) = self.authed_url(&base).await?;
        let body = serde_json::json!({
            "processorVersion": {
                "displayName": display_name
            }
        });
        let req = self.http.post(&url).json(&body);
        let req = self.apply_auth(req, token.as_deref());
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(DocAiError::Api(
                resp.status(),
                resp.text().await.unwrap_or_default(),
            ));
        }
        let body: serde_json::Value = resp.json().await?;
        body["name"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| DocAiError::Parse(serde::de::Error::custom("training response missing 'name'")))
    }

    /// Poll an LRO once. Returns `(done, error_message_if_failed)`.
    pub async fn poll_operation(&self, op_name: &str) -> Result<(bool, Option<String>), DocAiError> {
        let base = format!(
            "https://{}-documentai.googleapis.com/v1beta3/{}",
            self.config.location, op_name
        );
        let (url, token) = self.authed_url(&base).await?;
        let req = self.http.get(&url);
        let req = self.apply_auth(req, token.as_deref());
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(DocAiError::Api(
                resp.status(),
                resp.text().await.unwrap_or_default(),
            ));
        }
        let body: serde_json::Value = resp.json().await?;
        let done = body["done"].as_bool().unwrap_or(false);
        let err = body["error"]["message"].as_str().map(|s| s.to_string());
        Ok((done, err))
    }

    /// Set a processor version as the processor's default.
    pub async fn set_default_version(&self, version_id: &str) -> Result<(), DocAiError> {
        let base = format!(
            "https://{}-documentai.googleapis.com/v1beta3/projects/{}/locations/{}/processors/{}:setDefaultProcessorVersion",
            self.config.location, self.config.project_id, self.config.location, self.config.processor_id
        );
        let (url, token) = self.authed_url(&base).await?;
        let body = serde_json::json!({
            "defaultProcessorVersion": format!(
                "projects/{}/locations/{}/processors/{}/processorVersions/{}",
                self.config.project_id, self.config.location, self.config.processor_id, version_id
            )
        });
        let req = self.http.post(&url).json(&body);
        let req = self.apply_auth(req, token.as_deref());
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(DocAiError::Api(
                resp.status(),
                resp.text().await.unwrap_or_default(),
            ));
        }
        Ok(())
    }
}

/// Merge per-chunk parses into one `BankStatement`. Stage 3 / Item #16.
///
/// Chunks are assumed to be in document order. Transactions retain their
/// (already shifted) `page` numbers; total_pages sums; opening balance and
/// account number come from the first chunk; closing balance from the last.
fn merge_chunk_results(chunked: Vec<BankStatement>) -> BankStatement {
    let mut merged = BankStatement {
        total_pages: 0,
        transactions: Vec::new(),
        opening_balance: Decimal::ZERO,
        closing_balance: Decimal::ZERO,
        account_number: None,
    };
    for (i, stmt) in chunked.into_iter().enumerate() {
        merged.total_pages += stmt.total_pages;
        merged.transactions.extend(stmt.transactions);
        if i == 0 {
            merged.opening_balance = stmt.opening_balance;
            merged.account_number = stmt.account_number;
        }
        merged.closing_balance = stmt.closing_balance;
    }
    merged
}

fn extract_string_property(entity: &serde_json::Value, kind: &str) -> Option<String> {
    entity["properties"].as_array().and_then(|props| {
        props
            .iter()
            .find(|p| p["type"].as_str() == Some(kind))
            .and_then(|p| p["mentionText"].as_str())
            .map(|s| s.trim().to_string())
    })
}

fn extract_number_property(entity: &serde_json::Value, kind: &str) -> Option<Decimal> {
    extract_string_property(entity, kind)
        .and_then(|s| s.replace(['$', ','], "").parse::<f64>().ok())
        .map(f64_to_dec)
}

/// Stage 7.5: read the page index (0-based) and the row's bbox in
/// document-page units from a Document AI entity. Returns `(0, None)` when
/// the entity has no `pageAnchor.pageRefs[0]`.
///
/// `pages_dim` is the per-page `(width, height, unit)` table built from
/// `document.pages[].dimension`. We multiply normalized vertices by
/// `(width, height)` to land back in PDF-points-equivalent units. If the
/// entity stores bbox vertices already in absolute units we use those
/// directly.
fn entity_page_and_bbox(
    entity: &serde_json::Value,
    pages_dim: &[(f32, f32, String)],
) -> (usize, Option<[f32; 4]>) {
    let Some(refs) = entity["pageAnchor"]["pageRefs"].as_array() else {
        return (0, None);
    };
    let Some(first) = refs.first() else {
        return (0, None);
    };
    let page_idx = first["page"]
        .as_str()
        .and_then(|s| s.parse::<usize>().ok())
        .or_else(|| first["page"].as_u64().map(|n| n as usize))
        .unwrap_or(0);
    let bbox = bbox_from_bounding_poly(&first["boundingPoly"], page_idx, pages_dim);
    (page_idx, bbox)
}

/// Same as [`entity_page_and_bbox`] but for one of the entity's nested
/// `properties` (debit, credit, running_balance, …). Tries each kind in
/// order and returns the first one whose property has a bbox.
fn property_bbox(
    entity: &serde_json::Value,
    kinds: &[&str],
    pages_dim: &[(f32, f32, String)],
) -> Option<[f32; 4]> {
    let props = entity["properties"].as_array()?;
    for kind in kinds {
        for p in props {
            if p["type"].as_str() == Some(*kind) {
                let (_, bbox) = entity_page_and_bbox(p, pages_dim);
                if bbox.is_some() {
                    return bbox;
                }
            }
        }
    }
    None
}

fn bbox_from_bounding_poly(
    poly: &serde_json::Value,
    page_idx: usize,
    pages_dim: &[(f32, f32, String)],
) -> Option<[f32; 4]> {
    // DocAI prefers `normalizedVertices` (0..1). Older responses use
    // `vertices` in absolute pixel/point units.
    if let Some(verts) = poly["normalizedVertices"].as_array() {
        if verts.is_empty() {
            return None;
        }
        let (w, h) = pages_dim
            .get(page_idx)
            .map(|(w, h, _)| (*w, *h))
            .unwrap_or((1.0, 1.0));
        let mut x0 = f32::MAX;
        let mut y0 = f32::MAX;
        let mut x1 = f32::MIN;
        let mut y1 = f32::MIN;
        for v in verts {
            let x = v["x"].as_f64().unwrap_or(0.0) as f32 * w;
            let y = v["y"].as_f64().unwrap_or(0.0) as f32 * h;
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        return Some([x0, y0, x1, y1]);
    }
    if let Some(verts) = poly["vertices"].as_array() {
        if verts.is_empty() {
            return None;
        }
        let mut x0 = f32::MAX;
        let mut y0 = f32::MAX;
        let mut x1 = f32::MIN;
        let mut y1 = f32::MIN;
        for v in verts {
            let x = v["x"].as_f64().unwrap_or(0.0) as f32;
            let y = v["y"].as_f64().unwrap_or(0.0) as f32;
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        return Some([x0, y0, x1, y1]);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_response_into_bank_statement_works() {
        // Two table_items: one with deposit, one without (just a description) — the
        // empty one should be skipped by the parser because it has no money values.
        let json_str = r#"{
            "document": {
                "pages": [{}],
                "entities": [
                    {
                        "type": "table_item",
                        "mentionText": "09/02/2026 Interest Paid 242.83",
                        "confidence": 0.84,
                        "properties": [
                            { "type": "transaction_deposit_date", "mentionText": "09/02/2026" },
                            { "type": "transaction_deposit_description", "mentionText": "Interest Paid" },
                            { "type": "transaction_deposit", "mentionText": "242.83" }
                        ]
                    },
                    {
                        "type": "table_item",
                        "mentionText": "Header row",
                        "confidence": 0.5,
                        "properties": [
                            { "type": "transaction_deposit_description", "mentionText": "Date Description Amount" }
                        ]
                    },
                    { "type": "starting_balance", "mentionText": "$1000.00", "confidence": 0.95 },
                    { "type": "ending_balance",   "mentionText": "$1242.83", "confidence": 0.95 },
                    { "type": "account_number",   "mentionText": "807466413",  "confidence": 0.99 }
                ]
            }
        }"#;
        let val: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let stmt = DocumentAiClient::parse_response_into_bank_statement(&val).unwrap();
        assert_eq!(stmt.total_pages, 1);
        assert_eq!(
            stmt.transactions.len(),
            1,
            "header row without amounts should be skipped"
        );
        let tx = &stmt.transactions[0];
        assert_eq!(tx.date, "09/02/2026");
        assert_eq!(tx.raw_text, "Interest Paid");
        assert_eq!(tx.credit, Some(f64_to_dec(242.83)));
        assert_eq!(tx.debit, None);
        assert_eq!(stmt.opening_balance, f64_to_dec(1000.00));
        assert_eq!(stmt.closing_balance, f64_to_dec(1242.83));
        assert_eq!(stmt.account_number.as_deref(), Some("807466413"));
        match tx.provenance {
            Provenance::DocumentAI { confidence } => assert!((confidence - 0.84).abs() < 0.01),
            _ => panic!("Wrong provenance"),
        }
    }

    #[test]
    fn parse_response_handles_legacy_transaction_type() {
        // Older / custom processors emit `type: "transaction"` directly.
        let json_str = r#"{
            "document": {
                "pages": [{}],
                "entities": [
                    {
                        "type": "transaction",
                        "mentionText": "Coffee 3.50",
                        "confidence": 0.9,
                        "properties": [
                            { "type": "transaction_date", "mentionText": "2026-01-15" },
                            { "type": "debit",            "mentionText": "$3.50" }
                        ]
                    }
                ]
            }
        }"#;
        let val: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let stmt = DocumentAiClient::parse_response_into_bank_statement(&val).unwrap();
        assert_eq!(stmt.transactions.len(), 1);
        assert_eq!(stmt.transactions[0].debit, Some(f64_to_dec(3.50)));
    }

    #[test]
    fn jwt_claim_shape() {
        let key_content =
            std::fs::read_to_string("tests/fixtures/test_service_account.json").unwrap();
        let service_account: serde_json::Value = serde_json::from_str(&key_content).unwrap();
        let client_email = service_account["client_email"].as_str().unwrap();
        let private_key = service_account["private_key"].as_str().unwrap();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = JwtClaims {
            iss: client_email.to_string(),
            scope: "https://www.googleapis.com/auth/cloud-platform".to_string(),
            aud: "https://oauth2.googleapis.com/token".to_string(),
            iat: now,
            exp: now + 3600,
        };

        let mut header = Header::new(Algorithm::RS256);
        header.typ = Some("JWT".to_string());
        let encoding_key = EncodingKey::from_rsa_pem(private_key.as_bytes()).unwrap();
        let signed_jwt = encode(&header, &claims, &encoding_key).unwrap();

        let parts: Vec<&str> = signed_jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .unwrap();
        let token_data: serde_json::Value = serde_json::from_slice(&payload_bytes).unwrap();
        assert_eq!(token_data["iss"].as_str().unwrap(), client_email);
        assert_eq!(
            token_data["aud"].as_str().unwrap(),
            "https://oauth2.googleapis.com/token"
        );
        assert_eq!(
            token_data["exp"].as_u64().unwrap() - token_data["iat"].as_u64().unwrap(),
            3600
        );
    }

    #[test]
    fn extract_helpers_pull_nested_properties() {
        let entity: serde_json::Value = serde_json::from_str(
            r#"{
                "type": "transaction",
                "mentionText": "Coffee 3.50",
                "properties": [
                    { "type": "transaction_date", "mentionText": "2026-05-01" },
                    { "type": "debit", "mentionText": "$3.50" }
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(
            extract_string_property(&entity, "transaction_date").as_deref(),
            Some("2026-05-01")
        );
        assert_eq!(extract_number_property(&entity, "debit"), Some(f64_to_dec(3.50)));
        assert_eq!(extract_string_property(&entity, "credit"), None);
    }

    #[test]
    fn from_app_config_requires_credential() {
        let cfg = AppConfig {
            passphrase: "x".repeat(20),
            document_ai: Some(DocumentAiConfig {
                project_id: "p".into(),
                location: "us".into(),
                processor_id: "abc".into(),
                specialized_processor_id: None,
                service_account_path: String::new(),
                api_key: String::new(),
                adc_path: String::new(),
            }),
            ..AppConfig::default()
        };
        let res = DocumentAiClient::from_app_config(&cfg);
        assert!(matches!(res, Err(DocAiError::MissingConfig(_))));
    }

    fn fake_chunk(
        page_count: usize,
        opening: f64,
        closing: f64,
        tx_pages: &[usize],
    ) -> BankStatement {
        fake_chunk_with_account(page_count, opening, closing, tx_pages, None)
    }

    fn fake_chunk_with_account(
        page_count: usize,
        opening: f64,
        closing: f64,
        tx_pages: &[usize],
        account: Option<&str>,
    ) -> BankStatement {
        BankStatement {
            total_pages: page_count,
            transactions: tx_pages
                .iter()
                .enumerate()
                .map(|(i, p)| Transaction {
                    page: *p,
                    line_on_page: i,
                    date: "01/01/2026".into(),
                    raw_text: format!("tx{i}"),
                    debit: Some(f64_to_dec(10.0)),
                    credit: None,
                    running_balance: Some(f64_to_dec(opening + 10.0 * (i as f64 + 1.0))),
                    bbox: None,
                    field_bboxes: Default::default(),
                    provenance: Provenance::DocumentAI { confidence: 0.9 },
                })
                .collect(),
            opening_balance: f64_to_dec(opening),
            closing_balance: f64_to_dec(closing),
            account_number: account.map(|s| s.to_string()),
        }
    }

    #[test]
    fn merge_chunk_results_sums_pages_keeps_first_opening_last_closing() {
        // Two chunks: first 30 pages opening 100, second 20 pages closing 500.
        let chunks = vec![
            fake_chunk_with_account(30, 100.0, 350.0, &[0, 5, 12], Some("ACC123")),
            fake_chunk(20, 350.0, 500.0, &[30, 35]),
        ];
        let merged = merge_chunk_results(chunks);
        assert_eq!(merged.total_pages, 50);
        assert_eq!(merged.transactions.len(), 5);
        assert_eq!(merged.opening_balance, f64_to_dec(100.0));
        assert_eq!(merged.closing_balance, f64_to_dec(500.0));
        assert_eq!(merged.account_number.as_deref(), Some("ACC123"));
    }

    #[test]
    fn merge_chunk_results_preserves_transaction_order() {
        let chunks = vec![
            fake_chunk(30, 0.0, 0.0, &[0, 1, 2]),
            fake_chunk(30, 0.0, 0.0, &[30, 31]),
        ];
        let merged = merge_chunk_results(chunks);
        let pages: Vec<usize> = merged.transactions.iter().map(|t| t.page).collect();
        assert_eq!(pages, vec![0, 1, 2, 30, 31]);
    }

    #[test]
    fn merge_chunk_results_handles_single_chunk() {
        let chunks = vec![fake_chunk_with_account(10, 50.0, 200.0, &[0, 5], Some("X"))];
        let merged = merge_chunk_results(chunks);
        assert_eq!(merged.total_pages, 10);
        assert_eq!(merged.opening_balance, f64_to_dec(50.0));
        assert_eq!(merged.closing_balance, f64_to_dec(200.0));
    }

    #[test]
    fn merge_chunk_results_handles_empty() {
        let merged = merge_chunk_results(vec![]);
        assert_eq!(merged.total_pages, 0);
        assert!(merged.transactions.is_empty());
    }

    /// Stage 7.5: parse → edit pipeline integrity. Confirm that bbox info
    /// from Document AI's `pageAnchor.boundingPoly.normalizedVertices`
    /// flows all the way into `Transaction.bbox` and `Transaction.field_bboxes`.
    /// Without this the binary editor redacts the wrong region (or no
    /// region at all).
    #[test]
    fn parse_extracts_row_and_per_field_bboxes_from_page_anchor() {
        let json_str = r#"{
            "document": {
                "pages": [{
                    "pageNumber": 1,
                    "dimension": { "width": 612, "height": 792, "unit": "points" }
                }],
                "entities": [
                    {
                        "type": "table_item",
                        "mentionText": "INTEREST PAID",
                        "confidence": 0.92,
                        "pageAnchor": {
                            "pageRefs": [{
                                "page": "0",
                                "boundingPoly": {
                                    "normalizedVertices": [
                                        { "x": 0.10, "y": 0.30 },
                                        { "x": 0.90, "y": 0.30 },
                                        { "x": 0.90, "y": 0.34 },
                                        { "x": 0.10, "y": 0.34 }
                                    ]
                                }
                            }]
                        },
                        "properties": [
                            {
                                "type": "transaction_deposit_date",
                                "mentionText": "09/02/2026",
                                "pageAnchor": { "pageRefs": [{
                                    "page": "0",
                                    "boundingPoly": { "normalizedVertices": [
                                        { "x": 0.10, "y": 0.30 }, { "x": 0.20, "y": 0.30 },
                                        { "x": 0.20, "y": 0.34 }, { "x": 0.10, "y": 0.34 }
                                    ]}
                                }]}
                            },
                            {
                                "type": "transaction_deposit_description",
                                "mentionText": "Interest Paid",
                                "pageAnchor": { "pageRefs": [{
                                    "page": "0",
                                    "boundingPoly": { "normalizedVertices": [
                                        { "x": 0.22, "y": 0.30 }, { "x": 0.55, "y": 0.30 },
                                        { "x": 0.55, "y": 0.34 }, { "x": 0.22, "y": 0.34 }
                                    ]}
                                }]}
                            },
                            {
                                "type": "transaction_deposit",
                                "mentionText": "$242.83",
                                "pageAnchor": { "pageRefs": [{
                                    "page": "0",
                                    "boundingPoly": { "normalizedVertices": [
                                        { "x": 0.60, "y": 0.30 }, { "x": 0.72, "y": 0.30 },
                                        { "x": 0.72, "y": 0.34 }, { "x": 0.60, "y": 0.34 }
                                    ]}
                                }]}
                            },
                            {
                                "type": "running_balance",
                                "mentionText": "$1,242.83",
                                "pageAnchor": { "pageRefs": [{
                                    "page": "0",
                                    "boundingPoly": { "normalizedVertices": [
                                        { "x": 0.78, "y": 0.30 }, { "x": 0.90, "y": 0.30 },
                                        { "x": 0.90, "y": 0.34 }, { "x": 0.78, "y": 0.34 }
                                    ]}
                                }]}
                            }
                        ]
                    }
                ]
            }
        }"#;
        let val: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let stmt = DocumentAiClient::parse_response_into_bank_statement(&val).unwrap();
        assert_eq!(stmt.transactions.len(), 1);

        let tx = &stmt.transactions[0];
        // Row-level bbox: 0.10..0.90 of width 612 = 61.2..550.8, y 237.6..269.28
        let row = tx.bbox.expect("row bbox should be set from pageAnchor");
        assert!((row[0] - 61.2).abs() < 1.0, "x0={}", row[0]);
        assert!((row[1] - 237.6).abs() < 1.0, "y0={}", row[1]);
        assert!((row[2] - 550.8).abs() < 1.0, "x1={}", row[2]);
        assert!((row[3] - 269.28).abs() < 1.0, "y1={}", row[3]);

        // Per-field bboxes: each cell is its own narrower rectangle.
        let credit_box = tx.field_bboxes.credit
            .expect("transaction_deposit bbox should be set");
        // Credit box: 0.60..0.72 of width 612 = 367.2..440.64
        assert!((credit_box[0] - 367.2).abs() < 1.0, "credit x0={}", credit_box[0]);
        assert!((credit_box[2] - 440.64).abs() < 1.0, "credit x1={}", credit_box[2]);

        let bal_box = tx.field_bboxes.running_balance
            .expect("running_balance bbox should be set");
        assert!((bal_box[0] - 477.36).abs() < 1.0, "bal x0={}", bal_box[0]);
        assert!((bal_box[2] - 550.8).abs() < 1.0, "bal x1={}", bal_box[2]);

        // The credit and running_balance bboxes do not overlap horizontally.
        assert!(
            credit_box[2] < bal_box[0],
            "credit ({}..{}) and balance ({}..{}) overlap",
            credit_box[0], credit_box[2], bal_box[0], bal_box[2]
        );
    }

    /// Edit-payload integrity: the GUI's bbox_for_field equivalent must
    /// pick the field-specific bbox when present, and the row-level bbox
    /// otherwise. We test the data shape here so a refactor of the helper
    /// stays consistent with what DocAI actually returns.
    #[test]
    fn parse_falls_back_to_row_bbox_when_property_anchor_missing() {
        // Same as above but the deposit has no own pageAnchor — falls back
        // to the row's bbox.
        let json_str = r#"{
            "document": {
                "pages": [{ "pageNumber": 1, "dimension": { "width": 100, "height": 100, "unit": "points" } }],
                "entities": [
                    {
                        "type": "table_item",
                        "mentionText": "X",
                        "confidence": 0.9,
                        "pageAnchor": {
                            "pageRefs": [{
                                "page": "0",
                                "boundingPoly": { "normalizedVertices": [
                                    { "x": 0.0, "y": 0.0 }, { "x": 1.0, "y": 0.0 },
                                    { "x": 1.0, "y": 0.1 }, { "x": 0.0, "y": 0.1 }
                                ]}
                            }]
                        },
                        "properties": [
                            { "type": "transaction_deposit", "mentionText": "10.00" }
                        ]
                    }
                ]
            }
        }"#;
        let val: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let stmt = DocumentAiClient::parse_response_into_bank_statement(&val).unwrap();
        assert_eq!(stmt.transactions.len(), 1);
        let tx = &stmt.transactions[0];
        assert!(tx.bbox.is_some(), "row bbox should be set");
        assert!(
            tx.field_bboxes.credit.is_none(),
            "no property anchor → field bbox should be None"
        );
    }
}

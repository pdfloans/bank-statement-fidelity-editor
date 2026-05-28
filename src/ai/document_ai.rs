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
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

use crate::app::config::{AppConfig, DocumentAiConfig};
use crate::engine::model::{Provenance, Transaction};

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
    pub opening_balance: f64,
    pub closing_balance: f64,
    pub account_number: Option<String>,
}

#[derive(Serialize)]
struct JwtClaims {
    iss: String,
    scope: String,
    aud: String,
    iat: u64,
    exp: u64,
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

    fn process_url_v1beta3(&self) -> String {
        format!(
            "https://{}-documentai.googleapis.com/v1beta3/projects/{}/locations/{}/processors/{}:process",
            self.config.location, self.config.project_id, self.config.location, self.config.processor_id
        )
    }

    fn process_url_v1(&self) -> String {
        format!(
            "https://{}-documentai.googleapis.com/v1/projects/{}/locations/{}/processors/{}:process",
            self.config.location, self.config.project_id, self.config.location, self.config.processor_id
        )
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

    pub async fn parse_entire_statement(
        &self,
        pdf_path: &Path,
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
                "default", // we don't currently route to a specific version
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

        // 1. Primary: API key → v1beta3.
        if !self.config.api_key.is_empty() {
            let url = format!("{}?key={}", self.process_url_v1beta3(), self.config.api_key);
            tracing::debug!("[doc_ai] trying v1beta3 API-key auth");
            match self.http.post(&url).json(&body).send().await {
                Ok(resp) if resp.status().is_success() => {
                    let result: serde_json::Value = resp.json().await?;
                    let stmt = Self::parse_response_into_bank_statement(&result)?;
                    if let (Some(c), Some(k)) = (cache.as_ref(), cache_key.as_ref()) {
                        if let Err(e) = c.put(k, &stmt) {
                            tracing::warn!("[doc_ai] cache write failed: {}", e);
                        }
                    }
                    return Ok(stmt);
                }
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
                        tracing::warn!(
                            "[doc_ai] API-key auth rejected ({}); falling back to service-account",
                            status
                        );
                    } else {
                        // Non-auth errors propagate immediately.
                        return Err(DocAiError::Api(status, text));
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "[doc_ai] API-key request failed: {}; trying service-account",
                        e
                    );
                }
            }
        }

        // 2. Fallback: OAuth — get_access_token() handles ADC then SA in priority order.
        if self.config.adc_path.is_empty() && self.config.service_account_path.is_empty() {
            return Err(DocAiError::MissingConfig(
                "no OAuth credential available (neither ADC nor service account)",
            ));
        }

        let access_token = self.get_access_token().await?;
        let url = self.process_url_v1();
        tracing::debug!("[doc_ai] using v1 OAuth (ADC or service-account)");
        let response = self
            .http
            .post(&url)
            .bearer_auth(access_token)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(DocAiError::Api(
                response.status(),
                response.text().await.unwrap_or_default(),
            ));
        }

        let result: serde_json::Value = response.json().await?;
        let stmt = Self::parse_response_into_bank_statement(&result)?;
        if let (Some(c), Some(k)) = (cache.as_ref(), cache_key.as_ref()) {
            if let Err(e) = c.put(k, &stmt) {
                tracing::warn!("[doc_ai] cache write failed: {}", e);
            }
        }
        Ok(stmt)
    }

    fn parse_response_into_bank_statement(
        result: &serde_json::Value,
    ) -> Result<BankStatement, DocAiError> {
        let total_pages = result["document"]["pages"]
            .as_array()
            .map_or(0, |p| p.len());
        let mut transactions = Vec::new();
        let mut opening_balance = 0.0;
        let mut closing_balance = 0.0;
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

                        transactions.push(Transaction {
                            page: 0,
                            line_on_page: idx,
                            date,
                            raw_text: description,
                            debit,
                            credit,
                            running_balance,
                            bbox: None,
                            provenance: Provenance::DocumentAI { confidence },
                        });
                    }

                    // Some processors emit "transaction" directly with the
                    // same property layout — keep this branch as a fallback.
                    "transaction" => {
                        transactions.push(Transaction {
                            page: 0,
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
                            bbox: None,
                            provenance: Provenance::DocumentAI { confidence },
                        });
                    }

                    "starting_balance" | "opening_balance" => {
                        if let Ok(v) = text.replace(['$', ','], "").parse::<f64>() {
                            opening_balance = v;
                        }
                    }
                    "ending_balance" | "closing_balance" => {
                        if let Ok(v) = text.replace(['$', ','], "").parse::<f64>() {
                            closing_balance = v;
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

fn extract_number_property(entity: &serde_json::Value, kind: &str) -> Option<f64> {
    extract_string_property(entity, kind).and_then(|s| s.replace(['$', ','], "").parse().ok())
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
        assert_eq!(tx.credit, Some(242.83));
        assert_eq!(tx.debit, None);
        assert_eq!(stmt.opening_balance, 1000.00);
        assert_eq!(stmt.closing_balance, 1242.83);
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
        assert_eq!(stmt.transactions[0].debit, Some(3.50));
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
        assert_eq!(extract_number_property(&entity, "debit"), Some(3.50));
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
                service_account_path: String::new(),
                api_key: String::new(),
                adc_path: String::new(),
            }),
            ..AppConfig::default()
        };
        let res = DocumentAiClient::from_app_config(&cfg);
        assert!(matches!(res, Err(DocAiError::MissingConfig(_))));
    }
}

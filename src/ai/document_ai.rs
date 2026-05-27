//! Google Document AI client.
//!
//! Auth strategy:
//!  1. **Primary (Beta):** API key via the `v1beta3` endpoint with `?key=...`.
//!  2. **Fallback (legacy):** Service-account JWT against the `v1` endpoint.
//!
//! If both are configured the API key is tried first. If the API-key call
//! fails with an auth-class error (401/403) we automatically retry with the
//! service-account path. Network errors are not retried (caller decides).
//!
//! The response shape is the same on either endpoint, so the parser is shared.

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

#[derive(Debug, Clone)]
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
        if doc_ai.api_key.is_empty() && doc_ai.service_account_path.is_empty() {
            return Err(DocAiError::MissingConfig(
                "DOCUMENT_AI_API_KEY or GOOGLE_APPLICATION_CREDENTIALS",
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

    pub async fn parse_entire_statement(&self, pdf_path: &Path) -> Result<BankStatement, DocAiError> {
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
                    return Self::parse_response_into_bank_statement(&result);
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
                    tracing::warn!("[doc_ai] API-key request failed: {}; trying service-account", e);
                }
            }
        }

        // 2. Fallback: service-account JWT → v1.
        if self.config.service_account_path.is_empty() {
            return Err(DocAiError::MissingConfig(
                "service_account_path (no fallback available)",
            ));
        }

        let access_token = self.get_access_token().await?;
        let url = self.process_url_v1();
        tracing::debug!("[doc_ai] using v1 service-account auth");
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
        Self::parse_response_into_bank_statement(&result)
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
                let text = entity["mentionText"].as_str().unwrap_or("").trim().to_string();
                let confidence = entity["confidence"].as_f64().unwrap_or(0.0) as f32;

                match etype {
                    "transaction" => {
                        transactions.push(Transaction {
                            page: 0,
                            line_on_page: idx,
                            date: extract_string_property(entity, "transaction_date").unwrap_or_default(),
                            raw_text: text,
                            debit: extract_number_property(entity, "debit"),
                            credit: extract_number_property(entity, "credit"),
                            running_balance: extract_number_property(entity, "running_balance"),
                            bbox: None,
                            provenance: Provenance::DocumentAI { confidence },
                        });
                    }
                    "opening_balance" => {
                        if let Ok(v) = text.replace(['$', ','], "").parse::<f64>() {
                            opening_balance = v;
                        }
                    }
                    "closing_balance" | "ending_balance" => {
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
    entity["properties"]
        .as_array()
        .and_then(|props| {
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
        let json_str = r#"{
            "document": {
                "pages": [{}],
                "entities": [
                    { "type": "transaction", "mentionText": "Test 1", "confidence": 0.9 },
                    { "type": "transaction", "mentionText": "Test 2", "confidence": 0.85 }
                ]
            }
        }"#;
        let val: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let stmt = DocumentAiClient::parse_response_into_bank_statement(&val).unwrap();
        assert_eq!(stmt.total_pages, 1);
        assert_eq!(stmt.transactions.len(), 2);
        match stmt.transactions[0].provenance {
            Provenance::DocumentAI { confidence } => assert_eq!(confidence, 0.9),
            _ => panic!("Wrong provenance"),
        }
    }

    #[test]
    fn jwt_claim_shape() {
        let key_content = std::fs::read_to_string("tests/fixtures/test_service_account.json").unwrap();
        let service_account: serde_json::Value = serde_json::from_str(&key_content).unwrap();
        let client_email = service_account["client_email"].as_str().unwrap();
        let private_key = service_account["private_key"].as_str().unwrap();

        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
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
        let payload_bytes =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1]).unwrap();
        let token_data: serde_json::Value = serde_json::from_slice(&payload_bytes).unwrap();
        assert_eq!(token_data["iss"].as_str().unwrap(), client_email);
        assert_eq!(token_data["aud"].as_str().unwrap(), "https://oauth2.googleapis.com/token");
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
        assert_eq!(extract_string_property(&entity, "transaction_date").as_deref(), Some("2026-05-01"));
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
            }),
            ..AppConfig::default()
        };
        let res = DocumentAiClient::from_app_config(&cfg);
        assert!(matches!(res, Err(DocAiError::MissingConfig(_))));
    }
}

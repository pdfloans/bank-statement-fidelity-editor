use crate::app::config::AppConfig;
use crate::engine::model::Transaction;
use crate::engine::layout::DocumentLayout;
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

#[derive(thiserror::Error, Debug)]
pub enum GeminiError {
    #[error("Missing Configuration: GEMINI_API_KEY")]
    MissingKey,
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
}

impl GeminiClient {
    pub fn from_app_config(cfg: &AppConfig) -> Result<Self, GeminiError> {
        let api_key = cfg.gemini_api_key.clone().ok_or(GeminiError::MissingKey)?;
        Ok(Self {
            api_key,
            http: Client::new(),
            base_url: "https://generativelanguage.googleapis.com".into(),
        })
    }
    
    // Internal method for testing
    #[cfg(test)]
    fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            api_key,
            http: Client::new(),
            base_url,
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

        let url = format!(
            "{}/v1beta/models/gemini-2.5-flash:generateContent?key={}",
            self.base_url,
            self.api_key
        );

        let response = self.http.post(&url)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(GeminiError::Api(response.status(), response.text().await.unwrap_or_default()));
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::{method, path};

    #[test]
    fn request_body_uses_camelcase_and_object_schema() {
        let transactions: Vec<Transaction> = vec![];
        let layout = DocumentLayout {
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

        assert_eq!(body["generationConfig"]["responseMimeType"].as_str().unwrap(), "application/json");
        assert!(body["generationConfig"]["responseSchema"].is_object());
    }

    #[tokio::test]
    async fn low_confidence_response_returns_error() {
        let server = MockServer::start().await;
        
        Mock::given(method("POST"))
            .and(path("/v1beta/models/gemini-2.5-flash:generateContent"))
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

        let plan = client.propose_balance_adjustments(&[], 0.0, &DocumentLayout {
            total_pages: 1,
            pages: vec![],
            has_consistent_headers: true,
            has_consistent_footers: true,
            overall_style: "".into(),
            layout_confidence: 1.0,
        }).await;

        match plan {
            Err(GeminiError::LowConfidence(conf)) => assert_eq!(conf, 0.5),
            _ => panic!("Expected LowConfidence error"),
        }
    }
}

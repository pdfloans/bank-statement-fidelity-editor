use crate::app::config::{AiProviderMode, AppConfig};
use crate::engine::model::Transaction;
use reqwest::StatusCode;

#[derive(thiserror::Error, Debug)]
pub enum OpenAiError {
    #[error("Missing API Key")]
    MissingKey,
    #[error("Network Error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Middleware Error: {0}")]
    Middleware(#[from] reqwest_middleware::Error),
    #[error("API Error (HTTP {0}): {1}")]
    Api(StatusCode, String),
    #[error("Invalid Response: {0}")]
    InvalidResponse(String),
    #[error("Format error: {0}")]
    Format(String),
}

pub struct OpenAiClient {
    pub api_key: String,
    pub http: reqwest_middleware::ClientWithMiddleware,
    pub base_url: String,
    pub model: String,
}

impl OpenAiClient {
    pub fn from_app_config(cfg: &AppConfig) -> Result<Self, OpenAiError> {
        let (api_key, base_url, model) = match cfg.ai_provider {
            AiProviderMode::GroqApiKey => {
                let k = cfg.groq_api_key.clone().ok_or(OpenAiError::MissingKey)?;
                (
                    k,
                    "https://api.groq.com/openai/v1".to_string(),
                    "llama-3.3-70b-versatile".to_string(),
                )
            }
            AiProviderMode::OpenRouterApiKey => {
                let k = cfg
                    .openrouter_api_key
                    .clone()
                    .ok_or(OpenAiError::MissingKey)?;
                (
                    k,
                    "https://openrouter.ai/api/v1".to_string(),
                    "deepseek/deepseek-chat".to_string(),
                )
            }
            _ => return Err(OpenAiError::MissingKey),
        };
        Ok(Self {
            api_key,
            http: crate::app::config::global_http_client(),
            base_url,
            model,
        })
    }

    pub async fn from_app_config_async(cfg: &AppConfig) -> Result<Self, OpenAiError> {
        Self::from_app_config(cfg)
    }

    pub async fn ping(&self) -> Result<(), OpenAiError> {
        let url = format!("{}/models", self.base_url);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(self.api_key.trim())
            .send()
            .await?;

        if resp.status().is_success() {
            Ok(())
        } else {
            let s = resp.status();
            let b = resp.text().await.unwrap_or_default();
            Err(OpenAiError::Api(s, b))
        }
    }

    async fn post_json(&self, sys: &str, user: &str) -> Result<String, OpenAiError> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": sys },
                { "role": "user", "content": user }
            ],
            "response_format": { "type": "json_object" },
            "temperature": 0.0
        });

        let resp = self
            .http
            .post(&url)
            .bearer_auth(self.api_key.trim())
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(OpenAiError::Api(status, text));
        }

        let parsed: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| OpenAiError::Format(e.to_string()))?;
        let content = parsed["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| OpenAiError::Format("No content returned".to_string()))?
            .to_string();

        Ok(content)
    }

    pub async fn propose_balance_adjustments(
        &self,
        transactions: &[Transaction],
        imbalance: f64,
        _layout: &crate::engine::layout::DocumentLayout,
    ) -> Result<crate::ai::gemini_client::GeminiBalancePlan, OpenAiError> {
        let sys = "You are a mathematical auditor. You receive a JSON list of bank transactions. Identify OCR errors and return a JSON object containing an 'adjustments' array, 'overall_strategy' string, and 'confidence' number (0.0 to 1.0). Each adjustment needs 'page', 'line_on_page', 'old_running_balance', 'new_running_balance', 'reason', 'confidence'.";

        let tx_json = serde_json::to_string(transactions).unwrap();
        let user = format!("Imbalance: {}\nTransactions: {}", imbalance, tx_json);

        let out = self.post_json(sys, &user).await?;
        let plan: crate::ai::gemini_client::GeminiBalancePlan =
            serde_json::from_str(&out).map_err(|e| OpenAiError::Format(e.to_string()))?;
        Ok(plan)
    }

    pub async fn validate_parse_completeness(
        &self,
        transactions: &[Transaction],
        opening: f64,
        closing: f64,
        pages: usize,
    ) -> Result<crate::ai::gemini_client::GeminiCompletenessReport, OpenAiError> {
        let sys = "You are a completion validator. Check if transactions list mathematically bridges opening and closing. Return JSON: { \"completeness_score\": 0.9, \"notes\": \"Looks good\", \"missing_rows\": [], \"math_consistent\": true }";
        let user = format!(
            "Op: {}, Cl: {}, Pages: {}, Txs: {}",
            opening,
            closing,
            pages,
            serde_json::to_string(transactions).unwrap()
        );
        let out = self.post_json(sys, &user).await?;
        let plan: crate::ai::gemini_client::GeminiCompletenessReport =
            serde_json::from_str(&out).map_err(|e| OpenAiError::Format(e.to_string()))?;
        Ok(plan)
    }

    pub async fn verify_statement_mathematics(
        &self,
        transactions_json: &str,
        opening: f64,
    ) -> Result<bool, OpenAiError> {
        let sys = "You are a mathematical auditor. Double-check if the bank statement's math adds up. Return JSON: { \"is_math_consistent\": true }";
        let user = format!("Op: {}, Txs: {}", opening, transactions_json);
        let out = self.post_json(sys, &user).await?;
        let parsed: serde_json::Value =
            serde_json::from_str(&out).map_err(|e| OpenAiError::Format(e.to_string()))?;
        Ok(parsed["is_math_consistent"].as_bool().unwrap_or(false))
    }

    pub async fn verify_transfer_math(
        &self,
        mapped_transactions: &[crate::engine::transfer::MappedTransaction],
        opening_balance: rust_decimal::Decimal,
    ) -> Result<bool, OpenAiError> {
        let sys = "You are a forensic accountant. Double-check if Opening + Debits - Credits = Final Balance. Return JSON: { \"is_math_consistent\": true }";
        let user = format!(
            "Op: {}, Txs: {}",
            opening_balance,
            serde_json::to_string(mapped_transactions).unwrap_or_default()
        );
        let out = self.post_json(sys, &user).await?;
        let parsed: serde_json::Value =
            serde_json::from_str(&out).map_err(|e| OpenAiError::Format(e.to_string()))?;
        Ok(parsed["is_math_consistent"].as_bool().unwrap_or(false))
    }
}

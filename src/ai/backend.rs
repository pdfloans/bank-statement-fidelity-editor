use crate::ai::gemini_client::{GeminiClient, GeminiCompletenessReport, GeminiError};
use crate::ai::openai_client::{OpenAiClient, OpenAiError};
use crate::app::config::{AppConfig, AiProviderMode};
use crate::engine::model::Transaction;

pub enum AiBackend {
    Gemini(GeminiClient),
    OpenAi(OpenAiClient),
}

#[derive(thiserror::Error, Debug)]
pub enum AiBackendError {
    #[error("Gemini Error: {0}")]
    Gemini(#[from] GeminiError),
    #[error("OpenAI Error: {0}")]
    OpenAi(#[from] OpenAiError),
}

impl AiBackend {
    pub fn from_app_config(cfg: &AppConfig) -> Result<Self, AiBackendError> {
        match cfg.ai_provider {
            AiProviderMode::GroqApiKey | AiProviderMode::OpenRouterApiKey => {
                let c = OpenAiClient::from_app_config(cfg)?;
                Ok(Self::OpenAi(c))
            }
            AiProviderMode::GeminiApiKey | AiProviderMode::GeminiVertex => {
                let c = GeminiClient::from_app_config(cfg)?;
                Ok(Self::Gemini(c))
            }
            AiProviderMode::ManualOnly => Err(AiBackendError::Gemini(GeminiError::MissingKey)) // dummy
        }
    }

    pub async fn from_app_config_async(cfg: &AppConfig) -> Result<Self, AiBackendError> {
        match cfg.ai_provider {
            AiProviderMode::GroqApiKey | AiProviderMode::OpenRouterApiKey => {
                let c = OpenAiClient::from_app_config_async(cfg).await?;
                Ok(Self::OpenAi(c))
            }
            AiProviderMode::GeminiApiKey | AiProviderMode::GeminiVertex => {
                let c = GeminiClient::from_app_config_async(cfg).await?;
                Ok(Self::Gemini(c))
            }
            AiProviderMode::ManualOnly => Err(AiBackendError::Gemini(GeminiError::MissingKey)) // dummy
        }
    }

    pub async fn ping(&self) -> Result<(), AiBackendError> {
        match self {
            Self::Gemini(c) => c.ping().await.map_err(Into::into),
            Self::OpenAi(c) => c.ping().await.map_err(Into::into),
        }
    }

    pub async fn propose_balance_adjustments(
        &self,
        transactions: &[Transaction],
        imbalance: f64,
        layout: &crate::engine::layout::DocumentLayout,
    ) -> Result<crate::ai::gemini_client::GeminiBalancePlan, AiBackendError> {
        match self {
            Self::Gemini(c) => c.propose_balance_adjustments(transactions, imbalance, layout).await.map_err(Into::into),
            Self::OpenAi(c) => c.propose_balance_adjustments(transactions, imbalance, layout).await.map_err(Into::into),
        }
    }

    pub async fn validate_parse_completeness(
        &self,
        transactions: &[Transaction],
        opening: f64,
        closing: f64,
        pages: usize,
    ) -> Result<GeminiCompletenessReport, AiBackendError> {
        match self {
            Self::Gemini(c) => c.validate_parse_completeness(transactions, opening, closing, pages).await.map_err(Into::into),
            Self::OpenAi(c) => c.validate_parse_completeness(transactions, opening, closing, pages).await.map_err(Into::into),
        }
    }

    pub async fn verify_statement_mathematics(
        &self,
        transactions_json: &str,
        opening: f64,
    ) -> Result<bool, AiBackendError> {
        match self {
            Self::Gemini(c) => c.verify_statement_mathematics(transactions_json, opening).await.map_err(Into::into),
            Self::OpenAi(c) => c.verify_statement_mathematics(transactions_json, opening).await.map_err(Into::into),
        }
    }

    // Pass-through stubs for vision methods not supported by text-only OpenAI models.
    pub async fn validate_render_visually(
        &self,
        _doc: &Vec<u8>,
        _bboxes: &[[f32; 4]],
    ) -> Result<crate::ai::gemini_client::GeminiVisionReport, AiBackendError> {
        match self {
            Self::Gemini(c) => c.validate_render_visually(_doc, _bboxes).await.map_err(Into::into),
            Self::OpenAi(_) => Ok(crate::ai::gemini_client::GeminiVisionReport {
                anomaly_score: 0.0,
                hotspots: vec![],
                notes: "Vision check bypassed for text-only model".into(),
            }),
        }
    }

    pub async fn plan_transaction_transfer(
        &self,
        source_transactions: &[Transaction],
        target_transactions: &[Transaction],
        correction_hint: Option<&str>,
    ) -> Result<crate::engine::transfer::TransferPlan, AiBackendError> {
        match self {
            Self::Gemini(c) => c.plan_transaction_transfer(source_transactions, target_transactions, correction_hint).await.map_err(Into::into),
            Self::OpenAi(_) => Err(AiBackendError::OpenAi(OpenAiError::Format("Transfer planning requires Vision AI (Gemini)".into()))),
        }
    }

    pub async fn verify_transfer_math(
        &self,
        mapped_transactions: &[crate::engine::transfer::MappedTransaction],
        opening_balance: rust_decimal::Decimal,
    ) -> Result<bool, AiBackendError> {
         match self {
            Self::Gemini(c) => c.verify_transfer_math(mapped_transactions, opening_balance).await.map_err(Into::into),
            Self::OpenAi(c) => c.verify_transfer_math(mapped_transactions, opening_balance).await.map_err(Into::into),
        }
    }
}

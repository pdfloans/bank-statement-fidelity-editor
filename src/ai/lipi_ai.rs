use crate::error::{LipiAIError, LipiAIResult};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const LIPI_API_URL: &str = "https://api.lipi.ai/v1/font/complete";

#[derive(Debug, Clone, Serialize)]
pub struct LipiFontRequest {
    pub text: String,
    pub image_base64: String, // Context snippet around the text
}

#[derive(Debug, Clone, Deserialize)]
pub struct LipiFontResponse {
    pub font_name: String,
    pub confidence: f32,
    pub size_pts: Option<f32>,
    pub is_bold: Option<bool>,
    pub is_italic: Option<bool>,
    // Could include raw TTF bytes if Lipi actually synthesizes it
    pub font_bytes_base64: Option<String>,
}

pub struct LipiClient {
    client: Client,
    api_key: String,
}

impl LipiClient {
    pub fn new(api_key: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_default();
        Self { client, api_key }
    }

    pub async fn predict_font(&self, request: &LipiFontRequest) -> LipiAIResult<LipiFontResponse> {
        if self.api_key.is_empty() {
            return Err(LipiAIError::MissingApiKey("Lipi.ai".to_string()));
        }

        let res = self
            .client
            .post(LIPI_API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(request)
            .send()
            .await
            .map_err(|e| LipiAIError::Network(e.to_string()))?;

        if !res.status().is_success() {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            return Err(LipiAIError::ApiError(format!(
                "Lipi API failed with status {status}: {body}"
            )));
        }

        let resp: LipiFontResponse = res
            .json()
            .await
            .map_err(|e| LipiAIError::Serialization(e.to_string()))?;

        Ok(resp)
    }
}

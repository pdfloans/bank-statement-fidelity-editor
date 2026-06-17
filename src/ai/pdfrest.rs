//! pdfRest AI Client for High-Fidelity Rendering
//!
//! Provides Adobe-quality rendering by delegating to the pdfRest PDF-to-Images API.
//! This serves as the "Gold Standard" for visual verification (Approach §3.4).

use reqwest::multipart;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;
use tokio::fs;
use tokio::time::sleep;

#[derive(Error, Debug)]
pub enum PdfRestError {
    #[error("Failed to upload PDF: {0}")]
    Upload(String),
    #[error("Failed to poll job status: {0}")]
    Poll(String),
    #[error("Failed to download result: {0}")]
    Download(String),
    #[error("Operation timed out during {stage}")]
    Timeout { stage: &'static str },
    #[error("Authentication failed: Check your PDFREST_API_KEY")]
    Auth,
    #[error("Unexpected response from API: {0}")]
    BadResponse(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub struct PdfRestClient {
    api_key: String,
    http: reqwest_middleware::ClientWithMiddleware,
    base_url: String,
}

impl std::fmt::Debug for PdfRestClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PdfRestClient")
            .field(
                "api_key",
                &format!("<masked: {} chars>", self.api_key.len()),
            )
            .field("base_url", &self.base_url)
            .finish()
    }
}

#[derive(Debug, Deserialize)]
struct UploadResponse {
    #[serde(rename = "outputId")]
    output_id: Option<String>,
    #[serde(rename = "outputUrl")]
    output_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResourceResponse {
    #[serde(rename = "outputUrl")]
    output_url: Option<String>,
}

impl PdfRestClient {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            http: crate::app::config::global_http_client(),
            base_url: "https://api.pdfrest.com".into(),
        }
    }

    #[doc(hidden)]
    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        let mut client = Self::new(api_key);
        client.base_url = base_url;
        client
    }

    fn authed_request(
        &self,
        method: reqwest::Method,
        url: &str,
    ) -> reqwest_middleware::RequestBuilder {
        self.http
            .request(method, url)
            .header("Api-Key", &self.api_key)
    }

    /// Renders a PDF to PNG images using pdfRest.
    /// Returns a list of PathBufs to the downloaded images.
    pub async fn render_pdf_to_images(
        &self,
        pdf: &Path,
        out_dir: &Path,
        dpi: u32,
    ) -> Result<Vec<PathBuf>, PdfRestError> {
        fs::create_dir_all(out_dir).await?;

        let file = fs::File::open(pdf).await?;
        let stream = tokio_util::codec::FramedRead::new(file, tokio_util::codec::BytesCodec::new());
        let body = reqwest::Body::wrap_stream(stream);
        let filename = pdf
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("document.pdf")
            .to_string();

        let form = multipart::Form::new()
            .part(
                "file",
                multipart::Part::stream(body)
                    .file_name(filename)
                    .mime_str("application/pdf")
                    .map_err(|e| PdfRestError::Upload(e.to_string()))?,
            )
            .text("output_type", "png")
            .text("resolution", dpi.to_string());

        let url = format!("{}/pdf-to-images", self.base_url);
        let resp = self
            .authed_request(reqwest::Method::POST, &url)
            .multipart(form)
            .send()
            .await
            .map_err(|e| PdfRestError::Upload(e.to_string()))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED
            || resp.status() == reqwest::StatusCode::FORBIDDEN
        {
            return Err(PdfRestError::Auth);
        }

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(PdfRestError::Upload(body));
        }

        let upload_res: UploadResponse = resp
            .json()
            .await
            .map_err(|e| PdfRestError::BadResponse(e.to_string()))?;

        let mut output_urls = Vec::new();
        if let Some(url) = upload_res.output_url {
            output_urls.push(url);
        } else if let Some(id) = upload_res.output_id {
            // Poll for completion
            let poll_url = format!("{}/resource/{}", self.base_url, id);
            let mut attempts = 0;
            let max_attempts = 60;

            loop {
                if attempts >= max_attempts {
                    return Err(PdfRestError::Timeout { stage: "poll" });
                }

                let poll_resp = self
                    .authed_request(reqwest::Method::GET, &poll_url)
                    .send()
                    .await
                    .map_err(|e| PdfRestError::Poll(e.to_string()))?;

                if poll_resp.status().is_success() {
                    let res: ResourceResponse = poll_resp
                        .json()
                        .await
                        .map_err(|e| PdfRestError::Poll(e.to_string()))?;
                    if let Some(url) = res.output_url {
                        output_urls.push(url);
                        break;
                    }
                }

                attempts += 1;
                sleep(Duration::from_secs(1)).await;
            }
        } else {
            return Err(PdfRestError::BadResponse(
                "No outputUrl or outputId in response".into(),
            ));
        }

        let mut downloaded_paths = Vec::new();
        for (i, url) in output_urls.into_iter().enumerate() {
            let download_resp = self
                .authed_request(reqwest::Method::GET, &url)
                .send()
                .await
                .map_err(|e| PdfRestError::Download(e.to_string()))?;

            if !download_resp.status().is_success() {
                return Err(PdfRestError::Download(format!(
                    "Status: {}",
                    download_resp.status()
                )));
            }

            let bytes = download_resp
                .bytes()
                .await
                .map_err(|e| PdfRestError::Download(e.to_string()))?;
            let out_path = out_dir.join(format!("pdfrest_p{}.png", i + 1));
            fs::write(&out_path, bytes).await?;
            downloaded_paths.push(out_path);
        }

        Ok(downloaded_paths)
    }
}

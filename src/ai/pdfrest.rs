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
    http: reqwest::Client,
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
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_default();

        Self {
            api_key,
            http,
            base_url: "https://api.pdfrest.com".into(),
        }
    }

    #[doc(hidden)]
    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        let mut client = Self::new(api_key);
        client.base_url = base_url;
        client
    }

    fn authed_request(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
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

        let pdf_bytes = fs::read(pdf).await?;
        let filename = pdf
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("document.pdf")
            .to_string();

        let form = multipart::Form::new()
            .part(
                "file",
                multipart::Part::bytes(pdf_bytes)
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn happy_path_uploads_and_downloads() {
        let _ = dotenvy::dotenv();
        let server = MockServer::start().await;
        let api_key = "test-key".to_string();
        let client = PdfRestClient::with_base_url(api_key.clone(), server.uri());

        // 1. Mock Upload
        Mock::given(method("POST"))
            .and(path("/pdf-to-images"))
            .and(header("Api-Key", &api_key))
            .and(body_string_contains("resolution"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "outputUrl": format!("{}/download/1", server.uri())
            })))
            .expect(1)
            .mount(&server)
            .await;

        // 2. Mock Download
        let fake_png = b"fake-png-bytes";
        Mock::given(method("GET"))
            .and(path("/download/1"))
            .and(header("Api-Key", &api_key))
            .respond_with(ResponseTemplate::new(200).set_body_raw(fake_png.to_vec(), "image/png"))
            .expect(1)
            .mount(&server)
            .await;

        let temp_dir = tempdir().unwrap();
        let pdf_path = temp_dir.path().join("test.pdf");
        fs::write(&pdf_path, b"%PDF-1.4").await.unwrap();

        let results = client
            .render_pdf_to_images(&pdf_path, temp_dir.path(), 300)
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].exists());
        let content = fs::read(&results[0]).await.unwrap();
        assert_eq!(content, fake_png);
    }

    #[tokio::test]
    async fn auth_failure_returns_pdfrest_error_auth() {
        let server = MockServer::start().await;
        let client = PdfRestClient::with_base_url("bad-key".into(), server.uri());

        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let temp_dir = tempdir().unwrap();
        let pdf_path = temp_dir.path().join("test.pdf");
        fs::write(&pdf_path, b"%PDF-1.4").await.unwrap();

        let result = client
            .render_pdf_to_images(&pdf_path, temp_dir.path(), 300)
            .await;

        match result {
            Err(PdfRestError::Auth) => {}
            _ => panic!("Expected Auth error, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn poll_path_completes_after_running_state() {
        let server = MockServer::start().await;

        let client = PdfRestClient::with_base_url("test_key".into(), server.uri());

        let work_dir = tempdir().unwrap();
        let pdf_path = work_dir.path().join("test.pdf");
        tokio::fs::write(&pdf_path, b"%PDF-1.4").await.unwrap();

        let out_dir = tempdir().unwrap();

        // 1. Initial POST returns outputId
        Mock::given(method("POST"))
            .and(path("/pdf-to-images"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "outputId": "poll123"
            })))
            .mount(&server)
            .await;

        // 2. First GET returns no outputUrl (running state)
        Mock::given(method("GET"))
            .and(path("/resource/poll123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "processing"
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        // 3. Second GET returns outputUrl (done state)
        let download_url = format!("{}/dl/file.png", server.uri());
        Mock::given(method("GET"))
            .and(path("/resource/poll123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "outputUrl": download_url
            })))
            .mount(&server)
            .await;

        // 4. Download file request
        Mock::given(method("GET"))
            .and(path("/dl/file.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"fake_png".to_vec()))
            .mount(&server)
            .await;

        let result = client
            .render_pdf_to_images(&pdf_path, out_dir.path(), 300)
            .await
            .unwrap();
        assert_eq!(result.len(), 1);

        let dl_content = tokio::fs::read(&result[0]).await.unwrap();
        assert_eq!(dl_content, b"fake_png");
    }
}

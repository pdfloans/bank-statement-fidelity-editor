use pdfium_render::prelude::Pdfium;
use std::time::Duration;
use tokio::net::TcpStream;

#[test]
fn test_pdfium_library_loads() {
    let bindings = Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./"))
        .or_else(|_| Pdfium::bind_to_system_library());

    assert!(
        bindings.is_ok(),
        "FATAL: libpdfium.dylib (or platform equivalent) is missing or cannot be loaded! Dependency check failed. Error: {:?}", bindings.err().unwrap()
    );
}

#[tokio::test]
async fn test_ai_provider_dns_resolution() {
    let endpoints = vec![
        "generativelanguage.googleapis.com:443", // Gemini AI Studio
        "us-central1-aiplatform.googleapis.com:443", // Vertex AI
        "api.groq.com:443",                      // Groq
        "openrouter.ai:443",                     // OpenRouter
        "us-documentai.googleapis.com:443",      // Document AI
    ];

    for endpoint in endpoints {
        let result =
            tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(endpoint)).await;

        assert!(
            result.is_ok() && result.unwrap().is_ok(),
            "FATAL: Could not resolve or connect to AI provider {}. Network/DNS blocked?",
            endpoint
        );
    }
}

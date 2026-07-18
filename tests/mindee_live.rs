use dual_core_pdf_pipeline::ai::mindee::MindeeClient;
use dual_core_pdf_pipeline::app::config::AppConfig;
use std::sync::Arc;

#[tokio::test]
async fn test_mindee_live() {
    let cfg = match AppConfig::from_env() {
        Ok(c) => Arc::new(c),
        Err(e) => {
            println!("Skipping mindee live test due to missing config: {}", e);
            return;
        }
    };
    
    // We just check if the client can be built and if the key is valid.
    let mindee = MindeeClient::from_app_config(&cfg);
    assert!(mindee.is_ok(), "Failed to create mindee client");
}

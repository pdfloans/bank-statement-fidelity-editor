use dual_core_pdf_pipeline::ai::backend::AiBackend;
use dual_core_pdf_pipeline::ai::gemini_client::GeminiClient;
use dual_core_pdf_pipeline::app::config::AiProviderMode;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_transfer_test_loop_retry() {
    let _ = dotenvy::dotenv();
    let server = MockServer::start().await;

    let mock_response = r#"{
        "candidates": [
            {
                "content": {
                    "parts": [
                        {
                            "text": "{\"mappings\": [{\"source_index\": 0, \"target_page\": 1, \"target_line\": 1, \"converted_date\": \"01/01/2023\", \"adapted_description\": \"Adapted\"}], \"output_page_count\": 1, \"pages_to_clone\": [], \"pages_to_remove\": [], \"strategy\": \"Simple\", \"confidence\": 0.99}"
                        }
                    ]
                }
            }
        ]
    }"#;

    Mock::given(method("POST"))
        .and(path("/v1beta/models/gemini-pro-latest:generateContent"))
        .respond_with(ResponseTemplate::new(200).set_body_string(mock_response))
        .mount(&server)
        .await;

    let backend = AiBackend {
        primary: AiProviderMode::GeminiApiKey,
        gemini: Some(GeminiClient::with_base_url("test_key".into(), server.uri())),
        openrouter: None,
        groq: None,
    };

    use dual_core_pdf_pipeline::engine::model::{Provenance, Transaction};
    use rust_decimal_macros::dec;

    let tx1 = Transaction {
        page: 1,
        line_on_page: 1,
        date: "2023-01-01".into(),
        raw_text: "Target tx".into(),
        debit: None,
        credit: Some(dec!(100.0)),
        running_balance: None,
        bbox: None,
        field_bboxes: Default::default(),
        provenance: Provenance::Computed,
        category: None,
    };

    let result = backend
        .plan_transaction_transfer(
            &[tx1.clone()],
            &[tx1.clone()],
            Some("Format output as a single list of strings"),
        )
        .await;

    if let Err(e) = &result {
        println!("Error: {}", e);
    }
    assert!(result.is_ok());
    let mapped = result.unwrap();
    assert_eq!(mapped.mappings.len(), 1);
}

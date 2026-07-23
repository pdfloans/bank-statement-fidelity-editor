use dual_core_pdf_pipeline::ai::backend::AiBackend;
use dual_core_pdf_pipeline::app::config::AppConfig;
use std::sync::Arc;

#[tokio::test]
async fn test_transfer_test_loop_retry() {
    let _ = dotenvy::dotenv();
    
    let cfg = Arc::new(AppConfig::from_env().unwrap());
    let backend = AiBackend::from_app_config_async(&cfg).await.unwrap();

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
            std::slice::from_ref(&tx1),
            std::slice::from_ref(&tx1),
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

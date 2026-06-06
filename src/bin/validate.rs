// src/bin/validate.rs
use dual_core_pdf_pipeline::ai::gemini_client::GeminiClient;
use dual_core_pdf_pipeline::app::config::AppConfig;
use dual_core_pdf_pipeline::engine::model::{Transaction, Provenance};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Load configuration from environment (includes GEMINI_API_KEY if set)
    let cfg = AppConfig::from_env()?;
    let gemini = GeminiClient::from_app_config_async(&cfg).await?;

    // Dummy transaction list matching the Transaction struct definition
    let transactions = vec![
        Transaction {
            page: 0,
            line_on_page: 0,
            date: "2022-01-01".to_string(),
            raw_text: "Deposit".to_string(),
            debit: Some(Decimal::new(5000, 2)), // $50.00
            credit: None,
            running_balance: Some(Decimal::new(5000, 2)),
            bbox: None,
            field_bboxes: Default::default(),
            provenance: Provenance::Manual,
        },
        Transaction {
            page: 0,
            line_on_page: 1,
            date: "2022-01-02".to_string(),
            raw_text: "Withdrawal".to_string(),
            debit: None,
            credit: Some(Decimal::new(2000, 2)), // $20.00
            running_balance: Some(Decimal::new(3000, 2)),
            bbox: None,
            field_bboxes: Default::default(),
            provenance: Provenance::Manual,
        },
    ];

    // Opening balance as f64 (for the Gemini API signature)
    let opening_balance: f64 = 0.0;
    // Compute closing balance from the dummy data
    let mut closing_balance = opening_balance;
    for tx in &transactions {
        if let Some(c) = tx.credit {
            closing_balance -= c.to_f64().unwrap_or(0.0);
        }
        if let Some(d) = tx.debit {
            closing_balance += d.to_f64().unwrap_or(0.0);
        }
    }
    let total_pages: usize = 2;

    match gemini
        .validate_parse_completeness(&transactions, opening_balance, closing_balance, total_pages)
        .await
    {
        Ok(report) => println!("Completeness validation result: {:?}", report),
        Err(e) => eprintln!("Error during validation: {}", e),
    }
    Ok(())
}

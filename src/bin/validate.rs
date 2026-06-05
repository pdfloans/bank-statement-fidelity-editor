// src/bin/validate.rs
use dual_core_pdf_pipeline::ai::gemini_client::GeminiClient;
use dual_core_pdf_pipeline::engine::model::Transaction;
use rust_decimal::Decimal;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize Gemini client (placeholder values, replace with real config if needed)
    let gemini = GeminiClient::new()?;

    // Dummy transaction list for testing
    let transactions = vec![
        Transaction {
            date: "2022-01-01".to_string(),
            description: "Deposit".to_string(),
            debit: None,
            credit: Some(Decimal::new(5000, 2)), // $50.00
            running_balance: Decimal::new(5000, 2),
        },
        Transaction {
            date: "2022-01-02".to_string(),
            description: "Withdrawal".to_string(),
            debit: Some(Decimal::new(2000, 2)), // $20.00
            credit: None,
            running_balance: Decimal::new(3000, 2),
        },
    ];
    let opening_balance = Decimal::new(0, 2);

    match gemini.validate_parse_completeness(&transactions, opening_balance, opening_balance, 1).await {
        Ok(valid) => println!("Completeness validation result: {}", valid),
        Err(e) => eprintln!("Error during validation: {}", e),
    }
    Ok(())
}

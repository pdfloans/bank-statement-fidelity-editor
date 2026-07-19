use crate::ai::document_ai::BankStatement;
use crate::engine::model::Transaction;
use crate::engine::balance::recalculate_and_validate;
use crate::ai::backend::AiBackend;
use rust_decimal::Decimal;

pub async fn verify_and_repair_extraction(
    backend: &AiBackend,
    mut stmt: BankStatement,
    raw_ocr_text: &str,
) -> Result<BankStatement, String> {
    // 1. Check math locally
    let res = recalculate_and_validate(stmt.transactions.clone(), stmt.opening_balance);
    
    let mut needs_repair = false;
    let mut error_msg = String::new();

    match res {
        Ok(validated_tx) => {
            stmt.transactions = validated_tx;
            // Now check closing balance
            let closing = stmt.closing_balance; {
                let calc_closing = stmt.transactions.last().map(|tx| tx.running_balance.unwrap_or(stmt.opening_balance)).unwrap_or(stmt.opening_balance);
                let diff = (closing - calc_closing).abs();
                if diff >= rust_decimal_macros::dec!(0.01) {
                    needs_repair = true;
                    error_msg = format!("Final balance mismatch. Expected: {}, Calculated: {}. Diff: {}", closing, calc_closing, diff);
                }
            }
        }
        Err(e) => {
            needs_repair = true;
            error_msg = e.to_string();
        }
    }

    if !needs_repair {
        return Ok(stmt);
    }

    tracing::warn!("[repair] Extraction math verification failed: {}. Attempting AI repair...", error_msg);
    
    // 2. Call AI to repair the transactions based on raw OCR text
    let repaired_transactions = backend.repair_extracted_transactions(
        &stmt.transactions,
        stmt.opening_balance,
        stmt.closing_balance,
        raw_ocr_text,
        &error_msg
    ).await.map_err(|e| format!("AI repair failed: {}", e))?;
    
    stmt.transactions = repaired_transactions;
    
    // Validate the repaired statement
    if let Err(e2) = recalculate_and_validate(stmt.transactions.clone(), stmt.opening_balance) {
        tracing::error!("[repair] AI repair failed to fix math: {}", e2);
    } else {
        tracing::info!("[repair] AI extraction repair was successful!");
    }
    
    Ok(stmt)
}

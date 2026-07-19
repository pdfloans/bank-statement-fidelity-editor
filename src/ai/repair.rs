use crate::app::config::AppConfig;
use crate::ai::document_ai::BankStatement;
use crate::engine::model::Transaction;
use crate::engine::balance::recalculate_and_validate;
use crate::ai::backend::AiBackend;

pub async fn verify_and_repair_extraction(
    cfg: &AppConfig,
    mut stmt: BankStatement,
    raw_ocr_text: &str,
) -> Result<BankStatement, String> {
    // 1. Check math locally
    let res = recalculate_and_validate(stmt.transactions.clone(), stmt.opening_balance);
    
    match res {
        Ok(validated_tx) => {
            stmt.transactions = validated_tx;
            Ok(stmt)
        }
        Err(e) => {
            tracing::warn!("[repair] Extraction math verification failed: {}. Attempting AI repair...", e);
            
            // 2. Call AI to repair the transactions based on raw OCR text
            let backend = AiBackend::from_app_config_async(cfg)
                .await
                .map_err(|e| format!("Failed to init AI backend: {}", e))?;
                
            let repaired_transactions = backend.repair_extracted_transactions(
                &stmt.transactions,
                stmt.opening_balance,
                stmt.closing_balance,
                raw_ocr_text,
                &e.to_string()
            ).await.map_err(|e| format!("AI repair failed: {}", e))?;
            
            stmt.transactions = repaired_transactions;
            
            // Validate the repaired statement
            if let Err(e2) = recalculate_and_validate(stmt.transactions.clone(), stmt.opening_balance) {
                tracing::error!("[repair] AI repair failed to fix math: {}", e2);
                // Return the original statement anyway and let the UI handle the error later? No, let's return it but warn.
            } else {
                tracing::info!("[repair] AI extraction repair was successful!");
            }
            
            Ok(stmt)
        }
    }
}

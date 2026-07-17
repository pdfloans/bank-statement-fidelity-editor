import sys

def main():
    with open("src/app/runtime.rs", "r") as f:
        code = f.read()

    target = "                            let validation = crate::engine::workflow::ParseValidation {"
    
    gemini_block = """
                            use crate::app::config::AiProviderMode;

                            let (score, notes, missing, _math_ok) = match ai_provider {
                                AiProviderMode::ManualOnly => {
                                    let _ = res_tx.send(JobResult::Progress { label: "AI validation skipped (Manual Only mode)".into(), fraction: 0.7 });
                                    (0.8, "AI validation skipped (Manual Only mode).".into(), vec![], false)
                                }
                                _ => {
                                    let _ = res_tx.send(JobResult::Progress { label: "Asking Gemini to validate completeness".into(), fraction: 0.7 });

                                    let gemini_init_and_validate = async {
                                        let g = crate::ai::backend::AiBackend::from_app_config_async(&cfg).await?;
                                        g.validate_parse_completeness(
                                            &stmt.transactions,
                                            crate::engine::model::dec_to_f64(stmt.opening_balance),
                                            crate::engine::model::dec_to_f64(stmt.closing_balance),
                                            stmt.total_pages,
                                        ).await
                                    };

                                    match tokio::time::timeout(std::time::Duration::from_secs(30), gemini_init_and_validate).await {
                                        Ok(Ok(r)) => (r.completeness_score, r.notes, r.missing_rows, r.math_consistent),
                                        Ok(Err(e)) => {
                                            tracing::warn!("[workflow] Gemini validation failed: {e}; continuing");
                                            let _ = res_tx.send(JobResult::Progress { label: format!("AI validation skipped: {e}"), fraction: 0.7 });
                                            (0.7, format!("Gemini validation skipped: {e}"), vec![], false)
                                        }
                                        Err(_elapsed) => {
                                            tracing::warn!("[workflow] Gemini validation timed out after 30s; continuing without AI validation");
                                            let _ = res_tx.send(JobResult::Progress { label: "AI validation timed out after 30s".into(), fraction: 0.7 });
                                            (0.7, "Gemini validation timed out; skipped.".into(), vec![], false)
                                        }
                                    }
                                }
                            };

                            let validation = crate::engine::workflow::ParseValidation {"""
                            
    code = code.replace(target, gemini_block)
    
    with open("src/app/runtime.rs", "w") as f:
        f.write(code)

if __name__ == "__main__":
    main()

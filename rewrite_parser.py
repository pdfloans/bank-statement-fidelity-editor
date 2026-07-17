import sys

def main():
    with open("src/app/runtime.rs", "r") as f:
        lines = f.readlines()

    start_line = None
    end_line = None

    for i, line in enumerate(lines):
        if "let stmt = match parser_mode {" in line:
            start_line = i
        if start_line is not None and "let validation = crate::engine::workflow::ParseValidation {" in line:
            end_line = i - 2
            break

    if start_line is None or end_line is None:
        print("Lines not found.")
        return

    replacement = """
                            macro_rules! interactive_fallback_or_continue {
                                ($cfg:expr, $router:expr, $res_tx:expr, $err:expr, $next_parser:expr) => {{
                                    if $cfg.interactive_fallbacks {
                                        let mut req = crate::engine::interactive_fallback::InteractiveFallbackRequest::new(
                                            "Document Parsing",
                                            $err.to_string(),
                                        );
                                        req = req.add_alternative("mindee", "Try Mindee API", None);
                                        req = req.add_alternative("document_ai", "Try Document AI Again", None);
                                        req = req.add_alternative("llamaparse", "Try LlamaParse", None);
                                        req = req.add_alternative("offline_parser", "Fall back to Offline Parser (Local)", None);
                                        req = req.add_alternative("cancel", "Cancel Workflow", None);
                                        
                                        let (tx, rx) = tokio::sync::oneshot::channel();
                                        {
                                            let mut map = $router.lock().await;
                                            map.insert(req.id, tx);
                                        }
                                        let _ = $res_tx.send(JobResult::InteractiveFallbackRequired(req));
                                        let choice = rx.await.unwrap_or_else(|_| "cancel".to_string());
                                        match choice.as_str() {
                                            "mindee" => Some(DocumentParserMode::Mindee),
                                            "document_ai" => Some(DocumentParserMode::DocumentAi),
                                            "llamaparse" => Some(DocumentParserMode::LlamaParse),
                                            "offline_parser" => Some(DocumentParserMode::OfflineParser),
                                            _ => None,
                                        }
                                    } else {
                                        Some($next_parser)
                                    }
                                }};
                            }

                            let mut current_parser_mode = parser_mode.clone();
                            let stmt = loop {
                                match current_parser_mode {
                                    DocumentParserMode::DocumentAi => {
                                        let _ = res_tx.send(JobResult::Progress { label: "Parsing with Document AI".into(), fraction: 0.2 });
                                        match crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg) {
                                            Ok(client) => {
                                                let doc_ai = std::sync::Arc::new(client);
                                                let page_count = {
                                                    let p = input.clone();
                                                    tokio::task::spawn_blocking(move || -> usize {
                                                        use pdfium_render::prelude::Pdfium;
                                                        let bindings = Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./")).or_else(|_| Pdfium::bind_to_system_library());
                                                        match bindings { Ok(b) => Pdfium::new(b).load_pdf_from_file(&p, None).map(|d| d.pages().len() as usize).unwrap_or(0), Err(_) => 0 }
                                                    }).await.unwrap_or(0)
                                                };
                                                let final_version = version.clone().unwrap_or_else(|| "pretrained-bankstatement-v5.0-2023-12-06".to_string());
                                                match doc_ai.parse_smart_batch(&input, Some(&final_version), page_count).await {
                                                    Ok(s) => {
                                                        let mut retail_sum = s.opening_balance;
                                                        let mut formal_sum = s.opening_balance;
                                                        for tx in &s.transactions {
                                                            retail_sum += tx.debit.unwrap_or(rust_decimal::Decimal::ZERO) - tx.credit.unwrap_or(rust_decimal::Decimal::ZERO);
                                                            formal_sum += tx.credit.unwrap_or(rust_decimal::Decimal::ZERO) - tx.debit.unwrap_or(rust_decimal::Decimal::ZERO);
                                                        }
                                                        let expected = s.closing_balance;
                                                        let retail_diff = (retail_sum - expected).abs();
                                                        let formal_diff = (formal_sum - expected).abs();
                                                        let one_cent = rust_decimal_macros::dec!(0.01);
                                                        if !s.transactions.is_empty() && s.opening_balance != rust_decimal::Decimal::ZERO && retail_diff > one_cent && formal_diff > one_cent {
                                                            if let Some(next) = interactive_fallback_or_continue!(cfg, router, res_tx, "AI Fidelity Math Check Failed", DocumentParserMode::Mindee) {
                                                                current_parser_mode = next;
                                                                continue;
                                                            } else {
                                                                let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::FidelityCheckFailed("Math check failed".into())));
                                                                return;
                                                            }
                                                        }
                                                        break s;
                                                    }
                                                    Err(e) => {
                                                        tracing::warn!("[workflow] Document AI parse failed: {e}");
                                                        if let Some(next) = interactive_fallback_or_continue!(cfg, router, res_tx, format!("Document AI parse failed: {e}"), DocumentParserMode::Mindee) {
                                                            current_parser_mode = next;
                                                            continue;
                                                        } else {
                                                            let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::ParseFailed("Cancelled".into())));
                                                            return;
                                                        }
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                tracing::warn!("[workflow] Document AI not configured: {e}");
                                                if let Some(next) = interactive_fallback_or_continue!(cfg, router, res_tx, format!("Document AI not configured: {e}"), DocumentParserMode::Mindee) {
                                                    current_parser_mode = next;
                                                    continue;
                                                } else {
                                                    let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::ParseFailed("Cancelled".into())));
                                                    return;
                                                }
                                            }
                                        }
                                    }
                                    DocumentParserMode::Mindee => {
                                        let _ = res_tx.send(JobResult::Progress { label: "Parsing with Mindee...".into(), fraction: 0.3 });
                                        match crate::ai::mindee::MindeeClient::from_app_config(&cfg) {
                                            Ok(client) => match client.parse_statement(&input).await {
                                                Ok(s) => break s,
                                                Err(e) => {
                                                    tracing::warn!("[workflow] Mindee parse failed: {e}");
                                                    if let Some(next) = interactive_fallback_or_continue!(cfg, router, res_tx, format!("Mindee parse failed: {e}"), DocumentParserMode::OfflineParser) {
                                                        current_parser_mode = next;
                                                        continue;
                                                    } else {
                                                        let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::ParseFailed("Cancelled".into())));
                                                        return;
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                tracing::warn!("[workflow] Mindee not configured: {e}");
                                                if let Some(next) = interactive_fallback_or_continue!(cfg, router, res_tx, format!("Mindee not configured: {e}"), DocumentParserMode::OfflineParser) {
                                                    current_parser_mode = next;
                                                    continue;
                                                } else {
                                                    let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::ParseFailed("Cancelled".into())));
                                                    return;
                                                }
                                            }
                                        }
                                    }
                                    DocumentParserMode::LlamaParse => {
                                        let _ = res_tx.send(JobResult::Progress { label: "Parsing with LlamaParse...".into(), fraction: 0.2 });
                                        match crate::ai::llamaparse::LlamaParseClient::from_app_config(&cfg) {
                                            Ok(client) => match client.parse_statement(&input).await {
                                                Ok(s) => break s,
                                                Err(e) => {
                                                    tracing::warn!("[workflow] LlamaParse parse failed: {e}");
                                                    if let Some(next) = interactive_fallback_or_continue!(cfg, router, res_tx, format!("LlamaParse parse failed: {e}"), DocumentParserMode::DocumentAi) {
                                                        current_parser_mode = next;
                                                        continue;
                                                    } else {
                                                        let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::ParseFailed("Cancelled".into())));
                                                        return;
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                tracing::warn!("[workflow] LlamaParse not configured: {e}");
                                                if let Some(next) = interactive_fallback_or_continue!(cfg, router, res_tx, format!("LlamaParse not configured: {e}"), DocumentParserMode::DocumentAi) {
                                                    current_parser_mode = next;
                                                    continue;
                                                } else {
                                                    let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::ParseFailed("Cancelled".into())));
                                                    return;
                                                }
                                            }
                                        }
                                    }
                                    DocumentParserMode::OfflineParser => {
                                        let _ = res_tx.send(JobResult::Progress { label: "Parsing with Offline Parser...".into(), fraction: 0.35 });
                                        let eng = engine_for_tokio.clone();
                                        let path = input.clone();
                                        match tokio::task::spawn_blocking(move || {
                                            crate::engine::offline_parser::parse_statement_offline(&path, eng)
                                        }).await {
                                            Ok(Ok(s)) => break s,
                                            Ok(Err(e)) => {
                                                tracing::warn!("[workflow] Offline parser failed: {e}");
                                                if let Some(next) = interactive_fallback_or_continue!(cfg, router, res_tx, format!("Offline parser failed: {e}"), DocumentParserMode::OfflineParser) {
                                                    current_parser_mode = next;
                                                    continue;
                                                } else {
                                                    let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::ParseFailed(e)));
                                                    return;
                                                }
                                            }
                                            Err(e) => {
                                                let e_str = e.to_string();
                                                tracing::warn!("[workflow] Offline parser panicked: {e}");
                                                if let Some(next) = interactive_fallback_or_continue!(cfg, router, res_tx, format!("Offline parser panicked: {e}"), DocumentParserMode::OfflineParser) {
                                                    current_parser_mode = next;
                                                    continue;
                                                } else {
                                                    let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::ParseFailed(e_str)));
                                                    return;
                                                }
                                            }
                                        }
                                    }
                                    _ => {
                                        let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::ParseFailed("Unsupported parser mode".into())));
                                        return;
                                    }
                                }
                            };
\n"""
    
    new_lines = lines[:start_line] + [replacement] + lines[end_line+1:]
    with open("src/app/runtime.rs", "w") as f:
        f.writelines(new_lines)
    
    print("Done rewriting.")

if __name__ == "__main__":
    main()

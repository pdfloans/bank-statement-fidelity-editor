import sys

content = open('src/app/runtime.rs', 'r', encoding='utf-8').read()

target = """                                DocumentParserMode::LlamaParse => {
                                    let _ = res_tx.send(JobResult::Progress { label: "Parsing with LlamaParse...".into(), fraction: 0.2 });

                                    match crate::ai::llamaparse::LlamaParseClient::from_app_config(&cfg) {
                                        Ok(client) => {
                                            let _ = res_tx.send(JobResult::Progress { label: "LlamaParse: uploading document...".into(), fraction: 0.3 });

                                            match client.parse_statement(&input).await {
                                                Ok(s) => {
                                                    tracing::info!("[workflow] LlamaParse yielded {} transactions.", s.transactions.len());
                                                    let _ = res_tx.send(JobResult::Progress { label: format!("LlamaParse: {} transactions extracted", s.transactions.len()), fraction: 0.6 });
                                                    s
                                                }
                                                Err(e) => {
                                                    // Auto-fallback to offline parsing (same as DocAI/Mindee)
                                                    tracing::warn!("[workflow] LlamaParse extraction failed: {e}; falling back to offline parser");
                                                    let _ = res_tx.send(JobResult::Progress { label: format!("LlamaParse failed ({e}), falling back to offline parser..."), fraction: 0.3 });

                                                    let eng = engine_for_tokio.clone();
                                                    let path = input.clone();
                                                    match tokio::task::spawn_blocking(move || {
                                                        crate::engine::offline_parser::parse_statement_offline(&path, eng)
                                                    }).await {
                                                        Ok(Ok(s)) => s,
                                                        Ok(Err(e2)) => {
                                                            let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::ParseFailed(
                                                                format!("LlamaParse failed: {e}. Offline fallback also failed: {e2}")
                                                            )));
                                                            return;
                                                        }
                                                        Err(e2) => {
                                                            let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::ParseFailed(
                                                                format!("LlamaParse failed: {e}. Offline fallback panicked: {e2}")
                                                            )));
                                                            return;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            // LlamaParse not configured - auto-fallback to offline
                                            tracing::warn!("[workflow] LlamaParse not configured: {e}; auto-falling back to offline parser");
                                            let _ = res_tx.send(JobResult::Progress { label: "LlamaParse not configured, using offline parser...".into(), fraction: 0.3 });

                                            let eng = engine_for_tokio.clone();
                                            let path = input.clone();
                                            match tokio::task::spawn_blocking(move || {
                                                crate::engine::offline_parser::parse_statement_offline(&path, eng)
                                            }).await {
                                                Ok(Ok(s)) => s,
                                                Ok(Err(e2)) => {
                                                    let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::ParseFailed(
                                                        format!("LlamaParse not configured ({e}) and offline parser failed: {e2}")
                                                    )));
                                                    return;
                                                }
                                                Err(e2) => {
                                                    let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::ParseFailed(
                                                        format!("Offline parser panicked: {e2}")
                                                    )));
                                                    return;
                                                }
                                            }
                                        }
                                    }
                                }"""

replacement = """                                DocumentParserMode::LlamaParse => {
                                    let mut final_stmt = None;
                                    let mut errors = Vec::new();

                                    // 1. Try LlamaParse
                                    let _ = res_tx.send(JobResult::Progress { label: "Parsing with LlamaParse...".into(), fraction: 0.2 });
                                    match crate::ai::llamaparse::LlamaParseClient::from_app_config(&cfg) {
                                        Ok(client) => match client.parse_statement(&input).await {
                                            Ok(s) => final_stmt = Some(s),
                                            Err(e) => errors.push(format!("LlamaParse failed: {}", e)),
                                        },
                                        Err(e) => errors.push(format!("LlamaParse not configured: {}", e)),
                                    }

                                    // 2. Try DocumentAI
                                    if final_stmt.is_none() {
                                        let _ = res_tx.send(JobResult::Progress { label: "Falling back to Document AI...".into(), fraction: 0.3 });
                                        let page_count = {
                                            let p = input.clone();
                                            tokio::task::spawn_blocking(move || -> usize {
                                                use pdfium_render::prelude::Pdfium;
                                                let bindings = Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./")).or_else(|_| Pdfium::bind_to_system_library());
                                                match bindings { Ok(b) => Pdfium::new(b).load_pdf_from_file(&p, None).map(|d| d.pages().len() as usize).unwrap_or(0), Err(_) => 0 }
                                            }).await.unwrap_or(0)
                                        };
                                        match crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg) {
                                            Ok(client) => {
                                                let doc_ai = std::sync::Arc::new(client);
                                                let final_version = version.clone().unwrap_or_else(|| "pretrained-bankstatement-v5.0-2023-12-06".to_string());
                                                match doc_ai.parse_smart_batch(&input, Some(&final_version), page_count).await {
                                                    Ok(s) => final_stmt = Some(s),
                                                    Err(e) => errors.push(format!("DocumentAI failed: {}", e)),
                                                }
                                            }
                                            Err(e) => errors.push(format!("DocumentAI not configured: {}", e)),
                                        }
                                    }

                                    // 3. Try Mindee
                                    if final_stmt.is_none() {
                                        let _ = res_tx.send(JobResult::Progress { label: "Falling back to Mindee...".into(), fraction: 0.4 });
                                        match crate::ai::mindee::MindeeClient::from_app_config(&cfg) {
                                            Ok(client) => match client.parse_statement(&input).await {
                                                Ok(s) => final_stmt = Some(s),
                                                Err(e) => errors.push(format!("Mindee failed: {}", e)),
                                            }
                                            Err(e) => errors.push(format!("Mindee not configured: {}", e)),
                                        }
                                    }

                                    // 4. Try Offline Parser
                                    let s = if let Some(stmt) = final_stmt {
                                        stmt
                                    } else {
                                        let _ = res_tx.send(JobResult::Progress { label: "Falling back to offline parser...".into(), fraction: 0.5 });
                                        let eng = engine_for_tokio.clone();
                                        let path = input.clone();
                                        match tokio::task::spawn_blocking(move || {
                                            crate::engine::offline_parser::parse_statement_offline(&path, eng)
                                        }).await {
                                            Ok(Ok(s)) => s,
                                            Ok(Err(e)) => {
                                                errors.push(format!("Offline parser failed: {}", e));
                                                let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::ParseFailed(errors.join(" | "))));
                                                return;
                                            }
                                            Err(e) => {
                                                errors.push(format!("Offline parser panicked: {}", e));
                                                let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::ParseFailed(errors.join(" | "))));
                                                return;
                                            }
                                        }
                                    };

                                    // Phase 5: Fidelity Check
                                    let mut retail_sum = s.opening_balance;
                                    let mut formal_sum = s.opening_balance;
                                    for tx in &s.transactions {
                                        let d = tx.debit.unwrap_or(rust_decimal::Decimal::ZERO);
                                        let c = tx.credit.unwrap_or(rust_decimal::Decimal::ZERO);
                                        retail_sum += d - c;
                                        formal_sum += c - d;
                                    }
                                    let expected = s.closing_balance;
                                    let retail_diff = (retail_sum - expected).abs();
                                    let formal_diff = (formal_sum - expected).abs();
                                    let one_cent = rust_decimal_macros::dec!(0.01);

                                    if !s.transactions.is_empty() && s.opening_balance != rust_decimal::Decimal::ZERO && retail_diff > one_cent && formal_diff > one_cent {
                                        let msg = format!("AI Fidelity Math Check Failed. Expected Closing: {expected}, computed: {retail_sum} (retail) or {formal_sum} (formal).");
                                        let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::FidelityCheckFailed(msg)));
                                        return;
                                    }

                                    tracing::info!("[workflow] LlamaParse sequence yielded {} transactions.", s.transactions.len());
                                    let _ = res_tx.send(JobResult::Progress { label: format!("Extracted {} transactions", s.transactions.len()), fraction: 0.6 });
                                    
                                    s
                                }"""

if target in content:
    content = content.replace(target, replacement)
    open('src/app/runtime.rs', 'w', encoding='utf-8').write(content)
    print('SUCCESS')
else:
    print('TARGET NOT FOUND')

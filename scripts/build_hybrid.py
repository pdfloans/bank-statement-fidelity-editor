"""Apply hybrid native-first-with-Python-fallback logic to runtime.rs."""
import sys, re

with open('src/app/runtime.rs', 'r', encoding='utf-8') as f:
    content = f.read()
with open('extracted_dispatch.rs', 'r', encoding='utf-8') as f:
    real_dispatch = f.read()
with open('extracted_thread.rs', 'r', encoding='utf-8') as f:
    real_thread = f.read()

# 1. Replace the stub thread with the real thread
stub_thread_pattern = r'        // Python actor removed\n        let _python_stub_thread = thread::spawn\(move \|\| \{.*?\n        \}\);\n'
content = re.sub(stub_thread_pattern, real_thread, content, flags=re.DOTALL)

# 2. Replace the dispatch_python_job stub with the real dispatch function
stub_dispatch_pattern = r'/// Dispatches a Python job\nfn dispatch_python_job.*?\}\n'
content = re.sub(stub_dispatch_pattern, real_dispatch, content, flags=re.DOTALL)

# 3. Hybridize ClonePages in TransferTransactions
clone_pages_target = '''                                    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                                    let _ = py_tx.send((
                                        PythonJob::ClonePages {
                                            pdf_path: output_pdf.to_string_lossy().to_string(),
                                            output_path: temp_path.to_string_lossy().to_string(),
                                            page_indices: transfer_plan.pages_to_clone.clone(),
                                        },
                                        reply_tx,
                                    ));
                                    match reply_rx.await {
                                        Ok(PythonJobResult::Json(json_str)) => {
                                            if let Ok(res) = serde_json::from_str::<serde_json::Value>(&json_str) {
                                                if res["success"].as_bool().unwrap_or(false) {
                                                    actual_pages_added = res["cloned"].as_u64().unwrap_or(0) as usize;
                                                    let _ = std::fs::rename(&temp_path, &output_pdf);
                                                }
                                            }
                                            tracing::info!("[TRANSFER] Cloned {} pages", actual_pages_added);
                                        }
                                        other => tracing::warn!("[TRANSFER] Page cloning failed: {:?}", other),
                                    }'''
clone_pages_hybrid = '''                                    let eng = engine_for_tokio.clone();
                                    let p_in = output_pdf.clone();
                                    let p_out = temp_path.clone();
                                    let idxs = transfer_plan.pages_to_clone.clone();
                                    let native_res = tokio::task::spawn_blocking(move || {
                                        eng.clone_pages(&p_in, &p_out, idxs)
                                    }).await.unwrap_or(Ok(0));

                                    if let Ok(c) = native_res {
                                        if c > 0 {
                                            actual_pages_added = c;
                                            let _ = std::fs::rename(&temp_path, &output_pdf);
                                            tracing::info!("[TRANSFER] (Native) Cloned {} pages", actual_pages_added);
                                        }
                                    }
                                    
                                    if actual_pages_added == 0 {
                                        tracing::warn!("[TRANSFER] Native ClonePages failed or returned 0. Falling back to Python.");
                                        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                                        let _ = py_tx.send((
                                            PythonJob::ClonePages {
                                                pdf_path: output_pdf.to_string_lossy().to_string(),
                                                output_path: temp_path.to_string_lossy().to_string(),
                                                page_indices: transfer_plan.pages_to_clone.clone(),
                                            },
                                            reply_tx,
                                        ));
                                        match reply_rx.await {
                                            Ok(PythonJobResult::Json(json_str)) => {
                                                if let Ok(res) = serde_json::from_str::<serde_json::Value>(&json_str) {
                                                    if res["success"].as_bool().unwrap_or(false) {
                                                        actual_pages_added = res["cloned"].as_u64().unwrap_or(0) as usize;
                                                        let _ = std::fs::rename(&temp_path, &output_pdf);
                                                    }
                                                }
                                                tracing::info!("[TRANSFER] (Python) Cloned {} pages", actual_pages_added);
                                            }
                                            other => tracing::warn!("[TRANSFER] (Python) Page cloning failed: {:?}", other),
                                        }
                                    }'''
content = content.replace(clone_pages_target, clone_pages_hybrid)

# 4. Hybridize RemovePages
remove_pages_target = '''                                    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                                    let _ = py_tx.send((
                                        PythonJob::RemovePages {
                                            pdf_path: output_pdf.to_string_lossy().to_string(),
                                            output_path: temp_path.to_string_lossy().to_string(),
                                            page_indices: transfer_plan.pages_to_remove.clone(),
                                        },
                                        reply_tx,
                                    ));
                                    match reply_rx.await {
                                        Ok(PythonJobResult::Json(json_str)) => {
                                            if let Ok(res) = serde_json::from_str::<serde_json::Value>(&json_str) {
                                                if res["success"].as_bool().unwrap_or(false) {
                                                    actual_pages_removed = res["removed"].as_u64().unwrap_or(0) as usize;
                                                    let _ = std::fs::rename(&temp_path, &output_pdf);
                                                }
                                            }
                                            tracing::info!("[TRANSFER] Removed {} pages", actual_pages_removed);
                                        }
                                        other => tracing::warn!("[TRANSFER] Page removal failed: {:?}", other),
                                    }'''
remove_pages_hybrid = '''                                    let eng = engine_for_tokio.clone();
                                    let p_in = output_pdf.clone();
                                    let p_out = temp_path.clone();
                                    let idxs = transfer_plan.pages_to_remove.clone();
                                    let native_res = tokio::task::spawn_blocking(move || {
                                        eng.remove_pages(&p_in, &p_out, idxs)
                                    }).await.unwrap_or(Ok(0));

                                    if let Ok(c) = native_res {
                                        if c > 0 {
                                            actual_pages_removed = c;
                                            let _ = std::fs::rename(&temp_path, &output_pdf);
                                            tracing::info!("[TRANSFER] (Native) Removed {} pages", actual_pages_removed);
                                        }
                                    }

                                    if actual_pages_removed == 0 {
                                        tracing::warn!("[TRANSFER] Native RemovePages failed or returned 0. Falling back to Python.");
                                        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                                        let _ = py_tx.send((
                                            PythonJob::RemovePages {
                                                pdf_path: output_pdf.to_string_lossy().to_string(),
                                                output_path: temp_path.to_string_lossy().to_string(),
                                                page_indices: transfer_plan.pages_to_remove.clone(),
                                            },
                                            reply_tx,
                                        ));
                                        match reply_rx.await {
                                            Ok(PythonJobResult::Json(json_str)) => {
                                                if let Ok(res) = serde_json::from_str::<serde_json::Value>(&json_str) {
                                                    if res["success"].as_bool().unwrap_or(false) {
                                                        actual_pages_removed = res["removed"].as_u64().unwrap_or(0) as usize;
                                                        let _ = std::fs::rename(&temp_path, &output_pdf);
                                                    }
                                                }
                                                tracing::info!("[TRANSFER] (Python) Removed {} pages", actual_pages_removed);
                                            }
                                            other => tracing::warn!("[TRANSFER] (Python) Page removal failed: {:?}", other),
                                        }
                                    }'''
content = content.replace(remove_pages_target, remove_pages_hybrid)

# 5. Hybridize ApplyManyEdits (Chunked)
apply_many_target_1 = '''                                                    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                                                    let _ = py_tx.send((
                                                        PythonJob::ApplyManyEdits {
                                                            pdf_path: seg.path.to_string_lossy().to_string(),
                                                            output_path: edited_path.to_string_lossy().to_string(),
                                                            edits_json: edits_json.clone(),
                                                            font_path: font_override_path.clone(),
                                                        },
                                                        reply_tx,
                                                    ));
                                                    match reply_rx.await {
                                                        Ok(PythonJobResult::Json(json_str)) => {
                                                            if let Ok(res) = serde_json::from_str::<serde_json::Value>(&json_str) {
                                                                if res["success"].as_bool().unwrap_or(false) {
                                                                    edits_applied += res["applied"].as_u64().unwrap_or(0) as usize;
                                                                    if let Some(flags) = res["review_flags"].as_array() {
                                                                        for f in flags {
                                                                            if let Some(pg) = f.as_u64() {
                                                                                if let Some(gp) = map.to_global(i, pg as usize) {
                                                                                    fallback_fonts_used.push(gp);
                                                                                }
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                            final_paths.push(edited_path);
                                                        }
                                                        _ => {
                                                            tracing::warn!("[TRANSFER] Batch edit failed on segment {}, pushing unedited", i);
                                                            final_paths.push(seg.path.clone());
                                                        }
                                                    }'''
apply_many_hybrid_1 = '''                                                    let eng = engine_for_tokio.clone();
                                                    let p_in = seg.path.clone();
                                                    let p_out = edited_path.clone();
                                                    let e_json = edits_json.clone();
                                                    let f_path = font_override_path.clone();
                                                    
                                                    let native_res = tokio::task::spawn_blocking(move || {
                                                        let fp = f_path.map(std::path::PathBuf::from);
                                                        eng.apply_many_edits(&p_in, &p_out, &e_json, fp.as_deref())
                                                    }).await.unwrap_or(Ok(0));

                                                    let mut segment_applied = 0;
                                                    if let Ok(c) = native_res {
                                                        segment_applied = c;
                                                    }
                                                    
                                                    if segment_applied > 0 {
                                                        tracing::info!("[TRANSFER] (Native) Batch edit segment {} succeeded", i);
                                                        edits_applied += segment_applied;
                                                        final_paths.push(edited_path);
                                                    } else {
                                                        tracing::warn!("[TRANSFER] Native ApplyManyEdits failed or returned 0. Falling back to Python.");
                                                        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                                                        let _ = py_tx.send((
                                                            PythonJob::ApplyManyEdits {
                                                                pdf_path: seg.path.to_string_lossy().to_string(),
                                                                output_path: edited_path.to_string_lossy().to_string(),
                                                                edits_json: edits_json.clone(),
                                                                font_path: font_override_path.clone(),
                                                            },
                                                            reply_tx,
                                                        ));
                                                        match reply_rx.await {
                                                            Ok(PythonJobResult::Json(json_str)) => {
                                                                if let Ok(res) = serde_json::from_str::<serde_json::Value>(&json_str) {
                                                                    if res["success"].as_bool().unwrap_or(false) {
                                                                        edits_applied += res["applied"].as_u64().unwrap_or(0) as usize;
                                                                        if let Some(flags) = res["review_flags"].as_array() {
                                                                            for f in flags {
                                                                                if let Some(pg) = f.as_u64() {
                                                                                    if let Some(gp) = map.to_global(i, pg as usize) {
                                                                                        fallback_fonts_used.push(gp);
                                                                                    }
                                                                                }
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                                final_paths.push(edited_path);
                                                            }
                                                            _ => {
                                                                tracing::warn!("[TRANSFER] (Python) Batch edit failed on segment {}, pushing unedited", i);
                                                                final_paths.push(seg.path.clone());
                                                            }
                                                        }
                                                    }'''
content = content.replace(apply_many_target_1, apply_many_hybrid_1)

# 6. Hybridize ApplyManyEdits (Direct)
apply_many_target_2 = '''                                        let edits_json = serde_json::to_string(&batch_edits).unwrap_or_default();
                                        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                                        let _ = py_tx.send((
                                            PythonJob::ApplyManyEdits {
                                                pdf_path: output_pdf.to_string_lossy().to_string(),
                                                output_path: output_pdf.with_extension("temp.pdf").to_string_lossy().to_string(),
                                                edits_json,
                                                font_path: font_override_path.clone(),
                                            },
                                            reply_tx,
                                        ));

                                        match reply_rx.await {
                                            Ok(PythonJobResult::Json(json_str)) => {
                                                if let Ok(res) = serde_json::from_str::<serde_json::Value>(&json_str) {
                                                    if res["success"].as_bool().unwrap_or(false) {
                                                        edits_applied = res["applied"].as_u64().unwrap_or(0) as usize;
                                                        if let Some(flags) = res["review_flags"].as_array() {
                                                            for f in flags {
                                                                if let Some(pg) = f.as_u64() {
                                                                    fallback_fonts_used.push(pg as usize);
                                                                }
                                                            }
                                                        }
                                                        let _ = std::fs::rename(output_pdf.with_extension("temp.pdf"), &output_pdf);
                                                    }
                                                }
                                            }
                                            Ok(PythonJobResult::Error(e)) => tracing::error!("[TRANSFER] Batch edit failed: {}", e),
                                            _ => tracing::error!("[TRANSFER] Batch edit failed with unexpected result"),
                                        }'''
apply_many_hybrid_2 = '''                                        let edits_json = serde_json::to_string(&batch_edits).unwrap_or_default();
                                        let eng = engine_for_tokio.clone();
                                        let p_in = output_pdf.clone();
                                        let p_out = output_pdf.with_extension("temp.pdf");
                                        let f_path = font_override_path.clone();
                                        
                                        let native_res = tokio::task::spawn_blocking(move || {
                                            let fp = f_path.map(std::path::PathBuf::from);
                                            eng.apply_many_edits(&p_in, &p_out, &edits_json, fp.as_deref())
                                        }).await.unwrap_or(Ok(0));

                                        if let Ok(c) = native_res {
                                            if c > 0 {
                                                edits_applied = c;
                                                let _ = std::fs::rename(output_pdf.with_extension("temp.pdf"), &output_pdf);
                                                tracing::info!("[TRANSFER] (Native) Batch edit succeeded");
                                            }
                                        }

                                        if edits_applied == 0 {
                                            tracing::warn!("[TRANSFER] Native ApplyManyEdits failed or returned 0. Falling back to Python.");
                                            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                                            let _ = py_tx.send((
                                                PythonJob::ApplyManyEdits {
                                                    pdf_path: output_pdf.to_string_lossy().to_string(),
                                                    output_path: output_pdf.with_extension("temp.pdf").to_string_lossy().to_string(),
                                                    edits_json,
                                                    font_path: font_override_path.clone(),
                                                },
                                                reply_tx,
                                            ));

                                            match reply_rx.await {
                                                Ok(PythonJobResult::Json(json_str)) => {
                                                    if let Ok(res) = serde_json::from_str::<serde_json::Value>(&json_str) {
                                                        if res["success"].as_bool().unwrap_or(false) {
                                                            edits_applied = res["applied"].as_u64().unwrap_or(0) as usize;
                                                            if let Some(flags) = res["review_flags"].as_array() {
                                                                for f in flags {
                                                                    if let Some(pg) = f.as_u64() {
                                                                        fallback_fonts_used.push(pg as usize);
                                                                    }
                                                                }
                                                            }
                                                            let _ = std::fs::rename(output_pdf.with_extension("temp.pdf"), &output_pdf);
                                                            tracing::info!("[TRANSFER] (Python) Batch edit succeeded");
                                                        }
                                                    }
                                                }
                                                Ok(PythonJobResult::Error(e)) => tracing::error!("[TRANSFER] (Python) Batch edit failed: {}", e),
                                                _ => tracing::error!("[TRANSFER] (Python) Batch edit failed with unexpected result"),
                                            }
                                        }'''
content = content.replace(apply_many_target_2, apply_many_hybrid_2)

with open('src/app/runtime.rs', 'w', encoding='utf-8') as f:
    f.write(content)
print('Applied Hybrid Logic to runtime.rs successfully')

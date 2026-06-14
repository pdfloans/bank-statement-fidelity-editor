        let (python_tx, python_rx) =
            mpsc::channel::<(PythonJob, oneshot::Sender<PythonJobResult>)>();

        let primary_engine = Arc::new(crate::pdf::MuPdfEngine::new());
        let fallback_engine = Arc::new(crate::pdf::PyMuPdfEngine::new(job_tx.clone()));
        let engine: Arc<dyn crate::pdf::PdfEngine> = Arc::new(crate::pdf::PdfEngineSelector::new(
            primary_engine,
            fallback_engine,
        ));

        let audit_log = Arc::new(Mutex::new(audit_log));
        let history = Arc::new(Mutex::new(ChangeHistory::new()));

        let _python_actor_thread = thread::spawn(move || {
            let engine_result = PyEngine::init();

            if let Err(e) = &engine_result {
                tracing::error!("❌ [PYTHON_ACTOR] Failed to initialize PyEngine: {}", e);
            }

            while let Ok((job, reply_tx)) = python_rx.recv() {
                if let PythonJob::Ping = job {
                    let _ = reply_tx.send(PythonJobResult::Pong);
                    continue;
                }

                match &engine_result {
                    Ok(engine) => {
                        let res =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match job {
                                PythonJob::Ping => unreachable!(),
                                PythonJob::GetTextBlocks { pdf_path, page_num } => engine
                                    .get_text_blocks(&pdf_path, page_num)
                                    .map(PythonJobResult::Json),
                                PythonJob::ReplaceTextInRect {
                                    pdf_path,
                                    output_path,
                                    page_num,
                                    rect,
                                    new_text,
                                    font_path,
                                } => engine
                                    .replace_text_in_rect(
                                        &pdf_path,
                                        &output_path,
                                        page_num,
                                        rect,
                                        &new_text,
                                        font_path.as_deref(),
                                    )
                                    .map(|opt| {
                                        opt.map(|reason| {
                                            PythonJobResult::ReplacedWithReviewWarning { reason }
                                        })
                                        .unwrap_or(PythonJobResult::Success)
                                    }),
                                PythonJob::FindTextBlockAtClick {
                                    pdf_path,
                                    page_num,
                                    x,
                                    y,
                                } => engine
                                    .find_text_block_at_click(&pdf_path, page_num, x, y)
                                    .map(PythonJobResult::Json),
                                PythonJob::GetAllTransactions { pdf_path } => engine
                                    .get_all_transactions(&pdf_path)
                                    .map(PythonJobResult::Json),
                                PythonJob::AnalyzeDocumentLayout { pdf_path } => engine
                                    .analyze_document_layout(&pdf_path)
                                    .map(PythonJobResult::Json),
                                PythonJob::CompleteFontWithAdaption {
                                    pdf_path,
                                    font_name,
                                } => engine
                                    .complete_font_with_adaption(&pdf_path, &font_name)
                                    .map(PythonJobResult::Json),
                                PythonJob::DeepFontReplication {
                                    pdf_path,
                                    font_name,
                                    output_dir,
                                } => engine
                                    .deep_font_replication(&pdf_path, &font_name, &output_dir)
                                    .map(PythonJobResult::Json),
                                PythonJob::ApplyManyEdits {
                                    pdf_path,
                                    output_path,
                                    edits_json,
                                    font_path,
                                } => engine
                                    .apply_many_edits(
                                        &pdf_path,
                                        &output_path,
                                        &edits_json,
                                        font_path.as_deref(),
                                    )
                                    .map(PythonJobResult::Json),
                                PythonJob::ChunkPdfForDocai {
                                    pdf_path,
                                    output_dir,
                                    max_pages_per_chunk,
                                } => engine
                                    .chunk_pdf_for_docai(&pdf_path, &output_dir, max_pages_per_chunk)
                                    .map(PythonJobResult::Json),
                                PythonJob::AnalyzeFonts { pdf_path } => engine
                                    .analyze_fonts(&pdf_path)
                                    .map(PythonJobResult::Json),
                                PythonJob::ReplicateFontForMissingChars {
                                    pdf_path,
                                    font_name,
                                    missing_chars_csv,
                                    output_dir,
                                } => engine
                                    .replicate_font_for_missing_chars(
                                        &pdf_path,
                                        &font_name,
                                        &missing_chars_csv,
                                        &output_dir,
                                    )
                                    .map(PythonJobResult::Json),
                                PythonJob::ClonePages {
                                    pdf_path,
                                    output_path,
                                    page_indices,
                                } => engine
                                    .clone_pages(&pdf_path, &output_path, &page_indices)
                                    .map(PythonJobResult::Json),
                                PythonJob::RemovePages {
                                    pdf_path,
                                    output_path,
                                    page_indices,
                                } => engine
                                    .remove_pages(&pdf_path, &output_path, &page_indices)
                                    .map(PythonJobResult::Json),
                            }));

                        let final_res = match res {
                            Ok(Ok(pjr)) => pjr,
                            Ok(Err(e)) => PythonJobResult::Error(e),
                            Err(panic) => {
                                let msg = if let Some(s) = panic.downcast_ref::<&str>() {
                                    s.to_string()
                                } else if let Some(s) = panic.downcast_ref::<String>() {
                                    s.clone()
                                } else {
                                    "Unknown panic in Python actor".to_string()
                                };
                                PythonJobResult::Error(format!("PyO3 panic: {msg}"))
                            }
                        };
                        let _ = reply_tx.send(final_res);
                        // Stage 2 Memory Management: explicit collection
                        crate::ai::pyo3_bridge::PyEngine::garbage_collect();
                    }
                    Err(e) => {
                        let _ = reply_tx.send(PythonJobResult::Error(format!(
                            "Python Engine not initialized: {e}"
                        )));
                    }
                }
            }
        });

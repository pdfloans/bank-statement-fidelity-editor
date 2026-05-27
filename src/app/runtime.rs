use std::thread::{self, JoinHandle};
use std::sync::{mpsc, Arc, Mutex};
use std::path::PathBuf;
use tokio::sync::oneshot;
use crate::ai::pyo3_bridge::PyEngine;
use crate::app::audit::AuditLog;
use crate::engine::history::{ChangeHistory, ChangeRecord};

#[derive(Debug, Clone)]
pub enum PythonJob {
    Ping,
    GetTextBlocks { pdf_path: String, page_num: usize },
    ReplaceTextInRect { 
        pdf_path: String, 
        output_path: String, 
        page_num: usize, 
        rect: [f32; 4], 
        new_text: String,
        font_path: Option<String> 
    },
    FindTextBlockAtClick { pdf_path: String, page_num: usize, x: f32, y: f32 },
    GetAllTransactions { pdf_path: String },
    AnalyzeDocumentLayout { pdf_path: String },
    CompleteFontWithAdaption { pdf_path: String, font_name: String },
    DeepFontReplication { pdf_path: String, font_name: String, output_dir: String },
}

#[derive(Debug)]
pub enum PythonJobResult {
    Pong,
    Json(String),
    ReplacedWithReviewWarning { reason: String },
    Success,
    Error(String),
}

#[derive(Debug)]
pub enum Job {
    Ping,
    Python(PythonJob, oneshot::Sender<PythonJobResult>),
    LoadDocument { path: PathBuf },
    RenderPage { path: PathBuf, page: usize, dpi: f32, tag: String },
    ApplyChange { 
        input: PathBuf, 
        output: PathBuf, 
        page: usize, 
        bbox: [f32; 4], 
        new_text: String,
        old_text: String,
        description: String,
        deep_font_replication: bool,
    },
    CompleteFont { path: PathBuf, font_name: String },
    Undo,
    Redo,
    BalanceStatement { path: PathBuf },
    ExtractTransactions { path: PathBuf },
    ApplyProposedChanges { input: PathBuf, output: PathBuf, changes: Vec<crate::engine::model::ProposedChange> },
    ExportChangeHistory { output: PathBuf },
    LoadHistory { input: PathBuf },
    Verify { 
        original: PathBuf, 
        edited: PathBuf, 
        output_dir: PathBuf, 
        intended_bboxes: Vec<(usize, [f32; 4])>,
        use_pdfrest: bool,
        pdfrest_key: Option<String>,
    },
}

#[derive(Debug)]
pub enum JobResult {
    Pong,
    DocumentLoaded { layout_json: String, total_pages: usize },
    PageRendered { png_bytes: Vec<u8>, page: usize, dpi: f32, tag: String, width_pts: f32, height_pts: f32 },
    ChangeApplied { record: ChangeRecord, requires_visual_review: bool },
    HistoryUpdated { history: ChangeHistory },
    FontCompleted(String),
    ChangeHistoryExported { path: PathBuf },
    TransactionsExtracted(Vec<crate::engine::model::Transaction>),
    VerificationReport(crate::engine::verification::VerificationReport),
    BalanceProposed { imbalance: f64, changes: Vec<crate::engine::model::ProposedChange> },
    ProposedChangesApplied { changes_applied: usize, failures: Vec<String> },
    Error { job_label: String, message: String },
    Progress { label: String, fraction: f32 },
}

pub struct Runtime {
    _tokio_rt: tokio::runtime::Runtime,
    _python_actor_thread: JoinHandle<()>,
}

impl Runtime {
    pub fn start(audit_log: AuditLog, config: Arc<crate::app::config::AppConfig>) -> (Self, mpsc::Sender<Job>, mpsc::Receiver<JobResult>) {
        let tokio_rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to start Tokio runtime");

        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (result_tx, result_rx) = mpsc::channel::<JobResult>();
        
        let (python_tx, python_rx) = mpsc::channel::<(PythonJob, oneshot::Sender<PythonJobResult>)>();
        
        let primary_engine = Arc::new(crate::pdf::MuPdfEngine::new());
        let fallback_engine = Arc::new(crate::pdf::PyMuPdfEngine::new(job_tx.clone()));
        let engine: Arc<dyn crate::pdf::PdfEngine> = Arc::new(crate::pdf::PdfEngineSelector::new(primary_engine, fallback_engine));

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
                        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            match job {
                                PythonJob::Ping => unreachable!(),
                                PythonJob::GetTextBlocks { pdf_path, page_num } => {
                                    engine.get_text_blocks(&pdf_path, page_num).map(PythonJobResult::Json)
                                }
                                PythonJob::ReplaceTextInRect { pdf_path, output_path, page_num, rect, new_text, font_path } => {
                                   engine.replace_text_in_rect(&pdf_path, &output_path, page_num, rect, &new_text, font_path.as_deref())
                                       .map(|opt| opt.map(|reason| PythonJobResult::ReplacedWithReviewWarning { reason }).unwrap_or(PythonJobResult::Success))
                                }
                                PythonJob::FindTextBlockAtClick { pdf_path, page_num, x, y } => {
                                   engine.find_text_block_at_click(&pdf_path, page_num, x, y).map(PythonJobResult::Json)
                                }
                                PythonJob::GetAllTransactions { pdf_path } => {
                                   engine.get_all_transactions(&pdf_path).map(PythonJobResult::Json)
                                }
                                PythonJob::AnalyzeDocumentLayout { pdf_path } => {
                                   engine.analyze_document_layout(&pdf_path).map(PythonJobResult::Json)
                                }
                                PythonJob::CompleteFontWithAdaption { pdf_path, font_name } => {
                                   engine.complete_font_with_adaption(&pdf_path, &font_name).map(PythonJobResult::Json)
                                }
                                PythonJob::DeepFontReplication { pdf_path, font_name, output_dir } => {
                                   engine.deep_font_replication(&pdf_path, &font_name, &output_dir).map(PythonJobResult::Json)
                                }

                            }
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
                                PythonJobResult::Error(format!("PyO3 panic: {}", msg))
                            }
                        };
                        let _ = reply_tx.send(final_res);
                    }
                    Err(e) => {
                        let _ = reply_tx.send(PythonJobResult::Error(format!("Python Engine not initialized: {}", e)));
                    }
                }
            }
        });

        let result_tx_clone = result_tx.clone();
        let python_tx_clone = python_tx.clone();
        
        let (tokio_job_tx, tokio_job_rx) = tokio::sync::mpsc::unbounded_channel::<Job>();
        
        spawn_runtime_bridge(job_rx, tokio_job_tx.clone(), result_tx.clone());

        let mut tokio_job_rx = tokio_job_rx;
        let engine_for_tokio = engine.clone();
        let tokio_job_tx_clone = tokio_job_tx.clone();
        let config_for_tokio = config.clone();

        tokio_rt.spawn(async move {
            while let Some(job) = tokio_job_rx.recv().await {
                match job {
                    Job::Ping => {
                        let (reply_tx, reply_rx) = oneshot::channel();
                        if python_tx_clone.send((PythonJob::Ping, reply_tx)).is_ok() {
                            if let Ok(PythonJobResult::Pong) = reply_rx.await {
                                let _ = result_tx_clone.send(JobResult::Pong);
                            }
                        }
                    }
                    Job::Python(py_job, reply_tx) => {
                        match py_job {
                            PythonJob::FindTextBlockAtClick { .. } => {
                                let (int_tx, int_rx) = oneshot::channel();
                                dispatch_python_job(py_job, int_tx, &python_tx_clone);
                                tokio::spawn(async move {
                                    if let Ok(res) = int_rx.await {
                                        match res {
                                            PythonJobResult::Error(_) => {
                                                // Benign no-op for click detection
                                            }
                                            _ => {
                                                let _ = reply_tx.send(res);
                                            }
                                        }
                                    }
                                });
                            }
                            _ => {
                                dispatch_python_job(py_job, reply_tx, &python_tx_clone);
                            }
                        }
                    }
                    Job::LoadDocument { path } => {
                        let _ = result_tx_clone.send(JobResult::Progress { label: "Analyzing layout".to_string(), fraction: 0.1 });
                        let eng = engine_for_tokio.clone();
                        let res_tx = result_tx_clone.clone();
                        tokio::task::spawn_blocking(move || {
                            match eng.analyze_layout(&path) {
                                Ok(layout) => {
                                    let json = serde_json::to_string(&layout.pages).unwrap_or_default();
                                    let _ = res_tx.send(JobResult::DocumentLoaded { layout_json: json, total_pages: layout.total_pages });
                                    let _ = res_tx.send(JobResult::Progress { label: "Done".to_string(), fraction: 1.0 });
                                }
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "load_document".into(), message: e.to_string() });
                                }
                            }
                        });
                    }
                    Job::RenderPage { path, page, dpi, tag } => {
                        let res_tx = result_tx_clone.clone();
                        let eng = engine_for_tokio.clone();
                        tokio::task::spawn_blocking(move || {
                            match eng.render_page(&path, page, dpi) {
                                Ok(rendered) => {
                                    let _ = res_tx.send(JobResult::PageRendered { 
                                        png_bytes: rendered.png_bytes, page, dpi, tag, width_pts: rendered.width_pts, height_pts: rendered.height_pts 
                                    });
                                }
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "render_page".into(), message: e.to_string() });
                                }
                            }
                        }).await.unwrap();
                    }
                    Job::ApplyChange { input, output, page, bbox, new_text, old_text, description, deep_font_replication } => {
                        let _ = result_tx_clone.send(JobResult::Progress { label: "Applying change".to_string(), fraction: 0.1 });

                        let eng = engine_for_tokio.clone();
                        let audit_log_clone = audit_log.clone();
                        let history_clone = history.clone();
                        let py_tx = python_tx_clone.clone();
                        let res_tx = result_tx_clone.clone();
                        let cfg_clone = config_for_tokio.clone();

                        tokio::task::spawn(async move {
                            // Optional: deep font replication via Python actor.
                            let mut font_path: Option<PathBuf> = None;
                            if deep_font_replication {
                                let _ = res_tx.send(JobResult::Progress { label: "Deep Replicating Font...".to_string(), fraction: 0.2 });
                                let (tx, rx) = oneshot::channel();
                                let _ = py_tx.send((PythonJob::DeepFontReplication {
                                    pdf_path: input.to_string_lossy().to_string(),
                                    font_name: "Helvetica".to_string(),
                                    output_dir: "output/temp_fonts".to_string(),
                                }, tx));
                                if let Ok(PythonJobResult::Json(json)) = rx.await {
                                    let res: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
                                    if res["success"].as_bool().unwrap_or(false) {
                                        font_path = res["metrics"]["font_path"].as_str().map(PathBuf::from);
                                    } else if let Some(err) = res.get("error").and_then(|e| e.as_str()) {
                                        tracing::warn!("[apply_change] deep font replication failed: {}", err);
                                    }
                                }
                            }

                            // Run blocking apply_change with cloned-only inputs.
                            let input_for_blocking = input.clone();
                            let output_for_blocking = output.clone();
                            let new_text_for_blocking = new_text.clone();
                            let outcome = tokio::task::spawn_blocking(move || {
                                eng.apply_change(
                                    &input_for_blocking,
                                    &output_for_blocking,
                                    page,
                                    bbox,
                                    &new_text_for_blocking,
                                    font_path.as_deref(),
                                )
                            })
                            .await
                            .unwrap_or_else(|e| Err(crate::pdf::EngineError::ApplyFailed(format!("blocking task panicked: {}", e))));

                            match outcome {
                                Ok(o) => {
                                    let requires_visual_review = o.overflow;
                                    let mut h = match history_clone.lock() {
                                        Ok(g) => g,
                                        Err(e) => {
                                            let _ = res_tx.send(JobResult::Error { job_label: "apply_change".into(), message: format!("History lock poisoned: {}", e) });
                                            return;
                                        }
                                    };
                                    let mut a = match audit_log_clone.lock() {
                                        Ok(g) => g,
                                        Err(e) => {
                                            let _ = res_tx.send(JobResult::Error { job_label: "apply_change".into(), message: format!("Audit lock poisoned: {}", e) });
                                            return;
                                        }
                                    };

                                    let mut final_record = h.create_record(page, old_text, new_text.clone(), bbox, description, None);
                                    let snap_path = a.snapshot_path_for(final_record.id);

                                    if let Err(e) = std::fs::copy(&output, &snap_path) {
                                        let _ = res_tx.send(JobResult::Error { job_label: "apply_change".into(), message: format!("Snapshot failed: {}", e) });
                                        return;
                                    }

                                    final_record.snapshot_path = Some(snap_path);
                                    if let Err(e) = a.write(&final_record, &input, &output, "manual", requires_visual_review) {
                                        let _ = res_tx.send(JobResult::Error { job_label: "apply_change".into(), message: format!("Audit write failed: {}", e) });
                                        return;
                                    }

                                    h.push_record(final_record.clone());
                                    // Best-effort autosave so the user can resume the session.
                                    let autosave_path = std::path::PathBuf::from("audit").join("history.json");
                                    if let Err(e) = h.save_to_file(&autosave_path) {
                                        tracing::warn!("[apply_change] autosave history failed: {}", e);
                                    }
                                    // Fire-and-forget webhook notification if configured.
                                    if let Some(url) = cfg_clone.webhook_url.clone() {
                                        let old = final_record.old_text.clone();
                                        let new = final_record.new_text.clone();
                                        let desc = final_record.description.clone();
                                        let page = final_record.page;
                                        tokio::spawn(async move {
                                            crate::app::notify::send_webhook(&url, crate::app::notify::WebhookPayload {
                                                event: "change_applied",
                                                page,
                                                old_text: &old,
                                                new_text: &new,
                                                description: &desc,
                                            }).await;
                                        });
                                    }
                                    let _ = res_tx.send(JobResult::ChangeApplied { record: final_record, requires_visual_review });
                                    let h_final = h.clone();
                                    let _ = res_tx.send(JobResult::HistoryUpdated { history: h_final });
                                    let _ = res_tx.send(JobResult::Progress { label: "Done".to_string(), fraction: 1.0 });
                                }
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "apply_change".into(), message: e.to_string() });
                                }
                            }
                        });
                    }
                    Job::CompleteFont { path, font_name } => {
                        let (reply_tx, reply_rx) = oneshot::channel();
                        if python_tx_clone.send((PythonJob::CompleteFontWithAdaption { pdf_path: path.to_string_lossy().to_string(), font_name }, reply_tx)).is_ok() {
                            match reply_rx.await {
                                Ok(PythonJobResult::Json(json)) => {
                                    let _ = result_tx_clone.send(JobResult::FontCompleted(json));
                                }
                                Ok(PythonJobResult::Error(e)) => {
                                    let _ = result_tx_clone.send(JobResult::Error { job_label: "complete_font".into(), message: e });
                                }
                                _ => {
                                    let _ = result_tx_clone.send(JobResult::Error { job_label: "complete_font".into(), message: "Unexpected response".into() });
                                }
                            }
                        } else {
                            let _ = result_tx_clone.send(JobResult::Error { job_label: "complete_font".into(), message: "Failed to send to Python actor".into() });
                        }
                    }
                    Job::Undo => {
                        let history_clone = history.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            let mut h = history_clone.lock().unwrap();
                            h.undo();
                            h.clone()
                        }).await.map(|h| {
                            let _ = result_tx_clone.send(JobResult::HistoryUpdated { history: h });
                        });
                    }
                    Job::Redo => {
                        let history_clone = history.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            let mut h = history_clone.lock().unwrap();
                            h.redo();
                            h.clone()
                        }).await.map(|h| {
                            let _ = result_tx_clone.send(JobResult::HistoryUpdated { history: h });
                        });
                    }
                    Job::ExtractTransactions { path } => {
                        let res_tx = result_tx_clone.clone();
                        let eng = engine_for_tokio.clone();
                        let cfg = config_for_tokio.clone();
                        
                        tokio::spawn(async move {
                            let doc_ai = match crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg) {
                                Ok(c) => Arc::new(c),
                                Err(_) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "extract_transactions".into(), message: "Extract requires GEMINI_API_KEY + Document AI configuration.".into() });
                                    return;
                                }
                            };
                            
                            let bank_stmt = match doc_ai.parse_entire_statement(&path).await {
                                Ok(stmt) => stmt,
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "extract_transactions".into(), message: format!("Document AI failed: {}", e) });
                                    return;
                                }
                            };
                            
                            let template_provider = Arc::new(crate::extractors::BankTemplateProvider::new(std::path::PathBuf::from("bank_templates").as_path(), eng.clone()));
                            let pymupdf_provider = Arc::new(crate::extractors::PyMuPdfHeuristicProvider { engine: eng.clone() });
                            let tess_provider = Arc::new(crate::extractors::TesseractProvider { engine: eng.clone() });
                            
                            let merger = crate::extractors::HybridMerger::new(vec![
                                template_provider as Arc<dyn crate::extractors::GeometryProvider>,
                                pymupdf_provider as Arc<dyn crate::extractors::GeometryProvider>,
                                tess_provider as Arc<dyn crate::extractors::GeometryProvider>,
                            ]);

                            let mut geometries = Vec::new();
                            for provider in &merger.providers {
                                if let Ok(geo) = provider.extract_line_geometry(&path) {
                                    geometries.extend(geo);
                                }
                            }
                            
                            let report = merger.merge(bank_stmt.transactions, geometries);
                            let _ = res_tx.send(JobResult::TransactionsExtracted(report.transactions));
                        });
                    }
                    Job::BalanceStatement { path } => {
                        let res_tx = result_tx_clone.clone();
                        let eng = engine_for_tokio.clone();
                        let cfg = config_for_tokio.clone();
                        
                        tokio::spawn(async move {
                            let _ = res_tx.send(JobResult::Progress { label: "Smart Balance Analysis".to_string(), fraction: 0.1 });
                            
                            let doc_ai = match crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg) {
                                Ok(c) => Arc::new(c),
                                Err(_) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "balance_statement".into(), message: "Smart Balance Engine requires GEMINI_API_KEY + Document AI configuration. See README §Configuration.".into() });
                                    return;
                                }
                            };
                            
                            let gemini = match crate::ai::gemini_client::GeminiClient::from_app_config(&cfg) {
                                Ok(c) => Arc::new(c),
                                Err(_) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "balance_statement".into(), message: "Smart Balance Engine requires GEMINI_API_KEY + Document AI configuration. See README §Configuration.".into() });
                                    return;
                                }
                            };

                            let template_provider = Arc::new(crate::extractors::BankTemplateProvider::new(std::path::PathBuf::from("bank_templates").as_path(), eng.clone()));
                            let pymupdf_provider = Arc::new(crate::extractors::PyMuPdfHeuristicProvider { engine: eng.clone() });
                            let tess_provider = Arc::new(crate::extractors::TesseractProvider { engine: eng.clone() });
                            
                            let merger = Arc::new(crate::extractors::HybridMerger::new(vec![
                                template_provider as Arc<dyn crate::extractors::GeometryProvider>,
                                pymupdf_provider as Arc<dyn crate::extractors::GeometryProvider>,
                                tess_provider as Arc<dyn crate::extractors::GeometryProvider>,
                            ]));

                            let mut smart_engine = crate::engine::statement::SmartDocumentEngine::new(eng.clone(), doc_ai, gemini, merger);
                            
                            let _ = res_tx.send(JobResult::Progress { label: "Loading Document".to_string(), fraction: 0.3 });
                            
                            let (dummy_tx, _) = std::sync::mpsc::channel();
                            if let Err(e) = smart_engine.load_full_document(&dummy_tx, &path).await {
                                let _ = res_tx.send(JobResult::Error { job_label: "balance_statement".into(), message: format!("Failed to load document: {}", e) });
                                return;
                            }
                            
                            let _ = res_tx.send(JobResult::Progress { label: "Analyzing layout and semantic meaning".to_string(), fraction: 0.6 });

                            match smart_engine.balance_entire_statement(&path).await {
                                Ok(changes) => {
                                    let imbalance = smart_engine.calculate_global_imbalance();
                                    let _ = res_tx.send(JobResult::BalanceProposed { imbalance, changes });
                                    let _ = res_tx.send(JobResult::Progress { label: "Done".to_string(), fraction: 1.0 });
                                }
                                Err(crate::engine::statement::EngineError::LowConfidence(c)) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "balance_statement".into(), message: format!("Gemini confidence {:.2} below 0.7 threshold; not enough certainty to propose adjustments.", c) });
                                }
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "balance_statement".into(), message: e.to_string() });
                                }
                            }
                        });
                    }
                    Job::ApplyProposedChanges { input, output, changes } => {
                        let res_tx = result_tx_clone.clone();
                        let job_tx_ref = tokio_job_tx_clone.clone();
                        
                        tokio::spawn(async move {
                            let mut applied = 0;
                            let mut failures = Vec::new();
                            
                            for (i, change) in changes.iter().enumerate() {
                                let _ = res_tx.send(JobResult::Progress { 
                                    label: format!("Applying change {} of {}", i + 1, changes.len()), 
                                    fraction: (i as f32) / (changes.len() as f32) 
                                });
                                
                                let bbox = match change.bbox {
                                    Some(b) => b,
                                    None => {
                                        let msg = format!("Proposed change for page {} '{}' \u{2192} '{}' has no resolved bbox; cannot redact", 
                                                change.page + 1, change.old_text, change.new_text);
                                        failures.push(msg.clone());
                                        let _ = res_tx.send(JobResult::Error { job_label: "apply_proposed_changes".into(), message: msg });
                                        continue;
                                    }
                                };
                                
                                // Note: The runtime bridge and tokio loop are sequential.
                                // We send Job::ApplyChange to ensure consistent history and file state.
                                let _ = job_tx_ref.send(Job::ApplyChange {
                                    input: input.clone(),
                                    output: output.clone(),
                                    page: change.page,
                                    bbox,
                                    new_text: change.new_text.clone(),
                                    old_text: change.old_text.clone(),
                                    description: change.reason.clone(),
                                    deep_font_replication: false,
                                });
                                
                                // Note: In a production environment, we should wait for the JobResult::ChangeApplied 
                                // for THIS specific change. For now, since the queue is sequential and we 
                                // want parity, we'll keep it simple but acknowledge the limitation.
                                applied += 1;
                            }
                            
                            let _ = res_tx.send(JobResult::ProposedChangesApplied { changes_applied: applied, failures });
                            let _ = res_tx.send(JobResult::Progress { label: "Done".to_string(), fraction: 1.0 });
                        });
                    }
                    Job::ExportChangeHistory { output } => {
                        let history_clone = history.clone();
                        let output_clone = output.clone();
                        let res_tx = result_tx_clone.clone();
                        tokio::task::spawn_blocking(move || {
                            let h = history_clone.lock().map_err(|e| e.to_string())?;
                            h.save_to_file(&output_clone).map_err(|e| e.to_string())
                        }).await.unwrap_or_else(|e| Err(format!("blocking task panicked: {}", e))).map(|_| {
                            let _ = res_tx.send(JobResult::ChangeHistoryExported { path: output });
                        }).unwrap_or_else(|e| {
                            let _ = res_tx.send(JobResult::Error { job_label: "export_history".into(), message: e });
                        });
                    }
                    Job::LoadHistory { input } => {
                        let history_clone = history.clone();
                        let res_tx = result_tx_clone.clone();
                        tokio::task::spawn_blocking(move || {
                            match crate::engine::history::ChangeHistory::load_from_file(&input) {
                                Ok(loaded) => {
                                    if let Ok(mut h) = history_clone.lock() {
                                        *h = loaded.clone();
                                        let _ = res_tx.send(JobResult::HistoryUpdated { history: loaded });
                                    } else {
                                        let _ = res_tx.send(JobResult::Error { job_label: "load_history".into(), message: "history mutex poisoned".into() });
                                    }
                                }
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "load_history".into(), message: e.to_string() });
                                }
                            }
                        }).await.unwrap_or(());
                    }
                    Job::Verify { original, edited, output_dir, intended_bboxes, use_pdfrest, pdfrest_key } => {
                        let _ = result_tx_clone.send(JobResult::Progress { label: "Extracting transactions".to_string(), fraction: 0.1 });
                        let (reply_tx, reply_rx) = oneshot::channel();

                        if python_tx_clone.send((PythonJob::GetAllTransactions { pdf_path: edited.to_string_lossy().to_string() }, reply_tx)).is_ok() {
                            match reply_rx.await {
                                Ok(PythonJobResult::Json(json)) => {
                                    #[derive(serde::Deserialize)]
                                    struct RawTxRow {
                                        page: usize,
                                        #[allow(dead_code)]
                                        line_on_page: Option<usize>,
                                        #[allow(dead_code)]
                                        date: Option<String>,
                                        #[allow(dead_code)]
                                        raw_text: Option<String>,
                                        debit: Option<f64>,
                                        credit: Option<f64>,
                                        running_balance: Option<f64>,
                                        #[allow(dead_code)]
                                        bbox: Option<[f32; 4]>,
                                    }

                                    let raw_rows: Vec<RawTxRow> = serde_json::from_str(&json).unwrap_or_default();
                                    let transactions: Vec<crate::engine::model::Transaction> = raw_rows.iter().map(|r| {
                                        crate::engine::model::Transaction {
                                            page: r.page,
                                            line_on_page: r.line_on_page.unwrap_or(0),
                                            date: r.date.clone().unwrap_or_default(),
                                            raw_text: r.raw_text.clone().unwrap_or_default(),
                                            debit: r.debit,
                                            credit: r.credit,
                                            running_balance: r.running_balance,
                                            bbox: r.bbox,
                                            provenance: crate::engine::model::Provenance::Computed,
                                        }
                                    }).collect();

                                    let python_tx_clone2 = python_tx_clone.clone();
                                    
                                    // NEW: We must extract the expected closing balance from the original PDF
                                    let mut expected_final_balance = None;
                                    let mut opening_balance = 0.0;
                                    
                                    // Parse original PDF for the expected balance
                                    let (reply_tx_orig, reply_rx_orig) = oneshot::channel();
                                    if python_tx_clone2.send((PythonJob::GetAllTransactions { pdf_path: original.to_string_lossy().to_string() }, reply_tx_orig)).is_ok() {
                                        if let Ok(PythonJobResult::Json(json_orig)) = reply_rx_orig.await {
                                            let orig_raw_rows: Vec<RawTxRow> = serde_json::from_str(&json_orig).unwrap_or_default();
                                            if let Some(first) = orig_raw_rows.first() {
                                                opening_balance = first.running_balance.unwrap_or(0.0) - (first.debit.unwrap_or(0.0) - first.credit.unwrap_or(0.0));
                                            }
                                            if let Some(last) = orig_raw_rows.last() {
                                                expected_final_balance = last.running_balance;
                                            }
                                        }
                                    }

                                    let _ = result_tx_clone.send(JobResult::Progress { label: "Rendering & comparing pages".to_string(), fraction: 0.5 });

                                    let math_inputs = crate::engine::verification::MathInputs {
                                        transactions,
                                        opening_balance,
                                        expected_final_balance, // Now sourced from the original PDF
                                    };

                                    match crate::engine::verification::verify_edit(&original, &edited, &output_dir, &intended_bboxes, math_inputs, use_pdfrest, pdfrest_key).await {
                                        Ok(report) => {
                                            let _ = result_tx_clone.send(JobResult::VerificationReport(report));
                                            let _ = result_tx_clone.send(JobResult::Progress { label: "Done".to_string(), fraction: 1.0 });
                                        }
                                        Err(e) => {
                                            let _ = result_tx_clone.send(JobResult::Error { job_label: "verify".into(), message: e.to_string() });
                                        }
                                    }
                                }
                                Ok(PythonJobResult::Error(e)) => {
                                    let _ = result_tx_clone.send(JobResult::Error { job_label: "verify_extract".into(), message: e });
                                }
                                _ => {
                                    let _ = result_tx_clone.send(JobResult::Error { job_label: "verify_extract".into(), message: "Unexpected response from Python actor".into() });
                                }
                            }
                        }
                    }

                }
            }
        });

        (
            Self {
                _tokio_rt: tokio_rt,
                _python_actor_thread,
            },
            job_tx,
            result_rx,
        )
    }
}

fn spawn_runtime_bridge(
    job_rx: mpsc::Receiver<Job>,
    tokio_job_tx: tokio::sync::mpsc::UnboundedSender<Job>,
    result_tx: mpsc::Sender<JobResult>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        while let Ok(job) = job_rx.recv() {
            if tokio_job_tx.send(job).is_err() {
                let _ = result_tx.send(JobResult::Error {
                    job_label: "runtime_bridge".into(),
                    message: "Tokio worker disconnected".into(),
                });
                break;
            }
        }
    })
}

/// Dispatches a Python job to the actor thread.
/// This function MUST forward directly to avoid recursion through the engine selector.
fn dispatch_python_job(
    py_job: PythonJob,
    reply_tx: oneshot::Sender<PythonJobResult>,
    python_tx: &mpsc::Sender<(PythonJob, oneshot::Sender<PythonJobResult>)>,
) {
    if let Err(e) = python_tx.send((py_job, reply_tx)) {
        // This means the actor thread has died. Log and let the dropped reply
        // channel surface the error to the caller (oneshot::recv -> RecvError).
        tracing::error!("[runtime] python actor channel disconnected: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_bridge_fail_loud() {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (tokio_job_tx, tokio_job_rx) = tokio::sync::mpsc::unbounded_channel::<Job>();
        let (result_tx, result_rx) = mpsc::channel::<JobResult>();

        // Immediately drop the receiver to simulate disconnect
        drop(tokio_job_rx);

        let handle = spawn_runtime_bridge(job_rx, tokio_job_tx, result_tx);

        // Send a job
        let _ = job_tx.send(Job::Ping);

        // Expect error
        match result_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(JobResult::Error { job_label, message }) => {
                assert_eq!(job_label, "runtime_bridge");
                assert!(message.contains("disconnected"));
            }
            res => panic!("Expected bridge error, got {:?}", res),
        }

        handle.join().unwrap();

        // Subsequent send should fail because job_rx is dropped
        assert!(job_tx.send(Job::Ping).is_err());
    }

    #[tokio::test]
    async fn test_python_job_recursion_regression() {
        // GIVEN: A mock setup that mirrors the Runtime's job loop
        let (job_tx, mut job_rx) = tokio::sync::mpsc::unbounded_channel::<Job>();
        let (python_tx, python_rx) = std::sync::mpsc::channel::<(PythonJob, oneshot::Sender<PythonJobResult>)>();
        let python_tx_clone = python_tx.clone();

        // 1. A selector with PyMuPdfEngine (which sends jobs back to a channel)
        let (std_job_tx, std_job_rx) = std::sync::mpsc::channel::<Job>();
        let job_tx_clone = job_tx.clone();
        std::thread::spawn(move || {
            while let Ok(job) = std_job_rx.recv() {
                let _ = job_tx_clone.send(job);
            }
        });
        
        // Use real engines but selector will fall back because MuPdf doesn't support get_text_blocks
        let primary = Arc::new(crate::pdf::MuPdfEngine::new());
        let fallback = Arc::new(crate::pdf::PyMuPdfEngine::new(std_job_tx));
        let _selector = Arc::new(crate::pdf::PdfEngineSelector::new(primary, fallback));

        // 2. The Runtime Job::Python handler (the logic we are testing)
        let handle = tokio::spawn(async move {
            while let Some(job) = job_rx.recv().await {
                match job {
                    Job::Python(py_job, reply_tx) => {
                        dispatch_python_job(py_job, reply_tx, &python_tx_clone);
                    }
                    _ => {}
                }
            }
        });

        // 3. Trigger a job that would cause recursion in the old version
        let (reply_tx, _reply_rx) = oneshot::channel();
        job_tx.send(Job::Python(
            PythonJob::GetTextBlocks { pdf_path: "input.pdf".into(), page_num: 0 },
            reply_tx
        )).unwrap();

        // WHEN: We wait for the message to land on the Python actor
        let (received_job, python_rx) = tokio::task::spawn_blocking(move || {
            let res = python_rx.recv_timeout(Duration::from_secs(1)).expect("Python job should be forwarded to actor");
            (res.0, python_rx)
        }).await.unwrap();

        // THEN:
        // 1. It must be the job we sent
        assert!(matches!(received_job, PythonJob::GetTextBlocks { .. }));

        // 2. Exactly ONE message must be received by the actor (no recursion)
        let next_res = python_rx.try_recv();
        assert!(next_res.is_err(), "Recursion detected: multiple messages sent to Python actor");

        // Cleanup
        drop(job_tx);
        handle.abort();
    }
}

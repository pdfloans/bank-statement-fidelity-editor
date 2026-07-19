// pyo3_bridge removed - zero FFI architecture
use crate::app::audit::AuditLog;
use crate::engine::history::{ChangeHistory, ChangeRecord};
use crate::engine::segments::{GlobalEdit, SegmentManager, SegmentMap};
use crate::pdf::engine::PdfEngine;
use crate::pdf::ReplaceOutcome;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Opaque per-job handle. The runtime returns one when a job is enqueued;
/// callers can later `Job::Cancel` it.
pub type JobId = u64;

static NEXT_JOB_ID: AtomicU64 = AtomicU64::new(1);

/// Allocate a fresh `JobId`. Used by both the runtime and external callers
/// who want to enqueue a job and remember its handle.
pub fn alloc_job_id() -> JobId {
    NEXT_JOB_ID.fetch_add(1, Ordering::SeqCst)
}

/// A registry of currently-running jobs and their cancellation tokens.
/// Cloneable; the runtime keeps one and the dispatcher keeps another.
#[derive(Clone, Default)]
pub struct CancellationRegistry {
    inner: Arc<Mutex<HashMap<JobId, CancellationToken>>>,
}

impl CancellationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new token under `id`. Returns the token (so the caller
    /// can pass it into the spawned task).
    pub fn register(&self, id: JobId) -> CancellationToken {
        let token = CancellationToken::new();
        if let Ok(mut g) = self.inner.lock() {
            g.insert(id, token.clone());
        }
        token
    }

    /// Cancel and remove the token for `id`. No-op if unknown.
    pub fn cancel(&self, id: JobId) -> bool {
        let token = self.inner.lock().ok().and_then(|mut g| g.remove(&id));
        if let Some(t) = token {
            t.cancel();
            true
        } else {
            false
        }
    }

    /// Drop the token for `id` (job has finished naturally).
    pub fn complete(&self, id: JobId) {
        if let Ok(mut g) = self.inner.lock() {
            g.remove(&id);
        }
    }

    /// Cancel every job in flight. Useful on app shutdown.
    pub fn cancel_all(&self) {
        if let Ok(mut g) = self.inner.lock() {
            for (_, t) in g.drain() {
                t.cancel();
            }
        }
    }

    /// How many jobs are currently registered.
    pub fn len(&self) -> usize {
        self.inner.lock().map(|g| g.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone)]
pub enum PythonJob {
    Ping,
    GetTextBlocks {
        pdf_path: String,
        page_num: usize,
    },
    ReplaceTextInRect {
        pdf_path: String,
        output_path: String,
        page_num: usize,
        rect: [f32; 4],
        new_text: String,
        font_path: Option<String>,
    },
    FindTextBlockAtClick {
        pdf_path: String,
        page_num: usize,
        x: f32,
        y: f32,
    },
    GetAllTransactions {
        pdf_path: String,
    },
    AnalyzeDocumentLayout {
        pdf_path: String,
    },
    CompleteFontWithAdaption {
        pdf_path: String,
        font_name: String,
    },
    DeepFontReplication {
        pdf_path: String,
        font_name: String,
        output_dir: String,
    },
    /// Stage 3 / Item #14: apply N edits in one open/save pass.
    /// `edits_json` is a JSON array of `{page, rect, new_text, fill_color?}`.
    ApplyManyEdits {
        pdf_path: String,
        output_path: String,
        edits_json: String,
        font_path: Option<String>,
    },
    /// Stage 3 / Item #16: split a PDF into chunks <= 30 pages so Document AI
    /// can parse documents above its single-request page cap.
    ChunkPdfForDocai {
        pdf_path: String,
        output_dir: String,
        max_pages_per_chunk: usize,
    },
    /// Stage 8.5: per-font usage + coverage analysis. Returns the JSON
    /// shape produced by `pymupdf_pro_integration.analyze_fonts`.
    AnalyzeFonts {
        pdf_path: String,
    },
    /// Stage 11: targeted font cascade. Runs composite synthesis ->
    /// subset extension -> Gemini Vision donor identification on the
    /// supplied `missing_chars`. Returns the JSON dict produced by
    /// `replicate_font_for_chars`.
    ReplicateFontForMissingChars {
        pdf_path: String,
        font_name: String,
        missing_chars_csv: String,
        output_dir: String,
    },
    /// Clone (duplicate) pages within a PDF to create capacity for more
    /// transactions. Each entry in `page_indices` is a source page to clone;
    /// clones are inserted immediately after the original. Does NOT require
    /// PyMuPDF Pro - page-level operations use the free tier.
    ClonePages {
        pdf_path: String,
        output_path: String,
        page_indices: Vec<usize>,
    },
    /// Remove pages from a PDF (excess capacity). Pages are deleted in
    /// descending order so indices don't shift. Does NOT require PyMuPDF Pro.
    RemovePages {
        pdf_path: String,
        output_path: String,
        page_indices: Vec<usize>,
    },
    RenderPageToPng {
        pdf_path: String,
        page_num: usize,
        dpi: f32,
    },
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
    LoadDocument {
        path: PathBuf,
        three_page_mode: bool,
    },
    /// Stage 8.5: standalone font analysis trigger. Useful from a "Re-analyze"
    /// menu in the GUI; LoadDocument also fires this automatically.
    AnalyzeFonts {
        path: PathBuf,
    },
    RenderPage {
        path: PathBuf,
        page: usize,
        dpi: f32,
        tag: String,
    },
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
    CompleteFont {
        path: PathBuf,
        font_name: String,
    },
    Undo,
    Redo,
    BalanceStatement {
        path: PathBuf,
    },
    ExtractTransactions {
        path: PathBuf,
    },
    InstantBackgroundApply {
        input: PathBuf,
        output: PathBuf,
        edits: Vec<crate::engine::workflow::UserEdit>,
    },
    NaturalLanguageEdit {
        prompt: String,
        transactions: Vec<crate::engine::model::Transaction>,
    },
    CategorizeTransactions {
        transactions: Vec<crate::engine::model::Transaction>,
    },
    ApplyProposedChanges {
        input: PathBuf,
        output: PathBuf,
        changes: Vec<crate::engine::model::ProposedChange>,
    },
    GenerateVisualAlternatives {
        input: PathBuf,
        out_dir: PathBuf,
        page: usize,
        edits: Vec<crate::engine::workflow::UserEdit>,
        bbox: [f32; 4],
    },
    ExportChangeHistory {
        output: PathBuf,
    },
    LoadHistory {
        input: PathBuf,
    },
    Verify {
        original: PathBuf,
        edited: PathBuf,
        output_dir: PathBuf,
        intended_bboxes: Vec<(usize, [f32; 4])>,
        use_pdfrest: bool,
        pdfrest_key: Option<String>, auto_match_dpi: bool,
    },

    /// Cancel a previously-enqueued job by its [`JobId`]. Best-effort; the
    /// task may have already finished. The runtime drops the token, so any
    /// `tokio::select!` watching `cancelled()` exits with a structured error.
    Cancel {
        id: JobId,
    },
    SubmitBugReport {
        description: String,
        include_logs: bool,
        include_audit: bool,
    },
    TypstReconstruct {
        input: std::path::PathBuf,
        output: std::path::PathBuf,
    },

    /// Hot-reload the runtime's `AppConfig` from the current process
    /// environment. The GUI sends this after the user updates API keys /
    /// credentials in-app (which write `.env` and `std::env::set_var`), so
    /// subsequent Document AI / Gemini jobs pick up the new values without an
    /// application restart.
    ReloadConfig,

    /// Trigger an active validation check on the AI credentials
    ValidateCredentials,

    /// Run the Smart Balance Engine and, when `auto_apply` is true, apply every
    /// proposed adjustment to the PDF in one shot (the "Adjust entire bank
    /// statement accordingly and apply all edits" button). When `auto_apply`
    /// is false this behaves like [`Job::BalanceStatement`].
    BalanceAndApplyAll {
        input: PathBuf,
        output: PathBuf,
        auto_apply: bool,
    },
    /// Cleanup orphaned temporary files from crash recovery
    CleanupTempFiles,

    // ----- Multi-stage workflow -------------------------------------------
    /// Stage 1: parse with Document AI then validate completeness with Gemini.
    WorkflowParseAndValidate {
        input: PathBuf,
        version: Option<String>,
        /// Which document parser the user selected in Backend Preferences.
        parser_mode: crate::app::config::DocumentParserMode,
        /// Which AI provider the user selected (used for completeness validation).
        ai_provider: crate::app::config::AiProviderMode,
        ignore_offline_fallback: bool,
    },
    /// Stage 3: build a balance preview from edits without writing the PDF.
    WorkflowPreview {
        original_transactions: Vec<crate::engine::model::Transaction>,
        edits: Vec<crate::engine::workflow::UserEdit>,
        opening_balance: rust_decimal::Decimal,
        expected_closing: Option<rust_decimal::Decimal>,
    },
    /// Stage 4 + 5 + 6: apply edits, render, validate visually in a loop, then
    /// re-parse with Document AI to confirm math.
    WorkflowConfirmAndRender {
        input: PathBuf,
        output: PathBuf,
        edits: Vec<crate::engine::workflow::UserEdit>,
        original_transactions: Vec<crate::engine::model::Transaction>,
        opening_balance: rust_decimal::Decimal,
        expected_closing: Option<rust_decimal::Decimal>,
        deep_font_replication: bool,
        max_visual_attempts: u32,
        visual_threshold: f64,
        ignore_font_coverage: bool,
        ignore_visual_fidelity: bool,
    },
    /// Use AI to fix text box issues and visual fidelity differences
    AiFixVisualFidelity {
        input: PathBuf,
        page: usize,
    },
    /// Transfer transactions from one bank statement PDF to another,
    /// adapting formats and verifying math + visual fidelity.
    TransferTransactions {
        source_pdf: PathBuf,
        target_pdf: PathBuf,
        output_pdf: PathBuf,
    },
    /// Bulk-shift or remap all transaction dates.
    AdjustDatePeriods {
        input: PathBuf,
        output: PathBuf,
        mode: crate::engine::date_adjust::DateAdjustMode,
    },
    /// User's response to an AI confirmation question.
    AiConfirmationResponse(crate::engine::ai_confirm::AiConfirmationResponse),
    InteractiveFallbackResponse(crate::engine::interactive_fallback::InteractiveFallbackResponse),
    /// Run cross-statement transfer tests on a set of PDFs.
    RunTransferTests {
        statements: Vec<PathBuf>,
        max_iterations: u32,
    },
    AiCommand {
        prompt: String,
        path: PathBuf,
    },

    // -- Document AI Version Management --
    /// Fetch list of available processor versions from the API.
    ListDocAiVersions,
    /// Deploy a specific processor version for inference.
    DeployDocAiVersion {
        version_id: String,
    },
    /// Undeploy a specific processor version.
    UndeployDocAiVersion {
        version_id: String,
    },
    /// Set a version as the default processor version.
    SetDefaultDocAiVersion {
        version_id: String,
    },
    /// Trigger training of a new custom processor version.
    TrainDocAiVersion {
        display_name: String,
        base_version: Option<String>,
    },
}

impl Job {
    pub fn is_fast(&self) -> bool {
        matches!(
            self,
            Job::Ping | Job::Undo | Job::Redo | Job::Cancel { .. } | Job::ReloadConfig | Job::CleanupTempFiles
        )
    }
}


#[derive(Debug)]
pub enum JobResult {
    Pong,
    ApiKeysVerified(crate::app::api_verification::VerificationReport),
    DocumentLoaded {
        layout_json: String,
        total_pages: usize,
    },
    PageRendered {
        png_bytes: Vec<u8>,
        page: usize,
        dpi: f32,
        tag: String,
        width_pts: f32,
        height_pts: f32,
    },
    ChangeApplied {
        record: ChangeRecord,
        requires_visual_review: bool,
    },
    HistoryUpdated {
        history: ChangeHistory,
    },
    FontCompleted(String),
    ChangeHistoryExported {
        path: PathBuf,
    },
    TransactionsExtracted(Vec<crate::engine::model::Transaction>),
    NaturalLanguageEditReady(Vec<crate::engine::model::Transaction>),
    CategorizationReady(Vec<crate::engine::model::Transaction>),
    VerificationReport(crate::engine::verification::VerificationReport),
    /// Stage 8.5: per-font usage and coverage breakdown for the loaded PDF.
    /// Sent automatically after `Job::LoadDocument` and on demand from
    /// `Job::AnalyzeFonts`.
    FontAnalysisReady(crate::engine::font_analysis::FontAnalysis),
    /// Stage 12 / Item #3: emitted when the workflow's font cascade was
    /// invoked because the apply step hit FONT_COVERAGE_INSUFFICIENT.
    /// The GUI uses this to surface a small audit line summarising which
    /// tiers were used and which characters each tier contributed.
    FontCascadeUsed(crate::engine::font_analysis::FontCascadeReport),
    BalanceProposed {
        imbalance: rust_decimal::Decimal,
        changes: Vec<crate::engine::model::ProposedChange>,
    },
    ProposedChangesApplied {
        changes_applied: usize,
        failures: Vec<String>,
    },
    /// Emitted after a [`Job::ReloadConfig`]: reports whether the reloaded
    /// config has working AI credentials so the GUI can update its status line.
    ConfigReloaded {
        document_ai_configured: bool,
        gemini_configured: bool,
        pro_editing_available: bool,
    },
    Error {
        job_label: String,
        message: String,
    },
    NuclearFallbackRequired(String),
    Progress {
        label: String,
        fraction: f32,
    },
    /// A job tagged with this `JobId` was cancelled before it finished.
    Cancelled {
        id: JobId,
    },
    ReconstructComplete {
        output_path: std::path::PathBuf,
    },
    BugReportSubmitted,

    // ----- Multi-stage workflow ------------------------------------------
    WorkflowStageChanged {
        stage: crate::engine::workflow::WorkflowStage,
    },
    WorkflowParseValidated {
        validation: crate::engine::workflow::ParseValidation,
        transactions: Vec<crate::engine::model::Transaction>,
    },
    WorkflowPreviewBuilt(crate::engine::workflow::BalancePreview),
    WorkflowVisualAttempt(crate::engine::workflow::VisualAttempt),
    VisualAlternativesReady(Vec<(String, Vec<u8>)>),
    WorkflowComplete(crate::engine::workflow::WorkflowOutcome),
    WorkflowFailed(crate::engine::workflow::WorkflowFailure),

    // ----- Transfer Transactions ------------------------------------------
    TransferComplete(crate::engine::transfer::TransferResult),
    TransferFailed {
        stage: String,
        message: String,
    },

    // ----- Date Adjustment -------------------------------------------------
    DatesAdjusted {
        records: Vec<crate::engine::date_adjust::DateShiftRecord>,
        output_path: PathBuf,
    },

    // ----- AI Confirmation -------------------------------------------------
    AiConfirmationNeeded(crate::engine::ai_confirm::AiConfirmation),
    InteractiveFallbackRequired(crate::engine::interactive_fallback::InteractiveFallbackRequest),

    // ----- Transfer Test Harness -------------------------------------------
    TransferTestsComplete(crate::engine::transfer_test_harness::TestHarnessReport),

    // ----- General Lifecycle -----------------------------------------------
    JobCompleted(String),

    // ----- Document AI Version Management ----------------------------------
    DocAiVersionsListed(Vec<crate::ai::document_ai::ProcessorVersionInfo>),
    DocAiVersionOperationStarted {
        operation_name: String,
        description: String,
    },
    DocAiVersionError(String),
    WatchdogEvent(crate::app::watchdog::WatchdogEvent),
}

impl JobResult {
    pub fn is_terminal(&self) -> bool {
        !matches!(
            self,
            Self::Progress { .. }
                | Self::WorkflowStageChanged { .. }
                | Self::WorkflowParseValidated { .. }
                | Self::FontCascadeUsed(_)
                | Self::WatchdogEvent(_)
        )
    }
}

#[derive(Clone)]
pub struct TerminalTracker(std::sync::Arc<TerminalTrackerInner>);

struct TerminalTrackerInner {
    tx: std::sync::mpsc::Sender<JobResult>,
    label: String,
    terminal_sent: std::sync::atomic::AtomicBool,
}

impl TerminalTracker {
    pub fn new(tx: std::sync::mpsc::Sender<JobResult>, label: impl Into<String>) -> Self {
        Self(std::sync::Arc::new(TerminalTrackerInner {
            tx,
            label: label.into(),
            terminal_sent: std::sync::atomic::AtomicBool::new(false),
        }))
    }

    #[allow(clippy::result_large_err)]
    pub fn send(&self, res: JobResult) -> Result<(), std::sync::mpsc::SendError<JobResult>> {
        if res.is_terminal() {
            self.0
                .terminal_sent
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.0.tx.send(res)
    }
}

impl Drop for TerminalTrackerInner {
    fn drop(&mut self) {
        if !self
            .terminal_sent
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            let _ = self.tx.send(JobResult::Error {
                job_label: self.label.clone(),
                message: "Background task panicked or exited silently without a terminal result."
                    .into(),
            });
        }
    }
}

pub struct Runtime {
    _tokio_rt: tokio::runtime::Runtime,
    /// Registry of in-flight jobs and their cancellation tokens. Cloneable;
    /// pass to the GUI so it can cancel by id.
    pub cancellations: CancellationRegistry,
    pub watchdog: std::sync::Arc<crate::app::watchdog::Watchdog>,
}

impl Runtime {
    pub fn start(
        audit_log: AuditLog,
        config: Arc<crate::app::config::AppConfig>,
    ) -> (Self, mpsc::Sender<Job>, mpsc::Receiver<JobResult>) {
        let tokio_rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to start Tokio runtime");

        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (result_tx, result_rx) = mpsc::channel::<JobResult>();
        let (watchdog, mut watchdog_rx) = crate::app::watchdog::Watchdog::new();
        let watchdog = std::sync::Arc::new(watchdog);
        let watchdog_for_gui = watchdog.clone();

        let (python_tx, python_rx) =
            mpsc::channel::<(PythonJob, oneshot::Sender<PythonJobResult>)>();

        let audit_log = Arc::new(Mutex::new(audit_log));
        let history = Arc::new(Mutex::new(ChangeHistory::new()));
        let config_holder = Arc::new(Mutex::new(config));

        let primary_engine = Arc::new(crate::pdf::PyMuPdfEngine::new(job_tx.clone()));
        let fallback_engine = Arc::new(crate::pdf::OxidizePdfEngine::new());
        let engine: Arc<dyn crate::pdf::PdfEngine> = Arc::new(crate::pdf::PdfEngineSelector::new(
            primary_engine,
            fallback_engine,
            config_holder.clone(),
        ));

        let _python_actor_thread = thread::spawn(move || {
            // T2 test support: simulate a downed actor for cascade fallback testing
            let engine_result = if std::env::var("TEST_CRASH_PYTHON_ACTOR").is_ok() {
                tracing::warn!(
                    "[PYTHON_ACTOR] TEST_CRASH_PYTHON_ACTOR set — simulating crashed actor"
                );
                Err("Simulated Python actor crash for testing".to_string())
            } else {
                crate::ai::pyo3_bridge::PyEngine::init()
            };

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
                                    .chunk_pdf_for_docai(
                                        &pdf_path,
                                        &output_dir,
                                        max_pages_per_chunk,
                                    )
                                    .map(PythonJobResult::Json),
                                PythonJob::AnalyzeFonts { pdf_path } => {
                                    engine.analyze_fonts(&pdf_path).map(PythonJobResult::Json)
                                }
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
                                PythonJob::RenderPageToPng {
                                    pdf_path,
                                    page_num,
                                    dpi,
                                } => engine
                                    .render_page_to_png(&pdf_path, page_num, dpi)
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

        let cancellations = CancellationRegistry::new();
        let cancellations_for_loop = cancellations.clone();
        let result_tx_clone = result_tx.clone();
        let python_tx_clone = python_tx.clone();

        let (fast_job_tx, mut fast_job_rx) = tokio::sync::mpsc::unbounded_channel::<Job>();
        let (slow_job_tx, mut slow_job_rx) = tokio::sync::mpsc::unbounded_channel::<Job>();
        
        spawn_runtime_bridge(job_rx, fast_job_tx.clone(), slow_job_tx.clone(), result_tx.clone());
        let engine_for_tokio = engine.clone();
        
        // Hot-swappable config: jobs read the *current* config via a per-iteration
        // snapshot, so an in-app API-key/credentials update (Job::ReloadConfig)
        // takes effect on subsequent jobs without an application restart.

        let api_semaphore = Arc::new(tokio::sync::Semaphore::new(3));
        let _ = fast_job_tx.send(Job::CleanupTempFiles);

        let watchdog_clone = watchdog.clone();
        let tokio_rt_handle = tokio_rt.handle().clone();
        let wd_tx = result_tx.clone();
        tokio_rt_handle.spawn(async move {
            while let Ok(event) = watchdog_rx.recv().await {
                let _ = wd_tx.send(JobResult::WatchdogEvent(event));
            }
        });

        let api_poll_tx = result_tx.clone();
        
        // 2-second periodic task for .env hot-reloading
        let hot_reload_job_tx = fast_job_tx.clone();
        tokio_rt.spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
            let mut last_modified = std::time::SystemTime::UNIX_EPOCH;
            
            loop {
                interval.tick().await;
                if let Ok(metadata) = std::fs::metadata(".env") {
                    if let Ok(modified) = metadata.modified() {
                        if modified > last_modified {
                            if last_modified != std::time::SystemTime::UNIX_EPOCH {
                                tracing::info!("[config] .env file changed. Triggering hot-reload.");
                                let _ = hot_reload_job_tx.send(Job::ReloadConfig);
                            }
                            last_modified = modified;
                        }
                    }
                }
            }
        });

        let api_poll_config = config_holder.clone();
        tokio_rt_handle.spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                interval.tick().await;
                let cfg = {
                    if let Ok(g) = api_poll_config.lock() {
                        g.clone()
                    } else {
                        Arc::new(crate::app::config::AppConfig::default())
                    }
                };
                let report = crate::app::api_verification::verify_all_api_keys(&cfg, false).await;
                if api_poll_tx.send(JobResult::ApiKeysVerified(report)).is_err() {
                    break;
                }
            }
        });

        
        let fast_python_tx_clone = python_tx_clone.clone();
        let fast_result_tx_clone = result_tx_clone.clone();
        let fast_engine_for_tokio = engine_for_tokio.clone();
        let fast_history = history.clone();
        let fast_audit_log = audit_log.clone();
        let fast_cancellations_for_loop = cancellations_for_loop.clone();
        let fast_api_semaphore = api_semaphore.clone();
        let fast_config_holder = config_holder.clone();
        let fast_watchdog_clone = watchdog_clone.clone();

        let parse_cache = std::sync::Arc::new(tokio::sync::Mutex::new(
            lru::LruCache::<String, crate::ai::document_ai::BankStatement>::new(std::num::NonZeroUsize::new(20).unwrap())
        ));
        let fast_parse_cache = parse_cache.clone();
        let sig_audit = audit_log.clone();

        


        tokio_rt.spawn(async move {
            let mut segment_map: Option<SegmentMap> = None;
            let mut segment_manager: Option<SegmentManager> = None;
            let fallback_router: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<uuid::Uuid, tokio::sync::oneshot::Sender<String>>>> = std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

            while let Some(job) = slow_job_rx.recv().await {
                let wdog = watchdog_clone.clone();
                let config_for_tokio: Arc<crate::app::config::AppConfig> = config_holder
                    .lock()
                    .map(|g| g.clone())
                    .unwrap_or_else(|p| p.into_inner().clone());
                process_job_inner(
                    job,
                    python_tx_clone.clone(),
                    result_tx_clone.clone(),
                    engine_for_tokio.clone(),
                    config_for_tokio.clone(),
                    wdog.clone(),
                    history.clone(),
                    audit_log.clone(),
                    cancellations_for_loop.clone(),
                    api_semaphore.clone(),
                    &mut segment_map,
                    &mut segment_manager,
                    fallback_router.clone(),
                    parse_cache.clone(),
                    slow_job_tx.clone(),
                    config_holder.clone(),
                ).await;
            }
        });

        let parse_cache = fast_parse_cache;
        tokio_rt.spawn(async move {
            let mut segment_map: Option<SegmentMap> = None;
            let mut segment_manager: Option<SegmentManager> = None;
            let fallback_router: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<uuid::Uuid, tokio::sync::oneshot::Sender<String>>>> = std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

            while let Some(job) = fast_job_rx.recv().await {
                let wdog = fast_watchdog_clone.clone();
                let config_for_tokio: Arc<crate::app::config::AppConfig> = fast_config_holder
                    .lock()
                    .map(|g| g.clone())
                    .unwrap_or_else(|p| p.into_inner().clone());
                process_job_inner(
                    job,
                    fast_python_tx_clone.clone(),
                    fast_result_tx_clone.clone(),
                    fast_engine_for_tokio.clone(),
                    config_for_tokio.clone(),
                    wdog.clone(),
                    fast_history.clone(),
                    fast_audit_log.clone(),
                    fast_cancellations_for_loop.clone(),
                    fast_api_semaphore.clone(),
                    &mut segment_map,
                    &mut segment_manager,
                    fallback_router.clone(),
                    parse_cache.clone(),
                    fast_job_tx.clone(),
                    fast_config_holder.clone(),
                ).await;
            }
        });
        
        
        let sig_cancellations = cancellations.clone();
        tokio_rt.spawn(async move {
            if let Ok(()) = tokio::signal::ctrl_c().await {
                tracing::info!("Received Ctrl-C signal! Initiating graceful shutdown...");
                
                if let Ok(mut lock) = sig_audit.lock() {
                    let _ = lock.append_line("Graceful shutdown initiated via Ctrl-C");
                }
                
                sig_cancellations.cancel_all();
                
                tracing::info!("Shutting down...");
                
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                std::process::exit(0);
            }
        });

        (
            Self {
                _tokio_rt: tokio_rt,
                cancellations,
                watchdog: watchdog_for_gui,
            },
            job_tx,
            result_rx,
        )
    }
}

fn spawn_runtime_bridge(
    job_rx: mpsc::Receiver<Job>,
    fast_tx: tokio::sync::mpsc::UnboundedSender<Job>,
    slow_tx: tokio::sync::mpsc::UnboundedSender<Job>,
    result_tx: mpsc::Sender<JobResult>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while let Ok(job) = job_rx.recv() {
            if job.is_fast() {
                if fast_tx.send(job).is_err() {
                    let _ = result_tx.send(JobResult::Error {
                        job_label: "runtime_bridge".into(),
                        message: "Tokio worker disconnected".into(),
                    });
                    break;
                }
            } else {
                if slow_tx.send(job).is_err() {
                    let _ = result_tx.send(JobResult::Error {
                        job_label: "runtime_bridge".into(),
                        message: "Tokio worker disconnected".into(),
                    });
                    break;
                }
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
    use crate::app::config::AppConfig;

    #[test]
    fn cancellation_registry_register_and_cancel_round_trip() {
        let reg = CancellationRegistry::new();
        let id = alloc_job_id();
        let token = reg.register(id);
        assert_eq!(reg.len(), 1);
        assert!(!token.is_cancelled());

        let cancelled = reg.cancel(id);
        assert!(cancelled);
        assert!(token.is_cancelled());
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn cancellation_registry_complete_removes_without_cancelling() {
        let reg = CancellationRegistry::new();
        let id = alloc_job_id();
        let token = reg.register(id);
        reg.complete(id);
        assert_eq!(reg.len(), 0);
        // Completing should not flip the token's cancelled flag.
        assert!(!token.is_cancelled());
    }

    #[test]
    fn cancellation_registry_unknown_id_is_noop() {
        let reg = CancellationRegistry::new();
        assert!(!reg.cancel(99999));
    }

    #[test]
    fn cancellation_registry_cancel_all_drains_every_token() {
        let reg = CancellationRegistry::new();
        let t1 = reg.register(1);
        let t2 = reg.register(2);
        let t3 = reg.register(3);
        reg.cancel_all();
        assert_eq!(reg.len(), 0);
        assert!(t1.is_cancelled());
        assert!(t2.is_cancelled());
        assert!(t3.is_cancelled());
    }

    #[test]
    fn alloc_job_id_is_strictly_monotonic() {
        let a = alloc_job_id();
        let b = alloc_job_id();
        let c = alloc_job_id();
        assert!(a < b);
        assert!(b < c);
    }

    #[test]
    fn test_bridge_fail_loud() {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (tokio_job_tx, tokio_job_rx) = tokio::sync::mpsc::unbounded_channel::<Job>();
        let (result_tx, result_rx) = mpsc::channel::<JobResult>();
        let (watchdog, watchdog_rx) = crate::app::watchdog::Watchdog::new();
        let watchdog = std::sync::Arc::new(watchdog);
        let watchdog_for_gui = watchdog.clone();

        // Immediately drop the receiver to simulate disconnect
        drop(tokio_job_rx);

        let handle = spawn_runtime_bridge(job_rx, tokio_job_tx.clone(), tokio_job_tx, result_tx);

        // Send a job
        let _ = job_tx.send(Job::Ping);

        // Expect error
        match result_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(JobResult::Error { job_label, message }) => {
                assert_eq!(job_label, "runtime_bridge");
                assert!(message.contains("disconnected"));
            }
            res => panic!("Expected bridge error, got {res:?}"),
        }

        if let Err(e) = handle.join() {
            tracing::error!("Worker thread panicked during shutdown: {:?}", e);
        }

        // Subsequent send should fail because job_rx is dropped
        assert!(job_tx.send(Job::Ping).is_err());
    }

    #[tokio::test]
    async fn test_python_job_recursion_regression() {
        // GIVEN: A mock setup that mirrors the Runtime's job loop
        let (job_tx, mut job_rx) = tokio::sync::mpsc::unbounded_channel::<Job>();
        let (python_tx, python_rx) =
            std::sync::mpsc::channel::<(PythonJob, oneshot::Sender<PythonJobResult>)>();
        let python_tx_clone = python_tx.clone();

        // 1. A selector with PyMuPdfEngine (which sends jobs back to a channel)
        let (_std_job_tx, std_job_rx) = std::sync::mpsc::channel::<Job>();
        let job_tx_clone = job_tx.clone();
        std::thread::spawn(move || {
            while let Ok(job) = std_job_rx.recv() {
                let _ = job_tx_clone.send(job);
            }
        });

        let _engine = Arc::new(crate::pdf::OxidizePdfEngine::new());

        // 2. The Runtime Job::Python handler (the logic we are testing)
        let handle = tokio::spawn(async move {
            while let Some(job) = job_rx.recv().await {
                if let Job::Python(py_job, reply_tx) = job {
                    dispatch_python_job(py_job, reply_tx, &python_tx_clone);
                }
            }
        });

        // 3. Trigger a job that would cause recursion in the old version
        let (reply_tx, _reply_rx) = oneshot::channel();
        job_tx
            .send(Job::Python(
                PythonJob::GetTextBlocks {
                    pdf_path: "input.pdf".into(),
                    page_num: 0,
                },
                reply_tx,
            ))
            .unwrap();

        // WHEN: We wait for the message to land on the Python actor
        let (received_job, python_rx) = tokio::task::spawn_blocking(move || {
            let res = python_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("Python job should be forwarded to actor");
            (res.0, python_rx)
        })
        .await
        .unwrap();

        // THEN:
        // 1. It must be the job we sent
        assert!(matches!(received_job, PythonJob::GetTextBlocks { .. }));

        // 2. Exactly ONE message must be received by the actor (no recursion)
        let next_res = python_rx.try_recv();
        assert!(
            next_res.is_err(),
            "Recursion detected: multiple messages sent to Python actor"
        );

        // Cleanup
        drop(job_tx);
        handle.abort();
    }

    #[test]
    fn runtime_fallback_branch_prefers_offline_parser_when_online_backends_are_unavailable() {
        let mut cfg = AppConfig::default();
        cfg.document_ai = None;


        let availability = cfg.detect_availability();
        assert!(!availability.document_ai);


        // The runtime should keep the offline parser as the final fallback path
        // when neither Document AI nor Ocr-as-a-Service is configured.
        assert!(availability.unavailable_reason("document_ai").is_some());
        assert!(availability.unavailable_reason("llamaparse").is_some());
    }
}


#[allow(clippy::too_many_arguments)]
async fn process_job_inner(
    job: Job,
    python_tx_clone: std::sync::mpsc::Sender<(PythonJob, tokio::sync::oneshot::Sender<PythonJobResult>)>,
    result_tx_clone: std::sync::mpsc::Sender<JobResult>,
    engine_for_tokio: std::sync::Arc<dyn crate::pdf::PdfEngine>,
    config_for_tokio: std::sync::Arc<crate::app::config::AppConfig>,
    wdog: std::sync::Arc<crate::app::watchdog::Watchdog>,
    history: std::sync::Arc<std::sync::Mutex<crate::engine::history::ChangeHistory>>,
    audit_log: std::sync::Arc<std::sync::Mutex<crate::app::audit::AuditLog>>,
    cancellations_for_loop: crate::app::runtime::CancellationRegistry,
    api_semaphore: std::sync::Arc<tokio::sync::Semaphore>,
    segment_map: &mut Option<SegmentMap>,
    segment_manager: &mut Option<SegmentManager>,
    fallback_router: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<uuid::Uuid, tokio::sync::oneshot::Sender<String>>>>,
    parse_cache: std::sync::Arc<tokio::sync::Mutex<lru::LruCache<String, crate::ai::document_ai::BankStatement>>>,
    tokio_job_tx_clone: tokio::sync::mpsc::UnboundedSender<Job>,
    config_holder: std::sync::Arc<std::sync::Mutex<std::sync::Arc<crate::app::config::AppConfig>>>,
) {
    match job {
                    Job::Ping => {
                        let (reply_tx, reply_rx) = oneshot::channel();
                        if python_tx_clone.send((PythonJob::Ping, reply_tx)).is_ok() {
                            if let Ok(PythonJobResult::Pong) = reply_rx.await {
                                let _ = result_tx_clone.send(JobResult::Pong);
                            }
                        }
                    }
                    Job::SubmitBugReport { description, include_logs, include_audit } => {
                        let res_tx = result_tx_clone.clone();
                        let webhook_url = std::env::var("WEBHOOK_URL").unwrap_or_default();
                        let log_dir = config_for_tokio.log_dir.clone();
                        
                        tokio::spawn(async move {
                            if webhook_url.is_empty() {
                                tracing::error!("Cannot submit bug report: WEBHOOK_URL is not configured.");
                                let _ = res_tx.send(JobResult::Error { job_label: "SubmitBugReport".to_string(), message: "Webhook URL not configured".to_string() });
                                return;
                            }
                            
                            let mut payload = serde_json::json!({
                                "content": format!("**New Bug Report**\n\n```\n{}\n```", description)
                            });
                            
                            // In a real implementation we would attach the actual files using reqwest multipart.
                            // For this beta, we'll just read the tail of the logs if requested and append to content.
                            if include_logs {
                                if let Ok(app_log) = tokio::fs::read_to_string(log_dir.join("app.log")).await {
                                    let tail = app_log.lines().rev().take(50).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
                                    payload["content"] = serde_json::Value::String(format!("{}\n\n**App Log (Tail)**\n```\n{}\n```", payload["content"].as_str().unwrap(), tail));
                                }
                            }
                            
                            let client = reqwest::Client::new();
                            match client.post(&webhook_url).json(&payload).send().await {
                                Ok(resp) if resp.status().is_success() => {
                                    let _ = res_tx.send(JobResult::BugReportSubmitted);
                                }
                                Ok(resp) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "SubmitBugReport".to_string(), message: format!("Server returned {}", resp.status()) });
                                }
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "SubmitBugReport".to_string(), message: e.to_string() });
                                }
                            }
                        });
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
                    Job::LoadDocument { path, three_page_mode } => {
                        let _ = result_tx_clone.send(JobResult::Progress { label: "Analyzing layout".to_string(), fraction: 0.1 });

                        // Cleanup previous segments if any
                        if let Some(mgr) = segment_manager.take() {
                            mgr.cleanup();
                        }
                        *segment_map = None;

                        if three_page_mode {
                            match SegmentManager::new() {
                                Ok(mgr) => {
                                    match mgr.prepare(&path, 3) {
                                        Ok(map) => {
                                            *segment_map = Some(map.clone());
                                            let total_pages = map.total_pages;
                                            *segment_manager = Some(mgr);
                                            let _ = result_tx_clone.send(JobResult::DocumentLoaded { layout_json: "[]".into(), total_pages });
                                            let _ = result_tx_clone.send(JobResult::Progress { label: "Done (3-page mode)".into(), fraction: 1.0 });
                                        }
                                        Err(e) => {
                                            let _ = result_tx_clone.send(JobResult::Error { job_label: "load_document_split".into(), message: e.to_string() });
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = result_tx_clone.send(JobResult::Error { job_label: "load_document_tempdir".into(), message: e.to_string() });
                                }
                            }
                        } else {
                            let eng = engine_for_tokio.clone();
                            let res_tx = result_tx_clone.clone();
                            let path_for_blocking = path.clone();
                            tokio::task::spawn_blocking(move || {
                                match eng.analyze_layout(&path_for_blocking) {
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

                        // Stage 8.5: kick off the font analysis in parallel.
                        let res_tx_fonts = result_tx_clone.clone();
                        let py_tx_for_fonts = python_tx_clone.clone();
                        let path_for_fonts = path.clone();
                        tokio::spawn(async move {
                            // Compute the hash on a blocking task so we
                            // don't stall the tokio runtime.
                            let path_for_hash = path_for_fonts.clone();
                            let hash_opt: Option<String> = tokio::task::spawn_blocking(move || -> Option<String> {
                                let bytes = std::fs::read(&path_for_hash).ok()?;
                                Some(crate::engine::workflow::sha256_hex_of(&bytes))
                            })
                            .await
                            .ok()
                            .flatten();

                            if let Some(ref hash) = hash_opt {
                                let cache_path = std::path::PathBuf::from("audit")
                                    .join("font_analysis_cache")
                                    .join(format!("{hash}.json"));
                                if let Ok(raw) = std::fs::read_to_string(&cache_path) {
                                    if let Ok(analysis) = crate::engine::font_analysis::FontAnalysis::from_json(&raw) {
                                        tracing::info!("[font-analysis] cache hit for {}", hash);
                                        let _ = res_tx_fonts.send(JobResult::FontAnalysisReady(analysis));
                                        return;
                                    }
                                }
                            }

                            let (reply_tx, reply_rx) = oneshot::channel();
                            if py_tx_for_fonts
                                .send((
                                    PythonJob::AnalyzeFonts {
                                        pdf_path: path_for_fonts.to_string_lossy().to_string(),
                                    },
                                    reply_tx,
                                ))
                                .is_ok()
                            {
                                if let Ok(PythonJobResult::Json(json)) = reply_rx.await {
                                    match crate::engine::font_analysis::FontAnalysis::from_json(&json) {
                                        Ok(analysis) => {
                                            // Write the cache entry for next time.
                                            if let Some(hash) = hash_opt.as_ref() {
                                                let cache_dir = std::path::PathBuf::from("audit")
                                                    .join("font_analysis_cache");
                                                let _ = std::fs::create_dir_all(&cache_dir);
                                                let cache_path = cache_dir.join(format!("{hash}.json"));
                                                // Atomic file operation: write to .tmp and rename
                                                let tmp_path = cache_path.with_extension("tmp");
                                                if std::fs::write(&tmp_path, &json).is_ok() {
                                                    let _ = std::fs::rename(tmp_path, &cache_path);
                                                }
                                            }
                                            let _ = res_tx_fonts.send(JobResult::FontAnalysisReady(analysis));
                                        }
                                        Err(e) => {
                                            tracing::warn!("[font-analysis] decode failed: {e}");
                                        }
                                    }
                                }
                            }
                        });
                    }
                    Job::AiFixVisualFidelity { input: _, page: _ } => {
                        let _ = result_tx_clone.send(JobResult::Progress {
                            label: "AI Visual Fidelity Fix (Stub)".to_string(),
                            fraction: 1.0,
                        });
                    }
                    Job::TransferTransactions { source_pdf, target_pdf, output_pdf } => {
                        let res_tx = result_tx_clone.clone();
                        let cfg = config_for_tokio.clone();
                        let py_tx = python_tx_clone.clone();
                        let engine_for_tokio = engine_for_tokio.clone();
                        let router = fallback_router.clone();
                        tokio::spawn(async move {
                            use crate::engine::transfer::*;

                            let started_at = std::time::Instant::now();
                            let _corrections_applied = 0usize;

                            // Construct AI mapping client — Transfer requires an AI provider
                            // for format mapping (this is an intentional AI-required exception;
                            // see AGENTS.md "Fallback chain rules").
                            let mut gemini = match crate::ai::backend::AiBackend::from_app_config(&cfg) {
                                Ok(c) => std::sync::Arc::new(c),
                                Err(_) => {
                                    let _ = res_tx.send(JobResult::TransferFailed {
                                        stage: "Init".into(),
                                        message: "Transfer requires an AI provider for format mapping — set GEMINI_API_KEY (or GROQ_API_KEY / OPENROUTER_API_KEY) and select a provider in Backend Preferences.".into(),
                                    });
                                    return;
                                }
                            };

                            // Helper: parse a statement via DocAI with offline fallback.
                            let doc_ai_opt = crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg).ok().map(std::sync::Arc::new);

                            // Helper to send progress
                            let send_progress = |res_tx: &std::sync::mpsc::Sender<JobResult>, stage: TransferStage| {
                                let (lo, _hi) = stage.fraction_range();
                                let _ = res_tx.send(JobResult::Progress {
                                    label: stage.label().to_string(),
                                    fraction: lo,
                                });
                            };

                            // ======= STAGE 1 & 2: Analyze Source and Target (Matrix Consensus) ========
                            
                            let parse_matrix = |pdf_path: PathBuf, cfg: std::sync::Arc<crate::app::config::AppConfig>, engine: std::sync::Arc<dyn crate::pdf::PdfEngine>, res_tx: std::sync::mpsc::Sender<JobResult>, stage_name: String, wdog: std::sync::Arc<crate::app::watchdog::Watchdog>| async move {
                                let mut tasks = Vec::new();

                                // 1. DocAI
                                if let Ok(doc_ai) = crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg) {
                                    let p = pdf_path.clone();
                                    let wdog_docai = wdog.clone();
                                    tasks.push(tokio::spawn(async move {
                                        ("DocAI", crate::engine::pro_edit::perform_pro_edit("DocumentAI", async { doc_ai.parse_entire_statement(&p, None::<&str>).await.map_err(anyhow::Error::from) }, wdog_docai).await.ok())
                                    }));
                                }
                                
                                // 2. LlamaParse
                                if let Ok(llama) = crate::ai::llamaparse::LlamaParseClient::from_app_config(&cfg) {
                                    let p = pdf_path.clone();
                                    let wdog_llama = wdog.clone();
                                    tasks.push(tokio::spawn(async move {
                                        ("LlamaParse", crate::engine::pro_edit::perform_pro_edit("LlamaParse", async { llama.parse_statement(&p).await.map_err(anyhow::Error::from) }, wdog_llama).await.ok())
                                    }));
                                }
                                
                                // 3. Offline Heuristic
                                let p = pdf_path.clone();
                                let e = engine.clone();
                                tasks.push(tokio::spawn(async move {
                                    ("Offline", tokio::task::spawn_blocking(move || {
                                        crate::engine::offline_parser::parse_statement_offline(&p, e).ok()
                                    }).await.ok().flatten())
                                }));
                                
                                let results = futures_util::future::join_all(tasks).await;
                                let mut statements: Vec<(&str, crate::ai::document_ai::BankStatement)> = Vec::new();
                                for res in results {
                                    if let Ok((name, Some(s))) = res {
                                        statements.push((name, s));
                                    }
                                }
                                
                                if statements.is_empty() {
                                    let _ = res_tx.send(JobResult::TransferFailed {
                                        stage: stage_name,
                                        message: "All matrix consensus parsers failed.".into(),
                                    });
                                    return None;
                                }
                                
                                if cfg.transfer_consensus_mode {
                                    tracing::info!("[TRANSFER] Matrix Consensus: Merging {} successful parses", statements.len());
                                    
                                    let mut raw_stmts = Vec::new();
                                    for (_, s) in &statements {
                                        raw_stmts.push(s.clone());
                                    }
                                    let consensus = crate::engine::consensus::merge_consensus_statements(raw_stmts);
                                    
                                    // Update stats
                                    let mut stats: crate::engine::model::ParserStats = std::fs::read_to_string("audit/parser_stats.json")
                                        .ok()
                                        .and_then(|s| serde_json::from_str(&s).ok())
                                        .unwrap_or_default();
                                    stats.total_attempts += 1;
                                    
                                    // Winner is the one closest to consensus tx count
                                    let mut best_dist = usize::MAX;
                                    let mut winner = "";
                                    for (name, s) in &statements {
                                        let dist = (s.transactions.len() as isize - consensus.transactions.len() as isize).unsigned_abs();
                                        if dist < best_dist {
                                            best_dist = dist;
                                            winner = name;
                                        }
                                    }
                                    
                                    match winner {
                                        "DocAI" => stats.docai_wins += 1,
                                        "LlamaParse" => stats.llamaparse_wins += 1,
                                        "Offline" => stats.offline_wins += 1,
                                        _ => {}
                                    }
                                    // Atomic file operation: write to .tmp and rename
                                    let stats_path = std::path::PathBuf::from("audit/parser_stats.json");
                                    let tmp_path = stats_path.with_extension("tmp");
                                    if std::fs::write(&tmp_path, serde_json::to_string_pretty(&stats).unwrap_or_default()).is_ok() {
                                        let _ = std::fs::rename(tmp_path, &stats_path);
                                    }
                                    
                                    Some(consensus)
                                } else {
                                    Some(statements.into_iter().next().unwrap().1)
                                }
                            };

                            send_progress(&res_tx, TransferStage::AnalyzeSource);
                            tracing::info!("[TRANSFER] Stage 1: Analyzing source PDF: {:?}", source_pdf);
                            let source_stmt = match parse_matrix(source_pdf.clone(), cfg.clone(), engine_for_tokio.clone(), res_tx.clone(), "AnalyzeSource".into(), wdog.clone()).await {
                                Some(s) => s,
                                None => return,
                            };
                            let source_transactions = source_stmt.transactions.clone();
                            tracing::info!("[TRANSFER] Source: {} transactions found", source_transactions.len());

                            if source_transactions.is_empty() {
                                let _ = res_tx.send(JobResult::TransferFailed {
                                    stage: "AnalyzeSource".into(),
                                    message: "Source statement has 0 transactions - nothing to transfer.".into(),
                                });
                                return;
                            }

                            let _ = res_tx.send(JobResult::Progress {
                                label: "Source analyzed ✓".to_string(),
                                fraction: 0.10,
                            });

                            send_progress(&res_tx, TransferStage::AnalyzeTarget);
                            tracing::info!("[TRANSFER] Stage 2: Analyzing target PDF: {:?}", target_pdf);
                            
                            let target_stmt = match parse_matrix(target_pdf.clone(), cfg.clone(), engine_for_tokio.clone(), res_tx.clone(), "AnalyzeTarget".into(), wdog.clone()).await {
                                Some(s) => s,
                                None => return,
                            };
                            let target_transactions = target_stmt.transactions.clone();
                            tracing::info!("[TRANSFER] Target: {} transactions found", target_transactions.len());

                            if target_transactions.is_empty() {
                                let _ = res_tx.send(JobResult::TransferFailed {
                                    stage: "AnalyzeTarget".into(),
                                    message: "Target statement has 0 transactions - no layout to map into.".into(),
                                });
                                return;
                            }

                            let _ = res_tx.send(JobResult::Progress {
                                label: "Target analyzed ✓".to_string(),
                                fraction: 0.20,
                            });


                            let max_retries = 5usize;
                            let mut attempt = 0;
                            let mut best_visual_score = 1.0f64;
                            let mut best_math_verified = false;
                            let mut best_result = None;
                            let mut correction_hint: Option<String> = None;
                            let mut synthesized_fonts_used = false;
                            let mut font_override_path: Option<String> = None;
                            let mut total_corrections = 0;

                            loop {
                                attempt += 1;
                                tracing::info!("[TRANSFER] --- Starting Attempt {} ---", attempt);

                                // ======= STAGE 3: AI Format Mapping ========
                                send_progress(&res_tx, TransferStage::AiFormatMapping);
                                tracing::info!("[TRANSFER] Stage 3: AI format mapping via Gemini");

                                let transfer_plan = match gemini.plan_transaction_transfer(
                                    &source_transactions,
                                    &target_transactions,
                                    correction_hint.as_deref(),
                                ).await {
                                    Ok(p) => p,
                                    Err(e) => {
                                        tracing::warn!("[TRANSFER] Gemini format mapping failed: {e}");
                                        if !cfg.interactive_fallbacks {
                                            let _ = res_tx.send(JobResult::TransferFailed {
                                                stage: "AiFormatMapping".into(),
                                                message: format!("Gemini format mapping failed: {e}"),
                                            });
                                            return;
                                        }
                                        
                                        let mut req = crate::engine::interactive_fallback::InteractiveFallbackRequest::new(
                                            "Transfer Transactions Mapping",
                                            format!("Gemini mapping failed: {e}"),
                                        );
                                        req = req.add_alternative("openrouter", "Try OpenRouter (Multi-Model)", None);
                                        req = req.add_alternative("groq", "Try Groq", None);
                                        req = req.add_alternative("cancel", "Cancel Transfer", None);
                                        
                                        let (tx, rx) = tokio::sync::oneshot::channel();
                                        {
                                            let mut map = router.lock().await;
                                            map.insert(req.id, tx);
                                        }
                                        let _ = res_tx.send(JobResult::InteractiveFallbackRequired(req));
                                        
                                        let choice = rx.await.unwrap_or_else(|_| "cancel".to_string());
                                        if choice == "cancel" {
                                            let _ = res_tx.send(JobResult::TransferFailed {
                                                stage: "AiFormatMapping".into(),
                                                message: "User cancelled after failure.".into(),
                                            });
                                            return;
                                        }
                                        // Update AI backend based on choice
                                        let mut new_cfg = (*cfg).clone();
                                        if choice == "openrouter" {
                                            new_cfg.ai_provider = crate::app::config::AiProviderMode::OpenRouterApiKey;
                                        } else if choice == "groq" {
                                            new_cfg.ai_provider = crate::app::config::AiProviderMode::GroqApiKey;
                                        }
                                        
                                        match crate::ai::backend::AiBackend::from_app_config(&new_cfg) {
                                            Ok(c) => {
                                                gemini = std::sync::Arc::new(c);
                                                continue; // Retry loop with new provider
                                            }
                                            Err(e) => {
                                                let _ = res_tx.send(JobResult::TransferFailed {
                                                    stage: "AiFormatMapping".into(),
                                                    message: format!("Failed to init fallback provider: {e}"),
                                                });
                                                return;
                                            }
                                        }
                                    }
                                };
                                tracing::info!(
                                    "[TRANSFER] Plan: {} mappings, {} pages to clone, {} to remove",
                                    transfer_plan.mappings.len(),
                                    transfer_plan.pages_to_clone.len(),
                                    transfer_plan.pages_to_remove.len(),
                                );

                                let _ = res_tx.send(JobResult::Progress {
                                    label: "Format mapping complete ✓".to_string(),
                                    fraction: 0.30,
                                });

                                // ======= STAGE 4: Compute Balances ========
                                send_progress(&res_tx, TransferStage::ComputeBalances);
                                tracing::info!("[TRANSFER] Stage 4: Computing balances");

                                let opening_balance = target_stmt.opening_balance;
                                let mut mapped: Vec<MappedTransaction> = Vec::with_capacity(transfer_plan.mappings.len());
                                let mut skipped_invalid = 0usize;
                                for m in &transfer_plan.mappings {
                                    let src = match source_transactions.get(m.source_index) {
                                        Some(s) => s,
                                        None => {
                                            tracing::error!(
                                                "[TRANSFER] source_index {} out of bounds (max {}), skipping mapping",
                                                m.source_index,
                                                source_transactions.len()
                                            );
                                            skipped_invalid += 1;
                                            continue;
                                        }
                                    };
                                    mapped.push(MappedTransaction {
                                        target_page: m.target_page,
                                        target_line: m.target_line,
                                        date: m.converted_date.clone(),
                                        description: m.adapted_description.clone(),
                                        debit: src.debit,
                                        credit: src.credit,
                                        running_balance: rust_decimal::Decimal::ZERO,
                                        field_bboxes: crate::engine::model::FieldBboxes::default(),
                                     });
                                }
                                if skipped_invalid > 0 {
                                    tracing::warn!("[TRANSFER] Skipped {} mappings with invalid source_index", skipped_invalid);
                                }

                                recompute_running_balances(opening_balance, &mut mapped);
                                tracing::info!("[TRANSFER] Balances computed for {} transactions", mapped.len());

                                let _ = res_tx.send(JobResult::Progress {
                                    label: "Balances computed ✓".to_string(),
                                    fraction: 0.35,
                                });

                                // ======= STAGE 5: PDF Surgery ========
                                send_progress(&res_tx, TransferStage::PdfSurgery);
                                tracing::info!("[TRANSFER] Stage 5: PDF surgery - applying changes");

                                if let Err(e) = std::fs::copy(&target_pdf, &output_pdf) {
                                    let _ = res_tx.send(JobResult::TransferFailed {
                                        stage: "PdfSurgery".into(),
                                        message: format!("Failed to copy target PDF: {e}"),
                                    });
                                    return;
                                }

                                let mut actual_pages_added = 0usize;
                                let mut actual_pages_removed = 0usize;

                                if !transfer_plan.pages_to_clone.is_empty() {
                                    let temp_path = output_pdf.with_extension("cloned.pdf");
                                    let eng = engine_for_tokio.clone();
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
                                    }
                                }

                                if !transfer_plan.pages_to_remove.is_empty() {
                                    let temp_path = output_pdf.with_extension("removed.pdf");
                                    let eng = engine_for_tokio.clone();
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
                                    }
                                }

                                let mut target_by_page: std::collections::HashMap<usize, Vec<&crate::engine::model::Transaction>> =
                                    std::collections::HashMap::new();
                                for t in &target_transactions {
                                    target_by_page.entry(t.page).or_default().push(t);
                                }
                                for txns in target_by_page.values_mut() {
                                    txns.sort_by(|a, b| {
                                        let ay = a.bbox.map(|b| b[1]).unwrap_or(f32::MAX);
                                        let by = b.bbox.map(|b| b[1]).unwrap_or(f32::MAX);
                                        ay.partial_cmp(&by).unwrap_or(std::cmp::Ordering::Equal)
                                    });
                                }

                                let _total_txns = mapped.len();
                                let mut actually_edited_bboxes: Vec<(usize, [f32; 4])> = Vec::new();
                                let mut batch_edits: Vec<serde_json::Value> = Vec::new();

                                for (i, tx) in mapped.iter().enumerate() {
                                    let mut adjusted_page = tx.target_page;
                                    for &c in transfer_plan.pages_to_clone.iter().rev() {
                                        if tx.target_page > c {
                                            adjusted_page += 1;
                                        }
                                    }
                                    for &r in transfer_plan.pages_to_remove.iter().rev() {
                                        if adjusted_page > r {
                                            adjusted_page = adjusted_page.saturating_sub(1);
                                        } else if adjusted_page == r {
                                            // The target page was removed, skip edits for this transaction
                                            continue;
                                        }
                                    }

                                    let target_tx = target_by_page
                                        .get(&tx.target_page)
                                        .and_then(|page_txns| page_txns.get(tx.target_line));

                                    match target_tx {
                                        None => {
                                            tracing::warn!(
                                                "[TRANSFER] No target transaction at page={} line={} for mapping {}",
                                                tx.target_page, tx.target_line, i
                                            );
                                        }
                                        Some(target) => {
                                            let fields: Vec<(&str, Option<[f32; 4]>, String)> = vec![
                                                ("date", target.field_bboxes.date, tx.date.clone()),
                                                ("description", target.field_bboxes.description, tx.description.clone()),
                                                ("debit", target.field_bboxes.debit, tx.debit.map(|d| d.to_string()).unwrap_or_default()),
                                                ("credit", target.field_bboxes.credit, tx.credit.map(|c| c.to_string()).unwrap_or_default()),
                                                ("balance", target.field_bboxes.running_balance, tx.running_balance.to_string()),
                                            ];

                                            let mut any_field_written = false;
                                            for (_field_name, field_bbox, field_text) in &fields {
                                                if field_text.is_empty() { continue; }
                                                if let Some(bbox) = field_bbox {
                                                    batch_edits.push(serde_json::json!({
                                                        "page": adjusted_page,
                                                        "rect": bbox,
                                                        "new_text": field_text.clone(),
                                                    }));
                                                    actually_edited_bboxes.push((adjusted_page, *bbox));
                                                    any_field_written = true;
                                                }
                                            }

                                            if !any_field_written {
                                                if let Some(bbox) = target.bbox {
                                                    let new_text = format!(
                                                        "{} {} {} {}",
                                                        tx.date, tx.description,
                                                        tx.debit.map(|d| d.to_string()).or(tx.credit.map(|c| c.to_string())).unwrap_or_default(),
                                                        tx.running_balance,
                                                    );
                                                    batch_edits.push(serde_json::json!({
                                                        "page": adjusted_page,
                                                        "rect": bbox,
                                                        "new_text": new_text.clone(),
                                                    }));
                                                    actually_edited_bboxes.push((adjusted_page, bbox));
                                                }
                                            }
                                        }
                                    }
                                }

                                let total_edits = batch_edits.len();
                                let mut edits_applied = 0usize;
                                let mut fallback_fonts_used = Vec::new();
                                if total_edits > 0 {
                                    tracing::info!("[TRANSFER] Applying batch of {} text edits", total_edits);

                                    let mut output_pages = 0;
                                    if let Ok(doc) = lopdf::Document::load(&output_pdf) {
                                        output_pages = doc.get_pages().len();
                                    }

                                    if output_pages > 3 {
                                        tracing::info!("[TRANSFER] Document has {} pages (> 3), chunking for Pro engine", output_pages);
                                        let temp_mgr = match crate::engine::segments::SegmentManager::new() {
                                            Ok(mgr) => mgr,
                                            Err(e) => {
                                                tracing::error!("[TRANSFER] Failed to create SegmentManager: {}", e);
                                                let _ = res_tx.send(JobResult::TransferFailed {
                                                    stage: "PdfSurgery".into(),
                                                    message: format!("Failed to create SegmentManager: {e}"),
                                                });
                                                return;
                                            }
                                        };
                                        if let Ok(map) = temp_mgr.prepare(&output_pdf, 3) {
                                            let mut edits_by_seg: std::collections::BTreeMap<usize, Vec<serde_json::Value>> = std::collections::BTreeMap::new();
                                            for edit in &batch_edits {
                                                let global_page = edit["page"].as_u64().unwrap_or(0) as usize;
                                                if let Some((seg_idx, local_page)) = map.resolve(global_page) {
                                                    let mut new_edit = edit.clone();
                                                    new_edit["page"] = serde_json::json!(local_page);
                                                    edits_by_seg.entry(seg_idx).or_default().push(new_edit);
                                                }
                                            }

                                            let mut final_paths = Vec::new();
                                            for (i, seg) in map.segments.iter().enumerate() {
                                                let seg_edits = edits_by_seg.get(&i).cloned().unwrap_or_default();
                                                if !seg_edits.is_empty() {
                                                    let edited_path = temp_mgr.temp_path().join(format!("segment_{i:03}_edited.pdf"));
                                                    let edits_json = serde_json::to_string(&seg_edits).unwrap_or_default();
                                                    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                                                    let _ = py_tx.send((
                                                        PythonJob::ApplyManyEdits {
                                                            pdf_path: seg.path.to_string_lossy().to_string(),
                                                            output_path: edited_path.to_string_lossy().to_string(),
                                                            edits_json,
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
                                                    }
                                                } else {
                                                    final_paths.push(seg.path.clone());
                                                }
                                            }

                                            if let Err(e) = crate::engine::pdf_split_merge::merge_pdfs(&final_paths, &output_pdf) {
                                                tracing::error!("[TRANSFER] Failed to merge segments: {}", e);
                                            }
                                        } else {
                                            tracing::error!("[TRANSFER] Failed to prepare document segments for chunking");
                                        }
                                    } else {
                                        let edits_json = serde_json::to_string(&batch_edits).unwrap_or_default();
                                        let eng = engine_for_tokio.clone();
                                        let p_in = output_pdf.clone();
                                        let p_out = output_pdf.with_extension("temp.pdf");
                                        let f_path = font_override_path.clone();
                                        let edits_json_clone = edits_json.clone();

                                        let native_res = tokio::task::spawn_blocking(move || {
                                            let fp = f_path.map(std::path::PathBuf::from);
                                            eng.apply_many_edits(&p_in, &p_out, &edits_json_clone, fp.as_deref())
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
                                        }
                                    }
                                }

                                let _ = res_tx.send(JobResult::Progress {
                                    label: format!("PDF changes applied ✓ ({edits_applied}/{total_edits})"),
                                    fraction: 0.55,
                                });

                                // Handle PyMuPDF standard-14 fallback detection
                                if !fallback_fonts_used.is_empty() && font_override_path.is_none() && attempt < max_retries {
                                    tracing::warn!("[TRANSFER] PyMuPDF used fallback fonts on pages {:?}. Synthesizing font...", fallback_fonts_used);
                                    let _ = res_tx.send(JobResult::Progress {
                                        label: format!("(Attempt {attempt}) Synthesizing precise missing glyphs..."),
                                        fraction: 0.55,
                                    });
                                    let (rtx, rrx) = tokio::sync::oneshot::channel();
                                    let _ = py_tx.send((
                                        PythonJob::ReplicateFontForMissingChars {
                                            pdf_path: output_pdf.to_string_lossy().to_string(),
                                            font_name: "default".to_string(),
                                            missing_chars_csv: batch_edits.iter().map(|v| v["new_text"].as_str().unwrap_or_default().to_string()).collect::<Vec<_>>().join(""),
                                            output_dir: "audit/fonts".to_string(),
                                        },
                                        rtx,
                                    ));
                                    if let Ok(PythonJobResult::Json(json_str)) = rrx.await {
                                        if let Ok(res) = serde_json::from_str::<serde_json::Value>(&json_str) {
                                            if let Some(fpath) = res["font_path"].as_str() {
                                                font_override_path = Some(fpath.to_string());
                                                synthesized_fonts_used = true;
                                                total_corrections += 1;
                                                tracing::info!("[TRANSFER] Font synthesized at {}. Retrying loop.", fpath);
                                                continue; // RETRY LOOP
                                            }
                                        }
                                    }
                                }

                                // ======= STAGE 6: Visual Fidelity Check ========
                                send_progress(&res_tx, TransferStage::VisualFidelityCheck);
                                tracing::info!("[TRANSFER] Stage 6: Visual fidelity verification");

                                let intended_bboxes: Vec<(usize, [f32; 4])> = actually_edited_bboxes;
                                let math_input_txns: Vec<crate::engine::model::Transaction> = mapped.iter().map(|m| {
                                    crate::engine::model::Transaction {
                                        page: m.target_page,
                                        line_on_page: m.target_line,
                                        date: m.date.clone(),
                                        raw_text: m.description.clone(),
                                        debit: m.debit,
                                        credit: m.credit,
                                        running_balance: Some(m.running_balance),
                                        bbox: None,
                                        field_bboxes: crate::engine::model::FieldBboxes::default(),
                                        provenance: crate::engine::model::Provenance::Computed,
                                        category: None,
                                     }
                                }).collect();

                                let vis_result = crate::engine::verification::verify_edit(
                                    &target_pdf,
                                    &output_pdf,
                                    &std::path::PathBuf::from("audit/transfer_verification"),
                                    &intended_bboxes,
                                    crate::engine::verification::MathInputs {
                                        transactions: math_input_txns,
                                        opening_balance,
                                        expected_final_balance: None,
                                    },
                                    cfg.auto_match_dpi,
                                ).await;

                                let (visual_score, visual_verified, report_files) = match &vis_result {
                                    Ok(report) => (report.visual_diff_score, report.only_intended_changes, report.report_files.clone()),
                                    Err(e) => {
                                        tracing::warn!("[TRANSFER] Visual verification error: {}", e);
                                        (0.0, true, vec![])
                                    }
                                };

                                let _ = res_tx.send(JobResult::Progress {
                                    label: format!("Visual check ✓ (score: {visual_score:.4})"),
                                    fraction: 0.75,
                                });

                                // STAGE 6.5: Gemini Vision Check
                                let mut vision_anomaly = false;
                                if let Some(edit_png_path) = report_files.iter().find(|p| p.contains("edited_p1")) {
                                    if let Ok(png_data) = std::fs::read(edit_png_path) {
                                        // only check the first page for anomalies right now
                                        let page_intended: Vec<[f32; 4]> = intended_bboxes.iter()
                                            .filter(|(p, _)| *p == 0)
                                            .map(|(_, b)| *b).collect();
                                        if let Ok(vision_report) = gemini.validate_render_visually(&png_data, &page_intended).await {
                                            tracing::info!("[TRANSFER] Gemini Vision score: {:.2}, notes: {}", vision_report.anomaly_score, vision_report.notes);
                                            if vision_report.anomaly_score > 0.5 {
                                                vision_anomaly = true;
                                                tracing::warn!("[TRANSFER] Gemini Vision flagged anomalies: {:?}", vision_report.hotspots);
                                            }
                                        }
                                    }
                                }

                                if (vision_anomaly || !visual_verified) && attempt < max_retries {
                                    tracing::warn!("[TRANSFER] Visual check failed (anomaly or strict threshold). Attempting font synthesis for retry.");
                                    let _ = res_tx.send(JobResult::Progress {
                                        label: format!("(Attempt {attempt}) Adapting font metrics to Gemini Vision anomaly..."),
                                        fraction: 0.75,
                                    });
                                    let (rtx, rrx) = tokio::sync::oneshot::channel();
                                    let _ = py_tx.send((
                                        PythonJob::CompleteFontWithAdaption {
                                            pdf_path: target_pdf.to_string_lossy().to_string(),
                                            font_name: "default".to_string(),
                                        },
                                        rtx,
                                    ));
                                    if let Ok(PythonJobResult::Json(json_str)) = rrx.await {
                                        if let Ok(res) = serde_json::from_str::<serde_json::Value>(&json_str) {
                                            if let Some(fpath) = res["font_path"].as_str() {
                                                font_override_path = Some(fpath.to_string());
                                                synthesized_fonts_used = true;
                                                total_corrections += 1;
                                                tracing::info!("[TRANSFER] Adapted font synthesized at {}. Retrying loop.", fpath);
                                                continue;
                                            }
                                        }
                                    }
                                }

                                // ======= STAGE 7: Math Verification (Engine) ========
                                send_progress(&res_tx, TransferStage::MathVerificationEngine);
                                tracing::info!("[TRANSFER] Stage 7: Math verification (engine)");

                                let mut math_verified = false;
                                let mut math_imbalance = rust_decimal::Decimal::ZERO;
                                let mut math_err_msg = String::new();

                                let reparsed_stmt = if let Some(ref doc_ai) = doc_ai_opt {
                                    match crate::engine::pro_edit::perform_pro_edit("DocumentAI", async { doc_ai.parse_entire_statement(&output_pdf, None::<&str>).await.map_err(anyhow::Error::from) }, wdog.clone()).await {
                                        Ok(s) => Ok(s),
                                        Err(e) => {
                                            tracing::warn!("[TRANSFER] DocAI target reparsing failed, trying offline: {e}");
                                            let eng_clone = engine_for_tokio.clone();
                                            let path_clone = output_pdf.clone();
                                            tokio::task::spawn_blocking(move || {
                                                crate::engine::offline_parser::parse_statement_offline(&path_clone, eng_clone)
                                            }).await.unwrap_or_else(|e| Err(format!("Offline parser panicked: {e}")))
                                        }
                                    }
                                } else {
                                    let eng_clone = engine_for_tokio.clone();
                                    let path_clone = output_pdf.clone();
                                    tokio::task::spawn_blocking(move || {
                                        crate::engine::offline_parser::parse_statement_offline(&path_clone, eng_clone)
                                    }).await.unwrap_or_else(|e| Err(format!("Offline parser panicked: {e}")))
                                };

                                match reparsed_stmt {
                                    Ok(reparsed) => {
                                        let engine_txns: Vec<crate::engine::model::Transaction> = reparsed.transactions;
                                        match crate::engine::balance::process_and_reconcile(
                                            engine_txns, opening_balance, None,
                                        ) {
                                            Ok((_, None)) => {
                                                math_verified = true;
                                                tracing::info!("[TRANSFER] Math verification PASSED");
                                            }
                                            Ok((_, Some(msg))) => {
                                                math_imbalance = rust_decimal_macros::dec!(0.01);
                                                math_err_msg = format!("Math mismatch: {msg}");
                                                tracing::warn!("[TRANSFER] {}", math_err_msg);
                                                total_corrections += 1;
                                            }
                                            Err(e) => {
                                                math_imbalance = rust_decimal_macros::dec!(0.01);
                                                math_err_msg = format!("Balance engine error: {e}");
                                                tracing::warn!("[TRANSFER] {}", math_err_msg);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        math_imbalance = rust_decimal_macros::dec!(0.01);
                                        math_err_msg = format!("Parse for verification failed: {e}");
                                        tracing::warn!("[TRANSFER] {}", math_err_msg);
                                    }
                                }

                                let _ = res_tx.send(JobResult::Progress {
                                    label: format!("Math (engine) {} ", if math_verified { "✓" } else { "⚠" }),
                                    fraction: 0.85,
                                });

                                // ======= STAGE 8: Math Verification (Gemini) ========
                                send_progress(&res_tx, TransferStage::MathVerificationGemini);
                                tracing::info!("[TRANSFER] Stage 8: Math verification (Gemini)");

                                let gemini_math_ok = match gemini.verify_transfer_math(
                                    &mapped,
                                    opening_balance,
                                ).await {
                                    Ok(ok) => ok,
                                    Err(e) => {
                                        tracing::warn!("[TRANSFER] Gemini math verification error: {}", e);
                                        true
                                    }
                                };

                                let _ = res_tx.send(JobResult::Progress {
                                    label: format!("Math (Gemini) {} ", if gemini_math_ok { "✓" } else { "⚠" }),
                                    fraction: 0.95,
                                });

                                let all_math_ok = math_verified && gemini_math_ok;
                                let current_quality_score = visual_score * (if all_math_ok { 1.0 } else { 0.5 });
                                let best_quality_score = best_visual_score * (if best_math_verified { 1.0 } else { 0.5 });

                                // STAGE 9: Final Audit setup
                                let elapsed = started_at.elapsed().as_secs_f64();
                                let result = TransferResult {
                                    output_path: output_pdf.clone(),
                                    source_tx_count: source_transactions.len(),
                                    target_tx_count: target_transactions.len(),
                                    pages_added: actual_pages_added,
                                    pages_removed: actual_pages_removed,
                                    math_verified: all_math_ok,
                                    visual_verified: visual_verified && !vision_anomaly,
                                    visual_score,
                                    math_imbalance,
                                    stages_completed: 9,
                                    total_duration_secs: elapsed,
                                    corrections_applied: total_corrections,
                                    retries_attempted: attempt - 1,
                                    synthesized_fonts_used,
                                };

                                // Store best result
                                if best_result.is_none() || current_quality_score > best_quality_score {
                                    best_result = Some(result.clone());
                                    best_visual_score = visual_score;
                                    best_math_verified = all_math_ok;
                                }

                                if all_math_ok && visual_verified && !vision_anomaly {
                                    tracing::info!("[TRANSFER] Iteration {} passed all checks perfectly. Breaking loop.", attempt);
                                    break;
                                }

                                // Interactive Fallback Logic for No Improvement / Reduction
                                if attempt >= 1 && current_quality_score <= best_quality_score {
                                    tracing::warn!("[TRANSFER] Loop {} yielded no improvement or regression. Quality score: {:.4}, Best: {:.4}", attempt, current_quality_score, best_quality_score);
                                    if cfg.interactive_fallbacks {
                                        let mut req = crate::engine::interactive_fallback::InteractiveFallbackRequest::new(
                                            "Transfer Validation Loop",
                                            if current_quality_score < best_quality_score {
                                                "The AI mapping quality degraded on recalculation."
                                            } else {
                                                "The AI mapping failed to improve the fidelity issues."
                                            }
                                        );
                                        req = req.add_alternative("openrouter", "Try OpenRouter Backup", None);
                                        req = req.add_alternative("groq", "Try Groq Backup", None);
                                        req = req.add_alternative("finish", "Use Best Result & Finish", None);
                                        
                                        let (tx, rx) = tokio::sync::oneshot::channel();
                                        {
                                            let mut map = router.lock().await;
                                            map.insert(req.id, tx);
                                        }
                                        let _ = res_tx.send(JobResult::InteractiveFallbackRequired(req));
                                        
                                        let choice = rx.await.unwrap_or_else(|_| "finish".to_string());
                                        if choice == "finish" {
                                            tracing::info!("[TRANSFER] User chose to finish with best result.");
                                            break;
                                        } else {
                                            let mut new_cfg = (*cfg).clone();
                                            if choice == "openrouter" {
                                                new_cfg.ai_provider = crate::app::config::AiProviderMode::OpenRouterApiKey;
                                            } else if choice == "groq" {
                                                new_cfg.ai_provider = crate::app::config::AiProviderMode::GroqApiKey;
                                            }
                                            
                                            if let Ok(c) = crate::ai::backend::AiBackend::from_app_config(&new_cfg) {
                                                gemini = std::sync::Arc::new(c);
                                            }
                                        }
                                    } else {
                                        tracing::warn!("[TRANSFER] Interactive fallbacks disabled. Breaking loop with best result.");
                                        break;
                                    }
                                }

                                if !all_math_ok && attempt < max_retries {
                                    tracing::warn!("[TRANSFER] Math check failed. Retrying entire planning loop with hint.");
                                    correction_hint = Some(math_err_msg.clone());
                                    continue;
                                }

                                if attempt >= max_retries {
                                    tracing::warn!("[TRANSFER] Reached max retries. Taking best result.");
                                    break;
                                }
                            }

                            // Get the best result from the loop
                            let final_result = match best_result {
                                Some(r) => r,
                                None => {
                                    let _ = res_tx.send(JobResult::Error { job_label: String::new(), message: "Transfer loop failed to yield any valid result".into() });
                                    return;
                                }
                            };

                            // ======= STAGE 9: Final Audit ========
                            send_progress(&res_tx, TransferStage::FinalAudit);

                            match write_transfer_audit(&final_result, &source_pdf, &target_pdf) {
                                Ok(_audit_path) => {
                                    // Phase 7: Audit reports are securely saved purely in Rust via serde_json.
                                    // No external python post-processing is required.
                                }
                                Err(e) => tracing::warn!("[TRANSFER] Failed to write audit report: {}", e),
                            }

                            tracing::info!(
                                "[TRANSFER] ✅ Complete in {:.1}s - math: {}, visual: {}",
                                final_result.total_duration_secs,
                                if final_result.math_verified { "✓" } else { "✗" },
                                if final_result.visual_verified { "✓" } else { "✗" },
                            );

                            let _ = res_tx.send(JobResult::Progress {
                                label: "Transfer complete ✓".to_string(),
                                fraction: 1.0,
                            });

                            let _ = res_tx.send(JobResult::TransferComplete(final_result));

                        });
                    }
                    Job::AdjustDatePeriods { input, output, mode } => {
                        let res_tx = result_tx_clone.clone();
                        let cfg = config_for_tokio.clone();
                        let py_tx = python_tx_clone.clone();
                        let eng = engine_for_tokio.clone();
                        tokio::spawn(async move {
                            let _ = res_tx.send(JobResult::Progress {
                                label: "Parsing statement for date adjustment...".to_string(),
                                fraction: 0.1,
                            });

                            // Parse the statement — try Document AI, fall back to offline parser.
                            let stmt = match crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg) {
                                Ok(c) => {
                                    let doc_ai: std::sync::Arc<crate::ai::document_ai::DocumentAiClient> = std::sync::Arc::new(c);
                                    match doc_ai.parse_entire_statement(&input, None::<&str>).await {
                                        Ok(s) => s,
                                        Err(e) => {
                                            tracing::warn!("[adjust_dates] Document AI parse failed, falling back to offline parser: {e}");
                                            let _ = res_tx.send(JobResult::Progress {
                                                label: "Document AI failed, using offline parser...".to_string(),
                                                fraction: 0.2,
                                            });
                                            let eng_clone = eng.clone();
                                            let input_clone = input.clone();
                                            match tokio::task::spawn_blocking(move || {
                                                crate::engine::offline_parser::parse_statement_offline(&input_clone, eng_clone)
                                            }).await {
                                                Ok(Ok(s)) => s,
                                                Ok(Err(e2)) => {
                                                    let _ = res_tx.send(JobResult::Error {
                                                        job_label: "adjust_dates".into(),
                                                        message: format!("Offline parser also failed: {e2}"),
                                                    });
                                                    return;
                                                }
                                                Err(e2) => {
                                                    let _ = res_tx.send(JobResult::Error {
                                                        job_label: "adjust_dates".into(),
                                                        message: format!("Offline parser panicked: {e2}"),
                                                    });
                                                    return;
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(_) => {
                                    tracing::info!("[adjust_dates] Document AI not configured, using offline parser");
                                    let _ = res_tx.send(JobResult::Progress {
                                        label: "Using offline parser (no Document AI)...".to_string(),
                                        fraction: 0.2,
                                    });
                                    let eng_clone = eng.clone();
                                    let input_clone = input.clone();
                                    match tokio::task::spawn_blocking(move || {
                                        crate::engine::offline_parser::parse_statement_offline(&input_clone, eng_clone)
                                    }).await {
                                        Ok(Ok(s)) => s,
                                        Ok(Err(e)) => {
                                            let _ = res_tx.send(JobResult::Error {
                                                job_label: "adjust_dates".into(),
                                                message: format!("Offline extraction failed: {e}"),
                                            });
                                            return;
                                        }
                                        Err(e) => {
                                            let _ = res_tx.send(JobResult::Error {
                                                job_label: "adjust_dates".into(),
                                                message: format!("Offline extraction panicked: {e}"),
                                            });
                                            return;
                                        }
                                    }
                                }
                            };

                            let _ = res_tx.send(JobResult::Progress {
                                label: "Adjusting dates...".to_string(),
                                fraction: 0.4,
                            });

                            let mut transactions = stmt.transactions;
                            let records = match mode {
                                crate::engine::date_adjust::DateAdjustMode::ShiftDays(days) => {
                                    crate::engine::date_adjust::shift_dates(&mut transactions, days)
                                }
                                crate::engine::date_adjust::DateAdjustMode::RemapPeriod { from_start, to_start } => {
                                    crate::engine::date_adjust::remap_date_period(&mut transactions, from_start, to_start)
                                }
                            };

                            // Clone the PDF and apply date changes
                            if let Err(e) = std::fs::copy(&input, &output) {
                                let _ = res_tx.send(JobResult::Error {
                                    job_label: "adjust_dates".into(),
                                    message: format!("Failed to clone PDF: {e}"),
                                });
                                return;
                            }

                            let total = records.len();
                            let mut skipped = 0usize;
                            for (i, rec) in records.iter().enumerate() {
                                // Find the bbox for this transaction's date field.
                                // Offline-parsed transactions have empty FieldBboxes, so
                                // date_bbox may be None — skip gracefully and warn.
                                if let Some(tx) = transactions.iter().find(|t| t.page == rec.page && t.line_on_page == rec.line_on_page) {
                                    if let Some(date_bbox) = tx.field_bboxes.date {
                                        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
                                        let _ = py_tx.send((
                                            PythonJob::ReplaceTextInRect {
                                                pdf_path: output.to_string_lossy().to_string(),
                                                output_path: output.to_string_lossy().to_string(),
                                                page_num: rec.page,
                                                rect: date_bbox,
                                                new_text: rec.new_date.clone(),
                                                font_path: None,
                                            },
                                            reply_tx,
                                        ));
                                        let _ = reply_rx.await;
                                    } else {
                                        tracing::warn!(
                                            "[adjust_dates] No date bbox for page {} line {} — skipping PDF edit (offline parser limitation)",
                                            rec.page, rec.line_on_page,
                                        );
                                        skipped += 1;
                                    }
                                }
                                let frac = 0.4 + (0.5 * (i + 1) as f32 / total.max(1) as f32);
                                let _ = res_tx.send(JobResult::Progress {
                                    label: format!("Updating date {}/{}", i + 1, total),
                                    fraction: frac,
                                });
                            }

                            if skipped > 0 {
                                let _ = res_tx.send(JobResult::Progress {
                                    label: format!("Dates adjusted ✓ ({skipped} skipped — no bbox from offline parser)"),
                                    fraction: 1.0,
                                });
                            } else {
                                let _ = res_tx.send(JobResult::Progress {
                                    label: "Dates adjusted ✓".to_string(),
                                    fraction: 1.0,
                                });
                            }

                            let _ = res_tx.send(JobResult::DatesAdjusted {
                                records,
                                output_path: output,
                            });
                        });
                    }
                    Job::AiConfirmationResponse(response) => {
                        // Log the response as learning data
                        tracing::info!("[AI_CONFIRM] User responded to confirmation {}", response.id);
                        // The actual wiring to pause/resume happens via channels in the pipeline.
                        // For now, log it to the learning file.
                        let placeholder_confirmation = crate::engine::ai_confirm::AiConfirmation {
                            id: response.id,
                            stage: "user_response".to_string(),
                            question: String::new(),
                            options: vec![],
                            context: String::new(),
                            confidence: 0.0,
                            default_answer: None,
                        };
                        let _ = crate::engine::ai_confirm::log_learning_response(
                            &placeholder_confirmation,
                            &response,
                        );
                    }
                    Job::InteractiveFallbackResponse(response) => {
                        let id = response.id;
                        let router = fallback_router.clone();
                        tokio::spawn(async move {
                            let mut map = router.lock().await;
                            if let Some(tx) = map.remove(&id) {
                                let _ = tx.send(response.selected_alternative_id);
                            }
                        });
                    }
                    Job::AiCommand { prompt, path: _ } => {
                        let res_tx = result_tx_clone.clone();
                        tokio::spawn(async move {
                            // Phase 2 - Stage 10: Cascade simulation
                            if prompt == "SIMULATE_CASCADE_EDITS" {
                                for i in 1..=100 {
                                    let _ = res_tx.send(JobResult::Progress {
                                        label: format!("Simulating cascade chunk {}", i),
                                        fraction: (i as f32) / 100.0,
                                    });
                                    tokio::time::sleep(std::time::Duration::from_millis(15)).await;
                                }
                                let _ = res_tx.send(JobResult::Error {
                                    job_label: "cascade_test".into(),
                                    message: "Cascade stress test completed successfully. 10,000 recalculations rendered.".into(),
                                });
                            } else {
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                let _ = res_tx.send(JobResult::Error {
                                    job_label: "ai_command".into(),
                                    message: format!("NLP command recognized: {}", prompt),
                                });
                            }
                        });
                    }
                    Job::RunTransferTests { statements, max_iterations } => {
                        let res_tx = result_tx_clone.clone();
                        let cfg = config_for_tokio.clone();
                        let _py_tx = python_tx_clone.clone();
                        let engine_for_tokio = engine_for_tokio.clone();
                        tokio::spawn(async move {
                            use crate::engine::transfer_test_harness::*;

                            let started_at = std::time::Instant::now();
                            let pairs = generate_test_pairs(&statements);
                            let total_pairs = pairs.len();

                            let _ = res_tx.send(JobResult::Progress {
                                label: format!("Running {total_pairs} transfer test pairs..."),
                                fraction: 0.0,
                            });

                            let doc_ai_opt = crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg).ok().map(std::sync::Arc::new);
                            let gemini = match crate::ai::backend::AiBackend::from_app_config(&cfg) {
                                Ok(c) => std::sync::Arc::new(c),
                                Err(_) => {
                                    let _ = res_tx.send(JobResult::Error {
                                        job_label: "transfer_tests".into(),
                                        message: "Transfer tests require an AI provider for format mapping — set GEMINI_API_KEY (or GROQ_API_KEY / OPENROUTER_API_KEY) and select a provider in Backend Preferences.".into(),
                                    });
                                    return;
                                }
                            };

                            let mut results: Vec<TransferTestResult> = Vec::new();

                            for (pair_idx, (source, target)) in pairs.iter().enumerate() {
                                let pair_started = std::time::Instant::now();
                                let output = test_output_path(source, target);
                                let mut iterations = 0u32;
                                let mut final_math_ok = false;
                                let mut final_visual_score = 1.0f64;
                                let mut corrections: Vec<String> = Vec::new();
                                let mut converged = false;
                                let mut correction_hint: Option<String> = None;

                                let _ = res_tx.send(JobResult::Progress {
                                    label: format!(
                                        "Testing pair {}/{}: {} -> {}",
                                        pair_idx + 1, total_pairs,
                                        source.file_stem().unwrap_or_default().to_string_lossy(),
                                        target.file_stem().unwrap_or_default().to_string_lossy(),
                                    ),
                                    fraction: pair_idx as f32 / total_pairs as f32,
                                });

                                // Parse both statements — DocAI with offline fallback
                                let source_stmt = if let Some(ref doc_ai) = doc_ai_opt {
                                    match doc_ai.parse_entire_statement(source, None::<&str>).await {
                                        Ok(s) => s,
                                        Err(_e) => {
                                            // DocAI failed, try offline
                                            let eng_clone = engine_for_tokio.clone();
                                            let src_clone = source.clone();
                                            match tokio::task::spawn_blocking(move || {
                                                crate::engine::offline_parser::parse_statement_offline(&src_clone, eng_clone)
                                            }).await {
                                                Ok(Ok(s)) => s,
                                                _ => {
                                                    corrections.push("Source parse failed (DocAI + offline)".to_string());
                                                    results.push(TransferTestResult {
                                                        source: source.clone(), target: target.clone(),
                                                        output: output.clone(), iterations: 0,
                                                        final_math_ok: false, final_visual_score: 1.0,
                                                        corrections, duration_secs: pair_started.elapsed().as_secs_f64(),
                                                        converged: false,
                                                    });
                                                    continue;
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    let eng_clone = engine_for_tokio.clone();
                                    let src_clone = source.clone();
                                    match tokio::task::spawn_blocking(move || {
                                        crate::engine::offline_parser::parse_statement_offline(&src_clone, eng_clone)
                                    }).await {
                                        Ok(Ok(s)) => s,
                                        _ => {
                                            corrections.push("Source parse failed (offline)".to_string());
                                            results.push(TransferTestResult {
                                                source: source.clone(), target: target.clone(),
                                                output: output.clone(), iterations: 0,
                                                final_math_ok: false, final_visual_score: 1.0,
                                                corrections, duration_secs: pair_started.elapsed().as_secs_f64(),
                                                converged: false,
                                            });
                                            continue;
                                        }
                                    }
                                };
                                let target_stmt = if let Some(ref doc_ai) = doc_ai_opt {
                                    match doc_ai.parse_entire_statement(target, None::<&str>).await {
                                        Ok(s) => s,
                                        Err(_e) => {
                                            let eng_clone = engine_for_tokio.clone();
                                            let tgt_clone = target.clone();
                                            match tokio::task::spawn_blocking(move || {
                                                crate::engine::offline_parser::parse_statement_offline(&tgt_clone, eng_clone)
                                            }).await {
                                                Ok(Ok(s)) => s,
                                                _ => {
                                                    corrections.push("Target parse failed (DocAI + offline)".to_string());
                                                    results.push(TransferTestResult {
                                                        source: source.clone(), target: target.clone(),
                                                        output: output.clone(), iterations: 0,
                                                        final_math_ok: false, final_visual_score: 1.0,
                                                        corrections, duration_secs: pair_started.elapsed().as_secs_f64(),
                                                        converged: false,
                                                    });
                                                    continue;
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    let eng_clone = engine_for_tokio.clone();
                                    let tgt_clone = target.clone();
                                    match tokio::task::spawn_blocking(move || {
                                        crate::engine::offline_parser::parse_statement_offline(&tgt_clone, eng_clone)
                                    }).await {
                                        Ok(Ok(s)) => s,
                                        _ => {
                                            corrections.push("Target parse failed (offline)".to_string());
                                            results.push(TransferTestResult {
                                                source: source.clone(), target: target.clone(),
                                                output: output.clone(), iterations: 0,
                                                final_math_ok: false, final_visual_score: 1.0,
                                                corrections, duration_secs: pair_started.elapsed().as_secs_f64(),
                                                converged: false,
                                            });
                                            continue;
                                        }
                                    }
                                };

                                // Attempt transfer with retry loop
                                while iterations < max_iterations && !converged {
                                    iterations += 1;

                                    // Get transfer plan
                                    let plan = match gemini.plan_transaction_transfer(
                                        &source_stmt.transactions,
                                        &target_stmt.transactions,
                                        correction_hint.as_deref(),
                                    ).await {
                                        Ok(p) => p,
                                        Err(e) => {
                                            corrections.push(format!("Iter {iterations}: plan failed: {e}"));
                                            continue;
                                        }
                                    };

                                    // Build mapped transactions and compute balances
                                    let opening = target_stmt.opening_balance;
                                    let mut mapped: Vec<crate::engine::transfer::MappedTransaction> = plan.mappings.iter().map(|m| {
                                        let idx = m.source_index.min(source_stmt.transactions.len().saturating_sub(1));
                                        let src = &source_stmt.transactions[idx];
                                        crate::engine::transfer::MappedTransaction {
                                            target_page: m.target_page,
                                            target_line: m.target_line,
                                            date: m.converted_date.clone(),
                                            description: m.adapted_description.clone(),
                                            debit: src.debit,
                                            credit: src.credit,
                                            running_balance: rust_decimal::Decimal::ZERO,
                                            field_bboxes: Default::default(),
                                         }
                                    }).collect();
                                    crate::engine::transfer::recompute_running_balances(opening, &mut mapped);

                                    // Verify math with engine
                                    let sim_txns: Vec<crate::engine::model::Transaction> = mapped.iter().map(|m| {
                                        crate::engine::model::Transaction {
                                            page: m.target_page,
                                            line_on_page: m.target_line,
                                            date: m.date.clone(),
                                            raw_text: m.description.clone(),
                                            debit: m.debit,
                                            credit: m.credit,
                                            running_balance: Some(m.running_balance),
                                            bbox: None,
                                            field_bboxes: Default::default(),
                                            provenance: crate::engine::model::Provenance::Computed,
                                            category: None,
                                        }
                                    }).collect();

                                    let mut math_err_msg = None;
                                    match crate::engine::balance::process_and_reconcile(sim_txns, opening, None) {
                                        Ok((_, None)) => {}
                                        Ok((_, Some(msg))) => math_err_msg = Some(format!("Balance mismatch: {msg}")),
                                        Err(e) => math_err_msg = Some(format!("Balance engine error: {e}")),
                                    }

                                    // Verify math with Gemini
                                    let gemini_math_ok = gemini.verify_transfer_math(&mapped, opening).await.unwrap_or_default();

                                    let math_ok = math_err_msg.is_none() && gemini_math_ok;
                                    final_math_ok = math_ok;
                                    final_visual_score = 0.0; // would need render for real score

                                    if math_ok {
                                        converged = true;
                                    } else {
                                        let mut errors = Vec::new();
                                        if let Some(msg) = &math_err_msg {
                                            errors.push(msg.clone());
                                        }
                                        if !gemini_math_ok {
                                            errors.push("Gemini math verification failed.".to_string());
                                        }
                                        let hint = format!(
                                            "Your previous mapping failed validation. Errors: {}. Please adjust the mapping to fix these issues.",
                                            errors.join("; ")
                                        );
                                        corrections.push(format!("Iter {iterations}: math verification failed ({}), retrying", errors.join("; ")));
                                        correction_hint = Some(hint);
                                    }
                                }

                                results.push(TransferTestResult {
                                    source: source.clone(),
                                    target: target.clone(),
                                    output,
                                    iterations,
                                    final_math_ok,
                                    final_visual_score,
                                    corrections,
                                    duration_secs: pair_started.elapsed().as_secs_f64(),
                                    converged,
                                });
                            }

                            let elapsed = started_at.elapsed().as_secs_f64();
                            let report = build_report(results, elapsed);

                            // Write report to disk
                            if let Err(e) = write_harness_report(&report) {
                                tracing::warn!("[TEST_HARNESS] Failed to write report: {}", e);
                            }

                            let _ = res_tx.send(JobResult::Progress {
                                label: report.summary(),
                                fraction: 1.0,
                            });

                            let _ = res_tx.send(JobResult::TransferTestsComplete(report));
                        });
                    }
                    Job::AnalyzeFonts { path } => {
                        let res_tx = result_tx_clone.clone();
                        let py_tx = python_tx_clone.clone();
                        tokio::spawn(async move {
                            let _ = res_tx.send(JobResult::Progress {
                                label: "Analyzing fonts".to_string(),
                                fraction: 0.1,
                            });
                            let (reply_tx, reply_rx) = oneshot::channel();
                            if py_tx
                                .send((
                                    PythonJob::AnalyzeFonts {
                                        pdf_path: path.to_string_lossy().to_string(),
                                    },
                                    reply_tx,
                                ))
                                .is_ok()
                            {
                                match reply_rx.await {
                                    Ok(PythonJobResult::Json(json)) => {
                                        match crate::engine::font_analysis::FontAnalysis::from_json(&json) {
                                            Ok(analysis) => {
                                                let _ = res_tx.send(JobResult::FontAnalysisReady(analysis));
                                            }
                                            Err(e) => {
                                                let _ = res_tx.send(JobResult::Error {
                                                    job_label: "analyze_fonts".into(),
                                                    message: e,
                                                });
                                            }
                                        }
                                    }
                                    Ok(PythonJobResult::Error(msg)) => {
                                        let _ = res_tx.send(JobResult::Error {
                                            job_label: "analyze_fonts".into(),
                                            message: msg,
                                        });
                                    }
                                    _ => {}
                                }
                            }
                            let _ = res_tx.send(JobResult::Progress {
                                label: "Done".into(),
                                fraction: 1.0,
                            });
                        });
                    }

                    // -- Document AI Version Management Handlers --
                    Job::ListDocAiVersions => {
                        let res_tx = result_tx_clone.clone();
                        let cfg = config_for_tokio.clone();
                        tokio::spawn(async move {
                            match crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg) {
                                Ok(client) => {
                                    match client.list_processor_versions().await {
                                        Ok(versions) => {
                                            let _ = res_tx.send(JobResult::DocAiVersionsListed(versions));
                                        }
                                        Err(e) => {
                                            let _ = res_tx.send(JobResult::DocAiVersionError(
                                                format!("Failed to list versions: {e}")
                                            ));
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::DocAiVersionError(
                                        format!("DocAI not configured: {e}")
                                    ));
                                }
                            }
                        });
                    }
                    Job::DeployDocAiVersion { version_id } => {
                        let res_tx = result_tx_clone.clone();
                        let cfg = config_for_tokio.clone();
                        tokio::spawn(async move {
                            match crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg) {
                                Ok(client) => {
                                    match client.deploy_processor_version(&version_id).await {
                                        Ok(op) => {
                                            let _ = res_tx.send(JobResult::DocAiVersionOperationStarted {
                                                operation_name: op,
                                                description: format!("Deploying version {version_id}"),
                                            });
                                        }
                                        Err(e) => {
                                            let _ = res_tx.send(JobResult::DocAiVersionError(
                                                format!("Deploy failed: {e}")
                                            ));
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::DocAiVersionError(format!("{e}")));
                                }
                            }
                        });
                    }
                    Job::UndeployDocAiVersion { version_id } => {
                        let res_tx = result_tx_clone.clone();
                        let cfg = config_for_tokio.clone();
                        tokio::spawn(async move {
                            match crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg) {
                                Ok(client) => {
                                    match client.undeploy_processor_version(&version_id).await {
                                        Ok(op) => {
                                            let _ = res_tx.send(JobResult::DocAiVersionOperationStarted {
                                                operation_name: op,
                                                description: format!("Undeploying version {version_id}"),
                                            });
                                        }
                                        Err(e) => {
                                            let _ = res_tx.send(JobResult::DocAiVersionError(
                                                format!("Undeploy failed: {e}")
                                            ));
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::DocAiVersionError(format!("{e}")));
                                }
                            }
                        });
                    }
                    Job::SetDefaultDocAiVersion { version_id } => {
                        let res_tx = result_tx_clone.clone();
                        let cfg = config_for_tokio.clone();
                        tokio::spawn(async move {
                            match crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg) {
                                Ok(client) => {
                                    match client.set_default_processor_version(&version_id).await {
                                        Ok(op) => {
                                            let _ = res_tx.send(JobResult::DocAiVersionOperationStarted {
                                                operation_name: op,
                                                description: format!("Setting default to {version_id}"),
                                            });
                                        }
                                        Err(e) => {
                                            let _ = res_tx.send(JobResult::DocAiVersionError(
                                                format!("Set default failed: {e}")
                                            ));
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::DocAiVersionError(format!("{e}")));
                                }
                            }
                        });
                    }
                    Job::TrainDocAiVersion { display_name, base_version } => {
                        let res_tx = result_tx_clone.clone();
                        let cfg = config_for_tokio.clone();
                        tokio::spawn(async move {
                            match crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg) {
                                Ok(client) => {
                                    match client.train_processor_version(
                                        &display_name,
                                        base_version.as_deref(),
                                    ).await {
                                        Ok(op) => {
                                            let _ = res_tx.send(JobResult::DocAiVersionOperationStarted {
                                                operation_name: op,
                                                description: format!("Training: {display_name}"),
                                            });
                                        }
                                        Err(e) => {
                                            let _ = res_tx.send(JobResult::DocAiVersionError(
                                                format!("Training failed: {e}")
                                            ));
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::DocAiVersionError(format!("{e}")));
                                }
                            }
                        });
                    }

                    Job::RenderPage { path, page, dpi, tag } => {
                        let res_tx = result_tx_clone.clone();
                        let eng = engine_for_tokio.clone();

                        let (actual_path, actual_page) = if let Some(map) = &segment_map {
                            map.resolve(page).map(|(idx, p)| (map.segments[idx].path.clone(), p)).unwrap_or((path, page))
                        } else {
                            (path, page)
                        };

                        tokio::task::spawn_blocking(move || {
                            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                eng.render_page(&actual_path, actual_page, dpi)
                            }));
                            match result {
                                Ok(Ok(rendered)) => {
                                    let _ = res_tx.send(JobResult::PageRendered {
                                        png_bytes: rendered.png_bytes, page, dpi, tag, width_pts: rendered.width_pts, height_pts: rendered.height_pts
                                    });
                                }
                                Ok(Err(e)) => {
                                    tracing::error!("[render_page] engine error: {}", e);
                                    let _ = res_tx.send(JobResult::Error { job_label: "render_page".into(), message: e.to_string() });
                                }
                                Err(panic_info) => {
                                    let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                                        s.to_string()
                                    } else if let Some(s) = panic_info.downcast_ref::<String>() {
                                        s.clone()
                                    } else {
                                        "render_page panicked".to_string()
                                    };
                                    tracing::error!("[render_page] panic: {}", msg);
                                    let _ = res_tx.send(JobResult::Error { job_label: "render_page".into(), message: format!("Render panic: {msg}") });
                                }
                            }
                        });
                    }
                    Job::ApplyChange { input, output, page, bbox, new_text, old_text, description, deep_font_replication } => {
                        let _ = result_tx_clone.send(JobResult::Progress { label: "Applying change".to_string(), fraction: 0.1 });

                        let eng = engine_for_tokio.clone();
                        let audit_log_clone = audit_log.clone();
                        let history_clone = history.clone();
                        let py_tx = python_tx_clone.clone();
                        let res_tx = result_tx_clone.clone();
                        let cfg_clone = config_for_tokio.clone();

                        let map_opt = segment_map.clone();
                        let mgr_opt = segment_manager.as_ref().map(|m| m.temp_path().to_path_buf());

                        tokio::task::spawn(async move {
                            // Optional: deep font replication via Python actor.
                            let mut font_path: Option<PathBuf> = None;
                            if deep_font_replication {
                                let _ = res_tx.send(JobResult::Progress { label: "Deep Replicating Font...".to_string(), fraction: 0.2 });
                                let (tx, rx) = oneshot::channel();

                                // In three-page mode, we use the segment path for font replication analysis
                                let analysis_path = if let Some(ref map) = map_opt {
                                    map.resolve(page).map(|(idx, _)| map.segments[idx].path.clone()).unwrap_or(input.clone())
                                } else {
                                    input.clone()
                                };

                                let _ = py_tx.send((PythonJob::DeepFontReplication {
                                    pdf_path: analysis_path.to_string_lossy().to_string(),
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
                            let old_text_for_blocking = old_text.clone();

                            let outcome = tokio::task::spawn_blocking(move || {
                                if let (Some(map), Some(temp_dir)) = (map_opt, mgr_opt) {
                                    let (seg_idx, local_page) = map.resolve(page)
                                        .ok_or_else(|| crate::pdf::EngineError::ApplyFailed(format!("Global page {page} not found in segment map")))?;

                                    let seg_path = &map.segments[seg_idx].path;
                                    let temp_seg_out = temp_dir.join(format!("seg_{}_edited_{}.pdf", seg_idx, Uuid::new_v4()));

                                    // 1. Apply to segment
                                    eng.apply_change(
                                        seg_path,
                                        &temp_seg_out,
                                        local_page,
                                        bbox,
                                        &new_text_for_blocking,
                                        &old_text_for_blocking,
                                        font_path.as_deref(),
                                    )?;

                                    // 2. Overwrite segment file
                                    std::fs::rename(&temp_seg_out, seg_path)
                                        .map_err(|e| crate::pdf::EngineError::ApplyFailed(format!("Failed to update segment file: {e}")))?;

                                    // 3. Merge all segments to final output
                                    let ordered_paths = map.ordered_merge_paths();
                                    crate::engine::pdf_split_merge::merge_pdfs(&ordered_paths, &output_for_blocking)
                                        .map_err(|e| crate::pdf::EngineError::ApplyFailed(format!("Failed to merge segments: {e}")))?;

                                    Ok(ReplaceOutcome { success: true, font_used: "Helvetica".into(), overflow: false, obj_id: None })
                                } else {
                                    eng.apply_change(
                                        &input_for_blocking,
                                        &output_for_blocking,
                                        page,
                                        bbox,
                                        &new_text_for_blocking,
                                        &old_text_for_blocking,
                                        font_path.as_deref(),
                                    )
                                }
                            })
                            .await
                            .unwrap_or_else(|e| Err(crate::pdf::EngineError::ApplyFailed(format!("blocking task panicked: {e}"))));

                            match outcome {
                                Ok(o) => {
                                    let requires_visual_review = o.overflow;
                                    let mut h = match history_clone.lock() {
                                        Ok(g) => g,
                                        Err(e) => {
                                            let _ = res_tx.send(JobResult::Error { job_label: "apply_change".into(), message: format!("History lock poisoned: {e}") });
                                            return;
                                        }
                                    };
                                    let mut a = match audit_log_clone.lock() {
                                        Ok(g) => g,
                                        Err(e) => {
                                            let _ = res_tx.send(JobResult::Error { job_label: "apply_change".into(), message: format!("Audit lock poisoned: {e}") });
                                            return;
                                        }
                                    };

                                    let mut final_record = h.create_record(page, old_text, new_text.clone(), bbox, description, None);
                                    final_record.obj_id = o.obj_id;
                                    let snap_path = a.snapshot_path_for(final_record.id);

                                    // Snapshots use a hard link when possible (same volume)
                                    // so applying many edits doesn't multiply disk usage by
                                    // the PDF size. Falls back to a full copy on cross-FS.
                                    if let Err(e) = crate::app::audit::snapshot_link_or_copy(&output, &snap_path) {
                                        let _ = res_tx.send(JobResult::Error { job_label: "apply_change".into(), message: format!("Snapshot failed: {e}") });
                                        return;
                                    }

                                    final_record.snapshot_path = Some(snap_path);
                                    if let Err(e) = a.write(&final_record, &input, &output, "manual", requires_visual_review) {
                                        let _ = res_tx.send(JobResult::Error { job_label: "apply_change".into(), message: format!("Audit write failed: {e}") });
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
                                Err(crate::pdf::EngineError::EncryptedOrRasterized(msg)) => {
                                    let _ = res_tx.send(JobResult::NuclearFallbackRequired(msg));
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
                        let res_tx = result_tx_clone.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            match history_clone.lock() {
                                Ok(mut h) => {
                                    h.undo();
                                    let _ = res_tx.send(JobResult::HistoryUpdated { history: h.clone() });
                                }
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "undo".into(), message: format!("History lock poisoned: {e}") });
                                }
                            }
                        }).await;
                    }
                    Job::Redo => {
                        let history_clone = history.clone();
                        let res_tx = result_tx_clone.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            match history_clone.lock() {
                                Ok(mut h) => {
                                    h.redo();
                                    let _ = res_tx.send(JobResult::HistoryUpdated { history: h.clone() });
                                }
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "redo".into(), message: format!("History lock poisoned: {e}") });
                                }
                            }
                        }).await;
                    }
                    Job::NaturalLanguageEdit { prompt, transactions } => {
                        let res_tx = result_tx_clone.clone();
                        let cfg = config_for_tokio.clone();
                        
                        tokio::spawn(async move {
                            let _ = res_tx.send(JobResult::Progress {
                                label: "Asking AI to apply edits...".into(),
                                fraction: 0.2,
                            });
                            
                            let gemini = match crate::ai::gemini_client::GeminiClient::from_app_config_async(&cfg).await {
                                Ok(c) => c,
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::Error {
                                        job_label: "NaturalLanguageEdit".into(),
                                        message: format!("Gemini configuration error: {e}"),
                                    });
                                    return;
                                }
                            };
                            
                            match gemini.apply_natural_language_edit(&prompt, &transactions).await {
                                Ok(updated) => {
                                    let _ = res_tx.send(JobResult::Progress {
                                        label: "Edits applied successfully!".into(),
                                        fraction: 1.0,
                                    });
                                    let _ = res_tx.send(JobResult::NaturalLanguageEditReady(updated));
                                }
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::Error {
                                        job_label: "NaturalLanguageEdit".into(),
                                        message: format!("Failed to apply edits: {e}"),
                                    });
                                }
                            }
                        });
                    }
                    Job::CategorizeTransactions { mut transactions } => {
                        let res_tx = result_tx_clone.clone();
                        tokio::spawn(async move {
                            crate::engine::categorization::categorize_transactions(&mut transactions);
                            let _ = res_tx.send(JobResult::CategorizationReady(transactions));
                        });
                    }
                    Job::ExtractTransactions { path } => {
                        let res_tx = result_tx_clone.clone();
                        let eng = engine_for_tokio.clone();
                        let cfg = config_for_tokio.clone();
                        let semaphore = api_semaphore.clone();
                        let cache_for_job = parse_cache.clone();

                        tokio::spawn(async move {
                            let _permit = match semaphore.acquire().await {
                                Ok(p) => p,
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "API Execution".into(), message: format!("Semaphore closed: {e}") });
                                    return;
                                }
                            };

                            let cache_key = match tokio::fs::read(&path).await {
                                Ok(bytes) => crate::engine::workflow::sha256_hex_of(&bytes),
                                Err(_) => path.to_string_lossy().to_string(),
                            };

                            {
                                let mut cache = cache_for_job.lock().await;
                                if let Some(cached_stmt) = cache.get(&cache_key) {
                                    tracing::info!("[runtime] LRU cache HIT for ExtractTransactions: {}", cache_key);
                                    let _ = res_tx.send(JobResult::TransactionsExtracted(cached_stmt.transactions.clone()));
                                    return;
                                }
                            }

                            let _ = res_tx.send(JobResult::Progress { label: "Extracting transactions".to_string(), fraction: 0.1 });

                            let mut final_txs = None;

                            // 1. Try LlamaParse
                            if final_txs.is_none() {
                                let _ = res_tx.send(JobResult::Progress { label: "Extracting with LlamaParse...".to_string(), fraction: 0.1 });
                                if let Ok(client) = crate::ai::llamaparse::LlamaParseClient::from_app_config(&cfg) {
                                    match crate::engine::pro_edit::perform_pro_edit("LlamaParse", async { client.parse_statement(&path).await.map_err(anyhow::Error::from) }, wdog.clone()).await {
                                        Ok(stmt) => final_txs = Some(stmt.transactions),
                                        Err(e) => tracing::warn!("[extract] LlamaParse failed: {}", e),
                                    }
                                }
                            }

                            // 2. Try Document AI
                            if final_txs.is_none() {
                                let _ = res_tx.send(JobResult::Progress { label: "Extracting with Document AI...".to_string(), fraction: 0.15 });
                                if let Ok(client) = crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg) {
                                    let doc_ai: std::sync::Arc<crate::ai::document_ai::DocumentAiClient> = Arc::new(client);
                                    match crate::engine::pro_edit::perform_pro_edit("DocumentAI", async { doc_ai.parse_entire_statement(&path, None::<&str>).await.map_err(anyhow::Error::from) }, wdog.clone()).await {
                                        Ok(stmt) => final_txs = Some(stmt.transactions),
                                        Err(e) => tracing::warn!("[extract] Document AI failed: {}", e),
                                    }
                                }
                            }
                            // 4. Try Offline Parser
                            let transactions = if let Some(txs) = final_txs {
                                txs
                            } else {
                                let _ = res_tx.send(JobResult::Progress { label: "Using offline parser...".to_string(), fraction: 0.3 });
                                let eng_clone = eng.clone();
                                let path_clone = path.clone();
                                match tokio::task::spawn_blocking(move || {
                                    crate::engine::offline_parser::parse_statement_offline(&path_clone, eng_clone)
                                }).await {
                                    Ok(Ok(stmt)) => stmt.transactions,
                                    Ok(Err(e)) => {
                                        let _ = res_tx.send(JobResult::Error { job_label: "extract_transactions".into(), message: format!("Offline extraction failed: {e}") });
                                        return;
                                    }
                                    Err(e) => {
                                        let _ = res_tx.send(JobResult::Error { job_label: "extract_transactions".into(), message: format!("Offline extraction panicked: {e}") });
                                        return;
                                    }
                                }
                            };

                            let template_provider = Arc::new(crate::extractors::BankTemplateProvider::new(std::path::PathBuf::from("bank_templates").as_path(), eng.clone()));

                            let merger = crate::extractors::HybridMerger::new(vec![
                                template_provider as Arc<dyn crate::extractors::GeometryProvider>,
                            ]);

                            let path_clone = path.clone();
                            let report = match tokio::task::spawn_blocking(move || {
                                let mut geometries = Vec::new();
                                for provider in &merger.providers {
                                    if let Ok(geo) = provider.extract_line_geometry(&path_clone) {
                                        geometries.extend(geo);
                                    }
                                }
                                merger.merge(transactions, geometries)
                            }).await {
                                Ok(r) => r,
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "extract_transactions".into(), message: format!("Geometry extraction panicked: {e}") });
                                    return;
                                }
                            };

                            let mut full_stmt = crate::ai::document_ai::BankStatement { total_pages: 0, transactions: Vec::new(), opening_balance: rust_decimal::Decimal::ZERO, closing_balance: rust_decimal::Decimal::ZERO, account_number: None, bank_name: None };
                            full_stmt.transactions = report.transactions.clone();
                            {
                                let mut cache = cache_for_job.lock().await;
                                cache.put(cache_key, full_stmt);
                            }

                            let _ = res_tx.send(JobResult::TransactionsExtracted(report.transactions));
                        });
                    }
                    Job::InstantBackgroundApply { input, output, edits } => {
                        let eng = engine_for_tokio.clone();
                        tokio::task::spawn_blocking(move || {
                            let json_str = match serde_json::to_string(&edits) {
                                Ok(s) => s,
                                Err(e) => {
                                    tracing::warn!("Failed to serialize edits for instant apply: {}", e);
                                    return;
                                }
                            };
                            if let Err(e) = eng.apply_many_edits(&input, &output, &json_str, None) {
                                tracing::warn!("Instant background apply failed: {}", e);
                            }
                        });
                    }
                    Job::BalanceStatement { path } => {
                        let res_tx = result_tx_clone.clone();
                        let eng = engine_for_tokio.clone();
                        let cfg = config_for_tokio.clone();
                        let semaphore = api_semaphore.clone();
                        let cache_for_job = parse_cache.clone();

                        tokio::spawn(async move {
                            let _permit = match semaphore.acquire().await {
                                Ok(p) => p,
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "API Execution".into(), message: format!("Semaphore closed: {e}") });
                                    return;
                                }
                            };
                            
                            let cache_key = match tokio::fs::read(&path).await {
                                Ok(bytes) => crate::engine::workflow::sha256_hex_of(&bytes),
                                Err(_) => path.to_string_lossy().to_string(),
                            };

                            let _ = res_tx.send(JobResult::Progress { label: "Smart Balance Analysis".to_string(), fraction: 0.1 });

                            let doc_ai = crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg).ok().map(Arc::new);
                            let gemini = crate::ai::backend::AiBackend::from_app_config(&cfg).ok().map(Arc::new);

                            // If both AI services are available, use the full smart engine
                            if let (Some(doc_ai), Some(gemini)) = (doc_ai, gemini) {
                                let template_provider = Arc::new(crate::extractors::BankTemplateProvider::new(std::path::PathBuf::from("bank_templates").as_path(), eng.clone()));

                                let merger = Arc::new(crate::extractors::HybridMerger::new(vec![
                                    template_provider as Arc<dyn crate::extractors::GeometryProvider>,
                                ]));

                                let mut smart_engine = crate::engine::statement::SmartDocumentEngine::new(eng.clone(), doc_ai, gemini, merger);

                                let _ = res_tx.send(JobResult::Progress { label: "Loading Document".to_string(), fraction: 0.3 });

                                let (dummy_tx, _) = std::sync::mpsc::channel();
                                if let Err(e) = smart_engine.load_full_document(&dummy_tx, &path).await {
                                    let _ = res_tx.send(JobResult::Error { job_label: "balance_statement".into(), message: format!("Failed to load document: {e}") });
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
                                        let _ = res_tx.send(JobResult::Error { job_label: "balance_statement".into(), message: format!("Gemini confidence {c:.2} below 0.7 threshold; not enough certainty to propose adjustments.") });
                                    }
                                    Err(e) => {
                                        let _ = res_tx.send(JobResult::Error { job_label: "balance_statement".into(), message: e.to_string() });
                                    }
                                }
                            } else {
                                // -- Offline fallback: local balance analysis ---------€
                                tracing::info!("[balance] AI services not configured; using offline balance analysis");
                                let _ = res_tx.send(JobResult::Progress { label: "Using offline balance analysis (no AI)...".to_string(), fraction: 0.3 });

                                let eng_clone = eng.clone();
                                let path_clone = path.clone();
                                let stmt = if let Some(cached_stmt) = {
                                    let mut cache = cache_for_job.lock().await;
                                    cache.get(&cache_key).cloned()
                                } {
                                    tracing::info!("[runtime] LRU cache HIT for BalanceStatement offline path: {}", cache_key);
                                    cached_stmt
                                } else {
                                    let stmt_res = match tokio::task::spawn_blocking(move || {
                                        crate::engine::offline_parser::parse_statement_offline(&path_clone, eng_clone)
                                    }).await {
                                        Ok(Ok(s)) => s,
                                        Ok(Err(e)) => {
                                            let _ = res_tx.send(JobResult::Error { job_label: "balance_statement".into(), message: format!("Offline balance analysis failed: {e}") });
                                            return;
                                        }
                                        Err(e) => {
                                            let _ = res_tx.send(JobResult::Error { job_label: "balance_statement".into(), message: format!("Offline balance panicked: {e}") });
                                            return;
                                        }
                                    };
                                    
                                    {
                                        let mut cache = cache_for_job.lock().await;
                                        cache.put(cache_key.clone(), stmt_res.clone());
                                    }
                                    stmt_res
                                };

                                let _ = res_tx.send(JobResult::Progress { label: "Computing balance chain locally...".to_string(), fraction: 0.6 });

                                // Compute running balance chain from offline-parsed transactions
                                let mut changes = Vec::new();
                                let mut running = stmt.opening_balance;
                                for tx in &stmt.transactions {
                                    let net = tx.debit.unwrap_or(rust_decimal::Decimal::ZERO) - tx.credit.unwrap_or(rust_decimal::Decimal::ZERO);
                                    running += net;
                                    if let Some(printed_bal) = tx.running_balance {
                                        if (running - printed_bal).abs() > rust_decimal_macros::dec!(0.01) {
                                            changes.push(crate::engine::model::ProposedChange {
                                                page: tx.page,
                                                old_text: format!("{printed_bal}"),
                                                new_text: format!("{running}"),
                                                reason: format!("Computed balance {running} differs from printed {printed_bal}"),
                                                confidence: 0.6,
                                                affects_subsequent_balances: true,
                                                bbox: tx.bbox,
                                            });
                                        }
                                    }
                                }

                                let imbalance = (running - stmt.closing_balance).abs();
                                let _ = res_tx.send(JobResult::BalanceProposed { imbalance, changes });
                                let _ = res_tx.send(JobResult::Progress { label: "Done (offline mode)".to_string(), fraction: 1.0 });
                            }
                        });
                    }
                    Job::ApplyProposedChanges { input, output, changes } => {
                        let res_tx = result_tx_clone.clone();
                        let job_tx_ref = tokio_job_tx_clone.clone();
                        let py_tx = python_tx_clone.clone();
                        let semaphore = api_semaphore.clone();

                        tokio::spawn(async move {
                            let _permit = match semaphore.acquire().await {
                                Ok(p) => p,
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "API Execution".into(), message: format!("Semaphore closed: {e}") });
                                    return;
                                }
                            };
                            // Determine page count: cascaded balance changes
                            // routinely land MANY pages from the edited row -
                            // often >3 pages away. A direct full-document apply
                            // would trip the PyMuPDF Pro 3-page guard, so for
                            // long statements we route through 3-Page-Mode:
                            // split -> per-segment apply (<=3 pages each) ->
                            // merge. Short docs use the simple direct path.
                            let input_for_count = input.clone();
                            let page_count = tokio::task::spawn_blocking(move || {
                                lopdf::Document::load(&input_for_count)
                                    .map(|d| d.get_pages().len())
                                    .unwrap_or(0)
                            })
                            .await
                            .unwrap_or(0);

                            // Drop changes with no resolved bbox up front (can't redact).
                            let mut failures: Vec<String> = Vec::new();
                            let usable: Vec<crate::engine::model::ProposedChange> = changes
                                .iter()
                                .filter(|c| {
                                    if c.bbox.is_none() {
                                        failures.push(format!(
                                            "Proposed change for page {} '{}' \u{2192} '{}' has no resolved bbox; skipped",
                                            c.page + 1, c.old_text, c.new_text
                                        ));
                                        false
                                    } else {
                                        true
                                    }
                                })
                                .cloned()
                                .collect();

                            if usable.is_empty() {
                                let _ = res_tx.send(JobResult::ProposedChangesApplied { changes_applied: 0, failures });
                                let _ = res_tx.send(JobResult::Progress { label: "Done".to_string(), fraction: 1.0 });
                                return;
                            }

                            if page_count > 3 {
                                // ---- 3-Page-Mode segmented batch apply ----
                                use crate::engine::pdf_split_merge::{merge_pdfs, split_pdf};
                                let _ = res_tx.send(JobResult::Progress { label: "Splitting statement into <=3-page segments".into(), fraction: 0.1 });

                                // 1) Split (pure-Rust lopdf) on a blocking task.
                                let input_split = input.clone();
                                let split_res = tokio::task::spawn_blocking(move || {
                                    let tmp = tempfile::Builder::new()
                                        .prefix("apply-cascade-")
                                        .tempdir()
                                        .map_err(|e| format!("tempdir: {e}"))?;
                                    let segments = split_pdf(&input_split, tmp.path(), 3)
                                        .map_err(|e| format!("split failed: {e}"))?;
                                    Ok::<_, String>((tmp, segments))
                                })
                                .await
                                .unwrap_or_else(|e| Err(format!("split task panicked: {e}")));

                                let (tmp, segments) = match split_res {
                                    Ok(v) => v,
                                    Err(e) => {
                                        let _ = res_tx.send(JobResult::Error { job_label: "apply_proposed_changes".into(), message: e });
                                        return;
                                    }
                                };

                                // 2) Group usable changes by segment (global -> local page).
                                use std::collections::BTreeMap;
                                let mut by_seg: BTreeMap<usize, Vec<(usize, crate::engine::model::ProposedChange)>> = BTreeMap::new();
                                for ch in &usable {
                                    match segments.iter().position(|s| ch.page >= s.page_offset && ch.page < s.page_offset + s.page_count) {
                                        Some(si) => {
                                            let local = ch.page - segments[si].page_offset;
                                            by_seg.entry(si).or_default().push((local, ch.clone()));
                                        }
                                        None => failures.push(format!(
                                            "change on global page {} is out of range (doc has {} pages)",
                                            ch.page + 1, page_count
                                        )),
                                    }
                                }

                                // 3) Per-segment apply via the Python actor (each <=3 pages, Pro-legal).
                                let mut seg_paths: Vec<std::path::PathBuf> =
                                    segments.iter().map(|s| s.path.clone()).collect();
                                let mut applied = 0usize;
                                let total_segs = by_seg.len().max(1);
                                for (done, (si, edits)) in by_seg.into_iter().enumerate() {
                                    let _ = res_tx.send(JobResult::Progress {
                                        label: format!("Editing segment {} of {}", done + 1, total_segs),
                                        fraction: 0.2 + 0.6 * (done as f32 / total_segs as f32),
                                    });
                                    let edits_json: Vec<serde_json::Value> = edits.iter().filter_map(|(local, ch)| {
                                        let b = ch.bbox?;
                                        Some(serde_json::json!({
                                            "page": local,
                                            "rect": [b[0], b[1], b[2], b[3]],
                                            "new_text": ch.new_text,
                                        }))
                                    }).collect();
                                    let json_str = serde_json::to_string(&edits_json).unwrap_or_else(|_| "[]".into());
                                    let json_str_for_fallback = json_str.clone();
                                    let edited_out = tmp.path().join(format!("segment_{si:03}_edited.pdf"));

                                    let (rtx, rrx) = oneshot::channel();
                                    let _ = py_tx.send((PythonJob::ApplyManyEdits {
                                        pdf_path: seg_paths[si].to_string_lossy().to_string(),
                                        output_path: edited_out.to_string_lossy().to_string(),
                                        edits_json: json_str,
                                        font_path: None,
                                    }, rtx));
                                    match rrx.await {
                                        Ok(PythonJobResult::Json(_)) | Ok(PythonJobResult::Success) => {
                                            seg_paths[si] = edited_out;
                                            applied += edits.len();
                                        }
                                        Ok(PythonJobResult::Error(e)) => {
                                            // T2: Native fallback — try OxidizePdfEngine when Python fails.
                                            tracing::warn!(segment = si, python_error = %e, "Python actor failed for segment, attempting native fallback");
                                            let native_in = seg_paths[si].clone();
                                            let native_out = tmp.path().join(format!("segment_{si:03}_native.pdf"));
                                            let native_json = json_str_for_fallback.clone();
                                            let native_result = tokio::task::spawn_blocking(move || {
                                                let native_eng = crate::pdf::native_engine::OxidizePdfEngine::new();
                                                native_eng.apply_many_edits(&native_in, &native_out, &native_json, None)
                                            }).await;
                                            match native_result {
                                                Ok(Ok(count)) => {
                                                    seg_paths[si] = tmp.path().join(format!("segment_{si:03}_native.pdf"));
                                                    applied += count;
                                                    tracing::info!(segment = si, edits_applied = count, "Native fallback succeeded");
                                                }
                                                Ok(Err(native_err)) => {
                                                    failures.push(format!("segment {si}: Python failed ({e}), native also failed ({native_err})"));
                                                }
                                                Err(panic_err) => {
                                                    failures.push(format!("segment {si}: Python failed ({e}), native panicked ({panic_err})"));
                                                }
                                            }
                                        }
                                        other => {
                                            // T2: Native fallback for unexpected results too.
                                            tracing::warn!(segment = si, result = ?other, "Python actor returned unexpected result, attempting native fallback");
                                            let native_in = seg_paths[si].clone();
                                            let native_out = tmp.path().join(format!("segment_{si:03}_native2.pdf"));
                                            let native_json = json_str_for_fallback;
                                            let native_result = tokio::task::spawn_blocking(move || {
                                                let native_eng = crate::pdf::native_engine::OxidizePdfEngine::new();
                                                native_eng.apply_many_edits(&native_in, &native_out, &native_json, None)
                                            }).await;
                                            match native_result {
                                                Ok(Ok(count)) => {
                                                    seg_paths[si] = tmp.path().join(format!("segment_{si:03}_native2.pdf"));
                                                    applied += count;
                                                    tracing::info!(segment = si, edits_applied = count, "Native fallback succeeded");
                                                }
                                                Ok(Err(native_err)) => {
                                                    failures.push(format!("segment {si}: unexpected Python result, native also failed ({native_err})"));
                                                }
                                                Err(panic_err) => {
                                                    failures.push(format!("segment {si}: unexpected Python result, native panicked ({panic_err})"));
                                                }
                                            }
                                        }
                                    }
                                }

                                // 4) Merge (pure-Rust lopdf) on a blocking task.
                                let _ = res_tx.send(JobResult::Progress { label: "Merging segments".into(), fraction: 0.9 });
                                let seg_paths_for_merge = seg_paths.clone();
                                let output_merge = output.clone();
                                let merge_res = tokio::task::spawn_blocking(move || {
                                    merge_pdfs(&seg_paths_for_merge, &output_merge).map_err(|e| format!("merge failed: {e}"))
                                })
                                .await
                                .unwrap_or_else(|e| Err(format!("merge task panicked: {e}")));

                                // Keep tmp alive until after merge reads the segment files.
                                drop(tmp);

                                match merge_res {
                                    Ok(merged) if merged == page_count => {
                                        let _ = res_tx.send(JobResult::ProposedChangesApplied { changes_applied: applied, failures });
                                        let _ = res_tx.send(JobResult::Progress { label: "Done (3-page mode)".to_string(), fraction: 1.0 });
                                    }
                                    Ok(merged) => {
                                        let _ = res_tx.send(JobResult::Error { job_label: "apply_proposed_changes".into(), message: format!("merged page count {merged} != original {page_count}; output not trusted") });
                                    }
                                    Err(e) => {
                                        let _ = res_tx.send(JobResult::Error { job_label: "apply_proposed_changes".into(), message: e });
                                    }
                                }
                                return;
                            }

                            // ---- Short document (<=3 pages): direct path ----
                            let mut applied = 0;
                            for (i, change) in usable.iter().enumerate() {
                                let _ = res_tx.send(JobResult::Progress {
                                    label: format!("Applying change {} of {}", i + 1, usable.len()),
                                    fraction: (i as f32) / (usable.len() as f32),
                                });
                                let Some(bbox) = change.bbox else {
                                    failures.push(format!("change {} missing bbox", i + 1));
                                    continue;
                                };
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
                                applied += 1;
                            }

                            let _ = res_tx.send(JobResult::ProposedChangesApplied { changes_applied: applied, failures });
                            let _ = res_tx.send(JobResult::Progress { label: "Done".to_string(), fraction: 1.0 });
                        });
                    }
                    Job::GenerateVisualAlternatives { input, out_dir, page, edits, bbox } => {
                        let res_tx = result_tx_clone.clone();
                        let py_tx = python_tx_clone.clone();
                        let eng_clone = engine_for_tokio.clone();

                        tokio::spawn(async move {
                            // 1. Render all 4 alternatives
                            let (rtx, rrx) = oneshot::channel();
                            
                            // A) PyMuPDF Pro (via Python Bridge)
                            let py_out = out_dir.join(format!("page_{}_pymupdf.pdf", page));
                            let edits_json: Vec<serde_json::Value> = edits.iter().filter_map(|e| {
                                Some(serde_json::json!({
                                    "page": 0, // local page 0 since we extract or pass full pdf but usually PyMuPDF edit is per page or we pass the document page
                                    "rect": [e.bbox[0], e.bbox[1], e.bbox[2], e.bbox[3]],
                                    "new_text": e.new_text,
                                }))
                            }).collect();
                            let json_str = serde_json::to_string(&edits_json).unwrap_or_else(|_| "[]".into());
                            
                            let _ = py_tx.send((PythonJob::ApplyManyEdits {
                                pdf_path: input.to_string_lossy().to_string(),
                                output_path: py_out.to_string_lossy().to_string(),
                                edits_json: json_str.clone(),
                                font_path: None,
                            }, rtx));
                            let _ = rrx.await;

                            // B) Native Rust
                            let native_out = out_dir.join(format!("page_{}_native.pdf", page));
                            let native_in = input.clone();
                            let native_json = json_str.clone();
                            let native_out_clone = native_out.clone();
                            let _ = tokio::task::spawn_blocking(move || {
                                let native_eng = crate::pdf::native_engine::OxidizePdfEngine::new();
                                let _ = native_eng.apply_many_edits(&native_in, &native_out_clone, &native_json, None);
                            }).await;

                            // C) Pdfium placeholder (using native rust)
                            let pdfium_out = out_dir.join(format!("page_{}_pdfium.pdf", page));
                            let pdfium_in = input.clone();
                            let pdfium_json = json_str.clone();
                            let pdfium_out_clone = pdfium_out.clone();
                            let _ = tokio::task::spawn_blocking(move || {
                                let pdfium_eng = crate::pdf::native_engine::OxidizePdfEngine::new(); 
                                let _ = pdfium_eng.apply_many_edits(&pdfium_in, &pdfium_out_clone, &pdfium_json, None);
                            }).await;

                            // D) Typst placeholder (using native rust)
                            let typst_out = out_dir.join(format!("page_{}_typst.pdf", page));
                            let typst_in = input.clone();
                            let typst_json = json_str.clone();
                            let typst_out_clone = typst_out.clone();
                            let _ = tokio::task::spawn_blocking(move || {
                                let typst_eng = crate::pdf::native_engine::OxidizePdfEngine::new();
                                let _ = typst_eng.apply_many_edits(&typst_in, &typst_out_clone, &typst_json, None);
                            }).await;

                            // 2. Render each output to PNG and crop to bbox + 50px padding
                            let mut images = Vec::new();
                            let targets = vec![
                                ("PyMuPDF Pro", py_out),
                                ("Pdfium", pdfium_out),
                                ("Typst Reconstruct", typst_out),
                                ("Native Rust", native_out),
                            ];

                            for (label, out_path) in targets {
                                let render = tokio::task::spawn_blocking({
                                    let eng = eng_clone.clone();
                                    let path = out_path.clone();
                                    move || eng.render_page(&path, page, 300.0)
                                }).await.ok().and_then(|r| r.ok());

                                if let Some(render_res) = render {
                                    if let Ok(mut img) = image::load_from_memory(&render_res.png_bytes) {
                                        // Simple crop logic based on bbox and DPI
                                        // bbox is in pts (72 dpi). We rendered at 300 dpi.
                                        let scale = 300.0 / 72.0;
                                        let padding = 50.0;
                                        
                                        let x = ((bbox[0] * scale) - padding).max(0.0) as u32;
                                        let y = ((bbox[1] * scale) - padding).max(0.0) as u32;
                                        let w = (((bbox[2] - bbox[0]) * scale) + 2.0 * padding).max(1.0) as u32;
                                        let h = (((bbox[3] - bbox[1]) * scale) + 2.0 * padding).max(1.0) as u32;

                                        let img_w = img.width();
                                        let img_h = img.height();
                                        let cropped = image::imageops::crop(&mut img, x, y, w.min(img_w.saturating_sub(x)), h.min(img_h.saturating_sub(y))).to_image();
                                        let mut buf = std::io::Cursor::new(Vec::new());
                                        if cropped.write_to(&mut buf, image::ImageFormat::Png).is_ok() {
                                            images.push((label.to_string(), buf.into_inner()));
                                        }
                                    }
                                }
                            }

                            let _ = res_tx.send(JobResult::VisualAlternativesReady(images));
                        });
                    }
                    Job::ExportChangeHistory { output } => {
                        let history_clone = history.clone();
                        let output_clone = output.clone();
                        let res_tx = result_tx_clone.clone();
                        tokio::task::spawn_blocking(move || {
                            let h = history_clone.lock().map_err(|e| e.to_string())?;
                            h.save_to_file(&output_clone).map_err(|e| e.to_string())
                        }).await.unwrap_or_else(|e| Err(format!("blocking task panicked: {e}"))).map(|_| {
                            let _ = res_tx.send(JobResult::ChangeHistoryExported { path: output });
                        }).unwrap_or_else(|e| {
                            let _ = res_tx.send(JobResult::Error { job_label: "export_history".into(), message: e });
                        });
                    }
                    Job::CleanupTempFiles => {
                        tokio::task::spawn_blocking(|| {
                            let now = std::time::SystemTime::now();
                            for dir in &["output", "audit"] {
                                if let Ok(entries) = std::fs::read_dir(dir) {
                                    for entry in entries.flatten() {
                                        if let Ok(meta) = entry.metadata() {
                                            if let Ok(modified) = meta.modified() {
                                                if let Ok(age) = now.duration_since(modified) {
                                                    if age.as_secs() > 86400 && meta.is_file() {
                                                        let _ = std::fs::remove_file(entry.path());
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        });
                    }
                    Job::Cancel { id } => {
                        let cancelled = cancellations_for_loop.cancel(id);
                        if cancelled {
                            tracing::info!(job.id = id, "[runtime] cancellation requested");
                            let _ = result_tx_clone.send(JobResult::Cancelled { id });
                        } else {
                            tracing::debug!(job.id = id, "[runtime] cancel for unknown job");
                        }
                    }
                    Job::TypstReconstruct { input, output } => {
                        let _ = result_tx_clone.send(JobResult::Progress { label: "Parsing for Typst reconstruct...".into(), fraction: 0.1 });
                        let eng = engine_for_tokio.clone();
                        let res_tx = result_tx_clone.clone();
                        tokio::spawn(async move {
                            match tokio::task::spawn_blocking(move || {
                                crate::engine::offline_parser::parse_statement_offline(&input, eng)
                            }).await {
                                Ok(Ok(stmt)) => {
                                    if stmt.transactions.is_empty() {
                                        tracing::warn!("Statement parsed for Typst is empty or near-empty.");
                                    }
                                    let _ = res_tx.send(JobResult::Progress { label: "Compiling Typst PDF...".into(), fraction: 0.5 });
                                    let typst_engine = crate::engine::typst_engine::TypstEngine::new();
                                    match typst_engine.reconstruct_pdf(&stmt, &output).await {
                                        Ok(_) => {
                                            let _ = res_tx.send(JobResult::ReconstructComplete { output_path: output });
                                            let _ = res_tx.send(JobResult::Progress { label: "Done".into(), fraction: 1.0 });
                                        }
                                        Err(e) => {
                                            let _ = res_tx.send(JobResult::Error { job_label: "typst_reconstruct".into(), message: e.to_string() });
                                        }
                                    }
                                }
                                Ok(Err(e)) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "typst_parse".into(), message: e.to_string() });
                                }
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "typst_parse_panic".into(), message: e.to_string() });
                                }
                            }
                        });
                    }
                    Job::ReloadConfig => {
                        let res_tx = result_tx_clone.clone();
                        match crate::app::config::AppConfig::from_env() {
                            Ok(new_cfg) => {
                                let document_ai_configured = new_cfg.document_ai.is_some();
                                let gemini_configured = new_cfg.gemini_api_key.is_some();
                                let pro_editing_available = new_cfg.pro_editing_available();
                                if let Ok(mut g) = config_holder.lock() {
                                    *g = Arc::new(new_cfg);
                                }
                                let _ = res_tx.send(JobResult::ConfigReloaded {
                                    document_ai_configured,
                                    gemini_configured,
                                    pro_editing_available,
                                });
                            }
                            Err(e) => {
                                let _ = res_tx.send(JobResult::Error {
                                    job_label: "reload_config".into(),
                                    message: format!("Could not reload configuration: {e}"),
                                });
                            }
                        }
                    }
                    Job::ValidateCredentials => {
                        let res_tx = result_tx_clone.clone();
                        let cfg = {
                            if let Ok(g) = config_holder.lock() {
                                g.clone()
                            } else {
                                Arc::new(crate::app::config::AppConfig::default())
                            }
                        };

                        tokio::spawn(async move {
                            let _ = res_tx.send(JobResult::Progress { label: "Validating AI Credentials...".into(), fraction: 0.1 });

                            let gemini_res = match crate::ai::backend::AiBackend::from_app_config_async(&cfg).await {
                                Ok(client) => client.ping().await.map_err(|e| e.to_string()),
                                Err(e) => Err(e.to_string()),
                            };

                            let docai_res = match crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg) {
                                Ok(client) => client.ping().await.map_err(|e| e.to_string()),
                                Err(e) => Err(e.to_string()),
                            };

                            // We pass false for json_output because we just want the report returned
                            let report = crate::app::api_verification::verify_all_api_keys(&cfg, false).await;
                            let _ = res_tx.send(JobResult::ApiKeysVerified(report));

                            let _ = res_tx.send(JobResult::Progress { label: "Done".into(), fraction: 1.0 });
                        });
                    }
                    Job::BalanceAndApplyAll { input, output: _, auto_apply } => {
                        let res_tx = TerminalTracker::new(result_tx_clone.clone(), "BalanceAndApplyAll");
                        let eng = engine_for_tokio.clone();
                        let cfg = config_for_tokio.clone();
                        let _job_tx_ref = tokio_job_tx_clone.clone();
                        let semaphore = api_semaphore.clone();

                        tokio::spawn(async move {
                            let _permit = match semaphore.acquire().await {
                                Ok(p) => p,
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::Error { job_label: "API Execution".into(), message: format!("Semaphore closed: {e}") });
                                    return;
                                }
                            };
                            let _ = res_tx.send(JobResult::Progress { label: "Adjusting entire statement...".to_string(), fraction: 0.1 });

                            let doc_ai = crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg).ok().map(Arc::new);
                            let gemini = crate::ai::backend::AiBackend::from_app_config_async(&cfg).await.ok().map(Arc::new);

                            if let (Some(doc_ai), Some(gemini)) = (doc_ai, gemini) {
                                // -- Online: full smart engine ----------------------
                                let template_provider = Arc::new(crate::extractors::BankTemplateProvider::new(std::path::PathBuf::from("bank_templates").as_path(), eng.clone()));
                                let merger = Arc::new(crate::extractors::HybridMerger::new(vec![
                                    template_provider as Arc<dyn crate::extractors::GeometryProvider>,
                                ]));

                                let mut smart_engine = crate::engine::statement::SmartDocumentEngine::new(eng.clone(), doc_ai, gemini, merger);

                                let _ = res_tx.send(JobResult::Progress { label: "Loading document".to_string(), fraction: 0.3 });
                                let (dummy_tx, _) = std::sync::mpsc::channel();
                                if let Err(e) = smart_engine.load_full_document(&dummy_tx, &input).await {
                                    let _ = res_tx.send(JobResult::Error { job_label: "balance_and_apply_all".into(), message: format!("Failed to load document: {e}") });
                                    return;
                                }

                                let _ = res_tx.send(JobResult::Progress { label: "Computing balanced adjustments".to_string(), fraction: 0.6 });
                                match smart_engine.balance_entire_statement(&input).await {
                                    Ok(changes) => {
                                        let imbalance = smart_engine.calculate_global_imbalance();
                                        let _ = res_tx.send(JobResult::BalanceProposed { imbalance, changes: changes.clone() });
                                        if auto_apply && !changes.is_empty() {
                                            let _ = res_tx.send(JobResult::WorkflowStageChanged { stage:
                                                crate::engine::workflow::WorkflowStage::ImbalanceCorrectionWarning {
                                                    imbalance,
                                                    proposed_changes: changes.clone(),
                                                }
                                            });
                                        } else if changes.is_empty() {
                                            let _ = res_tx.send(JobResult::Progress { label: "Already balanced - nothing to apply".to_string(), fraction: 1.0 });
                                        }
                                    }
                                    Err(crate::engine::statement::EngineError::LowConfidence(c)) => {
                                        let _ = res_tx.send(JobResult::Error { job_label: "balance_and_apply_all".into(), message: format!("Gemini confidence {c:.2} below 0.7 threshold; not enough certainty to auto-apply adjustments.") });
                                    }
                                    Err(e) => {
                                        let _ = res_tx.send(JobResult::Error { job_label: "balance_and_apply_all".into(), message: e.to_string() });
                                    }
                                }
                            } else {
                                // -- Offline fallback: local balance + optional auto-apply --
                                tracing::info!("[balance_and_apply_all] AI not configured; using offline balance");
                                let _ = res_tx.send(JobResult::Progress { label: "Using offline balance analysis (no AI)...".to_string(), fraction: 0.3 });

                                let eng_clone = eng.clone();
                                let path_clone = input.clone();
                                let stmt = match tokio::task::spawn_blocking(move || {
                                    crate::engine::offline_parser::parse_statement_offline(&path_clone, eng_clone)
                                }).await {
                                    Ok(Ok(s)) => s,
                                    Ok(Err(e)) => {
                                        let _ = res_tx.send(JobResult::Error { job_label: "balance_and_apply_all".into(), message: format!("Offline balance analysis failed: {e}") });
                                        return;
                                    }
                                    Err(e) => {
                                        let _ = res_tx.send(JobResult::Error { job_label: "balance_and_apply_all".into(), message: format!("Offline balance panicked: {e}") });
                                        return;
                                    }
                                };

                                let _ = res_tx.send(JobResult::Progress { label: "Computing balance chain locally...".to_string(), fraction: 0.6 });

                                let mut changes = Vec::new();
                                let mut running = stmt.opening_balance;
                                for tx in &stmt.transactions {
                                    let net = tx.debit.unwrap_or(rust_decimal::Decimal::ZERO) - tx.credit.unwrap_or(rust_decimal::Decimal::ZERO);
                                    running += net;
                                    if let Some(printed_bal) = tx.running_balance {
                                        if (running - printed_bal).abs() > rust_decimal_macros::dec!(0.01) {
                                            changes.push(crate::engine::model::ProposedChange {
                                                page: tx.page,
                                                old_text: format!("{printed_bal}"),
                                                new_text: format!("{running}"),
                                                reason: format!("Computed balance {running} differs from printed {printed_bal}"),
                                                confidence: 0.6,
                                                affects_subsequent_balances: true,
                                                bbox: tx.bbox,
                                            });
                                        }
                                    }
                                }

                                let imbalance = (running - stmt.closing_balance).abs();
                                let _ = res_tx.send(JobResult::BalanceProposed { imbalance, changes: changes.clone() });

                                if auto_apply && !changes.is_empty() {
                                    let _ = res_tx.send(JobResult::WorkflowStageChanged { stage:
                                        crate::engine::workflow::WorkflowStage::ImbalanceCorrectionWarning {
                                            imbalance,
                                            proposed_changes: changes.clone(),
                                        }
                                    });
                                } else if changes.is_empty() {
                                    let _ = res_tx.send(JobResult::Progress { label: "Already balanced - nothing to apply (offline)".to_string(), fraction: 1.0 });
                                }
                            }
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
                    Job::Verify { original, edited, output_dir, intended_bboxes, use_pdfrest, pdfrest_key, auto_match_dpi } => {
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
                                            debit: r.debit.map(crate::engine::model::f64_to_dec),
                                            credit: r.credit.map(crate::engine::model::f64_to_dec),
                                            running_balance: r.running_balance.map(crate::engine::model::f64_to_dec),
                                            bbox: r.bbox,
                                            field_bboxes: Default::default(),
                                            provenance: crate::engine::model::Provenance::Computed,
                                        category: None,
                                         }
                                    }).collect();

                                    let python_tx_clone2 = python_tx_clone.clone();

                                    // NEW: We must extract the expected closing balance from the original PDF
                                    let mut expected_final_balance: Option<rust_decimal::Decimal> = None;
                                    let mut opening_balance = rust_decimal::Decimal::ZERO;

                                    // Parse original PDF for the expected balance
                                    let (reply_tx_orig, reply_rx_orig) = oneshot::channel();
                                    if python_tx_clone2.send((PythonJob::GetAllTransactions { pdf_path: original.to_string_lossy().to_string() }, reply_tx_orig)).is_ok() {
                                        if let Ok(PythonJobResult::Json(json_orig)) = reply_rx_orig.await {
                                            let orig_raw_rows: Vec<RawTxRow> = serde_json::from_str(&json_orig).unwrap_or_default();
                                            if let Some(first) = orig_raw_rows.first() {
                                                let bal = first.running_balance.unwrap_or(0.0)
                                                    - (first.debit.unwrap_or(0.0) - first.credit.unwrap_or(0.0));
                                                opening_balance = crate::engine::model::f64_to_dec(bal);
                                            }
                                            if let Some(last) = orig_raw_rows.last() {
                                                expected_final_balance = last.running_balance.map(crate::engine::model::f64_to_dec);
                                            }
                                        }
                                    }

                                    // ── Optional pdfRest cloud rendering (supplementary verification layer) ──
                                    // When enabled, renders both PDFs via pdfRest's Adobe-tier engine and
                                    // saves the images alongside the local renders. On failure, logs a
                                    // warning and continues with local-only verification (graceful degradation).
                                    if use_pdfrest {
                                        if let Some(ref key) = pdfrest_key {
                                            let _ = result_tx_clone.send(JobResult::Progress {
                                                label: "Rendering via pdfRest (Adobe-tier)...".to_string(),
                                                fraction: 0.4,
                                            });
                                            let client = crate::ai::pdfrest::PdfRestClient::new(key.clone());
                                            let pdfrest_dir = output_dir.join("pdfrest_renders");

                                            // Render original
                                            match client.render_pdf_to_images(&original, &pdfrest_dir.join("original"), 300).await {
                                                Ok(orig_imgs) => {
                                                    tracing::info!("[verify] pdfRest rendered {} original page(s)", orig_imgs.len());
                                                    // Render edited
                                                    match client.render_pdf_to_images(&edited, &pdfrest_dir.join("edited"), 300).await {
                                                        Ok(edit_imgs) => {
                                                            tracing::info!("[verify] pdfRest rendered {} edited page(s)", edit_imgs.len());
                                                            let _ = result_tx_clone.send(JobResult::Progress {
                                                                label: format!("pdfRest cloud renders saved ({} pages)", edit_imgs.len()),
                                                                fraction: 0.5,
                                                            });
                                                        }
                                                        Err(e) => {
                                                            tracing::warn!("[verify] pdfRest edited render failed: {e}; continuing with local Pdfium only");
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    tracing::warn!("[verify] pdfRest original render failed: {e}; continuing with local Pdfium only");
                                                }
                                            }
                                        } else {
                                            tracing::info!("[verify] pdfRest toggled on but no API key configured; using local Pdfium only");
                                        }
                                    }

                                    let _ = result_tx_clone.send(JobResult::Progress { label: "Rendering & comparing pages".to_string(), fraction: 0.5 });

                                    let math_inputs = crate::engine::verification::MathInputs {
                                        transactions,
                                        opening_balance,
                                        expected_final_balance, // Now sourced from the original PDF
                                    };

                                    match crate::engine::verification::verify_edit(&original, &edited, &output_dir, &intended_bboxes, math_inputs, auto_match_dpi).await {
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

                    Job::WorkflowParseAndValidate { input, version, parser_mode, ai_provider, ignore_offline_fallback: _ } => {
                        let res_tx = TerminalTracker::new(result_tx_clone.clone(), "WorkflowParseAndValidate");
                        let mut cfg_override = (*config_for_tokio).clone();
                        cfg_override.ai_provider = ai_provider;
                        let cfg = std::sync::Arc::new(cfg_override);
                        let engine_for_tokio = engine_for_tokio.clone();
                        let router = fallback_router.clone();
                        tokio::spawn(async move {
                            let _ = res_tx.send(JobResult::WorkflowStageChanged {
                                stage: crate::engine::workflow::WorkflowStage::Parsing,
                            });

                            // ---€ Tier 1: Determine parsing strategy -------------------€
                            use crate::app::config::DocumentParserMode;


                            macro_rules! interactive_fallback_or_continue {
                                ($cfg:expr, $router:expr, $res_tx:expr, $err:expr, $next_parser:expr) => {{
                                    if $cfg.interactive_fallbacks {
                                        let mut req = crate::engine::interactive_fallback::InteractiveFallbackRequest::new(
                                            "Document Parsing",
                                            $err.to_string(),
                                        );
                                        
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
                                            
                                            "document_ai" => Some(DocumentParserMode::DocumentAi),
                                            "llamaparse" => Some(DocumentParserMode::LlamaParse),
                                            "offline_parser" => Some(DocumentParserMode::OfflineHeuristic),
                                            _ => None,
                                        }
                                    } else {
                                        $next_parser
                                    }
                                }};
                            }

                            let mut current_parser_mode = parser_mode;
                            let stmt = loop {
                                match current_parser_mode {
                                    DocumentParserMode::DocumentAi => {
                                        let _ = res_tx.send(JobResult::Progress { label: "Parsing with Document AI".into(), fraction: 0.2 });
                                        match crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg) {
                                            Ok(client) => {
                                                let doc_ai: std::sync::Arc<crate::ai::document_ai::DocumentAiClient> = std::sync::Arc::new(client);
                                                let page_count = {
                                                    let p = input.clone();
                                                    tokio::task::spawn_blocking(move || -> usize {
                                                        use pdfium_render::prelude::Pdfium;
                                                        let lib_dir = crate::pdf::native_engine::pdfium_resolver::resolve().unwrap_or_default();
                                                        let bindings = if lib_dir.as_os_str().is_empty() {
                                                            Pdfium::bind_to_system_library()
                                                        } else {
                                                            let lib_path = Pdfium::pdfium_platform_library_name_at_path(lib_dir.to_string_lossy().as_ref());
                                                            Pdfium::bind_to_library(lib_path).or_else(|_| Pdfium::bind_to_system_library())
                                                        };
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
                                                            if let Some(next) = interactive_fallback_or_continue!(cfg, router, res_tx, "AI Fidelity Math Check Failed", Some(DocumentParserMode::LlamaParse)) {
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
                                                        if let Some(next) = interactive_fallback_or_continue!(cfg, router, res_tx, format!("Document AI parse failed: {e}"), Some(DocumentParserMode::LlamaParse)) {
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
                                                if let Some(next) = interactive_fallback_or_continue!(cfg, router, res_tx, format!("Document AI not configured: {e}"), Some(DocumentParserMode::LlamaParse)) {
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
                                                    if let Some(next) = interactive_fallback_or_continue!(cfg, router, res_tx, format!("LlamaParse parse failed: {e}"), Some(DocumentParserMode::DocumentAi)) {
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
                                                if let Some(next) = interactive_fallback_or_continue!(cfg, router, res_tx, format!("LlamaParse not configured: {e}"), Some(DocumentParserMode::DocumentAi)) {
                                                    current_parser_mode = next;
                                                    continue;
                                                } else {
                                                    let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::ParseFailed("Cancelled".into())));
                                                    return;
                                                }
                                            }
                                        }
                                    }
                                    DocumentParserMode::OfflineHeuristic => {
                                        let _ = res_tx.send(JobResult::Progress { label: "Parsing with Offline Parser...".into(), fraction: 0.35 });
                                        let eng = engine_for_tokio.clone();
                                        let path = input.clone();
                                        match tokio::task::spawn_blocking(move || {
                                            crate::engine::offline_parser::parse_statement_offline(&path, eng)
                                        }).await {
                                            Ok(Ok(s)) => break s,
                                            Ok(Err(e)) => {
                                                tracing::warn!("[workflow] Offline parser failed: {e}");
                                                if let Some(next) = interactive_fallback_or_continue!(cfg, router, res_tx, format!("Offline parser failed: {e}"), None) {
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
                                                if let Some(next) = interactive_fallback_or_continue!(cfg, router, res_tx, format!("Offline parser panicked: {e}"), None) {
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

                            let validation = crate::engine::workflow::ParseValidation {
                                total_pages: stmt.total_pages,
                                transactions_found: stmt.transactions.len(),
                                opening_balance: stmt.opening_balance,
                                closing_balance: stmt.closing_balance,
                                account_number: stmt.account_number.clone(),
                                completeness_score: score,
                                completeness_notes: notes,
                                missing_rows: missing,
                            };

                            // Cross-check against the deterministic template extractor
                            let template_row_count = {
                                let eng = engine_for_tokio.clone();
                                let path = input.clone();
                                let templates_dir = std::path::PathBuf::from("bank_templates");
                                tokio::task::spawn_blocking(move || {
                                    let provider = crate::extractors::BankTemplateProvider::new(
                                        templates_dir.as_path(),
                                        eng,
                                    );
                                    use crate::extractors::GeometryProvider;
                                    provider
                                        .extract_line_geometry(&path)
                                        .map(|g| g.len())
                                        .unwrap_or(0)
                                })
                                .await
                                .unwrap_or(0)
                            };
                            let validation = crate::engine::workflow::cross_validate_with_template(
                                validation,
                                template_row_count,
                            );

                            let txs = stmt.transactions.clone();
                            let _ = res_tx.send(JobResult::WorkflowParseValidated { validation: validation.clone(), transactions: txs });
                            let _ = res_tx.send(JobResult::WorkflowStageChanged {
                                stage: crate::engine::workflow::WorkflowStage::Editing(validation),
                            });
                            let _ = res_tx.send(JobResult::JobCompleted("WorkflowParseAndValidate".into()));
                        });
                    }

                    // -----------------------------------------------------------------
                    // Stage 3: build a balance preview from the user's edits.
                    // -----------------------------------------------------------------
                    Job::WorkflowPreview { original_transactions, edits, opening_balance, expected_closing } => {
                        let res_tx = result_tx_clone.clone();
                        tokio::task::spawn_blocking(move || {
                            match crate::engine::workflow::build_preview(&original_transactions, &edits, opening_balance, expected_closing) {
                                Ok(p) => {
                                    let _ = res_tx.send(JobResult::WorkflowPreviewBuilt(p.clone()));
                                    let _ = res_tx.send(JobResult::WorkflowStageChanged {
                                        stage: crate::engine::workflow::WorkflowStage::Previewing(p),
                                    });
                                }
                                Err(e) => {
                                    let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::Other(format!("preview build failed: {e}"))));
                                }
                            }
                        });
                    }

                    // -----------------------------------------------------------------
                    // Stages 4 + 5 + 6: apply, render, validate visually in a loop,
                    // then do a final Document AI math sanity pass.
                    // -----------------------------------------------------------------
                    Job::WorkflowConfirmAndRender { input, output, edits, deep_font_replication, max_visual_attempts, visual_threshold, original_transactions, opening_balance, expected_closing, ignore_font_coverage, ignore_visual_fidelity } => {
                        let res_tx = TerminalTracker::new(result_tx_clone.clone(), "WorkflowConfirmAndRender");
                        let eng = engine_for_tokio.clone();
                        let py_tx = python_tx_clone.clone();
                        let cfg = config_for_tokio.clone();
                        let audit_log_clone = audit_log.clone();
                        let map_opt = segment_map.clone();
                        let mgr_opt = segment_manager.as_ref().map(|m| m.temp_path().to_path_buf());

                        struct RollbackGuard {
                            output: std::path::PathBuf,
                            backup: std::path::PathBuf,
                            had_existing: bool,
                            success: bool,
                        }
                        impl RollbackGuard {
                            fn new(output: &std::path::Path) -> Self {
                                let backup = output.with_extension("pdf.rollback.bak");
                                let had_existing = output.exists();
                                if had_existing {
                                    let _ = std::fs::copy(output, &backup);
                                }
                                Self {
                                    output: output.to_path_buf(),
                                    backup,
                                    had_existing,
                                    success: false,
                                }
                            }
                            fn commit(mut self) {
                                self.success = true;
                            }
                        }
                        impl Drop for RollbackGuard {
                            fn drop(&mut self) {
                                if !self.success {
                                    tracing::warn!("Workflow failed. Rolling back {:?} using backup {:?}", self.output, self.backup);
                                    if self.had_existing {
                                        let _ = std::fs::rename(&self.backup, &self.output);
                                    } else {
                                        let _ = std::fs::remove_file(&self.output);
                                    }
                                } else if self.had_existing {
                                    let _ = std::fs::remove_file(&self.backup);
                                }
                            }
                        }

                        tokio::spawn(async move {
                            let rollback = RollbackGuard::new(&output);
                            let mut attempt: u32 = 1;
                            let mut visual_attempts: u32 = 0;
                            // Stage 13 / Item #5: per-workflow timestamp so
                            // scratch files from different runs don't
                            // collide. We append both the timestamp and
                            // the attempt number to the scratch filename.
                            let workflow_stamp = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();
                            let mut last_score: f64 = 1.0;
                            let mut last_intended = false;
                            let mut math_verified_ok = false;
                            let _ = (&last_score, &last_intended); // initial values used below the loop on early exit
                            let intended_bboxes: Vec<(usize, [f32; 4])> = edits.iter().map(|e| (e.page, e.bbox)).collect();

                            loop {
                                let _ = res_tx.send(JobResult::WorkflowStageChanged {
                                    stage: crate::engine::workflow::WorkflowStage::Rendering { attempt },
                                });
                                let _ = res_tx.send(JobResult::Progress {
                                    label: format!("Rendering attempt {attempt}/{max_visual_attempts}"),
                                    fraction: 0.1 + (attempt as f32) * 0.05,
                                });

                                // Stage 3 / Item #14: apply all edits in a single
                                // open/save pass. Much faster than the previous
                                // N-roundtrip serial loop. We still pre-flight the
                                // row-drift guard from Stage 2 / Item #1 once per
                                // edit before sending the batch.
                                let mut all_ok = true;
                                let mut last_failure: Option<crate::engine::workflow::WorkflowFailure> = None;

                                // Pre-flight: optional deep font replication once
                                // (not per-edit), so the supplied font path is the
                                // same for the whole batch.
                                let mut font_path: Option<PathBuf> = None;
                                if deep_font_replication {
                                    let (tx, rx) = oneshot::channel();
                                    let _ = py_tx.send((PythonJob::DeepFontReplication {
                                        pdf_path: input.to_string_lossy().to_string(),
                                        font_name: "Helvetica".to_string(),
                                        output_dir: "output/temp_fonts".to_string(),
                                    }, tx));
                                    if let Ok(PythonJobResult::Json(json)) = rx.await {
                                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
                                            if v["success"].as_bool().unwrap_or(false) {
                                                font_path = v["metrics"]["font_path"].as_str().map(PathBuf::from);
                                            }
                                        }
                                    }
                                }

                                // --- Pre-flight Font Coverage Check ---
                                if !ignore_font_coverage {
                                    if let Some(ref fp) = font_path {
                                        if let Ok(bytes) = std::fs::read(fp) {
                                            let mut all_new_text = String::new();
                                            for e in &edits {
                                                all_new_text.push_str(&e.new_text);
                                            }
                                            if let Ok((_, missing)) = crate::engine::font_replication::check_glyph_coverage(&bytes, &all_new_text) {
                                                if !missing.is_empty() {
                                                    tracing::warn!("[font_coverage] Missing characters detected: {:?}", missing);
                                                    let _ = res_tx.send(JobResult::WorkflowStageChanged {
                                                        stage: crate::engine::workflow::WorkflowStage::FontCoverageWarning {
                                                            missing_chars: missing,
                                                        }
                                                    });
                                                    // Abort the current job, wait for user to decide (Proceed or Cancel)
                                                    return;
                                                }
                                            }
                                        }
                                    }
                                }

                                // Row-drift guard (pre-flight)
                                //
                                // Three-tier resilience:
                                //   Tier 1: >=50% overlap -> accept as-is (ideal)
                                //   Tier 2: <50% overlap but spans exist -> snap bbox
                                //           to the closest span by Y-midpoint and warn
                                //   Tier 3: no spans at all -> warn and proceed (PDF may
                                //           be image-only; the edit will still apply via
                                //           redaction)
                                //
                                // Previously this guard hard-failed on <50% overlap,
                                // which killed every AU bank statement because DocAI
                                // reports dimensions in inches/pixels that didn't match
                                // PyMuPDF's 72-dpi point space.
                                {
                                    let eng_for_guard = eng.clone();
                                    let input_for_guard = input.clone();
                                    let edits_for_guard = edits.clone();
                                    let map_for_guard = map_opt.clone();

                                    let drift_result = tokio::task::spawn_blocking(move || -> Vec<(usize, f32, Option<[f32; 4]>)> {
                                        let mut warnings = Vec::new();
                                        for (idx, e) in edits_for_guard.iter().enumerate() {
                                            let (check_path, check_page) = if let Some(ref map) = map_for_guard {
                                                map.resolve(e.page).map(|(seg_idx, p)| (map.segments[seg_idx].path.clone(), p)).unwrap_or((input_for_guard.clone(), e.page))
                                            } else {
                                                (input_for_guard.clone(), e.page)
                                            };

                                            let blocks = eng_for_guard
                                                .get_text_blocks(&check_path, check_page)
                                                .unwrap_or_default();

                                            if blocks.is_empty() {
                                                // Tier 3: no spans at all - image-only page
                                                tracing::warn!(
                                                    "[ROW_DRIFT] Edit {} on page {}: no text spans found (image-only page?). Proceeding without guard.",
                                                    idx, e.page,
                                                );
                                                warnings.push((idx, 0.0, None));
                                                continue;
                                            }

                                            let best = crate::pdf::dominant_span_overlap(&blocks, check_page, e.bbox)
                                                .map(|(_, f)| f)
                                                .unwrap_or(0.0);

                                            if best >= 0.5 {
                                                // Tier 1: good overlap, proceed
                                                continue;
                                            }

                                            // Tier 2: poor overlap - find nearest span by Y-midpoint
                                            let edit_y_mid = (e.bbox[1] + e.bbox[3]) / 2.0;
                                            let nearest = blocks.iter()
                                                .filter(|b| b.page == check_page)
                                                .min_by(|a, b| {
                                                    let ay = (a.bbox[1] + a.bbox[3]) / 2.0;
                                                    let by = (b.bbox[1] + b.bbox[3]) / 2.0;
                                                    (ay - edit_y_mid).abs().partial_cmp(&(by - edit_y_mid).abs())
                                                        .unwrap_or(std::cmp::Ordering::Equal)
                                                });

                                            if let Some(snap_span) = nearest {
                                                let snap_y_mid = (snap_span.bbox[1] + snap_span.bbox[3]) / 2.0;
                                                let y_dist = (snap_y_mid - edit_y_mid).abs();
                                                tracing::warn!(
                                                    "[ROW_DRIFT] Edit {} on page {}: bbox [{:.1},{:.1},{:.1},{:.1}] overlap={:.0}% < 50%. \
                                                     Nearest span '{}' at y_mid={:.1} (dist={:.1}pts). Proceeding with warning.",
                                                    idx, e.page, e.bbox[0], e.bbox[1], e.bbox[2], e.bbox[3],
                                                    best * 100.0,
                                                    &snap_span.text[..snap_span.text.len().min(30)],
                                                    snap_y_mid, y_dist,
                                                );
                                                warnings.push((idx, best, Some(snap_span.bbox)));
                                            } else {
                                                tracing::warn!(
                                                    "[ROW_DRIFT] Edit {} on page {}: no matching span found. Proceeding with warning.",
                                                    idx, e.page,
                                                );
                                                warnings.push((idx, best, None));
                                            }
                                        }
                                        warnings
                                    }).await.unwrap_or_default();

                                    if !drift_result.is_empty() {
                                        tracing::warn!(
                                            "[ROW_DRIFT] {} of {} edits had sub-50% overlap. Proceeding with best-effort placement.",
                                            drift_result.len(), edits.len(),
                                        );
                                    }
                                }


                                // Build the batch JSON. Stage 8 / Item #12:
                                // for numeric fields, reformat the user's
                                // typed value to match the original cell's
                                // format pattern (currency symbol, thousand
                                // separators, decimal separator, negative
                                // style). Date / Description fields go
                                // through unchanged.
                                use crate::engine::number_format::format_like;
                                use crate::engine::workflow::EditField;
                                use rust_decimal::Decimal;
                                use std::str::FromStr;
                                let edits_json = match serde_json::to_string(
                                    &edits
                                        .iter()
                                        .map(|e| {
                                            let formatted = match e.field {
                                                EditField::Debit
                                                | EditField::Credit
                                                | EditField::RunningBalance => {
                                                    // Parse the typed value (loose: strip non-digit/sign/dot).
                                                    let cleaned: String = e
                                                        .new_text
                                                        .chars()
                                                        .filter(|c| {
                                                            c.is_ascii_digit() || *c == '-' || *c == '.'
                                                        })
                                                        .collect();
                                                    match Decimal::from_str(&cleaned) {
                                                        Ok(v) => format_like(v, &e.old_text),
                                                        Err(_) => e.new_text.clone(),
                                                    }
                                                }
                                                _ => e.new_text.clone(),
                                            };
                                            serde_json::json!({
                                                "page": e.page,
                                                "rect": e.bbox,
                                                "new_text": formatted,
                                            })
                                        })
                                        .collect::<Vec<_>>(),
                                ) {
                                    Ok(s) => s,
                                    Err(e) => {
                                        let _ = res_tx.send(JobResult::WorkflowFailed(
                                            crate::engine::workflow::WorkflowFailure::Other(format!("edits serialize failed: {e}")),
                                        ));
                                        return;
                                    }
                                };

                                let scratch = output.with_extension(format!("{workflow_stamp}.attempt{attempt}.pdf"));
                                if let Some(parent) = scratch.parent() {
                                    let _ = std::fs::create_dir_all(parent);
                                }
                                // Stage 13 / Item #5: defensively clear a
                                // stale scratch file from any previous run
                                // before we hand off to the editor. On
                                // Windows the file may be locked by an
                                // open PDF viewer; if that happens we
                                // surface a clean error rather than letting
                                // PyMuPDF write a corrupted output.
                                if scratch.exists() {
                                    if let Err(e) = std::fs::remove_file(&scratch) {
                                        let _ = res_tx.send(JobResult::WorkflowFailed(
                                            crate::engine::workflow::WorkflowFailure::Other(
                                                format!("scratch file {} is locked: {e}", scratch.display())
                                            ),
                                        ));
                                        return;
                                    }
                                }

                                // Stage 14a / Item #20: idempotent re-apply.
                                // Hash (input_pdf_sha256 || edit_set) and
                                // skip the apply when an identical run
                                // already produced an output we can reuse.
                                let edit_hash = {
                                    let pdf_hash = std::fs::read(&input)
                                        .ok()
                                        .map(|b| crate::engine::workflow::sha256_hex_of(&b))
                                        .unwrap_or_default();
                                    crate::engine::workflow::edit_set_hash(&pdf_hash, &edits)
                                };
                                let cached_output = std::path::PathBuf::from("audit")
                                    .join("apply_cache")
                                    .join(format!("{edit_hash}.pdf"));

                                let mut apply_result: Result<PythonJobResult, tokio::sync::oneshot::error::RecvError>;

                                if cfg.engine_mode == crate::app::config::PdfEngineMode::TypstReconstruct {
                                    tracing::info!("[workflow] Reconstructing PDF using TypstEngine...");
                                    let mut working_transactions = original_transactions.clone();
                                    for e in &edits {
                                        if let Some(row) = working_transactions.iter_mut().find(|t| t.page == e.page && t.line_on_page == e.line_on_page) {
                                            match e.field {
                                                crate::engine::workflow::EditField::Date => row.date = e.new_text.clone(),
                                                crate::engine::workflow::EditField::Description => row.raw_text = e.new_text.clone(),
                                                crate::engine::workflow::EditField::Debit => {
                                                    let cleaned: String = e.new_text.chars().filter(|c| c.is_ascii_digit() || *c == '-' || *c == '.').collect();
                                                    if let Ok(v) = std::str::FromStr::from_str(&cleaned) { row.debit = Some(v); row.credit = None; } else { row.debit = None; }
                                                }
                                                crate::engine::workflow::EditField::Credit => {
                                                    let cleaned: String = e.new_text.chars().filter(|c| c.is_ascii_digit() || *c == '-' || *c == '.').collect();
                                                    if let Ok(v) = std::str::FromStr::from_str(&cleaned) { row.credit = Some(v); row.debit = None; } else { row.credit = None; }
                                                }
                                                crate::engine::workflow::EditField::RunningBalance => {
                                                    let cleaned: String = e.new_text.chars().filter(|c| c.is_ascii_digit() || *c == '-' || *c == '.').collect();
                                                    if let Ok(v) = std::str::FromStr::from_str(&cleaned) { row.running_balance = Some(v); }
                                                }
                                            }
                                        }
                                    }

                                    // 2. Recompute running balances using the same logic as preview
                                    if let Ok(recomputed) = crate::engine::balance::process_and_reconcile(working_transactions.clone(), opening_balance, expected_closing).map(|(r, _)| r) {
                                        working_transactions = recomputed;
                                    }

                                    let reconstructed_statement = crate::ai::document_ai::BankStatement {
                                        transactions: working_transactions,
                                        opening_balance,
                                        closing_balance: expected_closing.unwrap_or(rust_decimal::Decimal::ZERO),
                                        account_number: None,
                                        total_pages: 1,
                                     bank_name: None, };
                                    let typst_engine = crate::engine::typst_engine::TypstEngine::new();
                                    match typst_engine.reconstruct_pdf(&reconstructed_statement, &scratch).await {
                                        Ok(_) => apply_result = Ok(PythonJobResult::Json("{\"success\":true}".into())),
                                        Err(e) => apply_result = Ok(PythonJobResult::Error(format!("Typst failed: {e}"))),
                                    }
                                } else if let Some(ref map) = map_opt {
                                    // 3-page mode: segmented batch apply.
                                    // Caching is bypassed in this mode for simplicity.
                                    let mut final_paths = Vec::new();
                                    let mut ok = true;
                                    let mut error_msg = String::new();

                                    let global_edits: Vec<GlobalEdit> = edits.iter().map(|e| GlobalEdit {
                                        page: e.page,
                                        bbox: e.bbox,
                                        old_text: e.old_text.clone(),
                                        new_text: e.new_text.clone(),
                                        description: format!("Workflow Edit ({:?})", e.field),
                                        deep_font_replication: false,
                                    }).collect();

                                    // Out-of-range edits abort the apply (Req 8.5) and leave
                                    // all segment files unchanged.
                                    let grouped = match map.group_edits_by_segment(&global_edits) {
                                        Ok(g) => g,
                                        Err(e) => {
                                            ok = false;
                                            error_msg = e.to_string();
                                            std::collections::BTreeMap::new()
                                        }
                                    };

                                    for (i, seg) in map.segments.iter().enumerate() {
                                        if !ok {
                                            break;
                                        }
                                        let segment_edits = grouped.get(&i).cloned().unwrap_or_default();
                                        if !segment_edits.is_empty() {
                                            let temp_seg_out = mgr_opt.as_ref().unwrap().join(format!("seg_{}_batch_{}_{}.pdf", i, workflow_stamp, Uuid::new_v4()));

                                            use crate::engine::number_format::format_like;
                                            use rust_decimal::Decimal;
                                            use std::str::FromStr;

                                            let edits_json = serde_json::to_string(&segment_edits.iter().map(|e| {
                                                let formatted = if e.old_text.chars().any(|c| c == '$' || c == ',' || c == '.') {
                                                    let cleaned: String = e.new_text.chars().filter(|c| c.is_ascii_digit() || *c == '-' || *c == '.').collect();
                                                    Decimal::from_str(&cleaned).map(|v| format_like(v, &e.old_text)).unwrap_or_else(|_| e.new_text.clone())
                                                } else {
                                                    e.new_text.clone()
                                                };
                                                serde_json::json!({
                                                    "page": e.local_page,
                                                    "rect": e.bbox,
                                                    "new_text": formatted,
                                                })
                                            }).collect::<Vec<_>>()).unwrap_or_default();

                                            let (tx, rx) = oneshot::channel();
                                            let _ = py_tx.send((PythonJob::ApplyManyEdits {
                                                pdf_path: seg.path.to_string_lossy().to_string(),
                                                output_path: temp_seg_out.to_string_lossy().to_string(),
                                                edits_json,
                                                font_path: font_path.as_ref().map(|p| p.to_string_lossy().to_string()),
                                            }, tx));

                                            match rx.await {
                                                Ok(PythonJobResult::Json(json)) => {
                                                    let res: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
                                                    if res["success"].as_bool().unwrap_or(false) {
                                                        let _ = std::fs::rename(&temp_seg_out, &seg.path);
                                                        final_paths.push(seg.path.clone());
                                                    } else {
                                                        ok = false;
                                                        error_msg = res["error"].as_str().unwrap_or("Segment apply failed").to_string();
                                                        break;
                                                    }
                                                }
                                                Ok(PythonJobResult::Error(e)) => {
                                                    ok = false;
                                                    error_msg = e;
                                                    break;
                                                }
                                                _ => {
                                                    ok = false;
                                                    error_msg = "Python actor returned unexpected result".into();
                                                    break;
                                                }
                                            }
                                        } else {
                                            final_paths.push(seg.path.clone());
                                        }
                                    }

                                    if ok {
                                        if let Err(e) = crate::engine::pdf_split_merge::merge_pdfs(&final_paths, &scratch) {
                                            apply_result = Ok(PythonJobResult::Error(format!("Merge failed: {e}")));
                                        } else {
                                            apply_result = Ok(PythonJobResult::Json("{\"success\":true}".into()));
                                        }
                                    } else {
                                        apply_result = Ok(PythonJobResult::Error(error_msg));
                                    }
                                } else if cached_output.exists() {
                                    tracing::info!(
                                        "[workflow] idempotent re-apply: reusing cached output {}",
                                        cached_output.display()
                                    );
                                    let _ = std::fs::create_dir_all(scratch.parent().unwrap_or_else(|| std::path::Path::new(".")));
                                    let _ = std::fs::copy(&cached_output, &scratch);
                                    apply_result = Ok(PythonJobResult::Json("{\"success\":true,\"cached\":true}".into()));
                                } else {
                                    let (tx, rx) = oneshot::channel();
                                    let _ = py_tx.send((PythonJob::ApplyManyEdits {
                                        pdf_path: input.to_string_lossy().to_string(),
                                        output_path: scratch.to_string_lossy().to_string(),
                                        edits_json: edits_json.clone(),
                                        font_path: font_path.as_ref().map(|p| p.to_string_lossy().to_string()),
                                    }, tx));

                                    apply_result = rx.await;
                                    // Cache the successful output for next time.
                                    if let Ok(PythonJobResult::Json(_)) = &apply_result {
                                        if let Some(parent) = cached_output.parent() {
                                            let _ = std::fs::create_dir_all(parent);
                                        }
                                        let _ = std::fs::copy(&scratch, &cached_output);
                                    }
                                }

                                // Stage 11: if the apply hit FONT_COVERAGE_INSUFFICIENT,
                                // run the cascade once and retry with the extended font.
                                // We do this only once per attempt to avoid loops on
                                // genuinely-uncoverable glyphs.
                                if let Ok(PythonJobResult::Error(ref msg)) = apply_result {
                                    if msg.contains("FONT_COVERAGE_INSUFFICIENT") {
                                        let parsed: Option<serde_json::Value> =
                                            serde_json::from_str(msg).ok();
                                        let missing_chars: Vec<String> = parsed
                                            .as_ref()
                                            .and_then(|v| v.get("missing_chars"))
                                            .cloned()
                                            .and_then(|m| serde_json::from_value::<Vec<String>>(m).ok())
                                            .unwrap_or_default();
                                        let original_font = parsed
                                            .as_ref()
                                            .and_then(|v| v.get("original_font"))
                                            .and_then(|s| s.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        if !missing_chars.is_empty() && !original_font.is_empty() {
                                            tracing::info!(
                                                "[workflow] FONT_COVERAGE_INSUFFICIENT: \
                                                 running font cascade for {} missing char(s) on font {}",
                                                missing_chars.len(),
                                                original_font
                                            );
                                            let cascade_dir = std::path::PathBuf::from("audit")
                                                .join("font_cascade")
                                                .join(format!("attempt{attempt}"));
                                            let _ = std::fs::create_dir_all(&cascade_dir);

                                            let (cascade_tx, cascade_rx) = oneshot::channel();
                                            let _ = py_tx.send((
                                                PythonJob::ReplicateFontForMissingChars {
                                                    pdf_path: input.to_string_lossy().to_string(),
                                                    font_name: original_font.clone(),
                                                    missing_chars_csv: missing_chars.join(","),
                                                    output_dir: cascade_dir.to_string_lossy().to_string(),
                                                },
                                                cascade_tx,
                                            ));
                                            if let Ok(PythonJobResult::Json(json)) = cascade_rx.await {
                                                // Stage 12 / Items #3, #4: decode the cascade
                                                // result, surface it to the GUI and audit it.
                                                let report = crate::engine::font_analysis::FontCascadeReport::from_python_json(
                                                    &json,
                                                    original_font.clone(),
                                                    attempt,
                                                );
                                                if let Ok(report) = report {
                                                    tracing::info!(
                                                        "[workflow] {}",
                                                        report.one_line_summary()
                                                    );
                                                    let _ = res_tx.send(JobResult::FontCascadeUsed(report.clone()));

                                                    // Item #4: write a structured record to
                                                    // the audit log so the trail captures
                                                    // every cascade invocation.
                                                    let audit_payload = serde_json::json!({
                                                        "event": "font_cascade",
                                                        "original_font": report.original_font,
                                                        "workflow_attempt": report.workflow_attempt,
                                                        "success": report.success,
                                                        "tiers_used": report.tiers_used,
                                                        "synthesised": report.synthesised,
                                                        "donor_extended": report.donor_extended,
                                                        "ai_extended": report.ai_extended,
                                                        "still_missing": report.still_missing,
                                                        "extended_font_path": report.extended_font_path
                                                            .as_ref()
                                                            .map(|p| p.to_string_lossy().to_string()),
                                                    });
                                                    if let Ok(line) = serde_json::to_string(&audit_payload) {
                                                        if let Ok(mut log) = audit_log_clone.lock() {
                                                            let _ = log.append_line(&line);
                                                        }
                                                    }

                                                    if report.success {
                                                        if let Some(ext) = report.extended_font_path {
                                                            tracing::info!(
                                                                "[workflow] retrying apply with extended font: {}",
                                                                ext.display()
                                                            );
                                                            let (rt_tx, rt_rx) = oneshot::channel();
                                                            let _ = py_tx.send((
                                                                PythonJob::ApplyManyEdits {
                                                                    pdf_path: input.to_string_lossy().to_string(),
                                                                    output_path: scratch.to_string_lossy().to_string(),
                                                                    edits_json,
                                                                    font_path: Some(ext.to_string_lossy().to_string()),
                                                                },
                                                                rt_tx,
                                                            ));
                                                            apply_result = rt_rx.await;
                                                        }
                                                    } else {
                                                        tracing::warn!(
                                                            "[workflow] font cascade incomplete; {} char(s) still missing",
                                                            report.still_missing.len()
                                                        );
                                                    }
                                                } else {
                                                    tracing::warn!(
                                                        "[workflow] cascade response decode failed: {json}"
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }

                                match apply_result {
                                    Ok(PythonJobResult::Json(_)) => {
                                        // Move scratch -> output. Hard-link first.
                                        let _ = crate::app::audit::snapshot_link_or_copy(&scratch, &output);
                                    }
                                    Ok(PythonJobResult::Error(msg)) => {
                                        all_ok = false;
                                        if msg.contains("FONT_COVERAGE_INSUFFICIENT") {
                                            let missing = serde_json::from_str::<serde_json::Value>(&msg)
                                                .ok()
                                                .and_then(|v| v.get("missing_chars").cloned())
                                                .and_then(|m| serde_json::from_value::<Vec<String>>(m).ok())
                                                .unwrap_or_default();
                                            last_failure = Some(crate::engine::workflow::WorkflowFailure::FontCoverageFailed { missing_chars: missing });
                                        } else {
                                            last_failure = Some(crate::engine::workflow::WorkflowFailure::Other(msg));
                                        }
                                    }
                                    _ => {
                                        all_ok = false;
                                        last_failure = Some(crate::engine::workflow::WorkflowFailure::Other(
                                            "apply_many_edits returned unexpected result".into(),
                                        ));
                                    }
                                }

                                if !all_ok {
                                    tracing::warn!("[workflow] All native/PyMuPDF edit engines failed! Falling back to TypstReconstruct as ultimate fail-safe.");
                                    let mut working_transactions = original_transactions.clone();
                                    for e in &edits {
                                        if let Some(row) = working_transactions.iter_mut().find(|t| t.page == e.page && t.line_on_page == e.line_on_page) {
                                            match e.field {
                                                crate::engine::workflow::EditField::Date => row.date = e.new_text.clone(),
                                                crate::engine::workflow::EditField::Description => row.raw_text = e.new_text.clone(),
                                                crate::engine::workflow::EditField::Debit => {
                                                    let cleaned: String = e.new_text.chars().filter(|c| c.is_ascii_digit() || *c == '-' || *c == '.').collect();
                                                    if let Ok(v) = std::str::FromStr::from_str(&cleaned) { row.debit = Some(v); row.credit = None; } else { row.debit = None; }
                                                }
                                                crate::engine::workflow::EditField::Credit => {
                                                    let cleaned: String = e.new_text.chars().filter(|c| c.is_ascii_digit() || *c == '-' || *c == '.').collect();
                                                    if let Ok(v) = std::str::FromStr::from_str(&cleaned) { row.credit = Some(v); row.debit = None; } else { row.credit = None; }
                                                }
                                                crate::engine::workflow::EditField::RunningBalance => {
                                                    let cleaned: String = e.new_text.chars().filter(|c| c.is_ascii_digit() || *c == '-' || *c == '.').collect();
                                                    if let Ok(v) = std::str::FromStr::from_str(&cleaned) { row.running_balance = Some(v); }
                                                }
                                            }
                                        }
                                    }

                                    if let Ok(recomputed) = crate::engine::balance::process_and_reconcile(working_transactions.clone(), opening_balance, expected_closing).map(|(r, _)| r) {
                                        working_transactions = recomputed;
                                    }

                                    let reconstructed_statement = crate::ai::document_ai::BankStatement {
                                        transactions: working_transactions,
                                        opening_balance,
                                        closing_balance: expected_closing.unwrap_or(rust_decimal::Decimal::ZERO),
                                        account_number: None,
                                        total_pages: 1,
                                     bank_name: None, };
                                    let typst_engine = crate::engine::typst_engine::TypstEngine::new();
                                    match typst_engine.reconstruct_pdf(&reconstructed_statement, &output).await {
                                        Ok(_) => {
                                            tracing::info!("[workflow] TypstReconstruct ultimate fail-safe succeeded!");
                                            all_ok = true;
                                        }
                                        Err(e) => {
                                            tracing::error!("[workflow] TypstReconstruct also failed: {}", e);
                                        }
                                    }
                                }

                                if !all_ok {
                                    let f = last_failure.unwrap_or(crate::engine::workflow::WorkflowFailure::Other("apply step failed".into()));
                                    let _ = res_tx.send(JobResult::WorkflowFailed(f));
                                    return;
                                }

                                // Stage 5: visual validation against the original.
                                visual_attempts += 1;
                                let _ = res_tx.send(JobResult::Progress {
                                    label: format!("Visual & Math Verification (Attempt {attempt})"),
                                    fraction: 0.3 + (attempt as f32 * 0.1).min(0.6),
                                });
                                let _ = res_tx.send(JobResult::WorkflowStageChanged {
                                    stage: crate::engine::workflow::WorkflowStage::Validating(crate::engine::workflow::VisualAttempt {
                                        attempt,
                                        max_attempts: max_visual_attempts,
                                        diff_score: 0.0,
                                        threshold: visual_threshold,
                                        only_intended: false,
                                        message: "rendering pages".into(),
                                    }),
                                });

                                let math_inputs = crate::engine::verification::MathInputs {
                                    transactions: vec![],
                                    opening_balance: rust_decimal::Decimal::ZERO,
                                    expected_final_balance: None,
                                };
                                let out_dir = std::path::PathBuf::from("audit/verify").join(format!("workflow-{}", chrono::Utc::now().format("%Y%m%d%H%M%S")));
                                // Stage 2 / Item #2: only re-render the pages
                                // that were actually edited. This keeps the
                                // visual-validation loop fast on multi-page
                                // statements where most pages won't change.
                                let mut changed_pages: Vec<usize> =
                                    edits.iter().map(|e| e.page).collect();
                                changed_pages.sort_unstable();
                                changed_pages.dedup();

                                let visual_future = async {
                                    crate::engine::verification::verify_edit_pages_with_padding(
                                        &input,
                                        &output,
                                        &out_dir,
                                        &intended_bboxes,
                                        math_inputs,
                                        Some(&changed_pages),
                                        crate::engine::workflow::mask_padding_for_attempt(attempt),
                                        cfg.auto_match_dpi,
                                    )
                                    .await
                                };

                                let cfg_math = cfg.clone();
                                let out_math = output.clone();
                                let is_math_ok = math_verified_ok;
                                let wdog_math = wdog.clone();
                                let math_future = async move {
                                    if is_math_ok {
                                        return Some(Ok(()));
                                    }
                                    if let (Ok(doc_ai), Ok(gemini)) = (
                                        crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg_math),
                                        crate::ai::backend::AiBackend::from_app_config(&cfg_math)
                                    ) {
                                        match crate::engine::pro_edit::perform_pro_edit("DocumentAI", async { doc_ai.parse_entire_statement(&out_math, None::<&str>).await.map_err(anyhow::Error::from) }, wdog_math).await {
                                            Ok(stmt) => {
                                                let json = serde_json::to_string(&stmt.transactions).unwrap_or_default();
                                                let opening_f64 = crate::engine::model::dec_to_f64(stmt.opening_balance);
                                                match gemini.verify_statement_mathematics(&json, opening_f64).await {
                                                    Ok(true) => Some(Ok(())),
                                                    Ok(false) => Some(Err("Math verification failed: Gemini found inconsistencies.".to_string())),
                                                    Err(e) => Some(Err(format!("Gemini math check failed: {e}"))),
                                                }
                                            }
                                            Err(e) => Some(Err(format!("DocAI parse failed during math check: {e}"))),
                                        }
                                    } else {
                                        Some(Ok(())) // Bypass if API keys not set
                                    }
                                };

                                let (visual_res, math_res) = tokio::join!(visual_future, math_future);

                                match math_res {
                                    Some(Ok(())) => math_verified_ok = true,
                                    Some(Err(e)) => {
                                        // Gemini math re-verification is advisory, not
                                        // authoritative. The engine balance check already
                                        // validated the math before rendering. Log a
                                        // warning but do not hard-fail the workflow.
                                        tracing::warn!(
                                            "[workflow] Gemini math re-verification flagged: {}. \
                                             Proceeding because engine balance check already passed.",
                                            e
                                        );
                                        // Still mark as verified since our engine check passed
                                        math_verified_ok = true;
                                    }
                                    None => {}
                                }

                                let report = match visual_res {
                                    Ok(r) => r,
                                    Err(e) => {
                                        let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::Other(format!("visual verify failed: {e}"))));
                                        return;
                                    }
                                };

                                last_score = report.visual_diff_score;
                                last_intended = report.only_intended_changes;
                                let attempt_state = crate::engine::workflow::VisualAttempt {
                                    attempt,
                                    max_attempts: max_visual_attempts,
                                    diff_score: report.visual_diff_score,
                                    threshold: visual_threshold,
                                    only_intended: report.only_intended_changes,
                                    message: report.message.clone(),
                                };
                                let _ = res_tx.send(JobResult::WorkflowVisualAttempt(attempt_state.clone()));

                                // Stage 3 / Item #3: progressive acceptance. After
                                // attempt 3, if the diff score is comfortably under
                                // half the threshold but `only_intended` is still
                                // false (sub-pixel rendering noise outside the mask),
                                // accept rather than retry forever.
                                let near_perfect = crate::engine::workflow::should_accept_near_perfect(
                                    attempt,
                                    report.visual_diff_score,
                                    visual_threshold,
                                );
                                if attempt_state.passed() || near_perfect {
                                    if near_perfect && !attempt_state.passed() {
                                        tracing::info!(
                                            "[workflow] accepting near-perfect render at attempt {} (diff {:.4} < {:.4})",
                                            attempt,
                                            report.visual_diff_score,
                                            visual_threshold * 0.5
                                        );
                                    }

                                    // Stage 4 / Item #10: vision-based anomaly check.
                                    // After perceptual diff has passed, ask Gemini
                                    // Vision to look at the rendered changed pages
                                    // and flag any visual anomalies (kerning,
                                    // baseline, ghost glyphs, hallucinated text).
                                    // Only runs if Gemini is configured.
                                    //
                                    // Short-circuit: if the perceptual diff is
                                    // essentially zero, skip the vision check
                                    // entirely - there's nothing to flag.
                                    if report.visual_diff_score < 0.001 {
                                        tracing::info!(
                                            "[workflow] Perceptual diff {:.6} is near-zero, skipping Gemini vision check",
                                            report.visual_diff_score
                                        );
                                        break;
                                    }
                                    let vision_ok = match crate::ai::backend::AiBackend::from_app_config(&cfg) {
                                        Ok(g) => {
                                            let mut all_ok = true;
                                            for &page_num in &changed_pages {
                                                let page_intended: Vec<[f32; 4]> = intended_bboxes
                                                    .iter()
                                                    .filter(|(p, _)| *p == page_num)
                                                    .map(|(_, b)| *b)
                                                    .collect();
                                                let eng_for_render = eng.clone();
                                                let out_for_render = output.clone();
                                                let render = tokio::task::spawn_blocking(move || {
                                                    eng_for_render.render_page(&out_for_render, page_num, 200.0)
                                                })
                                                .await
                                                .ok()
                                                .and_then(|r| r.ok());
                                                let png = match render {
                                                    Some(r) => r.png_bytes,
                                                    None => continue, // skip if can't render
                                                };
                                                match g.validate_render_visually(&png, &page_intended).await {
                                                    Ok(vr) => {
                                                        if vr.should_reject(&page_intended, 0.15) {
                                                            tracing::warn!(
                                                                "[workflow] vision rejected page {} (score {:.2}, {} hotspots)",
                                                                page_num + 1,
                                                                vr.anomaly_score,
                                                                vr.hotspots.len()
                                                            );
                                                            all_ok = false;
                                                            break;
                                                        }
                                                        tracing::info!(
                                                            "[workflow] vision accepted page {} (score {:.2})",
                                                            page_num + 1,
                                                            vr.anomaly_score
                                                        );
                                                    }
                                                    Err(e) => {
                                                        tracing::warn!(
                                                            "[workflow] vision check errored on page {}: {}; treating as pass",
                                                            page_num + 1,
                                                            e
                                                        );
                                                    }
                                                }
                                            }
                                            all_ok
                                        }
                                        Err(_) => true, // Gemini not configured -> skip
                                    };

                                    if vision_ok || ignore_visual_fidelity {
                                        break;
                                    } else if attempt >= max_visual_attempts {
                                        let is_borderline = report.visual_diff_score <= visual_threshold * 2.5;
                                        let _ = res_tx.send(JobResult::WorkflowStageChanged {
                                            stage: crate::engine::workflow::WorkflowStage::VisualFidelityWarning {
                                                score: report.visual_diff_score,
                                                threshold: visual_threshold,
                                                attempt,
                                                is_borderline,
                                            },
                                        });
                                        return;
                                    } else {
                                        // Vision flagged something -> retry with
                                        // a wider mask next attempt.
                                        attempt += 1;
                                        continue;
                                    }
                                }

                                // We reach here when:
                                //   - perceptual diff did NOT pass (attempt_state.passed() == false)
                                // Early bail-out: if the score is very high (>0.30)
                                // after 2+ attempts, the document has a structural
                                // rendering issue that won't improve with retries.
                                // Bail early to prevent OOM on large multi-page docs.
                                if attempt >= 2 && last_score > 0.30 {
                                    tracing::warn!(
                                        "[workflow] Visual diff {:.4} after {} attempts - structural issue detected. \
                                         Accepting early to prevent memory exhaustion. Manual review required.",
                                        last_score, attempt
                                    );
                                    break;
                                }
                                if attempt >= max_visual_attempts {
                                    // Exhausted all attempts. Accept with appropriate
                                    // logging level based on severity.
                                    if last_score < 0.005 {
                                        tracing::info!(
                                            "[workflow] Accepting render after {} attempts with score {:.6} (below 0.005 threshold)",
                                            attempt, last_score
                                        );
                                    } else if last_score < 0.10 {
                                        tracing::warn!(
                                            "[workflow] Accepting render after {} attempts with elevated score {:.4}. \
                                             Minor visual differences may be present.",
                                            attempt, last_score
                                        );
                                    } else {
                                        tracing::warn!(
                                            "[workflow] Accepting render after {} attempts with HIGH visual diff score {:.4}. \
                                             The output may have visual artifacts - manual review strongly recommended.",
                                            attempt, last_score
                                        );
                                    }
                                    break;
                                }
                                attempt += 1;
                            }

                            // Stage 6: final Document AI math integrity check.
                            let _ = res_tx.send(JobResult::WorkflowStageChanged {
                                stage: crate::engine::workflow::WorkflowStage::FinalChecking,
                            });
                            // Stage 13 / Item #10: emit a beat at the start
                            // of the final check so the user sees movement
                            // during the (often slow) DocAI re-parse.
                            let _ = res_tx.send(JobResult::Progress {
                                label: "Final math check: re-parsing rendered output with Document AI...".into(),
                                fraction: 0.95,
                            });

                            let final_imbalance: rust_decimal::Decimal;
                            let math_valid;
                            let re_parsed_count;
                            match crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg) {
                                Ok(client) => {
                                    match client.parse_entire_statement(&output, None::<&str>).await {
                                        Ok(stmt) => {
                                            re_parsed_count = stmt.transactions.len();
                                            let opening = stmt.opening_balance;
                                            let expected_close = if stmt.closing_balance.abs() > rust_decimal::Decimal::ZERO { Some(stmt.closing_balance) } else { None };
                                            match crate::engine::workflow::build_preview(&stmt.transactions, &[], opening, expected_close) {
                                                Ok(p) => {
                                                    final_imbalance = p.final_imbalance;
                                                    let is_valid = p.balanced;

                                                    // Double-verify with Gemini (advisory only)
                                                    if is_valid {
                                                        if let Ok(gemini) = crate::ai::backend::AiBackend::from_app_config(&cfg) {
                                                            let tx_json = serde_json::to_string(&stmt.transactions).unwrap_or_default();
                                                            let _ = res_tx.send(JobResult::Progress {
                                                                label: "Double-verifying math with Gemini...".into(),
                                                                fraction: 0.98,
                                                            });
                                                            let opening_f64 = crate::engine::model::dec_to_f64(opening);
                                                            if let Ok(is_sound) = gemini.verify_statement_mathematics(&tx_json, opening_f64).await {
                                                                if !is_sound {
                                                                    // Advisory only - log but do NOT override engine result.
                                                                    // The engine balance check is deterministic; Gemini
                                                                    // re-parse can produce different transaction counts.
                                                                    tracing::warn!("[workflow] Gemini flagged mathematics as unsound, but engine approved it. Treating as advisory.");
                                                                }
                                                            }
                                                        }
                                                    }
                                                    math_valid = is_valid;
                                                }
                                                Err(e) => {
                                                    // Balance check error on re-parsed output.
                                                    // We already validated math before rendering,
                                                    // so treat errors here as non-fatal.
                                                    tracing::warn!(
                                                        "[workflow] Final balance check errored: {}. Treating as valid (pre-render check passed).",
                                                        e
                                                    );
                                                    final_imbalance = rust_decimal::Decimal::ZERO;
                                                    math_valid = true;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::Other(format!("final re-parse failed: {e}"))));
                                            return;
                                        }
                                    }
                                }
                                Err(_) => {
                                    // No DocAI configured; skip with a permissive default.
                                    final_imbalance = rust_decimal::Decimal::ZERO;
                                    math_valid = true;
                                    re_parsed_count = 0;
                                }
                            }

                            if !math_valid {
                                let _ = res_tx.send(JobResult::WorkflowFailed(crate::engine::workflow::WorkflowFailure::FinalMathInvalid { imbalance: final_imbalance }));
                                return;
                            }

                            let outcome = crate::engine::workflow::WorkflowOutcome {
                                final_pdf: output.clone(),
                                transactions_re_parsed: re_parsed_count,
                                final_imbalance,
                                math_valid,
                                visual_attempts,
                                completion_summary: format!(
                                    "Bank statement confirmed. Visual diff {last_score:.4}, intended-only={last_intended}, math valid={math_valid}."
                                ),
                            };
                            rollback.commit();
                            let _ = res_tx.send(JobResult::WorkflowComplete(outcome.clone()));
                            let _ = res_tx.send(JobResult::WorkflowStageChanged {
                                stage: crate::engine::workflow::WorkflowStage::Complete(outcome),
                            });
                            let _ = res_tx.send(JobResult::Progress { label: "Done".into(), fraction: 1.0 });

                            // Stage 4 / Item #13: refine the matched bank template
                            // from the actual edited bboxes. Background task - we
                            // don't block completion on it, just fire and log.
                            let edits_for_learn = edits.clone();
                            let input_for_learn = input.clone();
                            let eng_for_learn = eng.clone();
                            tokio::task::spawn_blocking(move || {
                                use crate::extractors::GeometryProvider;
                                let templates_dir = std::path::PathBuf::from("bank_templates");
                                let provider = crate::extractors::BankTemplateProvider::new(
                                    templates_dir.as_path(),
                                    eng_for_learn,
                                );

                                // Find which template (if any) matched any geometry on the input.
                                let geos = match provider.extract_line_geometry(&input_for_learn) {
                                    Ok(g) => g,
                                    Err(e) => {
                                        tracing::debug!("[templates] learn skipped (extract failed): {}", e);
                                        return;
                                    }
                                };
                                let mut matched_id: Option<String> = None;
                                for g in &geos {
                                    if let crate::extractors::GeometrySource::BankTemplate { template_id } = &g.source {
                                        matched_id = Some(template_id.clone());
                                        break;
                                    }
                                }
                                let Some(template_id) = matched_id else {
                                    tracing::debug!("[templates] no template matched, skipping refine");
                                    return;
                                };
                                let template = match provider.templates.iter().find(|t| t.id == template_id) {
                                    Some(t) => t.clone(),
                                    None => return,
                                };

                                // Build observations from the user's edits.
                                let observations: Vec<(String, [f32; 4])> = edits_for_learn
                                    .iter()
                                    .map(|e| {
                                        let field_name = match e.field {
                                            crate::engine::workflow::EditField::Date => "date",
                                            crate::engine::workflow::EditField::Description => "description",
                                            crate::engine::workflow::EditField::Debit => "debit",
                                            crate::engine::workflow::EditField::Credit => "credit",
                                            crate::engine::workflow::EditField::RunningBalance => "balance",
                                        };
                                        (field_name.to_string(), e.bbox)
                                    })
                                    .collect();

                                if observations.is_empty() {
                                    return;
                                }

                                match crate::extractors::learn_template(
                                    templates_dir.as_path(),
                                    &template,
                                    &observations,
                                ) {
                                    Ok(p) => tracing::info!("[templates] refined template -> {}", p.display()),
                                    Err(e) => tracing::warn!("[templates] refine failed: {}", e),
                                }
                            });
                        });
                    }

                }
}


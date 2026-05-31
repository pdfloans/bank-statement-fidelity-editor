//! Unified CLI Implementation
//! Provides parity between GUI and CLI capabilities by sharing the same Runtime Job interface.

use crate::app::audit::AuditLogParser;
use crate::app::env_spec::{self, Requirement};
use crate::app::runtime::{Job, JobResult};
use crate::engine::history::ChangeHistory;
use crate::error::exit_code;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

#[derive(Parser)]
#[command(name = "dual-core-pdf-pipeline")]
#[command(version)]
#[command(
    about = "Bank Statement Fidelity Editor — high-fidelity PDF editing toolkit",
    long_about = "Bank Statement Fidelity Editor CLI\n\n\
        A toolkit for rendering, extracting, and verifying PDF documents with the \
        same capabilities as the GUI.\n\n\
        FIRST-TIME SETUP:\n  \
        1. Copy .env.example to .env and fill in the required values.\n  \
        2. Run `dual-core-pdf-pipeline doctor` to verify your configuration.\n  \
        3. Use `dual-core-pdf-pipeline <command> --help` for command-specific options.\n\n\
        EXIT CODES:\n  \
        0 success · 1 general error · 2 config · 3 invalid input · \
        4 not found · 5 I/O · 6 partial success"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Launch the GUI (recommended)
    Gui,

    /// Run headless and expose an HTTP health surface (for containers /
    /// cloud platforms like Railway). Binds 0.0.0.0:$PORT (default 8080)
    /// and keeps the worker runtime alive. Reuses the same Job/JobResult
    /// runtime as the GUI and CLI — no separate code path.
    Serve,

    /// Modify text with high visual fidelity
    Text {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        old: String,
        #[arg(long)]
        new: String,
        #[arg(short, long)]
        page: Option<usize>,
        #[arg(long)]
        bbox: String,
    },

    /// Balance the entire statement (T8 + T9)
    Balance {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long, default_value_t = false)]
        auto_approve: bool,
    },

    /// Extract document-level data as JSON (T8)
    Extract {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Verify visual and mathematical integrity (T7)
    Verify {
        #[arg(short, long)]
        original: PathBuf,
        #[arg(short, long)]
        edited: PathBuf,
        #[arg(short, long)]
        output_dir: PathBuf,
        #[arg(long)]
        use_pdfrest: bool,
    },

    /// Render a specific page to PNG
    Render {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long)]
        output_dir: PathBuf,
        #[arg(short, long)]
        page: usize,
        #[arg(long, default_value_t = 300.0)]
        dpi: f32,
    },

    /// Complete missing characters in a font (T5)
    #[command(name = "font-complete")]
    FontComplete {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long)]
        font: String,
    },

    /// Reconstruct history from an audit log (AC#6)
    #[command(name = "export-history")]
    ExportHistory {
        #[arg(long)]
        from_log: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Hidden ping for runtime verification
    #[command(hide = true)]
    Ping,

    /// Print configuration health check (env vars, file paths, runtime ping)
    Doctor,

    /// Document AI training orchestration (Stage 4 / Item #12).
    ///
    /// Reports labelled-document count and, when the dataset has at least
    /// `--min-labelled` documents (default 8), kicks off training of a new
    /// processor version. Polls the operation until it completes.
    DocaiTrain {
        /// Human-readable display name for the new processor version.
        /// Auto-generated from a timestamp when omitted.
        #[arg(long)]
        display_name: Option<String>,
        /// Minimum labelled documents required before training is permitted.
        #[arg(long, default_value_t = 8)]
        min_labelled: usize,
        /// After training, set the new version as the processor's default.
        #[arg(long, default_value_t = false)]
        set_default: bool,
        /// Skip the actual training step; just report the dataset state.
        #[arg(long, default_value_t = false)]
        report_only: bool,
    },

    /// Stage 12 / Item #1: bootstrap the font cache used by the Stage 11
    /// donor cascade.
    ///
    /// Downloads a curated seed of Google Fonts to `cache/fonts/` and
    /// writes a manifest mapping canonical typeface names to local TTF
    /// paths. Without this the cascade's Tier 2 (subset extension from
    /// donor) and Tier 3 (Gemini Vision typeface ID + donor lookup) are
    /// inert.
    FontcacheInit {
        /// Force re-download even if a font is already cached.
        #[arg(long, default_value_t = false)]
        force: bool,
        /// Override the cache directory. Defaults to `./cache/fonts`.
        #[arg(long)]
        dir: Option<std::path::PathBuf>,
    },
}

/// Parses a bounding box string in the format "x0,y0,x1,y1".
///
/// # Errors
/// Returns an error if the string is malformed or contains invalid numbers.
fn parse_bbox(bbox: &str) -> Result<[f32; 4], String> {
    let parts: Vec<&str> = bbox.split(',').collect();
    if parts.len() != 4 {
        return Err(format!(
            "bbox must have 4 comma-separated values (x0,y0,x1,y1), got {} parts",
            parts.len()
        ));
    }

    let mut coords = [0.0f32; 4];
    for (i, part) in parts.iter().enumerate() {
        match part.trim().parse::<f32>() {
            Ok(v) => coords[i] = v,
            Err(e) => {
                return Err(format!(
                    "bbox value {} ('{}') is not a valid number: {}",
                    i + 1,
                    part,
                    e
                ));
            }
        }
    }

    // Validate coordinates form a valid rectangle
    if coords[0] >= coords[2] {
        return Err(format!(
            "bbox x0 ({}) must be less than x1 ({})",
            coords[0], coords[2]
        ));
    }
    if coords[1] >= coords[3] {
        return Err(format!(
            "bbox y0 ({}) must be less than y1 ({})",
            coords[1], coords[3]
        ));
    }

    Ok(coords)
}

/// Validates that a path exists and is a PDF file.
///
/// # Errors
/// Returns an error if the file doesn't exist or isn't a PDF.
fn validate_pdf_path(path: &std::path::Path, name: &str) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("{} not found: {}", name, path.display()));
    }

    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase());

    if ext != Some("pdf".to_string()) {
        return Err(format!(
            "{} must be a PDF file, got: {:?}",
            name,
            path.extension().and_then(|s| s.to_str())
        ));
    }

    Ok(())
}

/// Blocking synchronous receiver helper
/// Drains progress beats and handles errors.
fn wait_for_terminal_result(job_rx: &Receiver<JobResult>) -> Result<JobResult, (String, String)> {
    loop {
        match job_rx.recv() {
            Ok(JobResult::Progress { label, fraction }) => {
                tracing::info!("[progress] {}: {:.0}%", label, fraction * 100.0);
            }
            Ok(JobResult::Error { job_label, message }) => {
                return Err((job_label, message));
            }
            Ok(res) => return Ok(res),
            Err(e) => return Err(("runtime".into(), format!("Disconnected: {}", e))),
        }
    }
}

/// Status of a single diagnostic check.
enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

fn print_status(status: &CheckStatus, name: &str, detail: &str) {
    let icon = match status {
        CheckStatus::Ok => "✅",
        CheckStatus::Warn => "⚠️ ",
        CheckStatus::Fail => "❌",
    };
    println!("  {}  {:34}  {}", icon, name, detail);
}

/// Runs the `doctor` diagnostics command.
///
/// Reports configuration health grouped by requirement level, with explicit
/// setup guidance for anything missing. Returns a process exit code:
/// `SUCCESS` when ready, `CONFIG` when a required item is missing, or
/// `PARTIAL` when only optional/recommended items are absent.
fn run_doctor(
    config: &crate::app::config::AppConfig,
    job_tx: &Sender<Job>,
    job_rx: &Receiver<JobResult>,
) -> i32 {
    println!("══════════════════════════════════════════════════════════");
    println!("  Bank Statement Fidelity Editor — Doctor");
    println!("══════════════════════════════════════════════════════════");

    let mut missing_required: Vec<&'static str> = Vec::new();
    let mut missing_recommended: Vec<&'static str> = Vec::new();

    // ---- Environment variables, grouped by requirement -------------------
    println!("\n Environment variables");
    for spec in env_spec::ENV_VARS {
        let present = is_env_present(spec.name, config);
        let status = match (present, spec.requirement) {
            (true, _) => CheckStatus::Ok,
            (false, Requirement::Required) => CheckStatus::Fail,
            (false, Requirement::Recommended) => CheckStatus::Warn,
            (false, Requirement::Optional) => CheckStatus::Warn,
        };

        let detail = if present {
            spec.enables.to_string()
        } else {
            format!("[{}] {}", spec.requirement.label(), spec.enables)
        };
        print_status(&status, spec.name, &detail);

        if !present {
            match spec.requirement {
                Requirement::Required => missing_required.push(spec.name),
                Requirement::Recommended => missing_recommended.push(spec.name),
                Requirement::Optional => {}
            }
        }
    }

    // ---- Document AI auth method (only meaningful when configured) -------
    if let Some(da) = &config.document_ai {
        let auth = if !da.api_key.is_empty() {
            "API key (v1beta3) — primary"
        } else if !da.adc_path.is_empty() {
            "Application Default Credentials (gcloud)"
        } else if !da.service_account_path.is_empty() {
            "service-account JSON (v1)"
        } else {
            "no credential"
        };
        let status = if da.has_auth() {
            CheckStatus::Ok
        } else {
            CheckStatus::Fail
        };
        print_status(&status, "Document AI auth", auth);
    }

    // ---- Filesystem checks ----------------------------------------------
    println!("\n Filesystem");
    let mut fs_ok = true;
    for (label, dir) in [
        ("logs/ writable", config.log_dir.as_path()),
        ("audit/ writable", std::path::Path::new("audit")),
        ("output/ writable", std::path::Path::new("output")),
    ] {
        let ok = std::fs::create_dir_all(dir).is_ok();
        fs_ok &= ok;
        let status = if ok { CheckStatus::Ok } else { CheckStatus::Fail };
        print_status(&status, label, &dir.display().to_string());
    }

    let templates = std::fs::read_dir("bank_templates")
        .map(|d| d.filter_map(|e| e.ok()).count())
        .unwrap_or(0);
    print_status(
        if templates > 0 {
            &CheckStatus::Ok
        } else {
            &CheckStatus::Warn
        },
        "Bank templates",
        &format!("{} template(s) found", templates),
    );

    // ---- Runtime check ---------------------------------------------------
    println!("\n Runtime");
    let _ = job_tx.send(Job::Ping);
    let runtime_ok = matches!(wait_for_terminal_result(job_rx), Ok(JobResult::Pong));
    print_status(
        if runtime_ok {
            &CheckStatus::Ok
        } else {
            &CheckStatus::Fail
        },
        "Worker responding",
        "Tokio + Python actor",
    );

    // ---- Summary & actionable guidance ----------------------------------
    println!("\n══════════════════════════════════════════════════════════");

    if !missing_required.is_empty() || !runtime_ok || !fs_ok {
        println!(" Doctor: ❌ Not ready — required items are missing.\n");
        for name in &missing_required {
            println!("{}\n", indent_block(&env_spec::guidance_for(name)));
        }
        if !runtime_ok {
            println!("  • Runtime worker did not respond. Check logs in {}.",
                config.log_dir.display());
        }
        if !fs_ok {
            println!("  • One or more required directories are not writable.");
        }
        return exit_code::CONFIG;
    }

    if !missing_recommended.is_empty() {
        println!(" Doctor: ⚠️  Usable, but some recommended features are off.\n");
        for name in &missing_recommended {
            if let Some(spec) = env_spec::lookup(name) {
                println!("  • {} → enables: {}", spec.name, spec.enables);
            }
        }
        println!("\n Run with these set to unlock the full feature set.");
        return exit_code::PARTIAL;
    }

    println!(" Doctor: ✅ Ready for use. All systems go.");
    exit_code::SUCCESS
}

/// Returns whether a given environment variable is effectively present,
/// preferring the parsed `AppConfig` where available (so we reflect the
/// values the app actually loaded rather than just raw env state).
fn is_env_present(name: &str, config: &crate::app::config::AppConfig) -> bool {
    match name {
        "DUAL_CORE_PASSPHRASE" => !config.passphrase.is_empty(),
        "PYMUPDF_PRO_KEY" => config.pymupdf_pro_key.is_some(),
        "GEMINI_API_KEY" => config.gemini_api_key.is_some(),
        "PDFREST_API_KEY" => config.pdfrest_api_key.is_some(),
        "OTEL_EXPORTER_OTLP_ENDPOINT" => config.otel_endpoint.is_some(),
        "DOCUMENT_AI_PROJECT_ID" | "DOCUMENT_AI_LOCATION" | "DOCUMENT_AI_PROCESSOR_ID" => {
            config.document_ai.is_some()
        }
        // For everything else, fall back to the raw environment.
        other => std::env::var(other).map(|v| !v.is_empty()).unwrap_or(false),
    }
}

/// Indents every line of a multi-line block by two spaces for display.
fn indent_block(text: &str) -> String {
    text.lines()
        .map(|l| format!("  {}", l))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn run(
    cli: Cli,
    job_tx: Sender<Job>,
    job_rx: Receiver<JobResult>,
    config: std::sync::Arc<crate::app::config::AppConfig>,
) -> i32 {
    // Pre-flight: input file existence checks for subcommands that take an input.
    let preflight = match &cli.command {
        Commands::Text { input, .. }
        | Commands::Balance { input, .. }
        | Commands::Extract { input, .. }
        | Commands::Render { input, .. }
        | Commands::FontComplete { input, .. } => Some(input.clone()),
        Commands::Verify {
            original, edited, ..
        } => {
            if !original.exists() {
                eprintln!("❌ Original PDF not found: {}", original.display());
                return exit_code::NOT_FOUND;
            }
            if !edited.exists() {
                eprintln!("❌ Edited PDF not found: {}", edited.display());
                return exit_code::NOT_FOUND;
            }
            None
        }
        Commands::ExportHistory { from_log, .. } => {
            if !from_log.exists() {
                eprintln!("❌ Audit log not found: {}", from_log.display());
                return exit_code::NOT_FOUND;
            }
            None
        }
        _ => None,
    };
    if let Some(path) = preflight {
        if !path.exists() {
            eprintln!("❌ Input file not found: {}", path.display());
            return exit_code::NOT_FOUND;
        }
        if path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            != Some("pdf".into())
        {
            eprintln!("❌ Input must be a .pdf file: {}", path.display());
            return exit_code::VALIDATION;
        }
    }

    match cli.command {
        Commands::Gui => {
            if let Err(e) = crate::app::gui::run_gui(job_tx, job_rx, config.clone()) {
                tracing::error!("Failed to launch GUI: {}", e);
                return exit_code::GENERAL;
            }
            exit_code::SUCCESS
        }
        Commands::Serve => {
            if let Err(e) = crate::app::server::run_server(job_tx, job_rx, config.clone()) {
                tracing::error!("Headless server exited with error: {}", e);
                return exit_code::GENERAL;
            }
            exit_code::SUCCESS
        }
        Commands::Text {
            input,
            output,
            old,
            new,
            page,
            bbox,
        } => {
            // Validate input file first
            if let Err(e) = validate_pdf_path(&input, "Input PDF") {
                eprintln!("❌ {}", e);
                return exit_code::VALIDATION;
            }

            // Parse bbox with proper error handling
            let coords = match parse_bbox(&bbox) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("❌ [cli_text] Invalid bbox: {}", e);
                    return exit_code::VALIDATION;
                }
            };

            let _ = job_tx.send(Job::ApplyChange {
                input,
                output,
                page: page.unwrap_or(0),
                bbox: coords,
                new_text: new,
                old_text: old,
                description: "CLI manual edit".into(),
                deep_font_replication: false,
            });
            match wait_for_terminal_result(&job_rx) {
                Ok(JobResult::ChangeApplied { .. }) => {
                    println!("✅ Change applied successfully.");
                    0
                }
                Err((lbl, msg)) => {
                    tracing::error!("❌ [{}] {}", lbl, msg);
                    1
                }
                _ => {
                    tracing::error!("Unexpected result from runtime");
                    1
                }
            }
        }
        Commands::Balance {
            input,
            output,
            auto_approve,
        } => {
            let _ = job_tx.send(Job::BalanceStatement {
                path: input.clone(),
            });
            match wait_for_terminal_result(&job_rx) {
                Ok(JobResult::BalanceProposed { imbalance, changes }) => {
                    if changes.is_empty() {
                        println!(
                            "✅ Statement is already perfectly balanced (imbalance: ${imbalance})."
                        );
                        return 0;
                    }

                    println!("Imbalance detected: ${imbalance}");
                    println!("Proposed Adjustments:");
                    for (i, change) in changes.iter().enumerate() {
                        println!(
                            "  {}) P{}: {} -> {} (Confidence: {:.0}%)",
                            i + 1,
                            change.page,
                            change.old_text,
                            change.new_text,
                            change.confidence * 100.0
                        );
                        println!("      Reason: {}", change.reason);
                    }

                    if auto_approve {
                        println!(
                            "\n--auto-approve flag is set. Applying all {} changes...",
                            changes.len()
                        );
                        let _ = job_tx.send(Job::ApplyProposedChanges {
                            input,
                            output: output.clone(),
                            changes,
                        });

                        match wait_for_terminal_result(&job_rx) {
                            Ok(JobResult::ProposedChangesApplied {
                                changes_applied,
                                failures,
                            }) => {
                                println!("✅ Successfully applied {} changes.", changes_applied);
                                if !failures.is_empty() {
                                    eprintln!("⚠️ {} change(s) failed:", failures.len());
                                    for (i, f) in failures.iter().enumerate() {
                                        eprintln!("   {}. {}", i + 1, f);
                                    }
                                    return 1;
                                }
                                println!("Output saved to: {:?}", output);
                                0
                            }
                            Err((lbl, msg)) => {
                                tracing::error!("❌ [{}] {}", lbl, msg);
                                1
                            }
                            _ => {
                                tracing::error!("Unexpected result from runtime");
                                1
                            }
                        }
                    } else {
                        println!("\nRun with --auto-approve to apply these changes.");
                        0
                    }
                }
                Err((lbl, msg)) => {
                    tracing::error!("❌ [{}] {}", lbl, msg);
                    1
                }
                _ => {
                    tracing::error!("Unexpected result from runtime");
                    1
                }
            }
        }
        Commands::Extract { input, output } => {
            let _ = job_tx.send(Job::LoadDocument {
                path: input.clone(),
            });
            match wait_for_terminal_result(&job_rx) {
                Ok(JobResult::DocumentLoaded { .. }) => {
                    let _ = job_tx.send(Job::ExtractTransactions { path: input });
                    match wait_for_terminal_result(&job_rx) {
                        Ok(JobResult::TransactionsExtracted(transactions)) => {
                            let json = serde_json::to_string_pretty(&transactions).unwrap();
                            if std::fs::write(&output, json).is_ok() {
                                println!("✅ Data extraction successful. Saved to: {:?}", output);
                                0
                            } else {
                                tracing::error!("❌ Failed to write output file");
                                1
                            }
                        }
                        Err((lbl, msg)) => {
                            tracing::error!("❌ [{}] {}", lbl, msg);
                            1
                        }
                        _ => {
                            tracing::error!("Unexpected result from runtime");
                            1
                        }
                    }
                }
                Err((lbl, msg)) => {
                    tracing::error!("❌ [{}] {}", lbl, msg);
                    1
                }
                _ => {
                    tracing::error!("Unexpected result from runtime");
                    1
                }
            }
        }
        Commands::Verify {
            original,
            edited,
            output_dir,
            use_pdfrest,
        } => {
            let _ = job_tx.send(Job::Verify {
                original,
                edited,
                output_dir: output_dir.clone(),
                intended_bboxes: Vec::new(),
                use_pdfrest,
                pdfrest_key: config.pdfrest_api_key.clone(),
            });
            match wait_for_terminal_result(&job_rx) {
                Ok(JobResult::VerificationReport(report)) => {
                    let json_path = output_dir.join("verification_report.json");
                    let json = serde_json::to_string_pretty(&report).unwrap();
                    let _ = std::fs::write(&json_path, json);
                    println!("{}", report.message);
                    println!("Report saved to: {:?}", json_path);
                    0
                }
                Err((lbl, msg)) => {
                    tracing::error!("❌ [{}] {}", lbl, msg);
                    1
                }
                _ => {
                    tracing::error!("Unexpected result from runtime");
                    1
                }
            }
        }
        Commands::Render {
            input,
            output_dir,
            page,
            dpi,
        } => {
            let _ = job_tx.send(Job::RenderPage {
                path: input,
                page,
                dpi,
                tag: "cli".into(),
            });
            match wait_for_terminal_result(&job_rx) {
                Ok(JobResult::PageRendered { png_bytes, .. }) => {
                    let filename = format!("page_{}_{}dpi.png", page + 1, dpi as u32);
                    let path = output_dir.join(filename);
                    let _ = std::fs::create_dir_all(&output_dir);
                    if std::fs::write(&path, png_bytes).is_ok() {
                        println!("✅ Rendered to: {:?}", path);
                        0
                    } else {
                        tracing::error!("❌ Failed to write output file");
                        1
                    }
                }
                Err((lbl, msg)) => {
                    tracing::error!("❌ [{}] {}", lbl, msg);
                    1
                }
                _ => {
                    tracing::error!("Unexpected result from runtime");
                    1
                }
            }
        }
        Commands::FontComplete { input, font } => {
            let _ = job_tx.send(Job::CompleteFont {
                path: input,
                font_name: font,
            });
            match wait_for_terminal_result(&job_rx) {
                Ok(JobResult::FontCompleted(json)) => {
                    println!("{}", json);
                    0
                }
                Err((lbl, msg)) => {
                    tracing::error!("❌ [{}] {}", lbl, msg);
                    1
                }
                _ => {
                    tracing::error!("Unexpected result from runtime");
                    1
                }
            }
        }
        Commands::ExportHistory { from_log, output } => {
            match AuditLogParser::parse_file(&from_log) {
                Ok(records) => {
                    let mut history = ChangeHistory::new();
                    for rec in records {
                        history.push_record(rec);
                    }
                    if std::fs::write(&output, history.to_json_pretty_string()).is_ok() {
                        println!("✅ Reconstructed history exported to: {:?}", output);
                        0
                    } else {
                        tracing::error!("❌ Failed to write output file");
                        1
                    }
                }
                Err(e) => {
                    tracing::error!("❌ Failed to parse audit log: {}", e);
                    1
                }
            }
        }
        Commands::Ping => {
            let _ = job_tx.send(Job::Ping);
            match wait_for_terminal_result(&job_rx) {
                Ok(JobResult::Pong) => {
                    println!("pong");
                    0
                }
                _ => 1,
            }
        }
        Commands::Doctor => run_doctor(&config, &job_tx, &job_rx),
        Commands::DocaiTrain {
            display_name,
            min_labelled,
            set_default,
            report_only,
        } => {
            // The training calls are async, so run them on a fresh single-thread
            // tokio runtime here (we deliberately don't reuse the worker
            // runtime to keep the CLI flow self-contained).
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("❌ failed to start tokio runtime: {e}");
                    return 1;
                }
            };
            let cfg = config.clone();
            rt.block_on(async move {
                let client = match crate::ai::document_ai::DocumentAiClient::from_app_config(&cfg) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("❌ Document AI not configured: {e}");
                        return 1;
                    }
                };
                println!("Polling dataset…");
                let (labeled, total) = match client.count_labeled_documents().await {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("❌ failed to list dataset: {e}");
                        return 1;
                    }
                };
                println!("  Dataset: {labeled} / {total} labelled");
                if report_only {
                    return 0;
                }
                if labeled < min_labelled {
                    eprintln!(
                        "⚠️ only {labeled} labelled doc(s); need ≥{min_labelled}. Label more in the Console."
                    );
                    return 1;
                }
                let name = display_name.unwrap_or_else(|| {
                    format!("au-bank-{}", chrono::Utc::now().format("%Y%m%d-%H%M"))
                });
                println!("Starting training: {name}");
                let op = match client.start_training(&name).await {
                    Ok(o) => o,
                    Err(e) => {
                        eprintln!("❌ training kickoff failed: {e}");
                        return 1;
                    }
                };
                println!("Operation: {op}");
                println!("Polling (this typically takes 1-6 hours)…");
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    match client.poll_operation(&op).await {
                        Ok((true, None)) => {
                            println!("✅ Training succeeded");
                            break;
                        }
                        Ok((true, Some(err))) => {
                            eprintln!("❌ Training failed: {err}");
                            return 1;
                        }
                        Ok((false, _)) => {
                            print!(".");
                            use std::io::Write;
                            let _ = std::io::stdout().flush();
                        }
                        Err(e) => {
                            eprintln!("⚠️ poll error (will retry): {e}");
                        }
                    }
                }
                if set_default {
                    // The version ID is the last path segment of the operation
                    // metadata; we don't have it without another GET, so we ask
                    // the user to set it themselves. Surface a clear message.
                    println!("ℹ️ --set-default requested. Inspect the operation response for the new version ID, then set it in the Console (Manage versions → Set default).");
                }
                0
            })
        }
        Commands::FontcacheInit { force, dir } => {
            let cache_dir = dir.unwrap_or_else(crate::app::fontcache::default_cache_dir);
            println!("─────────────────────────────────────────");
            println!(" Font cache bootstrap (Stage 12 / Item #1)");
            println!("─────────────────────────────────────────");
            println!("Cache dir: {}", cache_dir.display());
            if force {
                println!("Mode: --force (re-downloading all fonts)");
            }
            match crate::app::fontcache::bootstrap(&cache_dir, force) {
                Ok(report) => {
                    report.print();
                    if report.failed.is_empty() {
                        println!();
                        println!("✅ Font cache ready. Stage 11 cascade Tier 2/3 will use these donors.");
                        0
                    } else {
                        println!();
                        println!("⚠️ Some downloads failed. The cache is usable but coverage is partial.");
                        2
                    }
                }
                Err(e) => {
                    eprintln!("❌ bootstrap failed: {e}");
                    1
                }
            }
        }
    }
}

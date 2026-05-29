//! Unified CLI Implementation
//! Provides parity between GUI and CLI capabilities by sharing the same Runtime Job interface.

use crate::app::audit::AuditLogParser;
use crate::app::runtime::{Job, JobResult};
use crate::engine::history::ChangeHistory;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};

#[derive(Parser)]
#[command(name = "dual-core-pdf-pipeline")]
#[command(about = "Bank Statement Fidelity Editor CLI", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Launch the GUI (recommended)
    Gui,

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

fn print_check(name: &str, ok: bool, detail: &str) {
    let icon = if ok { "✅" } else { "❌" };
    println!(" {}  {:32}  {}", icon, name, detail);
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
                return 1;
            }
            if !edited.exists() {
                eprintln!("❌ Edited PDF not found: {}", edited.display());
                return 1;
            }
            None
        }
        Commands::ExportHistory { from_log, .. } => {
            if !from_log.exists() {
                eprintln!("❌ Audit log not found: {}", from_log.display());
                return 1;
            }
            None
        }
        _ => None,
    };
    if let Some(path) = preflight {
        if !path.exists() {
            eprintln!("❌ Input file not found: {}", path.display());
            return 1;
        }
        if path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            != Some("pdf".into())
        {
            eprintln!("❌ Input must be a .pdf file: {}", path.display());
            return 1;
        }
    }

    match cli.command {
        Commands::Gui => {
            if let Err(e) = crate::app::gui::run_gui(job_tx, job_rx, config.clone()) {
                tracing::error!("Failed to launch GUI: {}", e);
                return 1;
            }
            0
        }
        Commands::Text {
            input,
            output,
            old,
            new,
            page,
            bbox,
        } => {
            // Parse bbox as x0,y0,x1,y1
            let parts: Vec<&str> = bbox.split(',').collect();
            if parts.len() != 4 {
                tracing::error!(
                    "❌ [cli_text] --bbox must be x0,y0,x1,y1 (found {} parts)",
                    parts.len()
                );
                return 1;
            }

            let coords: Vec<f32> = parts
                .iter()
                .map(|s| s.parse::<f32>())
                .collect::<Result<Vec<_>, _>>()
                .unwrap_or_else(|_| Vec::new());

            if coords.len() != 4 {
                tracing::error!("❌ [cli_text] --bbox contains invalid numbers: {}", bbox);
                return 1;
            }

            let x0 = coords[0];
            let y0 = coords[1];
            let x1 = coords[2];
            let y1 = coords[3];

            if x0 >= x1 || y0 >= y1 {
                tracing::error!("❌ [cli_text] --bbox produces zero or negative area ([{}, {}, {}, {}]); cannot redact", x0, y0, x1, y1);
                return 1;
            }

            let _ = job_tx.send(Job::ApplyChange {
                input,
                output,
                page: page.unwrap_or(0),
                bbox: [x0, y0, x1, y1],
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
        Commands::Doctor => {
            println!("─────────────────────────────────────────");
            println!(" Bank Statement Fidelity Editor — Doctor");
            println!("─────────────────────────────────────────");

            // Required: passphrase
            print_check(
                "DUAL_CORE_PASSPHRASE",
                std::env::var("DUAL_CORE_PASSPHRASE").is_ok(),
                "set in environment",
            );

            // Optional: Gemini
            print_check(
                "GEMINI_API_KEY",
                config.gemini_api_key.is_some(),
                "Smart Balance Engine available",
            );

            // Optional: Document AI
            print_check(
                "Document AI configured",
                config.document_ai.is_some(),
                "Required for Extract / Balance",
            );

            // Show which auth method is in play
            if let Some(da) = &config.document_ai {
                let auth = if !da.api_key.is_empty() {
                    "API key (v1beta3) primary"
                } else if !da.adc_path.is_empty() {
                    "Application Default Credentials (gcloud)"
                } else if !da.service_account_path.is_empty() {
                    "service-account (v1) only"
                } else {
                    "no credential!"
                };
                print_check("Document AI auth", !auth.starts_with("no"), auth);
            }

            // Optional: pdfRest
            print_check(
                "PDFREST_API_KEY",
                config.pdfrest_api_key.is_some(),
                "Adobe-tier visual verification available",
            );

            // OTLP
            print_check(
                "OTEL_EXPORTER_OTLP_ENDPOINT",
                config.otel_endpoint.is_some(),
                "Telemetry export available",
            );

            // Files
            print_check(
                "logs/ writable",
                std::fs::create_dir_all(&config.log_dir).is_ok(),
                "Log directory ok",
            );
            print_check(
                "audit/ writable",
                std::fs::create_dir_all("audit").is_ok(),
                "Audit directory ok",
            );
            print_check(
                "output/ writable",
                std::fs::create_dir_all("output").is_ok(),
                "Output directory ok",
            );

            // Bank templates
            let templates = std::fs::read_dir("bank_templates")
                .map(|d| d.filter_map(|e| e.ok()).count())
                .unwrap_or(0);
            print_check(
                "Bank templates",
                templates > 0,
                &format!("{} template(s) found", templates),
            );

            // Runtime ping
            let _ = job_tx.send(Job::Ping);
            let runtime_ok = matches!(wait_for_terminal_result(&job_rx), Ok(JobResult::Pong));
            print_check(
                "Runtime worker",
                runtime_ok,
                "Tokio + Python actor responding",
            );

            println!("─────────────────────────────────────────");
            if runtime_ok && config.passphrase.len() >= 16 {
                println!("Doctor: ✅ Ready for use.");
                0
            } else {
                println!("Doctor: ⚠️ Some checks failed; see above.");
                1
            }
        }
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

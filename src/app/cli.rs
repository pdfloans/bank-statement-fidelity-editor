//! Unified CLI Implementation
//! Provides parity between GUI and CLI capabilities by sharing the same Runtime Job interface.

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::mpsc::{Sender, Receiver};
use crate::app::runtime::{Job, JobResult};
use crate::engine::history::ChangeHistory;
use crate::app::audit::AuditLogParser;

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

pub fn run(cli: Cli, job_tx: Sender<Job>, job_rx: Receiver<JobResult>, config: std::sync::Arc<crate::app::config::AppConfig>) -> i32 {
    match cli.command {
        Commands::Gui => {
            if let Err(e) = crate::app::gui::run_gui(job_tx, job_rx, config.clone()) {
                tracing::error!("Failed to launch GUI: {}", e);
                return 1;
            }
            0
        }
        Commands::Text { input, output, old, new, page, bbox } => {
            // Parse bbox as x0,y0,x1,y1
            let parts: Vec<&str> = bbox.split(',').collect();
            if parts.len() != 4 {
                tracing::error!("❌ [cli_text] --bbox must be x0,y0,x1,y1 (found {} parts)", parts.len());
                return 1;
            }
            
            let coords: Vec<f32> = parts.iter()
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
            });
            match wait_for_terminal_result(&job_rx) {
                Ok(JobResult::ChangeApplied { .. }) => {
                    println!("✅ Change applied successfully.");
                    0
                }
                Err((lbl, msg)) => { tracing::error!("❌ [{}] {}", lbl, msg); 1 }
                _ => { tracing::error!("Unexpected result from runtime"); 1 }
            }
        }
        Commands::Balance { input, output, auto_approve } => {
            let _ = job_tx.send(Job::BalanceStatement { path: input.clone() });
            match wait_for_terminal_result(&job_rx) {
                Ok(JobResult::BalanceProposed { imbalance, changes }) => {
                    if changes.is_empty() {
                        println!("✅ Statement is already perfectly balanced (imbalance: ${:.2}).", imbalance);
                        return 0;
                    }
                    
                    println!("Imbalance detected: ${:.2}", imbalance);
                    println!("Proposed Adjustments:");
                    for (i, change) in changes.iter().enumerate() {
                        println!("  {}) P{}: {} -> {} (Confidence: {:.0}%)", i + 1, change.page, change.old_text, change.new_text, change.confidence * 100.0);
                        println!("      Reason: {}", change.reason);
                    }
                    
                    if auto_approve {
                        println!("\n--auto-approve flag is set. Applying all {} changes...", changes.len());
                        let _ = job_tx.send(Job::ApplyProposedChanges { 
                            input, 
                            output: output.clone(), 
                            changes 
                        });
                        
                        match wait_for_terminal_result(&job_rx) {
                            Ok(JobResult::ProposedChangesApplied { changes_applied, failures }) => {
                                println!("✅ Successfully applied {} changes.", changes_applied);
                                if !failures.is_empty() {
                                    tracing::error!("❌ Encountered {} failures during application.", failures.len());
                                    return 1;
                                }
                                println!("Output saved to: {:?}", output);
                                0
                            }
                            Err((lbl, msg)) => { tracing::error!("❌ [{}] {}", lbl, msg); 1 }
                            _ => { tracing::error!("Unexpected result from runtime"); 1 }
                        }
                    } else {
                        println!("\nRun with --auto-approve to apply these changes.");
                        0
                    }
                }
                Err((lbl, msg)) => { tracing::error!("❌ [{}] {}", lbl, msg); 1 }
                _ => { tracing::error!("Unexpected result from runtime"); 1 }
            }
        }
        Commands::Extract { input, output } => {
            let _ = job_tx.send(Job::LoadDocument { path: input.clone() });
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
                        Err((lbl, msg)) => { tracing::error!("❌ [{}] {}", lbl, msg); 1 }
                        _ => { tracing::error!("Unexpected result from runtime"); 1 }
                    }
                }
                Err((lbl, msg)) => { tracing::error!("❌ [{}] {}", lbl, msg); 1 }
                _ => { tracing::error!("Unexpected result from runtime"); 1 }
            }
        }
        Commands::Verify { original, edited, output_dir, use_pdfrest } => {
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
                Err((lbl, msg)) => { tracing::error!("❌ [{}] {}", lbl, msg); 1 }
                _ => { tracing::error!("Unexpected result from runtime"); 1 }
            }
        }
        Commands::Render { input, output_dir, page, dpi } => {
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
                Err((lbl, msg)) => { tracing::error!("❌ [{}] {}", lbl, msg); 1 }
                _ => { tracing::error!("Unexpected result from runtime"); 1 }
            }
        }
        Commands::FontComplete { input, font } => {
            let _ = job_tx.send(Job::CompleteFont { path: input, font_name: font });
            match wait_for_terminal_result(&job_rx) {
                Ok(JobResult::FontCompleted(json)) => {
                    println!("{}", json);
                    0
                }
                Err((lbl, msg)) => { tracing::error!("❌ [{}] {}", lbl, msg); 1 }
                _ => { tracing::error!("Unexpected result from runtime"); 1 }
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
                Ok(JobResult::Pong) => { println!("pong"); 0 }
                _ => 1,
            }
        }
    }
}

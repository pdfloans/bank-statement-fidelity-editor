//! Bank Statement Fidelity Editor v0.4.0
//! High-fidelity text & number editing with automatic balance reconciliation + smart targeted selection

use clap::Parser;
use dual_core_pdf_pipeline::error::exit_code;
use dual_core_pdf_pipeline::*;
use std::sync::Arc;

fn main() {
    dotenvy::dotenv().ok();

    let config = Arc::new(app::config::AppConfig::from_env().unwrap_or_else(|e| {
        eprintln!("\n❌ Configuration Error\n");
        eprintln!("{}", e);
        eprintln!("\n💡 Tip: run `dual-core-pdf-pipeline doctor` to check your full setup,");
        eprintln!("   or copy .env.example to .env and fill in the required values.\n");
        std::process::exit(exit_code::CONFIG);
    }));

    let _telemetry_guard = app::telemetry::init(&config);

    // Parse CLI early so --help works without security gate
    let cli = app::cli::Cli::parse();

    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║   Bank Statement Fidelity Editor v0.4.0                   ║");
    println!("║   100% Visual Fidelity • Smart Targeted Editing           ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");

    // Software root of trust
    if let Err(e) = security::software_root::require_software_attestation() {
        tracing::error!("[SECURITY] {}", e);
        std::process::exit(exit_code::GENERAL);
    }

    // Open Audit Log
    let audit_log = match app::audit::AuditLog::open("audit") {
        Ok(log) => log,
        Err(e) => {
            tracing::error!("[AUDIT] Failed to open audit log: {}", e);
            std::process::exit(exit_code::IO);
        }
    };

    // Start Runtime (Unified Worker)
    let (_runtime, job_tx, job_rx) = app::runtime::Runtime::start(audit_log, config.clone());

    // Dispatch to CLI module
    let code = app::cli::run(cli, job_tx, job_rx, config.clone());
    std::process::exit(code);
}

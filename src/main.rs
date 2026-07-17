//! Bank Statement Fidelity Editor v0.5.1
//! High-fidelity text & number editing with automatic balance reconciliation + smart targeted selection

use clap::Parser;
use dual_core_pdf_pipeline::error::exit_code;
use dual_core_pdf_pipeline::{app, security};
use std::sync::Arc;

fn load_dotenv() {
    // Prefer a .env file from the working tree if present.
    if dotenvy::dotenv().is_ok() {
        return;
    }

    let dotenv_candidates = [
        std::env::var("DOTENV_FILE")
            .ok()
            .map(std::path::PathBuf::from),
        find_bundle_dotenv_path(),
    ];

    for candidate in dotenv_candidates.iter().flatten() {
        if candidate.exists() {
            let _ = dotenvy::from_filename(candidate);
            return;
        }
    }
}

fn find_bundle_dotenv_path() -> Option<std::path::PathBuf> {
    let exe_path = std::env::current_exe().ok()?;
    let exe_dir = exe_path.parent()?;

    // Mac app bundle: <App>.app/Contents/MacOS/<executable>
    if let Some(contents_dir) = exe_dir.parent() {
        let candidate = contents_dir.join("Resources").join(".env");
        if candidate.exists() {
            return Some(candidate);
        }
    }

    // Fallback: .env next to the executable.
    let sibling = exe_dir.join(".env");
    if sibling.exists() {
        return Some(sibling);
    }

    None
}

fn main() {
    load_dotenv();

    // Phase 3 - Stage 10: Sentry Integration for Telemetry
    let _sentry = sentry::init((
        std::env::var("SENTRY_DSN").unwrap_or_default(),
        sentry::ClientOptions {
            release: sentry::release_name!(),
            traces_sample_rate: 0.1, // Can be increased for full tracing
            ..Default::default()
        },
    ));

    let config = Arc::new(app::config::AppConfig::from_env().unwrap_or_else(|e| {
        eprintln!("\n❌ Configuration Error\n");
        eprintln!("{e}");
        eprintln!("\n💡 Tip: run `dual-core-pdf-pipeline doctor` to check your full setup,");
        eprintln!("   or copy .env.example to .env and fill in the required values.");
        eprintln!("   On macOS app bundles, place .env into Contents/Resources/.env or launch from the project root.\n");
        std::process::exit(exit_code::CONFIG);
    }));

    let _telemetry_guard = app::telemetry::init(&config);

    // Parse CLI early so --help works without security gate
    let cli = app::cli::Cli::parse();

    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║   Bank Statement Fidelity Editor v0.5.1                   ║");
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

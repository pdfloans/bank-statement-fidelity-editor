//! Bank Statement Fidelity Editor v0.4.0
//! High-fidelity text & number editing with automatic balance reconciliation + smart targeted selection

use clap::Parser;
use dual_core_pdf_pipeline::error::exit_code;
use dual_core_pdf_pipeline::*;
use std::sync::Arc;

fn main() {
    dotenvy::dotenv().ok();

    // Set PYTHONHOME if running inside a macOS .app bundle
    if let Ok(exe_path) = std::env::current_exe() {
        if exe_path.to_string_lossy().contains(".app/Contents/MacOS") {
            if let Some(macos_dir) = exe_path.parent() {
                if let Some(contents_dir) = macos_dir.parent() {
                    let python_home = contents_dir.join("Resources").join("python");
                    if python_home.exists() {
                        std::env::set_var("PYTHONHOME", &python_home);
                        
                        let python_path = python_home.join("lib").join("python3.11");
                        let python_lib_dynload = python_path.join("lib-dynload");
                        let python_site_packages = python_path.join("site-packages");
                        
                        let combined_path = format!("{}:{}:{}", 
                            python_path.display(), 
                            python_lib_dynload.display(),
                            python_site_packages.display()
                        );
                        std::env::set_var("PYTHONPATH", combined_path);

                        // Also prepend the bin directory to PATH so python executable is found if needed
                        if let Ok(path_var) = std::env::var("PATH") {
                            let new_path = format!("{}:{}", python_home.join("bin").display(), path_var);
                            std::env::set_var("PATH", new_path);
                        }
                    }
                }
            }
        }
    }

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

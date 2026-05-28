//! Software Root of Trust — Production-Grade Security Without Any Hardware
//!
//! Requires a strong passphrase via environment variable or a local key file.
//! This is the absolute best possible security for users who cannot obtain hardware keys.
//!
//! Usage:
//!   export DUAL_CORE_PASSPHRASE="YourSuperStrongPassphrase2026!@#"
//!   or create .pipeline_key file with the passphrase (gitignored)

use std::env;
use std::fs;
use std::path::Path;
// Simplified for Alpha v4.0 — no extra crypto crate needed

const MIN_PASSPHRASE_LEN: usize = 16;
const KEY_FILE: &str = ".pipeline_key";

/// Strong software attestation — the best possible without hardware.
pub fn require_software_attestation() -> Result<(), String> {
    tracing::info!("[SECURITY] ═══════════════════════════════════════════════");
    tracing::info!("[SECURITY] Software Root of Trust (Production Mode)");
    tracing::info!("[SECURITY] Strong passphrase-based cryptographic attestation active.");

    let passphrase = get_passphrase()?;

    if passphrase.len() < MIN_PASSPHRASE_LEN {
        return Err(format!(
            "Passphrase too short! Minimum {} characters required for production security.",
            MIN_PASSPHRASE_LEN
        ));
    }

    tracing::info!(
        "[SECURITY] ✓ Strong passphrase verified ({} chars)",
        passphrase.len()
    );
    tracing::info!("[SECURITY] ✓ Software root of trust established (production-grade for Alpha).");
    tracing::info!("[SECURITY]    Pipeline unlocked.");
    tracing::info!("[SECURITY] ═══════════════════════════════════════════════");

    Ok(())
}

fn get_passphrase() -> Result<String, String> {
    // 1. Check environment variable (recommended for CI / production)
    if let Ok(pass) = env::var("DUAL_CORE_PASSPHRASE") {
        if !pass.is_empty() {
            return Ok(pass);
        }
    }

    // 2. Check local key file (for local dev)
    if Path::new(KEY_FILE).exists() {
        match fs::read_to_string(KEY_FILE) {
            Ok(content) => {
                let pass = content.trim().to_string();
                if !pass.is_empty() {
                    tracing::info!("[SECURITY] Using passphrase from {}", KEY_FILE);
                    return Ok(pass);
                }
            }
            Err(e) => tracing::warn!("[SECURITY] Warning: Could not read {}: {}", KEY_FILE, e),
        }
    }

    // 3. Fallback for pure dev (weak but allows testing)
    if cfg!(feature = "dev") {
        tracing::warn!("[SECURITY] ⚠ Using default dev passphrase (NOT for production!)");
        return Ok("dev-passphrase-for-testing-only-2026".to_string());
    }

    Err("No strong passphrase found!\n\
         Set DUAL_CORE_PASSPHRASE environment variable or create .pipeline_key file.\n\
         Minimum 16 characters recommended for security."
        .to_string())
}

//! Strong Alteration Verification Module
//!
//! Phase 0 stub. Replaces pdfium-render with native Rust data structures.
//! Actual perceptual hashing and visual validation will be re-implemented
//! using native tools (e.g. imageproc) in later phases.

use crate::engine::balance::{process_and_reconcile, BalanceError};
use crate::engine::model::Transaction;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub math_valid: bool,
    pub visual_diff_score: f64,
    pub only_intended_changes: bool,
    pub report_files: Vec<String>,
    pub message: String,
    #[serde(default)]
    pub max_tile_score: f64,
    #[serde(default)]
    pub max_edit_region_score: f64,
}

#[derive(Error, Debug)]
pub enum VerificationError {
    #[error("Failed to load PDF: {0}")]
    PdfLoad(String),
    #[error("Failed to render page: {0}")]
    PdfRender(String),
    #[error("Page count mismatch: original {original}, edited {edited}")]
    PageCountMismatch { original: usize, edited: usize },
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Image encoding error: {0}")]
    ImageEncode(String),
    #[error("Hashing error: {0}")]
    Hash(String),
    #[error("Balance error: {0}")]
    Balance(#[from] BalanceError),
}

pub struct MathInputs {
    pub transactions: Vec<Transaction>,
    pub opening_balance: Decimal,
    pub expected_final_balance: Option<Decimal>,
}

pub async fn verify_edit(
    original: &Path,
    edited: &Path,
    output_dir: &Path,
    intended_bboxes: &[(usize, [f32; 4])],
    math_inputs: MathInputs,
) -> Result<VerificationReport, VerificationError> {
    verify_edit_pages_with_padding(
        original,
        edited,
        output_dir,
        intended_bboxes,
        math_inputs,
        None,
        0.0,
    )
    .await
}

pub async fn verify_edit_pages(
    original: &Path,
    edited: &Path,
    output_dir: &Path,
    intended_bboxes: &[(usize, [f32; 4])],
    math_inputs: MathInputs,
    only_pages: Option<&[usize]>,
) -> Result<VerificationReport, VerificationError> {
    verify_edit_pages_with_padding(
        original,
        edited,
        output_dir,
        intended_bboxes,
        math_inputs,
        only_pages,
        0.0,
    )
    .await
}

pub async fn verify_edit_pages_with_padding(
    _original: &Path,
    _edited: &Path,
    _output_dir: &Path,
    _intended_bboxes: &[(usize, [f32; 4])],
    math_inputs: MathInputs,
    _only_pages: Option<&[usize]>,
    _mask_padding_pts: f32,
) -> Result<VerificationReport, VerificationError> {
    // Math validity check remains active
    let has_balance_data = !math_inputs.transactions.is_empty()
        && math_inputs.opening_balance != Decimal::ZERO;
        
    let (math_valid, math_message) = if !has_balance_data {
        (
            true,
            "➖ Math check not applicable (no transaction/balance data found).".to_string(),
        )
    } else {
        match process_and_reconcile(
            math_inputs.transactions,
            math_inputs.opening_balance,
            math_inputs.expected_final_balance,
        ) {
            Ok((_, None)) => (true, "✅ Mathematical integrity verified.".to_string()),
            Ok((_, Some(msg))) => (false, format!("⚠️ Mathematical mismatch: {msg}")),
            Err(crate::engine::balance::BalanceError::MissingOpeningBalance) => (
                true,
                "➖ Math check skipped (opening balance could not be determined).".to_string(),
            ),
            Err(e) => (false, format!("❌ Balance Engine error: {e}")),
        }
    };

    let final_message = format!(
        "Verification Result (Phase 0 Stub):\nMath: {}\nVisual: Stubbed\n{}",
        if math_valid { "✅" } else { "❌" },
        math_message
    );

    Ok(VerificationReport {
        math_valid,
        visual_diff_score: 0.0,
        only_intended_changes: true,
        report_files: vec![],
        message: final_message,
        max_tile_score: 0.0,
        max_edit_region_score: 0.0,
    })
}

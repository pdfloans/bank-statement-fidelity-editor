//! Transaction Transfer Pipeline.
//!
//! Transfers transactions from a "source" bank statement PDF to a "target"
//! bank statement PDF, intelligently adapting formats (dates, numbers,
//! descriptions, column layouts) to match the target's visual style. The
//! pipeline runs through 9 stages with live progress reporting and exhaustive
//! AI + engine verification.

use crate::engine::model::FieldBboxes;
use crate::engine::number_format::NumberFormat;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// Describes the visual and structural format of a parsed bank statement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatementFormat {
    pub bank_name: String,
    /// e.g. "DD/MM/YYYY", "MM/DD/YYYY", "YYYY-MM-DD"
    pub date_format: String,
    /// Number rendering style (currency, separators, negative convention).
    pub number_format: NumberFormat,
    /// Ordered list of columns in the transaction table.
    pub column_order: Vec<ColumnType>,
    pub has_running_balance: bool,
    pub currency_symbol: String,
    /// Estimated transaction rows that fit on a single page.
    pub rows_per_page: usize,
    /// Page header area height in PDF points (logo, account info).
    pub header_height_pts: f32,
    /// Page footer area height in PDF points.
    pub footer_height_pts: f32,
    /// Bounding box of the transaction table area on a typical page.
    pub transaction_area_bbox: [f32; 4],
    /// Primary font used in the transaction table.
    pub font_name: String,
    /// Font size in points.
    pub font_size: f32,
    /// Vertical spacing between transaction rows in points.
    pub row_height_pts: f32,
    /// Which Document AI processor version works best for this format.
    pub parser_version: Option<String>,
}

/// Column types found in bank statement transaction tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColumnType {
    Date,
    Description,
    Debit,
    Credit,
    Amount,
    Balance,
    Reference,
    ValueDate,
}

/// A fully mapped transaction ready to be written into the target PDF.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MappedTransaction {
    /// Target page index (0-based).
    pub target_page: usize,
    /// Line index within the target page.
    pub target_line: usize,
    /// Date string already converted to the target's format.
    pub date: String,
    /// Description adapted to the target's style.
    pub description: String,
    /// Debit amount (money in).
    pub debit: Option<Decimal>,
    /// Credit amount (money out).
    pub credit: Option<Decimal>,
    /// Running balance recomputed from the target's opening balance.
    pub running_balance: Decimal,
    /// Where each field should be placed on the target page.
    pub field_bboxes: FieldBboxes,
}

/// Gemini's plan for how to execute the transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferPlan {
    /// Per-transaction mapping instructions.
    pub mappings: Vec<TransactionMapping>,
    /// How many pages the output will have.
    pub output_page_count: usize,
    /// Pages from the target to clone (for extra capacity).
    pub pages_to_clone: Vec<usize>,
    /// Pages from the target to remove (excess capacity).
    pub pages_to_remove: Vec<usize>,
    /// Overall strategy description.
    pub strategy: String,
    /// Confidence score (0..1).
    pub confidence: f32,
}

/// How a single source transaction maps to the target format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionMapping {
    /// Index into the source transaction list.
    pub source_index: usize,
    /// Target page the transaction lands on.
    pub target_page: usize,
    /// Target line within that page.
    pub target_line: usize,
    /// Date converted to the target's format.
    pub converted_date: String,
    /// Description adapted to the target's convention.
    pub adapted_description: String,
}

/// Result of the entire transfer pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferResult {
    pub output_path: PathBuf,
    pub source_tx_count: usize,
    pub target_tx_count: usize,
    pub pages_added: usize,
    pub pages_removed: usize,
    pub math_verified: bool,
    pub visual_verified: bool,
    pub visual_score: f64,
    pub math_imbalance: Decimal,
    pub stages_completed: u8,
    pub total_duration_secs: f64,
    pub corrections_applied: usize,
    pub retries_attempted: usize,
    pub synthesized_fonts_used: bool,
}

/// Tracks which stage the pipeline is currently executing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferStage {
    AnalyzeSource,
    AnalyzeTarget,
    AiFormatMapping,
    ComputeBalances,
    PdfSurgery,
    VisualFidelityCheck,
    MathVerificationEngine,
    MathVerificationGemini,
    FinalAudit,
}

impl TransferStage {
    pub fn label(&self) -> &'static str {
        match self {
            Self::AnalyzeSource => "Analyzing source statement...",
            Self::AnalyzeTarget => "Analyzing target statement...",
            Self::AiFormatMapping => "AI mapping transaction formats...",
            Self::ComputeBalances => "Computing balances...",
            Self::PdfSurgery => "Applying PDF changes...",
            Self::VisualFidelityCheck => "Verifying visual fidelity...",
            Self::MathVerificationEngine => "Verifying math (engine)...",
            Self::MathVerificationGemini => "Verifying math (AI)...",
            Self::FinalAudit => "Writing audit report...",
        }
    }

    /// Progress fraction range [start, end) for this stage.
    pub fn fraction_range(&self) -> (f32, f32) {
        match self {
            Self::AnalyzeSource => (0.00, 0.10),
            Self::AnalyzeTarget => (0.10, 0.20),
            Self::AiFormatMapping => (0.20, 0.30),
            Self::ComputeBalances => (0.30, 0.35),
            Self::PdfSurgery => (0.35, 0.55),
            Self::VisualFidelityCheck => (0.55, 0.75),
            Self::MathVerificationEngine => (0.75, 0.85),
            Self::MathVerificationGemini => (0.85, 0.95),
            Self::FinalAudit => (0.95, 1.00),
        }
    }
}

/// Recompute running balances from an opening balance and a set of
/// transactions (using the codebase's sign convention: debit = money in,
/// credit = money out).
pub fn recompute_running_balances(opening: Decimal, txns: &mut [MappedTransaction]) {
    let mut balance = opening;
    for tx in txns.iter_mut() {
        let delta_in = tx.debit.unwrap_or(Decimal::ZERO);
        let delta_out = tx.credit.unwrap_or(Decimal::ZERO);
        balance = balance + delta_in - delta_out;
        tx.running_balance = balance.round_dp(2);
    }
}

/// Convert a date string from one format to another.
/// Supports DD/MM/YYYY, MM/DD/YYYY, YYYY-MM-DD.
pub fn convert_date(date_str: &str, from_format: &str, to_format: &str) -> String {
    if from_format == to_format {
        return date_str.to_string();
    }

    // Parse the date components based on from_format
    let parts: Vec<&str> = if date_str.contains('/') {
        date_str.split('/').collect()
    } else if date_str.contains('-') {
        date_str.split('-').collect()
    } else {
        return date_str.to_string(); // can't parse, return as-is
    };

    if parts.len() < 3 {
        return date_str.to_string();
    }

    let (day, month, year) = match from_format {
        "DD/MM/YYYY" | "DD-MM-YYYY" => (parts[0], parts[1], parts[2]),
        "MM/DD/YYYY" | "MM-DD-YYYY" => (parts[1], parts[0], parts[2]),
        "YYYY-MM-DD" | "YYYY/MM/DD" => (parts[2], parts[1], parts[0]),
        _ => return date_str.to_string(),
    };

    let sep = if to_format.contains('/') { "/" } else { "-" };

    match to_format {
        "DD/MM/YYYY" | "DD-MM-YYYY" => format!("{day}{sep}{month}{sep}{year}"),
        "MM/DD/YYYY" | "MM-DD-YYYY" => format!("{month}{sep}{day}{sep}{year}"),
        "YYYY-MM-DD" | "YYYY/MM/DD" => format!("{year}{sep}{month}{sep}{day}"),
        _ => date_str.to_string(),
    }
}

/// Build a JSON audit report for the transfer operation.
pub fn write_transfer_audit(
    result: &TransferResult,
    source_path: &std::path::Path,
    target_path: &std::path::Path,
) -> std::io::Result<PathBuf> {
    let audit_dir = PathBuf::from("audit/transfers");
    std::fs::create_dir_all(&audit_dir)?;

    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let audit_path = audit_dir.join(format!("transfer_{timestamp}.json"));

    let report = serde_json::json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "source_pdf": source_path.to_string_lossy(),
        "target_pdf": target_path.to_string_lossy(),
        "output_pdf": result.output_path.to_string_lossy(),
        "source_tx_count": result.source_tx_count,
        "target_tx_count": result.target_tx_count,
        "pages_added": result.pages_added,
        "pages_removed": result.pages_removed,
        "math_verified": result.math_verified,
        "visual_verified": result.visual_verified,
        "visual_score": result.visual_score,
        "math_imbalance": result.math_imbalance.to_string(),
        "stages_completed": result.stages_completed,
        "total_duration_secs": result.total_duration_secs,
        "corrections_applied": result.corrections_applied,
        "retries_attempted": result.retries_attempted,
        "synthesized_fonts_used": result.synthesized_fonts_used,
    });

    let pretty = serde_json::to_string_pretty(&report)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&audit_path, pretty)?;
    Ok(audit_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn recompute_balances_from_opening() {
        let mut txns = vec![
            MappedTransaction {
                target_page: 0,
                target_line: 0,
                date: "01/01/2026".into(),
                description: "Deposit".into(),
                debit: Some(dec!(500)),
                credit: None,
                running_balance: Decimal::ZERO,
                field_bboxes: FieldBboxes::default(),
            },
            MappedTransaction {
                target_page: 0,
                target_line: 1,
                date: "02/01/2026".into(),
                description: "Withdrawal".into(),
                debit: None,
                credit: Some(dec!(200)),
                running_balance: Decimal::ZERO,
                field_bboxes: FieldBboxes::default(),
            },
        ];

        recompute_running_balances(dec!(1000), &mut txns);

        assert_eq!(txns[0].running_balance, dec!(1500.00));
        assert_eq!(txns[1].running_balance, dec!(1300.00));
    }

    #[test]
    fn convert_date_dd_mm_to_mm_dd() {
        assert_eq!(
            convert_date("25/12/2026", "DD/MM/YYYY", "MM/DD/YYYY"),
            "12/25/2026"
        );
    }

    #[test]
    fn convert_date_mm_dd_to_yyyy_mm_dd() {
        assert_eq!(
            convert_date("12/25/2026", "MM/DD/YYYY", "YYYY-MM-DD"),
            "2026-12-25"
        );
    }

    #[test]
    fn convert_date_same_format_is_identity() {
        assert_eq!(
            convert_date("25/12/2026", "DD/MM/YYYY", "DD/MM/YYYY"),
            "25/12/2026"
        );
    }

    #[test]
    fn transfer_stage_labels_all_defined() {
        let stages = [
            TransferStage::AnalyzeSource,
            TransferStage::AnalyzeTarget,
            TransferStage::AiFormatMapping,
            TransferStage::ComputeBalances,
            TransferStage::PdfSurgery,
            TransferStage::VisualFidelityCheck,
            TransferStage::MathVerificationEngine,
            TransferStage::MathVerificationGemini,
            TransferStage::FinalAudit,
        ];
        for s in stages {
            assert!(!s.label().is_empty());
            let (lo, hi) = s.fraction_range();
            assert!(lo < hi);
        }
    }
}

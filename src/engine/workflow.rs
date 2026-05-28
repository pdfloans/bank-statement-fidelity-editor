//! Multi-stage edit workflow.
//!
//! Stages (each gated by user action in the GUI):
//!
//! 1. `Parse`        — Document AI extracts a [`BankStatement`]; Gemini does a
//!                     completeness check ("did the parser capture all the rows
//!                     visible on the page?"). The result is a
//!                     [`ParseValidation`].
//! 2. `Edit`         — user edits any number of values. The app holds a
//!                     [`Vec<UserEdit>`] until they request a preview.
//! 3. `Preview`      — recompute every running balance from the user's edits,
//!                     produce a [`BalancePreview`] with a per-row diff and a
//!                     final imbalance number.
//! 4. `Render`       — for each accepted edit, call into the existing
//!                     `apply_change` path which already does
//!                     binary-level / supplied-font / structured-failure.
//! 5. `Validate`     — compare the rendered output to the page rendered with
//!                     target values overlaid. If the perceptual diff is
//!                     above a threshold the stage retries with
//!                     `tolerance_pixels` widened up to `max_attempts`.
//! 6. `FinalParse`   — re-run Document AI on the rendered output and verify
//!                     all amounts, balances and the running ledger are
//!                     mathematically consistent.
//!
//! Each stage is a pure data transformation; the runtime owns the I/O
//! (network, files, Python actor). This lets the lib tests cover the
//! state machine without mocking the world.

use serde::{Deserialize, Serialize};

/// One user-driven change inside the Edit stage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserEdit {
    /// Page index (0-based) of the row.
    pub page: usize,
    /// Row index inside that page (matches `Transaction::line_on_page`).
    pub line_on_page: usize,
    /// Bounding box of the field being edited; required for the apply step.
    pub bbox: [f32; 4],
    /// Old text exactly as Document AI extracted it.
    pub old_text: String,
    /// New text the user typed.
    pub new_text: String,
    /// Which field on the row the user is changing.
    pub field: EditField,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum EditField {
    Date,
    Description,
    Debit,
    Credit,
    RunningBalance,
}

/// The state machine the GUI walks through.
#[derive(Debug, Clone, PartialEq)]
pub enum WorkflowStage {
    /// Nothing loaded.
    Idle,
    /// Document AI parse + Gemini completeness check is running.
    Parsing,
    /// Parsing done, the user is editing.
    Editing(ParseValidation),
    /// "Balance Out Preview" — recompute and present diffs.
    Previewing(BalancePreview),
    /// "Confirm and Render" — apply edits to the PDF.
    Rendering { attempt: u32 },
    /// Render finished; visual validation is running.
    Validating(VisualAttempt),
    /// Final Document AI re-parse to confirm math integrity.
    FinalChecking,
    /// Done — the bank statement is confirmed correct.
    Complete(WorkflowOutcome),
    /// Terminal failure with a structured reason.
    Failed(WorkflowFailure),
}

/// Result of stage 1 — what DocAI got + Gemini's opinion of completeness.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ParseValidation {
    pub total_pages: usize,
    pub transactions_found: usize,
    pub opening_balance: f64,
    pub closing_balance: f64,
    pub account_number: Option<String>,
    /// Gemini score of "did the parser get everything?" 0..1.
    pub completeness_score: f32,
    pub completeness_notes: String,
    /// Anything Gemini saw on the page that DocAI didn't capture.
    pub missing_rows: Vec<String>,
}

impl ParseValidation {
    /// Whether the parse is good enough to proceed without re-parsing.
    pub fn is_acceptable(&self) -> bool {
        self.completeness_score >= 0.85 && self.transactions_found > 0
    }
}

/// Cross-check signal from a deterministic geometry extractor (e.g.
/// [`crate::extractors::BankTemplateProvider`]). When the template extractor
/// finds *materially more rows* than Document AI did, the Gemini-supplied
/// completeness score is multiplied down to reflect that mismatch.
///
/// Concretely:
///   * `delta = template_row_count - docai_row_count`
///   * `delta <= 1` → no penalty (rounding tolerance)
///   * `delta > 1` → multiply score by 0.7, surface the discrepancy in notes
///
/// `template_row_count == 0` (no template matched) means we have no signal
/// either way, so the score is left alone. This is Stage 2 / Item #11.
pub fn cross_validate_with_template(
    mut validation: ParseValidation,
    template_row_count: usize,
) -> ParseValidation {
    if template_row_count == 0 {
        return validation;
    }
    let docai = validation.transactions_found;
    let delta = template_row_count as i64 - docai as i64;
    if delta > 1 {
        let original = validation.completeness_score;
        validation.completeness_score = (original * 0.7).clamp(0.0, 1.0);
        let note = format!(
            " [template cross-check: extractor found {template_row_count} rows, \
             Document AI returned {docai}; completeness reduced from {original:.2} \
             to {:.2}]",
            validation.completeness_score
        );
        validation.completeness_notes.push_str(&note);
        validation
            .missing_rows
            .push(format!("template detected {} additional row(s)", delta));
    }
    validation
}

/// Result of stage 3 — every row, with the user's edits applied, plus the
/// running balance recomputed top-to-bottom.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BalancePreview {
    pub rows: Vec<PreviewRow>,
    /// Sum of credits - sum of debits + opening balance, vs. reported closing.
    pub final_imbalance: f64,
    /// True when the final imbalance is < 0.01 (typical accounting tolerance).
    pub balanced: bool,
    /// Auto-correction message if the engine offered to nudge the last row.
    pub auto_correction_message: Option<String>,
}

impl BalancePreview {
    /// Pages that contain at least one row whose value will change. The
    /// visual-validation loop in Stage 5 uses this to render and diff only
    /// the affected pages, avoiding a full-document re-render on every
    /// retry. (Stage 2 / Item #2.)
    pub fn changed_pages(&self) -> Vec<usize> {
        let mut pages: Vec<usize> = self
            .rows
            .iter()
            .filter(|r| r.will_change)
            .map(|r| r.page)
            .collect();
        pages.sort_unstable();
        pages.dedup();
        pages
    }

    /// Number of rows the renderer will redraw.
    pub fn changed_row_count(&self) -> usize {
        self.rows.iter().filter(|r| r.will_change).count()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreviewRow {
    pub page: usize,
    pub line_on_page: usize,
    pub date: String,
    pub description: String,
    pub debit: Option<f64>,
    pub credit: Option<f64>,
    /// Old running balance from the original parse.
    pub old_running_balance: Option<f64>,
    /// Newly-computed running balance after applying the user's edits.
    pub new_running_balance: Option<f64>,
    /// True when this row will be redrawn in the rendered PDF.
    pub will_change: bool,
}

/// One iteration of the visual-validation loop.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VisualAttempt {
    pub attempt: u32,
    pub max_attempts: u32,
    /// Normalised perceptual diff (0 = identical, 1 = max difference).
    pub diff_score: f64,
    /// Threshold below which we declare success (defaults to 0.02).
    pub threshold: f64,
    /// Were only the intended bboxes affected?
    pub only_intended: bool,
    pub message: String,
}

impl VisualAttempt {
    pub fn passed(&self) -> bool {
        self.diff_score < self.threshold && self.only_intended
    }
}

/// Result of the final DocAI re-parse.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowOutcome {
    pub final_pdf: std::path::PathBuf,
    pub transactions_re_parsed: usize,
    pub final_imbalance: f64,
    pub math_valid: bool,
    pub visual_attempts: u32,
    pub completion_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WorkflowFailure {
    /// DocAI couldn't parse, or found nothing.
    ParseFailed(String),
    /// Gemini rejected the parse as incomplete (score < threshold).
    Incomplete {
        score: f32,
        notes: String,
    },
    /// Editor returned `FONT_COVERAGE_INSUFFICIENT` and deep replication didn't
    /// produce a usable font.
    FontCoverageFailed {
        missing_chars: Vec<String>,
    },
    /// The visual validation loop hit `max_attempts` without converging.
    VisualNotConverged {
        last_score: f64,
        attempts: u32,
    },
    /// Final DocAI re-parse said the result is not mathematically correct.
    FinalMathInvalid {
        imbalance: f64,
    },
    Other(String),
}

impl WorkflowStage {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Parsing => "Parse + AI validate",
            Self::Editing(_) => "Edit",
            Self::Previewing(_) => "Balance preview",
            Self::Rendering { .. } => "Render",
            Self::Validating(_) => "Visual validate",
            Self::FinalChecking => "Final math check",
            Self::Complete(_) => "Complete",
            Self::Failed(_) => "Failed",
        }
    }

    pub fn step_index(&self) -> u8 {
        match self {
            Self::Idle => 0,
            Self::Parsing => 1,
            Self::Editing(_) => 2,
            Self::Previewing(_) => 3,
            Self::Rendering { .. } => 4,
            Self::Validating(_) => 5,
            Self::FinalChecking => 6,
            Self::Complete(_) => 7,
            Self::Failed(_) => 8,
        }
    }
}

// ---------------------------------------------------------------------------
// Stage 3 — pure preview computation. Lives here so we can unit-test it.
// ---------------------------------------------------------------------------

use crate::engine::balance::process_and_reconcile;
use crate::engine::model::Transaction;

/// Apply `edits` to `original` and rebuild the running ledger. Returns the
/// per-row diff plus the final imbalance.
pub fn build_preview(
    original: &[Transaction],
    edits: &[UserEdit],
    opening_balance: f64,
    expected_closing: Option<f64>,
) -> Result<BalancePreview, String> {
    // 1. Clone, apply edits in place.
    let mut working: Vec<Transaction> = original.to_vec();
    for e in edits {
        // Find the matching row by (page, line_on_page); skip silently if
        // there's no match (the edit was on a row that no longer exists).
        let Some(row) = working
            .iter_mut()
            .find(|t| t.page == e.page && t.line_on_page == e.line_on_page)
        else {
            continue;
        };
        match e.field {
            EditField::Date => row.date = e.new_text.clone(),
            EditField::Description => row.raw_text = e.new_text.clone(),
            EditField::Debit => {
                row.debit = parse_money(&e.new_text);
                if row.debit.is_some() {
                    row.credit = None;
                }
            }
            EditField::Credit => {
                row.credit = parse_money(&e.new_text);
                if row.credit.is_some() {
                    row.debit = None;
                }
            }
            EditField::RunningBalance => row.running_balance = parse_money(&e.new_text),
        }
    }

    // 2. Recompute running balances.
    let (recomputed, msg) =
        process_and_reconcile(working.clone(), opening_balance, expected_closing)
            .map_err(|e| e.to_string())?;

    // 3. Build per-row preview.
    let mut rows = Vec::with_capacity(recomputed.len());
    for (orig, new) in original.iter().zip(recomputed.iter()) {
        let will_change = new.running_balance != orig.running_balance
            || edits
                .iter()
                .any(|e| e.page == new.page && e.line_on_page == new.line_on_page);
        rows.push(PreviewRow {
            page: new.page,
            line_on_page: new.line_on_page,
            date: new.date.clone(),
            description: new.raw_text.clone(),
            debit: new.debit,
            credit: new.credit,
            old_running_balance: orig.running_balance,
            new_running_balance: new.running_balance,
            will_change,
        });
    }

    // 4. Compute final imbalance from recomputed last row vs expected closing.
    let computed_final = rows
        .last()
        .and_then(|r| r.new_running_balance)
        .unwrap_or(0.0);
    let final_imbalance = expected_closing
        .map(|e| (computed_final - e) * 100.0)
        .map(|v| v.round() / 100.0)
        .unwrap_or(0.0);

    Ok(BalancePreview {
        rows,
        final_imbalance,
        balanced: final_imbalance.abs() < 0.01,
        auto_correction_message: msg,
    })
}

fn parse_money(s: &str) -> Option<f64> {
    let cleaned: String = s
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '-' || *c == '.')
        .collect();
    cleaned.parse().ok()
}

/// Drop edits whose typed value already matches the cascade's output.
///
/// Stage 2 / Item #7: when a user edits debit on row 1 *and* the running
/// balance on row 5 to the value that the cascade would produce anyway,
/// applying both edits is redundant — it just adds visual noise (extra
/// redactions) without changing the document. We prune those after the
/// preview is built, returning the edits to actually apply plus a list of
/// the ones we dropped (for auditability).
///
/// Only `EditField::RunningBalance` edits are subject to this rule; other
/// fields (Date, Description, Debit, Credit) always go through because we
/// can't infer redundancy from the cascade alone.
pub fn prune_redundant_edits(
    edits: &[UserEdit],
    preview: &BalancePreview,
) -> (Vec<UserEdit>, Vec<UserEdit>) {
    let mut kept = Vec::with_capacity(edits.len());
    let mut dropped = Vec::new();
    for e in edits {
        if e.field != EditField::RunningBalance {
            kept.push(e.clone());
            continue;
        }
        let typed = match parse_money(&e.new_text) {
            Some(v) => v,
            None => {
                kept.push(e.clone());
                continue;
            }
        };
        let cascaded = preview
            .rows
            .iter()
            .find(|r| r.page == e.page && r.line_on_page == e.line_on_page)
            .and_then(|r| r.new_running_balance);
        match cascaded {
            Some(c) if (c - typed).abs() < 0.01 => {
                tracing::debug!(
                    "[workflow] dropping redundant edit on P{} L{}: typed={:.2} cascaded={:.2}",
                    e.page,
                    e.line_on_page,
                    typed,
                    c
                );
                dropped.push(e.clone());
            }
            _ => kept.push(e.clone()),
        }
    }
    (kept, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::model::Provenance;

    fn tx(
        page: usize,
        line: usize,
        debit: Option<f64>,
        credit: Option<f64>,
        bal: Option<f64>,
    ) -> Transaction {
        Transaction {
            page,
            line_on_page: line,
            date: "01/01/2026".into(),
            raw_text: "Test".into(),
            debit,
            credit,
            running_balance: bal,
            bbox: None,
            provenance: Provenance::Manual,
        }
    }

    #[test]
    fn build_preview_propagates_a_single_credit_edit_through_running_balance() {
        // NB: this codebase uses balance = balance + debit - credit (i.e. debit is
        // money in, credit is money out — see engine/balance.rs::recalculate_and_validate).
        // Two transactions starting from opening 100:
        //   row 0: debit 100 -> balance 200
        //   row 1: credit 50 -> balance 150
        let original = vec![
            tx(0, 0, Some(100.0), None, Some(200.0)),
            tx(0, 1, None, Some(50.0), Some(150.0)),
        ];
        // User changes row 0's debit from 100 to 200.
        let edits = vec![UserEdit {
            page: 0,
            line_on_page: 0,
            bbox: [0.0; 4],
            old_text: "100.00".into(),
            new_text: "200.00".into(),
            field: EditField::Debit,
        }];

        let preview = build_preview(&original, &edits, 100.0, None).unwrap();

        // Row 0: debit changed 100 -> 200, balance recomputes 100 + 200 = 300
        assert_eq!(preview.rows[0].debit, Some(200.0));
        assert_eq!(preview.rows[0].new_running_balance, Some(300.0));
        // Row 1 cascades: 300 - 50 = 250
        assert_eq!(preview.rows[1].new_running_balance, Some(250.0));
        // Old balance is preserved for the diff display
        assert_eq!(preview.rows[0].old_running_balance, Some(200.0));
        // Both rows are flagged as changed (row 0 directly, row 1 by cascade)
        assert!(preview.rows[0].will_change);
        assert!(preview.rows[1].will_change);
    }

    #[test]
    fn build_preview_marks_balanced_when_final_matches_expected() {
        // opening 100 + debit 100 = 200 closing
        let original = vec![tx(0, 0, Some(100.0), None, Some(200.0))];
        let preview = build_preview(&original, &[], 100.0, Some(200.0)).unwrap();
        assert!(preview.balanced);
        assert_eq!(preview.final_imbalance, 0.0);
    }

    #[test]
    fn parse_validation_acceptable_threshold_works() {
        let v = ParseValidation {
            total_pages: 1,
            transactions_found: 5,
            opening_balance: 0.0,
            closing_balance: 0.0,
            account_number: None,
            completeness_score: 0.86,
            completeness_notes: String::new(),
            missing_rows: vec![],
        };
        assert!(v.is_acceptable());

        let bad = ParseValidation {
            completeness_score: 0.5,
            ..v
        };
        assert!(!bad.is_acceptable());
    }

    fn validation(score: f32, found: usize) -> ParseValidation {
        ParseValidation {
            total_pages: 1,
            transactions_found: found,
            opening_balance: 0.0,
            closing_balance: 0.0,
            account_number: None,
            completeness_score: score,
            completeness_notes: String::new(),
            missing_rows: vec![],
        }
    }

    #[test]
    fn cross_validate_no_template_match_leaves_score_alone() {
        let v = validation(0.95, 10);
        let out = cross_validate_with_template(v.clone(), 0);
        assert_eq!(out.completeness_score, 0.95);
        assert!(out.missing_rows.is_empty());
    }

    #[test]
    fn cross_validate_template_agrees_leaves_score_alone() {
        let v = validation(0.95, 10);
        let out = cross_validate_with_template(v, 10);
        assert!((out.completeness_score - 0.95).abs() < 1e-6);
    }

    #[test]
    fn cross_validate_template_one_extra_row_is_within_tolerance() {
        let v = validation(0.95, 10);
        let out = cross_validate_with_template(v, 11);
        assert!((out.completeness_score - 0.95).abs() < 1e-6);
        assert!(out.missing_rows.is_empty());
    }

    #[test]
    fn cross_validate_template_three_extra_rows_drops_score_to_0_665() {
        // 0.95 * 0.7 = 0.665 — matches the Stage 2 acceptance criterion exactly.
        let v = validation(0.95, 8);
        let out = cross_validate_with_template(v, 11);
        assert!(
            (out.completeness_score - 0.665).abs() < 1e-3,
            "expected ~0.665, got {}",
            out.completeness_score
        );
        assert!(out.completeness_notes.contains("template cross-check"));
        assert_eq!(out.missing_rows.len(), 1);
        assert!(out.missing_rows[0].contains("3"));
    }

    #[test]
    fn cross_validate_clamps_to_unit_interval() {
        let v = validation(0.0, 0);
        let out = cross_validate_with_template(v, 5);
        assert!(out.completeness_score >= 0.0 && out.completeness_score <= 1.0);
    }

    fn preview_row(page: usize, line: usize, will_change: bool) -> PreviewRow {
        PreviewRow {
            page,
            line_on_page: line,
            date: "01/01/2026".into(),
            description: "test".into(),
            debit: None,
            credit: None,
            old_running_balance: Some(100.0),
            new_running_balance: Some(100.0),
            will_change,
        }
    }

    #[test]
    fn changed_pages_returns_unique_sorted_pages_with_changes() {
        let preview = BalancePreview {
            rows: vec![
                preview_row(0, 0, false),
                preview_row(2, 1, true),
                preview_row(0, 1, true),
                preview_row(2, 2, true),
                preview_row(5, 0, true),
            ],
            final_imbalance: 0.0,
            balanced: true,
            auto_correction_message: None,
        };
        assert_eq!(preview.changed_pages(), vec![0, 2, 5]);
        assert_eq!(preview.changed_row_count(), 4);
    }

    #[test]
    fn changed_pages_empty_when_no_rows_changed() {
        let preview = BalancePreview {
            rows: vec![preview_row(0, 0, false), preview_row(1, 0, false)],
            final_imbalance: 0.0,
            balanced: true,
            auto_correction_message: None,
        };
        assert!(preview.changed_pages().is_empty());
        assert_eq!(preview.changed_row_count(), 0);
    }

    fn make_user_edit(page: usize, line: usize, new_text: &str, field: EditField) -> UserEdit {
        UserEdit {
            page,
            line_on_page: line,
            bbox: [0.0; 4],
            old_text: "old".into(),
            new_text: new_text.into(),
            field,
        }
    }

    #[test]
    fn prune_redundant_edits_drops_running_balance_edits_that_match_cascade() {
        // Cascade says row 5's new balance is 250.00, and the user typed 250.00.
        let preview = BalancePreview {
            rows: vec![PreviewRow {
                page: 0,
                line_on_page: 5,
                date: "01/01/2026".into(),
                description: "test".into(),
                debit: None,
                credit: None,
                old_running_balance: Some(100.0),
                new_running_balance: Some(250.0),
                will_change: true,
            }],
            final_imbalance: 0.0,
            balanced: true,
            auto_correction_message: None,
        };
        let edits = vec![
            make_user_edit(0, 1, "200.00", EditField::Debit),
            make_user_edit(0, 5, "250.00", EditField::RunningBalance),
        ];
        let (kept, dropped) = prune_redundant_edits(&edits, &preview);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].field, EditField::Debit);
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].field, EditField::RunningBalance);
    }

    #[test]
    fn prune_redundant_edits_keeps_running_balance_edits_that_differ_from_cascade() {
        let preview = BalancePreview {
            rows: vec![PreviewRow {
                page: 0,
                line_on_page: 5,
                date: "01/01/2026".into(),
                description: "test".into(),
                debit: None,
                credit: None,
                old_running_balance: Some(100.0),
                new_running_balance: Some(250.0),
                will_change: true,
            }],
            final_imbalance: 0.0,
            balanced: true,
            auto_correction_message: None,
        };
        // User typed 999.99, cascade says 250.00 — keep the edit; the user
        // is trying to override the cascade.
        let edits = vec![make_user_edit(0, 5, "999.99", EditField::RunningBalance)];
        let (kept, dropped) = prune_redundant_edits(&edits, &preview);
        assert_eq!(kept.len(), 1);
        assert!(dropped.is_empty());
    }

    #[test]
    fn prune_redundant_edits_never_drops_non_running_balance_fields() {
        let preview = BalancePreview {
            rows: vec![preview_row(0, 0, true)],
            final_imbalance: 0.0,
            balanced: true,
            auto_correction_message: None,
        };
        let edits = vec![
            make_user_edit(0, 0, "01/01/2026", EditField::Date),
            make_user_edit(0, 0, "Coffee", EditField::Description),
            make_user_edit(0, 0, "100.00", EditField::Debit),
            make_user_edit(0, 0, "100.00", EditField::Credit),
        ];
        let (kept, dropped) = prune_redundant_edits(&edits, &preview);
        assert_eq!(kept.len(), 4);
        assert!(dropped.is_empty());
    }

    #[test]
    fn visual_attempt_passes_only_when_under_threshold_and_intended() {
        let pass = VisualAttempt {
            attempt: 1,
            max_attempts: 5,
            diff_score: 0.01,
            threshold: 0.02,
            only_intended: true,
            message: "ok".into(),
        };
        assert!(pass.passed());

        let fail_score = VisualAttempt {
            diff_score: 0.05,
            ..pass.clone()
        };
        assert!(!fail_score.passed());

        let fail_unintended = VisualAttempt {
            only_intended: false,
            ..pass
        };
        assert!(!fail_unintended.passed());
    }
}

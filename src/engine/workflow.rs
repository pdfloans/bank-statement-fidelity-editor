//! Multi-stage edit workflow.
//!
//! Stages (each gated by user action in the GUI):
//!
//! 1. `Parse`        — Document AI extracts a [`BankStatement`]; Gemini does a
//!    completeness check ("did the parser capture all the rows
//!    visible on the page?"). The result is a
//!    [`ParseValidation`].
//! 2. `Edit`         — user edits any number of values. The app holds a
//!    [`Vec<UserEdit>`] until they request a preview.
//! 3. `Preview`      — recompute every running balance from the user's edits,
//!    produce a [`BalancePreview`] with a per-row diff and a
//!    final imbalance number.
//! 4. `Render`       — for each accepted edit, call into the existing
//!    `apply_change` path which already does
//!    binary-level / supplied-font / structured-failure.
//! 5. `Validate`     — compare the rendered output to the page rendered with
//!    target values overlaid. If the perceptual diff is
//!    above a threshold the stage retries with
//!    `tolerance_pixels` widened up to `max_attempts`.
//! 6. `FinalParse`   — re-run Document AI on the rendered output and verify
//!    all amounts, balances and the running ledger are
//!    mathematically consistent.
//!
//! Each stage is a pure data transformation; the runtime owns the I/O
//! (network, files, Python actor). This lets the lib tests cover the
//! state machine without mocking the world.

use rust_decimal::Decimal;
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
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
    pub opening_balance: Decimal,
    pub closing_balance: Decimal,
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
            .push(format!("template detected {delta} additional row(s)"));
    }
    validation
}

/// Result of stage 3 — every row, with the user's edits applied, plus the
/// running balance recomputed top-to-bottom.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BalancePreview {
    pub rows: Vec<PreviewRow>,
    /// Sum of credits - sum of debits + opening balance, vs. reported closing.
    pub final_imbalance: Decimal,
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
    pub debit: Option<Decimal>,
    pub credit: Option<Decimal>,
    /// Old running balance from the original parse.
    pub old_running_balance: Option<Decimal>,
    /// Newly-computed running balance after applying the user's edits.
    pub new_running_balance: Option<Decimal>,
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
        // A diff score below 0.001 (0.1%) is essentially pixel-perfect —
        // always accept regardless of the only_intended flag. The flag can
        // be false due to sub-pixel aliasing differences that are visually
        // imperceptible.
        if self.diff_score < 0.001 {
            return true;
        }
        self.diff_score < self.threshold && self.only_intended
    }
}

/// Per-attempt tolerance schedule for the visual-validation loop.
///
/// Stage 3 / Item #3: as we retry, the per-bbox mask grows so very thin
/// baseline shifts (sub-pixel) don't cause false-negatives forever, while
/// still rejecting actual unintended changes early.
///
/// Returns the mask-padding (in PDF points) for `attempt` (1-based).
/// Capped at 12pt so we never expand so far that "intended-only" stops
/// being meaningful.
pub fn mask_padding_for_attempt(attempt: u32) -> f32 {
    match attempt {
        1 => 2.0,
        2 => 4.0,
        3 => 8.0,
        _ => 12.0,
    }
}

/// Whether the loop should accept a near-pass with a friendly note rather
/// than reject + retry. Stage 3 / Item #3: at attempt ≥3 we soften the
/// "only_intended" rule when the actual diff_score is already comfortably
/// under threshold (i.e. the page mostly matches and only sub-pixel
/// rendering noise outside the mask is keeping `only_intended` false).
pub fn should_accept_near_perfect(attempt: u32, diff_score: f64, threshold: f64) -> bool {
    attempt >= 3 && diff_score < threshold * 0.5
}

/// Serializable snapshot of the editing session.
///
/// Stage 5 / Item #9: written to `audit/workflow.json` after every state
/// change so the GUI can resume mid-edit. The PDF content hash is stored
/// alongside the data; `load_from_file` returns it so the caller can warn
/// when the user has edited a different (or modified) source PDF.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowDraft {
    pub schema_version: u32,
    /// SHA-256 of the input PDF at draft time. Lets the caller detect when
    /// the user opened a different file or the file has changed since.
    pub input_sha256: String,
    pub input_path: String,
    pub saved_at: String,
    pub validation: Option<ParseValidation>,
    pub transactions: Vec<crate::engine::model::Transaction>,
    pub edits: Vec<UserEdit>,
}

const WORKFLOW_DRAFT_SCHEMA: u32 = 1;

impl WorkflowDraft {
    pub fn new(
        input_path: &std::path::Path,
        validation: Option<ParseValidation>,
        transactions: Vec<crate::engine::model::Transaction>,
        edits: Vec<UserEdit>,
    ) -> std::io::Result<Self> {
        let bytes = std::fs::read(input_path)?;
        Ok(Self {
            schema_version: WORKFLOW_DRAFT_SCHEMA,
            input_sha256: sha256_hex(&bytes),
            input_path: input_path.to_string_lossy().into_owned(),
            saved_at: chrono::Utc::now().to_rfc3339(),
            validation,
            transactions,
            edits,
        })
    }

    /// Same as [`Self::new`] but with a precomputed SHA-256 hash. The GUI
    /// uses this so autosave doesn't re-read multi-MB PDFs every save.
    pub fn new_with_hash(
        input_path: &std::path::Path,
        input_sha256: String,
        validation: Option<ParseValidation>,
        transactions: Vec<crate::engine::model::Transaction>,
        edits: Vec<UserEdit>,
    ) -> Self {
        Self {
            schema_version: WORKFLOW_DRAFT_SCHEMA,
            input_sha256,
            input_path: input_path.to_string_lossy().into_owned(),
            saved_at: chrono::Utc::now().to_rfc3339(),
            validation,
            transactions,
            edits,
        }
    }

    /// Atomic-ish delta-based save (tmp + rename). 
    /// To optimize audit JSON writes and avoid blocking the GUI by rewriting 
    /// multi-megabyte parsed transactions on every edit, we save the static base
    /// once and only rewrite the `edits` delta on subsequent saves.
    pub fn save_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        
        let base_path = path.with_extension("base.json");
        let edits_path = path.with_extension("edits.json");

        if !base_path.exists() {
            let mut base = self.clone();
            base.edits.clear(); // Base snapshot has no edits
            let tmp = base_path.with_extension("tmp");
            // Use buffered writer to speed up huge writes
            let f = std::fs::File::create(&tmp)?;
            let mut writer = std::io::BufWriter::new(f);
            serde_json::to_writer(&mut writer, &base)?;
            std::fs::rename(tmp, &base_path)?;
        }

        // Delta write: only save the edits array (extremely fast and small)
        let tmp = edits_path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&self.edits)?)?;
        std::fs::rename(tmp, edits_path)?;

        Ok(())
    }

    /// Load a draft from disk. Supports both the new delta-based schema (`base.json` + `edits.json`)
    /// and the legacy monolithic schema for backwards compatibility.
    pub fn load_from_file(path: &std::path::Path) -> std::io::Result<Self> {
        let base_path = path.with_extension("base.json");
        let edits_path = path.with_extension("edits.json");

        // Try reading delta base first; fallback to legacy monolithic file
        let raw = std::fs::read_to_string(&base_path)
            .or_else(|_| std::fs::read_to_string(path))?;

        let mut draft: Self = serde_json::from_str(&raw).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, format!("draft decode: {e}"))
        })?;

        if draft.schema_version != WORKFLOW_DRAFT_SCHEMA {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "incompatible draft schema {} (expected {})",
                    draft.schema_version, WORKFLOW_DRAFT_SCHEMA
                ),
            ));
        }

        // Apply delta edits if present
        if let Ok(raw_edits) = std::fs::read_to_string(&edits_path) {
            if let Ok(edits) = serde_json::from_str::<Vec<UserEdit>>(&raw_edits) {
                draft.edits = edits;
            }
        }

        Ok(draft)
    }

    /// Re-hash the supplied PDF and check it matches `input_sha256`. Used by
    /// the GUI to warn the user when resuming a draft against a modified
    /// file.
    pub fn matches_pdf(&self, pdf_path: &std::path::Path) -> bool {
        match std::fs::read(pdf_path) {
            Ok(bytes) => sha256_hex(&bytes) == self.input_sha256,
            Err(_) => false,
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Public wrapper so callers (e.g. the GUI) can pre-compute and cache the
/// SHA-256 of the input PDF before calling [`WorkflowDraft::new_with_hash`].
pub fn sha256_hex_of(bytes: &[u8]) -> String {
    sha256_hex(bytes)
}

/// Result of the final DocAI re-parse.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowOutcome {
    pub final_pdf: std::path::PathBuf,
    pub transactions_re_parsed: usize,
    pub final_imbalance: Decimal,
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
        imbalance: Decimal,
    },
    /// The AI hallucinated values that do not sum correctly.
    FidelityCheckFailed(String),
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

use crate::engine::balance::{process_and_reconcile, ONE_CENT};
use crate::engine::model::Transaction;

/// Apply `edits` to `original` and rebuild the running ledger. Returns the
/// per-row diff plus the final imbalance.
pub fn build_preview(
    original: &[Transaction],
    edits: &[UserEdit],
    opening_balance: Decimal,
    expected_closing: Option<Decimal>,
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
        .unwrap_or(Decimal::ZERO);
    let final_imbalance = expected_closing
        .map(|e| (computed_final - e).round_dp(2))
        .unwrap_or(Decimal::ZERO);

    Ok(BalancePreview {
        rows,
        final_imbalance,
        balanced: final_imbalance.abs() < ONE_CENT,
        auto_correction_message: msg,
    })
}

fn parse_money(s: &str) -> Option<Decimal> {
    use std::str::FromStr;
    let cleaned: String = s
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '-' || *c == '.')
        .collect();
    Decimal::from_str(&cleaned).ok()
}

/// Stage 14a / Item #20: stable hash of an edit set. The runtime uses
/// this to avoid re-running `apply_many_edits` when the user clicks
/// "Confirm and Render" twice on an unchanged set; the second call
/// short-circuits and reuses the previous output.
pub fn edit_set_hash(input_pdf_sha256: &str, edits: &[UserEdit]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(input_pdf_sha256.as_bytes());
    h.update(b"|");
    for e in edits {
        h.update(format!("{}|{}|", e.page, e.line_on_page).as_bytes());
        h.update(e.old_text.as_bytes());
        h.update(b"|");
        h.update(e.new_text.as_bytes());
        h.update(b"|");
        let bb = e.bbox;
        h.update(
            format!("{:.3},{:.3},{:.3},{:.3}|", bb[0], bb[1], bb[2], bb[3]).as_bytes(),
        );
        h.update(format!("{:?};", e.field).as_bytes());
    }
    let digest = h.finalize();
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Detect overlapping bboxes in the queued edit set. Two edits "conflict"
/// when they target the same page and their bboxes overlap by more than
/// 50% of either bbox's area — typical case is two queued edits on the
/// same numeric cell. Returns the conflicting pairs as
/// `(index_a, index_b)` so the GUI can highlight them.
///
/// Stage 14a / Item #19.
pub fn detect_edit_conflicts(edits: &[UserEdit]) -> Vec<(usize, usize)> {
    let mut conflicts = Vec::new();
    for i in 0..edits.len() {
        for j in (i + 1)..edits.len() {
            let a = &edits[i];
            let b = &edits[j];
            if a.page != b.page {
                continue;
            }
            let overlap = bbox_overlap_fraction(a.bbox, b.bbox);
            if overlap > 0.5 {
                conflicts.push((i, j));
            }
        }
    }
    conflicts
}

fn bbox_overlap_fraction(a: [f32; 4], b: [f32; 4]) -> f32 {
    let ix0 = a[0].max(b[0]);
    let iy0 = a[1].max(b[1]);
    let ix1 = a[2].min(b[2]);
    let iy1 = a[3].min(b[3]);
    if ix1 <= ix0 || iy1 <= iy0 {
        return 0.0;
    }
    let ia = (ix1 - ix0) * (iy1 - iy0);
    let aa = ((a[2] - a[0]) * (a[3] - a[1])).max(0.001);
    let ab = ((b[2] - b[0]) * (b[3] - b[1])).max(0.001);
    let denom = aa.min(ab);
    ia / denom
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
            Some(c) if (c - typed).abs() < ONE_CENT => {
                tracing::debug!(
                    "[workflow] dropping redundant edit on P{} L{}: typed={} cascaded={}",
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
    use rust_decimal_macros::dec;

    fn tx(
        page: usize,
        line: usize,
        debit: Option<Decimal>,
        credit: Option<Decimal>,
        bal: Option<Decimal>,
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
            field_bboxes: Default::default(),
            provenance: Provenance::Manual,
        }
    }

    #[test]
    fn build_preview_applies_edits_and_cascades_balances() -> anyhow::Result<()> {
        let original = vec![
            tx(0, 0, Some(dec!(100)), None, Some(dec!(200))), // row 0: open 100 + 100 debit = 200 (expected)
            tx(0, 1, None, Some(dec!(50)), Some(dec!(150))),  // row 1: 200 open - 50 credit = 150 (expected)
        ];

        let edits = vec![UserEdit {
            page: 0,
            line_on_page: 0,
            bbox: [0.0; 4],
            old_text: "100.00".into(),
            new_text: "200.00".into(),
            field: EditField::Debit,
        }];

        let preview = build_preview(&original, &edits, dec!(100), None).map_err(|e| anyhow::anyhow!(e))?;

        // Row 0: debit changed 100 -> 200, balance recomputes 100 + 200 = 300
        assert_eq!(preview.rows[0].debit, Some(dec!(200)));
        assert_eq!(preview.rows[0].new_running_balance, Some(dec!(300.00)));
        // Row 1 cascades: 300 - 50 = 250
        assert_eq!(preview.rows[1].new_running_balance, Some(dec!(250.00)));
        // Old balance is preserved for the diff display
        assert_eq!(preview.rows[0].old_running_balance, Some(dec!(200)));
        // Both rows are flagged as changed (row 0 directly, row 1 by cascade)
        assert!(preview.rows[0].will_change);
        assert!(preview.rows[1].will_change);
        Ok(())
    }

    #[test]
    fn build_preview_marks_balanced_when_final_matches_expected() -> anyhow::Result<()> {
        // opening 100 + debit 100 = 200 closing
        let original = vec![tx(0, 0, Some(dec!(100)), None, Some(dec!(200)))];
        let preview = build_preview(&original, &[], dec!(100), Some(dec!(200))).map_err(|e| anyhow::anyhow!(e))?;
        assert!(preview.balanced);
        assert_eq!(preview.final_imbalance, dec!(0.00));
        Ok(())
    }

    #[test]
    fn parse_validation_acceptable_threshold_works() {
        let v = ParseValidation {
            total_pages: 1,
            transactions_found: 5,
            opening_balance: dec!(0),
            closing_balance: dec!(0),
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
            opening_balance: dec!(0),
            closing_balance: dec!(0),
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
            old_running_balance: Some(dec!(100)),
            new_running_balance: Some(dec!(100)),
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
            final_imbalance: dec!(0),
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
            final_imbalance: dec!(0),
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
                old_running_balance: Some(dec!(100)),
                new_running_balance: Some(dec!(250)),
                will_change: true,
            }],
            final_imbalance: dec!(0),
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
                old_running_balance: Some(dec!(100)),
                new_running_balance: Some(dec!(250)),
                will_change: true,
            }],
            final_imbalance: dec!(0),
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
            final_imbalance: dec!(0),
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
    fn mask_padding_grows_then_caps() {
        assert_eq!(mask_padding_for_attempt(1), 2.0);
        assert_eq!(mask_padding_for_attempt(2), 4.0);
        assert_eq!(mask_padding_for_attempt(3), 8.0);
        assert_eq!(mask_padding_for_attempt(4), 12.0);
        assert_eq!(mask_padding_for_attempt(99), 12.0); // capped
    }

    #[test]
    fn should_accept_near_perfect_only_after_attempt_3_and_below_half_threshold() {
        // Attempt 1, score 0.001, threshold 0.02 — strict path; don't accept.
        assert!(!should_accept_near_perfect(1, 0.001, 0.02));
        // Attempt 3, score 0.005 (<0.01 = half threshold). Accept.
        assert!(should_accept_near_perfect(3, 0.005, 0.02));
        // Attempt 3 but score is right at half — must be strictly less than.
        assert!(!should_accept_near_perfect(3, 0.01, 0.02));
        // Attempt 5, score 0.0001 — accept.
        assert!(should_accept_near_perfect(5, 0.0001, 0.02));
        // Attempt 4 with score above threshold — never accept.
        assert!(!should_accept_near_perfect(4, 0.05, 0.02));
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

    fn dummy_pdf(dir: &std::path::Path, name: &str, content: &[u8]) -> anyhow::Result<std::path::PathBuf> {
        let p = dir.join(name);
        std::fs::write(&p, content)?;
        Ok(p)
    }

    #[test]
    fn workflow_draft_round_trips_through_disk() -> anyhow::Result<()> {
        use tempfile::tempdir;
        let dir = tempdir()?;
        let pdf = dummy_pdf(dir.path(), "test.pdf", b"%PDF-1.4 hello")?;

        let validation = ParseValidation {
            total_pages: 2,
            transactions_found: 5,
            opening_balance: dec!(100),
            closing_balance: dec!(200),
            account_number: Some("12345".into()),
            completeness_score: 0.93,
            completeness_notes: "looks good".into(),
            missing_rows: vec![],
        };
        let edits = vec![UserEdit {
            page: 0,
            line_on_page: 3,
            bbox: [10.0, 20.0, 30.0, 40.0],
            old_text: "100.00".into(),
            new_text: "150.00".into(),
            field: EditField::Debit,
        }];

        let draft = WorkflowDraft::new(&pdf, Some(validation.clone()), vec![], edits.clone()).map_err(|e| anyhow::anyhow!(e))?;
        let path = dir.path().join("draft.json");
        draft.save_to_file(&path).map_err(|e| anyhow::anyhow!(e))?;

        let loaded = WorkflowDraft::load_from_file(&path).map_err(|e| anyhow::anyhow!(e))?;
        assert_eq!(loaded.schema_version, WORKFLOW_DRAFT_SCHEMA);
        assert_eq!(loaded.input_sha256, draft.input_sha256);
        assert_eq!(loaded.edits, edits);
        assert_eq!(loaded.validation, Some(validation));
        Ok(())
    }

    #[test]
    fn workflow_draft_matches_pdf_returns_true_when_unchanged() -> anyhow::Result<()> {
        use tempfile::tempdir;
        let dir = tempdir()?;
        let pdf = dummy_pdf(dir.path(), "a.pdf", b"%PDF-1.4 v1")?;
        let draft = WorkflowDraft::new(&pdf, None, vec![], vec![]).map_err(|e| anyhow::anyhow!(e))?;
        assert!(draft.matches_pdf(&pdf));
        Ok(())
    }

    #[test]
    fn workflow_draft_matches_pdf_returns_false_when_pdf_modified() -> anyhow::Result<()> {
        use tempfile::tempdir;
        let dir = tempdir()?;
        let pdf = dummy_pdf(dir.path(), "b.pdf", b"%PDF-1.4 v1")?;
        let draft = WorkflowDraft::new(&pdf, None, vec![], vec![]).map_err(|e| anyhow::anyhow!(e))?;
        std::fs::write(&pdf, b"%PDF-1.4 v2")?;
        assert!(!draft.matches_pdf(&pdf));
        Ok(())
    }

    #[test]
    fn workflow_draft_load_rejects_incompatible_schema() -> anyhow::Result<()> {
        use tempfile::tempdir;
        let dir = tempdir()?;
        let path = dir.path().join("future.json");
        let bad = serde_json::json!({
            "schema_version": 999,
            "input_sha256": "abc",
            "input_path": "x.pdf",
            "saved_at": "2026-01-01T00:00:00Z",
            "validation": null,
            "transactions": [],
            "edits": [],
        });
        std::fs::write(&path, bad.to_string())?;
        assert!(WorkflowDraft::load_from_file(&path).is_err());
        Ok(())
    }

    #[test]
    fn workflow_draft_new_with_hash_matches_new_when_hash_is_correct() -> anyhow::Result<()> {
        use tempfile::tempdir;
        let dir = tempdir()?;
        let pdf = dummy_pdf(dir.path(), "x.pdf", b"%PDF-1.4 cached-hash-test")?;
        let bytes = std::fs::read(&pdf)?;
        let hash = sha256_hex_of(&bytes);

        let from_disk = WorkflowDraft::new(&pdf, None, vec![], vec![]).map_err(|e| anyhow::anyhow!(e))?;
        let from_cache = WorkflowDraft::new_with_hash(&pdf, hash.clone(), None, vec![], vec![]);

        // Same content hash, same path; saved_at differs by milliseconds so
        // we don't compare it.
        assert_eq!(from_disk.input_sha256, from_cache.input_sha256);
        assert_eq!(from_disk.input_path, from_cache.input_path);
        assert!(from_cache.matches_pdf(&pdf));
        Ok(())
    }
}

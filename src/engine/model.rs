//! Core transaction model.
//!
//! # Sign convention (important)
//!
//! Throughout this codebase the running ledger is computed as:
//!
//! ```text
//! new_balance = old_balance + debit - credit
//! ```
//!
//! That is, **`debit` adds to the balance and `credit` subtracts**. This is
//! the inverse of formal double-entry accounting (where a credit increases an
//! asset/liability balance and a debit reduces it). We made this choice
//! historically because most retail bank statements show "Debit" as the
//! "money in" column for the *customer's* account, and "Credit" as
//! "money out" — these field names match what the user sees on their
//! statement, not the accountant-side journal.
//!
//! When you need to think in formal accounting terms, use [`Transaction::delta_in`]
//! and [`Transaction::delta_out`], which are unambiguous regardless of which
//! convention you grew up with.
//!
//! # Why `Decimal` and not `f64`
//!
//! Stage 7 / Item #11. Bank statements use exact decimal arithmetic; running
//! the cascade in `f64` accumulates representation drift across hundreds of
//! rows (the classic `0.1 + 0.2 != 0.3` problem). We use [`rust_decimal::Decimal`]
//! end-to-end for any monetary field, and only cross to `f64` at the GUI
//! plotting / image-pixel boundary where the precision doesn't matter.
//!
//! Helpers [`dec_to_f64`] and [`f64_to_dec`] convert at those edges.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// One row of a bank statement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Transaction {
    pub page: usize,
    pub line_on_page: usize,
    pub date: String,
    pub raw_text: String,
    /// Money flowing **into** the account on this row (in the customer's view).
    /// Adds to the running balance. See module docs for the sign convention.
    pub debit: Option<Decimal>,
    /// Money flowing **out of** the account on this row (in the customer's view).
    /// Subtracts from the running balance. See module docs for the sign convention.
    pub credit: Option<Decimal>,
    pub running_balance: Option<Decimal>,
    /// Bbox of the entire row in PDF points (x0,y0,x1,y1).
    pub bbox: Option<[f32; 4]>,
    /// Per-field bboxes (Date, Description, Debit, Credit, RunningBalance).
    /// When present, an edit on a specific field uses that bbox instead of
    /// the row-level `bbox`. Stage 7.5 — without these the binary editor
    /// would redact the entire row when the user only changed one cell.
    #[serde(default, skip_serializing_if = "FieldBboxes::is_empty")]
    pub field_bboxes: FieldBboxes,
    pub provenance: Provenance,
}

/// Per-field bounding boxes for a single transaction row. All bboxes are
/// in PDF points (origin top-left of page); `None` means the field wasn't
/// present in this row (e.g. a debit-only row has no `credit` bbox).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FieldBboxes {
    pub date: Option<[f32; 4]>,
    pub description: Option<[f32; 4]>,
    pub debit: Option<[f32; 4]>,
    pub credit: Option<[f32; 4]>,
    pub running_balance: Option<[f32; 4]>,
}

impl FieldBboxes {
    pub fn is_empty(&self) -> bool {
        self.date.is_none()
            && self.description.is_none()
            && self.debit.is_none()
            && self.credit.is_none()
            && self.running_balance.is_none()
    }
}

impl Transaction {
    /// Money flowing into the account on this row. Positive contribution to
    /// the running balance. Returns `0` when not present.
    #[inline]
    pub fn delta_in(&self) -> Decimal {
        self.debit.unwrap_or(Decimal::ZERO)
    }

    /// Money flowing out of the account on this row. Positive contribution
    /// to the *out* side (i.e. subtracted from the running balance).
    /// Returns `0` when not present.
    #[inline]
    pub fn delta_out(&self) -> Decimal {
        self.credit.unwrap_or(Decimal::ZERO)
    }

    /// Net change in the running balance contributed by this transaction.
    /// Positive = balance grows, negative = balance shrinks.
    #[inline]
    pub fn net_delta(&self) -> Decimal {
        self.delta_in() - self.delta_out()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Provenance {
    DocumentAI { confidence: f32 },
    Manual,
    Computed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposedChange {
    pub page: usize,
    pub old_text: String,
    pub new_text: String,
    pub reason: String,
    pub confidence: f32,
    pub affects_subsequent_balances: bool,
    pub bbox: Option<[f32; 4]>,
}

// ---------------------------------------------------------------------------
// Decimal / f64 conversion helpers (Stage 7 / Item #11)
// ---------------------------------------------------------------------------

/// Convert a `Decimal` to `f64`. Use only at GUI plotting / image-pixel
/// boundaries where precision is acceptable.
#[inline]
pub fn dec_to_f64(d: Decimal) -> f64 {
    use rust_decimal::prelude::ToPrimitive;
    d.to_f64().unwrap_or(0.0)
}

/// Convert an `f64` to a `Decimal`, rounded to two decimal places (the
/// natural precision of bank statements).
#[inline]
pub fn f64_to_dec(v: f64) -> Decimal {
    Decimal::from_f64_retain(v)
        .unwrap_or(Decimal::ZERO)
        .round_dp(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn tx(debit: Option<Decimal>, credit: Option<Decimal>) -> Transaction {
        Transaction {
            page: 0,
            line_on_page: 0,
            date: "2026-01-01".into(),
            raw_text: "test".into(),
            debit,
            credit,
            running_balance: None,
            bbox: None,
            field_bboxes: Default::default(),
            provenance: Provenance::Manual,
        }
    }

    #[test]
    fn proposed_change_roundtrips_with_bbox() {
        let change_with_bbox = ProposedChange {
            page: 1,
            old_text: "100.00".into(),
            new_text: "150.00".into(),
            reason: "Adjust".into(),
            confidence: 0.95,
            affects_subsequent_balances: true,
            bbox: Some([10.0, 20.0, 50.0, 40.0]),
        };
        let json = serde_json::to_string(&change_with_bbox).unwrap();
        let decoded: ProposedChange = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.bbox, Some([10.0, 20.0, 50.0, 40.0]));

        let change_no_bbox = ProposedChange {
            page: 1,
            old_text: "100.00".into(),
            new_text: "150.00".into(),
            reason: "Adjust".into(),
            confidence: 0.95,
            affects_subsequent_balances: true,
            bbox: None,
        };
        let json = serde_json::to_string(&change_no_bbox).unwrap();
        let decoded: ProposedChange = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.bbox, None);
    }

    /// Sign-convention regression: this codebase treats `debit` as money in
    /// (adds to balance) and `credit` as money out (subtracts). Anyone wiring
    /// formal double-entry semantics on top of this struct must use
    /// `delta_in()` / `delta_out()` / `net_delta()` to avoid silently
    /// inverting numbers.
    #[test]
    fn delta_helpers_match_running_balance_arithmetic() {
        let opening = dec!(100);
        let row_in = tx(Some(dec!(50)), None);
        let row_out = tx(None, Some(dec!(20)));
        let row_neither = tx(None, None);

        assert_eq!(row_in.delta_in(), dec!(50));
        assert_eq!(row_in.delta_out(), dec!(0));
        assert_eq!(row_in.net_delta(), dec!(50));

        assert_eq!(row_out.delta_in(), dec!(0));
        assert_eq!(row_out.delta_out(), dec!(20));
        assert_eq!(row_out.net_delta(), dec!(-20));

        assert_eq!(row_neither.net_delta(), dec!(0));

        // Compose: opening 100 + 50 - 20 = 130
        let final_balance = opening + row_in.net_delta() + row_out.net_delta();
        assert_eq!(final_balance, dec!(130));
    }

    /// The classic 0.1 + 0.2 != 0.3 problem under f64 — Decimal handles this
    /// exactly. This is the whole reason for Stage 7.
    #[test]
    fn decimal_avoids_f64_drift_across_hundreds_of_rows() {
        let one_cent = dec!(0.01);
        let mut bal = dec!(0);
        for _ in 0..1000 {
            bal += one_cent;
        }
        assert_eq!(bal, dec!(10.00));
    }
}

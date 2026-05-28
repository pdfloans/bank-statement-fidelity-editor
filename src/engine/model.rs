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

use serde::{Deserialize, Serialize};

/// One row of a bank statement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub page: usize,
    pub line_on_page: usize,
    pub date: String,
    pub raw_text: String,
    /// Money flowing **into** the account on this row (in the customer's view).
    /// Adds to the running balance. See module docs for the sign convention.
    pub debit: Option<f64>,
    /// Money flowing **out of** the account on this row (in the customer's view).
    /// Subtracts from the running balance. See module docs for the sign convention.
    pub credit: Option<f64>,
    pub running_balance: Option<f64>,
    pub bbox: Option<[f32; 4]>,
    pub provenance: Provenance,
}

impl Transaction {
    /// Money flowing into the account on this row. Positive contribution to
    /// the running balance. Returns `0.0` when not present.
    #[inline]
    pub fn delta_in(&self) -> f64 {
        self.debit.unwrap_or(0.0)
    }

    /// Money flowing out of the account on this row. Positive contribution
    /// to the *out* side (i.e. subtracted from the running balance).
    /// Returns `0.0` when not present.
    #[inline]
    pub fn delta_out(&self) -> f64 {
        self.credit.unwrap_or(0.0)
    }

    /// Net change in the running balance contributed by this transaction.
    /// Positive = balance grows, negative = balance shrinks.
    #[inline]
    pub fn net_delta(&self) -> f64 {
        self.delta_in() - self.delta_out()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tx(debit: Option<f64>, credit: Option<f64>) -> Transaction {
        Transaction {
            page: 0,
            line_on_page: 0,
            date: "2026-01-01".into(),
            raw_text: "test".into(),
            debit,
            credit,
            running_balance: None,
            bbox: None,
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
        let opening: f64 = 100.0;
        let row_in = tx(Some(50.0), None);
        let row_out = tx(None, Some(20.0));
        let row_neither = tx(None, None);

        assert_eq!(row_in.delta_in(), 50.0);
        assert_eq!(row_in.delta_out(), 0.0);
        assert_eq!(row_in.net_delta(), 50.0);

        assert_eq!(row_out.delta_in(), 0.0);
        assert_eq!(row_out.delta_out(), 20.0);
        assert_eq!(row_out.net_delta(), -20.0);

        assert_eq!(row_neither.net_delta(), 0.0);

        // Compose: opening 100 + 50 - 20 = 130
        let final_balance = opening + row_in.net_delta() + row_out.net_delta();
        assert_eq!(final_balance, 130.0);
    }
}

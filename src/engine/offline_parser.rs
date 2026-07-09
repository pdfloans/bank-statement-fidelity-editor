//! Offline fallback parser: converts `LineGeometry` (from any local
//! `GeometryProvider`) into a `BankStatement` compatible with the
//! Document AI output structure.
//!
//! This allows the full workflow to proceed without any network access by
//! using PyMuPDF/Pdfium text extraction + template-based heuristics.

use crate::ai::document_ai::BankStatement;
use crate::engine::model::{FieldBboxes, Provenance, Transaction};
use crate::extractors::geometry::LineGeometry;
use crate::extractors::templates::parsers;
use crate::pdf::PdfEngine;
use rust_decimal::Decimal;
use std::path::Path;
use std::sync::Arc;

/// Parse a bank statement from offline text-layer extraction.
///
/// Uses the PDF engine's `get_text_blocks` + `PyMuPdfHeuristicProvider` to
/// extract rows, then applies winnow parsers to identify dates, amounts,
/// and running balances. Produces a `BankStatement` with lower confidence
/// than Document AI but sufficient for the workflow to proceed offline.
pub fn parse_statement_offline(
    pdf_path: &Path,
    engine: Arc<dyn PdfEngine>,
) -> Result<BankStatement, String> {
    // Step 1: Get layout for page count
    let layout = engine
        .analyze_layout(pdf_path)
        .map_err(|e| format!("layout analysis failed: {e}"))?;

    let total_pages = layout.total_pages;
    if total_pages == 0 {
        return Err("PDF has 0 pages".into());
    }

    // Step 2: Extract text blocks from all pages and cluster into rows
    let mut all_rows: Vec<RawRow> = Vec::new();

    for page in 0..total_pages {
        #[allow(unused_mut)] // mutated only when cfg(feature = "ocr") is active
        let mut blocks = engine.get_text_blocks(pdf_path, page).unwrap_or_default();

        #[cfg(feature = "ocr")]
        {
            if blocks.is_empty() {
                tracing::info!(
                    "[offline_parser] No text found on page {}, falling back to OCR",
                    page
                );
                blocks = extract_text_via_ocr(pdf_path, page, engine.clone());
            }
        }

        // Cluster blocks into rows by y-coordinate proximity (±5pt)
        let mut current_y: Option<f32> = None;
        let mut current_row_blocks = Vec::new();
        let mut line_idx = 0usize;

        for block in &blocks {
            let y_center = (block.bbox[1] + block.bbox[3]) / 2.0;

            if let Some(y) = current_y {
                if (y_center - y).abs() < 5.0 {
                    current_row_blocks.push(block.clone());
                    continue;
                }
                // Flush current row
                if !current_row_blocks.is_empty() {
                    all_rows.push(RawRow::from_blocks(page, line_idx, &current_row_blocks));
                    line_idx += 1;
                }
                current_row_blocks.clear();
            }

            current_y = Some(y_center);
            current_row_blocks.push(block.clone());
        }
        // Flush final row on page
        if !current_row_blocks.is_empty() {
            all_rows.push(RawRow::from_blocks(page, line_idx, &current_row_blocks));
        }
    }

    // Step 3: Parse each row to identify transactions
    let (transactions, opening_balance, closing_balance) = parse_rows_into_transactions(&all_rows);

    tracing::info!(
        "[offline_parser] extracted {} transactions from {} pages (opening={}, closing={})",
        transactions.len(),
        total_pages,
        opening_balance,
        closing_balance,
    );

    Ok(BankStatement {
        total_pages,
        transactions,
        opening_balance,
        closing_balance,
        account_number: extract_account_number(&all_rows),
    })
}

/// Parse a bank statement from pre-extracted `LineGeometry` entries.
pub fn parse_statement_from_geometry(
    geometries: &[LineGeometry],
    total_pages: usize,
) -> Result<BankStatement, String> {
    let rows: Vec<RawRow> = geometries
        .iter()
        .map(|g| RawRow {
            page: g.page,
            line_on_page: g.line_on_page,
            text: g.text.clone(),
            bbox: g.bbox,
        })
        .collect();

    let (transactions, opening_balance, closing_balance) = parse_rows_into_transactions(&rows);

    Ok(BankStatement {
        total_pages,
        transactions,
        opening_balance,
        closing_balance,
        account_number: extract_account_number(&rows),
    })
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct RawRow {
    page: usize,
    line_on_page: usize,
    text: String,
    bbox: [f32; 4],
}

impl RawRow {
    fn from_blocks(page: usize, line_on_page: usize, blocks: &[crate::pdf::TextBlock]) -> Self {
        let mut text = String::new();
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;

        for b in blocks {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&b.text);
            min_x = min_x.min(b.bbox[0]);
            min_y = min_y.min(b.bbox[1]);
            max_x = max_x.max(b.bbox[2]);
            max_y = max_y.max(b.bbox[3]);
        }

        Self {
            page,
            line_on_page,
            text: text.trim().to_string(),
            bbox: [min_x, min_y, max_x, max_y],
        }
    }
}

// ---------------------------------------------------------------------------
// Row-level parsing
// ---------------------------------------------------------------------------

/// Currency regex for quick scanning before invoking the winnow parser
static AMOUNT_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"-?\$?[\d,]+\.\d{2}").unwrap());

static DATE_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(
        r"(?i)(\d{1,2}[/ ](jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec|\d{1,2})[/ ]\d{2,4})",
    )
    .unwrap()
});

/// Balance-related keywords (case-insensitive match)
static OPENING_KW: &[&str] = &[
    "opening balance",
    "beginning balance",
    "balance brought forward",
    "balance b/f",
    "opening bal",
    "brought forward",
];

static CLOSING_KW: &[&str] = &[
    "closing balance",
    "ending balance",
    "balance carried forward",
    "balance c/f",
    "closing bal",
    "carried forward",
];

fn parse_rows_into_transactions(rows: &[RawRow]) -> (Vec<Transaction>, Decimal, Decimal) {
    let mut transactions = Vec::new();
    let mut opening_balance = Decimal::ZERO;
    let mut closing_balance = Decimal::ZERO;
    let mut found_opening = false;
    let mut found_closing = false;

    for row in rows {
        let text_lower = row.text.to_lowercase();

        // Check for opening/closing balance lines
        let is_opening = OPENING_KW.iter().any(|kw| text_lower.contains(kw));
        let is_closing = CLOSING_KW.iter().any(|kw| text_lower.contains(kw));

        // Extract all amounts from this line
        let amounts = extract_amounts(&row.text);

        if is_opening && !amounts.is_empty() && !found_opening {
            opening_balance = *amounts.last().unwrap();
            found_opening = true;
            continue;
        }
        if is_closing && !amounts.is_empty() && !found_closing {
            closing_balance = *amounts.last().unwrap();
            found_closing = true;
            continue;
        }

        // Skip non-transaction rows (headers, labels, etc.)
        if !DATE_RE.is_match(&row.text) || amounts.is_empty() {
            continue;
        }

        // This row looks like a transaction: has a date and at least one amount
        let date = extract_date(&row.text);

        // Heuristic: if there are 3+ amounts, the last is likely the running balance
        // If 2 amounts, the first is debit/credit and the second is running balance
        // If 1 amount, it's a debit or credit with no running balance shown
        let (debit, credit, running_balance) = match amounts.len() {
            1 => {
                // Single amount - assume it's a debit (money in) if positive
                let amt = amounts[0];
                if amt >= Decimal::ZERO {
                    (Some(amt), None, None)
                } else {
                    (None, Some(amt.abs()), None)
                }
            }
            2 => {
                // Two amounts: first is debit/credit, second is running balance
                let amt = amounts[0];
                let bal = amounts[1];
                if amt >= Decimal::ZERO {
                    (Some(amt), None, Some(bal))
                } else {
                    (None, Some(amt.abs()), Some(bal))
                }
            }
            _ => {
                // 3+ amounts: try to identify debit, credit, running balance
                // Common layout: description | debit | credit | balance
                // where one of debit/credit is blank (shows as no match)
                let bal = *amounts.last().unwrap();

                // Look at the 2nd-to-last and 3rd-to-last
                // If only one non-balance amount, it's either debit or credit
                let non_bal = &amounts[..amounts.len() - 1];
                match non_bal.len() {
                    1 => {
                        let amt = non_bal[0];
                        if amt >= Decimal::ZERO {
                            (Some(amt), None, Some(bal))
                        } else {
                            (None, Some(amt.abs()), Some(bal))
                        }
                    }
                    _ => {
                        // Two amounts before balance: first=debit, second=credit (or vice versa)
                        let d = non_bal[non_bal.len() - 2];
                        let c = non_bal[non_bal.len() - 1];
                        (
                            if d != Decimal::ZERO {
                                Some(d.abs())
                            } else {
                                None
                            },
                            if c != Decimal::ZERO {
                                Some(c.abs())
                            } else {
                                None
                            },
                            Some(bal),
                        )
                    }
                }
            }
        };

        transactions.push(Transaction {
            page: row.page,
            line_on_page: row.line_on_page,
            date,
            raw_text: row.text.clone(),
            debit,
            credit,
            running_balance,
            bbox: Some(row.bbox),
            field_bboxes: FieldBboxes::default(),
            provenance: Provenance::Computed,
        });
    }

    // If we didn't find explicit opening/closing, try to infer from transactions
    if !found_opening && !transactions.is_empty() {
        if let Some(first_bal) = transactions[0].running_balance {
            // opening = first_balance - first_tx_net_delta
            let net = transactions[0].debit.unwrap_or(Decimal::ZERO)
                - transactions[0].credit.unwrap_or(Decimal::ZERO);
            opening_balance = first_bal - net;
        }
    }
    if !found_closing && !transactions.is_empty() {
        if let Some(last_bal) = transactions.last().and_then(|t| t.running_balance) {
            closing_balance = last_bal;
        }
    }

    (transactions, opening_balance, closing_balance)
}

fn extract_amounts(text: &str) -> Vec<Decimal> {
    AMOUNT_RE
        .find_iter(text)
        .filter_map(|m| {
            let s = m.as_str();
            let mut input = s;
            parsers::parse_currency(&mut input).ok()
        })
        .collect()
}

fn extract_date(text: &str) -> String {
    if let Some(m) = DATE_RE.find(text) {
        m.as_str().to_string()
    } else {
        String::new()
    }
}

fn extract_account_number(rows: &[RawRow]) -> Option<String> {
    let acct_re = regex::Regex::new(
        r"(?i)(?:account|acct|a/c)\s*(?:no\.?|number|#)?\s*[:.]?\s*(\d[\d\s-]{4,20}\d)",
    )
    .ok()?;
    for row in rows.iter().take(20) {
        // Only check first ~20 rows (header area)
        if let Some(caps) = acct_re.captures(&row.text) {
            if let Some(m) = caps.get(1) {
                let cleaned: String = m.as_str().chars().filter(|c| c.is_ascii_digit()).collect();
                if cleaned.len() >= 6 {
                    return Some(cleaned);
                }
            }
        }
    }
    None
}

#[cfg(feature = "ocr")]
fn extract_text_via_ocr(
    pdf_path: &Path,
    page: usize,
    engine: Arc<dyn PdfEngine>,
) -> Vec<crate::pdf::TextBlock> {
    tracing::info!(
        "[offline_parser] Attempting to render page {} for OCR fallback...",
        page
    );
    let _rendered = match engine.render_page(pdf_path, page, 300.0) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("[offline_parser] Failed to render page for OCR: {}", e);
            return vec![];
        }
    };

    tracing::warn!("[offline_parser] OCR models not bundled in this environment. Scanned PDF fallback aborted.");
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn extract_amounts_parses_various_formats() {
        let amounts = extract_amounts("Payment $1,234.56 Balance $5,678.90");
        assert_eq!(amounts.len(), 2);
        assert_eq!(amounts[0], dec!(1234.56));
        assert_eq!(amounts[1], dec!(5678.90));
    }

    #[test]
    fn extract_amounts_handles_negative() {
        let amounts = extract_amounts("-$500.00 remainder 1,000.00");
        assert_eq!(amounts.len(), 2);
        assert_eq!(amounts[0], dec!(-500.00));
        assert_eq!(amounts[1], dec!(1000.00));
    }

    #[test]
    fn extract_date_finds_date() {
        let d = extract_date("15/01/2024 Payment to grocery store $42.50");
        assert_eq!(d, "15/01/2024");
    }

    #[test]
    fn extract_date_finds_month_name() {
        let d = extract_date("15 Jan 2024 Direct debit $100.00");
        assert_eq!(d, "15 Jan 2024");
    }

    #[test]
    fn parse_rows_identifies_opening_closing() {
        let rows = vec![
            RawRow {
                page: 0,
                line_on_page: 0,
                text: "Opening Balance $1,000.00".into(),
                bbox: [0.0; 4],
            },
            RawRow {
                page: 0,
                line_on_page: 1,
                text: "15/01/2024 Direct Deposit $500.00 $1,500.00".into(),
                bbox: [0.0; 4],
            },
            RawRow {
                page: 0,
                line_on_page: 2,
                text: "16/01/2024 ATM Withdrawal -$200.00 $1,300.00".into(),
                bbox: [0.0; 4],
            },
            RawRow {
                page: 0,
                line_on_page: 3,
                text: "Closing Balance $1,300.00".into(),
                bbox: [0.0; 4],
            },
        ];
        let (txs, opening, closing) = parse_rows_into_transactions(&rows);
        assert_eq!(opening, dec!(1000.00));
        assert_eq!(closing, dec!(1300.00));
        assert_eq!(txs.len(), 2);
    }

    #[test]
    fn extract_account_number_finds_number() {
        let rows = vec![
            RawRow {
                page: 0,
                line_on_page: 0,
                text: "Bank of Test".into(),
                bbox: [0.0; 4],
            },
            RawRow {
                page: 0,
                line_on_page: 1,
                text: "Account No. 123-456-789".into(),
                bbox: [0.0; 4],
            },
        ];
        let acct = extract_account_number(&rows);
        assert_eq!(acct, Some("123456789".to_string()));
    }
}

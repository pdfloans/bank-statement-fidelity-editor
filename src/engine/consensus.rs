use crate::ai::document_ai::BankStatement;
use crate::engine::model::Transaction;
use rust_decimal::Decimal;

/// Takes up to 3 `BankStatement`s from different AI/Offline parsers and
/// performs a majority-rule vote to synthesize the most accurate result.
pub fn merge_consensus_statements(statements: Vec<BankStatement>) -> BankStatement {
    if statements.is_empty() {
        return BankStatement {
            total_pages: 0,
            transactions: Vec::new(),
            opening_balance: Decimal::ZERO,
            closing_balance: Decimal::ZERO,
            account_number: None,
            bank_name: None,
        };
    }

    // If only 1 statement, just return it.
    if statements.len() == 1 {
        return statements.into_iter().next().unwrap();
    }

    // Find majority opening balance
    let mut opening_votes = std::collections::HashMap::new();
    for s in &statements {
        *opening_votes.entry(s.opening_balance).or_insert(0) += 1;
    }
    let majority_opening = opening_votes
        .into_iter()
        .max_by_key(|&(_, v)| v)
        .map(|(k, _)| k)
        .unwrap_or(Decimal::ZERO);

    // Find majority closing balance
    let mut closing_votes = std::collections::HashMap::new();
    for s in &statements {
        *closing_votes.entry(s.closing_balance).or_insert(0) += 1;
    }
    let majority_closing = closing_votes
        .into_iter()
        .max_by_key(|&(_, v)| v)
        .map(|(k, _)| k)
        .unwrap_or(Decimal::ZERO);

    // For transactions, we will collect all transactions, and group them by (date, amount)
    let mut all_txs: Vec<Transaction> = Vec::new();
    for s in statements.clone() {
        all_txs.extend(s.transactions);
    }

    // Group transactions by simple heuristics to find identical ones across different parses.
    let mut grouped_txs: std::collections::HashMap<
        (String, Option<Decimal>, Option<Decimal>),
        Vec<Transaction>,
    > = std::collections::HashMap::new();
    for tx in all_txs {
        let key = (tx.date.clone(), tx.debit, tx.credit);
        grouped_txs.entry(key).or_default().push(tx);
    }

    let mut final_txs = Vec::new();
    for (_, group) in grouped_txs {
        // If it was detected by at least 2 parsers (or if we only had 2 parsers, by 1 or 2 depending on strictness)
        if group.len() >= 2 || (group.len() == 1 && statements.len() < 3) {
            final_txs.push(group[0].clone());
        }
    }

    // Sort transactions by page/line_on_page
    final_txs.sort_by(|a, b| {
        a.page
            .cmp(&b.page)
            .then(a.line_on_page.cmp(&b.line_on_page))
    });

    BankStatement {
        opening_balance: majority_opening,
        closing_balance: majority_closing,
        transactions: final_txs,
        account_number: statements[0].account_number.clone(),
        total_pages: statements[0].total_pages,
        bank_name: None,
    }
}

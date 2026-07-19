use crate::engine::model::Transaction;
use std::collections::HashMap;

/// A simple heuristic-based categorizer for transactions.
pub fn categorize_transactions(transactions: &mut [Transaction]) {
    // A basic mapping of keywords to categories
    let mut keyword_map: HashMap<&str, &str> = HashMap::new();
    keyword_map.insert("uber", "Transport");
    keyword_map.insert("lyft", "Transport");
    keyword_map.insert("mcdonalds", "Food & Dining");
    keyword_map.insert("starbucks", "Food & Dining");
    keyword_map.insert("dunkin", "Food & Dining");
    keyword_map.insert("walmart", "Shopping");
    keyword_map.insert("target", "Shopping");
    keyword_map.insert("amazon", "Shopping");
    keyword_map.insert("netflix", "Entertainment");
    keyword_map.insert("spotify", "Entertainment");
    keyword_map.insert("shell", "Gas");
    keyword_map.insert("exxon", "Gas");
    keyword_map.insert("chevron", "Gas");
    keyword_map.insert("payroll", "Income");
    keyword_map.insert("salary", "Income");
    keyword_map.insert("deposit", "Income");

    for tx in transactions.iter_mut() {
        let desc = tx.raw_text.to_lowercase();
        let mut matched = false;
        
        for (keyword, category) in &keyword_map {
            if desc.contains(keyword) {
                tx.category = Some(category.to_string());
                matched = true;
                break;
            }
        }
        
        if !matched {
            // Check if it's an income based on debit amount (money in)
            if tx.delta_in() > rust_decimal::Decimal::ZERO {
                tx.category = Some("Income".to_string());
            } else {
                tx.category = Some("Uncategorized".to_string());
            }
        }
    }
}

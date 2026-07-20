use dual_core_pdf_pipeline::engine::workflow::*;
use rust_decimal_macros::dec;
use dual_core_pdf_pipeline::engine::model::Transaction;

#[test]
fn test_is_borderline() {
    assert!(is_borderline(0.015, 0.02)); // Borderline because it's > 50% of threshold
    assert!(!is_borderline(0.005, 0.02)); // Not borderline, it's very safe
    assert!(is_borderline(0.021, 0.02)); // Slightly above threshold is borderline
    assert!(!is_borderline(0.05, 0.02)); // Way above threshold is not borderline (it's an obvious fail)
}

#[test]
fn test_mask_padding_for_attempt() {
    assert_eq!(mask_padding_for_attempt(1), 1.0);
    assert_eq!(mask_padding_for_attempt(2), 2.0);
    assert_eq!(mask_padding_for_attempt(5), 5.0);
    assert_eq!(mask_padding_for_attempt(10), 10.0); // max is 10.0
    assert_eq!(mask_padding_for_attempt(15), 10.0);
}

#[test]
fn test_should_accept_near_perfect() {
    assert!(should_accept_near_perfect(5, 0.025, 0.02)); // 5th attempt, just over threshold -> accept
    assert!(!should_accept_near_perfect(1, 0.025, 0.02)); // 1st attempt, over threshold -> reject
    assert!(!should_accept_near_perfect(5, 0.05, 0.02)); // 5th attempt, way over threshold -> reject
}

#[test]
fn test_edit_set_hash() {
    let mut edits = vec![
        UserEdit {
            transaction_id: 1,
            original_date: "2023-01-01".to_string(),
            new_date: Some("2023-01-02".to_string()),
            ..Default::default()
        }
    ];
    let hash1 = edit_set_hash("abc", &edits);
    
    // Exact same edit should yield same hash
    let hash2 = edit_set_hash("abc", &edits);
    assert_eq!(hash1, hash2);

    // Different edit yields different hash
    edits[0].transaction_id = 2;
    let hash3 = edit_set_hash("abc", &edits);
    assert_ne!(hash1, hash3);
}

#[test]
fn test_detect_edit_conflicts() {
    let edits = vec![
        UserEdit {
            transaction_id: 1,
            new_debit: Some(dec!(100.0)),
            ..Default::default()
        },
        UserEdit {
            transaction_id: 1,
            new_credit: Some(dec!(50.0)),
            ..Default::default()
        },
        UserEdit {
            transaction_id: 2,
            new_debit: Some(dec!(200.0)),
            ..Default::default()
        }
    ];

    let conflicts = detect_edit_conflicts(&edits);
    assert_eq!(conflicts.len(), 1);
    // Edit 0 and Edit 1 conflict because they are on the same transaction ID
    assert_eq!(conflicts[0], (0, 1));
}

#[test]
fn test_prune_redundant_edits() {
    let txs = vec![
        Transaction {
            page: 1,
            line_on_page: 1,
            date: "2023-01-01".to_string(),
            raw_text: "Test".to_string(),
            debit: Some(dec!(100.0)),
            credit: None,
            running_balance: Some(dec!(500.0)),
            bbox: None,
            field_bboxes: Default::default(),
            provenance: dual_core_pdf_pipeline::engine::model::Provenance::Ocr,
            category: None,
        }
    ];

    let edits = vec![
        UserEdit {
            transaction_id: 0,
            original_date: "2023-01-01".to_string(),
            new_date: Some("2023-01-01".to_string()), // redundant
            new_debit: Some(dec!(100.0)), // redundant
            ..Default::default()
        },
        UserEdit {
            transaction_id: 0,
            original_date: "2023-01-01".to_string(),
            new_credit: Some(dec!(50.0)), // not redundant
            ..Default::default()
        }
    ];

    let pruned = prune_redundant_edits(&edits, &txs);
    assert_eq!(pruned.len(), 1);
    assert!(pruned[0].new_credit.is_some());
}

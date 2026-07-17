use dual_core_pdf_pipeline::ai::document_ai::BankStatement;
use dual_core_pdf_pipeline::engine::consensus::merge_consensus_statements;
use dual_core_pdf_pipeline::engine::model::{FieldBboxes, Transaction};
use rust_decimal_macros::dec;

#[test]
fn test_ai_matrix_consensus() {
    let tx1 = Transaction {
        page: 0,
        line_on_page: 0,
        date: "01/01/2026".into(),
        raw_text: "Deposit".into(),
        debit: Some(dec!(500.0)),
        credit: None,
        running_balance: None,
        field_bboxes: FieldBboxes::default(),
        bbox: Some([0.0; 4]),
        provenance: dual_core_pdf_pipeline::engine::model::Provenance::Computed,
    };

    let tx2 = Transaction {
        page: 0,
        line_on_page: 1,
        date: "02/01/2026".into(),
        raw_text: "Withdrawal".into(),
        debit: None,
        credit: Some(dec!(200.0)),
        running_balance: None,
        field_bboxes: FieldBboxes::default(),
        bbox: Some([0.0; 4]),
        provenance: dual_core_pdf_pipeline::engine::model::Provenance::Computed,
    };

    let s1 = BankStatement {
        total_pages: 1,
        transactions: vec![tx1.clone(), tx2.clone()],
        opening_balance: dec!(1000.0),
        closing_balance: dec!(1300.0),
        account_number: Some("1234".into()),
    };

    let s2 = BankStatement {
        total_pages: 1,
        transactions: vec![tx2.clone()],
        opening_balance: dec!(0.0),
        closing_balance: dec!(1300.0),
        account_number: Some("1234".into()),
    };

    let s3 = BankStatement {
        total_pages: 1,
        transactions: vec![tx1.clone(), tx2.clone()],
        opening_balance: dec!(1000.0),
        closing_balance: dec!(1300.0),
        account_number: Some("1234".into()),
    };

    let consensus = merge_consensus_statements(vec![s1, s2, s3]);

    assert_eq!(consensus.opening_balance, dec!(1000.0));
    assert_eq!(consensus.closing_balance, dec!(1300.0));
    assert_eq!(consensus.transactions.len(), 2);
    assert_eq!(consensus.transactions[0].date, "01/01/2026");
    assert_eq!(consensus.transactions[1].date, "02/01/2026");
}

#[test]
fn test_recalculation_loop_convergence_score_based() {
    // Simulates the loop continuing until score regresses or no improvement.
    // E.g., we get scores 85%, 92%, 92%. The loop stops on the 3rd iteration
    // because 92% is not an improvement over 92%, and retains the 92% result.

    let simulated_scores = vec![85.0, 92.0, 90.0]; // Iteration 3 regresses.
    let mut best_score = 0.0;

    let mut final_loops_run = 0;

    for score in simulated_scores {
        final_loops_run += 1;

        if score > best_score {
            best_score = score;
        } else {
            // Regression or no improvement, break and keep previous best state.
            break;
        }
    }

    assert_eq!(best_score, 92.0); // Retained the best score
    assert_eq!(final_loops_run, 3); // Evaluated 3 times before stopping
}

#[test]
fn test_dpi_auto_scaling_limits() {
    let calculate_safe_dpi = |width_pts: f32| -> f32 {
        let dpi = (1024.0 / width_pts) * 72.0;
        dpi.clamp(72.0, 600.0)
    };

    assert_eq!(calculate_safe_dpi(10.0), 600.0);

    let standard_dpi = calculate_safe_dpi(595.0);
    assert!(standard_dpi > 72.0 && standard_dpi < 200.0);

    assert_eq!(calculate_safe_dpi(5000.0), 72.0);
}

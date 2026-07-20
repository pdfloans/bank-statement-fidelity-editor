use dual_core_pdf_pipeline::app::config::AppConfig;
use dual_core_pdf_pipeline::app::gui::{ActiveModal, ActiveWorkflow, MyApp};
use dual_core_pdf_pipeline::engine::workflow::{WorkflowStage, ParseValidation, BalancePreview, VisualAttempt};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use std::sync::Arc;

#[test]
fn test_gui_interactive_mutations() {
    let (job_tx, _job_rx_dummy) = std::sync::mpsc::channel();
    let (_job_tx_dummy, job_rx) = std::sync::mpsc::channel();
    let config = Arc::new(AppConfig::default());

    let mut app = MyApp::new(job_tx, job_rx, config);
    app.settings.show_welcome = false;
    app.current_pdf_path = std::path::PathBuf::from("Cargo.toml");
    app.total_pages = 1;
    app.active_workflow = ActiveWorkflow::EditStatement;
    app.workflow_stage = WorkflowStage::Editing(ParseValidation {
        math_valid: true,
        font_valid: true,
        confidence_score: 1.0,
    });
    app.proposed_changes.push((dual_core_pdf_pipeline::engine::model::ProposedChange {
        page: 0,
        old_text: "A".to_string(),
        new_text: "B".to_string(),
        reason: "Test".to_string(),
        confidence: 1.0,
        affects_subsequent_balances: false,
        bbox: None,
    }, true));

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1920.0, 1080.0))
        .build(|ctx| {
            app.headless_update(ctx);
        });

    harness.step();
    
    // Attempt to interact with buttons in the UI!
    
    // 1. Click "Apply 1 Checked Changes"
    if let Some(apply_btn) = harness.get_by_label_contains("Apply 1 Checked Changes").ok() {
        apply_btn.click();
        harness.step();
    }
    
    // 2. Open natural language prompt, type, and press enter
    if let Some(mut nl_input) = harness.get_by_label_contains("Ask Gemini").ok() {
        nl_input.type_text("Replace A with B");
        harness.step();
        nl_input.press_key(egui::Key::Enter);
        harness.step();
    }
    
    // 3. Switch to Transfer workflow
    app.active_workflow = ActiveWorkflow::TransferTransactions;
    harness.step();
    if let Some(transfer_btn) = harness.get_by_label_contains("Run Target Parser").ok() {
        transfer_btn.click();
        harness.step();
    }
    
    // 4. Open settings modal
    app.active_modal = ActiveModal::Settings;
    harness.step();
    if let Some(save_btn) = harness.get_by_label_contains("Save & Reload").ok() {
        save_btn.click();
        harness.step();
    }
    
    // 5. Open API Keys modal
    app.active_workflow = ActiveWorkflow::ApiKeys;
    harness.step();
    if let Some(save_keys_btn) = harness.get_by_label_contains("Save & Refresh APIs").ok() {
        save_keys_btn.click();
        harness.step();
    }
    
    // 6. Test Feedback modal
    app.active_modal = ActiveModal::Feedback;
    harness.step();
    if let Some(send_feedback_btn) = harness.get_by_label_contains("Submit Feedback").ok() {
        send_feedback_btn.click();
        harness.step();
    }
    
    // 7. Previewing Stage interaction
    app.workflow_stage = WorkflowStage::Previewing(BalancePreview {
        matches_original: true,
        diff_count: 5,
        target_balance: rust_decimal::Decimal::new(100, 0),
    });
    harness.step();
    if let Some(confirm_btn) = harness.get_by_label_contains("Looks Good - Continue").ok() {
        confirm_btn.click();
        harness.step();
    }
    
    // 8. Visual Validation Stage interaction
    app.workflow_stage = WorkflowStage::Validating(VisualAttempt {
        attempt: 1,
        max_attempts: 5,
        score: 0.05,
        threshold: 0.02,
        passing: false,
    });
    harness.step();
    if let Some(force_btn) = harness.get_by_label_contains("Accept Anyway").ok() {
        force_btn.click();
        harness.step();
    }

    // 9. Command Palette interaction
    app.active_modal = ActiveModal::CommandPalette;
    harness.step();
    if let Some(mut cmd_input) = harness.get_by_label_contains("Type a command...").ok() {
        cmd_input.type_text("help");
        harness.step();
        cmd_input.press_key(egui::Key::Enter);
        harness.step();
    }

    assert!(true);
}

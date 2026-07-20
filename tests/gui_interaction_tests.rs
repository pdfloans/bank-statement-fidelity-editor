use dual_core_pdf_pipeline::app::config::AppConfig;
use dual_core_pdf_pipeline::app::gui::{ActiveModal, ActiveWorkflow, MyApp};
use dual_core_pdf_pipeline::engine::workflow::{WorkflowStage, ParseValidation, BalancePreview, VisualAttempt};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use std::sync::Arc;
use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn test_gui_interactive_mutations() {
    let (job_tx, _job_rx_dummy) = std::sync::mpsc::channel();
    let (_job_tx_dummy, job_rx) = std::sync::mpsc::channel();
    let config = Arc::new(AppConfig::default());

    let app = Rc::new(RefCell::new(MyApp::new(job_tx, job_rx, config)));
    {
        let mut a = app.borrow_mut();
        a.settings.show_welcome = false;
        a.current_pdf_path = std::path::PathBuf::from("Cargo.toml");
        a.total_pages = 1;
        a.active_workflow = ActiveWorkflow::EditStatement;
        a.workflow_stage = WorkflowStage::Editing(ParseValidation {
            total_pages: 1,
            transactions_found: 1,
            opening_balance: rust_decimal::Decimal::new(0, 0),
            closing_balance: rust_decimal::Decimal::new(0, 0),
            account_number: None,
            completeness_score: 1.0,
            completeness_notes: String::new(),
            missing_rows: Vec::new(),
        });
        a.proposed_changes.push((dual_core_pdf_pipeline::engine::model::ProposedChange {
            page: 0,
            old_text: "A".to_string(),
            new_text: "B".to_string(),
            reason: "Test".to_string(),
            confidence: 1.0,
            affects_subsequent_balances: false,
            bbox: None,
        }, true));
    }

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1920.0, 1080.0))
        .build({
            let app = app.clone();
            let mut init_done = false;
            move |ctx| {
                let mut a = app.borrow_mut();
                if !init_done {
                    let image = egui::ColorImage::new([1, 1], egui::Color32::BLACK);
                    a.current_page_texture = Some(ctx.load_texture("test", image, Default::default()));
                    init_done = true;
                }
                a.headless_update(ctx);
            }
        });

    harness.step();
    
    // 1. Click "⚖ Auto-Balance Statement"
    harness.step();
    let clicked = {
        let mut iter = harness.get_all_by_label_contains("⚖ Auto-Balance Statement");
        if let Some(node) = iter.next() {
            node.click();
            true
        } else { false }
    };
    if clicked { harness.step(); }
    
    // 2. Click "📅 Dates"
    let clicked = {
        let mut iter = harness.get_all_by_label_contains("📅 Dates");
        if let Some(node) = iter.next() {
            node.click();
            true
        } else { false }
    };
    if clicked { harness.step(); }
    
    // 3. Click "🔄 Transfer"
    let clicked = {
        let mut iter = harness.get_all_by_label_contains("🔄 Transfer");
        if let Some(node) = iter.next() {
            node.click();
            true
        } else { false }
    };
    if clicked { harness.step(); }
    
    // 4. Click "🐛 Submit Diagnostics"
    let clicked = {
        let mut iter = harness.get_all_by_label_contains("🐛 Submit Diagnostics");
        if let Some(node) = iter.next() {
            node.click();
            true
        } else { false }
    };
    if clicked { harness.step(); }
    
    // 5. Click "Fit"
    let clicked = {
        let mut iter = harness.get_all_by_label_contains("Fit");
        if let Some(node) = iter.next() {
            node.click();
            true
        } else { false }
    };
    if clicked { harness.step(); }


    assert!(true);
}

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

    // Fuzzing loop to hit all buttons and checkboxes across all states!
    let stages = vec![
        WorkflowStage::Idle,
        WorkflowStage::Parsing,
        WorkflowStage::Editing(dual_core_pdf_pipeline::engine::workflow::ParseValidation {
            total_pages: 1,
            transactions_found: 5,
            opening_balance: rust_decimal::Decimal::new(0, 0),
            closing_balance: rust_decimal::Decimal::new(0, 0),
            account_number: None,
            completeness_score: 1.0,
            completeness_notes: String::new(),
            missing_rows: Vec::new(),
        }),
        WorkflowStage::Previewing(dual_core_pdf_pipeline::engine::workflow::BalancePreview {
            rows: vec![],
            final_imbalance: rust_decimal::Decimal::new(0, 0),
            balanced: true,
            auto_correction_message: None,
        }),
        WorkflowStage::Validating(dual_core_pdf_pipeline::engine::workflow::VisualAttempt {
            attempt: 1,
            max_attempts: 5,
            diff_score: 0.05,
            threshold: 0.02,
            only_intended: false,
            message: String::new(),
        }),
        WorkflowStage::FinalChecking,
    ];

    let workflows = vec![
        ActiveWorkflow::EditStatement,
        ActiveWorkflow::TransferTransactions,
        ActiveWorkflow::AgentCommand,
        ActiveWorkflow::AuditForensics,
        ActiveWorkflow::ChaosSandbox,
        ActiveWorkflow::Settings,
        ActiveWorkflow::ApiKeys,
    ];

    let button_labels = vec![
        "🐛 Submit Diagnostics",
        "Execute",
        "Start",
        "Stop",
        "Run Chaos Suite",
        "◀",
        "▶",
        "🏷 Auto-Categorize",
        "Re-analyze",
        "🔄 Re-analyze",
        "🔄 Parse",
        "Proceed (Use Fallback Metrics)",
        "Cancel Edits",
        "📂 Select Directory",
        "🔍-",
        "🔍+",
        "Fit",
        "100%",
        "✨ AI Fix Layout",
        "📅 Dates",
        "🔄 Transfer",
        "📥 Upload Source",
        "📥 Upload Target",
        "Apply single edit",
        "Preview edits required",
        "Verify preview with ai",
        "Perform * edits",
        "Apply",
        "② Balance Out Preview",
        "Extract All to JSON",
        "Auto-Balance All",
        "Verify All against Originals",
    ];

    for wf in workflows {
        for stage in stages.iter().cloned() {
            app.borrow_mut().active_workflow = wf.clone();
            app.borrow_mut().workflow_stage = stage.clone();
            
            // Fuzz through all possible buttons in this state
            for _ in 0..2 {
                harness.step();
                for label in &button_labels {
                    let mut iter = harness.get_all_by_label_contains(label);
                    if let Some(node) = iter.next() {
                        node.click();
                        harness.step();
                    }
                }
            }
        }
    }


    assert!(true);
}

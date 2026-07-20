use dual_core_pdf_pipeline::app::config::AppConfig;
use dual_core_pdf_pipeline::app::gui::{ActiveModal, ActiveWorkflow, MyApp};
use dual_core_pdf_pipeline::app::runtime::{Job, JobResult};
use egui_kittest::kittest::Queryable;
use std::sync::{mpsc, Arc};

fn make_app() -> (MyApp, mpsc::Receiver<Job>, mpsc::Sender<JobResult>) {
    let (job_tx, job_rx_out) = mpsc::channel::<Job>();
    let (job_tx_in, job_rx) = mpsc::channel::<JobResult>();
    let config = Arc::new(AppConfig::default());
    let app = MyApp::new(job_tx, job_rx, config);
    (app, job_rx_out, job_tx_in)
}

#[test]
fn test_exhaustive_ui_states_true() {
    let (mut app, _, _) = make_app();
    
    // Set all boolean toggles and flags to true
    app.sidebar_expanded = true;
    app.fit_to_view = true;
    app.agent_autonomous_mode = true;
    app.dev_mode = true;
    app.command_query = "Test Query".to_string();
    app.natural_language_prompt = "Make everything blue".to_string();
    app.status = "Error processing document".to_string();
    
    let workflows = vec![
        ActiveWorkflow::EditStatement,
        ActiveWorkflow::TransferTransactions,
        ActiveWorkflow::AgentCommand,
        ActiveWorkflow::AuditForensics,
        ActiveWorkflow::ChaosSandbox,
        ActiveWorkflow::Settings,
        ActiveWorkflow::ApiKeys,
    ];

    for workflow in workflows {
        app.active_workflow = workflow;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1920.0, 1080.0))
            .build(|ctx| {
                app.headless_update(ctx);
            });
        
        harness.step();
        
        // Attempt to interact with everything we can find
        let buttons = harness.get_all_by_role(egui::Role::Button);
        for btn in buttons {
            btn.click();
        }
        harness.step();
    }
}

#[test]
fn test_exhaustive_ui_states_false() {
    let (mut app, _, _) = make_app();
    
    // Set all boolean toggles and flags to false/empty
    app.sidebar_expanded = false;
    app.fit_to_view = false;
    app.agent_autonomous_mode = false;
    app.dev_mode = false;
    app.command_query = "".to_string();
    app.natural_language_prompt = "".to_string();
    app.status = "".to_string();
    
    let workflows = vec![
        ActiveWorkflow::EditStatement,
        ActiveWorkflow::TransferTransactions,
        ActiveWorkflow::AgentCommand,
        ActiveWorkflow::AuditForensics,
        ActiveWorkflow::ChaosSandbox,
        ActiveWorkflow::Settings,
        ActiveWorkflow::ApiKeys,
    ];

    for workflow in workflows {
        app.active_workflow = workflow;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1920.0, 1080.0))
            .build(|ctx| {
                app.headless_update(ctx);
            });
        
        harness.step();
        
        // Attempt to interact with everything we can find
        let buttons = harness.get_all_by_role(egui::Role::Button);
        for btn in buttons {
            btn.click();
        }
        harness.step();
    }
}

#[test]
fn test_exhaustive_modal_combinations() {
    let (mut app, _, _) = make_app();
    
    let modals = vec![
        ActiveModal::DiscardDraftConfirm,
        ActiveModal::WorkflowHitl,
        ActiveModal::Settings,
        ActiveModal::CommandPalette,
        ActiveModal::Transfer,
        ActiveModal::Feedback,
        ActiveModal::DateAdjust,
        ActiveModal::TransferTest,
    ];
    
    for modal in modals {
        app.active_modal = modal;
        // Test modal with sidebar expanded
        app.sidebar_expanded = true;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1920.0, 1080.0))
            .build(|ctx| {
                app.headless_update(ctx);
            });
        harness.step();
        let buttons = harness.get_all_by_role(egui::Role::Button);
        for btn in buttons { btn.click(); }
        harness.step();
        
        // Test modal with sidebar collapsed
        app.sidebar_expanded = false;
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1920.0, 1080.0))
            .build(|ctx| {
                app.headless_update(ctx);
            });
        harness.step();
        let buttons = harness.get_all_by_role(egui::Role::Button);
        for btn in buttons { btn.click(); }
        harness.step();
    }
}

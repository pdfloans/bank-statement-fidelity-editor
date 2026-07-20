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
fn test_all_modals_coverage() {
    let (mut app, _, _) = make_app();
    
    let modals = vec![
        ActiveModal::None,
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
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1920.0, 1080.0))
            .build(|ctx| {
                app.headless_update(ctx);
            });
        
        harness.step();
        harness.step();
    }
}

#[test]
fn test_all_workflows_coverage() {
    let (mut app, _, _) = make_app();
    
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
        harness.step();
    }
}

#[test]
fn test_ui_interactions() {
    let (mut app, _, _) = make_app();
    
    // Test EditStatement specific UI interactions
    app.active_workflow = ActiveWorkflow::EditStatement;
    app.input_path = "dummy.pdf".to_string(); // To enable Parse button
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1920.0, 1080.0))
        .build(|ctx| {
            app.headless_update(ctx);
        });
    harness.step();

    if let Some(btn) = harness.get_all_by_label_contains("Fit").next() { btn.click(); }
    harness.step();
    if let Some(btn) = harness.get_all_by_label_contains("100%").next() { btn.click(); }
    harness.step();
    if let Some(btn) = harness.get_all_by_label_contains("🔍+").next() { btn.click(); }
    harness.step();
    if let Some(btn) = harness.get_all_by_label_contains("🔍-").next() { btn.click(); }
    harness.step();
    
    if let Some(btn) = harness.get_all_by_label_contains("Parse").next() { btn.click(); }
    harness.step();
}

#[test]
fn test_more_modal_interactions() {
    let (mut app, _, _) = make_app();
    app.active_modal = ActiveModal::Settings;
    
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1920.0, 1080.0))
        .build(|ctx| {
            app.headless_update(ctx);
        });
    harness.step();
    
    if let Some(btn) = harness.get_all_by_label_contains("Save").next() { btn.click(); }
    harness.step();
    if let Some(btn) = harness.get_all_by_label_contains("Cancel").next() { btn.click(); }
    harness.step();
}

#[test]
fn test_workflow_interactions() {
    let (mut app, _, _) = make_app();
    app.active_workflow = ActiveWorkflow::Settings;
    
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1920.0, 1080.0))
        .build(|ctx| {
            app.headless_update(ctx);
        });
    harness.step();
    
    if let Some(btn) = harness.get_all_by_label_contains("Save").next() { btn.click(); }
    harness.step();
}

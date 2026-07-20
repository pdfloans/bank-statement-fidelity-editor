use dual_core_pdf_pipeline::app::config::AppConfig;
use dual_core_pdf_pipeline::app::gui::{MyApp, ActiveModal};
use dual_core_pdf_pipeline::app::runtime::{Job, JobResult};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::sync::{mpsc, Arc};

#[test]
fn test_settings_modal_renders() {
    let (job_tx, _job_rx_dummy) = mpsc::channel::<Job>();
    let (_job_tx_dummy, job_rx) = mpsc::channel::<JobResult>();
    let config = Arc::new(AppConfig::default());
    let mut app = MyApp::new(job_tx, job_rx, config);

    // Open settings modal
    app.active_modal = ActiveModal::Settings;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1920.0, 1080.0))
        .build(|ctx| {
            app.headless_update(ctx);
        });

    harness.step();

    // Check that Settings modal appears
    assert!(harness.get_all_by_label_contains("Settings").next().is_some());
    // Verify some text inside the modal
    assert!(harness.get_all_by_label_contains("Max Retries").next().is_some());
}

#[test]
fn test_feedback_modal_renders() {
    let (job_tx, _job_rx_dummy) = mpsc::channel::<Job>();
    let (_job_tx_dummy, job_rx) = mpsc::channel::<JobResult>();
    let config = Arc::new(AppConfig::default());
    let mut app = MyApp::new(job_tx, job_rx, config);

    // Open feedback modal
    app.active_modal = ActiveModal::Feedback;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1920.0, 1080.0))
        .build(|ctx| {
            app.headless_update(ctx);
        });

    harness.step();

    // Check that Feedback modal appears
    assert!(harness.get_all_by_label_contains("Submit to Developer").next().is_some());
}

#[test]
fn test_command_palette_renders() {
    let (job_tx, _job_rx_dummy) = mpsc::channel::<Job>();
    let (_job_tx_dummy, job_rx) = mpsc::channel::<JobResult>();
    let config = Arc::new(AppConfig::default());
    let mut app = MyApp::new(job_tx, job_rx, config);

    // Open command palette
    app.active_modal = ActiveModal::CommandPalette;

    let mut harness = Harness::builder()
        .with_size(egui::vec2(1920.0, 1080.0))
        .build(|ctx| {
            app.headless_update(ctx);
        });

    harness.step();

    // Check that Command Palette appears
    assert!(harness.get_all_by_label_contains("Command Palette").next().is_some());
}

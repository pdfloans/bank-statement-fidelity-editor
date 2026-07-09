use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use dual_core_pdf_pipeline::app::gui::MyApp;
use dual_core_pdf_pipeline::app::config::AppConfig;
use dual_core_pdf_pipeline::app::runtime::{Job, JobResult};
use std::sync::{Arc, mpsc};

#[test]
fn test_bank_statement_modifier_ui() {
    let (job_tx, _job_rx_dummy) = mpsc::channel::<Job>();
    let (_job_tx_dummy, job_rx) = mpsc::channel::<JobResult>();
    let config = Arc::new(AppConfig::default());
    
    let mut app = MyApp::new(job_tx, job_rx, config);
    
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1024.0, 768.0))
        .build(|ctx| {
            app.headless_update(ctx);
        });

    harness.step();

    // Verify basic rendering
    harness.step();
    
    // Simulate clicking the "File" menu
    harness.get_by_label("File").click();
    harness.step();
    
    // Check that "Open PDF..." appears after clicking File (get_by_label_contains panics if not found)
    let _open_pdf = harness.get_by_label_contains("Open PDF...");
}

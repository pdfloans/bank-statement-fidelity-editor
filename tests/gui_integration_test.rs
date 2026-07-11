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

    // Simulate clicking the "Transfer Transactions" workflow in the sidebar via its icon
    harness.get_by_label_contains("⇄").click();
    harness.step();
    
    // Check that "Source Statement" dropzone appears
    let _source_dropzone = harness.get_by_label_contains("Source Statement");

    // Click back to "Edit Statement" via its icon
    harness.get_by_label_contains("📄").click();
    harness.step();

    // Verify the "Editing Toolbox" appears
    let _toolbox = harness.get_by_label_contains("Editing Toolbox");

    // Test Settings Workflow
    harness.get_by_label_contains("⚙").click();
    harness.step();
    let _settings_header = harness.get_by_label_contains("App Settings");

    // Test API Keys Workflow
    harness.get_by_label_contains("🔑").click();
    harness.step();
    let _api_keys_header = harness.get_by_label_contains("API & Engine Preferences");
    
    // Test Modals (Trigger "Exit without saving" or something if applicable, 
    // but just checking the workflows gets us all the core screens).
}

use dual_core_pdf_pipeline::app::audit::AuditLog;
use dual_core_pdf_pipeline::app::config::AppConfig;
use dual_core_pdf_pipeline::app::runtime::{Job, Runtime};
use std::path::PathBuf;
use std::sync::Arc;

#[test]
fn test_runtime_stress_load() {
    let config = Arc::new(AppConfig::default());
    let audit_log = AuditLog::open(PathBuf::from("audit")).unwrap();

    // Spawn the runtime server
    let (_runtime, job_tx, _job_rx) = Runtime::start(audit_log, config);

    let test_pdf = PathBuf::from("examples/sample.pdf");

    // Dispatch 100 concurrent jobs to stress test the runtime queue
    let num_jobs = 100;
    for i in 0..num_jobs {
        let job = Job::LoadDocument {
            path: test_pdf.clone(),
            three_page_mode: false,
        };
        // Just verify we can push them without crashing
        assert!(job_tx.send(job).is_ok(), "Failed to enqueue job {}", i);
    }
}

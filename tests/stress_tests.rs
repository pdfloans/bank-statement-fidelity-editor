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

    // Dispatch 10 concurrent WorkflowParseAndValidate jobs to stress test the real AI APIs
    let num_jobs = 10;
    for i in 0..num_jobs {
        let job = Job::WorkflowParseAndValidate {
            input: test_pdf.clone(),
            version: None,
            parser_mode: dual_core_pdf_pipeline::app::config::DocumentParserMode::LlamaParse,
            ai_provider: dual_core_pdf_pipeline::app::config::AiProviderMode::GeminiApiKey,
            ignore_offline_fallback: false,
        };
        assert!(job_tx.send(job).is_ok(), "Failed to enqueue job {}", i);
    }

    // Wait for all 10 jobs to complete
    let mut completed = 0;
    while completed < num_jobs {
        let res = _job_rx.recv_timeout(std::time::Duration::from_secs(300)).expect("Timeout waiting for job completion");
        match res {
            dual_core_pdf_pipeline::app::runtime::JobResult::WorkflowParseValidated { .. } => {
                completed += 1;
                println!("Job completed successfully! {}/{}", completed, num_jobs);
            }
            dual_core_pdf_pipeline::app::runtime::JobResult::Error { message, .. } => {
                completed += 1;
                println!("Job failed: {} ({}/{})", message, completed, num_jobs);
            }
            _ => {} // Ignore progress updates
        }
    }
}

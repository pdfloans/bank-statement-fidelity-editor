use dual_core_pdf_pipeline::app::runtime::{Job, JobResult};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[test]
fn test_headless_server_e2e() {
    // We set a custom port to avoid clashing with anything else running
    std::env::set_var("PORT", "8181");
    
    let cfg = Arc::new(dual_core_pdf_pipeline::app::config::AppConfig::default());
    let (job_tx, job_rx) = mpsc::channel::<Job>();
    let (res_tx, res_rx) = mpsc::channel::<JobResult>();

    // We must spawn a fake worker thread that answers Job::Ping with JobResult::Pong
    // so that the /readyz endpoint works.
    let worker_handle = thread::spawn(move || {
        while let Ok(job) = job_rx.recv() {
            if let Job::Ping = job {
                let _ = res_tx.send(JobResult::Pong);
            }
        }
    });

    // Spawn the server in the background
    thread::spawn(move || {
        let _ = dual_core_pdf_pipeline::app::server::run_server(job_tx, res_rx, cfg);
    });

    // Give the server a moment to bind
    thread::sleep(Duration::from_millis(500));

    // Test 1: Liveness probe
    let health_resp = reqwest::blocking::get("http://localhost:8181/health").expect("Failed to fetch /health");
    assert_eq!(health_resp.status(), 200);
    assert_eq!(health_resp.text().unwrap(), r#"{"status":"ok"}"#);

    // Test 2: Readiness probe
    let ready_resp = reqwest::blocking::get("http://localhost:8181/readyz").expect("Failed to fetch /readyz");
    assert_eq!(ready_resp.status(), 200);
    assert_eq!(ready_resp.text().unwrap(), r#"{"status":"ready"}"#);

    // Test 3: Root HTML landing page
    let root_resp = reqwest::blocking::get("http://localhost:8181/").expect("Failed to fetch /");
    assert_eq!(root_resp.status(), 200);
    let html = root_resp.text().unwrap();
    assert!(html.contains("<!DOCTYPE html>"));
    assert!(html.contains("Headless Backend Mode"));
}

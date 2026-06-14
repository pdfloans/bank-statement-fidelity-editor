//! Exhaustive N×(N−1) cross-transfer stress test for AU bank statements.
//!
//! For every ordered pair of AU bank statements (source → target), this test
//! exercises the full 9-stage transfer pipeline:
//!   1. Parse source via Document AI
//!   2. Parse target via Document AI
//!   3. AI format mapping via Gemini
//!   4. Balance recomputation
//!   5. PDF surgery (page clone/remove, batch text edits)
//!   6. Visual fidelity verification
//!   7. Math verification (engine)
//!   8. Math verification (Gemini)
//!   9. Final audit report
//!
//! With 8 AU statements this generates 56 directional test pairs.

use dual_core_pdf_pipeline::app::audit::AuditLog;
use dual_core_pdf_pipeline::app::config::AppConfig;
use dual_core_pdf_pipeline::app::runtime::{Job, JobResult, Runtime};
use dual_core_pdf_pipeline::engine::transfer::TransferResult;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::tempdir;

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Drain the result channel until a predicate matches or the deadline passes.
fn drain_until<F: Fn(&JobResult) -> bool>(
    rx: &std::sync::mpsc::Receiver<JobResult>,
    pred: F,
    max: Duration,
) -> Option<JobResult> {
    let deadline = Instant::now() + max;
    while Instant::now() < deadline {
        if let Ok(r) = rx.recv_timeout(Duration::from_millis(500)) {
            if pred(&r) {
                return Some(r);
            }
        }
    }
    None
}

/// Collect all PDFs in a directory, sorted for deterministic ordering.
fn collect_pdfs(dir: &Path) -> Vec<PathBuf> {
    let mut pdfs: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("cannot read AU Bank Statements directory")
        .filter_map(|e| {
            let e = e.ok()?;
            let p = e.path();
            if p.extension().unwrap_or_default() == "pdf" {
                Some(p)
            } else {
                None
            }
        })
        .collect();
    pdfs.sort();
    pdfs
}

/// Human-friendly stem for logging.
fn stem(p: &Path) -> String {
    p.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

// ── Per-Pair Result ──────────────────────────────────────────────────────────

#[derive(Debug)]
struct PairResult {
    source: PathBuf,
    target: PathBuf,
    outcome: PairOutcome,
    duration: Duration,
}

#[derive(Debug)]
enum PairOutcome {
    Success(TransferResult),
    Failed { stage: String, message: String },
    Timeout,
}

impl PairResult {
    fn passed(&self) -> bool {
        matches!(&self.outcome, PairOutcome::Success(r) if r.math_verified)
    }
}

// ── The Test ─────────────────────────────────────────────────────────────────

#[test]
#[ignore]
fn test_all_au_transfer_pairs() {
    let cfg = Arc::new(AppConfig::from_env().unwrap());
    let dir_path = Path::new("AU Bank Statements");
    let pdfs = collect_pdfs(dir_path);

    eprintln!("\n╔══════════════════════════════════════════════════════════════╗");
    eprintln!("║  AU Bank Statement Cross-Transfer Stress Test              ║");
    eprintln!(
        "║  Statements: {}                                            ║",
        pdfs.len()
    );
    eprintln!(
        "║  Pairs:      {} (N×(N-1))                                  ║",
        pdfs.len() * (pdfs.len() - 1)
    );
    eprintln!("╚══════════════════════════════════════════════════════════════╝\n");

    for (i, p) in pdfs.iter().enumerate() {
        eprintln!("  [{}] {}", i, stem(p));
    }
    eprintln!();

    let total_start = Instant::now();
    let mut results: Vec<PairResult> = Vec::new();
    let total_pairs = pdfs.len() * (pdfs.len() - 1);
    let mut pair_idx = 0usize;

    for (si, source) in pdfs.iter().enumerate() {
        for (ti, target) in pdfs.iter().enumerate() {
            if si == ti {
                continue;
            }
            pair_idx += 1;

            eprintln!("\n┌─────────────────────────────────────────────────────────┐");
            eprintln!(
                "│  PAIR {}/{}: {} → {}",
                pair_idx,
                total_pairs,
                stem(source),
                stem(target)
            );
            eprintln!("└─────────────────────────────────────────────────────────┘");

            let pair_start = Instant::now();

            // Each pair gets its own Runtime to avoid state leaks.
            let tmp = tempdir().unwrap();
            let audit = AuditLog::open(tmp.path()).unwrap();
            let (_runtime, job_tx, job_rx) = Runtime::start(audit, cfg.clone());

            let output = tmp
                .path()
                .join(format!("{}__to__{}.pdf", stem(source), stem(target)));

            // Send the transfer job
            job_tx
                .send(Job::TransferTransactions {
                    source_pdf: source.clone(),
                    target_pdf: target.clone(),
                    output_pdf: output.clone(),
                })
                .unwrap();

            // Wait for completion — generous 5-minute timeout per pair
            let result = drain_until(
                &job_rx,
                |r| {
                    matches!(
                        r,
                        JobResult::TransferComplete(_)
                            | JobResult::TransferFailed { .. }
                            | JobResult::Error { .. }
                    )
                },
                Duration::from_secs(300),
            );

            let duration = pair_start.elapsed();

            let outcome = match result {
                Some(JobResult::TransferComplete(tr)) => {
                    eprintln!(
                        "  ✅ SUCCESS in {:.1}s — math:{} visual:{} score:{:.4} txns:{}→{}",
                        duration.as_secs_f64(),
                        if tr.math_verified { "✓" } else { "✗" },
                        if tr.visual_verified { "✓" } else { "✗" },
                        tr.visual_score,
                        tr.source_tx_count,
                        tr.target_tx_count,
                    );
                    PairOutcome::Success(tr)
                }
                Some(JobResult::TransferFailed { stage, message }) => {
                    eprintln!(
                        "  ❌ FAILED at stage '{}' in {:.1}s: {}",
                        stage,
                        duration.as_secs_f64(),
                        message
                    );
                    PairOutcome::Failed { stage, message }
                }
                Some(JobResult::Error { message, .. }) => {
                    eprintln!("  ❌ ERROR in {:.1}s: {}", duration.as_secs_f64(), message);
                    PairOutcome::Failed {
                        stage: "Runtime".into(),
                        message,
                    }
                }
                None => {
                    eprintln!("  ⏱ TIMEOUT after {:.1}s", duration.as_secs_f64());
                    PairOutcome::Timeout
                }
                _ => {
                    // Unexpected JobResult variant — should not happen since
                    // drain_until filters, but handle gracefully.
                    eprintln!(
                        "  ⚠ UNEXPECTED result variant in {:.1}s",
                        duration.as_secs_f64()
                    );
                    PairOutcome::Failed {
                        stage: "Unknown".into(),
                        message: "Unexpected JobResult variant".into(),
                    }
                }
            };

            results.push(PairResult {
                source: source.clone(),
                target: target.clone(),
                outcome,
                duration,
            });
        }
    }

    let total_duration = total_start.elapsed();

    // ── Summary ──────────────────────────────────────────────────────────────

    let passed = results.iter().filter(|r| r.passed()).count();
    let failed = results.len() - passed;

    eprintln!("\n╔══════════════════════════════════════════════════════════════╗");
    eprintln!("║  STRESS TEST SUMMARY                                       ║");
    eprintln!("╠══════════════════════════════════════════════════════════════╣");
    eprintln!(
        "║  Total pairs: {:<4} | Passed: {:<4} | Failed: {:<4}           ║",
        results.len(),
        passed,
        failed
    );
    eprintln!(
        "║  Total time:  {:.0}s                                         ║",
        total_duration.as_secs_f64()
    );
    eprintln!("╠══════════════════════════════════════════════════════════════╣");

    // Print a matrix view
    let n = pdfs.len();
    eprint!("║  {:>20} │", "TARGET →");
    for tp in &pdfs {
        eprint!(" {:>4}", &stem(tp)[..4.min(stem(tp).len())]);
    }
    eprintln!("  ║");
    eprintln!("║  ─────────────────────┼{}──║", "─────".repeat(n));

    for (si, sp) in pdfs.iter().enumerate() {
        eprint!("║  {:>20} │", &stem(sp)[..20.min(stem(sp).len())]);
        for (ti, _tp) in pdfs.iter().enumerate() {
            if si == ti {
                eprint!("    ·");
            } else {
                let idx = results
                    .iter()
                    .position(|r| r.source == *sp && r.target == pdfs[ti])
                    .unwrap();
                let sym = if results[idx].passed() {
                    "  ✓"
                } else {
                    "  ✗"
                };
                eprint!("  {}", sym);
            }
        }
        eprintln!("  ║");
    }
    eprintln!("╚══════════════════════════════════════════════════════════════╝\n");

    // Write JSON report
    let report_dir = PathBuf::from("audit/transfer_tests");
    let _ = std::fs::create_dir_all(&report_dir);
    let ts = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let report_path = report_dir.join(format!("stress_test_{ts}.json"));

    let report_json: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            serde_json::json!({
                "source": stem(&r.source),
                "target": stem(&r.target),
                "passed": r.passed(),
                "duration_secs": r.duration.as_secs_f64(),
                "outcome": match &r.outcome {
                    PairOutcome::Success(tr) => serde_json::json!({
                        "status": "success",
                        "math_verified": tr.math_verified,
                        "visual_verified": tr.visual_verified,
                        "visual_score": tr.visual_score,
                        "source_tx_count": tr.source_tx_count,
                        "target_tx_count": tr.target_tx_count,
                        "corrections": tr.corrections_applied,
                        "retries": tr.retries_attempted,
                    }),
                    PairOutcome::Failed { stage, message } => serde_json::json!({
                        "status": "failed",
                        "stage": stage,
                        "message": message,
                    }),
                    PairOutcome::Timeout => serde_json::json!({
                        "status": "timeout",
                    }),
                },
            })
        })
        .collect();

    let full_report = serde_json::json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "statement_count": pdfs.len(),
        "total_pairs": results.len(),
        "passed": passed,
        "failed": failed,
        "total_duration_secs": total_duration.as_secs_f64(),
        "pairs": report_json,
    });

    if let Err(e) = std::fs::write(
        &report_path,
        serde_json::to_string_pretty(&full_report).unwrap_or_default(),
    ) {
        eprintln!("Failed to write stress test report: {}", e);
    } else {
        eprintln!("Report written to: {}", report_path.display());
    }

    // Print failures for easy debugging
    if failed > 0 {
        eprintln!("\n── FAILURES ─────────────────────────────────────────────────");
        for r in &results {
            if !r.passed() {
                match &r.outcome {
                    PairOutcome::Failed { stage, message } => {
                        eprintln!(
                            "  {} → {}: [{}] {}",
                            stem(&r.source),
                            stem(&r.target),
                            stage,
                            message
                        );
                    }
                    PairOutcome::Timeout => {
                        eprintln!(
                            "  {} → {}: TIMEOUT ({:.0}s)",
                            stem(&r.source),
                            stem(&r.target),
                            r.duration.as_secs_f64()
                        );
                    }
                    PairOutcome::Success(tr) if !tr.math_verified => {
                        eprintln!(
                            "  {} → {}: math not verified (visual: {:.4})",
                            stem(&r.source),
                            stem(&r.target),
                            tr.visual_score
                        );
                    }
                    _ => {}
                }
            }
        }
    }
}

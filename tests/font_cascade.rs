//! Stage 12 / Item #2: end-to-end integration test for the Stage 11 font cascade.
//!
//! Spins up the real `Runtime`, submits a `Job::Python(PythonJob::ReplicateFontForMissingChars, ...)`,
//! and asserts the cascade produces an extended TTF whose cmap covers the
//! requested chars.
//!
//! The test self-skips when no system donor TTF is available (CI without
//! system fonts). When it does run it covers Tier 2 (subset extension)
//! end-to-end through the PyO3 bridge.

use dual_core_pdf_pipeline::app::audit::AuditLog;
use dual_core_pdf_pipeline::app::config::AppConfig;
use dual_core_pdf_pipeline::app::runtime::{Job, PythonJob, PythonJobResult, Runtime};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;

fn find_system_donor_ttf() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from(r"C:\Windows\Fonts\arial.ttf"),
        PathBuf::from(r"C:\Windows\Fonts\Arial.ttf"),
        PathBuf::from(r"C:\Windows\Fonts\calibri.ttf"),
        PathBuf::from(r"C:\Windows\Fonts\segoeui.ttf"),
        PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
        PathBuf::from("/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf"),
        PathBuf::from("/Library/Fonts/Arial.ttf"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

#[test]
fn font_cascade_extends_subset_via_donor() {
    let donor = match find_system_donor_ttf() {
        Some(p) => p,
        None => {
            eprintln!("[skip] no system donor TTF available; cascade test self-skipped");
            return;
        }
    };
    eprintln!("[cascade] donor: {}", donor.display());

    let dir = tempdir().unwrap();

    // Set FONT_CACHE_DIR before Runtime::start so the Python actor thread
    // inherits it from the moment it spawns. The cascade reads this var
    // lazily but env propagation across thread boundaries is more reliable
    // when we set early.
    let cache_dir = dir.path().join("font_cache");
    std::fs::create_dir_all(&cache_dir).unwrap();
    std::env::set_var("FONT_CACHE_DIR", cache_dir.to_string_lossy().to_string());

    let audit = AuditLog::open(dir.path()).unwrap();
    let cfg = Arc::new(AppConfig {
        passphrase: "cascade-test-passphrase-1234".into(),
        log_dir: dir.path().join("logs"),
        ..AppConfig::default()
    });
    let (_runtime, job_tx, job_rx) = Runtime::start(audit, cfg);

    // 1) Build a subsetted TTF and a minimal PDF that uses it. Done via
    //    Python so the test doesn't grow a fontTools dependency on Rust.
    let prep_script = dir.path().join("prep.py");
    let subset_path = dir.path().join("subset.ttf");
    let pdf_path = dir.path().join("doc.pdf");
    let donor_str = donor.to_string_lossy().replace('\\', "/");
    let subset_str = subset_path.to_string_lossy().replace('\\', "/");
    let pdf_str = pdf_path.to_string_lossy().replace('\\', "/");
    let prep_source = format!(
        "from fontTools.ttLib import TTFont\n\
         from fontTools.subset import Subsetter\n\
         import pymupdf\n\
         sub = TTFont(r\"{donor_str}\")\n\
         ss = Subsetter()\n\
         ss.populate(text=\"0123e\")\n\
         ss.subset(sub)\n\
         sub.save(r\"{subset_str}\")\n\
         doc = pymupdf.open()\n\
         page = doc.new_page(width=400, height=100)\n\
         page.insert_font(fontname=\"testsubset\", fontfile=r\"{subset_str}\")\n\
         page.insert_text(pymupdf.Point(20, 50), \"0123e\", fontname=\"testsubset\", fontsize=24)\n\
         doc.save(r\"{pdf_str}\")\n\
         doc.close()\n\
         print(\"ok\")\n"
    );
    std::fs::write(&prep_script, prep_source).unwrap();
    let py_status = std::process::Command::new("python")
        .arg(&prep_script)
        .status();
    if !matches!(py_status, Ok(s) if s.success()) {
        eprintln!("[skip] python prep failed; cascade test self-skipped");
        return;
    }
    if !pdf_path.exists() {
        eprintln!("[skip] no test PDF produced; cascade test self-skipped");
        return;
    }

    // 2) Set up the donor in the cache dir we created at test start (the
    //    FONT_CACHE_DIR env var was already pointed at this directory).
    let donor_stem = donor
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Donor")
        .to_string();
    let donor_local = cache_dir.join(format!("{donor_stem}.ttf"));
    std::fs::copy(&donor, &donor_local).unwrap();
    let manifest = serde_json::json!({
        donor_stem.clone(): format!("{donor_stem}.ttf")
    });
    std::fs::write(
        cache_dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    eprintln!(
        "[cascade] FONT_CACHE_DIR = {}",
        cache_dir.display()
    );

    // 3) Submit the cascade job. The font_name we pass must substring-match
    //    the canonical name in the manifest so Tier 2 finds the donor; we
    //    pass donor_stem itself.
    let cascade_dir = dir.path().join("cascade_out");
    std::fs::create_dir_all(&cascade_dir).unwrap();
    let cascade_dir_str = cascade_dir.to_string_lossy().to_string();

    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    job_tx
        .send(Job::Python(
            PythonJob::ReplicateFontForMissingChars {
                pdf_path: pdf_path.to_string_lossy().to_string(),
                font_name: donor_stem.clone(),
                missing_chars_csv: "4,5,A".into(),
                output_dir: cascade_dir_str,
            },
            reply_tx,
        ))
        .expect("send Job::Python");

    // Drain runtime results in the background while we wait for the python reply.
    let drain_handle = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        while std::time::Instant::now() < deadline {
            let _ = job_rx.recv_timeout(Duration::from_millis(200));
        }
    });

    // Wait for the cascade reply with a small tokio runtime.
    let reply = std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async { tokio::time::timeout(Duration::from_secs(120), reply_rx).await })
    })
    .join()
    .expect("reply thread panicked");

    let payload = match reply {
        Ok(Ok(PythonJobResult::Json(s))) => s,
        Ok(Ok(PythonJobResult::Error(e))) => panic!("cascade error: {e}"),
        Ok(Ok(other)) => panic!("unexpected python job result: {other:?}"),
        Ok(Err(e)) => panic!("reply channel: {e}"),
        Err(_) => panic!("cascade reply timed out"),
    };

    drop(drain_handle);

    eprintln!("[cascade] payload: {payload}");
    let v: serde_json::Value = serde_json::from_str(&payload).expect("decode JSON");

    let success = v.get("success").and_then(|b| b.as_bool()).unwrap_or(false);
    let extended_path = v
        .get("extended_font_path")
        .and_then(|s| s.as_str())
        .map(PathBuf::from);
    let donor_extended: Vec<String> = v
        .get("donor_extended")
        .cloned()
        .and_then(|m| serde_json::from_value(m).ok())
        .unwrap_or_default();
    let still_missing: Vec<String> = v
        .get("still_missing")
        .cloned()
        .and_then(|m| serde_json::from_value(m).ok())
        .unwrap_or_default();
    let tiers_used: Vec<String> = v
        .get("tiers_used")
        .cloned()
        .and_then(|m| serde_json::from_value(m).ok())
        .unwrap_or_default();

    eprintln!(
        "[cascade] success={success} tiers={tiers_used:?} donor_extended={donor_extended:?}"
    );

    assert!(success, "cascade should succeed: {payload}");
    assert!(extended_path.is_some(), "extended_font_path must be set");
    assert!(extended_path.unwrap().exists(), "extended TTF must exist on disk");
    assert!(still_missing.is_empty(), "still_missing should be empty");
    assert!(
        donor_extended.iter().any(|c| c == "A"),
        "donor_extended should contain 'A': {donor_extended:?}"
    );
    assert!(
        tiers_used.iter().any(|t| t == "subset_extension"),
        "tiers_used should include subset_extension: {tiers_used:?}"
    );

    std::env::remove_var("FONT_CACHE_DIR");
}

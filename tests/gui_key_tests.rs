use dual_core_pdf_pipeline::app::config::{AiProviderMode, AppConfig};
use dual_core_pdf_pipeline::app::gui::MyApp;
use flate2::read::GzDecoder;
use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex, OnceLock};
use tar::Archive;
use tempfile::TempDir;

static TEST_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

struct ScopedCurrentDir {
    original: PathBuf,
}

impl ScopedCurrentDir {
    fn new(dir: &Path) -> Self {
        let original = std::env::current_dir().expect("current dir");
        std::env::set_current_dir(dir).expect("set temp dir");
        Self { original }
    }
}

impl Drop for ScopedCurrentDir {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original);
    }
}

#[test]
fn test_save_credentials_updates_env_and_dotenv() {
    let _guard = TEST_MUTEX.get_or_init(|| Mutex::new(())).lock().unwrap();
    let temp_dir = TempDir::new().expect("create temp dir");
    let _cwd = ScopedCurrentDir::new(temp_dir.path());

    std::env::remove_var("GEMINI_API_KEY");
    std::env::remove_var("PDFREST_API_KEY");
    std::env::remove_var("AI_PROVIDER");
    std::env::remove_var("USE_APPLITOOLS");

    let (job_tx, _job_rx) = mpsc::channel();
    let (_job_tx, job_rx) = mpsc::channel();
    let config = std::sync::Arc::new(AppConfig::default());
    let mut app = MyApp::new(job_tx, job_rx, config);

    app.edit_gemini_api_key = "test-gemini-key".to_string();
    app.edit_pdfrest_api_key = "test-pdfrest-key".to_string();
    app.settings.ai_provider = AiProviderMode::GroqApiKey;
    app.settings.use_applitools = false;

    app.save_credentials();

    let dotenv = fs::read_to_string(".env").expect("read .env");
    assert!(dotenv.contains("GEMINI_API_KEY=test-gemini-key"));
    assert!(dotenv.contains("PDFREST_API_KEY=test-pdfrest-key"));
    assert!(dotenv.contains("AI_PROVIDER=groq_api_key"));
    assert!(dotenv.contains("USE_APPLITOOLS=0"));

    assert_eq!(std::env::var("GEMINI_API_KEY").unwrap(), "test-gemini-key");
    assert_eq!(
        std::env::var("PDFREST_API_KEY").unwrap(),
        "test-pdfrest-key"
    );
    assert_eq!(std::env::var("AI_PROVIDER").unwrap(), "groq_api_key");
    assert_eq!(std::env::var("USE_APPLITOOLS").unwrap(), "0");
}

#[test]
fn test_export_to_excel_writes_history_and_emits_toast() {
    let _guard = TEST_MUTEX.get_or_init(|| Mutex::new(())).lock().unwrap();
    let temp_dir = TempDir::new().expect("create temp dir");
    let _cwd = ScopedCurrentDir::new(temp_dir.path());

    let (job_tx, _job_rx) = mpsc::channel();
    let (_job_tx, job_rx) = mpsc::channel();
    let config = std::sync::Arc::new(AppConfig::default());
    let mut app = MyApp::new(job_tx, job_rx, config);

    app.history_state.push_change(
        0,
        "old text".to_string(),
        "new text".to_string(),
        [0.0, 0.0, 100.0, 20.0],
        "change description".to_string(),
    );

    app.export_to_excel();

    let output_path = Path::new("output/export.xlsx");
    assert!(
        output_path.exists(),
        "expected output Excel workbook to be created"
    );

    let toast = app.last_toast().expect("expected at least one toast");
    assert_eq!(
        toast.kind,
        dual_core_pdf_pipeline::app::gui::ToastKind::Success
    );
    assert!(toast.text.contains("Exported history"));
}

#[test]
fn test_build_artifact_bundle_includes_input_output_and_audit_files() {
    let _guard = TEST_MUTEX.get_or_init(|| Mutex::new(())).lock().unwrap();
    let temp_dir = TempDir::new().expect("create temp dir");
    let _cwd = ScopedCurrentDir::new(temp_dir.path());

    fs::write("input.pdf", b"PDF content").expect("write input pdf");
    fs::create_dir_all("output").expect("create output dir");
    fs::write("output/edited.pdf", b"edited content").expect("write edited pdf");
    fs::create_dir_all("audit").expect("create audit dir");
    fs::write("audit/log1.txt", b"audit log").expect("write audit log");
    fs::write("audit/change_history.json", b"{}").expect("write history json");

    let bundle_path = Path::new("bundle.tar.gz");
    MyApp::build_artifact_bundle("input.pdf", Path::new("output/edited.pdf"), bundle_path)
        .expect("build artifact bundle");

    assert!(bundle_path.exists());

    let file = File::open(bundle_path).expect("open bundle");
    let mut archive = Archive::new(GzDecoder::new(file));
    let mut entries = archive
        .entries()
        .expect("read entries")
        .map(|entry| {
            entry
                .expect("entry")
                .path()
                .expect("entry path")
                .into_owned()
        })
        .collect::<Vec<PathBuf>>();
    entries.sort();

    let expected = vec![
        PathBuf::from("bundle/audit/change_history.json"),
        PathBuf::from("bundle/audit/log1.txt"),
        PathBuf::from("bundle/change_history.json"),
        PathBuf::from("bundle/edited.pdf"),
        PathBuf::from("bundle/input.pdf"),
    ];

    assert_eq!(entries, expected);
}

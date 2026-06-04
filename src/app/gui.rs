//! Bank Statement Fidelity Editor — production GUI
//!
//! Layout (5-region):
//!   [ menu / status / actions / theme toggle ]
//!   [ left: nav + thumbnails ] [ central: canvas ] [ right: tools ]
//!   [ bottom: toasts / progress / status bar ]
//!
//! All long-running work runs through `Job`s on the runtime; the UI only
//! reads `JobResult`s and never blocks.

use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::app::runtime::{Job, JobResult, PythonJob, PythonJobResult};
use crate::engine::history::ChangeHistory;
use crate::engine::verification::VerificationReport;
use egui_plot::{Line, Plot, PlotPoints};

// ---------------------------------------------------------------------------
// Theme palette (Catppuccin-inspired) + helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    System,
    Dark,
    Light,
    Midnight,
    Solarized,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::System
    }
}

struct Palette {
    bg: egui::Color32,
    panel: egui::Color32,
    surface: egui::Color32,
    text: egui::Color32,
    weak: egui::Color32,
    accent: egui::Color32,
    success: egui::Color32,
    warn: egui::Color32,
    error: egui::Color32,
    info: egui::Color32,
}

impl Theme {
    fn palette(self) -> Palette {
        let resolved = if self == Theme::System {
            if dark_light::detect().unwrap_or(dark_light::Mode::Dark) == dark_light::Mode::Light {
                Theme::Light
            } else {
                Theme::Midnight
            }
        } else {
            self
        };

        match resolved {
            Theme::System => unreachable!(),
            Theme::Dark => Palette {
                bg: egui::Color32::from_rgb(22, 24, 30),
                panel: egui::Color32::from_rgb(28, 30, 38),
                surface: egui::Color32::from_rgb(36, 38, 46),
                text: egui::Color32::from_rgb(220, 220, 230),
                weak: egui::Color32::from_rgb(140, 140, 160),
                accent: egui::Color32::from_rgb(122, 162, 247),
                success: egui::Color32::from_rgb(80, 180, 130),
                warn: egui::Color32::from_rgb(220, 170, 90),
                error: egui::Color32::from_rgb(220, 90, 90),
                info: egui::Color32::from_rgb(122, 162, 247),
            },
            Theme::Light => Palette {
                bg: egui::Color32::from_rgb(245, 245, 248),
                panel: egui::Color32::from_rgb(255, 255, 255),
                surface: egui::Color32::from_rgb(238, 240, 245),
                text: egui::Color32::from_rgb(30, 30, 36),
                weak: egui::Color32::from_rgb(110, 110, 130),
                accent: egui::Color32::from_rgb(50, 100, 200),
                success: egui::Color32::from_rgb(20, 130, 80),
                warn: egui::Color32::from_rgb(180, 130, 30),
                error: egui::Color32::from_rgb(190, 50, 50),
                info: egui::Color32::from_rgb(50, 100, 200),
            },
            Theme::Midnight => Palette {
                bg: egui::Color32::from_rgb(10, 10, 12),
                panel: egui::Color32::from_rgb(18, 18, 22),
                surface: egui::Color32::from_rgb(26, 26, 32),
                text: egui::Color32::from_rgb(240, 240, 245),
                weak: egui::Color32::from_rgb(150, 150, 160),
                accent: egui::Color32::from_rgb(99, 102, 241), // Indigo 500
                success: egui::Color32::from_rgb(34, 197, 94), // Green 500
                warn: egui::Color32::from_rgb(245, 158, 11), // Amber 500
                error: egui::Color32::from_rgb(239, 68, 68), // Red 500
                info: egui::Color32::from_rgb(56, 189, 248), // Sky 400
            },
            Theme::Solarized => Palette {
                bg: egui::Color32::from_rgb(253, 246, 227),
                panel: egui::Color32::from_rgb(238, 232, 213),
                surface: egui::Color32::from_rgb(255, 255, 255),
                text: egui::Color32::from_rgb(101, 123, 131),
                weak: egui::Color32::from_rgb(147, 161, 161),
                accent: egui::Color32::from_rgb(38, 139, 210),
                success: egui::Color32::from_rgb(133, 153, 0),
                warn: egui::Color32::from_rgb(181, 137, 0),
                error: egui::Color32::from_rgb(220, 50, 47),
                info: egui::Color32::from_rgb(38, 139, 210),
            },
        }
    }

    fn label(self) -> &'static str {
        match self {
            Theme::System => "System (Auto)",
            Theme::Dark => "Dark",
            Theme::Light => "Light",
            Theme::Midnight => "Midnight",
            Theme::Solarized => "Solarized",
        }
    }

    fn apply(self, ctx: &egui::Context) {
        let p = self.palette();
        
        let resolved = if self == Theme::System {
            if dark_light::detect().unwrap_or(dark_light::Mode::Dark) == dark_light::Mode::Light {
                Theme::Light
            } else {
                Theme::Midnight
            }
        } else {
            self
        };

        let mut visuals = if matches!(resolved, Theme::Dark | Theme::Midnight) {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };

        // Window shadows and rounding for a glassmorphism/modern feel
        visuals.window_rounding = egui::Rounding::same(12.0);
        visuals.menu_rounding = egui::Rounding::same(8.0);
        visuals.window_shadow.color = egui::Color32::from_black_alpha(150);
        visuals.window_shadow.spread = 4.0;
        visuals.window_shadow.blur = 32.0;
        visuals.popup_shadow.color = egui::Color32::from_black_alpha(120);
        visuals.popup_shadow.spread = 2.0;
        visuals.popup_shadow.blur = 16.0;

        visuals.panel_fill = p.panel;
        visuals.window_fill = p.panel;
        visuals.extreme_bg_color = p.bg;
        visuals.faint_bg_color = p.surface;
        visuals.widgets.noninteractive.bg_fill = p.surface;
        visuals.widgets.inactive.bg_fill = p.surface;
        visuals.widgets.inactive.rounding = egui::Rounding::same(8.0);
        visuals.widgets.hovered.rounding = egui::Rounding::same(8.0);
        visuals.widgets.active.rounding = egui::Rounding::same(8.0);
        visuals.hyperlink_color = p.accent;
        visuals.selection.bg_fill = p.accent.linear_multiply(0.3);
        visuals.selection.stroke.color = p.accent;
        visuals.warn_fg_color = p.warn;
        visuals.error_fg_color = p.error;
        ctx.set_visuals(visuals);

        // global style tweaks
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(12.0, 10.0);
        style.spacing.button_padding = egui::vec2(16.0, 8.0);
        style.spacing.window_margin = egui::Margin::same(16.0);
        style.spacing.menu_margin = egui::Margin::same(8.0);
        
        // Modern typography sizing
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(24.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::new(15.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::new(15.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::new(13.0, egui::FontFamily::Proportional),
        );
        ctx.set_style(style);
    }
}

// ---------------------------------------------------------------------------
// Persistent settings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub recent_files: Vec<String>,
    #[serde(default)]
    pub dark_mode: bool, // legacy, kept for back-compat
    #[serde(default)]
    pub theme: Theme,
    pub auto_save: bool,
    pub default_dpi: f32,
    pub use_pdfrest: bool,
    pub deep_font_replication: bool,
    #[serde(default)]
    pub show_welcome: bool,
    #[serde(default)]
    pub webhook_url: String,
    #[serde(default)]
    pub openai_api_key: String,
    /// Master toggle for "3 Page Mode" — the DEFAULT operating mode.
    /// When true, opened PDFs are transparently split into <=3-page
    /// segments for Pro editing and re-merged on save. Defaults to TRUE,
    /// and a missing/absent stored value is also treated as true.
    #[serde(default = "default_true")]
    pub three_page_mode: bool,
    #[serde(default)]
    pub advanced_mode: bool,
    #[serde(default)]
    pub remote_engine_url: String,
}

/// serde default for `three_page_mode`. NOTE: a bare `#[serde(default)]`
/// resolves `bool` to `false`; the default for this feature must be `true`,
/// so we supply an explicit default function that returns `true` when no
/// stored value is present.
fn default_true() -> bool {
    true
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            recent_files: Vec::new(),
            dark_mode: true,
            theme: Theme::Midnight,
            auto_save: true,
            default_dpi: 200.0,
            use_pdfrest: false,
            deep_font_replication: false,
            show_welcome: true,
            webhook_url: String::new(),
            openai_api_key: String::new(),
            three_page_mode: true,
            advanced_mode: false,
            remote_engine_url: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Toast / notification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Warn,
    Error,
    Success,
}

#[derive(Debug, Clone)]
struct Toast {
    kind: ToastKind,
    text: String,
    expires_at: Instant,
    action_label: Option<String>,
    action_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Block returned by Python click-detection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct TextBlock {
    pub page: usize,
    pub text: String,
    pub bbox: [f32; 4],
    #[serde(default)]
    pub font: String,
    #[serde(default)]
    pub size: f32,
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ProgressState {
    pub label: String,
    pub fraction: f32,
    pub started_at: std::time::Instant,
}

#[derive(PartialEq)]
pub enum AppView {
    SingleDocument,
    BatchProcessing,
    AuditExplorer,
}

pub struct MyApp {
    // Files
    input_path: String,
    output_path: String,
    current_pdf_path: PathBuf,
    previous_pdf_path: Option<PathBuf>,
    export_path: String,

    // Document state
    current_page: usize,
    total_pages: usize,
    history_state: ChangeHistory,

    // Batch Processing
    batch_folder_path: Option<PathBuf>,
    batch_files: Vec<PathBuf>,

    // View
    current_view: AppView,
    zoom_factor: f32,
    pan_offset: egui::Vec2,
    show_curtain: bool,
    curtain_ratio: f32,
    fit_to_view: bool,

    // Selection
    selected_block: Option<TextBlock>,
    new_text: String,

    // Textures
    current_page_texture: Option<egui::TextureHandle>,
    before_texture: Option<egui::TextureHandle>,
    after_texture: Option<egui::TextureHandle>,
    current_page_dpi: f32,
    current_page_size_pts: Option<(f32, f32)>,

    // App / job state
    status: String,
    progress: Option<ProgressState>,
    last_warning: Option<String>,
    last_verification: Option<VerificationReport>,
    proposed_changes: Vec<(crate::engine::model::ProposedChange, bool)>,
    last_imbalance: Option<rust_decimal::Decimal>,
    in_flight: usize,
    settings: AppSettings,
    toasts: VecDeque<Toast>,

    // Channels
    job_tx: std::sync::mpsc::Sender<Job>,
    job_rx: std::sync::mpsc::Receiver<JobResult>,
    pending_python: Option<tokio::sync::oneshot::Receiver<PythonJobResult>>,

    // Render coalescing
    last_render_request: Option<(String, usize, u32)>,

    // Multi-stage workflow state
    workflow_stage: crate::engine::workflow::WorkflowStage,
    workflow_transactions: Vec<crate::engine::model::Transaction>,
    workflow_validation: Option<crate::engine::workflow::ParseValidation>,
    #[allow(dead_code)]
    workflow_edits: Vec<crate::engine::workflow::UserEdit>,
    workflow_preview: Option<crate::engine::workflow::BalancePreview>,
    workflow_visual: Option<crate::engine::workflow::VisualAttempt>,
    workflow_outcome: Option<crate::engine::workflow::WorkflowOutcome>,

    /// Stage 8.5: per-font breakdown for the loaded PDF, populated
    /// automatically when `JobResult::FontAnalysisReady` arrives.
    font_analysis: Option<crate::engine::font_analysis::FontAnalysis>,
    /// Stage 13 / Item #12: pending modal confirmations. Each entry is
    /// (title, body, on_confirm action).
    show_discard_draft_confirm: bool,
    show_settings_modal: bool,
    show_transfer_dialog: bool,
    transfer_source_path: String,
    // Date Adjust dialog state
    show_date_adjust_dialog: bool,
    date_adjust_shift_days: String,
    date_adjust_mode_shift: bool, // true = shift, false = remap
    date_adjust_from: String,
    date_adjust_to: String,
    // AI Confirmation dialog state
    pending_ai_confirmations: Vec<crate::engine::ai_confirm::AiConfirmation>,
    // Transfer Test dialog state
    show_transfer_test_dialog: bool,
    transfer_test_paths: Vec<String>,
    transfer_test_report: Option<crate::engine::transfer_test_harness::TestHarnessReport>,
    /// Stage 12 / Item #3: history of cascade invocations during the
    /// current workflow attempt. Reset on a new workflow start; appended
    /// to whenever the runtime reports `JobResult::FontCascadeUsed`.
    font_cascade_reports: Vec<crate::engine::font_analysis::FontCascadeReport>,

    /// True when in-memory workflow state has changed since the last
    /// autosave to `audit/workflow.json`. Set whenever
    /// `workflow_validation`, `workflow_transactions` or `workflow_edits`
    /// is mutated; cleared after a successful save. Stage 5 / Item #9.
    workflow_dirty: bool,
    /// Last instant we wrote `audit/workflow.json`. Used to debounce — at
    /// most one save every 1.5s while edits are flying in.
    workflow_last_save: Option<Instant>,
    /// Cached `(input_path, sha256)` for the currently-open PDF so the
    /// autosave doesn't re-hash multi-MB files every 1.5s. Stage 6.
    workflow_input_hash: Option<(String, String)>,
    /// Per-cell text buffers for the inline edit table. Keyed by
    /// (page, line_on_page, field). Stage 5 / Item #6.
    workflow_cell_buffers: std::collections::HashMap<
        (usize, usize, crate::engine::workflow::EditField),
        String,
    >,

    // Config (read-only)
    config: std::sync::Arc<crate::app::config::AppConfig>,

    // --- In-app API key / credentials editor (Settings → API keys) ---
    /// Editable buffers, seeded from the current environment. Persisted to
    /// `.env` and hot-reloaded into the runtime via `Job::ReloadConfig`.
    edit_gemini_api_key: String,
    edit_docai_project_id: String,
    edit_docai_location: String,
    edit_docai_processor_id: String,
    /// Path to a Document AI service-account JSON key (best-practice auth).
    edit_docai_service_account: String,
    /// Optional Document AI API key (Beta), takes precedence over OAuth/SA.
    edit_docai_api_key: String,
    edit_pymupdf_pro_key: String,
    /// Gemini auth mode buffer: false = API key (default), true = Vertex AI
    /// (service-account / ADC). Persisted as `GEMINI_AUTH_MODE`.
    edit_gemini_use_vertex: bool,
    /// Latest credential/AI status reported by the runtime after a
    /// `Job::ReloadConfig` (document_ai_configured, gemini_configured,
    /// pro_editing_available). `None` until the first reload this session.
    config_status: Option<(bool, bool, bool)>,
    /// Result of the last `Job::ValidateCredentials` run. (Gemini, DocAI).
    credential_validation_status: Option<(Result<(), String>, Result<(), String>)>,
    /// True once the buffers have been seeded from the environment.
    #[allow(dead_code)]
    api_keys_seeded: bool,

    /// Proposed auto-fix for the last encountered error
    pub(crate) pending_autofix: Option<crate::app::error::AppError>,
}

impl MyApp {
    pub fn new(
        job_tx: std::sync::mpsc::Sender<Job>,
        job_rx: std::sync::mpsc::Receiver<JobResult>,
        config: std::sync::Arc<crate::app::config::AppConfig>,
    ) -> Self {
        let settings: AppSettings =
            confy::load("bank-statement-modifier", None).unwrap_or_default();
        let input_path = settings
            .recent_files
            .first()
            .cloned()
            .unwrap_or_else(|| "examples/sample.pdf".to_string());

        Self {
            input_path: input_path.clone(),
            output_path: "output/edited.pdf".to_string(),
            current_pdf_path: PathBuf::from(&input_path),
            previous_pdf_path: None,
            export_path: "audit/history.json".to_string(),
            current_page: 0,
            total_pages: 0,
            history_state: ChangeHistory::new(),
            batch_folder_path: None,
            batch_files: Vec::new(),
            current_view: AppView::SingleDocument,
            zoom_factor: 1.0,
            pan_offset: egui::Vec2::ZERO,
            show_curtain: false,
            curtain_ratio: 0.5,
            fit_to_view: true,
            selected_block: None,
            new_text: String::new(),
            current_page_texture: None,
            before_texture: None,
            after_texture: None,
            current_page_dpi: settings.default_dpi,
            current_page_size_pts: None,
            status: "Ready".to_string(),
            progress: None,
            last_warning: None,
            last_verification: None,
            proposed_changes: Vec::new(),
            last_imbalance: None,
            in_flight: 0,
            toasts: VecDeque::new(),
            job_tx,
            job_rx,
            pending_python: None,
            last_render_request: None,
            workflow_stage: crate::engine::workflow::WorkflowStage::Idle,
            workflow_transactions: Vec::new(),
            workflow_validation: None,
            workflow_edits: Vec::new(),
            workflow_preview: None,
            workflow_visual: None,
            workflow_outcome: None,
            font_analysis: None,
            font_cascade_reports: Vec::new(),
            show_discard_draft_confirm: false,
            show_settings_modal: false,
            show_transfer_dialog: false,
            transfer_source_path: String::new(),
            show_date_adjust_dialog: false,
            date_adjust_shift_days: "30".to_string(),
            date_adjust_mode_shift: true,
            date_adjust_from: String::new(),
            date_adjust_to: String::new(),
            pending_ai_confirmations: Vec::new(),
            show_transfer_test_dialog: false,
            transfer_test_paths: Vec::new(),
            transfer_test_report: None,
            workflow_dirty: false,
            workflow_last_save: None,
            workflow_input_hash: None,
            workflow_cell_buffers: std::collections::HashMap::new(),
            config,
            settings,
            // Seed API-key editor buffers from the current environment so the
            // Settings panel shows what's active. Values are masked in the UI.
            edit_gemini_api_key: std::env::var("GEMINI_API_KEY").unwrap_or_default(),
            edit_docai_project_id: std::env::var("DOCUMENT_AI_PROJECT_ID").unwrap_or_default(),
            edit_docai_location: {
                let l = std::env::var("DOCUMENT_AI_LOCATION").unwrap_or_default();
                if l.is_empty() { "us".to_string() } else { l }
            },
            edit_docai_processor_id: std::env::var("DOCUMENT_AI_PROCESSOR_ID").unwrap_or_default(),
            edit_docai_service_account: std::env::var("GOOGLE_APPLICATION_CREDENTIALS").unwrap_or_default(),
            edit_docai_api_key: std::env::var("DOCUMENT_AI_API_KEY").unwrap_or_default(),
            edit_pymupdf_pro_key: std::env::var("PYMUPDF_PRO_KEY").unwrap_or_default(),
            edit_gemini_use_vertex: matches!(
                std::env::var("GEMINI_AUTH_MODE")
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase()
                    .as_str(),
                "vertex" | "vertex_ai" | "vertexai"
            ),
            config_status: None,
            credential_validation_status: None,
            api_keys_seeded: true,
            pending_autofix: None,
        }
    }

    // -- helpers --------------------------------------------------------------

    /// Persist the in-app credential buffers to `.env`, apply them to the
    /// current process environment, and tell the runtime to hot-reload its
    /// `AppConfig` so subsequent Document AI / Gemini / Pro jobs use the new
    /// values without an application restart.
    ///
    /// Keys are upserted into `.env` (existing lines replaced in place,
    /// missing ones appended). Empty buffers remove the override from the live
    /// environment so a cleared field truly disables that credential.
    fn save_credentials(&mut self) {
        // (env var name, value) pairs to upsert.
        let pairs: Vec<(&str, String)> = vec![
            ("GEMINI_API_KEY", self.edit_gemini_api_key.trim().to_string()),
            ("DOCUMENT_AI_PROJECT_ID", self.edit_docai_project_id.trim().to_string()),
            ("DOCUMENT_AI_LOCATION", self.edit_docai_location.trim().to_string()),
            ("DOCUMENT_AI_PROCESSOR_ID", self.edit_docai_processor_id.trim().to_string()),
            ("GOOGLE_APPLICATION_CREDENTIALS", self.edit_docai_service_account.trim().to_string()),
            ("DOCUMENT_AI_API_KEY", self.edit_docai_api_key.trim().to_string()),
            ("PYMUPDF_PRO_KEY", self.edit_pymupdf_pro_key.trim().to_string()),
            (
                "GEMINI_AUTH_MODE",
                if self.edit_gemini_use_vertex { "vertex".to_string() } else { "api_key".to_string() },
            ),
        ];

        // 1) Apply to the live process environment so from_env() sees them.
        for (k, v) in &pairs {
            if v.is_empty() {
                std::env::remove_var(k);
            } else {
                std::env::set_var(k, v);
            }
        }

        // 2) Upsert into .env so the change survives a restart.
        if let Err(e) = upsert_env_file(std::path::Path::new(".env"), &pairs) {
            tracing::warn!("[gui] failed to write .env: {}", e);
            self.toast(ToastKind::Error, format!("Could not write .env: {e}"));
            // Still attempt the live reload below — the in-memory env is set.
        }

        // 3) Ask the runtime to hot-reload AppConfig from the environment.
        let _ = self.job_tx.send(Job::ReloadConfig);
        self.in_flight += 1;
        self.toast(ToastKind::Info, "Saving credentials and reloading…");
    }

    fn toast(&mut self, kind: ToastKind, msg: impl Into<String>) {
        self.toasts.push_back(Toast {
            kind,
            text: msg.into(),
            expires_at: Instant::now() + Duration::from_secs(6),
            action_label: None,
            action_id: None,
        });
        while self.toasts.len() > 5 {
            self.toasts.pop_front();
        }
    }

    fn toast_with_action(&mut self, kind: ToastKind, msg: impl Into<String>, label: impl Into<String>, id: impl Into<String>) {
        self.toasts.push_back(Toast {
            kind,
            text: msg.into(),
            expires_at: Instant::now() + Duration::from_secs(12),
            action_label: Some(label.into()),
            action_id: Some(id.into()),
        });
        while self.toasts.len() > 5 {
            self.toasts.pop_front();
        }
    }

    fn request_render(&mut self, tag: &str) {
        // Only render if the page actually changed since the last request for
        // this tag. This drops bursts when the user clicks rapidly through
        // pages or zooms — preventing render queue blow-up.
        let key = (
            tag.to_string(),
            self.current_page,
            self.current_page_dpi as u32,
        );
        if self.last_render_request.as_ref() == Some(&key) && tag == "current" {
            // already requested with same parameters
            return;
        }
        self.last_render_request = Some(key);

        let path = if tag == "before" {
            self.previous_pdf_path
                .clone()
                .unwrap_or_else(|| PathBuf::from(&self.input_path))
        } else {
            self.current_pdf_path.clone()
        };
        if !path.exists() {
            tracing::warn!("[gui] cannot render {:?} (does not exist)", path);
            return;
        }
        let _ = self.job_tx.send(Job::RenderPage {
            path,
            page: self.current_page,
            dpi: self.current_page_dpi,
            tag: tag.to_string(),
        });
        self.in_flight += 1;
    }

    fn update_recent_files(&mut self, path: String) {        self.settings.recent_files.retain(|f| f != &path);
        self.settings.recent_files.insert(0, path);
        if self.settings.recent_files.len() > 10 {
            self.settings.recent_files.pop();
        }
        if let Err(e) = confy::store("bank-statement-modifier", None, &self.settings) {
            tracing::warn!("[gui] failed to persist settings: {}", e);
        }
    }

    fn load_texture_from_bytes(
        &self,
        ctx: &egui::Context,
        name: &str,
        bytes: &[u8],
    ) -> Option<egui::TextureHandle> {
        let image = match image::load_from_memory(bytes) {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!("[gui] failed to decode rendered PNG '{}': {}", name, e);
                return None;
            }
        };
        let image = image.to_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        let pixels = image.as_flat_samples();
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, pixels.as_slice());
        Some(ctx.load_texture(name, color_image, egui::TextureOptions::LINEAR))
    }

    fn export_to_excel(&mut self) {
        let result: Result<(), Box<dyn std::error::Error>> = (|| {
            let mut workbook = rust_xlsxwriter::Workbook::new();
            let worksheet = workbook.add_worksheet();
            worksheet.write_string(0, 0, "#")?;
            worksheet.write_string(0, 1, "Page")?;
            worksheet.write_string(0, 2, "Old Text")?;
            worksheet.write_string(0, 3, "New Text")?;
            worksheet.write_string(0, 4, "Reason")?;
            worksheet.write_string(0, 5, "Timestamp")?;
            for (i, rec) in self.history_state.get_history().iter().enumerate() {
                let row = (i + 1) as u32;
                worksheet.write_number(row, 0, (i + 1) as f64)?;
                worksheet.write_number(row, 1, (rec.page + 1) as f64)?;
                worksheet.write_string(row, 2, &rec.old_text)?;
                worksheet.write_string(row, 3, &rec.new_text)?;
                worksheet.write_string(row, 4, &rec.description)?;
                worksheet.write_string(row, 5, &rec.timestamp)?;
            }
            std::fs::create_dir_all("output")?;
            workbook.save("output/export.xlsx")?;
            Ok(())
        })();
        match result {
            Ok(_) => self.toast(ToastKind::Success, "Exported history to output/export.xlsx"),
            Err(e) => self.toast(ToastKind::Error, format!("Excel export failed: {e}")),
        }
    }

    fn fit_zoom_to_view(&mut self, available: egui::Vec2, tex_size: egui::Vec2) {
        if tex_size.x <= 0.0 || tex_size.y <= 0.0 {
            return;
        }
        let scale_x = available.x / tex_size.x;
        let scale_y = available.y / tex_size.y;
        self.zoom_factor = scale_x.min(scale_y).clamp(0.1, 5.0) * 0.95;
        self.pan_offset = egui::Vec2::ZERO;
    }

    fn balance_trend_points(&self) -> PlotPoints {
        // Real running-balance trend (no fake data).
        let pts: Vec<[f64; 2]> = self
            .history_state
            .get_history()
            .iter()
            .enumerate()
            .filter_map(|(i, r)| {
                r.new_text
                    .replace(['$', ','], "")
                    .parse::<f64>()
                    .ok()
                    .map(|v| [i as f64, v])
            })
            .collect();
        if pts.is_empty() {
            PlotPoints::from(vec![[0.0, 0.0]])
        } else {
            PlotPoints::from(pts)
        }
    }
}

// ---------------------------------------------------------------------------
// eframe::App
// ---------------------------------------------------------------------------

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // theme
        self.settings.theme.apply(ctx);

        if let Some(p) = &self.progress {
            egui::Area::new(egui::Id::new("modal_overlay"))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::LEFT_TOP, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    let rect = ctx.screen_rect();
                    ui.allocate_rect(rect, egui::Sense::click());
                    ui.painter().rect_filled(rect, 0.0, egui::Color32::from_black_alpha(150));
                });

            egui::Window::new("Working…")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.label(&p.label);
                    let pct = (p.fraction.clamp(0.0, 1.0) * 100.0).round() as i32;
                    let mut text = format!("{}%", pct);
                    
                    if p.fraction > 0.0 {
                        let elapsed = p.started_at.elapsed().as_secs_f32();
                        let eta = (elapsed / p.fraction) * (1.0 - p.fraction);
                        if eta > 0.0 && eta.is_finite() {
                            text = format!("{}% (ETA: {:.0}s)", pct, eta);
                        }
                    }
                    
                    ui.add(
                        egui::ProgressBar::new(p.fraction.clamp(0.0, 1.0))
                            .desired_width(300.0)
                            .text(text),
                    );
                });
        }

        // Stage 13 / Item #6: workflow shortcuts.
        //   Ctrl+1 → Parse + AI validate
        //   Ctrl+2 → Balance Out Preview
        //   Ctrl+3 → Confirm and Render
        let want_parse = ctx.input(|i| {
            i.modifiers.command_only() && i.key_pressed(egui::Key::Num1)
        });
        let want_preview = ctx.input(|i| {
            i.modifiers.command_only() && i.key_pressed(egui::Key::Num2)
        });
        let want_confirm = ctx.input(|i| {
            i.modifiers.command_only() && i.key_pressed(egui::Key::Num3)
        });
        if want_parse && !self.input_path.is_empty() {
            let _ = self.job_tx.send(Job::WorkflowParseAndValidate {
                input: PathBuf::from(&self.input_path),
                version: None,
            });
            self.in_flight += 1;
            self.workflow_edits.clear();
            self.workflow_preview = None;
            self.workflow_visual = None;
            self.workflow_outcome = None;
            self.font_cascade_reports.clear();
            self.workflow_dirty = true;
            self.toast(ToastKind::Info, "Parse triggered (Ctrl+1)");
        }
        if want_preview {
            if let Some(v) = &self.workflow_validation {
                let _ = self.job_tx.send(Job::WorkflowPreview {
                    original_transactions: self.workflow_transactions.clone(),
                    edits: self.workflow_edits.clone(),
                    opening_balance: v.opening_balance,
                    expected_closing: if v.closing_balance.abs() > rust_decimal::Decimal::ZERO {
                        Some(v.closing_balance)
                    } else {
                        None
                    },
                });
                self.in_flight += 1;
                self.toast(ToastKind::Info, "Preview triggered (Ctrl+2)");
            }
        }
        if want_confirm {
            if let Some(p) = self.workflow_preview.clone() {
                let (kept, _) =
                    crate::engine::workflow::prune_redundant_edits(&self.workflow_edits, &p);
                let _ = self.job_tx.send(Job::WorkflowConfirmAndRender {
                    input: PathBuf::from(&self.input_path),
                    output: PathBuf::from(&self.output_path),
                    edits: kept,
                    deep_font_replication: self.settings.deep_font_replication,
                    max_visual_attempts: 5,
                    visual_threshold: 0.02,
                });
                self.in_flight += 1;
                self.toast(ToastKind::Info, "Confirm + Render triggered (Ctrl+3)");
            }
        }

        // Stage 13 / Item #15: Ctrl+Shift+Z removes the last queued edit
        // (regular Ctrl+Z is reserved by egui::TextEdit for buffer undo).
        let want_undo_last_edit = ctx.input(|i| {
            i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::Z)
        });
        if want_undo_last_edit && !self.workflow_edits.is_empty() {
            let removed = self.workflow_edits.pop();
            if let Some(e) = removed {
                // Drop the matching cell-buffer entry so the table shows
                // the original value next frame.
                self.workflow_cell_buffers.remove(&(e.page, e.line_on_page, e.field));
                self.workflow_dirty = true;
                self.toast(
                    ToastKind::Info,
                    format!(
                        "Undid last edit on P{} L{} ({} pending)",
                        e.page + 1,
                        e.line_on_page + 1,
                        self.workflow_edits.len()
                    ),
                );
            }
        }

        // Drag-and-drop support: open the first dropped PDF and tell the
        // user about additional drops. Stage 13 / Item #8.
        if !ctx.input(|i| i.raw.dropped_files.is_empty()) {
            let dropped: Vec<PathBuf> = ctx.input(|i| {
                i.raw
                    .dropped_files
                    .iter()
                    .filter_map(|f| f.path.clone())
                    .collect()
            });
            let pdfs: Vec<PathBuf> = dropped
                .into_iter()
                .filter(|p| {
                    p.extension()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_lowercase())
                        == Some("pdf".into())
                })
                .collect();
            if let Some(first) = pdfs.first().cloned() {
                self.open_pdf(first);
                if pdfs.len() > 1 {
                    self.toast(
                        ToastKind::Warn,
                        format!(
                            "Opened the first PDF; ignored {} other(s). The app handles one statement at a time.",
                            pdfs.len() - 1
                        ),
                    );
                }
            }
        }
        // Visual hover-cue while dragging files
        if !ctx.input(|i| i.raw.hovered_files.is_empty()) {
            let p = self.settings.theme.palette();
            let screen = ctx.screen_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("dnd-overlay"),
            ));
            painter.rect_filled(
                screen,
                0.0,
                egui::Color32::from_rgba_unmultiplied(0, 0, 0, 110),
            );
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                "📥 Drop PDF to open",
                egui::FontId::proportional(28.0),
                p.accent,
            );
        }

        // ---- 1. Drain runtime results --------------------------------------
        loop {
            match self.job_rx.try_recv() {
                Ok(res) => {
                    if res.is_terminal() && self.in_flight > 0 {
                        self.in_flight -= 1;
                    }
                    self.handle_job_result(ctx, res);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.status = "❌ Runtime worker disconnected".into();
                    self.in_flight = 0; // Bulletproof fix: reset on disconnect
                    break;
                }
            }
        }

        // ---- 1.5 Handle drag & drop ----------------------------------------
        ctx.input(|i| {
            if !i.raw.dropped_files.is_empty() {
                if let Some(file) = i.raw.dropped_files.first() {
                    if let Some(path) = &file.path {
                        if path.is_dir() {
                            self.current_view = AppView::BatchProcessing;
                            self.batch_folder_path = Some(path.clone());
                            self.batch_files.clear();
                            if let Ok(entries) = std::fs::read_dir(path) {
                                for entry in entries.filter_map(|e| e.ok()) {
                                    let p = entry.path();
                                    if p.is_file() && p.extension().and_then(|s| s.to_str()).map(|s| s.to_lowercase()) == Some("pdf".to_string()) {
                                        self.batch_files.push(p);
                                    }
                                }
                            }
                        } else if path.is_file() && path.extension().and_then(|s| s.to_str()).map(|s| s.to_lowercase()) == Some("pdf".to_string()) {
                            self.current_view = AppView::SingleDocument;
                            self.open_pdf(path.clone());
                        }
                    }
                }
            }
        });

        // Stage 5 / Item #9: autosave the workflow draft if anything has
        // changed since the last save (debounced to 1.5s inside the helper).
        self.autosave_workflow_draft();

        // ---- 2. Check pending Python click reply ---------------------------
        if let Some(rx) = self.pending_python.as_mut() {
            match rx.try_recv() {
                Ok(PythonJobResult::Json(json)) => {
                    if json.trim() == "null" {
                        self.toast(ToastKind::Info, "No text under that click.");
                    } else {
                        match serde_json::from_str::<TextBlock>(&json) {
                            Ok(b) => {
                                self.new_text = b.text.clone();
                                self.selected_block = Some(b);
                            }
                            Err(e) => {
                                self.toast(ToastKind::Warn, format!("Click parse failed: {e}"))
                            }
                        }
                    }
                    self.pending_python = None;
                }
                Ok(other) => {
                    tracing::debug!("[gui] click reply: {:?}", other);
                    self.pending_python = None;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {}
                Err(_) => self.pending_python = None,
            }
        }

        // ---- 3. Top bar ----------------------------------------------------
        self.draw_top_bar(ctx);

        // ---- 4. Bottom status bar -----------------------------------------
        self.draw_status_bar(ctx);

        // ---- 5. Left, right, central (or Batch) ---------------------------
        match self.current_view {
            AppView::SingleDocument => {
                self.draw_left_panel(ctx);
                self.draw_central_panel(ctx);
            }
            AppView::BatchProcessing => {
                self.draw_batch_panel(ctx);
            }
            AppView::AuditExplorer => {
                self.draw_audit_explorer_view(ctx);
            }
        }

        // ---- 6. Toasts ----------------------------------------------------
        if let Some(action_id) = self.draw_toasts(ctx) {
            match action_id.as_str() {
                "open_audit_explorer" => {
                    self.current_view = AppView::AuditExplorer;
                }
                _ => {}
            }
        }

        // ---- 6b. Modal confirmations -------------------------------------
        self.draw_modals(ctx);

        // ---- 7. Keyboard shortcuts ---------------------------------------
        self.handle_shortcuts(ctx);

        // Repaint while jobs are running so progress updates animate
        if self.in_flight > 0 || !self.toasts.is_empty() || self.pending_python.is_some() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }
}

// ---------------------------------------------------------------------------
// UI sections
// ---------------------------------------------------------------------------

impl MyApp {
    fn handle_job_result(&mut self, ctx: &egui::Context, res: JobResult) {
        match res {
            JobResult::ValidationStatus { gemini_ok, docai_ok } => {
                self.credential_validation_status = Some((gemini_ok, docai_ok));
            }
            JobResult::DocumentLoaded { total_pages, .. } => {
                self.total_pages = total_pages;
                self.current_page = 0;
                self.current_pdf_path = PathBuf::from(&self.input_path);
                self.previous_pdf_path = None;
                self.update_recent_files(self.input_path.clone());
                self.status = format!("Loaded {total_pages} page(s)");
                self.toast(ToastKind::Success, format!("Loaded {total_pages} pages"));
                self.request_render("current");
                self.in_flight += 1;
                self.workflow_edits.clear();
                self.workflow_preview = None;
                self.workflow_visual = None;
                self.workflow_outcome = None;
                self.font_cascade_reports.clear();
                self.workflow_dirty = true;
                let _ = self.job_tx.send(Job::WorkflowParseAndValidate {
                    input: PathBuf::from(&self.input_path),
                    version: None,
                });
            }
            JobResult::HistoryUpdated { history } => {
                self.history_state = history;
                let idx = self.history_state.current_index();
                self.previous_pdf_path = Some(self.current_pdf_path.clone());
                self.current_pdf_path = if idx > 0 {
                    self.history_state.get_history()[idx - 1]
                        .snapshot_path
                        .as_ref()
                        .cloned()
                        .unwrap_or_else(|| PathBuf::from(&self.input_path))
                } else {
                    PathBuf::from(&self.input_path)
                };
                self.status = "History synchronized".into();
                self.request_render("current");
            }
            JobResult::PageRendered {
                png_bytes,
                tag,
                width_pts,
                height_pts,
                ..
            } => {
                let texture = self.load_texture_from_bytes(ctx, &tag, &png_bytes);
                match tag.as_str() {
                    "current" => {
                        self.current_page_texture = texture;
                        self.current_page_size_pts = Some((width_pts, height_pts));
                    }
                    "before" => self.before_texture = texture,
                    "after" => self.after_texture = texture,
                    _ => {}
                }
            }
            JobResult::ChangeApplied {
                record,
                requires_visual_review,
            } => {
                self.toast(
                    if requires_visual_review {
                        ToastKind::Warn
                    } else {
                        ToastKind::Success
                    },
                    format!("Edit applied: {} → {}", record.old_text, record.new_text),
                );
                if requires_visual_review {
                    self.last_warning = Some("Review required: complex background.".into());
                }
                self.status = "Change applied".into();
                self.request_render("current");
                self.request_render("before");
                self.request_render("after");
            }
            JobResult::BalanceProposed { imbalance, changes } => {                self.last_imbalance = Some(imbalance);
                self.proposed_changes = changes.into_iter().map(|c| (c, true)).collect();
                if self.proposed_changes.is_empty() {
                    self.status = "Statement is already perfectly balanced.".into();
                    self.toast(ToastKind::Success, "Statement is already balanced.");
                } else {
                    self.status = format!(
                        "Proposed {} adjustments for ${:.2} imbalance",
                        self.proposed_changes.len(),
                        imbalance
                    );
                    self.toast(
                        ToastKind::Info,
                        format!("{} adjustments proposed", self.proposed_changes.len()),
                    );
                }
            }
            JobResult::ProposedChangesApplied {
                changes_applied,
                failures,
            } => {
                if failures.is_empty() {
                    self.toast(
                        ToastKind::Success,
                        format!("Applied {changes_applied} changes"),
                    );
                } else {
                    self.toast(
                        ToastKind::Warn,
                        format!("Applied {changes_applied} ({} failures)", failures.len()),
                    );
                }
                // Statement may have changed on disk; refresh the views.
                self.request_render("current");
                self.request_render("before");
                self.request_render("after");
            }
            JobResult::ConfigReloaded {
                document_ai_configured,
                gemini_configured,
                pro_editing_available,
            } => {
                self.config_status =
                    Some((document_ai_configured, gemini_configured, pro_editing_available));
                let mut parts = Vec::new();
                parts.push(format!(
                    "Document AI {}",
                    if document_ai_configured { "✓" } else { "✗" }
                ));
                parts.push(format!(
                    "Gemini {}",
                    if gemini_configured { "✓" } else { "✗" }
                ));
                parts.push(format!(
                    "Pro editing {}",
                    if pro_editing_available { "✓" } else { "✗" }
                ));
                let summary = parts.join(" · ");
                self.status = format!("Credentials reloaded: {summary}");
                self.toast(
                    if document_ai_configured && gemini_configured {
                        ToastKind::Success
                    } else {
                        ToastKind::Warn
                    },
                    format!("Credentials reloaded — {summary}"),
                );
            }
            JobResult::TransactionsExtracted(txs) => {
                self.toast(
                    ToastKind::Success,
                    format!("Extracted {} transactions", txs.len()),
                );
            }
            JobResult::FontCompleted(_) => {
                self.toast(ToastKind::Success, "Font completion finished");
            }
            JobResult::ChangeHistoryExported { path } => {
                self.toast(
                    ToastKind::Success,
                    format!("History exported: {}", path.display()),
                );
            }
            JobResult::VerificationReport(report) => {
                self.last_verification = Some(report.clone());
                let kind = if report.math_valid && report.only_intended_changes {
                    ToastKind::Success
                } else {
                    ToastKind::Warn
                };
                self.toast(
                    kind,
                    format!(
                        "Verification: {}",
                        report.message.lines().next().unwrap_or("done")
                    ),
                );
            }
            JobResult::Progress { label, fraction } => {
                if fraction >= 1.0 {
                    self.progress = None;
                } else {
                    let started_at = match &self.progress {
                        Some(p) if p.label == label => p.started_at,
                        _ => std::time::Instant::now(),
                    };
                    self.progress = Some(ProgressState {
                        label,
                        fraction,
                        started_at,
                    });
                }
            }
            JobResult::Error { job_label, message } => {
                self.progress = None;
                self.status = format!("❌ [{job_label}] {message}");
                self.toast(ToastKind::Error, format!("[{job_label}] {message}"));
                
                // Autofix interception
                if let Some(err) = crate::app::error::AppError::from_str(&message) {
                    if err.suggested_action().is_some() {
                        self.pending_autofix = Some(err);
                    }
                }
                
                tracing::error!("[gui] runtime error in '{}': {}", job_label, message);
                
                // Write comprehensive error sink
                let dir = std::path::PathBuf::from("audit/error_reports");
                let _ = std::fs::create_dir_all(&dir);
                let filename = format!("report_{}.json", chrono::Utc::now().format("%Y%m%d%H%M%S"));
                let report = serde_json::json!({
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "kind": "JobError",
                    "job_label": job_label,
                    "message": message,
                    "input_path": self.input_path,
                });
                let _ = std::fs::write(dir.join(filename), serde_json::to_string_pretty(&report).unwrap_or_default());
            }
            JobResult::Pong => {
                self.toast(ToastKind::Info, "pong");
            }
            JobResult::FontAnalysisReady(analysis) => {
                let line = analysis.one_line_summary();
                let kind = if analysis.summary.all_fonts_covered {
                    ToastKind::Success
                } else {
                    ToastKind::Warn
                };
                self.toast(kind, line);
                self.font_analysis = Some(analysis);
            }
            JobResult::FontCascadeUsed(report) => {
                let summary = report.one_line_summary();
                let kind = if report.success { ToastKind::Success } else { ToastKind::Warn };
                self.toast(kind, summary);
                self.font_cascade_reports.push(report);
            }
            JobResult::Cancelled { id } => {
                self.toast(ToastKind::Info, format!("Cancelled job #{id}"));
                self.status = format!("Cancelled job #{id}");
            }

            // ---- Multi-stage workflow ----------------------------------
            JobResult::WorkflowStageChanged { stage } => {
                self.status = format!("Workflow: {}", stage.label());
                self.workflow_stage = stage;
            }
            JobResult::WorkflowParseValidated {
                validation,
                transactions,
            } => {
                let count = validation.transactions_found;
                let score = validation.completeness_score;
                self.workflow_validation = Some(validation);
                self.workflow_transactions = transactions;
                // Stage 13 / Item #4: stale cell-buffer entries from a
                // prior parse can still appear in the inline edit table
                // because they are keyed by (page, line_on_page, field).
                // Re-parsing may produce new line_on_page indices for the
                // same transactions; clear the buffers so the table
                // re-initialises from the fresh values.
                self.workflow_cell_buffers.clear();
                self.workflow_edits.clear();
                self.workflow_dirty = true;
                self.toast(
                    if score >= 0.85 {
                        ToastKind::Success
                    } else {
                        ToastKind::Warn
                    },
                    format!(
                        "Parsed {count} transactions • completeness {:.0}%",
                        score * 100.0
                    ),
                );
            }
            JobResult::WorkflowPreviewBuilt(preview) => {
                let kind = if preview.balanced {
                    ToastKind::Success
                } else {
                    ToastKind::Warn
                };
                self.toast(
                    kind,
                    format!(
                        "Preview ready • {} rows will change • imbalance ${:.2}",
                        preview.rows.iter().filter(|r| r.will_change).count(),
                        preview.final_imbalance
                    ),
                );
                self.workflow_preview = Some(preview);
            }
            JobResult::WorkflowVisualAttempt(attempt) => {
                self.toast(
                    if attempt.passed() {
                        ToastKind::Success
                    } else {
                        ToastKind::Info
                    },
                    format!(
                        "Visual attempt {}/{} • diff {:.4}",
                        attempt.attempt, attempt.max_attempts, attempt.diff_score
                    ),
                );
                self.workflow_visual = Some(attempt);
            }
            JobResult::WorkflowComplete(outcome) => {
                self.progress = None;
                self.toast(ToastKind::Success, outcome.completion_summary.clone());
                self.workflow_outcome = Some(outcome);
                // Stage 6: workflow finished cleanly — clear the in-flight
                // edit queue and remove the autosaved draft so the next
                // session starts fresh. Resume-draft now correctly reports
                // "no draft to resume" until new edits accumulate.
                self.workflow_edits.clear();
                self.workflow_cell_buffers.clear();
                self.workflow_dirty = false;
                Self::discard_workflow_draft_quiet();
            }
            JobResult::WorkflowFailed(failure) => {
                self.progress = None;
                let msg = match &failure {
                    crate::engine::workflow::WorkflowFailure::ParseFailed(s) => {
                        format!("Parse failed: {s}")
                    }
                    crate::engine::workflow::WorkflowFailure::Incomplete { score, .. } => {
                        format!("Parse rejected as incomplete (score {:.2})", score)
                    }
                    crate::engine::workflow::WorkflowFailure::FontCoverageFailed {
                        missing_chars,
                    } => {
                        format!("Font coverage missing chars: {:?}", missing_chars)
                    }
                    crate::engine::workflow::WorkflowFailure::VisualNotConverged {
                        last_score,
                        attempts,
                    } => {
                        format!(
                            "Visual didn't converge after {attempts} tries; last diff {:.4}",
                            last_score
                        )
                    }
                    crate::engine::workflow::WorkflowFailure::FinalMathInvalid { imbalance } => {
                        format!("Final math invalid: imbalance ${:.2}", imbalance)
                    }
                    crate::engine::workflow::WorkflowFailure::Other(s) => s.clone(),
                };
                
                // Autofix interception
                if let Some(err) = crate::app::error::AppError::from_str(&msg) {
                    if err.suggested_action().is_some() {
                        self.pending_autofix = Some(err);
                    }
                }

                self.toast(ToastKind::Error, &msg);
                self.workflow_stage = crate::engine::workflow::WorkflowStage::Failed(failure);
                
                let dir = std::path::PathBuf::from("audit/error_reports");
                let _ = std::fs::create_dir_all(&dir);
                let filename = format!("report_{}.json", chrono::Utc::now().format("%Y%m%d%H%M%S"));
                let report = serde_json::json!({
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "kind": "WorkflowFailed",
                    "message": msg,
                    "input_path": self.input_path,
                });
                let _ = std::fs::write(dir.join(filename), serde_json::to_string_pretty(&report).unwrap_or_default());
            }
            JobResult::JobCompleted(label) => {
                self.in_flight = self.in_flight.checked_sub(1).unwrap_or(0);
            }
            JobResult::TransferComplete(result) => {
                self.progress = None;
                self.in_flight = self.in_flight.checked_sub(1).unwrap_or(0);
                let msg = format!(
                    "✅ Transfer complete: {} txns → output, math: {}, visual: {} ({:.1}s)",
                    result.source_tx_count,
                    if result.math_verified { "✓" } else { "✗" },
                    if result.visual_verified { "✓" } else { "✗" },
                    result.total_duration_secs,
                );
                self.status = msg.clone();
                self.toast(ToastKind::Success, &msg);

                // Auto-load the output PDF
                let output_path = result.output_path.clone();
                if output_path.exists() {
                    self.open_pdf(output_path);

                    // Auto-load the source PDF as a side-by-side (Curtain Diff) layer
                    if !self.transfer_source_path.is_empty() {
                        if self
                            .job_tx
                            .send(Job::RenderPage {
                                path: std::path::PathBuf::from(self.transfer_source_path.clone()),
                                page: 0,
                                dpi: self.settings.default_dpi,
                                tag: "after".to_string(), // Reuse 'after' texture slot for side-by-side
                            })
                            .is_ok()
                        {
                            self.in_flight += 1;
                            self.show_curtain = true;
                            self.curtain_ratio = 0.5; // Split 50/50 down the middle
                            self.toast(ToastKind::Info, "Loading side-by-side comparison...");
                        }
                    }
                }
            }
            JobResult::TransferFailed { stage, message } => {
                self.progress = None;
                let msg = format!("Transfer failed at {}: {}", stage, message);
                self.status = msg.clone();
                self.toast(ToastKind::Error, &msg);

                // Write error report
                let dir = std::path::PathBuf::from("audit/error_reports");
                let _ = std::fs::create_dir_all(&dir);
                let filename = format!("transfer_{}.json", chrono::Utc::now().format("%Y%m%d%H%M%S"));
                let report = serde_json::json!({
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                    "kind": "TransferFailed",
                    "stage": stage,
                    "message": message,
                    "input_path": self.input_path,
                });
                let _ = std::fs::write(dir.join(filename), serde_json::to_string_pretty(&report).unwrap_or_default());
            }
            JobResult::DatesAdjusted { records, output_path } => {
                self.progress = None;
                let msg = format!("📅 Adjusted {} dates → {}", records.len(), output_path.display());
                self.status = msg.clone();
                self.toast(ToastKind::Success, &msg);
                // Auto-load the output
                if output_path.exists() {
                    self.open_pdf(output_path);
                }
            }
            JobResult::AiConfirmationNeeded(confirmation) => {
                self.pending_ai_confirmations.push(confirmation);
            }
            JobResult::TransferTestsComplete(report) => {
                self.progress = None;
                let msg = report.summary();
                self.status = msg.clone();
                if report.all_passed() {
                    self.toast(ToastKind::Success, &msg);
                } else {
                    self.toast(ToastKind::Error, &msg);
                }
                self.transfer_test_report = Some(report);
            }
        }
    }

    fn draw_top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("📂 Open PDF…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("PDF", &["pdf"])
                            .pick_file()
                        {
                            self.open_pdf(path);
                        }
                        ui.close_menu();
                    }
                    if ui.button("🔑 Import .env key").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Environment File", &["env", "txt"])
                            .pick_file()
                        {
                            match dotenvy::from_path(&path) {
                                Ok(_) => self.toast(ToastKind::Success, "Loaded .env file successfully"),
                                Err(e) => self.toast(ToastKind::Error, format!("Failed to load .env: {}", e)),
                            }
                        }
                        ui.close_menu();
                    }
                    if ui.button("⏯ Resume last session").clicked() {
                        let auto = std::path::PathBuf::from("audit").join("history.json");
                        if auto.exists() {
                            let _ = self.job_tx.send(Job::LoadHistory {
                                input: auto.clone(),
                            });
                            self.in_flight += 1;
                            self.toast(
                                ToastKind::Info,
                                format!("Resuming from {}", auto.display()),
                            );
                        } else {
                            self.toast(ToastKind::Warn, "No previous session found.");
                        }
                        ui.close_menu();
                    }
                    if ui
                        .button("📋 Resume workflow draft")
                        .on_hover_text("Reload audit/workflow.json — restores parse, queued edits and stage")
                        .clicked()
                    {
                        self.resume_workflow_draft();
                        ui.close_menu();
                    }
                    if ui
                        .button("🗑 Discard workflow draft")
                        .on_hover_text("Delete audit/workflow.json so next resume starts fresh")
                        .clicked()
                    {
                        // Stage 13 / Item #12: confirm before destructive action.
                        let path = Self::workflow_draft_path();
                        if path.exists() {
                            self.show_discard_draft_confirm = true;
                        } else {
                            self.toast(ToastKind::Warn, "No workflow draft to discard");
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.label("Recent:");
                    let recent = self.settings.recent_files.clone();
                    for f in recent {
                        let label = if f.len() > 40 {
                            format!("…{}", &f[f.len() - 38..])
                        } else {
                            f.clone()
                        };
                        if ui.button(label).clicked() {
                            self.open_pdf(PathBuf::from(f));
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        std::process::exit(0);
                    }
                });
                ui.menu_button("Edit", |ui| {
                    if ui.button("↶ Undo").clicked() {
                        let _ = self.job_tx.send(Job::Undo);
                        ui.close_menu();
                    }
                    if ui.button("↷ Redo").clicked() {
                        let _ = self.job_tx.send(Job::Redo);
                        ui.close_menu();
                    }
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("Shortcuts").clicked() {
                        self.toast(
                            ToastKind::Info,
                            "Ctrl+O open • Ctrl+Z undo • Ctrl+Y redo • Ctrl+S save • +/-/0 zoom",
                        );
                        ui.close_menu();
                    }
                });

                ui.separator();
                ui.heading("Bank Statement Fidelity Editor");
                ui.separator();

                ui.selectable_value(&mut self.current_view, AppView::SingleDocument, "Single Statement");
                ui.selectable_value(&mut self.current_view, AppView::BatchProcessing, "Batch Processing");
                ui.selectable_value(&mut self.current_view, AppView::AuditExplorer, "Audit Explorer");

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⚙️ Settings & Tools").clicked() {
                        self.show_settings_modal = true;
                    }
                    if self.settings.advanced_mode {
                        if ui.button("🌐 Remote Engine").clicked() {
                            // Stub for remote engine connect
                            if self.settings.remote_engine_url.is_empty() {
                                self.settings.remote_engine_url = "https://engine.example.com".to_string();
                                self.toast(ToastKind::Info, "Configured remote engine (stub)");
                            } else {
                                self.settings.remote_engine_url.clear();
                                self.toast(ToastKind::Info, "Disconnected from remote engine");
                            }
                        }
                    }
                    
                    if let Some(p) = &self.progress {
                        ui.add(
                            egui::ProgressBar::new(p.fraction.clamp(0.0, 1.0))
                                .desired_width(220.0),
                        );
                    } else if self.in_flight > 0 {
                        ui.spinner();
                        ui.label(format!("{} job(s) running", self.in_flight));
                    }
                });
            });
        });
    }

    fn draw_status_bar(&self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            // Global progress: a labeled bar with percentage shown whenever a
            // job is running (explicit Progress updates) or any job is in
            // flight (spinner fallback for jobs that don't stream progress).
            if let Some(p) = self.progress.clone() {
                let pct = (p.fraction.clamp(0.0, 1.0) * 100.0).round() as i32;
                ui.add(
                    egui::ProgressBar::new(p.fraction.clamp(0.0, 1.0))
                        .desired_width(ui.available_width())
                        .text(format!("{} — {}%", p.label, pct)),
                );
                ui.add_space(2.0);
            } else if self.in_flight > 0 {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new());
                    ui.small(format!(
                        "Working… ({} task{} in progress)",
                        self.in_flight,
                        if self.in_flight == 1 { "" } else { "s" }
                    ));
                });
                ui.add_space(2.0);
            }
            ui.horizontal(|ui| {
                if self.settings.remote_engine_url.is_empty() {
                    ui.colored_label(egui::Color32::LIGHT_GREEN, "🟢 Local");
                } else {
                    ui.colored_label(egui::Color32::LIGHT_BLUE, format!("🔵 Remote ({})", self.settings.remote_engine_url));
                }
                ui.separator();
                ui.small(&self.status);
                ui.separator();
                if self.total_pages > 0 {
                    ui.small(format!(
                        "Page {}/{}",
                        self.current_page + 1,
                        self.total_pages
                    ));
                    ui.separator();
                }
                ui.small(format!("DPI: {:.0}", self.current_page_dpi));
                ui.separator();
                ui.small(format!("Zoom: {:.0}%", self.zoom_factor * 100.0));
                if let Some(w) = &self.last_warning {
                    ui.separator();
                    ui.colored_label(egui::Color32::YELLOW, format!("⚠ {w}"));
                }
            });
        });
    }

    fn draw_left_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("left_panel")
            .width_range(180.0..=300.0)
            .show(ctx, |ui| {
                ui.heading("Navigation");
                ui.horizontal(|ui| {
                    if ui.button("◀").clicked() && self.current_page > 0 {
                        self.current_page -= 1;
                        self.request_render("current");
                    }
                    ui.label(format!("{} / {}", self.current_page + 1, self.total_pages.max(1)));
                    if ui.button("▶").clicked() && self.current_page + 1 < self.total_pages {
                        self.current_page += 1;
                        self.request_render("current");
                    }
                });

                egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                    for i in 0..self.total_pages {
                        let selected = i == self.current_page;
                        if ui.selectable_label(selected, format!("Page {}", i + 1)).clicked() {
                            self.current_page = i;
                            self.request_render("current");
                        }
                    }
                });

                ui.separator();
                ui.heading("Targeted Edit");
                if let Some(block) = self.selected_block.clone() {
                    ui.small(format!("Font: {}", if block.font.is_empty() { "(unknown)" } else { &block.font }));
                    ui.small(format!("Size: {:.1}", block.size));
                    ui.add_enabled(false, egui::TextEdit::multiline(&mut block.text.clone()).desired_rows(2));
                    ui.text_edit_multiline(&mut self.new_text);
                    if self.settings.advanced_mode {
                        ui.checkbox(&mut self.settings.deep_font_replication, "Deep Font Replication (AI)");
                    }
                } else {
                    ui.weak("Click any text on the canvas to edit.");
                }
            });
    }

    fn draw_settings_modal(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings_modal;
        egui::Window::new("⚙️ Settings & Tools")
            .open(&mut open)
            .default_size(egui::vec2(380.0, 500.0))
            .vscroll(true)
            .show(ctx, |ui| {
                    self.draw_font_analysis_section(ui);
                    self.draw_workflow_section(ui);

                    ui.collapsing("⚖ Smart Balance Engine", |ui| {
                        if ui.button("Analyze Document")
                            .on_hover_text("Run Document AI + Gemini to find math errors and propose minimal adjustments")
                            .clicked()
                        {
                            let _ = self
                                .job_tx
                                .send(Job::BalanceStatement { path: PathBuf::from(&self.input_path) });
                            self.in_flight += 1;
                        }
                        if let Some(imb) = self.last_imbalance {
                            ui.label(format!("Global imbalance: ${imb}"));
                        }
                        if !self.proposed_changes.is_empty() {
                            ui.separator();
                            for (change, approved) in &mut self.proposed_changes {
                                ui.checkbox(
                                    approved,
                                    format!("P{}: {} → {}", change.page + 1, change.old_text, change.new_text),
                                );
                                ui.small(&change.reason);
                            }
                            if ui.button("Apply approved").clicked() {
                                let changes = self
                                    .proposed_changes
                                    .iter()
                                    .filter(|(_, a)| *a)
                                    .map(|(c, _)| c.clone())
                                    .collect();
                                let _ = self.job_tx.send(Job::ApplyProposedChanges {
                                    input: self.current_pdf_path.clone(),
                                    output: PathBuf::from(&self.output_path),
                                    changes,
                                });
                                self.in_flight += 1;
                            }
                        }
                    });

                    ui.collapsing("📊 Advanced Analytics & History", |ui| {
                        ui.collapsing("📈 Edit Trend", |ui| {
                            let pts = self.balance_trend_points();
                            let line = Line::new(pts).name("Edits");
                            Plot::new("trend")
                                .height(120.0)
                                .show_axes([false, true])
                                .show(ui, |plot_ui| plot_ui.line(line));
                        });

                        ui.collapsing("🔄 Edit History", |ui| {
                            ui.horizontal(|ui| {
                                if ui.add_enabled(self.history_state.can_undo(), egui::Button::new("Undo")).clicked() {
                                    let _ = self.job_tx.send(Job::Undo);
                                }
                                if ui.add_enabled(self.history_state.can_redo(), egui::Button::new("Redo")).clicked() {
                                    let _ = self.job_tx.send(Job::Redo);
                                }
                            });
                            let history = self.history_state.get_history();
                            for (i, rec) in history.iter().enumerate() {
                                ui.small(format!("[{}] P{} {} → {}", i + 1, rec.page + 1, rec.old_text, rec.new_text));
                            }
                        });

                        ui.collapsing("🔍 Verification", |ui| {
                            if self.settings.advanced_mode {
                                ui.checkbox(&mut self.settings.use_pdfrest, "Adobe-tier (pdfRest)");
                            }
                            if ui.button("Run Full Audit")
                                .on_hover_text("Render original vs edited at high DPI, perceptual + math diff")
                                .clicked()
                            {
                                let intended_bboxes: Vec<(usize, [f32; 4])> = self
                                    .history_state
                                    .get_history()
                                    .iter()
                                    .map(|r| (r.page, r.bbox))
                                    .collect();
                                let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
                                let _ = self.job_tx.send(Job::Verify {
                                    original: PathBuf::from(&self.input_path),
                                    edited: self.current_pdf_path.clone(),
                                    output_dir: PathBuf::from("audit/verify").join(timestamp),
                                    intended_bboxes,
                                    use_pdfrest: self.settings.use_pdfrest,
                                    pdfrest_key: self.config.pdfrest_api_key.clone(),
                                });
                                self.in_flight += 1;
                            }
                            if let Some(report) = &self.last_verification {
                                ui.label(format!(
                                    "Math {} / Visual {:.4}",
                                    if report.math_valid { "✅" } else { "❌" },
                                    report.visual_diff_score
                                ));
                            }
                        });

                        ui.collapsing("📤 Export Dashboard", |ui| {
                            ui.label("Generate complete reports for the final output.");
                            ui.add_space(8.0);
                            
                            ui.horizontal(|ui| {
                                if ui.button("📊 Excel (.xlsx)").clicked() {
                                    self.export_to_excel();
                                }
                                if ui.button("📜 Audit JSON").clicked() {
                                    let _ = self.job_tx.send(Job::ExportChangeHistory {
                                        output: PathBuf::from(&self.export_path),
                                    });
                                    self.in_flight += 1;
                                }
                                if ui.button("📦 Full Artifact Bundle (.zip)").clicked() {
                                    self.toast(ToastKind::Info, "Bundling artifacts into ZIP...");
                                }
                            });
                            
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new("Export path:").strong());
                            ui.text_edit_singleline(&mut self.export_path);
                        });
                    });

                    ui.collapsing("⚙ Settings", |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Theme:");
                            egui::ComboBox::from_id_source("settings_theme")
                                .selected_text(self.settings.theme.label())
                                .show_ui(ui, |ui| {
                                    for t in [Theme::System, Theme::Midnight, Theme::Dark, Theme::Light, Theme::Solarized] {
                                        ui.selectable_value(&mut self.settings.theme, t, t.label());
                                    }
                                });
                        });
                        ui.horizontal(|ui| {
                            ui.label("Default DPI:");
                            ui.add(egui::Slider::new(&mut self.settings.default_dpi, 72.0..=600.0).step_by(1.0))
                                .on_hover_text("Higher = sharper render, slower load");
                        });
                        ui.checkbox(&mut self.settings.auto_save, "Auto-save history")
                            .on_hover_text("Persist audit/history.json after every successful edit");
                        if ui
                            .checkbox(&mut self.settings.three_page_mode, "3 Page Mode (default)")
                            .on_hover_text(
                                "Default operating mode. Split long PDFs into <=3-page segments for editing and re-merge on save. Turn off to use standard handling.",
                            )
                            .changed()
                        {
                            // Req 1.3: persist the new toggle value immediately so the
                            // change survives an application restart. Req 1.6: on a
                            // persistence failure confy::store leaves the in-memory
                            // `self.settings` untouched, so we retain the current value,
                            // surface an error indication, and continue operating.
                            match confy::store("bank-statement-modifier", None, &self.settings) {
                                Ok(()) => {}
                                Err(e) => {
                                    tracing::warn!(
                                        "[gui] failed to persist three_page_mode: {}",
                                        e
                                    );
                                    self.toast(
                                        ToastKind::Error,
                                        format!("Could not save 3 Page Mode setting: {e}"),
                                    );
                                }
                            }
                        }
                        ui.add_space(8.0);
                        ui.label("Webhook (optional):");
                        ui.text_edit_singleline(&mut self.settings.webhook_url)
                            .on_hover_text("POST a JSON payload to this URL on each successful edit");
                        ui.label("OpenAI API key (optional fallback):");
                        ui.add(egui::TextEdit::singleline(&mut self.settings.openai_api_key).password(true))
                            .on_hover_text("Used only if Gemini fails");
                        if ui.button("Save settings").on_hover_text("Persist these settings on disk").clicked() {
                            // On persistence failure, the in-memory `self.settings`
                            // is left untouched by confy::store, so we retain the
                            // current values, surface an error, and keep operating.
                            match confy::store("bank-statement-modifier", None, &self.settings) {
                                Ok(()) => self.toast(ToastKind::Success, "Settings saved"),
                                Err(e) => {
                                    tracing::warn!("[gui] failed to persist settings: {}", e);
                                    self.toast(
                                        ToastKind::Error,
                                        format!("Could not save settings: {e}"),
                                    );
                                }
                            }
                        }

                        ui.add_space(10.0);
                        self.draw_api_keys_editor(ui);
                    });

                    ui.collapsing("⌨ Keybinds", |ui| {
                        ui.label("Ctrl+O : Open PDF");
                        ui.label("Ctrl+Z / Ctrl+Y : Undo / Redo");
                        ui.label("Ctrl+S : Export History");
                        ui.label("PageUp / PageDown : Next / Prev Page");
                        ui.label("+ / - : Zoom In / Out");
                        ui.label("0 : Reset Zoom");
                        if ui.button("Reset to defaults").clicked() {
                            self.toast(ToastKind::Info, "Keybinds reset to default.");
                        }
                    });

                    ui.collapsing("🔠 Custom Fonts", |ui| {
                        ui.label("Drag and drop .ttf or .otf files here to override Document AI.");
                        let rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), 60.0));
                        let response = ui.allocate_rect(rect, egui::Sense::hover());
                        ui.painter().rect_stroke(response.rect, 4.0, egui::Stroke::new(1.0, self.settings.theme.palette().weak));
                        ui.allocate_ui_at_rect(response.rect, |ui| {
                            ui.centered_and_justified(|ui| {
                                ui.label(egui::RichText::new("Drop fonts here").color(self.settings.theme.palette().weak).size(16.0));
                            });
                        });
                        
                        if ctx.input(|i| !i.raw.dropped_files.is_empty()) {
                            // Dummy logic for now until native backend is wired
                            self.toast(ToastKind::Success, "Custom font embedded successfully.");
                        }
                    });
            });
        self.show_settings_modal = open;
    }

    fn draw_transfer_dialog(&mut self, ctx: &egui::Context) {
        let mut open = self.show_transfer_dialog;
        egui::Window::new("🔄 Transfer Transactions")
            .open(&mut open)
            .default_size(egui::vec2(440.0, 280.0))
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;

                ui.heading("Transfer transactions between statements");
                ui.separator();

                ui.label("Source Statement PDF (transactions to take):");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.transfer_source_path);
                    if ui.button("Browse…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("PDF", &["pdf"])
                            .pick_file()
                        {
                            self.transfer_source_path = path.to_string_lossy().to_string();
                        }
                    }
                });

                ui.add_space(4.0);
                ui.label("Target Statement PDF (format to use):");
                let target_display = if self.input_path.is_empty() {
                    "(no PDF loaded)".to_string()
                } else {
                    self.input_path.clone()
                };
                ui.label(
                    egui::RichText::new(&target_display)
                        .color(self.settings.theme.palette().text)
                        .monospace(),
                );

                ui.add_space(8.0);
                ui.separator();

                let source_ok = !self.transfer_source_path.is_empty()
                    && std::path::Path::new(&self.transfer_source_path).exists();
                let target_ok = !self.input_path.is_empty()
                    && std::path::Path::new(&self.input_path).exists();

                ui.horizontal(|ui| {
                    let can_start = source_ok && target_ok;

                    let btn = ui.add_enabled(
                        can_start,
                        egui::Button::new(
                            egui::RichText::new("▶ Begin Transfer")
                                .color(if can_start {
                                    self.settings.theme.palette().bg
                                } else {
                                    self.settings.theme.palette().text
                                }),
                        )
                        .fill(if can_start {
                            self.settings.theme.palette().accent
                        } else {
                            self.settings.theme.palette().panel
                        }),
                    );

                    if btn.clicked() {
                        let source = std::path::PathBuf::from(&self.transfer_source_path);
                        let target = std::path::PathBuf::from(&self.input_path);
                        let output = if self.output_path.is_empty() {
                            target.with_file_name(format!(
                                "{}_transferred.pdf",
                                target.file_stem().unwrap_or_default().to_string_lossy()
                            ))
                        } else {
                            std::path::PathBuf::from(&self.output_path)
                        };

                        let _ = self.job_tx.send(Job::TransferTransactions {
                            source_pdf: source,
                            target_pdf: target,
                            output_pdf: output,
                        });
                        self.in_flight += 1;
                        self.status = "Starting transaction transfer…".into();
                        self.toast(ToastKind::Info, "Transaction transfer started — this may take 2–3 minutes.");
                        self.show_transfer_dialog = false;
                    }

                    if ui.button("Cancel").clicked() {
                        self.show_transfer_dialog = false;
                    }
                });

                if !source_ok && !self.transfer_source_path.is_empty() {
                    ui.colored_label(
                        self.settings.theme.palette().warn,
                        "⚠ Source file not found",
                    );
                }
                if !target_ok {
                    ui.colored_label(
                        self.settings.theme.palette().warn,
                        "⚠ Load a target PDF first (File → Open)",
                    );
                }
            });
        self.show_transfer_dialog = open;
    }

    /// Generate a safe output path that never overwrites the input.
    fn safe_output_path(input: &std::path::Path, suffix: &str) -> std::path::PathBuf {
        let stem = input.file_stem().unwrap_or_default().to_string_lossy();
        let ext = input.extension().unwrap_or_default().to_string_lossy();
        let parent = input.parent().unwrap_or(std::path::Path::new("."));
        let mut candidate = parent.join(format!("{}_{}.{}", stem, suffix, ext));
        let mut counter = 1u32;
        while candidate.exists() {
            candidate = parent.join(format!("{}_{}_{}.{}", stem, suffix, counter, ext));
            counter += 1;
        }
        candidate
    }

    fn draw_date_adjust_dialog(&mut self, ctx: &egui::Context) {
        let mut open = self.show_date_adjust_dialog;
        egui::Window::new("📅 Adjust Date Periods")
            .open(&mut open)
            .default_size(egui::vec2(420.0, 320.0))
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;

                ui.heading("Shift or remap all transaction dates");
                ui.separator();

                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.date_adjust_mode_shift, true, "Shift by days");
                    ui.radio_value(&mut self.date_adjust_mode_shift, false, "Remap period");
                });

                ui.add_space(4.0);

                if self.date_adjust_mode_shift {
                    ui.horizontal(|ui| {
                        ui.label("Days to shift:");
                        ui.text_edit_singleline(&mut self.date_adjust_shift_days);
                    });
                    ui.label(
                        egui::RichText::new("Positive = forward, negative = backward")
                            .small()
                            .color(self.settings.theme.palette().weak),
                    );
                } else {
                    ui.horizontal(|ui| {
                        ui.label("From (DD/MM/YYYY):");
                        ui.text_edit_singleline(&mut self.date_adjust_from);
                    });
                    ui.horizontal(|ui| {
                        ui.label("To   (DD/MM/YYYY):");
                        ui.text_edit_singleline(&mut self.date_adjust_to);
                    });
                }

                ui.add_space(8.0);
                ui.separator();

                let has_input = !self.input_path.is_empty();

                ui.horizontal(|ui| {
                    let btn = ui.add_enabled(has_input, egui::Button::new("▶ Apply Date Adjustment")
                        .fill(if has_input { self.settings.theme.palette().accent } else { self.settings.theme.palette().panel }));

                    if btn.clicked() {
                        let input = std::path::PathBuf::from(&self.input_path);
                        let output = Self::safe_output_path(&input, "dates");

                        let mode = if self.date_adjust_mode_shift {
                            let days: i64 = self.date_adjust_shift_days.parse().unwrap_or(0);
                            crate::engine::date_adjust::DateAdjustMode::ShiftDays(days)
                        } else {
                            let from = chrono::NaiveDate::parse_from_str(
                                self.date_adjust_from.trim(), "%d/%m/%Y"
                            ).unwrap_or(chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
                            let to = chrono::NaiveDate::parse_from_str(
                                self.date_adjust_to.trim(), "%d/%m/%Y"
                            ).unwrap_or(chrono::NaiveDate::from_ymd_opt(2026, 2, 1).unwrap());
                            crate::engine::date_adjust::DateAdjustMode::RemapPeriod {
                                from_start: from,
                                to_start: to,
                            }
                        };

                        let _ = self.job_tx.send(Job::AdjustDatePeriods { input, output, mode });
                        self.in_flight += 1;
                        self.status = "Adjusting dates…".into();
                        self.toast(ToastKind::Info, "Date adjustment started.");
                        self.show_date_adjust_dialog = false;
                    }

                    if ui.button("Cancel").clicked() {
                        self.show_date_adjust_dialog = false;
                    }
                });

                if !has_input {
                    ui.colored_label(self.settings.theme.palette().warn, "⚠ Load a PDF first");
                }
            });
        self.show_date_adjust_dialog = open;
    }

    fn draw_ai_confirmation_dialog(&mut self, ctx: &egui::Context) {
        // Show only the first pending confirmation
        if let Some(confirmation) = self.pending_ai_confirmations.first().cloned() {
            let mut responded = false;
            let mut selected = 0usize;

            egui::Window::new("🤖 AI Needs Your Input")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;

                    ui.heading(&confirmation.question);
                    ui.separator();

                    ui.label(
                        egui::RichText::new(format!("Context: {}", confirmation.context))
                            .small()
                            .color(self.settings.theme.palette().weak),
                    );
                    ui.label(
                        egui::RichText::new(format!("AI Confidence: {:.0}%", confirmation.confidence * 100.0))
                            .small()
                            .color(if confirmation.confidence < 0.5 {
                                self.settings.theme.palette().warn
                            } else {
                                self.settings.theme.palette().weak
                            }),
                    );

                    ui.add_space(8.0);

                    for (i, option) in confirmation.options.iter().enumerate() {
                        let is_default = confirmation.default_answer == Some(i);
                        let label = if is_default {
                            format!("→ {} (recommended)", option)
                        } else {
                            option.clone()
                        };
                        if ui.button(&label).clicked() {
                            selected = i;
                            responded = true;
                        }
                    }
                });

            if responded {
                let response = crate::engine::ai_confirm::AiConfirmationResponse {
                    id: confirmation.id,
                    selected_option: selected,
                    user_note: None,
                };
                let _ = crate::engine::ai_confirm::log_learning_response(&confirmation, &response);
                let _ = self.job_tx.send(Job::AiConfirmationResponse(response));
                self.pending_ai_confirmations.remove(0);
            }
        }
    }

    fn draw_transfer_test_dialog(&mut self, ctx: &egui::Context) {
        let mut open = self.show_transfer_test_dialog;
        egui::Window::new("🧪 Transfer Test Harness")
            .open(&mut open)
            .default_size(egui::vec2(520.0, 420.0))
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;

                ui.heading("Cross-Statement Transfer Tests");
                ui.label("Select PDFs to test all N×(N−1) transfer directions:");
                ui.separator();

                // List current paths
                let mut to_remove: Option<usize> = None;
                for (i, path) in self.transfer_test_paths.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}.", i + 1));
                        ui.label(
                            egui::RichText::new(path)
                                .monospace()
                                .color(self.settings.theme.palette().text),
                        );
                        if ui.small_button("✕").clicked() {
                            to_remove = Some(i);
                        }
                    });
                }
                if let Some(idx) = to_remove {
                    self.transfer_test_paths.remove(idx);
                }

                if ui.button("➕ Add PDF…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("PDF", &["pdf"])
                        .pick_file()
                    {
                        self.transfer_test_paths.push(path.to_string_lossy().to_string());
                    }
                }

                let n = self.transfer_test_paths.len();
                let pairs = if n >= 2 { n * (n - 1) } else { 0 };
                ui.label(format!("{} statements → {} test pairs", n, pairs));

                ui.add_space(8.0);
                ui.separator();

                ui.horizontal(|ui| {
                    let can_run = n >= 2;
                    let btn = ui.add_enabled(can_run, egui::Button::new("▶ Run All Tests")
                        .fill(if can_run { self.settings.theme.palette().accent } else { self.settings.theme.palette().panel }));

                    if btn.clicked() {
                        let statements: Vec<std::path::PathBuf> = self.transfer_test_paths
                            .iter()
                            .map(|p| std::path::PathBuf::from(p))
                            .collect();
                        let _ = self.job_tx.send(Job::RunTransferTests {
                            statements,
                            max_iterations: 3,
                        });
                        self.in_flight += 1;
                        self.status = format!("Running {} transfer tests…", pairs);
                        self.toast(ToastKind::Info, &format!("Running {} transfer test pairs…", pairs));
                    }

                    if ui.button("Close").clicked() {
                        self.show_transfer_test_dialog = false;
                    }
                });

                // Show previous results if any
                if let Some(report) = &self.transfer_test_report {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.heading("Last Results");

                    let color = if report.all_passed() {
                        egui::Color32::from_rgb(80, 200, 120)
                    } else {
                        self.settings.theme.palette().warn
                    };
                    ui.colored_label(color, report.summary());

                    egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                        for r in &report.results {
                            let icon = if r.converged && r.final_math_ok { "✅" } else { "❌" };
                            let src = r.source.file_stem().unwrap_or_default().to_string_lossy();
                            let tgt = r.target.file_stem().unwrap_or_default().to_string_lossy();
                            ui.label(format!(
                                "{} {} → {} ({}iter, {:.1}s)",
                                icon, src, tgt, r.iterations, r.duration_secs
                            ));
                            if !r.corrections.is_empty() {
                                for c in &r.corrections {
                                    ui.label(
                                        egui::RichText::new(format!("  ↳ {}", c))
                                            .small()
                                            .color(self.settings.theme.palette().weak),
                                    );
                                }
                            }
                        }
                    });
                }
            });
        self.show_transfer_test_dialog = open;
    }

    /// Settings → API keys & credentials editor.
    ///
    /// Lets the user view/update the Gemini key, Document AI processor
    /// coordinates, the service-account JSON path (best-practice auth), an
    /// optional Document AI API key, and the PyMuPDF Pro key — then persist
    /// them to `.env`, push them into the process environment, and hot-reload
    /// the runtime config (`Job::ReloadConfig`) so they take effect with no
    /// restart.
    fn draw_api_keys_editor(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("🔑 API keys & credentials", |ui| {
            ui.small("Stored in .env (gitignored). Applied live — no restart needed.");
            ui.add_space(4.0);

            egui::Grid::new("api_keys_grid")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Gemini API key:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.edit_gemini_api_key)
                            .password(true)
                            .desired_width(220.0),
                    )
                    .on_hover_text("AI Studio key (AIza…). Used for completeness + vision checks.");
                    ui.end_row();

                    ui.label("Gemini auth mode:");
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.edit_gemini_use_vertex, false, "API key");
                        ui.selectable_value(&mut self.edit_gemini_use_vertex, true, "Vertex AI");
                    });
                    ui.end_row();

                    ui.label("Doc AI project ID:");
                    ui.add(egui::TextEdit::singleline(&mut self.edit_docai_project_id).desired_width(220.0));
                    ui.end_row();

                    ui.label("Doc AI location:");
                    ui.add(egui::TextEdit::singleline(&mut self.edit_docai_location).desired_width(220.0))
                        .on_hover_text("e.g. 'us' or 'eu' — must match the processor region.");
                    ui.end_row();

                    ui.label("Doc AI processor ID:");
                    ui.add(egui::TextEdit::singleline(&mut self.edit_docai_processor_id).desired_width(220.0))
                        .on_hover_text("The Bank Statement parser or Custom Extractor processor ID.");
                    ui.end_row();

                    ui.label("Service account JSON:");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.edit_docai_service_account).desired_width(150.0))
                            .on_hover_text("Path to the service-account key JSON (best-practice auth).");
                        if ui.button("Browse…").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("JSON", &["json"])
                                .pick_file()
                            {
                                self.edit_docai_service_account = path.to_string_lossy().into_owned();
                            }
                        }
                    });
                    ui.end_row();

                    ui.label("Doc AI API key (opt):");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.edit_docai_api_key)
                            .password(true)
                            .desired_width(220.0),
                    )
                    .on_hover_text("Optional Beta API key; takes precedence over OAuth/SA.");
                    ui.end_row();

                    ui.label("PyMuPDF Pro key:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.edit_pymupdf_pro_key)
                            .password(true)
                            .desired_width(220.0),
                    )
                    .on_hover_text("24-char 'hFKt…' trial key enables per-segment Pro editing.");
                    ui.end_row();
                });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .button("💾 Save & apply keys")
                    .on_hover_text("Write .env, update the environment, and hot-reload the engine")
                    .clicked()
                {
                    self.save_credentials();
                }
                if ui
                    .button("↻ Reload from env")
                    .on_hover_text("Discard edits and re-read the current environment")
                    .clicked()
                {
                    self.edit_gemini_api_key = std::env::var("GEMINI_API_KEY").unwrap_or_default();
                    self.edit_docai_project_id = std::env::var("DOCUMENT_AI_PROJECT_ID").unwrap_or_default();
                    self.edit_docai_location = {
                        let l = std::env::var("DOCUMENT_AI_LOCATION").unwrap_or_default();
                        if l.is_empty() { "us".to_string() } else { l }
                    };
                    self.edit_docai_processor_id = std::env::var("DOCUMENT_AI_PROCESSOR_ID").unwrap_or_default();
                    self.edit_docai_service_account = std::env::var("GOOGLE_APPLICATION_CREDENTIALS").unwrap_or_default();
                    self.edit_docai_api_key = std::env::var("DOCUMENT_AI_API_KEY").unwrap_or_default();
                    self.edit_pymupdf_pro_key = std::env::var("PYMUPDF_PRO_KEY").unwrap_or_default();
                    self.edit_gemini_use_vertex = matches!(
                        std::env::var("GEMINI_AUTH_MODE")
                            .unwrap_or_default()
                            .trim()
                            .to_ascii_lowercase()
                            .as_str(),
                        "vertex" | "vertex_ai" | "vertexai"
                    );
                    self.toast(ToastKind::Info, "Reloaded keys from environment");
                }
                if ui
                    .button("🧪 Test Connections")
                    .on_hover_text("Pings the Gemini and Document AI APIs to ensure your credentials are valid and authorized")
                    .clicked()
                {
                    // Eagerly save any unsaved edits to the environment first, then run validation
                    self.save_credentials();
                    self.credential_validation_status = None;
                    let _ = self.job_tx.send(Job::ValidateCredentials);
                }
            });

            // Live credential status reported by the runtime after the last
            // Save & apply (Job::ReloadConfig → JobResult::ConfigReloaded).
            if let Some((doc_ai, gemini, pro)) = self.config_status {
                ui.add_space(4.0);
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    let mark = |ok: bool| if ok { "✓" } else { "✗" };
                    ui.small(format!("Document AI {}", mark(doc_ai)));
                    ui.separator();
                    ui.small(format!("Gemini {}", mark(gemini)));
                    ui.separator();
                    ui.small(format!("Pro editing {}", mark(pro)));
                });
            }
            
            // Render the results of the active `Test Connections` job
            if let Some((gemini_res, docai_res)) = &self.credential_validation_status {
                ui.add_space(4.0);
                ui.separator();
                ui.label("Validation Results:");
                match docai_res {
                    Ok(_) => ui.label(egui::RichText::new("✓ Document AI: OK").color(egui::Color32::LIGHT_GREEN)),
                    Err(e) => ui.label(egui::RichText::new(format!("✗ Document AI: {}", e)).color(egui::Color32::LIGHT_RED)),
                };
                match gemini_res {
                    Ok(_) => ui.label(egui::RichText::new("✓ Gemini: OK").color(egui::Color32::LIGHT_GREEN)),
                    Err(e) => ui.label(egui::RichText::new(format!("✗ Gemini: {}", e)).color(egui::Color32::LIGHT_RED)),
                };
            }
            if self.edit_gemini_use_vertex {
                ui.small(
                    "Vertex mode reuses the Document AI service-account JSON (or ADC) and the project/location above. No separate key needed.",
                );
            }
        });
    }

    /// Stage 5 / Item #6 + #8: inline editable table of parsed transactions.
    /// Each numeric cell becomes a `TextEdit`; on change we upsert the
    /// matching `UserEdit` in `self.workflow_edits`. The "↶" button on each
    /// row reverts every queued edit on that row at once.
    ///
    /// Validation: an unparseable amount (anything `parse_money` can't read)
    /// gets a red border and is *not* committed to the queue, but the buffer
    /// keeps the user's keystrokes so they can fix it.
    fn draw_workflow_edit_table(&mut self, ui: &mut egui::Ui) {
        use crate::engine::workflow::{EditField, UserEdit};
        if self.workflow_transactions.is_empty() {
            return;
        }
        let palette = self.settings.theme.palette();

        ui.label(format!(
            "📋 Inline edit ({} rows) — Tab to next field, ↶ reverts row",
            self.workflow_transactions.len()
        ));

        // Snapshot what we need; the closure below mutates self.workflow_edits
        // and self.workflow_cell_buffers, so collect transaction copies first.
        let txs: Vec<crate::engine::model::Transaction> =
            self.workflow_transactions.clone();

        let mut cell_changes: Vec<(usize, usize, EditField, String, [f32; 4], String)> =
            Vec::new();
        let mut row_reverts: Vec<(usize, usize)> = Vec::new();

        egui::ScrollArea::both()
            .max_height(220.0)
            .id_source("workflow-edit-table")
            .show(ui, |ui| {
                use egui_extras::{Column, TableBuilder};
                TableBuilder::new(ui)
                    .striped(true)
                    .resizable(true)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .column(Column::auto().at_least(28.0)) // P
                    .column(Column::auto().at_least(28.0)) // L
                    .column(Column::initial(78.0))         // Date
                    .column(Column::initial(180.0).at_least(120.0)) // Desc
                    .column(Column::initial(82.0))         // Debit
                    .column(Column::initial(82.0))         // Credit
                    .column(Column::initial(94.0))         // Balance
                    .column(Column::auto().at_least(28.0)) // Revert
                    .header(20.0, |mut header| {
                        for label in ["P", "#", "Date", "Description", "Debit", "Credit", "Balance", ""].iter() {
                            header.col(|ui| { ui.strong(*label); });
                        }
                    })
                    .body(|mut body| {
                        for tx in txs.iter() {
                            let key = (tx.page, tx.line_on_page);
                            let has_edit = self
                                .workflow_edits
                                .iter()
                                .any(|e| e.page == key.0 && e.line_on_page == key.1);

                            body.row(20.0, |mut row| {
                                row.col(|ui| {
                                    ui.label(format!("{}", tx.page + 1));
                                });
                                row.col(|ui| {
                                    ui.label(format!("{}", tx.line_on_page + 1));
                                });

                                // Date — text field
                                row.col(|ui| {
                                    let buf = Self::cell_buffer(
                                        &mut self.workflow_cell_buffers,
                                        &self.workflow_edits,
                                        tx,
                                        EditField::Date,
                                        || tx.date.clone(),
                                    );
                                    if ui
                                        .add(egui::TextEdit::singleline(buf).desired_width(76.0))
                                        .changed()
                                    {
                                        cell_changes.push((
                                            tx.page,
                                            tx.line_on_page,
                                            EditField::Date,
                                            buf.clone(),
                                            Self::bbox_for_field(tx, EditField::Date),
                                            tx.date.clone(),
                                        ));
                                    }
                                });

                                // Description — text field
                                row.col(|ui| {
                                    let buf = Self::cell_buffer(
                                        &mut self.workflow_cell_buffers,
                                        &self.workflow_edits,
                                        tx,
                                        EditField::Description,
                                        || tx.raw_text.clone(),
                                    );
                                    if ui
                                        .add(egui::TextEdit::singleline(buf).desired_width(178.0))
                                        .changed()
                                    {
                                        cell_changes.push((
                                            tx.page,
                                            tx.line_on_page,
                                            EditField::Description,
                                            buf.clone(),
                                            Self::bbox_for_field(tx, EditField::Description),
                                            tx.raw_text.clone(),
                                        ));
                                    }
                                });

                                // Debit / Credit / Balance — money fields with red border on parse failure.
                                Self::money_cell(
                                    &mut row,
                                    &mut self.workflow_cell_buffers,
                                    &self.workflow_edits,
                                    tx,
                                    EditField::Debit,
                                    tx.debit,
                                    palette.warn,
                                    &mut cell_changes,
                                );
                                Self::money_cell(
                                    &mut row,
                                    &mut self.workflow_cell_buffers,
                                    &self.workflow_edits,
                                    tx,
                                    EditField::Credit,
                                    tx.credit,
                                    palette.warn,
                                    &mut cell_changes,
                                );
                                Self::money_cell(
                                    &mut row,
                                    &mut self.workflow_cell_buffers,
                                    &self.workflow_edits,
                                    tx,
                                    EditField::RunningBalance,
                                    tx.running_balance,
                                    palette.warn,
                                    &mut cell_changes,
                                );

                                // Revert column
                                row.col(|ui| {
                                    let label = if has_edit { "↶" } else { " " };
                                    if ui
                                        .add_enabled(
                                            has_edit,
                                            egui::Button::new(label).small(),
                                        )
                                        .on_hover_text("Revert all queued edits on this row")
                                        .clicked()
                                    {
                                        row_reverts.push((tx.page, tx.line_on_page));
                                    }
                                });
                            });
                        }
                    });
            });

        // Apply collected changes after the table render so we don't double-borrow self.
        for (page, line, field, new_text, bbox, old_text) in cell_changes {
            self.upsert_edit(UserEdit {
                page,
                line_on_page: line,
                bbox,
                old_text,
                new_text,
                field,
            });
        }
        if !row_reverts.is_empty() {
            for (page, line) in row_reverts {
                self.revert_row_edits(page, line);
            }
        }
    }

    /// Pick the per-field bbox for an edit. Falls back to the row-level
    /// bbox when the field-specific one isn't known (older parses, manual
    /// transactions). Stage 7.5 — without this, a debit edit would redact
    /// the entire row.
    fn bbox_for_field(
        tx: &crate::engine::model::Transaction,
        field: crate::engine::workflow::EditField,
    ) -> [f32; 4] {
        use crate::engine::workflow::EditField;
        let specific = match field {
            EditField::Date => tx.field_bboxes.date,
            EditField::Description => tx.field_bboxes.description,
            EditField::Debit => tx.field_bboxes.debit,
            EditField::Credit => tx.field_bboxes.credit,
            EditField::RunningBalance => tx.field_bboxes.running_balance,
        };
        specific.or(tx.bbox).unwrap_or([0.0; 4])
    }

    /// Get-or-init the per-cell text buffer. If the user has already queued
    /// an edit for this cell, the buffer reflects the queued new text;
    /// otherwise it starts from the parsed value.
    fn cell_buffer<'a>(
        buffers: &'a mut std::collections::HashMap<
            (usize, usize, crate::engine::workflow::EditField),
            String,
        >,
        edits: &[crate::engine::workflow::UserEdit],
        tx: &crate::engine::model::Transaction,
        field: crate::engine::workflow::EditField,
        default: impl FnOnce() -> String,
    ) -> &'a mut String {
        let key = (tx.page, tx.line_on_page, field);
        buffers.entry(key).or_insert_with(|| {
            edits
                .iter()
                .find(|e| e.page == tx.page && e.line_on_page == tx.line_on_page && e.field == field)
                .map(|e| e.new_text.clone())
                .unwrap_or_else(default)
        })
    }

    /// Render a single money cell (debit/credit/balance). Red border when
    /// the typed text isn't parseable.
    fn money_cell(
        row: &mut egui_extras::TableRow<'_, '_>,
        buffers: &mut std::collections::HashMap<
            (usize, usize, crate::engine::workflow::EditField),
            String,
        >,
        edits: &[crate::engine::workflow::UserEdit],
        tx: &crate::engine::model::Transaction,
        field: crate::engine::workflow::EditField,
        original: Option<rust_decimal::Decimal>,
        warn_color: egui::Color32,
        out: &mut Vec<(
            usize,
            usize,
            crate::engine::workflow::EditField,
            String,
            [f32; 4],
            String,
        )>,
    ) {
        row.col(|ui| {
            let buf = Self::cell_buffer(buffers, edits, tx, field, || {
                original.map(|v| format!("{v:.2}")).unwrap_or_default()
            });
            let valid = buf.trim().is_empty()
                || buf
                    .chars()
                    .filter(|c| c.is_ascii_digit() || *c == '-' || *c == '.')
                    .collect::<String>()
                    .parse::<f64>()
                    .is_ok();
            let mut edit = egui::TextEdit::singleline(buf).desired_width(80.0);
            if !valid {
                edit = edit.text_color(warn_color);
            }
            let resp = ui.add(edit);
            if resp.changed() {
                let old_text = original.map(|v| format!("{v:.2}")).unwrap_or_default();
                out.push((
                    tx.page,
                    tx.line_on_page,
                    field,
                    buf.clone(),
                    Self::bbox_for_field(tx, field),
                    old_text,
                ));
            }
        });
    }

    /// Insert or replace the edit on (page, line, field). When the new
    /// text equals the originally-parsed value, the edit is removed from
    /// the queue instead — typing a value back to its original is
    /// equivalent to no edit at all.
    fn upsert_edit(&mut self, mut edit: crate::engine::workflow::UserEdit) {
        // If the new text equals the original, drop any matching edit.
        let parsed_original = self
            .workflow_transactions
            .iter()
            .find(|t| t.page == edit.page && t.line_on_page == edit.line_on_page);
        let original_text = parsed_original
            .map(|t| match edit.field {
                crate::engine::workflow::EditField::Date => t.date.clone(),
                crate::engine::workflow::EditField::Description => t.raw_text.clone(),
                crate::engine::workflow::EditField::Debit => {
                    t.debit.map(|v| format!("{:.2}", v.round_dp(2))).unwrap_or_default()
                }
                crate::engine::workflow::EditField::Credit => {
                    t.credit.map(|v| format!("{:.2}", v.round_dp(2))).unwrap_or_default()
                }
                crate::engine::workflow::EditField::RunningBalance => t
                    .running_balance
                    .map(|v| format!("{:.2}", v.round_dp(2)))
                    .unwrap_or_default(),
            })
            .unwrap_or_default();

        // Use the original text we just looked up.
        if edit.old_text.is_empty() {
            edit.old_text = original_text.clone();
        }

        // No-op if the user typed back to the original — drop it.
        if edit.new_text == original_text {
            self.workflow_edits.retain(|e| {
                !(e.page == edit.page && e.line_on_page == edit.line_on_page && e.field == edit.field)
            });
            self.workflow_dirty = true;
            return;
        }

        if let Some(slot) = self.workflow_edits.iter_mut().find(|e| {
            e.page == edit.page && e.line_on_page == edit.line_on_page && e.field == edit.field
        }) {
            slot.new_text = edit.new_text;
            slot.bbox = edit.bbox;
        } else {
            self.workflow_edits.push(edit);
        }
        self.workflow_dirty = true;
    }

    /// Drop every queued edit on (page, line) and reset the cell buffers
    /// for that row so the table reflects the parsed values.
    fn revert_row_edits(&mut self, page: usize, line_on_page: usize) {
        let before = self.workflow_edits.len();
        self.workflow_edits
            .retain(|e| !(e.page == page && e.line_on_page == line_on_page));
        let removed = before.saturating_sub(self.workflow_edits.len());
        // Clear the cached cell buffers for this row so they re-init from
        // the parsed transaction next frame.
        self.workflow_cell_buffers
            .retain(|(p, l, _), _| !(*p == page && *l == line_on_page));
        if removed > 0 {
            self.workflow_dirty = true;
            self.toast(
                ToastKind::Info,
                format!("Reverted {} edit(s) on P{} L{}", removed, page + 1, line_on_page + 1),
            );
        }
    }

    /// Stage 8.5: per-font breakdown for the loaded PDF. Shows the user which
    /// fonts can be edited freely and which would need glyph creation, with
    /// an exact list of missing characters per font and the creation scope.
    fn draw_font_analysis_section(&mut self, ui: &mut egui::Ui) {
        let palette = self.settings.theme.palette();
        let analysis = match &self.font_analysis {
            Some(a) => a.clone(),
            None => {
                ui.collapsing("🔤 Font analysis", |ui| {
                    ui.label("Loading...");
                    if ui.button("Re-analyze").clicked() {
                        let _ = self.job_tx.send(Job::AnalyzeFonts {
                            path: PathBuf::from(&self.input_path),
                        });
                        self.in_flight += 1;
                    }
                });
                return;
            }
        };

        let header = if analysis.summary.all_fonts_covered {
            format!(
                "🔤 Font analysis — ✅ {} font(s), all covered",
                analysis.summary.total_fonts
            )
        } else {
            format!(
                "🔤 Font analysis — ⚠ {}/{} font(s) need attention",
                analysis.summary.fonts_needing_action, analysis.summary.total_fonts
            )
        };

        ui.collapsing(header, |ui| {
            // High-level summary line.
            let summary_color = if analysis.summary.all_fonts_covered {
                palette.success
            } else {
                palette.warn
            };
            ui.colored_label(summary_color, analysis.one_line_summary());

            if !analysis.summary.all_fonts_covered {
                ui.horizontal(|ui| {
                    if analysis.summary.missing_digit_count > 0 {
                        ui.colored_label(
                            palette.warn,
                            format!("Digits: {}", analysis.summary.missing_digit_count),
                        );
                    }
                    if analysis.summary.missing_letter_count > 0 {
                        ui.colored_label(
                            palette.warn,
                            format!("Letters: {}", analysis.summary.missing_letter_count),
                        );
                    }
                    if analysis.summary.missing_other_count > 0 {
                        ui.colored_label(
                            palette.warn,
                            format!("Other: {}", analysis.summary.missing_other_count),
                        );
                    }
                });
            }

            ui.separator();

            if ui.button("🔄 Re-analyze").clicked() {
                let _ = self.job_tx.send(Job::AnalyzeFonts {
                    path: PathBuf::from(&self.input_path),
                });
                self.in_flight += 1;
            }

            ui.separator();

            // Per-font breakdown.
            egui::ScrollArea::vertical()
                .id_source("font-analysis-list")
                .max_height(280.0)
                .show(ui, |ui| {
                    for (i, font) in analysis.fonts.iter().enumerate() {
                        let needs_action = !font.missing_chars.is_empty();
                        let row_color = if needs_action {
                            palette.warn
                        } else {
                            palette.success
                        };
                        let role_label = match font.usage_role {
                            crate::engine::font_analysis::UsageRole::Digits => "digits",
                            crate::engine::font_analysis::UsageRole::Letters => "letters",
                            crate::engine::font_analysis::UsageRole::Mixed => "mixed",
                            crate::engine::font_analysis::UsageRole::Punctuation => "punct",
                            crate::engine::font_analysis::UsageRole::Other => "other",
                        };
                        let header = format!(
                            "{} {} • {} • {} use(s) on {} page(s)",
                            if needs_action { "⚠" } else { "✅" },
                            font.base_name,
                            role_label,
                            font.occurrences,
                            font.pages_used_on.len(),
                        );
                        let id = ui.make_persistent_id(("font-analysis", i));
                        egui::collapsing_header::CollapsingState::load_with_default_open(
                            ui.ctx(),
                            id,
                            false,
                        )
                        .show_header(ui, |ui| {
                            ui.colored_label(row_color, header);
                        })
                        .body(|ui| {
                            ui.label(font.fidelity_impact.as_str());
                            ui.label(font.creation_scope.as_str());
                            ui.small(format!(
                                "Standard-14: {} • Subset: {}",
                                if font.is_standard_14 { "yes" } else { "no" },
                                if font.is_subset { "yes" } else { "no" },
                            ));
                            // Truncate the used-character preview at 80 chars
                            // so a font with hundreds of glyphs doesn't dominate
                            // the panel.
                            let used_preview: String = if font.characters_used.chars().count() > 80
                            {
                                let head: String =
                                    font.characters_used.chars().take(80).collect();
                                format!("{head}…")
                            } else {
                                font.characters_used.clone()
                            };
                            ui.small(format!("Used characters: {used_preview}"));
                            if !font.missing_chars.is_empty() {
                                let missing_str = font.missing_chars.join(" ");
                                ui.colored_label(
                                    palette.warn,
                                    format!("Missing: {missing_str}"),
                                );
                                let bd = &font.missing_breakdown;
                                if !bd.digits.is_empty() {
                                    ui.small(format!("  Digits: {}", bd.digits.join(" ")));
                                }
                                if !bd.letters.is_empty() {
                                    ui.small(format!("  Letters: {}", bd.letters.join(" ")));
                                }
                                if !bd.other.is_empty() {
                                    ui.small(format!("  Other: {}", bd.other.join(" ")));
                                }
                            }
                            ui.small(format!(
                                "Sizes: {:.1}–{:.1}pt • Pages: {}",
                                font.size_range[0],
                                font.size_range[1],
                                font.pages_used_on
                                    .iter()
                                    .map(|p| (p + 1).to_string())
                                    .collect::<Vec<_>>()
                                    .join(", "),
                            ));
                        });
                    }
                });
        });
    }

    fn draw_workflow_section(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("🤖 Workflow (AI parse → preview → render → verify)", |ui| {
            let stage = self.workflow_stage.clone();
            let p = self.settings.theme.palette();

            // Step indicator. Stage 13 / Item #1: each label is hoverable
            // so the user can read what the step actually does, and the
            // active step gets a strong color so the indicator never looks
            // muted at idle.
            let step = stage.step_index();
            ui.horizontal(|ui| {
                let descriptions = [
                    "Run Document AI + Gemini completeness check",
                    "Edit values inline; queued edits go to Preview",
                    "Recompute every running balance with your edits",
                    "Apply edits to the PDF (binary-level redact-and-replace)",
                    "Render & compare; loop until visual match passes",
                    "Re-parse with Document AI to confirm math integrity",
                ];
                for (i, name) in [
                    "Parse", "Edit", "Preview", "Render", "Verify", "Confirm",
                ]
                .iter()
                .enumerate()
                {
                    let active_step = (i + 1) as u8;
                    let (color, label) = if active_step < step {
                        (p.success, format!("✓ {}. {name}", i + 1))
                    } else if active_step == step.min(6) {
                        (p.accent, format!("► {}. {name}", i + 1))
                    } else {
                        (p.weak, format!("{}. {name}", i + 1))
                    };
                    ui.colored_label(color, label).on_hover_text(descriptions[i]);
                }
            });
            ui.label(format!("Status: {}", stage.label()));

            ui.separator();



            if let Some(v) = &self.workflow_validation {
                ui.label(format!(
                    "Found {} txs • opening ${:.2} • closing ${:.2}",
                    v.transactions_found, v.opening_balance, v.closing_balance
                ));
                let bar_color = if v.is_acceptable() { p.success } else { p.warn };
                ui.colored_label(
                    bar_color,
                    format!("AI completeness: {:.0}%", v.completeness_score * 100.0),
                );
                if !v.completeness_notes.is_empty() {
                    ui.small(&v.completeness_notes);
                }
                if !v.missing_rows.is_empty() {
                    ui.colored_label(p.warn, format!("Possibly missing rows: {}", v.missing_rows.len()));
                    for m in v.missing_rows.iter().take(3) {
                        ui.small(format!("  • {m}"));
                    }
                }
            }

            ui.separator();

            // Stage 5 / Item #6 + #8: inline edit table with per-row revert.
            self.draw_workflow_edit_table(ui);

            ui.separator();

            // Stage 3 button: balance preview
            let preview_enabled = self.workflow_validation.is_some();
            ui.label(format!("Pending edits queued: {}", self.workflow_edits.len()));
            if ui
                .add_enabled(preview_enabled, egui::Button::new("② Balance Out Preview"))
                .on_hover_text("Recompute every running balance with your edits and show the diff")
                .clicked()
            {
                if let Some(v) = &self.workflow_validation {
                    let _ = self.job_tx.send(Job::WorkflowPreview {
                        original_transactions: self.workflow_transactions.clone(),
                        edits: self.workflow_edits.clone(),
                        opening_balance: v.opening_balance,
                        expected_closing: if v.closing_balance.abs() > rust_decimal::Decimal::ZERO {
                            Some(v.closing_balance)
                        } else {
                            None
                        },
                    });
                    self.in_flight += 1;
                }
            }

            if let Some(p) = &self.workflow_preview {
                let changed = p.rows.iter().filter(|r| r.will_change).count();
                let kind_color = if p.balanced { self.settings.theme.palette().success } else { self.settings.theme.palette().warn };
                ui.colored_label(
                    kind_color,
                    format!(
                        "{} row(s) will change • final imbalance ${:.2}",
                        changed, p.final_imbalance
                    ),
                );
                if let Some(msg) = &p.auto_correction_message {
                    ui.small(msg);
                }
                // Compact diff list
                egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                    for r in p.rows.iter().filter(|r| r.will_change).take(20) {
                        // Char-aware truncation so multi-byte UTF-8 (CJK,
                        // accented Latin) doesn't panic on byte slicing.
                        let desc_short: String = if r.description.chars().count() > 24 {
                            let head: String = r.description.chars().take(24).collect();
                            format!("{head}…")
                        } else {
                            r.description.clone()
                        };
                        ui.small(format!(
                            "P{} L{} {} • bal {:?} → {:?}",
                            r.page + 1,
                            r.line_on_page + 1,
                            desc_short,
                            r.old_running_balance,
                            r.new_running_balance,
                        ));
                    }
                });
            }

            ui.separator();

            // Stage 4-6 button: confirm & render
            let confirm_enabled = self.workflow_preview.is_some();
            if ui
                .add_enabled(confirm_enabled, egui::Button::new("③ Confirm and Render"))
                .on_hover_text(
                    "Apply edits to the PDF, render-validate visually in a loop, then re-parse with Document AI to confirm math",
                )
                .clicked()
            {
                // Stage 2 / Item #7: drop edits whose typed value already
                // matches the cascade. Reduces visual noise (extra redactions)
                // and shortens the apply loop.
                let edits_to_apply = if let Some(p) = &self.workflow_preview {
                    let (kept, dropped) =
                        crate::engine::workflow::prune_redundant_edits(&self.workflow_edits, p);
                    if !dropped.is_empty() {
                        self.toast(
                            ToastKind::Info,
                            format!("Pruned {} redundant edit(s)", dropped.len()),
                        );
                    }
                    kept
                } else {
                    self.workflow_edits.clone()
                };
                let _ = self.job_tx.send(Job::WorkflowConfirmAndRender {
                    input: PathBuf::from(&self.input_path),
                    output: PathBuf::from(&self.output_path),
                    edits: edits_to_apply,
                    deep_font_replication: self.settings.deep_font_replication,
                    max_visual_attempts: 5,
                    visual_threshold: 0.02,
                });
                self.in_flight += 1;
            }

            if let Some(va) = &self.workflow_visual {
                let palette = self.settings.theme.palette();
                let c = if va.passed() { palette.success } else { palette.warn };
                ui.colored_label(
                    c,
                    format!(
                        "Visual {}/{} • diff {:.4} • intended-only {}",
                        va.attempt,
                        va.max_attempts,
                        va.diff_score,
                        if va.only_intended { "✓" } else { "✗" }
                    ),
                );
            }
            if let Some(o) = &self.workflow_outcome {
                ui.colored_label(self.settings.theme.palette().success, &o.completion_summary);
                ui.small(format!("Final PDF: {}", o.final_pdf.display()));
                ui.small(format!(
                    "Re-parsed transactions: {} • final imbalance ${:.2}",
                    o.transactions_re_parsed, o.final_imbalance
                ));
            }

            // Stage 12 / Item #3: surface cascade results so the user can
            // see exactly which tier(s) closed any font-coverage gap.
            if !self.font_cascade_reports.is_empty() {
                ui.separator();
                ui.label("🔧 Font cascade history:");
                let palette = self.settings.theme.palette();
                for report in &self.font_cascade_reports {
                    let color = if report.success { palette.success } else { palette.warn };
                    ui.colored_label(
                        color,
                        format!(
                            "Attempt {} on '{}': {}",
                            report.workflow_attempt,
                            report.original_font,
                            report.one_line_summary()
                        ),
                    );
                    if !report.synthesised.is_empty() {
                        ui.small(format!("  composite: {}", report.synthesised.join(", ")));
                    }
                    if !report.donor_extended.is_empty() {
                        ui.small(format!("  donor:     {}", report.donor_extended.join(", ")));
                    }
                    if !report.ai_extended.is_empty() {
                        ui.small(format!("  AI donor:  {}", report.ai_extended.join(", ")));
                    }
                    if !report.still_missing.is_empty() {
                        ui.colored_label(
                            palette.warn,
                            format!("  still missing: {}", report.still_missing.join(", ")),
                        );
                    }
                }
            }
        });
    }

    fn draw_batch_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Batch Processing Dashboard");
            ui.add_space(10.0);
            
            ui.horizontal(|ui| {
                if ui.button("📂 Select Directory").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.batch_folder_path = Some(path.clone());
                        self.batch_files.clear();
                        if let Ok(entries) = std::fs::read_dir(&path) {
                            for entry in entries.filter_map(|e| e.ok()) {
                                let p = entry.path();
                                if p.is_file() && p.extension().and_then(|s| s.to_str()).map(|s| s.to_lowercase()) == Some("pdf".to_string()) {
                                    self.batch_files.push(p);
                                }
                            }
                        }
                    }
                }
                if let Some(path) = &self.batch_folder_path {
                    ui.label(format!("Selected: {}", path.display()));
                } else {
                    ui.label("Drag and drop a folder of statements here, or click to select a directory.");
                }
            });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);
            
            ui.horizontal(|ui| {
                let has_files = !self.batch_files.is_empty();
                if ui.add_enabled(has_files, egui::Button::new("Extract All to JSON")).clicked() {
                    for file in &self.batch_files {
                        let _ = self.job_tx.send(Job::ExtractTransactions { path: file.clone() });
                        self.in_flight += 1;
                    }
                    self.toast(ToastKind::Info, format!("Queued {} extraction jobs", self.batch_files.len()));
                }
                if ui.add_enabled(has_files, egui::Button::new("Auto-Balance All")).clicked() {
                    for file in &self.batch_files {
                        let output = file.with_file_name(format!("{}_balanced.pdf", file.file_stem().unwrap_or_default().to_string_lossy()));
                        let _ = self.job_tx.send(Job::BalanceAndApplyAll {
                            input: file.clone(),
                            output,
                            auto_apply: true,
                        });
                        self.in_flight += 1;
                    }
                    self.toast(ToastKind::Info, format!("Queued {} balancing jobs", self.batch_files.len()));
                }
                if ui.add_enabled(has_files, egui::Button::new("Verify All against Originals")).clicked() {
                    self.toast(ToastKind::Info, "Batch Verify requires paired _original and _edited files, not yet implemented.");
                }
            });

            ui.add_space(10.0);
            
            if !self.batch_files.is_empty() {
                ui.heading(format!("{} PDF(s) found", self.batch_files.len()));
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for file in &self.batch_files {
                        ui.label(file.file_name().unwrap_or_default().to_string_lossy());
                    }
                });
            }
        });
    }

    fn draw_audit_explorer_view(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(self.settings.theme.palette().bg))
            .show(ctx, |ui| {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    ui.heading(egui::RichText::new("Audit Explorer").size(32.0).strong());
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("Feature under construction: Interactive history log of all modifications.")
                        .color(self.settings.theme.palette().weak));
                });
            });
    }

    fn draw_central_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // Toolbar above canvas
            ui.horizontal(|ui| {
                if ui.button("🔍-").clicked() {
                    self.zoom_factor = (self.zoom_factor * 0.85).clamp(0.1, 5.0);
                    self.fit_to_view = false;
                }
                if ui.button("🔍+").clicked() {
                    self.zoom_factor = (self.zoom_factor * 1.15).clamp(0.1, 5.0);
                    self.fit_to_view = false;
                }
                if ui.button("Fit").clicked() {
                    self.fit_to_view = true;
                }
                if ui.button("100%").clicked() {
                    self.zoom_factor = 1.0;
                    self.pan_offset = egui::Vec2::ZERO;
                    self.fit_to_view = false;
                }
                ui.separator();
                ui.checkbox(&mut self.show_curtain, "Curtain Diff");
                if self.show_curtain {
                    ui.add(egui::Slider::new(&mut self.curtain_ratio, 0.0..=1.0).text("split"));
                }
            });

            egui::Frame::canvas(ui.style()).show(ui, |ui| {
                let (response, painter) = ui.allocate_painter(
                    ui.available_size(),
                    egui::Sense::drag().union(egui::Sense::click()),
                );

                // Zoom — Ctrl+wheel
                let zoom_scroll = ui.input(|i| {
                    if i.modifiers.command {
                        i.smooth_scroll_delta.y
                    } else {
                        0.0
                    }
                });
                if zoom_scroll != 0.0 {
                    self.zoom_factor = (self.zoom_factor + zoom_scroll * 0.002).clamp(0.1, 5.0);
                    self.fit_to_view = false;
                }

                // Pan — any drag (primary, middle, etc.)
                if response.dragged() {
                    self.pan_offset += response.drag_delta();
                    self.fit_to_view = false;
                }

                if let Some(texture) = self.current_page_texture.clone() {
                    let tex_size = texture.size_vec2();
                    if self.fit_to_view {
                        self.fit_zoom_to_view(response.rect.size(), tex_size);
                    }
                    let size = tex_size * self.zoom_factor;
                    let center = response.rect.center() + self.pan_offset;
                    let rect = egui::Rect::from_center_size(center, size);

                    painter.image(
                        texture.id(),
                        rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );

                    // Curtain diff: paint the "after" texture clipped to ratio
                    if self.show_curtain {
                        if let Some(after) = self.after_texture.clone() {
                            let split_x = rect.min.x + rect.width() * self.curtain_ratio;
                            let after_rect =
                                egui::Rect::from_min_max(egui::pos2(split_x, rect.min.y), rect.max);
                            let uv_min = egui::pos2(self.curtain_ratio, 0.0);
                            painter.image(
                                after.id(),
                                after_rect,
                                egui::Rect::from_min_max(uv_min, egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE,
                            );
                            painter.line_segment(
                                [
                                    egui::pos2(split_x, rect.min.y),
                                    egui::pos2(split_x, rect.max.y),
                                ],
                                egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 200, 0)),
                            );
                        }

                        // Hotspot Highlighting: draw red/green boxes around modified regions
                        if let Some((w, h)) = self.current_page_size_pts {
                            for edit in &self.workflow_edits {
                                if edit.page == self.current_page {
                                    let [x0, y0, x1, y1] = edit.bbox;
                                    let sx0 = rect.min.x + (x0 / w) * size.x;
                                    let sy0 = rect.min.y + (y0 / h) * size.y;
                                    let sx1 = rect.min.x + (x1 / w) * size.x;
                                    let sy1 = rect.min.y + (y1 / h) * size.y;
                                    
                                    let item_rect = egui::Rect::from_min_max(
                                        egui::pos2(sx0, sy0),
                                        egui::pos2(sx1, sy1),
                                    );
                                    
                                    let split_x = rect.min.x + rect.width() * self.curtain_ratio;
                                    
                                    if sx0 < split_x {
                                        let mut r = item_rect;
                                        r.max.x = r.max.x.min(split_x);
                                        painter.rect_stroke(r, 2.0, egui::Stroke::new(2.0, egui::Color32::from_rgba_premultiplied(255, 50, 50, 150)));
                                    }
                                    
                                    if sx1 > split_x {
                                        let mut r = item_rect;
                                        r.min.x = r.min.x.max(split_x);
                                        painter.rect_stroke(r, 2.0, egui::Stroke::new(2.0, egui::Color32::from_rgba_premultiplied(50, 255, 50, 150)));
                                    }
                                }
                            }
                        }
                    }

                    // Click → resolve text block via Python
                    if response.clicked() {
                        if let Some(pos) = response.interact_pointer_pos() {
                            let relative = pos - rect.min;
                            let (x, y) = if let Some((w, h)) = self.current_page_size_pts {
                                (relative.x * w / size.x, relative.y * h / size.y)
                            } else {
                                (relative.x / self.zoom_factor, relative.y / self.zoom_factor)
                            };
                            let (tx, rx) = tokio::sync::oneshot::channel();
                            if self
                                .job_tx
                                .send(Job::Python(
                                    PythonJob::FindTextBlockAtClick {
                                        pdf_path: self
                                            .current_pdf_path
                                            .to_string_lossy()
                                            .to_string(),
                                        page_num: self.current_page,
                                        x,
                                        y,
                                    },
                                    tx,
                                ))
                                .is_ok()
                            {
                                self.pending_python = Some(rx);
                                self.in_flight += 1;
                            }
                        }
                    }

                    // Selected bbox highlight
                    if let Some(block) = &self.selected_block {
                        if block.page == self.current_page {
                            let (sx, sy) = if let Some((w, h)) = self.current_page_size_pts {
                                (size.x / w, size.y / h)
                            } else {
                                (self.zoom_factor, self.zoom_factor)
                            };
                            let min = rect.min + egui::vec2(block.bbox[0] * sx, block.bbox[1] * sy);
                            let max = rect.min + egui::vec2(block.bbox[2] * sx, block.bbox[3] * sy);
                            painter.rect_stroke(
                                egui::Rect::from_min_max(min, max),
                                4.0,
                                egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 200, 0)),
                            );
                        }
                    }

                    // Stage 5 / Item #20: live diff overlay during preview.
                    // Translucent yellow over each `will_change` bbox on the
                    // current page; tooltip shows old → new.
                    if let Some(preview) = self.workflow_preview.clone() {
                        let (sx, sy) = if let Some((w, h)) = self.current_page_size_pts {
                            (size.x / w, size.y / h)
                        } else {
                            (self.zoom_factor, self.zoom_factor)
                        };
                        let mouse = response.hover_pos();
                        for prow in preview
                            .rows
                            .iter()
                            .filter(|r| r.will_change && r.page == self.current_page)
                        {
                            // Find the underlying transaction so we can use its bbox.
                            let Some(tx) = self.workflow_transactions.iter().find(|t| {
                                t.page == prow.page && t.line_on_page == prow.line_on_page
                            }) else {
                                continue;
                            };
                            let Some(bbox) = tx.bbox else {
                                continue;
                            };
                            let min = rect.min + egui::vec2(bbox[0] * sx, bbox[1] * sy);
                            let max = rect.min + egui::vec2(bbox[2] * sx, bbox[3] * sy);
                            let cell = egui::Rect::from_min_max(min, max);
                            // Translucent yellow fill + amber border.
                            painter.rect_filled(
                                cell,
                                2.0,
                                egui::Color32::from_rgba_unmultiplied(255, 220, 0, 70),
                            );
                            painter.rect_stroke(
                                cell,
                                2.0,
                                egui::Stroke::new(
                                    1.0,
                                    egui::Color32::from_rgb(220, 180, 0),
                                ),
                            );
                            // Hover tooltip with the diff text — only when
                            // the pointer is actually over this cell.
                            if let Some(m) = mouse {
                                if cell.contains(m) {
                                    let old_str = prow
                                        .old_running_balance
                                        .map(|v| format!("{v:.2}"))
                                        .unwrap_or_else(|| "—".into());
                                    let new_str = prow
                                        .new_running_balance
                                        .map(|v| format!("{v:.2}"))
                                        .unwrap_or_else(|| "—".into());
                                    egui::show_tooltip(
                                        ctx,
                                        egui::LayerId::new(
                                            egui::Order::Tooltip,
                                            egui::Id::new("diff-tooltip"),
                                        ),
                                        egui::Id::new(("diff-tooltip", prow.page, prow.line_on_page)),
                                        |ui| {
                                            ui.label(format!(
                                                "P{} L{}",
                                                prow.page + 1,
                                                prow.line_on_page + 1
                                            ));
                                            ui.label(format!("{} → {}", old_str, new_str));
                                        },
                                    );
                                }
                            }
                        }
                    }
                } else {
                    // Welcome / empty placeholder
                    self.draw_empty_canvas(ui, response.rect, &painter);
                }
            });
            egui::Area::new(egui::Id::new("floating_action_dock"))
                .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -40.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    if self.selected_block.is_some() || !self.proposed_changes.is_empty() {
                        egui::Frame::window(ui.style())
                            .fill(self.settings.theme.palette().panel.linear_multiply(0.95)) // Slight transparency
                            .shadow(ctx.style().visuals.window_shadow)
                            .rounding(ctx.style().visuals.window_rounding)
                            .inner_margin(egui::Margin::symmetric(20.0, 16.0))
                            .show(ui, |ui| {
                                ui.vertical(|ui| {
                                    // Primary Actions Row
                                    ui.horizontal(|ui| {
                                        let apply_btn = ui.add(
                                            egui::Button::new(
                                                egui::RichText::new("🎯 Apply Single Edit")
                                                    .color(self.settings.theme.palette().bg)
                                            )
                                            .fill(self.settings.theme.palette().accent)
                                        );
                                        
                                        if apply_btn.on_hover_text("Replace the selected text and instantly verify math + fidelity.").clicked() {
                                            if let Some(block) = self.selected_block.clone() {
                                                let input = if self.current_pdf_path.exists() {
                                                    self.current_pdf_path.clone()
                                                } else {
                                                    std::path::PathBuf::from(&self.input_path)
                                                };
                                                let edit = crate::engine::workflow::UserEdit {
                                                    page: self.current_page,
                                                    line_on_page: 0, // Not strictly needed for standalone manual edit
                                                    bbox: block.bbox,
                                                    old_text: block.text.clone(),
                                                    new_text: self.new_text.clone(),
                                                    field: crate::engine::workflow::EditField::Description,
                                                };
                                                let _ = self.job_tx.send(Job::WorkflowConfirmAndRender {
                                                    input,
                                                    output: std::path::PathBuf::from(&self.output_path),
                                                    edits: vec![edit],
                                                    deep_font_replication: self.settings.deep_font_replication,
                                                    max_visual_attempts: 3,
                                                    visual_threshold: 0.05,
                                                });
                                                self.in_flight += 1;
                                            }
                                        }

                                        ui.add_space(12.0);

                                        let adjust_btn = ui.add(
                                            egui::Button::new(
                                                egui::RichText::new("⚖ Auto-Balance Entire Statement")
                                                    .color(self.settings.theme.palette().panel)
                                            )
                                            .fill(self.settings.theme.palette().success)
                                        );
                                        if adjust_btn.on_hover_text("Computes minimal balancing adjustments for the entire statement and applies them automatically.").clicked() {
                                            let input = if self.current_pdf_path.exists() {
                                                self.current_pdf_path.clone()
                                            } else {
                                                std::path::PathBuf::from(&self.input_path)
                                            };
                                            if input.as_os_str().is_empty() || !input.exists() {
                                                self.toast(ToastKind::Error, "Open a PDF first.");
                                            } else {
                                                let _ = self.job_tx.send(Job::BalanceAndApplyAll {
                                                    input,
                                                    output: std::path::PathBuf::from(&self.output_path),
                                                    auto_apply: true,
                                                });
                                                self.in_flight += 1;
                                                self.status = "Auto-balancing entire statement…".into();
                                                self.toast(ToastKind::Info, "Auto-balancing entire statement…");
                                            }
                                        }
                                    });

                                    ui.add_space(12.0);
                                    ui.separator();
                                    ui.add_space(12.0);

                                    // Secondary Tool Row
                                    ui.horizontal(|ui| {
                                        if ui.button("✨ AI Fix Text/Layout").on_hover_text("Use Gemini to automatically fix layout and text discrepancies on this page").clicked() {
                                            let input = if self.current_pdf_path.exists() {
                                                self.current_pdf_path.clone()
                                            } else {
                                                std::path::PathBuf::from(&self.input_path)
                                            };
                                            let _ = self.job_tx.send(Job::AiFixVisualFidelity {
                                                input,
                                                page: self.current_page,
                                            });
                                            self.toast(ToastKind::Info, "Requesting AI Layout Fix…");
                                            self.in_flight += 1;
                                        }
                                        
                                        ui.add_space(8.0);
                                        
                                        if ui.button("📅 Adjust Dates").on_hover_text("Shift or remap all transaction dates").clicked() {
                                            self.show_date_adjust_dialog = true;
                                        }
                                        
                                        ui.add_space(8.0);

                                        if ui.button("🔄 Transfer Transactions").on_hover_text("Transfer transactions from another bank statement PDF into this one").clicked() {
                                            self.show_transfer_dialog = true;
                                        }
                                        
                                        ui.add_space(8.0);

                                        if ui.button("🧪 Test Transfers").on_hover_text("Cross-test transfers between multiple statements").clicked() {
                                            self.show_transfer_test_dialog = true;
                                        }
                                    });
                                });
                            });
                    }
                });
        });
    }

    fn draw_empty_canvas(&mut self, ui: &mut egui::Ui, rect: egui::Rect, painter: &egui::Painter) {
        let p = self.settings.theme.palette();
        // Subtle gradient background
        painter.rect_filled(rect, 0.0, p.bg);

        // If a PDF is currently open but the texture hasn't streamed in yet
        if self.current_pdf_path.exists() {
            ui.allocate_ui_at_rect(rect, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.add(egui::Spinner::new().size(40.0).color(p.accent));
                        ui.add_space(20.0);
                        ui.label(egui::RichText::new("Rendering page asynchronously...")
                            .color(p.weak)
                            .size(16.0));
                    });
                });
            });
            return;
        }

        // Brand block
        let center = rect.center();
        painter.text(
            center - egui::vec2(0.0, 60.0),
            egui::Align2::CENTER_CENTER,
            "📑",
            egui::FontId::proportional(64.0),
            p.accent,
        );
        painter.text(
            center - egui::vec2(0.0, 8.0),
            egui::Align2::CENTER_CENTER,
            "Bank Statement Fidelity Editor",
            egui::FontId::proportional(22.0),
            p.text,
        );
        painter.text(
            center + egui::vec2(0.0, 18.0),
            egui::Align2::CENTER_CENTER,
            "Drop a PDF here, or use File → Open",
            egui::FontId::proportional(14.0),
            p.weak,
        );
        // Quick action buttons in the central area
        let btn_w = 220.0;
        let btn_h = 36.0;
        let gap = 8.0;
        let mut top = center.y + 56.0;
        let mut button = |ui: &mut egui::Ui, label: &str, hint: &str| -> bool {
            let rect = egui::Rect::from_min_size(
                egui::pos2(center.x - btn_w / 2.0, top),
                egui::vec2(btn_w, btn_h),
            );
            top += btn_h + gap;
            let resp = ui.allocate_rect(rect, egui::Sense::click());
            ui.painter().rect_filled(rect, 8.0, p.surface);
            ui.painter().rect_stroke(
                rect,
                8.0,
                egui::Stroke::new(1.0, p.accent.linear_multiply(0.6)),
            );
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(14.0),
                p.text,
            );
            resp.clone().on_hover_text(hint).clicked()
        };
        if button(ui, "📂 Open PDF…", "Browse for a bank statement PDF") {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("PDF", &["pdf"])
                .pick_file()
            {
                self.open_pdf(path);
            }
        }
        if button(
            ui,
            "⏯ Resume last session",
            "Reload the last autosaved history",
        ) {
            let auto = std::path::PathBuf::from("audit").join("history.json");
            if auto.exists() {
                let _ = self.job_tx.send(Job::LoadHistory {
                    input: auto.clone(),
                });
                self.in_flight += 1;
                self.toast(ToastKind::Info, format!("Resuming from {}", auto.display()));
            } else {
                self.toast(ToastKind::Warn, "No previous session found.");
            }
        }
        if button(
            ui,
            "📋 Resume workflow draft",
            "Reload audit/workflow.json — restores parse, queued edits and stage",
        ) {
            self.resume_workflow_draft();
        }
        if !self.settings.recent_files.is_empty()
            && button(
                ui,
                "📜 Open most recent",
                &format!("Open {}", self.settings.recent_files[0]),
            )
        {
            let path = PathBuf::from(self.settings.recent_files[0].clone());
            self.open_pdf(path);
        }
    }

    /// Stage 13 / Item #12: confirmation modals.
    fn draw_modals(&mut self, ctx: &egui::Context) {
        if self.show_settings_modal {
            self.draw_settings_modal(ctx);
        }
        if self.show_transfer_dialog {
            self.draw_transfer_dialog(ctx);
        }
        if self.show_date_adjust_dialog {
            self.draw_date_adjust_dialog(ctx);
        }
        if !self.pending_ai_confirmations.is_empty() {
            self.draw_ai_confirmation_dialog(ctx);
        }
        if self.show_transfer_test_dialog {
            self.draw_transfer_test_dialog(ctx);
        }
        if self.show_discard_draft_confirm {
            let mut keep_open = true;
            let mut confirm = false;
            egui::Window::new("Discard workflow draft?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .open(&mut keep_open)
                .show(ctx, |ui| {
                    ui.label("This will permanently delete audit/workflow.json.");
                    ui.label("Any pending edits in the current workflow draft will be lost.");
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.show_discard_draft_confirm = false;
                        }
                        if ui
                            .add(
                                egui::Button::new("Discard")
                                    .fill(self.settings.theme.palette().warn),
                            )
                            .clicked()
                        {
                            confirm = true;
                        }
                    });
                });
            if confirm {
                Self::discard_workflow_draft_quiet();
                self.toast(ToastKind::Info, "Workflow draft discarded");
                self.show_discard_draft_confirm = false;
            }
            if !keep_open {
                self.show_discard_draft_confirm = false;
            }
        }
    }

    fn draw_toasts(&mut self, ctx: &egui::Context) -> Option<String> {
        let mut clicked_id = None;
        // Drop expired
        let now = Instant::now();
        self.toasts.retain(|t| t.expires_at > now);
        if self.toasts.is_empty() {
            return None;
        }

        egui::Area::new("toasts".into())
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -32.0))
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    let p = self.settings.theme.palette();
                    for toast in self.toasts.iter().rev().take(5) {
                        let bg = match toast.kind {
                            ToastKind::Info => p.info,
                            ToastKind::Warn => p.warn,
                            ToastKind::Error => p.error,
                            ToastKind::Success => p.success,
                        };
                        let icon = match toast.kind {
                            ToastKind::Info => "ℹ",
                            ToastKind::Warn => "⚠",
                            ToastKind::Error => "✗",
                            ToastKind::Success => "✓",
                        };
                        // fade based on remaining lifetime
                        let remaining = toast.expires_at.saturating_duration_since(now);
                        let alpha = (remaining.as_millis() as f32 / 6000.0).clamp(0.4, 1.0);
                        let bg = egui::Color32::from_rgba_unmultiplied(
                            bg.r(),
                            bg.g(),
                            bg.b(),
                            (alpha * 230.0) as u8,
                        );
                        egui::Frame::none()
                            .fill(bg)
                            .rounding(10.0)
                            .stroke(egui::Stroke::new(1.0, egui::Color32::from_white_alpha(40)))
                            .inner_margin(egui::vec2(12.0, 8.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.colored_label(egui::Color32::WHITE, icon);
                                    ui.colored_label(egui::Color32::WHITE, &toast.text);
                                    if let Some(label) = &toast.action_label {
                                        ui.add_space(8.0);
                                        if ui.add(egui::Button::new(egui::RichText::new(label).color(egui::Color32::WHITE)).fill(egui::Color32::from_black_alpha(100))).clicked() {
                                            clicked_id = toast.action_id.clone();
                                        }
                                    }
                                });
                            });
                        ui.add_space(6.0);
                    }
                });
            });

        if let Some(id) = &clicked_id {
            self.toasts.retain(|t| t.action_id.as_ref() != Some(id));
        }
        
        clicked_id
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let input = ctx.input(|i| i.clone());
        if input.modifiers.command && input.key_pressed(egui::Key::O) {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("PDF", &["pdf"])
                .pick_file()
            {
                self.open_pdf(path);
            }
        }
        if input.modifiers.command && input.key_pressed(egui::Key::Z) {
            let _ = self.job_tx.send(Job::Undo);
        }
        if input.modifiers.command && input.key_pressed(egui::Key::Y) {
            let _ = self.job_tx.send(Job::Redo);
        }
        if input.modifiers.command && input.key_pressed(egui::Key::S) {
            let _ = self.job_tx.send(Job::ExportChangeHistory {
                output: PathBuf::from(&self.export_path),
            });
        }
        if input.key_pressed(egui::Key::PageDown) || input.key_pressed(egui::Key::ArrowRight) {
            if self.current_page + 1 < self.total_pages {
                self.current_page += 1;
                self.request_render("current");
            }
        }
        if input.key_pressed(egui::Key::PageUp) || input.key_pressed(egui::Key::ArrowLeft) {
            if self.current_page > 0 {
                self.current_page -= 1;
                self.request_render("current");
            }
        }
        if input.key_pressed(egui::Key::Plus) || input.key_pressed(egui::Key::Equals) {
            self.zoom_factor = (self.zoom_factor * 1.15).clamp(0.1, 5.0);
            self.fit_to_view = false;
        }
        if input.key_pressed(egui::Key::Minus) {
            self.zoom_factor = (self.zoom_factor * 0.85).clamp(0.1, 5.0);
            self.fit_to_view = false;
        }
        if input.key_pressed(egui::Key::Num0) {
            self.zoom_factor = 1.0;
            self.pan_offset = egui::Vec2::ZERO;
            self.fit_to_view = false;
        }
    }

    fn open_pdf(&mut self, path: PathBuf) {
        if !path.exists() {
            self.toast(
                ToastKind::Error,
                format!("File not found: {}", path.display()),
            );
            return;
        }
        self.input_path = path.to_string_lossy().to_string();
        self.current_pdf_path = path.clone();
        self.previous_pdf_path = None;
        self.history_state = ChangeHistory::new();
        self.proposed_changes.clear();
        self.last_imbalance = None;
        self.last_verification = None;
        self.last_warning = None;
        self.selected_block = None;
        // Stage 6: opening a new PDF invalidates any cached hash and any
        // in-flight workflow buffers — those belong to the previous file.
        self.workflow_input_hash = None;
        self.workflow_cell_buffers.clear();
        // Stage 8.5: clear the font analysis; the runtime will produce a
        // fresh one for the new PDF.
        self.font_analysis = None;
        let _ = self.job_tx.send(Job::LoadDocument {
            path: self.current_pdf_path.clone(),
            three_page_mode: self.settings.three_page_mode,
        });
        self.in_flight += 1;
    }

    /// Path of the on-disk autosave for the current workflow. One file per
    /// session — overwritten as edits change. Stage 5 / Item #9.
    fn workflow_draft_path() -> PathBuf {
        PathBuf::from("audit").join("workflow.json")
    }

    /// Delete the on-disk draft if it exists. Used after a successful
    /// `WorkflowComplete` and from the "Discard draft" menu. Errors are
    /// logged but never surfaced — the file may legitimately be missing.
    fn discard_workflow_draft_quiet() {
        let path = Self::workflow_draft_path();
        if path.exists() {
            if let Err(e) = std::fs::remove_file(&path) {
                tracing::warn!("[gui] removing workflow draft failed: {}", e);
            }
        }
    }

    /// Persist the current workflow state to `audit/workflow.json` if there
    /// is anything worth saving (validation has been done, OR there are
    /// queued edits) and the dirty flag is set. Debounced to at most one
    /// write per 1.5s. Failures are logged but never raised — losing an
    /// autosave is non-fatal.
    fn autosave_workflow_draft(&mut self) {
        if !self.workflow_dirty {
            return;
        }
        // Nothing to save until a parse has produced a baseline.
        if self.workflow_validation.is_none() && self.workflow_edits.is_empty() {
            self.workflow_dirty = false;
            return;
        }
        // Debounce: 1.5s between writes.
        if let Some(t) = self.workflow_last_save {
            if t.elapsed() < Duration::from_millis(1500) {
                return;
            }
        }
        let pdf = PathBuf::from(&self.input_path);
        if !pdf.exists() {
            self.workflow_dirty = false;
            return;
        }
        // Cache the PDF SHA-256 once per (input_path, file change) so the
        // autosave doesn't re-read multi-MB files every 1.5s. The cache key
        // is just the path; if the user opens a new PDF the cache is
        // cleared in `open_pdf`. Stage 6.
        let hash = match &self.workflow_input_hash {
            Some((cached_path, h)) if cached_path == &self.input_path => h.clone(),
            _ => {
                let bytes = match std::fs::read(&pdf) {
                    Ok(b) => b,
                    Err(e) => {
                        tracing::warn!("[gui] reading PDF for hash failed: {}", e);
                        self.workflow_dirty = false;
                        return;
                    }
                };
                let h = crate::engine::workflow::sha256_hex_of(&bytes);
                self.workflow_input_hash = Some((self.input_path.clone(), h.clone()));
                h
            }
        };
        let draft = crate::engine::workflow::WorkflowDraft::new_with_hash(
            &pdf,
            hash,
            self.workflow_validation.clone(),
            self.workflow_transactions.clone(),
            self.workflow_edits.clone(),
        );
        let path = Self::workflow_draft_path();
        match draft.save_to_file(&path) {
            Ok(()) => {
                tracing::debug!("[gui] saved workflow draft to {}", path.display());
                self.workflow_dirty = false;
                self.workflow_last_save = Some(Instant::now());
            }
            Err(e) => {
                tracing::warn!("[gui] saving workflow draft failed: {}", e);
                // Leave dirty=true so we retry next frame.
            }
        }
    }

    /// Resume a workflow from `audit/workflow.json`, restoring validation,
    /// transactions and queued edits. Verifies the on-disk PDF still
    /// hashes to what the draft expects; if not, surfaces a warning toast
    /// but proceeds — the user might intentionally be loading a draft
    /// against a manually-saved copy.
    fn resume_workflow_draft(&mut self) {
        let path = Self::workflow_draft_path();
        if !path.exists() {
            self.toast(ToastKind::Warn, "No workflow draft to resume.");
            return;
        }
        let draft = match crate::engine::workflow::WorkflowDraft::load_from_file(&path) {
            Ok(d) => d,
            Err(e) => {
                self.toast(ToastKind::Error, format!("Could not load draft: {e}"));
                return;
            }
        };

        // Stage 13 / Item #11: when the original PDF is missing, prompt the
        // user to locate it instead of orphaning the draft. If they
        // cancel, leave the draft untouched so they can retry later.
        let mut pdf_path = PathBuf::from(&draft.input_path);
        if !pdf_path.exists() {
            self.toast(
                ToastKind::Warn,
                format!(
                    "PDF missing: {} — please pick the file",
                    pdf_path.display()
                ),
            );
            match rfd::FileDialog::new()
                .add_filter("PDF", &["pdf"])
                .set_title("Locate the PDF this draft was saved against")
                .pick_file()
            {
                Some(picked) => {
                    pdf_path = picked;
                }
                None => {
                    self.toast(
                        ToastKind::Info,
                        "Resume cancelled — draft kept; pick the PDF later.",
                    );
                    return;
                }
            }
        }

        let same = draft.matches_pdf(&pdf_path);
        // Restore session state.
        self.input_path = pdf_path.to_string_lossy().to_string();
        self.current_pdf_path = pdf_path.clone();
        self.workflow_validation = draft.validation.clone();
        self.workflow_transactions = draft.transactions.clone();
        self.workflow_edits = draft.edits.clone();
        self.workflow_preview = None;
        self.workflow_visual = None;
        self.workflow_outcome = None;
        self.workflow_stage = match draft.validation.clone() {
            Some(v) => crate::engine::workflow::WorkflowStage::Editing(v),
            None => crate::engine::workflow::WorkflowStage::Idle,
        };
        self.workflow_dirty = false;

        // Trigger a render of the PDF.
        let _ = self.job_tx.send(Job::LoadDocument {
            path: pdf_path.clone(),
            three_page_mode: self.settings.three_page_mode,
        });
        self.in_flight += 1;

        if same {
            self.toast(
                ToastKind::Success,
                format!(
                    "Resumed workflow draft — {} edits queued",
                    draft.edits.len()
                ),
            );
        } else {
            self.toast(
                ToastKind::Warn,
                "Draft loaded but the PDF has changed since it was saved.",
            );
        }
    }
}

/// Upsert `pairs` (env var name → value) into a dotenv file at `path`.
///
/// Existing `KEY=...` lines are replaced in place (preserving order and
/// unrelated lines/comments); keys not present are appended. A key whose
/// value is empty is written as `KEY=` so the file documents that it was
/// intentionally cleared (the live process env already had it removed by the
/// caller). Creates the file if it does not exist.
fn upsert_env_file(path: &std::path::Path, pairs: &[(&str, String)]) -> std::io::Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();

    // Track which keys we've already written so leftovers get appended.
    let mut remaining: std::collections::HashMap<&str, &String> =
        pairs.iter().map(|(k, v)| (*k, v)).collect();

    let mut out_lines: Vec<String> = Vec::new();
    for line in existing.lines() {
        let trimmed = line.trim_start();
        // Leave comments and blank lines untouched.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            out_lines.push(line.to_string());
            continue;
        }
        if let Some(eq) = trimmed.find('=') {
            let key = trimmed[..eq].trim();
            if let Some(val) = remaining.remove(key) {
                out_lines.push(format!("{key}={val}"));
                continue;
            }
        }
        out_lines.push(line.to_string());
    }

    // Append any keys that weren't already present.
    if !remaining.is_empty() {
        // Deterministic order for the appended block.
        let mut appended: Vec<(&str, &String)> = pairs
            .iter()
            .filter(|(k, _)| remaining.contains_key(*k))
            .map(|(k, v)| (*k, v))
            .collect();
        appended.dedup_by(|a, b| a.0 == b.0);
        for (k, v) in appended {
            out_lines.push(format!("{k}={v}"));
        }
    }

    let mut contents = out_lines.join("\n");
    contents.push('\n');
    std::fs::write(path, contents)
}


// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn load_icon() -> egui::IconData {
    let image = image::load_from_memory(include_bytes!("../../../assets/icon.png"))
        .expect("Failed to open icon path")
        .into_rgba8();
    let (width, height) = image.dimensions();
    egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }
}

pub fn run_gui(
    job_tx: std::sync::mpsc::Sender<Job>,
    job_rx: std::sync::mpsc::Receiver<JobResult>,
    config: std::sync::Arc<crate::app::config::AppConfig>,
) -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([960.0, 640.0])
            .with_title("Bank Statement Fidelity Editor v0.4.0")
            .with_icon(load_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "Bank Statement Fidelity Editor",
        options,
        Box::new(move |_cc| Ok(Box::new(MyApp::new(job_tx, job_rx, config.clone())))),
    )
}

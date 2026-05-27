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
    Dark,
    Light,
    Midnight,
    Solarized,
}

impl Default for Theme {
    fn default() -> Self { Theme::Midnight }
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
        match self {
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
                bg: egui::Color32::from_rgb(15, 17, 25),
                panel: egui::Color32::from_rgb(22, 25, 36),
                surface: egui::Color32::from_rgb(30, 34, 48),
                text: egui::Color32::from_rgb(230, 232, 245),
                weak: egui::Color32::from_rgb(130, 140, 170),
                accent: egui::Color32::from_rgb(140, 170, 255),
                success: egui::Color32::from_rgb(120, 220, 160),
                warn: egui::Color32::from_rgb(245, 195, 100),
                error: egui::Color32::from_rgb(245, 110, 110),
                info: egui::Color32::from_rgb(140, 170, 255),
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
            Theme::Dark => "Dark",
            Theme::Light => "Light",
            Theme::Midnight => "Midnight",
            Theme::Solarized => "Solarized",
        }
    }

    fn apply(self, ctx: &egui::Context) {
        let p = self.palette();
        let mut visuals = match self {
            Theme::Light | Theme::Solarized => egui::Visuals::light(),
            _ => egui::Visuals::dark(),
        };
        visuals.window_rounding = 10.0.into();
        visuals.menu_rounding = 8.0.into();
        visuals.widgets.noninteractive.rounding = 6.0.into();
        visuals.widgets.inactive.rounding = 6.0.into();
        visuals.widgets.hovered.rounding = 6.0.into();
        visuals.widgets.active.rounding = 6.0.into();
        visuals.widgets.open.rounding = 6.0.into();
        visuals.panel_fill = p.panel;
        visuals.window_fill = p.panel;
        visuals.extreme_bg_color = p.bg;
        visuals.faint_bg_color = p.surface;
        visuals.widgets.noninteractive.bg_fill = p.surface;
        visuals.widgets.inactive.bg_fill = p.surface;
        visuals.hyperlink_color = p.accent;
        visuals.selection.bg_fill = p.accent.linear_multiply(0.4);
        visuals.selection.stroke.color = p.accent;
        visuals.warn_fg_color = p.warn;
        visuals.error_fg_color = p.error;
        ctx.set_visuals(visuals);

        // global style tweaks
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 4.0);
        style.spacing.window_margin = egui::Margin::same(10.0);
        style.spacing.menu_margin = egui::Margin::same(6.0);
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(18.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::new(13.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::new(13.0, egui::FontFamily::Proportional),
        );
        style.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::new(11.5, egui::FontFamily::Proportional),
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
        }
    }
}

// ---------------------------------------------------------------------------
// Toast / notification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind { Info, Warn, Error, Success }

#[derive(Debug, Clone)]
struct Toast {
    kind: ToastKind,
    text: String,
    expires_at: Instant,
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

    // View
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
    progress: Option<(String, f32)>,
    last_warning: Option<String>,
    last_verification: Option<VerificationReport>,
    proposed_changes: Vec<(crate::engine::model::ProposedChange, bool)>,
    last_imbalance: Option<f64>,
    in_flight: usize,
    settings: AppSettings,
    toasts: VecDeque<Toast>,

    // Channels
    job_tx: std::sync::mpsc::Sender<Job>,
    job_rx: std::sync::mpsc::Receiver<JobResult>,
    pending_python: Option<tokio::sync::oneshot::Receiver<PythonJobResult>>,

    // Render coalescing
    last_render_request: Option<(String, usize, u32)>,

    // Config (read-only)
    config: std::sync::Arc<crate::app::config::AppConfig>,
}

impl MyApp {
    pub fn new(
        job_tx: std::sync::mpsc::Sender<Job>,
        job_rx: std::sync::mpsc::Receiver<JobResult>,
        config: std::sync::Arc<crate::app::config::AppConfig>,
    ) -> Self {
        let settings: AppSettings = confy::load("bank-statement-modifier", None).unwrap_or_default();
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
            config,
            settings,
        }
    }

    // -- helpers --------------------------------------------------------------

    fn toast(&mut self, kind: ToastKind, msg: impl Into<String>) {
        self.toasts.push_back(Toast {
            kind,
            text: msg.into(),
            expires_at: Instant::now() + Duration::from_secs(6),
        });
        while self.toasts.len() > 5 {
            self.toasts.pop_front();
        }
    }

    fn request_render(&mut self, tag: &str) {
        // Only render if the page actually changed since the last request for
        // this tag. This drops bursts when the user clicks rapidly through
        // pages or zooms — preventing render queue blow-up.
        let key = (tag.to_string(), self.current_page, self.current_page_dpi as u32);
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

    fn update_recent_files(&mut self, path: String) {
        self.settings.recent_files.retain(|f| f != &path);
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
            .filter_map(|(i, r)| r.new_text.replace(['$', ','], "").parse::<f64>().ok().map(|v| [i as f64, v]))
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

        // Drag-and-drop support: open the first dropped PDF.
        if !ctx.input(|i| i.raw.dropped_files.is_empty()) {
            let dropped: Vec<PathBuf> = ctx
                .input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
            for path in dropped {
                if path.extension().and_then(|s| s.to_str()).map(|s| s.to_lowercase()) == Some("pdf".into()) {
                    self.open_pdf(path);
                    break;
                }
            }
        }
        // Visual hover-cue while dragging files
        if !ctx.input(|i| i.raw.hovered_files.is_empty()) {
            let p = self.settings.theme.palette();
            let screen = ctx.screen_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("dnd-overlay")));
            painter.rect_filled(screen, 0.0, egui::Color32::from_rgba_unmultiplied(0, 0, 0, 110));
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
                    if self.in_flight > 0 {
                        self.in_flight -= 1;
                    }
                    self.handle_job_result(ctx, res);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.status = "❌ Runtime worker disconnected".into();
                    break;
                }
            }
        }

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
                            Err(e) => self.toast(ToastKind::Warn, format!("Click parse failed: {e}")),
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

        // ---- 5. Left, right, central --------------------------------------
        self.draw_left_panel(ctx);
        self.draw_right_panel(ctx);
        self.draw_central_panel(ctx);

        // ---- 6. Toasts ----------------------------------------------------
        self.draw_toasts(ctx);

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
            JobResult::DocumentLoaded { total_pages, .. } => {
                self.total_pages = total_pages;
                self.current_page = 0;
                self.current_pdf_path = PathBuf::from(&self.input_path);
                self.previous_pdf_path = None;
                self.update_recent_files(self.input_path.clone());
                self.status = format!("Loaded {total_pages} page(s)");
                self.toast(ToastKind::Success, format!("Loaded {total_pages} pages"));
                self.request_render("current");
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
            JobResult::PageRendered { png_bytes, tag, width_pts, height_pts, .. } => {
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
            JobResult::ChangeApplied { record, requires_visual_review } => {
                self.toast(
                    if requires_visual_review { ToastKind::Warn } else { ToastKind::Success },
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
            JobResult::BalanceProposed { imbalance, changes } => {
                self.last_imbalance = Some(imbalance);
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
                    self.toast(ToastKind::Info, format!("{} adjustments proposed", self.proposed_changes.len()));
                }
            }
            JobResult::ProposedChangesApplied { changes_applied, failures } => {
                if failures.is_empty() {
                    self.toast(ToastKind::Success, format!("Applied {changes_applied} changes"));
                } else {
                    self.toast(
                        ToastKind::Warn,
                        format!("Applied {changes_applied} ({} failures)", failures.len()),
                    );
                }
            }
            JobResult::TransactionsExtracted(txs) => {
                self.toast(ToastKind::Success, format!("Extracted {} transactions", txs.len()));
            }
            JobResult::FontCompleted(_) => {
                self.toast(ToastKind::Success, "Font completion finished");
            }
            JobResult::ChangeHistoryExported { path } => {
                self.toast(ToastKind::Success, format!("History exported: {}", path.display()));
            }
            JobResult::VerificationReport(report) => {
                self.last_verification = Some(report.clone());
                let kind = if report.math_valid && report.only_intended_changes {
                    ToastKind::Success
                } else {
                    ToastKind::Warn
                };
                self.toast(kind, format!("Verification: {}", report.message.lines().next().unwrap_or("done")));
            }
            JobResult::Progress { label, fraction } => {
                if fraction >= 1.0 {
                    self.progress = None;
                } else {
                    self.progress = Some((label, fraction));
                }
            }
            JobResult::Error { job_label, message } => {
                self.status = format!("❌ [{job_label}] {message}");
                self.toast(ToastKind::Error, format!("[{job_label}] {message}"));
                tracing::error!("[gui] runtime error in '{}': {}", job_label, message);
            }
            JobResult::Pong => {
                self.toast(ToastKind::Info, "pong");
            }
        }
    }

    fn draw_top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("📂 Open PDF…").clicked() {
                        if let Some(path) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file() {
                            self.open_pdf(path);
                        }
                        ui.close_menu();
                    }
                    if ui.button("⏯ Resume last session").clicked() {
                        let auto = std::path::PathBuf::from("audit").join("history.json");
                        if auto.exists() {
                            let _ = self.job_tx.send(Job::LoadHistory { input: auto.clone() });
                            self.in_flight += 1;
                            self.toast(ToastKind::Info, format!("Resuming from {}", auto.display()));
                        } else {
                            self.toast(ToastKind::Warn, "No previous session found.");
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.label("Recent:");
                    let recent = self.settings.recent_files.clone();
                    for f in recent {
                        let label = if f.len() > 40 { format!("…{}", &f[f.len() - 38..]) } else { f.clone() };
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

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_source("theme_picker")
                        .selected_text(self.settings.theme.label())
                        .width(100.0)
                        .show_ui(ui, |ui| {
                            for t in [Theme::Midnight, Theme::Dark, Theme::Light, Theme::Solarized] {
                                ui.selectable_value(&mut self.settings.theme, t, t.label());
                            }
                        });
                    if let Some((label, fraction)) = &self.progress {
                        ui.add(egui::ProgressBar::new(*fraction).text(label).desired_width(220.0));
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
            ui.horizontal(|ui| {
                ui.small(&self.status);
                ui.separator();
                if self.total_pages > 0 {
                    ui.small(format!("Page {}/{}", self.current_page + 1, self.total_pages));
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
                    ui.checkbox(&mut self.settings.deep_font_replication, "Deep Font Replication (AI)");
                    if ui.button("🎯 Apply Edit")
                        .on_hover_text("Replace the selected text with the new value")
                        .clicked() {
                        let input = if self.current_pdf_path.exists() {
                            self.current_pdf_path.clone()
                        } else {
                            PathBuf::from(&self.input_path)
                        };
                        let _ = self.job_tx.send(Job::ApplyChange {
                            input,
                            output: PathBuf::from(&self.output_path),
                            page: self.current_page,
                            bbox: block.bbox,
                            new_text: self.new_text.clone(),
                            old_text: block.text.clone(),
                            description: "Manual edit".into(),
                            deep_font_replication: self.settings.deep_font_replication,
                        });
                        self.in_flight += 1;
                    }
                } else {
                    ui.weak("Click any text on the canvas to edit.");
                }
            });
    }

    fn draw_right_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("right_panel")
            .width_range(280.0..=380.0)
            .show(ctx, |ui| {
                ui.heading("Analysis & Tools");
                egui::ScrollArea::vertical().show(ui, |ui| {
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
                            ui.label(format!("Global imbalance: ${imb:.2}"));
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
                        ui.checkbox(&mut self.settings.use_pdfrest, "Adobe-tier (pdfRest)");
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

                    ui.collapsing("📤 Export", |ui| {
                        if ui.button("Excel (.xlsx)").clicked() {
                            self.export_to_excel();
                        }
                        if ui.button("Audit JSON").clicked() {
                            let _ = self.job_tx.send(Job::ExportChangeHistory {
                                output: PathBuf::from(&self.export_path),
                            });
                            self.in_flight += 1;
                        }
                    });

                    ui.collapsing("⚙ Settings", |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Theme:");
                            egui::ComboBox::from_id_source("settings_theme")
                                .selected_text(self.settings.theme.label())
                                .show_ui(ui, |ui| {
                                    for t in [Theme::Midnight, Theme::Dark, Theme::Light, Theme::Solarized] {
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
                        ui.add_space(8.0);
                        ui.label("Webhook (optional):");
                        ui.text_edit_singleline(&mut self.settings.webhook_url)
                            .on_hover_text("POST a JSON payload to this URL on each successful edit");
                        ui.label("OpenAI API key (optional fallback):");
                        ui.add(egui::TextEdit::singleline(&mut self.settings.openai_api_key).password(true))
                            .on_hover_text("Used only if Gemini fails");
                        if ui.button("Save settings").on_hover_text("Persist these settings on disk").clicked() {
                            let _ = confy::store("bank-statement-modifier", None, &self.settings);
                            self.toast(ToastKind::Success, "Settings saved");
                        }
                    });
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

                // Pan — middle mouse, or shift+drag
                if response.dragged_by(egui::PointerButton::Middle)
                    || (response.dragged() && ui.input(|i| i.modifiers.shift))
                {
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
                            let after_rect = egui::Rect::from_min_max(egui::pos2(split_x, rect.min.y), rect.max);
                            let uv_min = egui::pos2(self.curtain_ratio, 0.0);
                            painter.image(
                                after.id(),
                                after_rect,
                                egui::Rect::from_min_max(uv_min, egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE,
                            );
                            painter.line_segment(
                                [egui::pos2(split_x, rect.min.y), egui::pos2(split_x, rect.max.y)],
                                egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 200, 0)),
                            );
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
                            if self.job_tx
                                .send(Job::Python(
                                    PythonJob::FindTextBlockAtClick {
                                        pdf_path: self.current_pdf_path.to_string_lossy().to_string(),
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
                } else {
                    // Welcome / empty placeholder
                    self.draw_empty_canvas(ui, response.rect, &painter);
                }
            });
        });
    }

    fn draw_empty_canvas(&mut self, ui: &mut egui::Ui, rect: egui::Rect, painter: &egui::Painter) {
        let p = self.settings.theme.palette();
        // Subtle gradient background
        painter.rect_filled(rect, 0.0, p.bg);

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
            ui.painter().rect_stroke(rect, 8.0, egui::Stroke::new(1.0, p.accent.linear_multiply(0.6)));
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(14.0), p.text);
            resp.clone().on_hover_text(hint).clicked()
        };
        if button(ui, "📂 Open PDF…", "Browse for a bank statement PDF") {
            if let Some(path) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file() {
                self.open_pdf(path);
            }
        }
        if button(ui, "⏯ Resume last session", "Reload the last autosaved history") {
            let auto = std::path::PathBuf::from("audit").join("history.json");
            if auto.exists() {
                let _ = self.job_tx.send(Job::LoadHistory { input: auto.clone() });
                self.in_flight += 1;
                self.toast(ToastKind::Info, format!("Resuming from {}", auto.display()));
            } else {
                self.toast(ToastKind::Warn, "No previous session found.");
            }
        }
        if !self.settings.recent_files.is_empty()
            && button(ui, "📜 Open most recent", &format!("Open {}", self.settings.recent_files[0]))
        {
            let path = PathBuf::from(self.settings.recent_files[0].clone());
            self.open_pdf(path);
        }
    }

    fn draw_toasts(&mut self, ctx: &egui::Context) {
        // Drop expired
        let now = Instant::now();
        self.toasts.retain(|t| t.expires_at > now);
        if self.toasts.is_empty() {
            return;
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
                            bg.r(), bg.g(), bg.b(), (alpha * 230.0) as u8,
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
                                });
                            });
                        ui.add_space(6.0);
                    }
                });
            });
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let input = ctx.input(|i| i.clone());
        if input.modifiers.command && input.key_pressed(egui::Key::O) {
            if let Some(path) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).pick_file() {
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
            self.toast(ToastKind::Error, format!("File not found: {}", path.display()));
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
        let _ = self.job_tx.send(Job::LoadDocument { path: self.current_pdf_path.clone() });
        self.in_flight += 1;
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn run_gui(
    job_tx: std::sync::mpsc::Sender<Job>,
    job_rx: std::sync::mpsc::Receiver<JobResult>,
    config: std::sync::Arc<crate::app::config::AppConfig>,
) -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([960.0, 640.0])
            .with_title("Bank Statement Fidelity Editor v0.4.0"),
        ..Default::default()
    };

    eframe::run_native(
        "Bank Statement Fidelity Editor",
        options,
        Box::new(move |_cc| Ok(Box::new(MyApp::new(job_tx, job_rx, config.clone())))),
    )
}

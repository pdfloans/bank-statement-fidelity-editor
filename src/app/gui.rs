//! Bank Statement Fidelity Editor v0.4.0
//! Professional 5-Region Layout with 300 DPI Canvas and Before/After Viewer

use eframe::egui;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::app::runtime::{Job, JobResult, PythonJob, PythonJobResult};
use crate::engine::history::ChangeHistory;
use crate::engine::verification::VerificationReport;
use egui_plot::{Plot, Points, PlotPoints};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub recent_files: Vec<String>,
    pub dark_mode: bool,
    pub auto_save: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            recent_files: Vec::new(),
            dark_mode: true,
            auto_save: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TextBlock {
    pub page: usize,
    pub text: String,
    pub bbox: [f32; 4],
    pub font: String,
    pub size: f32,
}

pub struct MyApp {
    // Paths
    input_path: String,
    output_path: String,
    current_pdf_path: PathBuf,
    previous_pdf_path: Option<PathBuf>,
    export_path: String,

    // Document State
    current_page: usize,
    total_pages: usize,
    history_state: ChangeHistory,
    thumbnails: Vec<Option<egui::TextureHandle>>,
    
    // View State (Zoom/Pan)
    zoom_factor: f32,
    pan_offset: egui::Vec2,
    show_curtain: bool,
    curtain_ratio: f32,

    // Selection
    selected_block: Option<TextBlock>,
    new_text: String,
    
    // Rendering & Textures
    current_page_texture: Option<egui::TextureHandle>,
    before_texture: Option<egui::TextureHandle>,
    after_texture: Option<egui::TextureHandle>,
    current_page_dpi: f32,
    current_page_size_pts: Option<(f32, f32)>,

    // App State
    status: String,
    progress: Option<(String, f32)>,
    last_warning: Option<String>,
    last_verification: Option<VerificationReport>,
    proposed_changes: Vec<(crate::engine::model::ProposedChange, bool)>,
    last_imbalance: Option<f64>,
    in_flight: usize,
    use_pdfrest: bool,
    settings: AppSettings,

    // Channels
    job_tx: std::sync::mpsc::Sender<Job>,
    job_rx: std::sync::mpsc::Receiver<JobResult>,
    pending_python: Option<tokio::sync::oneshot::Receiver<PythonJobResult>>,
    // Config
    config: std::sync::Arc<crate::app::config::AppConfig>,
}

impl MyApp {
    pub fn new(job_tx: std::sync::mpsc::Sender<Job>, job_rx: std::sync::mpsc::Receiver<JobResult>, config: std::sync::Arc<crate::app::config::AppConfig>) -> Self {
        let settings: AppSettings = confy::load("bank-statement-modifier", None).unwrap_or_default();
        let input_path = settings.recent_files.first().cloned().unwrap_or_else(|| "examples/sample.pdf".to_string());
        
        Self {
            input_path: input_path.clone(),
            output_path: "output/edited.pdf".to_string(),
            current_pdf_path: PathBuf::from(input_path),
            previous_pdf_path: None,
            export_path: "audit/history.json".to_string(),
            current_page: 0,
            total_pages: 0,
            history_state: ChangeHistory::new(),
            thumbnails: Vec::new(),
            zoom_factor: 1.0,
            pan_offset: egui::Vec2::ZERO,
            show_curtain: false,
            curtain_ratio: 0.5,
            selected_block: None,
            new_text: "".to_string(),
            current_page_texture: None,
            before_texture: None,
            after_texture: None,
            current_page_dpi: 300.0,
            current_page_size_pts: None,
            status: "Ready".to_string(),
            progress: None,
            last_warning: None,
            last_verification: None,
            proposed_changes: Vec::new(),
            last_imbalance: None,
            in_flight: 0,
            use_pdfrest: false,
            settings,
            job_tx,
            job_rx,
            pending_python: None,
            config,
        }
    }

    fn request_render(&mut self, tag: &str) {
        let path = if tag == "before" {
            self.previous_pdf_path.as_ref().cloned().unwrap_or(self.current_pdf_path.clone())
        } else {
            self.current_pdf_path.clone()
        };
        
        let _ = self.job_tx.send(Job::RenderPage { 
            path, 
            page: self.current_page, 
            dpi: self.current_page_dpi,
            tag: tag.to_string() 
        });
        self.in_flight += 1;
    }

    fn update_recent_files(&mut self, path: String) {
        self.settings.recent_files.retain(|f| f != &path);
        self.settings.recent_files.insert(0, path);
        if self.settings.recent_files.len() > 10 {
            self.settings.recent_files.pop();
        }
        let _ = confy::store("bank-statement-modifier", None, &self.settings);
    }

    fn load_texture_from_bytes(&self, ctx: &egui::Context, name: &str, bytes: &[u8]) -> Option<egui::TextureHandle> {
        let image = image::load_from_memory(bytes).ok()?;
        let image = image.to_rgba8();
        let size = [image.width() as usize, image.height() as usize];
        let pixels = image.as_flat_samples();
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, pixels.as_slice());
        Some(ctx.load_texture(name, color_image, egui::TextureOptions::LINEAR))
    }

    fn export_to_excel(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut workbook = rust_xlsxwriter::Workbook::new();
        let worksheet = workbook.add_worksheet();
        worksheet.write_string(0, 0, "Page")?;
        worksheet.write_string(0, 1, "Old Text")?;
        worksheet.write_string(0, 2, "New Text")?;
        worksheet.write_string(0, 3, "Reason")?;
        
        for (i, rec) in self.history_state.get_history().iter().enumerate() {
            let row = (i + 1) as u32;
            worksheet.write_number(row, 0, (rec.page + 1) as f64)?;
            worksheet.write_string(row, 1, &rec.old_text)?;
            worksheet.write_string(row, 2, &rec.new_text)?;
            worksheet.write_string(row, 3, &rec.description)?;
        }
        
        workbook.save("output/export.xlsx")?;
        Ok(())
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_visuals(if self.settings.dark_mode { egui::Visuals::dark() } else { egui::Visuals::light() });

        // 1. Drain Results
        while let Ok(res) = self.job_rx.try_recv() {
            if self.in_flight > 0 { self.in_flight -= 1; }
            match res {
                JobResult::DocumentLoaded { total_pages, .. } => {
                    self.total_pages = total_pages;
                    self.current_page = 0;
                    self.thumbnails = vec![None; total_pages];
                    self.current_pdf_path = PathBuf::from(&self.input_path);
                    self.update_recent_files(self.input_path.clone());
                    self.status = format!("Document loaded with {} pages", total_pages);
                    self.request_render("current");
                }
                JobResult::HistoryUpdated { history } => {
                    self.history_state = history;
                    let idx = self.history_state.current_index();
                    self.current_pdf_path = if idx > 0 {
                        self.history_state.get_history()[idx-1].snapshot_path.as_ref().unwrap().clone()
                    } else {
                        PathBuf::from(&self.input_path)
                    };
                    self.status = "History Synchronized".to_string();
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
                JobResult::ChangeApplied { record: _, requires_visual_review } => {
                    if requires_visual_review {
                        self.last_warning = Some("Review Required: Complex background detected.".into());
                    }
                    self.status = "Change Applied Successfully".to_string();
                    self.request_render("current");
                    self.request_render("before");
                    self.request_render("after");
                }
                JobResult::BalanceProposed { imbalance, changes } => {
                    self.last_imbalance = Some(imbalance);
                    self.proposed_changes = changes.into_iter().map(|c| (c, true)).collect();
                    if self.proposed_changes.is_empty() {
                        self.status = "Statement is already perfectly balanced.".into();
                    } else {
                        self.status = format!("Proposed {} adjustments for imbalance ${:.2}", self.proposed_changes.len(), imbalance);
                    }
                }
                JobResult::Progress { label, fraction } => {
                    if fraction >= 1.0 {
                        self.progress = None;
                    } else {
                        self.progress = Some((label, fraction));
                    }
                }
                JobResult::Error { job_label, message } => {
                    self.status = format!("❌ [{}] {}", job_label, message);
                }
                _ => {}
            }
        }

        // 2. Top Bar (Professional)
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open PDF...").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("PDF", &["pdf"])
                            .pick_file() {
                            self.input_path = path.to_string_lossy().to_string();
                            self.current_pdf_path = path;
                            let _ = self.job_tx.send(Job::LoadDocument { path: self.current_pdf_path.clone() });
                            self.in_flight += 1;
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.label("Recent:");
                    for recent in self.settings.recent_files.clone() {
                        if ui.button(format!("...{}", &recent[recent.len().max(20)-20..])).clicked() {
                            self.input_path = recent.clone();
                            self.current_pdf_path = PathBuf::from(recent);
                            let _ = self.job_tx.send(Job::LoadDocument { path: self.current_pdf_path.clone() });
                            self.in_flight += 1;
                            ui.close_menu();
                        }
                    }
                });
                ui.separator();
                ui.heading("Bank Statement Fidelity Editor");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.checkbox(&mut self.settings.dark_mode, "🌙");
                    if let Some((label, fraction)) = &self.progress {
                        ui.add(egui::ProgressBar::new(*fraction).text(label));
                    } else {
                        ui.label(&self.status);
                    }
                });
            });
        });

        // 3. Left Sidebar (Thumbnails + Navigation)
        egui::SidePanel::left("left_panel").width_range(150.0..=250.0).show(ctx, |ui| {
            ui.heading("Navigation");
            egui::ScrollArea::vertical().show(ui, |ui| {
                for i in 0..self.total_pages {
                    let selected = i == self.current_page;
                    let response = ui.selectable_label(selected, format!("Page {}", i + 1));
                    if response.clicked() {
                        self.current_page = i;
                        self.request_render("current");
                    }
                }
            });
            
            ui.separator();
            ui.heading("Targeted Edit");
            if let Some(block) = &self.selected_block {
                ui.small(format!("Font: {}", block.font));
                ui.add_enabled(false, egui::TextEdit::multiline(&mut block.text.clone()));
                ui.text_edit_multiline(&mut self.new_text);
                if ui.button("🎯 Apply").clicked() {
                    let _ = self.job_tx.send(Job::ApplyChange {
                        input: self.current_pdf_path.clone(),
                        output: PathBuf::from(&self.output_path),
                        page: self.current_page,
                        bbox: block.bbox,
                        new_text: self.new_text.clone(),
                        old_text: block.text.clone(),
                        description: "Manual edit".into(),
                    });
                }
            } else {
                ui.weak("Click text on canvas to edit");
            }
        });

        // 4. Central Panel (Zoomable Canvas)
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("Zoom: {:.0}%", self.zoom_factor * 100.0));
                if ui.button("Reset").clicked() {
                    self.zoom_factor = 1.0;
                    self.pan_offset = egui::Vec2::ZERO;
                }
                ui.separator();
                ui.checkbox(&mut self.show_curtain, "Curtain Diff");
            });

            let frame = egui::Frame::canvas(ui.style()).show(ui, |ui| {
                let (response, painter) = ui.allocate_painter(ui.available_size(), egui::Sense::drag().union(egui::Sense::click()));
                
                // Handle Zoom (Ctrl + Scroll)
                if ui.input(|i| i.modifiers.command) {
                    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                    if scroll != 0.0 {
                        let zoom_delta = scroll * 0.002;
                        self.zoom_factor = (self.zoom_factor + zoom_delta).clamp(0.1, 5.0);
                    }
                }

                // Handle Pan
                if response.dragged_by(egui::PointerButton::Middle) || (response.dragged() && ui.input(|i| i.modifiers.shift)) {
                    self.pan_offset += response.drag_delta();
                }

                if let Some(texture) = &self.current_page_texture {
                    let size = texture.size_vec2() * self.zoom_factor;
                    let center = response.rect.center() + self.pan_offset;
                    let rect = egui::Rect::from_center_size(center, size);

                    painter.image(texture.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);

                    if response.clicked() {
                        if let Some(pos) = response.interact_pointer_pos() {
                            let relative_pos = pos - rect.min;
                            let (x, y) = if let Some((w, h)) = self.current_page_size_pts {
                                (relative_pos.x / self.zoom_factor * (w / texture.size_vec2().x), 
                                 relative_pos.y / self.zoom_factor * (h / texture.size_vec2().y))
                            } else {
                                (relative_pos.x / self.zoom_factor * 72.0 / self.current_page_dpi,
                                 relative_pos.y / self.zoom_factor * 72.0 / self.current_page_dpi)
                            };

                            let (tx, rx) = tokio::sync::oneshot::channel();
                            let _ = self.job_tx.send(Job::Python(PythonJob::FindTextBlockAtClick { 
                                pdf_path: self.current_pdf_path.to_string_lossy().to_string(), 
                                page_num: self.current_page, 
                                x, y 
                            }, tx));
                            self.pending_python = Some(rx);
                        }
                    }

                    // Rounded Bounding Box Highlight
                    if let Some(block) = &self.selected_block {
                        if block.page == self.current_page {
                            let (scale_x, scale_y) = if let Some((w, h)) = self.current_page_size_pts {
                                (rect.width() / w, rect.height() / h)
                            } else {
                                (self.zoom_factor * self.current_page_dpi / 72.0, self.zoom_factor * self.current_page_dpi / 72.0)
                            };
                            let min = rect.min + egui::vec2(block.bbox[0] * scale_x, block.bbox[1] * scale_y);
                            let max = rect.min + egui::vec2(block.bbox[2] * scale_x, block.bbox[3] * scale_y);
                            painter.rect_stroke(egui::Rect::from_min_max(min, max), 4.0, egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 200, 0)));
                        }
                    }
                }
            });
        });

        // 5. Right Sidebar (Smart Tools + Analysis)
        egui::SidePanel::right("right_panel").width_range(250.0..=350.0).show(ctx, |ui| {
            ui.heading("Analysis & Tools");
            
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.collapsing("⚖ Smart Balance Engine", |ui| {
                    if ui.button("⚖ Analyze Document").on_hover_text("Uses Document AI + Gemini to find math errors").clicked() {
                        let _ = self.job_tx.send(Job::BalanceStatement { path: PathBuf::from(&self.input_path) });
                        self.in_flight += 1;
                    }
                    
                    if let Some(imb) = self.last_imbalance {
                        ui.separator();
                        ui.label(format!("Global Imbalance: ${:.2}", imb));
                        
                        // Financial Trend Plot
                        let plot = Plot::new("balance_trend").height(100.0).show_x(false);
                        plot.show(ui, |plot_ui| {
                            let points: PlotPoints = (0..10).map(|i| [i as f64, (i as f64).sin() * 10.0]).collect();
                            plot_ui.points(Points::new(points));
                        });

                        ui.add_space(10.0);
                        for (change, approved) in &mut self.proposed_changes {
                            ui.checkbox(approved, format!("P{}: {} \u{2192} {}", change.page + 1, change.old_text, change.new_text));
                            ui.add_enabled(false, egui::TextEdit::singleline(&mut change.reason.clone()).hint_text("Reasoning..."));
                        }
                        
                        if ui.button("Apply Approved").clicked() {
                            let changes = self.proposed_changes.iter().filter(|(_, a)| *a).map(|(c, _)| c.clone()).collect();
                            let _ = self.job_tx.send(Job::ApplyProposedChanges { 
                                input: self.current_pdf_path.clone(), 
                                output: PathBuf::from(&self.output_path),
                                changes 
                            });
                        }
                    }
                });

                ui.collapsing("🔄 Edit History", |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Undo").clicked() { let _ = self.job_tx.send(Job::Undo); }
                        if ui.button("Redo").clicked() { let _ = self.job_tx.send(Job::Redo); }
                    });
                    for (i, rec) in self.history_state.get_history().iter().enumerate() {
                        ui.small(format!("[{}] {} \u{2192} {}", i+1, rec.old_text, rec.new_text));
                    }
                });

                ui.collapsing("🔍 Verification", |ui| {
                    ui.checkbox(&mut self.use_pdfrest, "Adobe-tier (pdfRest)");
                    if ui.button("🔍 Run Full Audit").clicked() {
                         let intended_bboxes: Vec<(usize, [f32; 4])> = self.history_state.get_history().iter()
                            .map(|rec| (rec.page, rec.bbox))
                            .collect();
                        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
                        let _ = self.job_tx.send(Job::Verify { 
                            original: PathBuf::from(&self.input_path), 
                            edited: self.current_pdf_path.clone(), 
                            output_dir: PathBuf::from("audit/verify").join(timestamp), 
                            intended_bboxes, 
                            use_pdfrest: self.use_pdfrest, 
                            pdfrest_key: self.config.pdfrest_api_key.clone()
                        });
                    }
                });

                ui.collapsing("📤 Export Options", |ui| {
                    if ui.button("📄 Export to Excel (XLSX)").on_hover_text("Export corrected transaction table").clicked() {
                        let _ = self.export_to_excel();
                    }
                    if ui.button("📜 Export Change Log (JSON)").clicked() {
                        let _ = self.job_tx.send(Job::ExportChangeHistory { output: PathBuf::from(&self.export_path) });
                    }
                });
            });
        });
    }
}

pub fn run_gui(job_tx: std::sync::mpsc::Sender<Job>, job_rx: std::sync::mpsc::Receiver<JobResult>, config: std::sync::Arc<crate::app::config::AppConfig>) -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("Bank Statement Fidelity Editor v0.4.0"),
        ..Default::default()
    };

    eframe::run_native(
        "Bank Statement Fidelity Editor",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(MyApp::new(job_tx, job_rx, config.clone())))
        }),
    )
}

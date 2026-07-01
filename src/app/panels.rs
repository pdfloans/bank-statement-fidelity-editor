use crate::app::gui::{AppView, MyApp, Toast, ToastKind};
use crate::app::theme::Theme;
use crate::engine::font_analysis::*;
use crate::engine::model::*;
use crate::engine::workflow::*;
use egui::*;

use crate::app::runtime::{Job, PythonJob};
use std::path::PathBuf;


pub(crate) trait AppPanels {
    fn draw_top_bar(&mut self, ctx: &egui::Context);
    fn draw_status_bar(&self, ctx: &egui::Context);
    fn draw_left_panel(&mut self, ctx: &egui::Context);
    fn draw_batch_panel(&mut self, ctx: &egui::Context);
    fn draw_central_panel(&mut self, ctx: &egui::Context);
    fn draw_audit_explorer_view(&mut self, ctx: &egui::Context);
    fn draw_workflow_section(&mut self, ui: &mut egui::Ui);
    fn draw_workflow_edit_table(&mut self, ui: &mut egui::Ui);
    fn draw_font_analysis_section(&mut self, ui: &mut egui::Ui);
    fn draw_empty_canvas(&mut self, ui: &mut egui::Ui, rect: egui::Rect, painter: &egui::Painter);

}

impl AppPanels for MyApp {
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
                        let pairs = Self::pair_originals_and_edited(&self.batch_files);
                        if pairs.is_empty() {
                            self.toast(ToastKind::Warn, "No paired _original/_edited PDFs found in folder.");
                        } else {
                            let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
                            for (original, edited) in &pairs {
                                let stem = edited.file_stem().unwrap_or_default().to_string_lossy().to_string();
                                let _ = self.job_tx.send(Job::Verify {
                                    original: original.clone(),
                                    edited: edited.clone(),
                                    output_dir: PathBuf::from("audit/verify/batch").join(&timestamp).join(&stem),
                                    intended_bboxes: Vec::new(),
                                    use_pdfrest: self.settings.use_pdfrest,
                                    pdfrest_key: self.config.pdfrest_api_key.clone(),
                                });
                                self.in_flight += 1;
                            }
                            self.toast(ToastKind::Info, format!("Queued {} verification job(s)", pairs.len()));
                        }
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
                                                        original_transactions: self.workflow_transactions.clone(),
                                                        opening_balance: self.workflow_validation.as_ref().map(|v| v.opening_balance).unwrap_or_default(),
                                                        expected_closing: self.workflow_validation.as_ref().and_then(|v| {
                                                            if v.closing_balance.abs() > rust_decimal::Decimal::ZERO {
                                                                Some(v.closing_balance)
                                                            } else {
                                                                None
                                                            }
                                                        }),
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
                        input: std::path::PathBuf::from(&self.input_path),
                        output: std::path::PathBuf::from(&self.output_path),
                        edits: edits_to_apply,
                        original_transactions: self.workflow_transactions.clone(),
                        opening_balance: self.workflow_validation.as_ref().map(|v| v.opening_balance).unwrap_or_default(),
                        expected_closing: self.workflow_validation.as_ref().and_then(|v| {
                            if v.closing_balance.abs() > rust_decimal::Decimal::ZERO {
                                Some(v.closing_balance)
                            } else {
                                None
                            }
                        }),
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

}

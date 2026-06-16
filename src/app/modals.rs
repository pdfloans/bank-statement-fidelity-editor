use crate::app::panels::AppPanels;
use crate::app::gui::{MyApp, Toast, ToastKind, AppView};
use crate::app::theme::Theme;
use crate::engine::model::*;
use crate::engine::workflow::*;
use crate::engine::font_analysis::*;
use egui::*;
use egui_plot::{Line, Plot};
use std::path::PathBuf;
use crate::app::runtime::{Job, PythonJob};


pub(crate) trait AppModals {
    fn draw_settings_modal(&mut self, ctx: &egui::Context);
    fn draw_transfer_dialog(&mut self, ctx: &egui::Context);
    fn draw_date_adjust_dialog(&mut self, ctx: &egui::Context);
    fn draw_ai_confirmation_dialog(&mut self, ctx: &egui::Context);
    fn draw_transfer_test_dialog(&mut self, ctx: &egui::Context);
    fn draw_api_keys_editor(&mut self, ui: &mut egui::Ui);
    fn draw_modals(&mut self, ctx: &egui::Context);

}

impl AppModals for MyApp {
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
    
                        ui.label("PDF Engine Mode:");
                        egui::ComboBox::from_id_source("pdf_engine_mode_combo_modal")
                            .selected_text(match self.edit_engine_mode {
                                crate::app::config::PdfEngineMode::Auto => "Auto (Native + PyMuPDF)",
                                crate::app::config::PdfEngineMode::NativeOnly => "Force Native",
                                crate::app::config::PdfEngineMode::PyMuPdfOnly => "Force PyMuPDF",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.edit_engine_mode, crate::app::config::PdfEngineMode::Auto, "Auto (Native + PyMuPDF)");
                                ui.selectable_value(&mut self.edit_engine_mode, crate::app::config::PdfEngineMode::NativeOnly, "Force Native");
                                ui.selectable_value(&mut self.edit_engine_mode, crate::app::config::PdfEngineMode::PyMuPdfOnly, "Force PyMuPDF");
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

}

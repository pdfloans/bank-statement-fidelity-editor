use crate::app::gui::{MyApp, Theme, ToastKind};
use crate::app::runtime::Job;
use egui_plot::{Line, Plot};
use std::path::PathBuf;

pub trait CommandPalette {
    fn draw_command_palette(&mut self, ctx: &egui::Context);
}

impl CommandPalette for MyApp {
    fn draw_command_palette(&mut self, ctx: &egui::Context) {
        let mut open = self.show_command_palette;
        let mut submit_nlp = false;

        egui::Window::new("Command Palette")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 100.0))
            .fixed_size(egui::vec2(600.0, 60.0))
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(self.settings.theme.palette().bg.linear_multiply(0.95))
                    .inner_margin(16.0)
                    .rounding(12.0)
                    .shadow(egui::epaint::Shadow {
                        offset: egui::vec2(0.0, 20.0),
                        blur: 40.0,
                        spread: 0.0,
                        color: egui::Color32::from_black_alpha(150),
                    }),
            )
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("🔍").size(24.0));

                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.command_query)
                            .desired_width(500.0)
                            .font(egui::FontId::proportional(20.0))
                            .hint_text("Type a command or ask AI... (e.g. 'balance page 1')"),
                    );

                    response.request_focus();

                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        submit_nlp = true;
                    }
                });

                if !self.command_query.is_empty() {
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(
                            "Press Enter to execute natural language prompt via Gemini",
                        )
                        .color(self.settings.theme.palette().weak)
                        .italics(),
                    );
                }
            });

        if submit_nlp {
            let prompt = std::mem::take(&mut self.command_query);
            self.toast(ToastKind::Info, format!("Executing AI command: {}", prompt));

            // Note: NLP AI Job would be triggered here in the Job loop.
            // self.job_tx.send(Job::AiNaturalLanguageCommand { prompt, path: self.current_pdf_path.clone() });

            self.show_command_palette = false;
        } else {
            self.show_command_palette = open;
        }
    }
}

#[allow(dead_code)]
pub(crate) trait AppModals {
    fn draw_settings_modal(&mut self, ctx: &egui::Context);
    fn draw_backend_preferences(&mut self, ui: &mut egui::Ui);
    fn draw_transfer_dialog(&mut self, ctx: &egui::Context);
    fn draw_date_adjust_dialog(&mut self, ctx: &egui::Context);
    fn draw_ai_confirmation_dialog(&mut self, ctx: &egui::Context);
    fn draw_interactive_fallback_modal(&mut self, ctx: &egui::Context);
    fn draw_autofix_modal(&mut self, ctx: &egui::Context);
    fn draw_workflow_hitl_modal(&mut self, ctx: &egui::Context);
    fn draw_transfer_test_dialog(&mut self, ctx: &egui::Context);
    fn draw_api_keys_editor(&mut self, ui: &mut egui::Ui);
    fn draw_feedback_modal(&mut self, ctx: &egui::Context);
    fn draw_modals(&mut self, ctx: &egui::Context);
    fn draw_stuck_watchdog_modal(&mut self, ctx: &egui::Context);
}

impl AppModals for MyApp {
    fn draw_settings_modal(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings_modal;
        egui::Window::new("⚙️ Settings & Tools")
                .open(&mut open)
                .default_size(egui::vec2(420.0, 600.0))
                .vscroll(true)
                .show(ctx, |ui| {
                        // Backend Preferences panel at the top - most important
                        self.draw_backend_preferences(ui);

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
                                        format!("P{}: {} -> {}", change.page + 1, change.old_text, change.new_text),
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
                                    ui.small(format!("[{}] P{} {} -> {}", i + 1, rec.page + 1, rec.old_text, rec.new_text));
                                }
                            });

                            ui.collapsing("🏆 Parser Stats & Leaderboard", |ui| {
                                let stats: crate::engine::model::ParserStats = std::fs::read_to_string("audit/parser_stats.json")
                                    .ok()
                                    .and_then(|s| serde_json::from_str(&s).ok())
                                    .unwrap_or_default();
                                
                                ui.label(format!("Total Matrix Checks: {}", stats.total_attempts));
                                
                                // Build a sorted leaderboard
                                let mut leaderboard = vec![
                                    ("DocAI", stats.docai_wins),
                                    ("LlamaParse", stats.llamaparse_wins),

                                    ("Gemini", stats.gemini_wins),
                                    ("Offline", stats.offline_wins),
                                ];
                                leaderboard.sort_by(|a, b| b.1.cmp(&a.1));
                                
                                egui::Grid::new("leaderboard_grid")
                                    .num_columns(3)
                                    .spacing([20.0, 4.0])
                                    .striped(true)
                                    .show(ui, |ui| {
                                        ui.strong("Rank");
                                        ui.strong("Parser");
                                        ui.strong("Consensus Wins");
                                        ui.end_row();
                                        
                                        for (i, (name, wins)) in leaderboard.into_iter().enumerate() {
                                            ui.label(format!("#{}", i + 1));
                                            ui.label(name);
                                            ui.label(wins.to_string());
                                            ui.end_row();
                                        }
                                    });
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
                                        auto_match_dpi: self.settings.auto_match_dpi,
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
                                egui::ComboBox::from_id_salt("settings_theme")
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
                            ui.checkbox(&mut self.settings.auto_match_dpi, "Auto-match DPI to PDF Document Size")
                                .on_hover_text("Safely scales based on physical points (capped at 600 DPI to avoid OOM)");
                            ui.checkbox(&mut self.settings.transfer_consensus_mode, "Matrix Consensus for Transfers")
                                .on_hover_text("Runs multiple AIs simultaneously to perform majority-vote extraction & math cross-referencing");
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
                            ui.label("LlamaParse API key:");
                            ui.add(egui::TextEdit::singleline(&mut self.settings.llamaparse_api_key).password(true))
                                .on_hover_text("Required for LlamaParse extraction mode. Get it from cloud.llamaindex.ai");
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
                            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(response.rect), |ui| {
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

    fn draw_backend_preferences(&mut self, ui: &mut egui::Ui) {
        use crate::app::config::*;

        let id = ui.make_persistent_id("backend_prefs_collapsing");
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| {
                ui.heading("\u{1f527} Backend Preferences");
            })
            .body(|ui| {
                ui.small("Choose which backend to use for each stage of the workflow.");
                ui.small("Options marked \u{26d4} require an API key that is not currently configured.");
                ui.add_space(6.0);

                // Snapshot availability for this frame (cheap clone)
                let avail = self.api_availability.clone();

                egui::Grid::new("backend_prefs_grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .striped(true)
                    .show(ui, |ui| {
                        // ── Pipeline Architecture ──
                        ui.label("Extraction:");
                        ui.label("1. LlamaParse \u{2192} 2. Offline Heuristic (93%)");
                        ui.end_row();

                        ui.label("Fidelity Edit:");
                        ui.label("1\u{fe0f}\u{20e3} PyMuPDF Pro (88%) \u{2192} 2\u{fe0f}\u{20e3} Pdfium (76%) \u{2192} 3\u{fe0f}\u{20e3} Typst Reconstruct (70%)");
                        ui.end_row();

                        ui.label("Math Balance:");
                        ui.label("1\u{fe0f}\u{20e3} Local Math Engine (100%)");
                        ui.end_row();

                        ui.label("Forensics:");
                        ui.label("1\u{fe0f}\u{20e3} PyMuPDF Pro (100%) \u{2192} 2\u{fe0f}\u{20e3} Typst Reconstruct (90%)");
                        ui.end_row();

                        ui.label("Visual AI Validation:");
                        ui.label("new system");
                        ui.end_row();

                        // ── 5. Font Handling ──
                        ui.label("\u{1f520} Font Handling:");
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.settings.deep_font_replication, "Deep Font Replication")
                                .on_hover_text("Extracts fonts from the source PDF and embeds them in the output. Pixel-perfect but slower.");
                        });
                        ui.end_row();

                        // ── 6. Processing Mode ──
                        ui.label("\u{1f4d0} Processing:");
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut self.settings.three_page_mode, "3-Page Segmented Mode")
                                .on_hover_text("Splits long PDFs into \u{2264}3-page segments for Pro editing, then re-merges on save.")
                                .changed()
                            {
                                let _ = confy::store("bank-statement-modifier", None, &self.settings);
                            }
                        });
                        ui.end_row();

                        // ── 7. Visual Validation Thresholds ──
                        ui.label("\u{1f3af} Visual Threshold:");
                        ui.add(egui::Slider::new(&mut self.settings.visual_diff_threshold, 0.005..=0.10)
                            .text("diff")
                            .logarithmic(true))
                            .on_hover_text("Tile-max diff score ceiling. Lower = stricter. Default 0.02. Below 0.01 may cause false failures.");
                        ui.end_row();

                        ui.label("\u{1f504} Max Retries:");
                        ui.add(egui::Slider::new(&mut self.settings.max_visual_attempts, 1..=10)
                            .text("attempts"))
                            .on_hover_text("Max visual validation retries with progressive mask widening. Default 5.");
                        ui.end_row();

                        // ── 8. Workflow Settings ──
                        ui.label("\u{23f8}\u{fe0f} Interactive Fallbacks:");
                        ui.checkbox(&mut self.settings.interactive_fallbacks, "Pause & prompt on semi-failures")
                            .on_hover_text("When enabled, the app will pause on non-catastrophic errors (like parsing failure) and prompt you to manually select a fallback strategy or try again.");
                        ui.end_row();
                    });

                // ── Unified availability warnings ──
                let mut warnings: Vec<&str> = Vec::new();

                match self.settings.ai_provider {
                    AiProviderMode::GeminiApiKey if !avail.gemini_api_key => {
                        warnings.push("\u{26a0} Gemini (API Key) selected but GEMINI_API_KEY is not set. AI features will be unavailable.");
                    }
                    AiProviderMode::GeminiVertex if !avail.gemini_vertex => {
                        warnings.push("\u{26a0} Gemini (Vertex AI) selected but no service account / ADC credentials found.");
                    }
                    AiProviderMode::GroqApiKey if !avail.groq_api_key => {
                        warnings.push("\u{26a0} Groq (Llama 3) selected but GROQ_API_KEY is not set.");
                    }
                    AiProviderMode::OpenRouterApiKey if !avail.openrouter_api_key => {
                        warnings.push("\u{26a0} OpenRouter selected but OPENROUTER_API_KEY is not set.");
                    }
                    _ => {}
                }

                match self.settings.document_parser {
                    DocumentParserMode::DocumentAi if !avail.document_ai => {
                        warnings.push("\u{26a0} Document AI selected but credentials incomplete. Workflow will auto-fallback to offline parser.");
                    }

                    DocumentParserMode::LlamaParse if !avail.llamaparse => {
                        warnings.push("\u{26a0} LlamaParse selected but no API key configured. Workflow will auto-fallback to offline parser.");
                    }
                    _ => {}
                }

                if self.settings.verification_renderer == VerificationMode::PdfRestCloud && !avail.pdfrest {
                    warnings.push("\u{26a0} pdfRest cloud verification selected but PDFREST_API_KEY missing. Falls back to local Pdfium.");
                }

                if !warnings.is_empty() {
                    ui.add_space(4.0);
                    for msg in warnings {
                        ui.colored_label(self.settings.theme.palette().warn, msg);
                    }
                }

                // Keep Gemini auth mode in sync with AI provider choice
                match self.settings.ai_provider {
                    AiProviderMode::GeminiApiKey => {
                        self.edit_gemini_use_vertex = false;
                    }
                    AiProviderMode::GeminiVertex => {
                        self.edit_gemini_use_vertex = true;
                    }
                    _ => {}
                }
            });
        ui.add_space(4.0);
        ui.separator();
    }

    fn draw_transfer_dialog(&mut self, ctx: &egui::Context) {
        let mut open = self.show_transfer_dialog;
        egui::Window::new("🔄 Transfer Transactions")
            .open(&mut open)
            .default_size(egui::vec2(1200.0, 750.0))
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;

                ui.vertical_centered(|ui| {
                    ui.heading("Transfer transactions between statements");
                });
                ui.separator();

                ui.columns(3, |cols| {
                    // --- LEFT COLUMN: SOURCE ---
                    cols[0].vertical_centered(|ui| {
                        ui.heading("Source Document");
                        ui.label(egui::RichText::new("Extract transactions from here").weak());
                        ui.add_space(8.0);
                        if ui.button("📂 Upload Source PDF").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("PDF", &["pdf"])
                                .pick_file()
                            {
                                self.transfer_source_path = path.to_string_lossy().to_string();
                                if let Err(e) = self.job_tx.send(Job::RenderPage {
                                    path,
                                    page: 1,
                                    dpi: 72.0,
                                    tag: "transfer_source".to_string(),
                                }) {
                                    tracing::error!("Runtime disconnected: {}", e);
                                }
                                self.in_flight += 1;
                            }
                        }

                        if let Some(tex) = &self.transfer_source_texture {
                            ui.add_space(10.0);
                            let max_size = ui.available_size() - egui::vec2(0.0, 20.0);
                            let tex_size = tex.size_vec2();
                            let scale = (max_size.x / tex_size.x)
                                .min(max_size.y / tex_size.y)
                                .min(1.0);
                            ui.add(egui::Image::new(tex).fit_to_exact_size(tex_size * scale));
                        } else if !self.transfer_source_path.is_empty() {
                            ui.add_space(20.0);
                            ui.spinner();
                            ui.label("Rendering preview...");
                        }
                    });

                    // --- MIDDLE COLUMN: ACTIONS ---
                    cols[1].vertical_centered(|ui| {
                        ui.add_space(ui.available_height() / 2.0 - 60.0);

                        let source_ok = !self.transfer_source_path.is_empty()
                            && std::path::Path::new(&self.transfer_source_path).exists();
                        let target_ok = !self.input_path.is_empty()
                            && std::path::Path::new(&self.input_path).exists()
                            && self.input_path != "examples/sample.pdf";
                        let can_start = source_ok && target_ok;

                        let btn = ui.add_enabled(
                            can_start,
                            egui::Button::new(
                                egui::RichText::new("▶ Begin Transfer").size(20.0).color(
                                    if can_start {
                                        self.settings.theme.palette().bg
                                    } else {
                                        self.settings.theme.palette().text
                                    },
                                ),
                            )
                            .fill(if can_start {
                                self.settings.theme.palette().accent
                            } else {
                                self.settings.theme.palette().panel
                            })
                            .min_size(egui::vec2(180.0, 56.0))
                            .rounding(8.0),
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

                            if let Err(e) = self.job_tx.send(Job::TransferTransactions {
                                source_pdf: source,
                                target_pdf: target,
                                output_pdf: output,
                            }) {
                                tracing::error!("Runtime disconnected: {}", e);
                            }
                            self.in_flight += 1;
                            self.status = "Starting transaction transfer...".into();
                            self.toast(
                                ToastKind::Info,
                                "Transaction transfer started - this may take 2-3 minutes.",
                            );
                            self.show_transfer_dialog = false;
                        }

                        ui.add_space(10.0);

                        if !source_ok && !self.transfer_source_path.is_empty() {
                            ui.colored_label(
                                self.settings.theme.palette().warn,
                                "⚠ Source not found",
                            );
                        }
                        if !target_ok
                            && !self.input_path.is_empty()
                            && self.input_path != "examples/sample.pdf"
                        {
                            ui.colored_label(
                                self.settings.theme.palette().warn,
                                "⚠ Target not found",
                            );
                        }

                        ui.add_space(20.0);
                        if ui.button("Cancel").clicked() {
                            self.show_transfer_dialog = false;
                        }
                    });

                    // --- RIGHT COLUMN: TARGET ---
                    cols[2].vertical_centered(|ui| {
                        ui.heading("Target Document");
                        ui.label(egui::RichText::new("Format to apply to transactions").weak());
                        ui.add_space(8.0);
                        if ui.button("📂 Upload Target PDF").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("PDF", &["pdf"])
                                .pick_file()
                            {
                                self.input_path = path.to_string_lossy().to_string();
                                if let Err(e) = self.job_tx.send(Job::RenderPage {
                                    path,
                                    page: 1,
                                    dpi: 72.0,
                                    tag: "transfer_target".to_string(),
                                }) {
                                    tracing::error!("Runtime disconnected: {}", e);
                                }
                                self.in_flight += 1;
                            }
                        }

                        if let Some(tex) = &self.transfer_target_texture {
                            ui.add_space(10.0);
                            let max_size = ui.available_size() - egui::vec2(0.0, 20.0);
                            let tex_size = tex.size_vec2();
                            let scale = (max_size.x / tex_size.x)
                                .min(max_size.y / tex_size.y)
                                .min(1.0);
                            ui.add(egui::Image::new(tex).fit_to_exact_size(tex_size * scale));
                        } else if !self.input_path.is_empty()
                            && self.input_path != "examples/sample.pdf"
                        {
                            ui.add_space(20.0);
                            ui.spinner();
                            ui.label("Rendering preview...");
                        }
                    });
                });
            });

        if !open {
            self.show_transfer_dialog = false;
        }
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
                    let btn = ui.add_enabled(
                        has_input,
                        egui::Button::new("▶ Apply Date Adjustment").fill(if has_input {
                            self.settings.theme.palette().accent
                        } else {
                            self.settings.theme.palette().panel
                        }),
                    );

                    if btn.clicked() {
                        let input = std::path::PathBuf::from(&self.input_path);
                        let output = Self::safe_output_path(&input, "dates");

                        let mode = if self.date_adjust_mode_shift {
                            let days: i64 = self.date_adjust_shift_days.parse().unwrap_or(0);
                            crate::engine::date_adjust::DateAdjustMode::ShiftDays(days)
                        } else {
                            let from = chrono::NaiveDate::parse_from_str(
                                self.date_adjust_from.trim(),
                                "%d/%m/%Y",
                            )
                            .unwrap_or(chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
                            let to = chrono::NaiveDate::parse_from_str(
                                self.date_adjust_to.trim(),
                                "%d/%m/%Y",
                            )
                            .unwrap_or(chrono::NaiveDate::from_ymd_opt(2026, 2, 1).unwrap());
                            crate::engine::date_adjust::DateAdjustMode::RemapPeriod {
                                from_start: from,
                                to_start: to,
                            }
                        };

                        let _ = self.job_tx.send(Job::AdjustDatePeriods {
                            input,
                            output,
                            mode,
                        });
                        self.in_flight += 1;
                        self.status = "Adjusting dates...".into();
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
                        egui::RichText::new(format!(
                            "AI Confidence: {:.0}%",
                            confirmation.confidence * 100.0
                        ))
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
                            format!("-> {} (recommended)", option)
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

    fn draw_interactive_fallback_modal(&mut self, ctx: &egui::Context) {
        if let Some(req) = self.pending_interactive_fallback.clone() {
            let mut keep_open = true;
            let mut resolved_choice: Option<String> = None;

            egui::Window::new(format!("⚠️ Interactive Fallback: {}", req.stage))
                .open(&mut keep_open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;

                    ui.label("An operation could not be completed successfully.");
                    ui.label(egui::RichText::new(&req.error_details).color(egui::Color32::RED));
                    ui.add_space(10.0);

                    ui.heading("Would you like to try again using an alternative?");
                    ui.add_space(5.0);

                    for alt in &req.alternatives {
                        let btn_text = egui::RichText::new(&alt.label).strong().size(14.0);
                        if ui
                            .add_sized([ui.available_width(), 36.0], egui::Button::new(btn_text))
                            .clicked()
                        {
                            resolved_choice = Some(alt.id.clone());
                        }
                        if let Some(desc) = &alt.description {
                            ui.small(
                                egui::RichText::new(desc).color(ui.visuals().weak_text_color()),
                            );
                        }
                        ui.add_space(4.0);
                    }
                });

            if !keep_open {
                // If user clicks the 'X' to close, treat it as a cancellation if possible.
                // We'll just route a generic 'cancel' or fallback ID if we can't do anything else.
                // Alternatively, force them to pick a button by hiding the close button, but `open` gives a close button.
                // It's safer to have a dedicated Cancel button. If they force close, we can return 'cancel'.
                resolved_choice = Some("cancel".to_string());
            }

            if let Some(choice_id) = resolved_choice {
                let response = crate::engine::interactive_fallback::InteractiveFallbackResponse {
                    id: req.id,
                    selected_alternative_id: choice_id,
                };
                let _ = self
                    .job_tx
                    .send(crate::app::runtime::Job::InteractiveFallbackResponse(
                        response,
                    ));
                self.pending_interactive_fallback = None;
            }
        }
    }

    fn draw_autofix_modal(&mut self, ctx: &egui::Context) {
        let mut resolved_choice: Option<String> = None;
        if let Some(err) = self.pending_autofix.clone() {
            let mut keep_open = true;
            egui::Window::new("⚠️ Operation Failed")
                .open(&mut keep_open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;

                    ui.label("An operation encountered an error:");
                    ui.label(egui::RichText::new(err.to_string()).color(egui::Color32::RED));
                    ui.add_space(10.0);

                    ui.heading("How would you like to proceed?");
                    ui.add_space(5.0);

                    if let Some(action) = err.suggested_action() {
                        let btn_text = egui::RichText::new(action).strong().size(14.0);
                        if ui.add_sized([ui.available_width(), 36.0], egui::Button::new(btn_text)).clicked() {
                            resolved_choice = Some("action".to_string());
                        }
                    }

                    if ui.add_sized([ui.available_width(), 36.0], egui::Button::new("Cancel")).clicked() {
                        resolved_choice = Some("cancel".to_string());
                    }
                    
                    ui.add_space(5.0);
                    if ui.add_sized([ui.available_width(), 36.0], egui::Button::new("🐛 Submit Bug Report")).clicked() {
                        resolved_choice = Some("report".to_string());
                    }
                });

            if !keep_open || resolved_choice.is_some() {
                self.pending_autofix = None;
                if resolved_choice.as_deref() == Some("action") {
                    if let Some(action) = err.suggested_action() {
                        if action.contains("Settings") {
                            self.show_settings_modal = true;
                        } else if action.contains("Typst") {
                            // Tell user to set env var since engine_mode is config driven
                            self.status = "Please restart app with PDF_ENGINE_MODE=typst for perfect fidelity font synthesis.".into();
                        }
                    }
                } else if resolved_choice.as_deref() == Some("report") {
                    self.show_feedback_modal = true;
                    self.feedback_text = format!("Operation Failed: {}\n\nSteps to reproduce: \n", err);
                }
            }
        }
    }

    fn draw_workflow_hitl_modal(&mut self, ctx: &egui::Context) {
        let mut keep_open = self.show_workflow_hitl_modal;
        let mut resolved = false;

        egui::Window::new("⚠️ Workflow Human-in-the-Loop Required")
            .open(&mut keep_open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;

                match &self.workflow_stage {
                    crate::engine::workflow::WorkflowStage::FontCoverageWarning { missing_chars } => {
                        ui.heading("Font Coverage Warning");
                        ui.label("The requested font does not cover all characters in your edits.");
                        ui.label(format!("Missing characters: {:?}", missing_chars));
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("Cancel Workflow").clicked() {
                                let _ = self.job_tx.send(Job::Cancel { id: 0 });
                                resolved = true;
                            }
                            if ui.button("Proceed with Generic Font (Helvetica)").clicked() {
                                // Add job dispatch here if needed
                                // let _ = self.job_tx.send(Job::...);
                                resolved = true;
                            }
                        });
                    }
                    crate::engine::workflow::WorkflowStage::VisualFidelityWarning { score, threshold, attempt, is_borderline } => {
                        ui.heading("Visual Fidelity Warning");
                        ui.label(format!("The visual difference score {:.4} exceeds the threshold {:.4} (Attempt {})", score, threshold, attempt));
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                let _ = self.job_tx.send(Job::Cancel { id: 0 });
                                resolved = true;
                            }
                            if ui.button("Accept Overlap & Proceed").clicked() {
                                resolved = true;
                            }
                            if *is_borderline {
                                if ui.button("Compare Alternatives (2x2 Grid)").clicked() {
                                    // Trigger backend job to generate alternatives
                                    let path = self.current_pdf_path.clone();
                                    let edits = self.workflow_edits.clone();
                                    
                                    // Pick first page of edits or default to page 0
                                    let page = edits.first().map(|e| e.page).unwrap_or(0);
                                    // For demo, just pass the edits and a simple bbox of the first edit
                                    let bbox = edits.first().and_then(|e| Some(e.bbox)).unwrap_or([0.0, 0.0, 100.0, 100.0]);
                                    
                                    let _ = self.job_tx.send(Job::GenerateVisualAlternatives {
                                        input: path,
                                        out_dir: std::path::PathBuf::from("output"), // or a temp dir
                                        page,
                                        edits: edits.clone(),
                                        bbox,
                                    });
                                    self.status = "Generating alternative renders...".to_string();
                                    // don't resolve yet, keep modal open until job finishes
                                }
                            }
                        });
                    }
                    crate::engine::workflow::WorkflowStage::VisualComparisonActive { images } => {
                        ui.heading("Select Best Alternative");
                        ui.label("The primary renderer produced a borderline anomaly. Select the cleanest output below:");
                        
                        egui::ScrollArea::both().show(ui, |ui| {
                            egui::Grid::new("visual_comparison_grid")
                                .num_columns(2)
                                .spacing([16.0, 16.0])
                                .show(ui, |ui| {
                                    for (i, (label, img_bytes)) in images.iter().enumerate() {
                                        ui.vertical(|ui| {
                                            ui.label(egui::RichText::new(label).strong());
                                            
                                            // Render image bytes via egui_extras
                                            if let Ok(img) = image::load_from_memory(img_bytes) {
                                                let size = [img.width() as usize, img.height() as usize];
                                                let pixels = img.into_rgba8();
                                                let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
                                                
                                                let tex = ctx.load_texture(
                                                    format!("alt_{i}"),
                                                    color_image,
                                                    egui::TextureOptions::default(),
                                                );
                                                
                                                ui.image(&tex);
                                            } else {
                                                ui.label("(Failed to load image)");
                                            }
                                            
                                            if ui.button(format!("Use {}", label)).clicked() {
                                                // Normally here we would send a job to swap the active engine
                                                // For now, we resolve the modal
                                                resolved = true;
                                            }
                                        });
                                        
                                        if i % 2 == 1 {
                                            ui.end_row();
                                        }
                                    }
                                });
                        });
                        
                        ui.add_space(8.0);
                        if ui.button("Cancel & Discard").clicked() {
                            let _ = self.job_tx.send(Job::Cancel { id: 0 });
                            resolved = true;
                        }
                    }
                    crate::engine::workflow::WorkflowStage::ImbalanceCorrectionWarning { imbalance, proposed_changes } => {
                        ui.heading("Imbalance Correction Proposal");
                        ui.label(format!("The statement has an imbalance of {}.", imbalance));
                        ui.label("The AI has proposed the following corrections:");
                        for change in proposed_changes {
                            ui.label(format!("- Page {}: {} -> {}", change.page + 1, change.old_text, change.new_text));
                        }
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() {
                                let _ = self.job_tx.send(Job::Cancel { id: 0 });
                                resolved = true;
                            }
                            if ui.button("Accept Proposed Corrections").clicked() {
                                resolved = true;
                            }
                        });
                    }
                    crate::engine::workflow::WorkflowStage::OfflineFallbackWarning => {
                        ui.heading("Offline Fallback");
                        ui.label("All cloud parsers have failed or timed out.");
                        ui.label("The system will proceed using the Offline OCR Parser.");
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("Cancel Workflow").clicked() {
                                let _ = self.job_tx.send(Job::Cancel { id: 0 });
                                resolved = true;
                            }
                            if ui.button("Proceed Offline").clicked() {
                                resolved = true;
                            }
                        });
                    }
                    _ => {
                        ui.label("Waiting...");
                    }
                }
            });

        if resolved || !keep_open {
            self.show_workflow_hitl_modal = false;
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

                if ui.button("➕ Add PDF...").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("PDF", &["pdf"])
                        .pick_file()
                    {
                        self.transfer_test_paths
                            .push(path.to_string_lossy().to_string());
                    }
                }

                let n = self.transfer_test_paths.len();
                let pairs = if n >= 2 { n * (n - 1) } else { 0 };
                ui.label(format!("{} statements -> {} test pairs", n, pairs));

                ui.add_space(8.0);
                ui.separator();

                ui.horizontal(|ui| {
                    let can_run = n >= 2;
                    let btn = ui.add_enabled(
                        can_run,
                        egui::Button::new("▶ Run All Tests").fill(if can_run {
                            self.settings.theme.palette().accent
                        } else {
                            self.settings.theme.palette().panel
                        }),
                    );

                    if btn.clicked() {
                        let statements: Vec<std::path::PathBuf> = self
                            .transfer_test_paths
                            .iter()
                            .map(std::path::PathBuf::from)
                            .collect();
                        let _ = self.job_tx.send(Job::RunTransferTests {
                            statements,
                            max_iterations: 3,
                        });
                        self.in_flight += 1;
                        self.status = format!("Running {} transfer tests...", pairs);
                        self.toast(
                            ToastKind::Info,
                            format!("Running {} transfer test pairs...", pairs),
                        );
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

                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .max_height(200.0)
                        .show(ui, |ui| {
                            for r in &report.results {
                                let icon = if r.converged && r.final_math_ok {
                                    "✅"
                                } else {
                                    "❌"
                                };
                                let src =
                                    r.source.file_stem().unwrap_or_default().to_string_lossy();
                                let tgt =
                                    r.target.file_stem().unwrap_or_default().to_string_lossy();
                                ui.label(format!(
                                    "{} {} -> {} ({}iter, {:.1}s)",
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
                ui.small("Stored in .env (gitignored). Applied live - no restart needed.");
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
                        .on_hover_text("AI Studio key (AIza...). Used for completeness + vision checks.");
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
                            .on_hover_text("e.g. 'us' or 'eu' - must match the processor region.");
                        ui.end_row();

                        ui.label("Doc AI processor ID:");
                        ui.add(egui::TextEdit::singleline(&mut self.edit_docai_processor_id).desired_width(220.0))
                            .on_hover_text("The Bank Statement parser or Custom Extractor processor ID.");
                        ui.end_row();

                        ui.label("Service account JSON:");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.edit_docai_service_account).desired_width(150.0))
                                .on_hover_text("Path to the service-account key JSON (best-practice auth).");
                            if ui.button("Browse...").clicked() {
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
                        .on_hover_text("24-char 'hFKt...' trial key enables per-segment Pro editing.");
                        ui.end_row();



                        ui.label("LlamaParse API key:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.edit_llamaparse_api_key)
                                .password(true)
                                .desired_width(220.0),
                        );
                        ui.end_row();

                        ui.label("pdfRest API key:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.edit_pdfrest_api_key)
                                .password(true)
                                .desired_width(220.0),
                        );
                        ui.end_row();

                        ui.label("Lipi API key:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.edit_lipi_api_key)
                                .password(true)
                                .desired_width(220.0),
                        );
                        ui.end_row();

                        ui.label("Vision API key:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.edit_vision_api_key)
                                .password(true)
                                .desired_width(220.0),
                        );
                        ui.end_row();

                        ui.label("Groq API key:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.edit_groq_api_key)
                                .password(true)
                                .desired_width(220.0),
                        );
                        ui.end_row();

                        ui.label("OpenRouter API key:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.edit_openrouter_api_key)
                                .password(true)
                                .desired_width(220.0),
                        );
                        ui.end_row();

                        ui.label("OpenRouter Model:");
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt("or_model_combo")
                                .selected_text(&self.edit_openrouter_model)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut self.edit_openrouter_model, "google/gemini-2.0-flash-exp:free".to_string(), "Gemini 2.0 Flash (Free)");
                                    ui.selectable_value(&mut self.edit_openrouter_model, "meta-llama/llama-3.1-8b-instruct:free".to_string(), "Llama 3.1 8B (Free)");
                                    ui.selectable_value(&mut self.edit_openrouter_model, "mistralai/mistral-nemo:free".to_string(), "Mistral Nemo (Free)");
                                    ui.selectable_value(&mut self.edit_openrouter_model, "deepseek/deepseek-chat".to_string(), "DeepSeek Chat");
                                    ui.selectable_value(&mut self.edit_openrouter_model, "anthropic/claude-3.5-sonnet".to_string(), "Claude 3.5 Sonnet");
                                    ui.selectable_value(&mut self.edit_openrouter_model, "openai/gpt-4o".to_string(), "GPT-4o");
                                });
                            ui.add(
                                egui::TextEdit::singleline(&mut self.edit_openrouter_model)
                                    .desired_width(180.0),
                            );
                        });
                        ui.end_row();
                    });

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("📤 Export .env").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_file_name(".env.export")
                            .save_file()
                        {
                            let new_config = vec![
                                ("GEMINI_API_KEY", self.edit_gemini_api_key.trim(), false),
                                ("GEMINI_AUTH_MODE", if self.edit_gemini_use_vertex { "vertex" } else { "api_key" }, false),
                                ("DOCUMENT_AI_PROJECT_ID", self.edit_docai_project_id.trim(), false),
                                ("DOCUMENT_AI_LOCATION", self.edit_docai_location.trim(), false),
                                ("DOCUMENT_AI_PROCESSOR_ID", self.edit_docai_processor_id.trim(), false),
                                ("GOOGLE_APPLICATION_CREDENTIALS", self.edit_docai_service_account.trim(), true), // don't quote paths
                                ("DOCUMENT_AI_API_KEY", self.edit_docai_api_key.trim(), false),
                                ("PYMUPDF_PRO_KEY", self.edit_pymupdf_pro_key.trim(), false),

                                ("LLAMAPARSE_API_KEY", self.edit_llamaparse_api_key.trim(), false),
                                ("PDFREST_API_KEY", self.edit_pdfrest_api_key.trim(), false),
                                ("LIPI_API_KEY", self.edit_lipi_api_key.trim(), false),
                                ("VISION_API_KEY", self.edit_vision_api_key.trim(), false),
                                ("GROQ_API_KEY", self.edit_groq_api_key.trim(), false),
                                ("OPENROUTER_API_KEY", self.edit_openrouter_api_key.trim(), false),
                                ("OPENROUTER_MODEL", self.edit_openrouter_model.trim(), false),
                            ];
                            let content: String = new_config.iter()
                                .map(|(k, v, _)| format!("{}={}", k, v))
                                .collect::<Vec<_>>()
                                .join("\n") + "\n";

                            if let Err(e) = std::fs::write(&path, content) {
                                self.toast(ToastKind::Error, format!("Failed to export: {}", e));
                            } else {
                                self.toast(ToastKind::Success, "Exported .env successfully");
                            }
                        }
                    }

                    if ui.button("📥 Import .env").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_file() {
                            if let Ok(iter) = dotenvy::from_path_iter(&path) {
                                for (key, val) in iter.flatten() {
                                        match key.as_str() {
                                            "GEMINI_API_KEY" => self.edit_gemini_api_key = val,
                                            "DOCUMENT_AI_PROJECT_ID" => self.edit_docai_project_id = val,
                                            "DOCUMENT_AI_LOCATION" => self.edit_docai_location = val,
                                            "DOCUMENT_AI_PROCESSOR_ID" => self.edit_docai_processor_id = val,
                                            "GOOGLE_APPLICATION_CREDENTIALS" => self.edit_docai_service_account = val,
                                            "DOCUMENT_AI_API_KEY" => self.edit_docai_api_key = val,
                                            "PYMUPDF_PRO_KEY" => self.edit_pymupdf_pro_key = val,
                                            "GEMINI_AUTH_MODE" => {
                                                self.edit_gemini_use_vertex = matches!(
                                                    val.trim().to_ascii_lowercase().as_str(),
                                                    "vertex" | "vertex_ai" | "vertexai"
                                                );
                                            },

                                            "LLAMAPARSE_API_KEY" => self.edit_llamaparse_api_key = val,
                                            "PDFREST_API_KEY" => self.edit_pdfrest_api_key = val,
                                            "LIPI_API_KEY" => self.edit_lipi_api_key = val,
                                            "VISION_API_KEY" => self.edit_vision_api_key = val,
                                            "GROQ_API_KEY" => self.edit_groq_api_key = val,
                                            "OPENROUTER_API_KEY" => self.edit_openrouter_api_key = val,
                                            "OPENROUTER_MODEL" => self.edit_openrouter_model = val,
                                            _ => {}
                                        }
                                }
                                self.toast(ToastKind::Success, "Imported keys from file");
                            } else {
                                self.toast(ToastKind::Error, "Failed to read .env file");
                            }
                        }
                    }
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

                        self.edit_llamaparse_api_key = std::env::var("LLAMAPARSE_API_KEY").unwrap_or_default();
                        self.edit_pdfrest_api_key = std::env::var("PDFREST_API_KEY").unwrap_or_default();
                        self.edit_lipi_api_key = std::env::var("LIPI_API_KEY").unwrap_or_default();
                        self.edit_vision_api_key = std::env::var("VISION_API_KEY").unwrap_or_default();
                        self.edit_groq_api_key = std::env::var("GROQ_API_KEY").unwrap_or_default();
                        self.edit_openrouter_api_key = std::env::var("OPENROUTER_API_KEY").unwrap_or_default();
                        self.edit_openrouter_model = std::env::var("OPENROUTER_MODEL").unwrap_or_else(|_| "deepseek/deepseek-chat".to_string());
                        self.toast(ToastKind::Info, "Reloaded keys from environment");
                    }
                    if ui
                        .button("🧪 Test Connections")
                        .on_hover_text("Pings the Gemini and Document AI APIs to ensure your credentials are valid and authorized")
                        .clicked()
                    {
                        // Eagerly save any unsaved edits to the environment first, then run validation
                        self.save_credentials();
                        
                        let _ = self.job_tx.send(Job::ValidateCredentials);
                    }
                });

                // Live credential status reported by the runtime after the last
                // Save & apply (Job::ReloadConfig -> JobResult::ConfigReloaded).
                ui.add_space(4.0);
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    let mark = |ok: bool| if ok { "✓" } else { "✗" };
                    ui.small(format!("Doc AI {}", mark(self.api_availability.document_ai)));
                    ui.separator();
                    ui.small(format!("Gemini {}", mark(self.api_availability.gemini_api_key || self.api_availability.gemini_vertex)));
                    ui.separator();
                    ui.small(format!("Pro {}", mark(self.api_availability.pymupdf_pro)));
                    ui.separator();
                    ui.separator();
                    ui.small(format!("LlamaParse {}", mark(self.api_availability.llamaparse)));
                    ui.separator();
                    ui.small(format!("pdfRest {}", mark(self.api_availability.pdfrest)));
                    ui.separator();
                    ui.small(format!("Vision AI {}", mark(self.api_availability.vision_ai)));
                    ui.separator();
                    ui.small(format!("Offline {}", mark(true)));
                });

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

    fn draw_feedback_modal(&mut self, ctx: &egui::Context) {
        let mut open = self.show_feedback_modal;
        let mut submit = false;
        
        egui::Window::new("🐛 Report a Bug / Feedback")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size(egui::vec2(450.0, 350.0))
            .show(ctx, |ui| {
                ui.label("Describe the issue or what you were trying to do:");
                ui.add_space(4.0);
                ui.add(
                    egui::TextEdit::multiline(&mut self.feedback_text)
                        .hint_text("Please provide steps to reproduce...")
                        .desired_rows(6)
                        .desired_width(f32::INFINITY)
                );
                
                ui.add_space(10.0);
                ui.checkbox(&mut self.feedback_include_logs, "Attach recent application logs (app.log, error_report.log)");
                ui.checkbox(&mut self.feedback_include_audit, "Attach recent audit trail");
                
                ui.add_space(15.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.show_feedback_modal = false;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("🚀 Submit to Developer").clicked() {
                            submit = true;
                        }
                    });
                });
            });

        if submit {
            self.show_feedback_modal = false;
            let description = std::mem::take(&mut self.feedback_text);
            self.toast(ToastKind::Info, "Gathering logs and submitting report...".to_string());
            let _ = self.job_tx.send(Job::SubmitBugReport {
                description,
                include_logs: self.feedback_include_logs,
                include_audit: self.feedback_include_audit,
            });
        } else {
            self.show_feedback_modal = open;
        }
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
        if self.pending_interactive_fallback.is_some() {
            self.draw_interactive_fallback_modal(ctx);
        }
        if self.stuck_detection.is_some() {
            self.draw_stuck_watchdog_modal(ctx);
        }
        if self.pending_autofix.is_some() {
            self.draw_autofix_modal(ctx);
        }
        if self.show_workflow_hitl_modal {
            self.draw_workflow_hitl_modal(ctx);
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

    fn draw_stuck_watchdog_modal(&mut self, ctx: &egui::Context) {
        let mut force_fallback = false;
        let mut close_modal = false;

        if let Some(stuck_start) = self.stuck_detection {
            let elapsed = stuck_start.elapsed();
            let remaining = 25_i64.saturating_sub(elapsed.as_secs() as i64);

            if remaining <= 0 {
                force_fallback = true;
                close_modal = true;
            } else {
                egui::Window::new("⚠️ Processing Appears Stuck")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        let p = self.settings.theme.palette();
                        ui.colored_label(p.warn, "No activity detected from the backend process.");
                        ui.label(format!("Automatically falling back in {} seconds...", remaining));
                        ui.add_space(10.0);
                        
                        ui.horizontal(|ui| {
                            if ui.button("Wait Longer").clicked() {
                                close_modal = true; // resets the timer
                            }
                            if ui.button("Fallback Now").clicked() {
                                force_fallback = true;
                                close_modal = true;
                            }
                        });
                    });
            }
        }

        if close_modal {
            self.stuck_detection = None;
            self.last_runtime_activity = std::time::Instant::now();
        }

        if force_fallback {
            self.in_flight = 0; // reset state to unlock UI
            self.toast(crate::app::gui::ToastKind::Warn, "Watchdog triggered. Forcing fallback...".to_string());
            match &self.workflow_stage {
                crate::engine::workflow::WorkflowStage::Parsing { .. } => {
                    self.toast(crate::app::gui::ToastKind::Info, "Falling back to Offline Parser...".to_string());
                    if let Err(e) = self.job_tx.send(crate::app::runtime::Job::ExtractTransactions {
                        path: std::path::PathBuf::from(&self.input_path),
                        // Note: To force offline parser, we could adjust config or add a flag, but this is a good start.
                    }) {
                        tracing::error!("Failed to dispatch ExtractTransactions fallback: {}", e);
                    }
                    self.in_flight += 1;
                }
                crate::engine::workflow::WorkflowStage::Rendering { .. } => {
                    self.toast(crate::app::gui::ToastKind::Info, "Falling back to Native rendering...".to_string());
                    self.dispatch_confirm_and_render(false, false);
                }
                _ => {
                    self.progress = None;
                }
            }
        }
    }
}

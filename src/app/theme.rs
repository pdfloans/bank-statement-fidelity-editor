use egui::Color32;
use serde::{Deserialize, Serialize};

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

pub struct Palette {
    pub bg: egui::Color32,
    pub panel: egui::Color32,
    pub surface: egui::Color32,
    pub text: egui::Color32,
    pub weak: egui::Color32,
    pub accent: egui::Color32,
    pub success: egui::Color32,
    pub warn: egui::Color32,
    pub error: egui::Color32,
    pub info: egui::Color32,
}

impl Theme {
    pub fn palette(self) -> Palette {
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

    pub fn label(self) -> &'static str {
        match self {
            Theme::System => "System (Auto)",
            Theme::Dark => "Dark",
            Theme::Light => "Light",
            Theme::Midnight => "Midnight",
            Theme::Solarized => "Solarized",
        }
    }

    pub fn apply(self, ctx: &egui::Context) {
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

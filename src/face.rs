use chrono::{NaiveDateTime, Timelike};
use eframe::egui::{self, pos2, Align2, Color32, FontId, Pos2, RichText, Sense, Stroke, Vec2};

use crate::color::prefers_dark_text;
use crate::config::{ClockConfig, FaceMode, ThemeMode};
use crate::i18n::format_clock_time;

pub fn show_face(ui: &mut egui::Ui, config: &ClockConfig, civil: NaiveDateTime) {
    let time = format_clock_time(civil, config.hour24, config.show_seconds, config.language);
    let width = ui.available_width().max(160.0);
    let font = font_color(ui, config);
    match config.face {
        FaceMode::Digital => {
            let size = (width / 5.2).clamp(34.0, 96.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(time)
                        .size(size)
                        .monospace()
                        .strong()
                        .color(font),
                );
            });
        }
        FaceMode::Analog => {
            let radius = (width * 0.38).clamp(72.0, 220.0);
            ui.vertical_centered(|ui| {
                draw_dial(ui, config, civil, radius);
            });
            let size = (width / 12.0).clamp(16.0, 32.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(time).size(size).monospace().color(font));
            });
        }
    }
}

pub fn font_color(ui: &egui::Ui, config: &ClockConfig) -> Color32 {
    config
        .font_color
        .map(color_from_rgb)
        .unwrap_or_else(|| clock_visuals(ui, config).text_color())
}

pub fn dial_color(ui: &egui::Ui, config: &ClockConfig) -> Color32 {
    config
        .dial_color
        .map(color_from_rgb)
        .unwrap_or_else(|| clock_visuals(ui, config).text_color())
}

pub fn second_hand_color(ui: &egui::Ui, config: &ClockConfig) -> Color32 {
    config
        .second_hand_color
        .map(color_from_rgb)
        .unwrap_or_else(|| {
            if clock_visuals(ui, config).dark_mode {
                Color32::from_rgb(255, 150, 150)
            } else {
                Color32::from_rgb(190, 45, 45)
            }
        })
}

fn clock_visuals(ui: &egui::Ui, config: &ClockConfig) -> egui::Visuals {
    if let Some(rgb) = config.background {
        return if prefers_dark_text(rgb) {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        };
    }
    match config.theme {
        ThemeMode::System => ui.visuals().clone(),
        ThemeMode::Light => egui::Visuals::light(),
        ThemeMode::Dark => egui::Visuals::dark(),
    }
}

fn color_from_rgb(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

fn draw_dial(ui: &mut egui::Ui, config: &ClockConfig, civil: NaiveDateTime, radius: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(radius * 2.0 + 8.0), Sense::hover());
    let center = rect.center();
    let painter = ui.painter();
    let fg = dial_color(ui, config);
    let second = second_hand_color(ui, config);
    painter.circle_stroke(center, radius, Stroke::new(2.0, fg));

    for mark in 0..60 {
        let degrees = mark as f32 * 6.0;
        let outer = polar(center, radius - 3.0, degrees);
        let length = if mark % 5 == 0 {
            radius * 0.12
        } else {
            radius * 0.055
        };
        let inner = polar(center, radius - 3.0 - length, degrees);
        let width = if mark % 5 == 0 { 2.0 } else { 1.0 };
        painter.line_segment([inner, outer], Stroke::new(width, fg));
    }

    for (number, degrees) in [(12, 0.0), (3, 90.0), (6, 180.0), (9, 270.0)] {
        let point = polar(center, radius * 0.72, degrees);
        painter.text(
            point,
            Align2::CENTER_CENTER,
            number.to_string(),
            FontId::proportional(radius * 0.16),
            fg,
        );
    }

    let hour = civil.time().hour();
    let minute = civil.time().minute();
    let seconds = civil.time().second();
    let hour_degrees =
        (hour % 12) as f32 * 30.0 + minute as f32 * 0.5 + seconds as f32 * (0.5 / 60.0);
    let minute_degrees = minute as f32 * 6.0 + seconds as f32 * 0.1;
    hand(painter, center, radius * 0.50, hour_degrees, 4.0, fg);
    hand(painter, center, radius * 0.74, minute_degrees, 3.0, fg);
    if config.show_seconds {
        hand(
            painter,
            center,
            radius * 0.82,
            seconds as f32 * 6.0,
            1.5,
            second,
        );
    }
    painter.circle_filled(center, 4.0, fg);
}

fn hand(
    painter: &egui::Painter,
    center: Pos2,
    length: f32,
    degrees_from_12: f32,
    width: f32,
    color: Color32,
) {
    painter.line_segment(
        [center, polar(center, length, degrees_from_12)],
        Stroke::new(width, color),
    );
}

fn polar(center: Pos2, length: f32, degrees_from_12: f32) -> Pos2 {
    let radians = degrees_from_12.to_radians();
    pos2(
        center.x + length * radians.sin(),
        center.y - length * radians.cos(),
    )
}

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, FixedOffset, Utc};
use eframe::egui::{self, Color32, RichText, ViewportCommand, WindowLevel};

use crate::config::{ClockConfig, FaceMode, Language, ThemeMode};
use crate::face::{dial_color, font_color, second_hand_color, show_face};
use crate::i18n::{format_clock_date, format_compared, tr};
use crate::icon;
use crate::search::{HitKind, SearchHit, ZoneIndex};
use crate::tray::{self, TrayGuard};
use crate::zone::{
    city_label, computer_offset_seconds, delta_seconds, format_delta, format_utc_label,
    window_title, zone_name, zone_offset_seconds,
};

pub fn run() -> eframe::Result<()> {
    let config = crate::zone::default_config();
    let title = window_title(&config.zone);
    let slot = (std::process::id() % 6) as f32;
    let size = clock_window_size(config.face);
    let mut options = eframe::NativeOptions {
        persist_window: false,
        centered: false,
        ..Default::default()
    };
    options.viewport = egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size(size)
        .with_min_inner_size(size)
        .with_position([80.0 + slot * 36.0, 60.0 + slot * 28.0])
        .with_icon(Arc::new(egui::IconData {
            rgba: icon::clock_icon_rgba(64),
            width: 64,
            height: 64,
        }));

    eframe::run_native(
        "clocks",
        options,
        Box::new(move |cc| Ok(Box::new(ClockApp::new(cc, config)))),
    )
}

struct ClockApp {
    config: ClockConfig,
    settings_open: bool,
    index: ZoneIndex,
    query: String,
    hits: Vec<SearchHit>,
    hits_language: Language,
    title: String,
    applied_top: Option<bool>,
    applied_face: FaceMode,
    language_code: Arc<AtomicU8>,
    tray: Option<TrayGuard>,
}

impl ClockApp {
    fn new(cc: &eframe::CreationContext<'_>, config: ClockConfig) -> Self {
        let language_code = Arc::new(AtomicU8::new(config.language as u8));
        let tray = tray::hwnd_from_creation(cc).map(|hwnd| {
            tray::tag_clock_window(hwnd);
            tray::spawn(Arc::clone(&language_code))
        });
        let applied_face = config.face;
        Self {
            title: window_title(&config.zone),
            settings_open: false,
            hits_language: config.language,
            applied_top: None,
            applied_face,
            index: ZoneIndex::load(),
            query: String::new(),
            hits: Vec::new(),
            language_code,
            tray,
            config,
        }
    }

    fn refresh_hits(&mut self) {
        self.hits = self.index.search(&self.query, self.config.language);
        self.hits_language = self.config.language;
    }

    fn apply_hit(&mut self, hit: SearchHit) {
        self.config.zone = hit.choice;
        self.query.clear();
        self.hits.clear();
        self.hits_language = self.config.language;
    }
}

impl eframe::App for ClockApp {
    fn persist_egui_memory(&self) -> bool {
        false
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.language_code
            .store(self.config.language as u8, Ordering::Relaxed);
        ctx.set_theme(egui::ThemePreference::System);
        let _ = &self.tray;
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let title = window_title(&self.config.zone);
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        if self.applied_top != Some(self.config.always_on_top) {
            ctx.send_viewport_cmd(ViewportCommand::WindowLevel(if self.config.always_on_top {
                WindowLevel::AlwaysOnTop
            } else {
                WindowLevel::Normal
            }));
            self.applied_top = Some(self.config.always_on_top);
        }
        ctx.input(|input| {
            let viewport = input.viewport();
            if let Some(rect) = viewport.inner_rect {
                self.config.window.width = rect.width();
                self.config.window.height = rect.height();
            }
            if let Some(rect) = viewport.outer_rect {
                self.config.window.x = rect.min.x;
                self.config.window.y = rect.min.y;
            }
        });

        let interval = if self.config.show_seconds { 100 } else { 500 };
        ctx.request_repaint_after(Duration::from_millis(interval));

        let now = Utc::now();
        let civil = civil_time(&self.config, now);
        let clock_offset = zone_offset_seconds(&self.config.zone, now);
        let delta = format_delta(delta_seconds(clock_offset, computer_offset_seconds(now)));
        let utc_label = format_utc_label(clock_offset);
        let zone_label = zone_name(&self.config.zone);
        let city = city_label(&self.config.zone).map(str::to_string);

        let inner = egui::Frame::NONE
            .fill(clock_fill(ui, &self.config))
            .inner_margin(16.0)
            .show(ui, |ui| {
                self.show_clock(ui, civil, &city, &zone_label, &utc_label, &delta);
            });
        let language = self.config.language;
        inner.response.container_context_menu(|ui| {
            ui.menu_button(tr(language, "clock_type"), |ui| {
                if ui
                    .selectable_value(
                        &mut self.config.face,
                        FaceMode::Digital,
                        tr(language, "digital"),
                    )
                    .clicked()
                {
                    ui.close();
                }
                if ui
                    .selectable_value(
                        &mut self.config.face,
                        FaceMode::Analog,
                        tr(language, "analog"),
                    )
                    .clicked()
                {
                    ui.close();
                }
            });
            if ui.button(tr(language, "settings")).clicked() {
                if self.settings_open {
                    let settings = settings_viewport_id();
                    ui.ctx()
                        .send_viewport_cmd_to(settings, ViewportCommand::Minimized(false));
                    ui.ctx()
                        .send_viewport_cmd_to(settings, ViewportCommand::Focus);
                } else {
                    self.settings_open = true;
                }
                ui.close();
            }
            if ui.button(tr(language, "new_clock")).clicked() {
                tray::launch_new_clock();
                ui.close();
            }
        });

        if self.settings_open {
            self.show_settings_window(&ctx);
        }
        self.apply_face_size(&ctx);
    }
}

impl ClockApp {
    fn show_clock(
        &mut self,
        ui: &mut egui::Ui,
        civil: chrono::NaiveDateTime,
        city: &Option<String>,
        zone_label: &str,
        utc_label: &str,
        delta: &str,
    ) {
        let language = self.config.language;
        let ink = font_color(ui, &self.config);
        show_face(ui, &self.config, civil);
        ui.add_space(8.0);
        let info = (ui.available_width() / 18.0).clamp(14.0, 22.0);
        ui.vertical_centered(|ui| {
            if self.config.show_date {
                ui.label(
                    RichText::new(format_clock_date(civil.date(), language))
                        .size(info)
                        .color(ink),
                );
            }
            if let Some(city) = city {
                ui.label(RichText::new(city).size(info + 2.0).strong().color(ink));
            }
            ui.label(RichText::new(zone_label).size(info).color(ink));
            if zone_label != utc_label {
                ui.label(RichText::new(utc_label).size(info).color(ink));
            }
            ui.label(
                RichText::new(format_compared(language, delta))
                    .size(info - 1.0)
                    .color(ink),
            );
        });
        ui.allocate_space(ui.available_size());
    }

    fn show_settings_window(&mut self, ctx: &egui::Context) {
        let language = self.config.language;
        let mut close = false;
        ctx.show_viewport_immediate(
            settings_viewport_id(),
            egui::ViewportBuilder::default()
                .with_title(tr(language, "settings"))
                .with_inner_size([420.0, 640.0])
                .with_min_inner_size([360.0, 480.0])
                .with_window_level(if self.config.always_on_top {
                    WindowLevel::AlwaysOnTop
                } else {
                    WindowLevel::Normal
                }),
            |ui, _class| {
                egui::Frame::NONE
                    .fill(ui.visuals().panel_fill)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.label(RichText::new(tr(self.config.language, "zone")).strong());
                                self.show_zone_search(ui);
                                ui.add_space(8.0);
                                show_settings(ui, &mut self.config);
                            });
                    });
                if ui.input(|input| input.viewport().close_requested()) {
                    close = true;
                }
                ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
            },
        );
        if close {
            self.settings_open = false;
        }
    }

    fn apply_face_size(&mut self, ctx: &egui::Context) {
        if self.applied_face == self.config.face {
            return;
        }
        let size = clock_window_size(self.config.face);
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, ViewportCommand::MinInnerSize(size));
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, ViewportCommand::InnerSize(size));
        self.applied_face = self.config.face;
    }

    fn show_zone_search(&mut self, ui: &mut egui::Ui) {
        let language = self.config.language;
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .hint_text(tr(language, "search_hint"))
                .desired_width(ui.available_width()),
        );
        if response.changed() || self.hits_language != language {
            self.refresh_hits();
        }
        let choose_first = response.lost_focus()
            && ui.input(|input| input.key_pressed(egui::Key::Enter))
            && !self.hits.is_empty();
        if choose_first {
            let hit = self.hits[0].clone();
            self.apply_hit(hit);
        } else if self.query.is_empty() {
            self.hits.clear();
        } else if self.hits.is_empty() {
            ui.label(tr(language, "no_results"));
        } else {
            let row_height = 28.0;
            let height = (self.hits.len() as f32 * row_height).min(row_height * 8.0);
            let hits = self.hits.clone();
            egui::ScrollArea::vertical()
                .max_height(height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for hit in hits {
                        let text = hit_line(&hit, language);
                        if ui
                            .add_sized([ui.available_width(), row_height], egui::Button::new(text))
                            .clicked()
                        {
                            self.apply_hit(hit);
                            break;
                        }
                    }
                });
        }
    }
}

fn show_settings(ui: &mut egui::Ui, config: &mut ClockConfig) {
    let language = config.language;
    ui.label(tr(language, "face"));
    ui.horizontal(|ui| {
        ui.selectable_value(&mut config.face, FaceMode::Digital, tr(language, "digital"));
        ui.selectable_value(&mut config.face, FaceMode::Analog, tr(language, "analog"));
    });
    ui.checkbox(&mut config.show_seconds, tr(language, "seconds"));
    ui.checkbox(&mut config.show_date, tr(language, "date"));
    ui.label(tr(language, "hour_cycle"));
    ui.horizontal(|ui| {
        ui.selectable_value(&mut config.hour24, true, tr(language, "h24"));
        ui.selectable_value(&mut config.hour24, false, tr(language, "h12"));
    });
    ui.checkbox(&mut config.always_on_top, tr(language, "always_on_top"));

    ui.label(tr(language, "theme"));
    ui.horizontal(|ui| {
        ui.selectable_value(
            &mut config.theme,
            ThemeMode::System,
            tr(language, "theme_system"),
        );
        ui.selectable_value(
            &mut config.theme,
            ThemeMode::Light,
            tr(language, "theme_light"),
        );
        ui.selectable_value(
            &mut config.theme,
            ThemeMode::Dark,
            tr(language, "theme_dark"),
        );
    });

    ui.label(tr(language, "language"));
    ui.horizontal(|ui| {
        ui.selectable_value(
            &mut config.language,
            Language::En,
            tr(language, "language_en"),
        );
        ui.selectable_value(
            &mut config.language,
            Language::Pl,
            tr(language, "language_pl"),
        );
    });

    color_setting(
        ui,
        &tr(language, "background"),
        &tr(language, "reset_background"),
        &mut config.background,
        rgb_of(ui.visuals().panel_fill),
    );
    let automatic_font = rgb_of(font_color(ui, config));
    let automatic_dial = rgb_of(dial_color(ui, config));
    let automatic_second = rgb_of(second_hand_color(ui, config));
    let reset = tr(language, "reset_automatic");
    color_setting(
        ui,
        &tr(language, "font_color"),
        &reset,
        &mut config.font_color,
        automatic_font,
    );
    color_setting(
        ui,
        &tr(language, "dial_color"),
        &reset,
        &mut config.dial_color,
        automatic_dial,
    );
    color_setting(
        ui,
        &tr(language, "second_hand_color"),
        &reset,
        &mut config.second_hand_color,
        automatic_second,
    );
}

fn color_setting(
    ui: &mut egui::Ui,
    label: &str,
    reset: &str,
    value: &mut Option<[u8; 3]>,
    automatic: [u8; 3],
) {
    ui.horizontal(|ui| {
        ui.label(label);
        let mut rgb = value.unwrap_or(automatic);
        if egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed() {
            *value = Some(rgb);
        }
        if value.is_some() && ui.button(reset).clicked() {
            *value = None;
        }
    });
}

fn hit_line(hit: &SearchHit, language: Language) -> String {
    let kind = tr(
        language,
        match hit.kind {
            HitKind::Offset => "kind_offset",
            HitKind::City => "kind_city",
            HitKind::Country => "kind_country",
            HitKind::Zone => "kind_zone",
        },
    );
    if hit.secondary.is_empty() {
        format!("{kind}: {}", hit.primary)
    } else {
        format!("{kind}: {} — {}", hit.primary, hit.secondary)
    }
}

fn civil_time(config: &ClockConfig, now: DateTime<Utc>) -> chrono::NaiveDateTime {
    let offset = zone_offset_seconds(&config.zone, now);
    let fixed = FixedOffset::east_opt(offset).unwrap_or_else(|| FixedOffset::east_opt(0).unwrap());
    now.with_timezone(&fixed).naive_local()
}

fn clock_fill(ui: &egui::Ui, config: &ClockConfig) -> Color32 {
    if let Some(rgb) = config.background {
        return Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
    }
    match config.theme {
        ThemeMode::System => ui.visuals().panel_fill,
        ThemeMode::Light => egui::Visuals::light().panel_fill,
        ThemeMode::Dark => egui::Visuals::dark().panel_fill,
    }
}

fn rgb_of(color: Color32) -> [u8; 3] {
    [color.r(), color.g(), color.b()]
}

fn clock_window_size(face: FaceMode) -> egui::Vec2 {
    match face {
        FaceMode::Digital => egui::vec2(300.0, 220.0),
        FaceMode::Analog => egui::vec2(340.0, 460.0),
    }
}

fn settings_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("clock-settings")
}

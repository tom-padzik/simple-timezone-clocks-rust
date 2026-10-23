use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, FixedOffset, Timelike, Utc};
use eframe::egui::{self, Align, Color32, Layout, RichText, ViewportCommand, WindowLevel};

use crate::config::{ClockConfig, FaceMode, Language, ThemeMode};
use crate::face::{dial_color, font_color, second_hand_color, show_face};
use crate::i18n::{format_clock_date, format_compared, tr};
use crate::icon;
use crate::place::{self, Bounds};
use crate::search::{HitKind, SearchHit, ZoneIndex};
use crate::tray::{self, TrayGuard};
use crate::zone::{
    city_label, computer_offset_seconds, delta_seconds, format_delta, format_utc_label,
    window_title, zone_name, zone_offset_seconds,
};

pub fn run() -> eframe::Result<()> {
    let launch = Launch::from_args();
    let config = launch.config;
    let title = window_title(&config.zone);
    let min_size = clock_window_size(config.face);
    let size = if launch.centered {
        min_size
    } else {
        egui::vec2(
            config.window.width.max(min_size.x),
            config.window.height.max(min_size.y),
        )
    };
    let mut options = eframe::NativeOptions {
        persist_window: false,
        centered: launch.centered,
        ..Default::default()
    };
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size(size)
        .with_min_inner_size(min_size)
        .with_icon(Arc::new(egui::IconData {
            rgba: icon::clock_icon_rgba(64),
            width: 64,
            height: 64,
        }));
    if launch.centered {
        // `centered` assumes the primary monitor starts at the origin. Place the
        // window on the monitor under the cursor instead, which is the tray click.
        options.window_builder = Some(Box::new(move |builder| {
            let inner = builder.inner_size.unwrap_or(size);
            match tray::center_on_cursor_monitor(inner.x, inner.y) {
                Some((x, y)) => builder.with_position([x, y]),
                None => builder,
            }
        }));
    } else {
        viewport = viewport.with_position([config.window.x, config.window.y]);
    }
    options.viewport = viewport;

    eframe::run_native(
        "clocks",
        options,
        Box::new(move |cc| Ok(Box::new(ClockApp::new(cc, config)))),
    )
}

/// How this process was started.
struct Launch {
    config: ClockConfig,
    /// Tray and a bare launch open in the middle of the screen.
    centered: bool,
}

impl Launch {
    fn from_args() -> Self {
        Self::from_iter(std::env::args().skip(1))
    }

    fn from_iter(args: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        let mut args = args.into_iter();
        if args
            .next()
            .as_ref()
            .is_some_and(|arg| arg.as_ref() == "--clock")
        {
            if let Some(json) = args.next() {
                if let Ok(config) = serde_json::from_str(json.as_ref()) {
                    return Self {
                        config,
                        centered: false,
                    };
                }
            }
        }
        Self {
            config: crate::zone::default_config(),
            centered: true,
        }
    }
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
    computer: ComputerOffset,
    language_code: Arc<AtomicU8>,
    tray: Option<TrayGuard>,
    hwnd: Option<isize>,
    /// Clocks this window has already opened. The next one steps further up.
    spawned: u32,
    settings_placed: bool,
    settings_inner: Option<egui::Vec2>,
}

/// The computer's own UTC offset, refreshed at most every [`Self::REFRESH`].
///
/// Reading the local zone goes through the Windows time-zone API, so doing it on
/// every repaint is wasted work; a zone change is picked up within a few seconds.
#[derive(Default)]
struct ComputerOffset {
    seconds: i32,
    refreshed: Option<Instant>,
}

impl ComputerOffset {
    const REFRESH: Duration = Duration::from_secs(30);

    fn seconds(&mut self, now: DateTime<Utc>) -> i32 {
        if self
            .refreshed
            .is_none_or(|at| at.elapsed() >= Self::REFRESH)
        {
            self.seconds = computer_offset_seconds(now);
            self.refreshed = Some(Instant::now());
        }
        self.seconds
    }
}

impl ClockApp {
    fn new(cc: &eframe::CreationContext<'_>, config: ClockConfig) -> Self {
        let language_code = Arc::new(AtomicU8::new(config.language as u8));
        let hwnd = tray::hwnd_from_creation(cc).map(|hwnd| {
            tray::tag_clock_window(hwnd);
            hwnd.0 as isize
        });
        let tray = hwnd.map(|_| tray::spawn(Arc::clone(&language_code)));
        let applied_face = config.face;
        Self {
            title: window_title(&config.zone),
            settings_open: false,
            hits_language: config.language,
            applied_top: None,
            applied_face,
            computer: ComputerOffset::default(),
            index: ZoneIndex::load(),
            query: String::new(),
            hits: Vec::new(),
            language_code,
            tray,
            hwnd,
            spawned: 0,
            settings_placed: false,
            settings_inner: None,
            config,
        }
    }

    fn launch_child(&mut self, ctx: &egui::Context) {
        let mut child = self.config.clone();
        let min_size = clock_window_size(child.face);
        let inner = egui::vec2(
            child.window.width.max(min_size.x),
            child.window.height.max(min_size.y),
        );
        let chrome = chrome_height(ctx);
        let step = self.spawned.saturating_add(1);
        if let Some(work) = self.work_bounds(ctx) {
            let origin = place::cascade_above(
                (child.window.x, child.window.y),
                (inner.x, inner.y + chrome),
                work,
                step,
            );
            child.window.x = origin.0;
            child.window.y = origin.1;
        } else {
            child.window.x += 32.0 * step as f32;
            child.window.y -= 32.0 * step as f32;
        }
        child.window.width = inner.x;
        child.window.height = inner.y;
        if let Ok(json) = serde_json::to_string(&child) {
            tray::launch_inherited(&json);
            self.spawned = step;
        }
    }

    fn work_bounds(&self, ctx: &egui::Context) -> Option<Bounds> {
        let px = tray::window_work_px(self.hwnd?)?;
        let scale = ctx.pixels_per_point();
        if scale <= 0.0 {
            return None;
        }
        Some(Bounds {
            x: px.left as f32 / scale,
            y: px.top as f32 / scale,
            width: (px.right - px.left) as f32 / scale,
            height: (px.bottom - px.top) as f32 / scale,
        })
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
        ctx.set_theme(match self.config.theme {
            ThemeMode::System => egui::ThemePreference::System,
            ThemeMode::Light => egui::ThemePreference::Light,
            ThemeMode::Dark => egui::ThemePreference::Dark,
        });
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

        let now = Utc::now();
        // Analog hands advance on every second even when the second hand is
        // hidden. A digital face without seconds changes on the minute.
        ctx.request_repaint_after(next_read_delay(
            self.config.face,
            self.config.show_seconds,
            now.second(),
            now.timestamp_subsec_millis(),
        ));

        let civil = civil_time(&self.config, now);
        let clock_offset = zone_offset_seconds(&self.config.zone, now);
        let delta = format_delta(delta_seconds(clock_offset, self.computer.seconds(now)));
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
                let ctx = ui.ctx().clone();
                self.launch_child(&ctx);
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
        let form_width = settings_form_width(ctx, &self.config);
        let chrome = chrome_height(ctx);
        let margin = SETTINGS_MARGIN;
        let inner = self
            .settings_inner
            .unwrap_or(egui::vec2(form_width + margin * 2.0, 520.0));
        let mut builder = egui::ViewportBuilder::default()
            .with_title(tr(language, "settings"))
            .with_inner_size(inner)
            .with_min_inner_size([320.0, 200.0])
            .with_resizable(false)
            .with_window_level(if self.config.always_on_top {
                WindowLevel::AlwaysOnTop
            } else {
                WindowLevel::Normal
            });
        if !self.settings_placed {
            let outer = egui::vec2(inner.x, inner.y + chrome);
            if let Some(origin) = self.settings_origin(ctx, outer) {
                builder = builder.with_position(origin);
                self.settings_placed = true;
            }
        }
        let mut close = false;
        let mut measured = None;
        ctx.show_viewport_immediate(settings_viewport_id(), builder, |ui, _class| {
            let laid = egui::Frame::NONE
                .fill(ui.visuals().panel_fill)
                .inner_margin(margin)
                .show(ui, |ui| {
                    ui.set_min_width(form_width);
                    ui.set_max_width(form_width);
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.label(RichText::new(tr(self.config.language, "zone")).strong());
                    self.show_zone_search(ui);
                    ui.add_space(10.0);
                    show_settings(ui, &mut self.config);
                });
            measured = Some(laid.response.rect.size());
            if ui.input(|input| input.viewport().close_requested()) {
                close = true;
            }
            ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
        });
        if let Some(size) = measured {
            self.remember_settings_size(size);
        }
        if close {
            self.settings_open = false;
            self.settings_placed = false;
        }
    }

    fn settings_origin(&self, ctx: &egui::Context, outer: egui::Vec2) -> Option<egui::Pos2> {
        let clock = ctx.input(|input| input.viewport().outer_rect)?;
        let work = if let Some(work) = self.work_bounds(ctx) {
            work
        } else {
            let size = ctx.input(|input| input.viewport().monitor_size)?;
            Bounds {
                x: 0.0,
                y: 0.0,
                width: size.x,
                height: size.y,
            }
        };
        let anchor = Bounds {
            x: clock.min.x,
            y: clock.min.y,
            width: clock.width(),
            height: clock.height(),
        };
        let (x, y) = place::beside(anchor, (outer.x, outer.y), work);
        Some(egui::pos2(x, y))
    }

    fn remember_settings_size(&mut self, size: egui::Vec2) {
        let size = egui::vec2(size.x.round(), size.y.round());
        let changed = self.settings_inner.is_none_or(|current| {
            (current.x - size.x).abs() > 1.0 || (current.y - size.y).abs() > 1.0
        });
        if changed {
            self.settings_inner = Some(size);
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
            // One letter never matches anything, so do not claim there are no results.
            if self.query.trim().chars().count() >= 2 {
                ui.label(tr(language, "no_results"));
            }
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
    let option_column = column_width(
        ui,
        &[
            tr(language, "face"),
            tr(language, "hour_cycle"),
            tr(language, "theme"),
            tr(language, "language"),
        ],
    );
    option_row(ui, &tr(language, "face"), option_column, |ui| {
        ui.selectable_value(&mut config.face, FaceMode::Digital, tr(language, "digital"));
        ui.selectable_value(&mut config.face, FaceMode::Analog, tr(language, "analog"));
    });
    option_row(ui, "", option_column, |ui| {
        ui.checkbox(&mut config.show_seconds, tr(language, "seconds"));
        ui.checkbox(&mut config.show_date, tr(language, "date"));
    });
    option_row(ui, &tr(language, "hour_cycle"), option_column, |ui| {
        ui.selectable_value(&mut config.hour24, true, tr(language, "h24"));
        ui.selectable_value(&mut config.hour24, false, tr(language, "h12"));
    });
    option_row(ui, "", option_column, |ui| {
        ui.checkbox(&mut config.always_on_top, tr(language, "always_on_top"));
    });

    ui.add_space(6.0);
    option_row(ui, &tr(language, "theme"), option_column, |ui| {
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
    option_row(ui, &tr(language, "language"), option_column, |ui| {
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

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(4.0);

    let color_column = column_width(
        ui,
        &[
            tr(language, "background"),
            tr(language, "font_color"),
            tr(language, "dial_color"),
            tr(language, "second_hand_color"),
        ],
    );
    let automatic_background = rgb_of(clock_fill(ui, config));
    color_setting(
        ui,
        &tr(language, "background"),
        color_column,
        &tr(language, "reset_background"),
        &mut config.background,
        automatic_background,
    );
    let automatic_font = rgb_of(font_color(ui, config));
    let automatic_dial = rgb_of(dial_color(ui, config));
    let automatic_second = rgb_of(second_hand_color(ui, config));
    let reset = tr(language, "reset_automatic");
    color_setting(
        ui,
        &tr(language, "font_color"),
        color_column,
        &reset,
        &mut config.font_color,
        automatic_font,
    );
    color_setting(
        ui,
        &tr(language, "dial_color"),
        color_column,
        &reset,
        &mut config.dial_color,
        automatic_dial,
    );
    color_setting(
        ui,
        &tr(language, "second_hand_color"),
        color_column,
        &reset,
        &mut config.second_hand_color,
        automatic_second,
    );
}

fn option_row(ui: &mut egui::Ui, label: &str, column: f32, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(column, ui.spacing().interact_size.y),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.label(label);
            },
        );
        ui.horizontal(|ui| add(ui));
    });
}

fn color_setting(
    ui: &mut egui::Ui,
    label: &str,
    column: f32,
    reset: &str,
    value: &mut Option<[u8; 3]>,
    automatic: [u8; 3],
) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(column, ui.spacing().interact_size.y),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.label(label);
            },
        );
        let mut rgb = value.unwrap_or(automatic);
        if egui::color_picker::color_edit_button_srgb(ui, &mut rgb).changed() {
            *value = Some(rgb);
        }
        if value.is_some() && ui.button(reset).clicked() {
            *value = None;
        }
    });
}

fn column_width(ui: &egui::Ui, labels: &[String]) -> f32 {
    labels
        .iter()
        .map(|label| text_width(ui.ctx(), label))
        .fold(0.0, f32::max)
        + 16.0
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

/// Wait until the drawn time changes, and never less than half a second.
///
/// A paint just before a second or minute boundary must not schedule the next
/// read immediately. Half a second is the shortest gap, so the hands still
/// catch each new second without polling faster than that.
fn next_read_delay(face: FaceMode, show_seconds: bool, second: u32, millis: u32) -> Duration {
    const MIN_MS: u32 = 500;
    let until_change = if face == FaceMode::Analog || show_seconds {
        1_000 - millis
    } else {
        60_000 - (second * 1_000 + millis)
    };
    Duration::from_millis(u64::from(until_change.max(MIN_MS)) + 15)
}

const SETTINGS_MARGIN: f32 = 16.0;

fn chrome_height(ctx: &egui::Context) -> f32 {
    ctx.input(|input| {
        let viewport = input.viewport();
        match (viewport.outer_rect, viewport.inner_rect) {
            (Some(outer), Some(inner)) => (outer.height() - inner.height()).max(0.0),
            _ => 40.0,
        }
    })
}

/// Width of the settings form, wide enough for the longest row in this language.
fn settings_form_width(ctx: &egui::Context, config: &ClockConfig) -> f32 {
    let language = config.language;
    let style = ctx.global_style();
    let spacing = style.spacing.clone();
    let button = spacing.button_padding.x * 2.0;
    let gap = spacing.item_spacing.x;
    let row = |widths: &[f32]| -> f32 {
        let gaps = widths.len().saturating_sub(1) as f32 * gap;
        widths.iter().sum::<f32>() + widths.len() as f32 * button + gaps
    };
    let text = |key: &str| text_width(ctx, &tr(language, key));
    let option_column = [
        text("face"),
        text("hour_cycle"),
        text("theme"),
        text("language"),
    ]
    .into_iter()
    .fold(0.0, f32::max)
        + 16.0;
    let face = option_column + gap + row(&[text("digital"), text("analog")]);
    let checks = option_column
        + gap
        + text("seconds")
        + text("date")
        + spacing.icon_width * 2.0
        + spacing.icon_spacing * 2.0
        + gap;
    let hours = option_column + gap + row(&[text("h24"), text("h12")]);
    let pinned =
        option_column + gap + text("always_on_top") + spacing.icon_width + spacing.icon_spacing;
    let theme = option_column
        + gap
        + row(&[
            text("theme_system"),
            text("theme_light"),
            text("theme_dark"),
        ]);
    let language_row = option_column + gap + row(&[text("language_en"), text("language_pl")]);
    let color_column = [
        text("background"),
        text("font_color"),
        text("dial_color"),
        text("second_hand_color"),
    ]
    .into_iter()
    .fold(0.0, f32::max)
        + 16.0;
    let mut colors = color_column + gap + spacing.interact_size.x;
    if config.background.is_some() {
        colors = colors.max(
            color_column + gap + spacing.interact_size.x + gap + text("reset_background") + button,
        );
    }
    if config.font_color.is_some()
        || config.dial_color.is_some()
        || config.second_hand_color.is_some()
    {
        colors = colors.max(
            color_column + gap + spacing.interact_size.x + gap + text("reset_automatic") + button,
        );
    }
    let search = text("search_hint") + 48.0;
    face.max(checks)
        .max(hours)
        .max(pinned)
        .max(theme)
        .max(language_row)
        .max(colors)
        .max(search)
        + 24.0
}

fn text_width(ctx: &egui::Context, text: &str) -> f32 {
    let font = egui::TextStyle::Body.resolve(&ctx.global_style());
    ctx.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(text.to_string(), font, Color32::WHITE)
            .rect
            .width()
    })
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

#[cfg(test)]
mod tests {
    use super::{next_read_delay, Launch};
    use crate::config::{ClockConfig, FaceMode, Language, ThemeMode, WindowRect, ZoneChoice};

    #[test]
    fn analog_hands_wake_on_the_second_when_seconds_are_hidden() {
        let delay = next_read_delay(FaceMode::Analog, false, 10, 0);
        assert_eq!(delay.as_millis(), 1_015);
    }

    #[test]
    fn a_late_paint_waits_at_least_half_a_second() {
        let delay = next_read_delay(FaceMode::Analog, true, 10, 800);
        assert!(delay.as_millis() >= 500);
        assert_eq!(delay.as_millis(), 515);
    }

    #[test]
    fn digital_without_seconds_waits_for_the_minute() {
        let delay = next_read_delay(FaceMode::Digital, false, 10, 0);
        assert_eq!(delay.as_millis(), 50_015);
    }

    #[test]
    fn a_clock_argument_keeps_the_inherited_settings() {
        let config = ClockConfig {
            zone: ZoneChoice::Fixed { seconds: 3600 },
            face: FaceMode::Analog,
            show_seconds: false,
            show_date: false,
            hour24: false,
            always_on_top: true,
            theme: ThemeMode::Dark,
            language: Language::Pl,
            background: Some([1, 2, 3]),
            font_color: None,
            dial_color: None,
            second_hand_color: None,
            window: WindowRect {
                x: 40.0,
                y: 50.0,
                width: 340.0,
                height: 460.0,
            },
        };
        let json = serde_json::to_string(&config).unwrap();
        let launch = Launch::from_iter(["--clock", json.as_str()]);
        assert!(!launch.centered);
        assert_eq!(launch.config, config);
    }

    #[test]
    fn a_bare_launch_opens_centered() {
        let launch = Launch::from_iter(std::iter::empty::<&str>());
        assert!(launch.centered);
        assert!(launch.config.show_date);
    }
}

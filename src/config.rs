use serde::{Deserialize, Serialize};

/// How the time is drawn. Analog mode still shows a small digital readout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaceMode {
    Digital,
    Analog,
}

/// Window theme used when the user has not picked a background color.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

/// Interface language of one clock. Each process keeps its own value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    En = 0,
    Pl = 1,
}

impl Language {
    pub fn from_code(code: u8) -> Self {
        if code == Self::Pl as u8 {
            Self::Pl
        } else {
            Self::En
        }
    }
}

/// A zone chosen by the user.
///
/// `city` is set only when the user picked a city or a country that maps to one
/// representative city. A bare IANA name or a fixed offset leaves it empty.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ZoneChoice {
    Iana { id: String, city: Option<String> },
    Fixed { seconds: i32 },
}

/// Last known window rectangle, in logical points.
///
/// `x`/`y` come from the outer (frame) rectangle and `width`/`height` from the
/// inner (client) rectangle. Kept on the config so a future save can persist
/// it. This version never writes it to disk.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Default for WindowRect {
    fn default() -> Self {
        Self {
            x: 120.0,
            y: 80.0,
            width: 300.0,
            height: 220.0,
        }
    }
}

/// Entire state of one clock. The window reads and writes only this struct.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClockConfig {
    pub zone: ZoneChoice,
    pub face: FaceMode,
    pub show_seconds: bool,
    pub show_date: bool,
    pub hour24: bool,
    pub always_on_top: bool,
    pub theme: ThemeMode,
    pub language: Language,
    pub background: Option<[u8; 3]>,
    /// `None` keeps the theme or contrast color.
    pub font_color: Option<[u8; 3]>,
    /// Dial, numerals, and the hour and minute hands. `None` follows the text color.
    pub dial_color: Option<[u8; 3]>,
    /// Second hand only. `None` keeps the automatic red.
    pub second_hand_color: Option<[u8; 3]>,
    pub window: WindowRect,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrips_through_json() {
        let config = ClockConfig {
            zone: ZoneChoice::Iana {
                id: "Europe/Warsaw".to_string(),
                city: Some("Warszawa".to_string()),
            },
            face: FaceMode::Analog,
            show_seconds: false,
            show_date: true,
            hour24: false,
            always_on_top: true,
            theme: ThemeMode::Dark,
            language: Language::Pl,
            background: Some([20, 40, 80]),
            font_color: Some([10, 20, 30]),
            dial_color: Some([40, 50, 60]),
            second_hand_color: Some([180, 30, 30]),
            window: WindowRect {
                x: 10.0,
                y: 20.0,
                width: 400.0,
                height: 500.0,
            },
        };
        let json = serde_json::to_string(&config).unwrap();
        let back: ClockConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, back);
    }
}

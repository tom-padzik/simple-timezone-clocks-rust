use std::collections::HashMap;
use std::sync::LazyLock;

use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};

use crate::config::Language;

const EN_JSON: &str = include_str!("../locales/en.json");
const PL_JSON: &str = include_str!("../locales/pl.json");

static EN: LazyLock<HashMap<String, String>> = LazyLock::new(|| load(EN_JSON));
static PL: LazyLock<HashMap<String, String>> = LazyLock::new(|| load(PL_JSON));

fn load(json: &str) -> HashMap<String, String> {
    serde_json::from_str(json).unwrap_or_default()
}

fn catalog(language: Language) -> &'static HashMap<String, String> {
    match language {
        Language::En => &EN,
        Language::Pl => &PL,
    }
}

pub fn tr(language: Language, key: &str) -> String {
    catalog(language)
        .get(key)
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

pub fn format_compared(language: Language, delta: &str) -> String {
    tr(language, "compared").replace("{delta}", delta)
}

pub fn format_clock_time(
    civil: NaiveDateTime,
    hour24: bool,
    seconds: bool,
    language: Language,
) -> String {
    let hour = civil.hour();
    let minute = civil.minute();
    let second = civil.second();
    let shown_hour = if hour24 {
        hour
    } else {
        match hour % 12 {
            0 => 12,
            other => other,
        }
    };
    let body = if hour24 && seconds {
        format!("{shown_hour:02}:{minute:02}:{second:02}")
    } else if hour24 {
        format!("{shown_hour:02}:{minute:02}")
    } else if seconds {
        format!("{shown_hour}:{minute:02}:{second:02}")
    } else {
        format!("{shown_hour}:{minute:02}")
    };
    if hour24 {
        body
    } else {
        let suffix = if hour < 12 {
            tr(language, "am")
        } else {
            tr(language, "pm")
        };
        format!("{body} {suffix}")
    }
}

pub fn format_clock_date(date: NaiveDate, language: Language) -> String {
    let weekday = tr(
        language,
        &format!("weekday_{}", date.weekday().num_days_from_monday()),
    );
    let month = tr(language, &format!("month_{}", date.month()));
    format!("{}, {} {} {}", weekday, date.day(), month, date.year())
}

/// Polish or English from the Windows UI language. Anything else is English.
pub fn system_language() -> Language {
    let primary = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() } & 0x3ff;
    if primary == 0x15 {
        Language::Pl
    } else {
        Language::En
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_have_the_same_keys() {
        let english: HashMap<String, serde_json::Value> = serde_json::from_str(EN_JSON).unwrap();
        let polish: HashMap<String, serde_json::Value> = serde_json::from_str(PL_JSON).unwrap();
        let mut english_keys: Vec<_> = english.keys().cloned().collect();
        let mut polish_keys: Vec<_> = polish.keys().cloned().collect();
        english_keys.sort();
        polish_keys.sort();
        assert_eq!(english_keys, polish_keys);
        for day in 0..7 {
            assert!(english.contains_key(&format!("weekday_{day}")));
        }
        for month in 1..=12 {
            assert!(english.contains_key(&format!("month_{month}")));
        }
    }

    #[test]
    fn polish_date_uses_genitive_month() {
        let date = NaiveDate::from_ymd_opt(2026, 10, 17).unwrap();
        assert_eq!(
            format_clock_date(date, Language::Pl),
            "sobota, 17 października 2026"
        );
        assert_eq!(
            format_clock_date(date, Language::En),
            "Saturday, 17 October 2026"
        );
    }
}

use chrono::{DateTime, Local, Offset, TimeZone, Utc};
use chrono_tz::Tz;

use crate::config::{ClockConfig, FaceMode, ThemeMode, ZoneChoice};
use crate::i18n::system_language;

/// Difference shown on the clock: positive means this clock is ahead of the computer.
pub fn delta_seconds(clock_offset: i32, computer_offset: i32) -> i32 {
    clock_offset - computer_offset
}

/// `+6h`, `-1h`, `0h`, `+5h 30m`, `-1h 30m`.
pub fn format_delta(total_seconds: i32) -> String {
    if total_seconds == 0 {
        return "0h".to_string();
    }
    let sign = if total_seconds > 0 { '+' } else { '-' };
    let abs = total_seconds.unsigned_abs();
    let hours = abs / 3600;
    let minutes = (abs % 3600) / 60;
    if minutes == 0 {
        format!("{sign}{hours}h")
    } else {
        format!("{sign}{hours}h {minutes}m")
    }
}

/// `UTC`, `UTC+1`, `UTC-5`, `UTC+5:30`.
pub fn format_utc_label(total_seconds: i32) -> String {
    if total_seconds == 0 {
        return "UTC".to_string();
    }
    let sign = if total_seconds > 0 { '+' } else { '-' };
    let abs = total_seconds.unsigned_abs();
    let hours = abs / 3600;
    let minutes = (abs % 3600) / 60;
    if minutes == 0 {
        format!("UTC{sign}{hours}")
    } else {
        format!("UTC{sign}{hours}:{minutes:02}")
    }
}

/// Parses `UTC`, `GMT`, `UTC+1`, `UTC-5`, `UTC+5:30`, `UTC+0530`.
///
/// Minutes must be 00, 30, or 45. The result must sit in `-12:00..=+14:00`.
pub fn parse_offset(input: &str) -> Option<i32> {
    let compact: String = input
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(|c| c.to_lowercase())
        .collect();
    let rest = compact
        .strip_prefix("utc")
        .or_else(|| compact.strip_prefix("gmt"))?;
    if rest.is_empty() {
        return Some(0);
    }
    let (sign, number) = if let Some(rest) = rest.strip_prefix('+') {
        (1_i32, rest)
    } else if let Some(rest) = rest.strip_prefix('-') {
        (-1, rest)
    } else {
        (1, rest)
    };
    if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit() || c == ':') {
        return None;
    }
    let (hours_text, minutes_text) = if let Some((hours, minutes)) = number.split_once(':') {
        (hours, minutes)
    } else {
        match number.len() {
            1 | 2 => (number, ""),
            3 => (&number[..1], &number[1..]),
            4 => (&number[..2], &number[2..]),
            _ => return None,
        }
    };
    if hours_text.is_empty() || !hours_text.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let hours: i32 = hours_text.parse().ok()?;
    let minutes: i32 = if minutes_text.is_empty() {
        0
    } else if minutes_text.len() == 2 && minutes_text.chars().all(|c| c.is_ascii_digit()) {
        minutes_text.parse().ok()?
    } else {
        return None;
    };
    if !matches!(minutes, 0 | 30 | 45) {
        return None;
    }
    let total = sign * (hours * 3600 + minutes * 60);
    if (-12 * 3600..=14 * 3600).contains(&total) {
        Some(total)
    } else {
        None
    }
}

pub fn zone_offset_seconds(choice: &ZoneChoice, now: DateTime<Utc>) -> i32 {
    match choice {
        ZoneChoice::Fixed { seconds } => *seconds,
        ZoneChoice::Iana { id, .. } => id
            .parse::<Tz>()
            .map(|tz| {
                tz.offset_from_utc_datetime(&now.naive_utc())
                    .fix()
                    .local_minus_utc()
            })
            .unwrap_or(0),
    }
}

pub fn local_iana() -> String {
    iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".to_string())
}

pub fn computer_offset_seconds(now: DateTime<Utc>) -> i32 {
    let name = local_iana();
    if let Ok(tz) = name.parse::<Tz>() {
        tz.offset_from_utc_datetime(&now.naive_utc())
            .fix()
            .local_minus_utc()
    } else {
        Local
            .from_utc_datetime(&now.naive_utc())
            .offset()
            .fix()
            .local_minus_utc()
    }
}

pub fn zone_name(choice: &ZoneChoice) -> String {
    match choice {
        ZoneChoice::Iana { id, .. } => id.clone(),
        ZoneChoice::Fixed { seconds } => format_utc_label(*seconds),
    }
}

pub fn window_title(choice: &ZoneChoice) -> String {
    match choice {
        ZoneChoice::Iana {
            city: Some(city), ..
        } if !city.is_empty() => city.clone(),
        other => zone_name(other),
    }
}

pub fn city_label(choice: &ZoneChoice) -> Option<&str> {
    match choice {
        ZoneChoice::Iana {
            city: Some(city), ..
        } if !city.is_empty() => Some(city.as_str()),
        _ => None,
    }
}

/// A new clock in the computer's own zone, with no city label yet.
pub fn default_config() -> ClockConfig {
    let iana = local_iana();
    let zone = if iana.parse::<Tz>().is_ok() {
        ZoneChoice::Iana {
            id: iana,
            city: None,
        }
    } else {
        ZoneChoice::Fixed {
            seconds: computer_offset_seconds(Utc::now()),
        }
    };
    ClockConfig {
        zone,
        face: FaceMode::Digital,
        show_seconds: true,
        show_date: true,
        hour24: true,
        always_on_top: false,
        theme: ThemeMode::System,
        language: system_language(),
        background: None,
        font_color: None,
        dial_color: None,
        second_hand_color: None,
        window: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_is_clock_minus_computer() {
        assert_eq!(delta_seconds(9 * 3600, 2 * 3600), 7 * 3600);
        assert_eq!(format_delta(7 * 3600), "+7h");
        assert_eq!(format_delta(-3600), "-1h");
        assert_eq!(format_delta(0), "0h");
        assert_eq!(format_delta(5 * 3600 + 30 * 60), "+5h 30m");
        assert_eq!(format_delta(15 * 60), "+0h 15m");
        assert_eq!(format_delta(-90 * 60), "-1h 30m");
    }

    #[test]
    fn utc_labels_skip_zero_minutes() {
        assert_eq!(format_utc_label(0), "UTC");
        assert_eq!(format_utc_label(3600), "UTC+1");
        assert_eq!(format_utc_label(-5 * 3600), "UTC-5");
        assert_eq!(format_utc_label(5 * 3600 + 30 * 60), "UTC+5:30");
    }

    #[test]
    fn parses_utc_and_gmt_offsets() {
        assert_eq!(parse_offset("UTC"), Some(0));
        assert_eq!(parse_offset("gmt"), Some(0));
        assert_eq!(parse_offset(" UTC + 1 "), Some(3600));
        assert_eq!(parse_offset("UTC-5"), Some(-5 * 3600));
        assert_eq!(parse_offset("UTC+5:30"), Some(5 * 3600 + 30 * 60));
        assert_eq!(parse_offset("UTC+05:30"), Some(5 * 3600 + 30 * 60));
        assert_eq!(parse_offset("utc+0530"), Some(5 * 3600 + 30 * 60));
        assert_eq!(parse_offset("UTC-3:30"), Some(-(3 * 3600 + 30 * 60)));
        assert_eq!(parse_offset("UTC+5:45"), Some(5 * 3600 + 45 * 60));
        assert_eq!(parse_offset("UTC+14"), Some(14 * 3600));
        assert_eq!(parse_offset("UTC+15"), None);
        assert_eq!(parse_offset("UTC+5:20"), None);
        assert_eq!(parse_offset("Warsaw"), None);
    }

    #[test]
    fn named_zone_follows_dst_and_fixed_offset_does_not() {
        let winter = "2026-01-15T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let summer = "2026-07-15T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let warsaw = ZoneChoice::Iana {
            id: "Europe/Warsaw".to_string(),
            city: None,
        };
        let fixed = ZoneChoice::Fixed { seconds: 3600 };
        assert_eq!(zone_offset_seconds(&warsaw, winter), 3600);
        assert_eq!(zone_offset_seconds(&warsaw, summer), 7200);
        assert_eq!(zone_offset_seconds(&fixed, winter), 3600);
        assert_eq!(zone_offset_seconds(&fixed, summer), 3600);
    }
}

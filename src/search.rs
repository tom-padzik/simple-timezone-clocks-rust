use std::collections::{HashMap, HashSet};

use chrono_tz::Tz;
use serde_json::Value;

use crate::config::{Language, ZoneChoice};
use crate::zone::{format_utc_label, parse_offset};

const ZONE1970: &str = include_str!("../data/zone1970.tab");
const TERRITORIES_EN: &str = include_str!("../data/territories-en.json");
const TERRITORIES_PL: &str = include_str!("../data/territories-pl.json");

/// Extra names that should find a real IANA zone.
/// Polish cities inside Poland share Europe/Warsaw, the country's only zone.
const CITY_ALIASES: &str = r#"
Warszawa	Europe/Warsaw
Kraków	Europe/Warsaw
Wrocław	Europe/Warsaw
Gdańsk	Europe/Warsaw
Poznań	Europe/Warsaw
Łódź	Europe/Warsaw
Szczecin	Europe/Warsaw
Lublin	Europe/Warsaw
Katowice	Europe/Warsaw
Białystok	Europe/Warsaw
Londyn	Europe/London
Paryż	Europe/Paris
Madryt	Europe/Madrid
Rzym	Europe/Rome
Wiedeń	Europe/Vienna
Praga	Europe/Prague
Budapeszt	Europe/Budapest
Moskwa	Europe/Moscow
Kijów	Europe/Kyiv
Ateny	Europe/Athens
Lizbona	Europe/Lisbon
Bruksela	Europe/Brussels
Amsterdam	Europe/Amsterdam
Sztokholm	Europe/Stockholm
Kopenhaga	Europe/Copenhagen
Zurych	Europe/Zurich
Tokio	Asia/Tokyo
Pekin	Asia/Shanghai
Beijing	Asia/Shanghai
Szanghaj	Asia/Shanghai
Hongkong	Asia/Hong_Kong
Seul	Asia/Seoul
Singapur	Asia/Singapore
Delhi	Asia/Kolkata
Bombaj	Asia/Kolkata
Kalkuta	Asia/Kolkata
Dubaj	Asia/Dubai
Stambuł	Europe/Istanbul
Kair	Africa/Cairo
Meksyk	America/Mexico_City
Nowy Jork	America/New_York
NYC	America/New_York
Waszyngton	America/New_York
Washington	America/New_York
Miami	America/New_York
Boston	America/New_York
Atlanta	America/New_York
Filadelfia	America/New_York
Dallas	America/Chicago
Houston	America/Chicago
Seattle	America/Los_Angeles
San Francisco	America/Los_Angeles
Las Vegas	America/Los_Angeles
Los Angeles	America/Los_Angeles
Ottawa	America/Toronto
Montreal	America/Toronto
São Paulo	America/Sao_Paulo
Brasilia	America/Sao_Paulo
Buenos Aires	America/Argentina/Buenos_Aires
Hanoi	Asia/Ho_Chi_Minh
Rijad	Asia/Riyadh
Jerozolima	Asia/Jerusalem
Teheran	Asia/Tehran
Katmandu	Asia/Kathmandu
Dżakarta	Asia/Jakarta
Tajpej	Asia/Taipei
Hawana	America/Havana
Rejkiawik	Atlantic/Reykjavik
Belgrad	Europe/Belgrade
Bukareszt	Europe/Bucharest
Zagrzeb	Europe/Zagreb
Bratysława	Europe/Bratislava
Lublana	Europe/Ljubljana
Wilno	Europe/Vilnius
Ryga	Europe/Riga
Tallinn	Europe/Tallinn
Mińsk	Europe/Minsk
Nairobi	Africa/Nairobi
Lagos	Africa/Lagos
Casablanca	Africa/Casablanca
Algier	Africa/Algiers
Tunis	Africa/Tunis
Bejrut	Asia/Beirut
Amman	Asia/Amman
Damaszek	Asia/Damascus
Bagdad	Asia/Baghdad
Doha	Asia/Qatar
Kuwejt	Asia/Kuwait
Kabul	Asia/Kabul
Tbilisi	Asia/Tbilisi
Erywań	Asia/Yerevan
Baku	Asia/Baku
Ałmaty	Asia/Almaty
Taszkent	Asia/Tashkent
Biszkek	Asia/Bishkek
Ułan Bator	Asia/Ulaanbaatar
Pjongjang	Asia/Pyongyang
Kolombo	Asia/Colombo
Dhaka	Asia/Dhaka
Canberra	Australia/Sydney
Wellington	Pacific/Auckland
Luksemburg	Europe/Luxembourg
Monako	Europe/Monaco
Andora	Europe/Andorra
Nikozja	Asia/Nicosia
Sarajewo	Europe/Sarajevo
Skopje	Europe/Skopje
Tirana	Europe/Tirane
Podgorica	Europe/Podgorica
Kiszyniów	Europe/Chisinau
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum HitKind {
    Offset,
    City,
    Country,
    Zone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub kind: HitKind,
    pub primary: String,
    pub secondary: String,
    pub choice: ZoneChoice,
}

struct Entry {
    keys: Vec<String>,
    kind: HitKind,
    city: Option<String>,
    country: Option<String>,
    zone_id: String,
}

struct ZoneRow {
    countries: Vec<String>,
    id: String,
    comments: String,
}

pub struct ZoneIndex {
    entries: Vec<Entry>,
    countries_en: HashMap<String, String>,
    countries_pl: HashMap<String, String>,
}

impl ZoneIndex {
    pub fn load() -> Self {
        let countries_en = load_countries(TERRITORIES_EN, "en");
        let countries_pl = load_countries(TERRITORIES_PL, "pl");
        let rows = parse_zone_table();
        let mut entries = Vec::new();
        let mut seen_zones = HashSet::new();
        for tz in chrono_tz::TZ_VARIANTS {
            let id = tz.name();
            if id.starts_with("Etc/") || !seen_zones.insert(id.to_string()) {
                continue;
            }
            let city = city_from_id(id);
            entries.push(zone_entry(id));
            entries.push(city_entry(id, &city, &[&city]));
        }

        for row in &rows {
            if row.id.parse::<Tz>().is_err() {
                continue;
            }
            let city = city_from_id(&row.id);
            for code in &row.countries {
                let mut keys = vec![
                    code.clone(),
                    countries_en.get(code).cloned().unwrap_or_default(),
                    countries_pl.get(code).cloned().unwrap_or_default(),
                    row.comments.clone(),
                ];
                keys.extend(extra_country_keys(code).iter().map(|s| (*s).to_string()));
                entries.push(Entry {
                    keys: normalize_keys(&keys),
                    kind: HitKind::Country,
                    city: Some(city.clone()),
                    country: Some(code.clone()),
                    zone_id: row.id.clone(),
                });
            }
        }

        for line in CITY_ALIASES.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((alias, zone)) = line.split_once('\t') else {
                continue;
            };
            if zone.parse::<Tz>().is_err() {
                continue;
            }
            let canonical = city_from_id(zone);
            if normalize(alias) == normalize(&canonical) {
                continue;
            }
            entries.push(city_entry(zone, alias, &[alias]));
        }

        Self {
            entries,
            countries_en,
            countries_pl,
        }
    }

    pub fn search(&self, query: &str, language: Language) -> Vec<SearchHit> {
        let normalized = normalize(query);
        if normalized.is_empty() {
            return Vec::new();
        }
        if normalized.starts_with("utc") || normalized.starts_with("gmt") {
            if let Some(seconds) = parse_offset(query) {
                return vec![SearchHit {
                    kind: HitKind::Offset,
                    primary: format_utc_label(seconds),
                    secondary: String::new(),
                    choice: ZoneChoice::Fixed { seconds },
                }];
            }
        }

        let mut found: Vec<(u8, usize)> = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            if let Some(score) = entry
                .keys
                .iter()
                .filter_map(|key| score_key(key, &normalized))
                .min()
            {
                found.push((score, index));
            }
        }
        found.sort_by(|a, b| {
            let left = &self.entries[a.1];
            let right = &self.entries[b.1];
            a.0.cmp(&b.0)
                .then(left.kind.cmp(&right.kind))
                .then(left.zone_id.cmp(&right.zone_id))
                .then(left.city.cmp(&right.city))
        });

        let mut hits = Vec::new();
        let mut seen = HashSet::new();
        for (_, index) in found {
            let entry = &self.entries[index];
            let choice = ZoneChoice::Iana {
                id: entry.zone_id.clone(),
                city: match entry.kind {
                    HitKind::Zone => None,
                    _ => entry.city.clone(),
                },
            };
            let key = format!("{:?}|{}|{:?}", entry.kind, entry.zone_id, entry.city);
            if !seen.insert(key) {
                continue;
            }
            hits.push(self.hit_from(entry, choice, language));
            if hits.len() == 40 {
                break;
            }
        }
        hits
    }

    fn hit_from(&self, entry: &Entry, choice: ZoneChoice, language: Language) -> SearchHit {
        let secondary = entry.zone_id.clone();
        let primary = match entry.kind {
            HitKind::City | HitKind::Zone => {
                entry.city.clone().unwrap_or_else(|| secondary.clone())
            }
            HitKind::Country => {
                let code = entry.country.as_deref().unwrap_or("");
                let country = self.country_name(code, language);
                let city = entry.city.clone().unwrap_or_default();
                if city.is_empty() {
                    country
                } else {
                    format!("{country} — {city}")
                }
            }
            HitKind::Offset => String::new(),
        };
        let secondary = if entry.kind == HitKind::Zone {
            String::new()
        } else {
            secondary
        };
        let primary = if entry.kind == HitKind::Zone {
            entry.zone_id.clone()
        } else {
            primary
        };
        SearchHit {
            kind: entry.kind,
            primary,
            secondary,
            choice,
        }
    }

    fn country_name(&self, code: &str, language: Language) -> String {
        let preferred = match language {
            Language::Pl => self
                .countries_pl
                .get(code)
                .or_else(|| self.countries_en.get(code)),
            Language::En => self.countries_en.get(code),
        };
        preferred.cloned().unwrap_or_else(|| code.to_string())
    }
}

fn zone_entry(id: &str) -> Entry {
    Entry {
        keys: normalize_keys(&[id, &id.replace('_', " ")]),
        kind: HitKind::Zone,
        city: None,
        country: None,
        zone_id: id.to_string(),
    }
}

fn city_entry(id: &str, city: &str, raw_keys: &[&str]) -> Entry {
    Entry {
        keys: normalize_keys(raw_keys),
        kind: HitKind::City,
        city: Some(city.to_string()),
        country: None,
        zone_id: id.to_string(),
    }
}

fn city_from_id(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).replace('_', " ")
}

fn extra_country_keys(code: &str) -> &'static [&'static str] {
    match code {
        "US" => &["usa", "us"],
        "GB" => &["uk", "britain"],
        "AE" => &["uae"],
        "CZ" => &["czechia", "czechy"],
        "NL" => &["holland", "holandia"],
        _ => &[],
    }
}

fn parse_zone_table() -> Vec<ZoneRow> {
    ZONE1970
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let codes = parts.next()?;
            let _coordinates = parts.next()?;
            let id = parts.next()?;
            let comments = parts.next().unwrap_or("");
            Some(ZoneRow {
                countries: codes
                    .split(',')
                    .filter(|code| !code.is_empty())
                    .map(str::to_string)
                    .collect(),
                id: id.to_string(),
                comments: comments.to_string(),
            })
        })
        .collect()
}

fn load_countries(json: &str, language_key: &str) -> HashMap<String, String> {
    let value: Value = serde_json::from_str(json).unwrap_or(Value::Null);
    let Some(territories) = value
        .get("main")
        .and_then(|main| main.get(language_key))
        .and_then(|lang| lang.get("localeDisplayNames"))
        .and_then(|names| names.get("territories"))
        .and_then(Value::as_object)
    else {
        return HashMap::new();
    };
    territories
        .iter()
        .filter(|(code, _)| code.len() == 2 && code.chars().all(|c| c.is_ascii_uppercase()))
        .filter_map(|(code, name)| name.as_str().map(|name| (code.clone(), name.to_string())))
        .collect()
}

fn normalize_keys(raw: &[impl AsRef<str>]) -> Vec<String> {
    let mut keys = Vec::new();
    for value in raw {
        let key = normalize(value.as_ref());
        if key.len() >= 2 && !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

fn normalize(input: &str) -> String {
    let mut out = String::new();
    let mut pending_space = false;
    for ch in input.chars() {
        for folded in fold_char(ch).chars() {
            if folded.is_whitespace() {
                pending_space = true;
                continue;
            }
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(folded);
        }
    }
    out
}

fn fold_char(ch: char) -> String {
    let lowered: String = ch.to_lowercase().collect();
    let mut out = String::new();
    for ch in lowered.chars() {
        match ch {
            'ą' | 'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => out.push('a'),
            'ć' | 'č' | 'ç' => out.push('c'),
            'ę' | 'é' | 'è' | 'ê' | 'ë' | 'ě' => out.push('e'),
            'ł' => out.push('l'),
            'ń' | 'ñ' => out.push('n'),
            'ó' | 'ø' => out.push('o'),
            'ś' | 'š' | 'ş' => out.push('s'),
            'ź' | 'ż' => out.push('z'),
            'í' | 'ì' | 'î' | 'ï' | 'ı' => out.push('i'),
            'ú' | 'ù' | 'û' | 'ü' | 'ů' | 'ū' => out.push('u'),
            'ý' | 'ÿ' => out.push('y'),
            'ř' => out.push('r'),
            'đ' => out.push('d'),
            'ğ' => out.push('g'),
            'æ' => out.push_str("ae"),
            'œ' => out.push_str("oe"),
            'ß' => out.push_str("ss"),
            other => out.push(other),
        }
    }
    out
}

fn score_key(key: &str, query: &str) -> Option<u8> {
    if query.len() < 2 {
        return None;
    }
    if key == query {
        return Some(0);
    }
    if key.starts_with(query) {
        return Some(1);
    }
    if query.len() < 3 {
        return None;
    }
    let mut word_start = true;
    for (index, ch) in key.char_indices() {
        if word_start && key[index..].starts_with(query) {
            return Some(2);
        }
        word_start = !ch.is_ascii_alphanumeric();
    }
    if key.contains(query) {
        Some(3)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_city_country_zone_and_offset() {
        let index = ZoneIndex::load();

        let warsaw = index.search("Warszawa", Language::Pl);
        assert!(warsaw.iter().any(|hit| {
            hit.choice
                == ZoneChoice::Iana {
                    id: "Europe/Warsaw".to_string(),
                    city: Some("Warszawa".to_string()),
                }
        }));

        let poland = index.search("Polska", Language::Pl);
        assert!(poland.iter().any(|hit| {
            matches!(
                &hit.choice,
                ZoneChoice::Iana { id, city: Some(city) }
                    if id == "Europe/Warsaw" && city == "Warsaw"
            )
        }));

        let zone = index.search("Europe/Warsaw", Language::En);
        assert!(zone.iter().any(|hit| {
            hit.kind == HitKind::Zone
                && hit.choice
                    == ZoneChoice::Iana {
                        id: "Europe/Warsaw".to_string(),
                        city: None,
                    }
        }));

        let states = index.search("United States", Language::En);
        let ids: HashSet<_> = states
            .iter()
            .filter_map(|hit| match &hit.choice {
                ZoneChoice::Iana { id, .. } => Some(id.clone()),
                ZoneChoice::Fixed { .. } => None,
            })
            .collect();
        assert!(ids.len() > 1, "a country with many zones must not collapse");

        let offset = index.search("UTC+5:30", Language::En);
        assert_eq!(
            offset,
            vec![SearchHit {
                kind: HitKind::Offset,
                primary: "UTC+5:30".to_string(),
                secondary: String::new(),
                choice: ZoneChoice::Fixed { seconds: 19800 },
            }]
        );
    }
}

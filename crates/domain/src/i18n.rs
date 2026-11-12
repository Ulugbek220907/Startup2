//! Translations for Uzbek (Latin), Russian and English.
//!
//! Strings live in `locales/*.json` (flat `"key": "text"` maps) and are
//! embedded at compile time. The same files serve the web UI, the Telegram
//! bot and server-side messages. A test checks that all three languages
//! define exactly the same keys.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    #[default]
    Uz,
    Ru,
    En,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::Uz, Lang::Ru, Lang::En];

    pub fn code(&self) -> &'static str {
        match self {
            Lang::Uz => "uz",
            Lang::Ru => "ru",
            Lang::En => "en",
        }
    }

    /// Name of the language in that language, for the picker.
    pub fn native_name(&self) -> &'static str {
        match self {
            Lang::Uz => "Oʻzbekcha",
            Lang::Ru => "Русский",
            Lang::En => "English",
        }
    }

    pub fn parse(code: &str) -> Option<Lang> {
        match code {
            "uz" => Some(Lang::Uz),
            "ru" => Some(Lang::Ru),
            "en" => Some(Lang::En),
            _ => None,
        }
    }

    /// Pick a language from a browser/Telegram tag such as `ru-RU`, `uz`,
    /// `uz-Cyrl-UZ` or `en_US`. Unknown languages fall back to Uzbek;
    /// Kazakh/Tajik/etc. speakers in the region usually read Russian, so
    /// other CIS language tags map to Russian.
    pub fn detect(tag: &str) -> Lang {
        let primary = tag.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
        match primary.as_str() {
            "uz" => Lang::Uz,
            "ru" | "kk" | "ky" | "tg" | "be" | "uk" => Lang::Ru,
            "en" => Lang::En,
            _ => Lang::Uz,
        }
    }

    /// First supported language in a list of preferences
    /// (`navigator.languages`, or an `Accept-Language` header split on `,`).
    pub fn detect_from<'a>(tags: impl IntoIterator<Item = &'a str>) -> Lang {
        for tag in tags {
            let tag = tag.split(';').next().unwrap_or("").trim();
            let primary = tag.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
            if matches!(primary.as_str(), "uz" | "ru" | "en" | "kk" | "ky" | "tg" | "be" | "uk") {
                return Lang::detect(tag);
            }
        }
        Lang::Uz
    }
}

type Table = HashMap<String, String>;

fn tables() -> &'static [Table; 3] {
    // `OnceLock` initialises the value on first use and then hands out a
    // shared reference; the JSON is parsed once per process.
    static TABLES: OnceLock<[Table; 3]> = OnceLock::new();
    TABLES.get_or_init(|| {
        let parse = |src: &str| -> Table { serde_json::from_str(src).expect("locale file is valid JSON (checked by tests)") };
        [parse(include_str!("../locales/uz.json")), parse(include_str!("../locales/ru.json")), parse(include_str!("../locales/en.json"))]
    })
}

fn table(lang: Lang) -> &'static Table {
    &tables()[match lang {
        Lang::Uz => 0,
        Lang::Ru => 1,
        Lang::En => 2,
    }]
}

/// Translate `key`. A missing key returns the key itself so the gap is
/// visible in the UI instead of crashing.
pub fn t(lang: Lang, key: &str) -> &'static str {
    match table(lang).get(key) {
        Some(s) => s.as_str(),
        None => match table(Lang::En).get_key_value(key) {
            Some((_, s)) => s.as_str(),
            // Leak-free fallback: return a static marker; callers that need
            // the key text use `t_owned`.
            None => "⟨?⟩",
        },
    }
}

/// Like [`t`] but returns the key when missing, and substitutes `{name}`
/// placeholders from `args`.
pub fn tf(lang: Lang, key: &str, args: &[(&str, &str)]) -> String {
    let mut s = match table(lang).get(key).or_else(|| table(Lang::En).get(key)) {
        Some(s) => s.clone(),
        None => return key.to_string(),
    };
    for (name, value) in args {
        s = s.replace(&format!("{{{name}}}"), value);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn all_languages_have_the_same_keys() {
        let keys = |l: Lang| table(l).keys().cloned().collect::<BTreeSet<_>>();
        let uz = keys(Lang::Uz);
        for l in [Lang::Ru, Lang::En] {
            let other = keys(l);
            let missing: Vec<_> = uz.difference(&other).collect();
            let extra: Vec<_> = other.difference(&uz).collect();
            assert!(missing.is_empty() && extra.is_empty(), "{l:?}: missing {missing:?}, extra {extra:?}");
        }
        assert!(uz.len() > 50);
    }

    #[test]
    fn no_empty_strings_and_placeholders_match() {
        for (key, uz) in table(Lang::Uz) {
            let holes = |s: &str| {
                let mut v: Vec<String> = s.split('{').skip(1).filter_map(|p| p.split('}').next().map(str::to_string)).collect();
                v.sort();
                v
            };
            for l in Lang::ALL {
                let s = table(l).get(key).unwrap();
                assert!(!s.trim().is_empty(), "{l:?} {key} is empty");
                assert_eq!(holes(s), holes(uz), "placeholders differ for {key} in {l:?}");
            }
        }
    }

    #[test]
    fn uzbek_uses_the_proper_modifier_letter() {
        // oʻ and gʻ must be written with U+02BB, not an ASCII apostrophe.
        for (key, s) in table(Lang::Uz) {
            assert!(!s.contains("o'") && !s.contains("g'") && !s.contains("O'") && !s.contains("G'"), "{key}: {s}");
        }
    }

    #[test]
    fn detection() {
        assert_eq!(Lang::detect("ru-RU"), Lang::Ru);
        assert_eq!(Lang::detect("uz-Cyrl-UZ"), Lang::Uz);
        assert_eq!(Lang::detect("en_US"), Lang::En);
        assert_eq!(Lang::detect("kk"), Lang::Ru);
        assert_eq!(Lang::detect("de"), Lang::Uz);
        assert_eq!(Lang::detect_from(["de-DE", "en;q=0.8", "ru"]), Lang::En);
        assert_eq!(Lang::detect_from(["fr"]), Lang::Uz);
    }

    #[test]
    fn formatting() {
        assert_eq!(tf(Lang::En, "definitely.missing", &[]), "definitely.missing");
        assert_eq!(t(Lang::En, "definitely.missing"), "⟨?⟩");
    }
}

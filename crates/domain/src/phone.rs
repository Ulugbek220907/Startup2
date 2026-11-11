//! Uzbek mobile numbers, normalised to `+998XXXXXXXXX`.

/// Normalise user input to `+998XXXXXXXXX`.
///
/// Accepts the forms people actually type: `901234567`, `90 123 45 67`,
/// `(90) 123-45-67`, `998901234567`, `+998 90 123 45 67` and the old
/// trunk-prefix form `8 90 123 45 67`.
pub fn normalize(input: &str) -> Result<String, PhoneError> {
    let mut digits = String::with_capacity(12);
    for c in input.chars() {
        match c {
            '0'..='9' => digits.push(c),
            ' ' | '-' | '(' | ')' | '+' | '.' | '\u{00A0}' | '\u{202F}' => {}
            _ => return Err(PhoneError::InvalidCharacter),
        }
    }
    if digits.len() > 15 {
        return Err(PhoneError::WrongLength);
    }
    let national: &str = match digits.len() {
        9 => &digits,
        12 if digits.starts_with("998") => &digits[3..],
        10 if digits.starts_with('8') => &digits[1..],
        12 | 10 => return Err(PhoneError::NotUzbek),
        _ => return Err(PhoneError::WrongLength),
    };
    // The two-digit operator/area code never starts with 0 or 1.
    if matches!(national.as_bytes()[0], b'0' | b'1') {
        return Err(PhoneError::NotUzbek);
    }
    Ok(format!("+998{national}"))
}

/// `+998901234567` -> `+998 90 123 45 67` for display.
pub fn format_display(normalized: &str) -> String {
    if normalized.len() == 13 && normalized.starts_with("+998") && normalized.is_ascii() {
        let n = &normalized[4..];
        format!("+998 {} {} {} {}", &n[0..2], &n[2..5], &n[5..7], &n[7..9])
    } else {
        normalized.to_string()
    }
}

/// Hide the middle digits for logs: `+998 90 *** ** 67`.
pub fn mask(normalized: &str) -> String {
    if normalized.len() == 13 && normalized.is_ascii() {
        format!("+998 {} *** ** {}", &normalized[4..6], &normalized[11..13])
    } else {
        "***".to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhoneError {
    InvalidCharacter,
    WrongLength,
    NotUzbek,
}

impl std::fmt::Display for PhoneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PhoneError::InvalidCharacter => "phone number contains invalid characters",
            PhoneError::WrongLength => "phone number has the wrong length",
            PhoneError::NotUzbek => "not an Uzbekistan phone number",
        })
    }
}

impl std::error::Error for PhoneError {}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn accepts_common_spellings() {
        for s in [
            "901234567",
            "90 123 45 67",
            "(90) 123-45-67",
            "998901234567",
            "+998901234567",
            "+998 90 123 45 67",
            "8 90 123 45 67",
            "+998 (90) 123-45-67",
        ] {
            assert_eq!(normalize(s).as_deref(), Ok("+998901234567"), "{s}");
        }
    }

    #[test]
    fn rejects_bad_numbers() {
        assert_eq!(normalize("12345"), Err(PhoneError::WrongLength));
        assert_eq!(normalize(""), Err(PhoneError::WrongLength));
        assert_eq!(normalize("+7 901 123 45 67"), Err(PhoneError::WrongLength));
        assert_eq!(normalize("791234567890"), Err(PhoneError::NotUzbek));
        assert_eq!(normalize("001234567"), Err(PhoneError::NotUzbek));
        assert_eq!(normalize("90 123 45 6a"), Err(PhoneError::InvalidCharacter));
        assert_eq!(normalize("90123456789012345678"), Err(PhoneError::WrongLength));
    }

    #[test]
    fn display_and_mask() {
        assert_eq!(format_display("+998901234567"), "+998 90 123 45 67");
        assert_eq!(mask("+998901234567"), "+998 90 *** ** 67");
        assert_eq!(mask("garbage"), "***");
    }

    proptest! {
        #[test]
        fn normalize_is_idempotent(code in 20u32..100, rest in 0u32..10_000_000) {
            let n = normalize(&format!("{code}{rest:07}")).unwrap();
            prop_assert_eq!(n.len(), 13);
            prop_assert_eq!(normalize(&n).unwrap(), n.clone());
            prop_assert_eq!(normalize(&format_display(&n)).unwrap(), n);
        }

        #[test]
        fn never_panics(s in ".{0,40}") {
            let _ = normalize(&s);
            let _ = mask(&s);
            let _ = format_display(&s);
        }
    }
}

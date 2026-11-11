//! Text blocklist matching for design moderation.
//!
//! A match does not reject an order; it flags the design for a human to
//! review. So the matcher leans towards catching obfuscated spellings
//! (`s.o.‘.z`, `С0ʻЗ`) over avoiding every false positive.

/// Lower-case, fold look-alike characters and drop everything that is not a
/// letter or digit, so `B-A_D w0rd` becomes `badword`.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(|c| c.to_lowercase()) {
        let folded = match c {
            '0' => 'o',
            '1' | '!' | '|' => 'i',
            '3' => 'e',
            '4' | '@' => 'a',
            '5' | '$' => 's',
            '7' => 't',
            'ё' => 'е',
            other => other,
        };
        if folded.is_alphanumeric() {
            out.push(folded);
        }
    }
    out
}

/// Returns the first blocklist term found in `text`, if any. Terms are
/// matched as substrings after normalisation; very short terms (< 3 letters)
/// are ignored because they would match inside innocent words.
pub fn find_blocked<'a>(text: &str, terms: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let haystack = normalize(text);
    if haystack.is_empty() {
        return None;
    }
    terms.into_iter().find(|term| {
        let needle = normalize(term);
        needle.chars().count() >= 3 && haystack.contains(&needle)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_obfuscated_spellings() {
        let terms = ["badword", "плохое"];
        assert_eq!(find_blocked("this is a B-A-D w0rd!", terms), Some("badword"));
        assert_eq!(find_blocked("ПЛОХОЕ слово", terms), Some("плохое"));
        assert_eq!(find_blocked("b.a.d\nw.o.r.d", terms), Some("badword"));
        assert_eq!(find_blocked("Oʻzbekiston — vatanim", terms), None);
        assert_eq!(find_blocked("", terms), None);
    }

    #[test]
    fn ignores_too_short_terms() {
        assert_eq!(find_blocked("classic", ["as", ""]), None);
    }
}

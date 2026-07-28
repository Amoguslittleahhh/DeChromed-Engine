//! Named character reference table for A2's character-reference states
//! (`NamedCharacterReference` etc).
//!
//! Vendored from the WHATWG HTML spec's `entities.json`
//! (<https://html.spec.whatwg.org/entities.json>, mirrored via
//! `raw.githubusercontent.com/w3c/html`) at `vendor/entities.json`, with the
//! leading `&` stripped from each key so lookups are keyed by the name that
//! appears *after* the `&` in source text (e.g. `"amp;"`, `"amp"`,
//! `"acute;"`). Both the with-semicolon and legacy without-semicolon forms
//! are present as separate keys, exactly as the spec defines them -- the
//! tokenizer decides which one applies based on context (see
//! `NamedCharacterReference` state in `tokenizer.rs`).

use std::collections::HashMap;
use std::sync::OnceLock;

static TABLE: OnceLock<HashMap<String, Vec<u32>>> = OnceLock::new();

fn table() -> &'static HashMap<String, Vec<u32>> {
    TABLE.get_or_init(|| {
        let raw = include_str!("../vendor/entities.json");
        let parsed: HashMap<String, Vec<u32>> =
            serde_json::from_str(raw).expect("vendor/entities.json is valid JSON");
        parsed
    })
}

/// Longest-prefix match of a named character reference against `input`
/// (the text immediately following the `&`), per the spec's
/// `NamedCharacterReference` state: try progressively shorter prefixes of
/// `input` until one matches a known entity name, and return the matched
/// name's length (in chars) and its codepoints. Returns `None` if no prefix
/// matches at all.
pub fn longest_match(input: &str) -> Option<(usize, &'static [u32])> {
    let t = table();
    // Entity names are at most this many chars (checked against the vendored
    // table at generation time; kept as a constant so we don't rescan the
    // whole table on every lookup).
    const MAX_NAME_LEN: usize = 32;

    let chars: Vec<char> = input.chars().take(MAX_NAME_LEN).collect();
    for len in (1..=chars.len()).rev() {
        let candidate: String = chars[..len].iter().collect();
        if let Some(codepoints) = t.get(&candidate) {
            return Some((len, codepoints.as_slice()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_semicolon_form() {
        let (len, cps) = longest_match("amp;rest").unwrap();
        assert_eq!(len, 4);
        assert_eq!(cps, &[38]);
    }

    #[test]
    fn matches_legacy_form_without_semicolon() {
        let (len, cps) = longest_match("amp rest").unwrap();
        assert_eq!(len, 3);
        assert_eq!(cps, &[38]);
    }

    #[test]
    fn prefers_longest_match() {
        // "notin;" should match the full "notin;" entity, not just "not".
        let (len, _) = longest_match("notin;x").unwrap();
        assert_eq!(len, 6);
    }

    #[test]
    fn no_match_returns_none() {
        assert!(longest_match("zzzznotanentity").is_none());
    }
}

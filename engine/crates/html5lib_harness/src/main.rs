//! A1's WPT-style test harness, scoped to A2's actual exit criterion:
//! ">=95% pass rate on html5lib-tests tokenizer tests" (full WPT execution
//! needs a JS engine to run testharness.js, which doesn't exist yet --
//! html5lib-tests' plain JSON format doesn't, so it's the right harness to
//! stand up first).
//!
//! Test files are vendored under `vendor/tokenizer/*.test` (fetched from
//! html5lib/html5lib-tests) so this runs offline and reproducibly in CI
//! without depending on GitHub being reachable at test time.
//!
//! Right now `html::tokenize()` is a placeholder (see crates/html/src/tokenizer.rs)
//! that always returns just an EOF token, so this harness is expected to
//! report a near-zero pass rate -- that's A1's exit criterion being met,
//! not a bug in the harness.

use html::{Token, TokenizerState};
use serde::Deserialize;
use serde_json::Value;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct TestFile {
    // `xmlViolation.test` uses "xmlViolationTests" as its top-level key
    // instead of "tests" -- same schema otherwise, so just accept both.
    #[serde(alias = "xmlViolationTests")]
    tests: Vec<TestCase>,
}

#[derive(Debug, Deserialize)]
struct TestCase {
    description: String,
    input: String,
    #[serde(default)]
    output: Vec<Value>,
    #[serde(rename = "doubleEscaped", default)]
    double_escaped: bool,
    /// Which tokenizer state(s) to start in; defaults to "Data state" when
    /// absent. A test with multiple states should be run once per state
    /// per html5lib-tests' own convention.
    #[serde(rename = "initialStates", default)]
    initial_states: Vec<String>,
    /// Primes `last_start_tag_name` for RCDATA/RAWTEXT/ScriptData's
    /// "appropriate end tag token" check, since without a tree builder
    /// driving the tokenizer there's no earlier start tag to have set it.
    #[serde(rename = "lastStartTag", default)]
    last_start_tag: Option<String>,
}

fn parse_initial_state(s: &str) -> TokenizerState {
    match s {
        "PLAINTEXT state" => TokenizerState::PlainText,
        "RCDATA state" => TokenizerState::RcData,
        "RAWTEXT state" => TokenizerState::RawText,
        "Script data state" => TokenizerState::ScriptData,
        "CDATA section state" => TokenizerState::CdataSection,
        _ => TokenizerState::Data,
    }
}

/// A canonical, comparison-friendly view of a token stream: consecutive
/// character tokens are coalesced into one run (both the spec's expected
/// output and our tokenizer's actual output already do/will do this), and
/// EOF is dropped since expected `output` arrays never include it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Canonical {
    Doctype {
        name: String,
        public_id: Option<String>,
        system_id: Option<String>,
        correctness: bool,
    },
    StartTag(String, Vec<(String, String)>),
    EndTag(String),
    Comment(String),
    Characters(String),
}

fn canonicalize_actual(tokens: &[Token]) -> Vec<Canonical> {
    let mut out = Vec::new();
    let mut pending_chars = String::new();
    let flush = |pending: &mut String, out: &mut Vec<Canonical>| {
        if !pending.is_empty() {
            out.push(Canonical::Characters(std::mem::take(pending)));
        }
    };
    for token in tokens {
        match token {
            Token::Character(c) => pending_chars.push(*c),
            Token::Eof => {}
            other => {
                flush(&mut pending_chars, &mut out);
                out.push(match other {
                    Token::Doctype {
                        name,
                        public_id,
                        system_id,
                        force_quirks,
                    } => Canonical::Doctype {
                        name: name.clone().unwrap_or_default(),
                        public_id: public_id.clone(),
                        system_id: system_id.clone(),
                        correctness: !force_quirks,
                    },
                    Token::StartTag {
                        name, attributes, ..
                    } => {
                        // Attribute order isn't spec-observable (expected
                        // output is a JSON object/set), so sort both sides
                        // the same way before comparing.
                        let mut attrs = attributes.clone();
                        attrs.sort();
                        Canonical::StartTag(name.clone(), attrs)
                    }
                    Token::EndTag { name } => Canonical::EndTag(name.clone()),
                    Token::Comment(c) => Canonical::Comment(c.clone()),
                    Token::Character(_) | Token::Eof => unreachable!(),
                });
            }
        }
    }
    flush(&mut pending_chars, &mut out);
    out
}

fn canonicalize_expected(output: &[Value]) -> Vec<Canonical> {
    let mut out = Vec::new();
    let mut pending_chars = String::new();
    let flush = |pending: &mut String, out: &mut Vec<Canonical>| {
        if !pending.is_empty() {
            out.push(Canonical::Characters(std::mem::take(pending)));
        }
    };
    for entry in output {
        let arr = match entry.as_array() {
            Some(a) => a,
            None => continue,
        };
        let kind = arr.first().and_then(Value::as_str).unwrap_or_default();
        match kind {
            "Character" => {
                if let Some(s) = arr.get(1).and_then(Value::as_str) {
                    pending_chars.push_str(s);
                }
            }
            "DOCTYPE" => {
                flush(&mut pending_chars, &mut out);
                let name = arr
                    .get(1)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let public_id = arr.get(2).and_then(Value::as_str).map(str::to_string);
                let system_id = arr.get(3).and_then(Value::as_str).map(str::to_string);
                // The spec's expected-output array's 5th element is
                // "correctness" (true = not force-quirks); absent means
                // true, matching html5lib-tests' own convention.
                let correctness = arr.get(4).and_then(Value::as_bool).unwrap_or(true);
                out.push(Canonical::Doctype {
                    name,
                    public_id,
                    system_id,
                    correctness,
                });
            }
            "StartTag" => {
                flush(&mut pending_chars, &mut out);
                let name = arr
                    .get(1)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let mut attrs: Vec<(String, String)> = arr
                    .get(2)
                    .and_then(Value::as_object)
                    .map(|obj| {
                        obj.iter()
                            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                attrs.sort();
                out.push(Canonical::StartTag(name, attrs));
            }
            "EndTag" => {
                flush(&mut pending_chars, &mut out);
                let name = arr
                    .get(1)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                out.push(Canonical::EndTag(name));
            }
            "Comment" => {
                flush(&mut pending_chars, &mut out);
                let data = arr
                    .get(1)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                out.push(Canonical::Comment(data));
            }
            _ => {}
        }
    }
    flush(&mut pending_chars, &mut out);
    out
}

/// html5lib-tests uses `\uXXXX` escapes in `input`/`output` strings for
/// characters that don't survive JSON source round-tripping cleanly
/// (lone surrogates etc), flagged by `doubleEscaped: true` on the test case.
fn unescape_double_escaped(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && chars.get(i + 1) == Some(&'u') && i + 6 <= chars.len() {
            let hex: String = chars[i + 2..i + 6].iter().collect();
            if let Ok(code) = u32::from_str_radix(&hex, 16) {
                if let Some(c) = char::from_u32(code) {
                    out.push(c);
                    i += 6;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn main() {
    let vendor_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/tokenizer");
    let mut entries: Vec<_> = fs::read_dir(&vendor_dir)
        .unwrap_or_else(|e| panic!("reading {vendor_dir:?}: {e}"))
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "test"))
        .collect();
    entries.sort_by_key(|e| e.path());

    let mut total = 0usize;
    let mut passed = 0usize;
    let mut failures_shown = 0usize;
    const MAX_FAILURES_SHOWN: usize = 5;

    for entry in &entries {
        let path = entry.path();
        let contents =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path:?}: {e}"));
        let file: TestFile = match serde_json::from_str(&contents) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("skipping {path:?}: {e}");
                continue;
            }
        };

        for case in file.tests {
            let input = if case.double_escaped {
                unescape_double_escaped(&case.input)
            } else {
                case.input.clone()
            };

            let mut expected = canonicalize_expected(&case.output);
            if case.double_escaped {
                expected = expected
                    .into_iter()
                    .map(|c| match c {
                        Canonical::Characters(s) => {
                            Canonical::Characters(unescape_double_escaped(&s))
                        }
                        Canonical::Comment(s) => Canonical::Comment(unescape_double_escaped(&s)),
                        other => other,
                    })
                    .collect();
            }

            let states = if case.initial_states.is_empty() {
                vec!["Data state".to_string()]
            } else {
                case.initial_states.clone()
            };

            for state_name in &states {
                total += 1;
                let state = parse_initial_state(state_name);
                let actual = canonicalize_actual(&html::tokenize_with(
                    &input,
                    state,
                    case.last_start_tag.as_deref(),
                ));

                if actual == expected {
                    passed += 1;
                } else if failures_shown < MAX_FAILURES_SHOWN {
                    failures_shown += 1;
                    eprintln!(
                        "FAIL [{}] {} ({state_name}): expected {:?}, got {:?}",
                        path.file_name().unwrap().to_string_lossy(),
                        case.description,
                        expected,
                        actual
                    );
                }
            }
        }
    }

    let pct = if total == 0 {
        0.0
    } else {
        100.0 * passed as f64 / total as f64
    };
    println!("html5lib-tests tokenizer: {passed}/{total} passed ({pct:.1}%)");
    println!(
        "(A2's exit criterion is >=95% -- this number is the metric to watch as A2 is implemented)"
    );
}

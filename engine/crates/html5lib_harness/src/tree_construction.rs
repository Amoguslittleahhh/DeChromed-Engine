//! A3's conformance harness: the WPT-hosted html5lib-tests tree-construction
//! corpus (`.dat` format), vendored under `vendor/tree-construction/`.
//!
//! Format: blocks separated by a `#data` marker line, each containing
//! `#data` (the input, verbatim, possibly multi-line), `#errors` (ignored --
//! we don't track parse errors, same as the tokenizer harness), optional
//! `#new-errors`, optional `#document-fragment` (a context element name,
//! meaning this test wants *fragment* parsing -- skipped, since A3 doesn't
//! implement that algorithm, a documented gap), and `#document` (the
//! expected tree dump, in html5lib's own indented `| <tag>` notation).

use html::parse_document;
use std::fs;
use std::path::Path;

struct TestCase {
    data: String,
    expected_document: String,
    is_fragment: bool,
}

fn parse_dat_file(contents: &str) -> Vec<TestCase> {
    let mut cases = Vec::new();
    let mut lines = contents.lines().peekable();

    while let Some(line) = lines.peek() {
        if *line != "#data" {
            lines.next();
            continue;
        }
        lines.next(); // consume "#data"

        let mut data_lines = Vec::new();
        while let Some(line) = lines.peek() {
            if line.starts_with('#') {
                break;
            }
            data_lines.push(*line);
            lines.next();
        }

        let mut is_fragment = false;
        // Skip #errors, #new-errors, #script-on/#script-off, and
        // #document-fragment (noting it) until we reach #document.
        while let Some(line) = lines.peek() {
            match *line {
                "#document" => break,
                "#document-fragment" => {
                    is_fragment = true;
                    lines.next();
                }
                _ => {
                    lines.next();
                }
            }
        }

        if lines.peek() != Some(&"#document") {
            // Malformed/unexpected trailing content; stop parsing this file.
            break;
        }
        lines.next(); // consume "#document"

        let mut doc_lines = Vec::new();
        while let Some(line) = lines.peek() {
            if *line == "#data" {
                break;
            }
            doc_lines.push(*line);
            lines.next();
        }
        // Trailing blank lines between blocks end up in doc_lines; the
        // expected document itself never contains truly blank lines since
        // every real line starts with "| ", so trimming trailing empties is safe.
        while doc_lines.last() == Some(&"") {
            doc_lines.pop();
        }

        cases.push(TestCase {
            data: data_lines.join("\n"),
            expected_document: doc_lines.join("\n"),
            is_fragment,
        });
    }

    cases
}

/// Renders a parsed document in html5lib's own `| <tag>` tree-dump
/// notation, so it can be compared line-for-line against `#document`.
/// Unlike `Document`'s own `Display` impl (used by the shell's smoke test),
/// this omits the synthetic `#document` root line and starts `<html>` at
/// depth 0 -- that's the convention html5lib-tests' `.dat` files use.
fn serialize_html5lib(doc: &dom::Document) -> String {
    let mut out = String::new();
    for &child in doc.children(doc.root()) {
        write_node(doc, child, 0, &mut out);
    }
    out.trim_end_matches('\n').to_string()
}

fn write_node(doc: &dom::Document, id: dom::NodeId, depth: usize, out: &mut String) {
    use dom::NodeData;
    use std::fmt::Write;
    let indent = "  ".repeat(depth);
    match doc.data(id) {
        NodeData::Document => {}
        NodeData::Doctype(d) => match (&d.public_id, &d.system_id) {
            (None, None) => {
                let _ = writeln!(out, "| {indent}<!DOCTYPE {}>", d.name);
            }
            (public, system) => {
                let _ = writeln!(
                    out,
                    "| {indent}<!DOCTYPE {} \"{}\" \"{}\">",
                    d.name,
                    public.as_deref().unwrap_or(""),
                    system.as_deref().unwrap_or("")
                );
            }
        },
        NodeData::Element(el) => {
            let _ = writeln!(out, "| {indent}<{}>", el.local_name);
            let mut attrs = el.attributes.clone();
            attrs.sort();
            for (k, v) in &attrs {
                let _ = writeln!(out, "| {indent}  {k}=\"{v}\"");
            }
        }
        NodeData::Text(t) => {
            let _ = writeln!(out, "| {indent}\"{t}\"");
        }
        NodeData::Comment(c) => {
            let _ = writeln!(out, "| {indent}<!-- {c} -->");
        }
    }
    for &child in doc.children(id) {
        write_node(doc, child, depth + 1, out);
    }
}

pub fn run() {
    let vendor_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/tree-construction");
    let mut entries: Vec<_> = fs::read_dir(&vendor_dir)
        .unwrap_or_else(|e| panic!("reading {vendor_dir:?}: {e}"))
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "dat"))
        .collect();
    entries.sort_by_key(|e| e.path());

    // Suppress the default panic hook's stderr spam -- caught panics are
    // reported through our own FAIL lines below instead.
    std::panic::set_hook(Box::new(|_| {}));

    let mut total = 0usize;
    let mut passed = 0usize;
    let mut skipped_fragment = 0usize;
    let mut failures_shown = 0usize;
    let max_failures_shown: usize = std::env::var("HARNESS_MAX_FAILURES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let skip_known_gaps = std::env::var("HARNESS_SKIP_KNOWN_GAPS").is_ok();
    let only_file = std::env::var("HARNESS_ONLY_FILE").ok();
    let mut per_file: std::collections::BTreeMap<String, (usize, usize)> =
        std::collections::BTreeMap::new();

    let debug_progress = std::env::var("HARNESS_DEBUG").is_ok();
    for entry in &entries {
        let path = entry.path();
        let file_name = path.file_name().unwrap().to_string_lossy().to_string();
        if let Some(only) = &only_file {
            if &file_name != only {
                continue;
            }
        }
        if skip_known_gaps
            && matches!(
                file_name.as_str(),
                "svg.dat"
                    | "math.dat"
                    | "namespace-sensitivity.dat"
                    | "foreign-fragment.dat"
                    | "menuitem-element.dat"
                    | "template.dat"
                    | "tests9.dat"
                    | "tests10.dat"
                    | "tests11.dat"
                    | "tests21.dat"
                    | "processing-instructions.dat"
            )
        {
            continue;
        }
        if debug_progress {
            eprintln!("--- {path:?}");
        }
        let contents = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => {
                // A couple of vendored files contain non-UTF-8 bytes
                // (deliberately, to test malformed-byte handling) --
                // read those lossily rather than skipping the whole file.
                let bytes = fs::read(&path).unwrap_or_else(|e| panic!("reading {path:?}: {e}"));
                String::from_utf8_lossy(&bytes).into_owned()
            }
        };

        for case in parse_dat_file(&contents) {
            if case.is_fragment {
                skipped_fragment += 1;
                continue;
            }
            total += 1;
            let data = case.data.clone();
            if debug_progress {
                eprintln!("  case {total}: {:?}", data);
            }
            // A single malformed/edge-case input panicking (stack
            // underflow bugs etc, same class of bug A2 hit and fixed)
            // shouldn't take down the whole run and hide the real pass
            // rate for everything else -- catch it and count as a failure.
            let result = std::panic::catch_unwind(move || {
                let doc = parse_document(&data);
                serialize_html5lib(&doc)
            });
            let actual = match result {
                Ok(a) => a,
                Err(_) => "<panicked>".to_string(),
            };
            let entry = per_file.entry(file_name.clone()).or_insert((0, 0));
            entry.1 += 1;
            if actual == case.expected_document {
                passed += 1;
                entry.0 += 1;
            } else if failures_shown < max_failures_shown {
                failures_shown += 1;
                eprintln!(
                    "FAIL [{}] input={:?}\n  expected:\n{}\n  actual:\n{}\n",
                    path.file_name().unwrap().to_string_lossy(),
                    case.data,
                    indent(&case.expected_document),
                    indent(&actual),
                );
            }
        }
    }

    let pct = if total == 0 {
        0.0
    } else {
        100.0 * passed as f64 / total as f64
    };
    println!("html5lib-tests tree-construction: {passed}/{total} passed ({pct:.1}%)");
    println!(
        "({skipped_fragment} fragment-context cases skipped -- fragment parsing not implemented)"
    );
    println!(
        "(A3's exit criterion is >=95% -- this number is the metric to watch as A3 is implemented)"
    );

    if std::env::var("HARNESS_PER_FILE").is_ok() {
        println!("\nPer-file breakdown:");
        for (name, (p, t)) in &per_file {
            if p != t {
                println!("  {name}: {p}/{t}");
            }
        }
    }
}

fn indent(s: &str) -> String {
    s.lines()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

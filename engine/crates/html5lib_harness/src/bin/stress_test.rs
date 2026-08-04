//! A cross-crate stress-test binary for Track A (A1-A10): hunts for
//! process-crashing bugs (panics, stack overflows) across the HTML
//! tokenizer/tree builder, CSS tokenizer/parser/selectors/cascade/CSSOM,
//! and the XML parser, using truncation fuzzing (every prefix length of a
//! realistic seed document), random mutation fuzzing, pure random-byte-
//! soup fuzzing, and a curated set of known-tricky adversarial inputs.
//!
//! This is deliberately not a conformance harness (that's `html5lib_harness`
//! and `tree_construction_harness`) -- it doesn't check *correctness* of
//! output against an expected answer, only that every parser returns
//! *something* (or a graceful `Err`) instead of panicking, hanging, or
//! aborting the process. A deterministic PRNG (seeded, no external `rand`
//! dependency) makes runs reproducible; any failure found gets printed
//! with its exact reproducer input so it can be turned into a permanent
//! regression test.
//!
//! Run with `cargo run --release -p html5lib_harness --bin stress_test`
//! (release mode matters -- some of this, like the truncation fuzzer,
//! does real work and is slow unoptimized).

use std::panic::{self, AssertUnwindSafe};

/// A small, fast, seedable PRNG (splitmix64) -- deterministic so a failure
/// found by a run is always reproducible from its seed, without pulling in
/// an external `rand` dependency for what's fundamentally "generate some
/// adversarial bytes."
struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        SplitMix64(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    fn next_range(&mut self, n: usize) -> usize {
        (self.next_u64() as usize) % n.max(1)
    }
}

/// Alphabets biased toward each format's own syntactically load-bearing
/// characters, since uniformly random Unicode almost never reaches the
/// interesting states (unbalanced tags, stray entities, ...) that byte
/// soup needs to hit to be a useful fuzzer.
const HTML_ALPHABET: &[char] = &[
    '<', '>', '/', '=', '"', '\'', '&', ';', '!', '-', '[', ']', 'a', 'b', 'p', 'd', 'i', 'v', 's',
    'c', 'r', 't', ' ', '\n', '\t', '#', '%', '0', '9', 'x',
];
const CSS_ALPHABET: &[char] = &[
    '{', '}', '(', ')', '[', ']', ':', ';', ',', '.', '#', '@', '"', '\'', '\\', '/', '*', '-',
    '+', '!', 'a', 'p', 'n', 'v', 'r', ' ', '\n', '0', '9', '%',
];
const XML_ALPHABET: &[char] = &[
    '<', '>', '/', '=', '"', '\'', '&', ';', '!', '[', ']', '?', ':', 'a', 'b', 'x', 'm', 'l', 'n',
    's', ' ', '\n', '#', '0', '9',
];

fn random_string(rng: &mut SplitMix64, alphabet: &[char], len: usize) -> String {
    (0..len)
        .map(|_| alphabet[rng.next_range(alphabet.len())])
        .collect()
}

fn mutate(rng: &mut SplitMix64, base: &str, alphabet: &[char], edits: usize) -> String {
    let mut chars: Vec<char> = base.chars().collect();
    for _ in 0..edits {
        if chars.is_empty() {
            chars.push(alphabet[rng.next_range(alphabet.len())]);
            continue;
        }
        match rng.next_range(3) {
            0 => {
                let i = rng.next_range(chars.len());
                chars.insert(i, alphabet[rng.next_range(alphabet.len())]);
            }
            1 => {
                let i = rng.next_range(chars.len());
                chars.remove(i);
            }
            _ => {
                let i = rng.next_range(chars.len());
                chars[i] = alphabet[rng.next_range(alphabet.len())];
            }
        }
    }
    chars.into_iter().collect()
}

struct Report {
    tool_name: &'static str,
    failures: Vec<(String, String)>,
    attempted: usize,
}

impl Report {
    fn new(tool_name: &'static str) -> Self {
        Report {
            tool_name,
            failures: Vec::new(),
            attempted: 0,
        }
    }

    fn try_input(&mut self, input: String, f: impl FnOnce(&str) + std::panic::UnwindSafe) {
        self.attempted += 1;
        let input_for_panic = input.clone();
        let result = panic::catch_unwind(AssertUnwindSafe(|| f(&input_for_panic)));
        if let Err(payload) = result {
            let msg = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "<non-string panic payload>".to_string());
            // Dedupe by panic message so one root cause hit by many
            // fuzzed inputs doesn't flood the report.
            if !self.failures.iter().any(|(_, m)| m == &msg) {
                self.failures.push((input, msg));
            }
        }
    }

    fn print_summary(&self) {
        println!(
            "{}: {} attempted, {} distinct failures",
            self.tool_name,
            self.attempted,
            self.failures.len()
        );
        for (input, msg) in &self.failures {
            println!("  FAIL input={input:?}\n    panic: {msg}");
        }
    }
}

const HTML_SEEDS: &[&str] = &[
    "<!DOCTYPE html><html><head><title>t</title></head><body><p class=\"x\">hi<b>bold<i>both</b>only-i</i></p></body></html>",
    "<table><tr><td><svg><foreignObject><p>x</p></foreignObject></svg></td></tr></table>",
    "<div><script>var x = '<div>';</script><style>a{color:red}</style></div>",
    "<select><option>a<option>b</select>",
    "<math><mi>x</mi><annotation-xml encoding=\"text/html\"><div>y</div></annotation-xml></math>",
    "<!DOCTYPE html PUBLIC \"-//W3C//DTD HTML 4.01//EN\"><body xlink:href=foo><math xlink:href=bar></math>",
    // B3/B4: a table with a colspan cell plus a nested flex container, to
    // exercise layout_table/layout_flex_container together with the rest
    // of the fuzzer's HTML+CSS combinations.
    "<div style=\"display:flex\"><table><thead><tr><th colspan=\"2\">h</th></tr></thead><tbody><tr><td>a</td><td><div style=\"display:flex;flex-direction:column\">x<span>y</span></div></td></tr></tbody></table><p>item</p></div>",
    // B5/B6: a grid with explicit + auto-placed items, plus relative and
    // absolute positioning.
    "<div style=\"display:grid;grid-template-columns:1fr 2fr\"><div style=\"grid-column:2\">a</div><div>b</div><div style=\"position:relative;top:5px\">c</div><div style=\"position:absolute;left:10px\">d</div></div>",
    // B7/B8: multi-column with a forced break, plus RTL/logical properties.
    "<div style=\"column-count:3;direction:rtl\"><p style=\"margin-inline-start:5px\">one</p><p style=\"break-before:always\">two</p><p>three four five</p></div>",
];

const CSS_SEEDS: &[&str] = &[
    "@media screen { p.a, div#b[x~=y] { color: red !important; margin: 0 1px 2% auto; } }",
    "a { --x: var(--y, blue); color: var(--x); } .b::before { content: \"\\\"quoted\\\"\"; }",
    "@font-face { src: url(foo.woff) format(\"woff\"); } @import url(bar.css);",
    // CSS Nesting Module: & substitution, implicit descendant nesting,
    // comma-separated nested/parent selectors, nested @media.
    ".a, .b { color: red; &:hover, & .c { color: blue; } @media (min-width: 1px) { color: green; &:focus { color: purple; } } }",
    "@layer base { .a { color: red; & .b { color: blue; } } }",
    // B3/B4: table/flex display values plus flexbox longhands.
    "table { display: table; } tr { display: table-row; } td { display: table-cell; width: 40px; } .f { display: flex; flex-wrap: wrap; flex-grow: 1; flex-shrink: 2; flex-basis: 10%; justify-content: space-between; align-items: center; }",
    // B5/B6: grid template/placement and positioning longhands.
    ".g { display: grid; grid-template-columns: 50px 1fr auto; grid-template-rows: 30px; } .item { grid-column: span 2; } .p { position: absolute; top: 10%; left: 5px; right: 0; bottom: 0; }",
    // B7/B8: multi-column, break properties, direction, and logical
    // margin/padding longhands.
    ".m { column-count: 3; column-width: 50px; column-gap: 10px; } .b { break-before: always; break-after: avoid; break-inside: avoid; } .d { direction: rtl; writing-mode: vertical-rl; margin-inline-start: 4px; margin-block-end: 2px; padding-inline-end: 1px; }",
];

const SELECTOR_SEEDS: &[&str] = &[
    "div.a#b[c~=\"d\" i]:not(.e):is(.f, .g)::before",
    ":nth-child(2n+1 of .a, .b)",
    "a > b + c ~ d e",
    // Large-magnitude An+B literals: past bug (fixed via `saturating_neg`)
    // was a negation overflow when a huge negative B casts to `i32::MIN`.
    ":nth-child(3n- -999999999999999999999)",
    ":nth-last-child(-999999999999999999999n+999999999999999999999)",
    ":lang(en-US, \"fr\", *-CA):dir(rtl)",
];

const XML_SEEDS: &[&str] = &[
    "<?xml version=\"1.0\"?><root xmlns:x=\"urn:x\"><x:a b=\"c\">text &amp; &#65; <![CDATA[<raw>]]></x:a></root>",
    "<!DOCTYPE root SYSTEM \"x.dtd\"><root><!--c--><?pi data?></root>",
];

fn truncation_fuzz<F: Fn(&str) + std::panic::UnwindSafe + Copy>(
    report: &mut Report,
    seeds: &[&str],
    parse: F,
) {
    for seed in seeds {
        let chars: Vec<char> = seed.chars().collect();
        for len in 0..=chars.len() {
            let prefix: String = chars[..len].iter().collect();
            report.try_input(prefix, parse);
        }
    }
}

fn mutation_fuzz<F: Fn(&str) + std::panic::UnwindSafe + Copy>(
    report: &mut Report,
    rng: &mut SplitMix64,
    seeds: &[&str],
    alphabet: &[char],
    iterations: usize,
    parse: F,
) {
    for seed in seeds {
        for _ in 0..iterations {
            let edits = 1 + rng.next_range(20);
            let mutated = mutate(rng, seed, alphabet, edits);
            report.try_input(mutated, parse);
        }
    }
}

fn random_fuzz<F: Fn(&str) + std::panic::UnwindSafe + Copy>(
    report: &mut Report,
    rng: &mut SplitMix64,
    alphabet: &[char],
    iterations: usize,
    parse: F,
) {
    for _ in 0..iterations {
        let len = rng.next_range(200);
        let s = random_string(rng, alphabet, len);
        report.try_input(s, parse);
    }
}

/// Targeted generator for the adoption agency algorithm: random open/close
/// order of formatting elements (`a`/`b`/`i`) interleaved with block
/// elements (`div`/`p`), which is specifically what triggers it. Uniform
/// random-byte fuzzing rarely produces enough of this exact shape by
/// chance to meaningfully exercise its 8-iteration outer loop, bookmark
/// tracking, and furthest-block reparenting.
fn random_adoption_agency_input(rng: &mut SplitMix64, tokens: usize) -> String {
    let tags = ["a", "b", "i", "div", "p", "span"];
    let mut open: Vec<&str> = Vec::new();
    let mut out = String::new();
    for _ in 0..tokens {
        if !open.is_empty() && rng.next_range(3) == 0 {
            // Close a random currently-open tag, not necessarily the most
            // recent one -- exactly the "misnesting" shape that triggers
            // the adoption agency algorithm.
            let i = rng.next_range(open.len());
            let tag = open.remove(i);
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
        } else {
            let tag = tags[rng.next_range(tags.len())];
            out.push('<');
            out.push_str(tag);
            out.push('>');
            open.push(tag);
        }
    }
    out
}

fn main() {
    // Silence the default panic hook's stderr spam -- caught panics are
    // reported through our own FAIL lines instead (same pattern as
    // tree_construction_harness).
    panic::set_hook(Box::new(|_| {}));

    let seed: u64 = std::env::var("STRESS_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0xC0FFEE);
    let iterations: usize = std::env::var("STRESS_ITERATIONS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20_000);
    let mut rng = SplitMix64::new(seed);

    println!("stress_test: seed={seed:#x} iterations={iterations} per fuzzer");

    let mut html_report = Report::new("html::parse_document");
    truncation_fuzz(&mut html_report, HTML_SEEDS, |s| {
        let _ = html::parse_document(s);
    });
    mutation_fuzz(
        &mut html_report,
        &mut rng,
        HTML_SEEDS,
        HTML_ALPHABET,
        iterations,
        |s| {
            let _ = html::parse_document(s);
        },
    );
    random_fuzz(&mut html_report, &mut rng, HTML_ALPHABET, iterations, |s| {
        let _ = html::parse_document(s);
    });
    for _ in 0..iterations {
        let tokens = 4 + rng.next_range(60);
        let input = random_adoption_agency_input(&mut rng, tokens);
        html_report.try_input(input, |s| {
            let _ = html::parse_document(s);
        });
    }
    html_report.print_summary();

    let mut css_report = Report::new("css::parse_stylesheet");
    truncation_fuzz(&mut css_report, CSS_SEEDS, |s| {
        let _ = css::parse_stylesheet(s);
    });
    mutation_fuzz(
        &mut css_report,
        &mut rng,
        CSS_SEEDS,
        CSS_ALPHABET,
        iterations,
        |s| {
            let _ = css::parse_stylesheet(s);
        },
    );
    random_fuzz(&mut css_report, &mut rng, CSS_ALPHABET, iterations, |s| {
        let _ = css::parse_stylesheet(s);
    });
    css_report.print_summary();

    let mut selector_report = Report::new("css::selectors::parse_selector_list");
    truncation_fuzz(&mut selector_report, SELECTOR_SEEDS, |s| {
        let _ = css::selectors::parse_selector_list(s);
    });
    mutation_fuzz(
        &mut selector_report,
        &mut rng,
        SELECTOR_SEEDS,
        CSS_ALPHABET,
        iterations,
        |s| {
            let _ = css::selectors::parse_selector_list(s);
        },
    );
    random_fuzz(
        &mut selector_report,
        &mut rng,
        CSS_ALPHABET,
        iterations,
        |s| {
            let _ = css::selectors::parse_selector_list(s);
        },
    );
    selector_report.print_summary();

    let mut xml_report = Report::new("xml::parse_document");
    truncation_fuzz(&mut xml_report, XML_SEEDS, |s| {
        let _ = xml::parse_document(s);
    });
    mutation_fuzz(
        &mut xml_report,
        &mut rng,
        XML_SEEDS,
        XML_ALPHABET,
        iterations,
        |s| {
            let _ = xml::parse_document(s);
        },
    );
    random_fuzz(&mut xml_report, &mut rng, XML_ALPHABET, iterations, |s| {
        let _ = xml::parse_document(s);
    });
    xml_report.print_summary();

    // A6/A7: fuzz the cascade/style-engine pipeline end to end -- parse a
    // mutated HTML seed into a real DOM, parse a mutated CSS seed into a
    // real stylesheet, then run the full cascade + computed-style pass
    // over every element. This exercises selector matching and cascade
    // logic against structurally-valid-but-unusual trees that a bare
    // string fuzzer wouldn't otherwise reach.
    let mut cascade_report = Report::new("css::cascade::compute_document_styles");
    for _ in 0..iterations {
        let html_edits = 1 + rng.next_range(10);
        let css_edits = 1 + rng.next_range(10);
        let html_seed = HTML_SEEDS[rng.next_range(HTML_SEEDS.len())];
        let css_seed = CSS_SEEDS[rng.next_range(CSS_SEEDS.len())];
        let html_input = mutate(&mut rng, html_seed, HTML_ALPHABET, html_edits);
        let css_input = mutate(&mut rng, css_seed, CSS_ALPHABET, css_edits);
        let combined = format!("{html_input}\u{0}{css_input}");
        cascade_report.try_input(combined, |s| {
            let (html_part, css_part) = s.split_once('\u{0}').unwrap();
            let doc = html::parse_document(html_part);
            let sheet = css::parse_stylesheet(css_part);
            let sources = [css::cascade::StyleSource {
                origin: css::cascade::Origin::Author,
                sheet: &sheet,
            }];
            let _ = css::cascade::compute_document_styles(&doc, &sources);
        });
    }
    cascade_report.print_summary();

    // B1/B2: fuzz the box-tree/layout pipeline the same way -- computed
    // styles from a mutated HTML+CSS pair, built into a real box tree and
    // laid out at a few different containing-block widths (including
    // pathologically narrow ones, since that's what stresses the inline
    // line-breaking loop hardest).
    let mut layout_report = Report::new("layout::layout");
    for _ in 0..iterations {
        let html_edits = 1 + rng.next_range(10);
        let css_edits = 1 + rng.next_range(10);
        let html_seed = HTML_SEEDS[rng.next_range(HTML_SEEDS.len())];
        let css_seed = CSS_SEEDS[rng.next_range(CSS_SEEDS.len())];
        let html_input = mutate(&mut rng, html_seed, HTML_ALPHABET, html_edits);
        let css_input = mutate(&mut rng, css_seed, CSS_ALPHABET, css_edits);
        let width = [0.0, 1.0, 60.0, 800.0][rng.next_range(4)];
        let combined = format!("{html_input}\u{0}{css_input}\u{0}{width}");
        layout_report.try_input(combined, |s| {
            let mut parts = s.splitn(3, '\u{0}');
            let html_part = parts.next().unwrap();
            let css_part = parts.next().unwrap();
            let width: f64 = parts.next().unwrap().parse().unwrap_or(800.0);
            let doc = html::parse_document(html_part);
            let sheet = css::parse_stylesheet(css_part);
            let sources = [css::cascade::StyleSource {
                origin: css::cascade::Origin::Author,
                sheet: &sheet,
            }];
            let styles = css::cascade::compute_document_styles(&doc, &sources);
            if let Some(tree) = layout::build_box_tree(&doc, &styles) {
                let _ = layout::layout(&tree, &styles, width);
            }
        });
    }
    layout_report.print_summary();

    // B9/B11: fuzz the display-list lowering + rasterizer the same way, on
    // top of the same fuzzed HTML+CSS+width combinations already stressing
    // layout above -- since `paint::build_display_list` and
    // `paint::rasterize` both walk the resulting `Fragment` tree, this
    // exercises them against a much wider range of fragment-tree shapes
    // (deeply nested, zero-size, pathologically narrow) than the paint
    // crate's own hand-written unit tests would ever construct.
    let mut paint_report = Report::new("paint::build_display_list + rasterize");
    for _ in 0..iterations {
        let html_edits = 1 + rng.next_range(10);
        let css_edits = 1 + rng.next_range(10);
        let html_seed = HTML_SEEDS[rng.next_range(HTML_SEEDS.len())];
        let css_seed = CSS_SEEDS[rng.next_range(CSS_SEEDS.len())];
        let html_input = mutate(&mut rng, html_seed, HTML_ALPHABET, html_edits);
        let css_input = mutate(&mut rng, css_seed, CSS_ALPHABET, css_edits);
        let width = [0.0, 1.0, 60.0, 800.0][rng.next_range(4)];
        let combined = format!("{html_input}\u{0}{css_input}\u{0}{width}");
        paint_report.try_input(combined, |s| {
            let mut parts = s.splitn(3, '\u{0}');
            let html_part = parts.next().unwrap();
            let css_part = parts.next().unwrap();
            let width: f64 = parts.next().unwrap().parse().unwrap_or(800.0);
            let doc = html::parse_document(html_part);
            let sheet = css::parse_stylesheet(css_part);
            let sources = [css::cascade::StyleSource {
                origin: css::cascade::Origin::Author,
                sheet: &sheet,
            }];
            let styles = css::cascade::compute_document_styles(&doc, &sources);
            if let Some(tree) = layout::build_box_tree(&doc, &styles) {
                let fragment = layout::layout(&tree, &styles, width);
                let list = paint::build_display_list(&fragment, &styles);
                let border_box = fragment.border_box();
                let raster_width = border_box.width.max(0.0).round() as usize;
                let raster_height = border_box.height.max(0.0).round() as usize;
                let _ = paint::rasterize(&list, raster_width.min(2000), raster_height.min(2000));
            }
        });
    }
    paint_report.print_summary();

    let total_failures: usize = [
        &html_report,
        &css_report,
        &selector_report,
        &xml_report,
        &cascade_report,
        &layout_report,
        &paint_report,
    ]
    .iter()
    .map(|r| r.failures.len())
    .sum();
    println!("\ntotal distinct failures across all fuzzers: {total_failures}");
    if total_failures > 0 {
        std::process::exit(1);
    }
}

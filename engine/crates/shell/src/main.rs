//! Roadmap phase: Track F7 (browser UI shell), but today this is just a
//! CLI smoke test wiring every crate together end to end. A2 (tokenizer)
//! through A10 (XML) are real now -- this demonstrates parsing a real
//! document, parsing a real stylesheet, matching a real selector,
//! cascading/computing real styles, incrementally restyling after a
//! targeted DOM mutation, parsing inline SVG via foreign content (A8/A9),
//! and parsing a standalone XML document (A10). Layout/paint are still
//! placeholders (Track B).

use css::cascade::Origin;
use css::cssom::CssomSheet;
use css::style_engine::StyleEngine;
use dom::NodeData;

fn main() {
    let input = "<!DOCTYPE html><html><body><p class=\"greeting\">hello</p></body></html>";
    let mut document = html::parse_document(input);

    let stylesheet = css::parse_stylesheet("p.greeting { color: red; }");
    let selector =
        css::selectors::parse_selector_list("p.greeting").expect("selector should parse");

    let fragments = layout::layout();
    let _display_list = paint::build_display_list(&fragments);

    println!("DeChromed Engine -- pipeline smoke test");
    println!("input: {input:?}");
    println!("dom:\n{document}");
    println!("stylesheet: {} rule(s) parsed", stylesheet.rules.len());

    let mut matched = 0;
    document.walk(document.root(), &mut |id, _depth| {
        if css::selectors::matches(&document, id, &selector) {
            matched += 1;
        }
    });
    println!("selector \"p.greeting\" matched {matched} element(s) in the tree");

    // A6/A7: build a live style engine over the same document and a
    // second (unrelated) rule that only kicks in once a class is added,
    // then prove the restyle after that mutation is targeted, not a
    // full-document recompute.
    let sheet = CssomSheet::parse("p.greeting { color: red; } p.warn { color: orange; }");
    let mut engine = StyleEngine::new(&document, vec![(Origin::Author, sheet)]);
    let p = find_first(&document, "p");
    println!(
        "getComputedStyle(p).color = {:?} (before mutation)",
        engine.get_computed_style(p, "color")
    );

    if let NodeData::Element(e) = document.data_mut(p) {
        match e.attributes.iter_mut().find(|(k, _)| k == "class") {
            Some((_, v)) => *v = "greeting warn".to_string(),
            None => e
                .attributes
                .push(("class".to_string(), "greeting warn".to_string())),
        }
    }
    let touched = engine.notify_class_changed(&document, p, "warn");
    println!(
        "getComputedStyle(p).color = {:?} (after adding class \"warn\", {} node(s) restyled)",
        engine.get_computed_style(p, "color"),
        touched.len()
    );

    // A8/A9: inline <svg> inside HTML reaches real foreign content --
    // the nested <path> gets the SVG namespace, not the HTML one.
    let svg_doc = html::parse_document("<body><svg><path d=\"M0 0\"></path></svg></body>");
    let svg_el = find_first(&svg_doc, "svg");
    if let NodeData::Element(e) = svg_doc.data(svg_el) {
        println!("inline <svg>'s namespace: {}", e.namespace);
    }

    // A10: a standalone XML document, parsed by a real (non-HTML,
    // fail-fast-on-malformed) XML parser.
    let xml_doc = xml::parse_document("<config><item id=\"1\">value</item></config>")
        .expect("well-formed XML");
    println!("standalone XML document:\n{}", xml::serialize(&xml_doc));

    println!("(layout/paint stages ran but are placeholders -- see ROADMAP.md Track B)");
}

fn find_first(document: &dom::Document, tag: &str) -> dom::NodeId {
    let mut found = None;
    document.walk(document.root(), &mut |id, _depth| {
        if found.is_none()
            && let NodeData::Element(e) = document.data(id)
            && e.local_name == tag
        {
            found = Some(id);
        }
    });
    found.expect("tag not found")
}

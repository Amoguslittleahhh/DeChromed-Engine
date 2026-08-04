//! Roadmap phase: Track F7 (browser UI shell), but today this is just a
//! CLI smoke test wiring every crate together end to end. A2 (tokenizer)
//! through A10 (XML) are real now -- this demonstrates parsing a real
//! document, parsing a real stylesheet, matching a real selector,
//! cascading/computing real styles, incrementally restyling after a
//! targeted DOM mutation, parsing inline SVG via foreign content (A8/A9),
//! and parsing a standalone XML document (A10). B1-B9 (box tree through
//! fragment-tree queries + display list) are real now too, run below
//! against a fixed 800px containing-block width (there's no window/
//! viewport concept yet -- see `layout`'s own module docs for what's
//! real and what's a documented gap in each phase). B11's software
//! rasterizer paints the display list's `FillRect` items into a real
//! pixel buffer (text painting is a documented B11 gap -- no glyph
//! outlines exist yet).

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
    // `display: block` on html/body/p below isn't a browser default this
    // engine ships (there's no UA stylesheet yet -- a documented Track A6
    // gap) -- spelling it out explicitly here is what lets B1-B9's demo
    // below show a normal box tree instead of collapsing everything into
    // one degenerate inline formatting context, exactly like this
    // project's own `layout` unit tests already have to do for the same
    // reason.
    let sheet = CssomSheet::parse(
        "html, body, p { display: block; } p.greeting { color: red; background-color: #ffd; } p.warn { color: orange; }",
    );
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

    // B1/B2: build a real box tree from the computed styles above and lay
    // it out against a fixed 800px containing-block width.
    let mut styles = layout::StyleMap::new();
    document.walk(document.root(), &mut |id, _depth| {
        if let Some(style) = engine.computed_style(id) {
            styles.insert(id, style.clone());
        }
    });
    if let Some(box_tree) = layout::build_box_tree(&document, &styles) {
        let fragment = layout::layout(&box_tree, &styles, 800.0);
        let border_box = fragment.border_box();
        println!(
            "layout: <html> border-box = {}x{} at ({}, {})",
            border_box.width, border_box.height, border_box.x, border_box.y
        );

        // B9: query the fragment tree directly, no re-derived geometry.
        if let Some(rect) = layout::bounding_client_rect(&fragment, p) {
            println!(
                "getBoundingClientRect(p) = {}x{} at ({}, {})",
                rect.width, rect.height, rect.x, rect.y
            );
        }
        let center_x = border_box.x + border_box.width / 2.0;
        let center_y = border_box.y + border_box.height / 2.0;
        let hit = layout::element_from_point(&fragment, center_x, center_y);
        println!("elementFromPoint(<html>'s center) = {hit:?}");

        // B9/B11: lower to a real display list, then rasterize it into a
        // real RGBA pixel buffer (see `paint`'s own module docs for what
        // B11's rasterizer does and doesn't paint yet).
        let display_list = paint::build_display_list(&fragment, &styles);
        println!("display list: {} item(s)", display_list.items.len());
        let canvas = paint::rasterize(
            &display_list,
            border_box.width as usize,
            border_box.height as usize,
        );
        println!("rasterized canvas: {}x{} px", canvas.width, canvas.height);
    }

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

    println!(
        "(text painting in B11's rasterizer is still a documented gap -- see ROADMAP.md Track B)"
    );
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

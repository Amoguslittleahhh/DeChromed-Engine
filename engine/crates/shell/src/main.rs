//! Roadmap phase: Track F7 (browser UI shell), but today this is just a
//! CLI smoke test wiring every crate together end to end. A2 (tokenizer)
//! through A10 (XML) are real now -- this demonstrates parsing a real
//! document, parsing a real stylesheet, matching a real selector,
//! cascading/computing real styles, incrementally restyling after a
//! targeted DOM mutation, parsing inline SVG via foreign content (A8/A9),
//! and parsing a standalone XML document (A10). C1's real spec-shaped DOM
//! API (`dom::api`/`dom::class_list`/`css::query`) is demonstrated next --
//! nodeName, textContent, classList, querySelector/closest, cloneNode.
//! B1-B9 (box tree through
//! fragment-tree queries + display list) are real now too, run below
//! against a fixed 800px containing-block width (there's no window/
//! viewport concept yet -- see `layout`'s own module docs for what's
//! real and what's a documented gap in each phase). B11's software
//! rasterizer paints the display list's `FillRect` items *and* real
//! shaped/rasterized text (`DrawText`, via `crates/text`'s real
//! HarfBuzz-equivalent shaping and TrueType glyph outlines) into a real
//! pixel buffer. B12's layer compositor (`paint::compositor`) composites
//! that same rasterized frame as its own layer, offset and faded, to
//! demonstrate the real translate/opacity math. B13's Canvas 2D
//! primitives (`paint::canvas2d`) paint a small filled-and-stroked shape
//! onto a standalone canvas, independent of the HTML/CSS pipeline (there's
//! no `<canvas>` DOM element or JS binding to reach it through yet).

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

    // C1: the real spec-shaped DOM API layered on the arena tree --
    // Node/Element navigation, real mutation algorithms, classList, and
    // (via css::query, built on A5's selector matcher) querySelector/
    // querySelectorAll/closest.
    println!("nodeName(p) = {:?}", document.node_name(p));
    println!("textContent(p) = {:?}", document.text_content(p));
    if let NodeData::Element(e) = document.data_mut(p) {
        e.add_class("featured");
        println!("classList after add(\"featured\") -> {:?}", e.attr("class"));
        println!("classList.contains(\"warn\") = {}", e.has_class("warn"));
        e.toggle_class("warn");
        println!("classList after toggle(\"warn\") -> {:?}", e.attr("class"));
    }
    let body = find_first(&document, "body");
    let query = css::selectors::parse_selector_list("p.featured").expect("selector should parse");
    println!(
        "querySelector(body, \"p.featured\") = {:?}",
        css::query::query_selector(&document, body, &query)
    );
    println!(
        "closest(p, \"body\") = {:?}",
        css::query::closest(
            &document,
            p,
            &css::selectors::parse_selector_list("body").expect("selector should parse")
        )
    );
    // cloneNode demonstrated on a throwaway copy of the document, not the
    // live one -- the real `document` below feeds straight into B1/B2's
    // layout pass, and an unstyled clone spliced into it would confuse
    // that pipeline rather than the DOM API this section is about.
    let mut scratch = html::parse_document(input);
    let scratch_p = find_first(&scratch, "p");
    let scratch_body = find_first(&scratch, "body");
    let clone = scratch.clone_node(scratch_p, true);
    scratch.append_existing(scratch_body, clone);
    println!(
        "cloneNode(p, deep=true) -> new node {clone:?}, {} total <p> now (getElementsByTagName)",
        scratch.get_elements_by_tag_name(scratch.root(), "p").len()
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
        // "hello" ends up styled `color: orange` by this point (the
        // class mutation above already applied "warn" before this style
        // map was built) -- count pixels matching that exact color to
        // prove B11 rasterized real letterform outlines, distinct from
        // the pale-yellow `background-color` FillRect that also covers
        // most of this canvas.
        let text_color = [255, 165, 0, 255];
        let text_pixels = (0..canvas.height)
            .flat_map(|y| (0..canvas.width).map(move |x| (x, y)))
            .filter(|&(x, y)| canvas.get_pixel(x, y) == text_color)
            .count();
        println!(
            "  -> {text_pixels} real glyph pixel(s) painted for \"hello\" (B11 now rasterizes real glyph outlines, not just FillRects)"
        );

        // B12: promote that same rasterized frame to its own compositor
        // layer and composite it again, offset and half-faded -- the real
        // per-pixel work (`Canvas::composite_over`) that lets a real
        // engine move/fade already-rasterized content without re-running
        // paint at all.
        let mut layer = paint::Layer::new(display_list, canvas.width, canvas.height);
        layer.offset_x = 20.0;
        layer.offset_y = 10.0;
        layer.opacity = 0.5;
        let composited = paint::composite_layers(&[layer], canvas.width + 40, canvas.height + 20);
        println!(
            "composited frame (1 layer, offset (20, 10), 50% opacity): {}x{} px",
            composited.width, composited.height
        );
    }

    // B13: Canvas 2D graphics primitives, independent of the HTML/CSS
    // pipeline above -- real scanline polygon fill (nonzero winding rule)
    // and Bresenham line stroking onto a standalone pixel buffer.
    let mut canvas2d = paint::Canvas::new(40, 40);
    let mut path = paint::Path2D::new();
    path.rect(5.0, 5.0, 20.0, 20.0);
    paint::fill(&mut canvas2d, &path, paint::Color::rgb(0, 128, 0));
    paint::stroke(&mut canvas2d, &path, paint::Color::rgb(0, 0, 0), 2.0);
    println!(
        "canvas2d: filled+stroked a 20x20 rect on a {}x{} canvas, center pixel = {:?}",
        canvas2d.width,
        canvas2d.height,
        canvas2d.get_pixel(15, 15)
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

    println!(
        "(remaining Track B gaps -- vertical writing modes, GPU compositing/rasterization, \
WebGL/WebGPU -- are documented in ROADMAP.md's B8/B11-B13 entries)"
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

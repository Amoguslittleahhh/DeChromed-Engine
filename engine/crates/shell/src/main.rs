//! Roadmap phase: Track F7 (browser UI shell), but today this is just a
//! CLI smoke test wiring every crate together end to end. A2 (tokenizer),
//! A3 (tree construction), A4 (CSS parser), and A5 (selectors) are real
//! now -- this demonstrates parsing a real document, parsing a real
//! stylesheet, and matching a real selector against the resulting tree.
//! Layout/paint are still placeholders (Track B).

fn main() {
    let input = "<!DOCTYPE html><html><body><p class=\"greeting\">hello</p></body></html>";
    let document = html::parse_document(input);

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
    println!("(layout/paint stages ran but are placeholders -- see ROADMAP.md Track B)");
}

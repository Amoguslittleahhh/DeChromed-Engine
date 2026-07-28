//! Roadmap phase: Track F7 (browser UI shell), but today this is just a
//! CLI smoke test wiring every crate together end to end -- html -> dom ->
//! (eventually) css -> layout -> paint -- so A1's exit criterion ("empty
//! but structured repo") means something more concrete than "it compiles":
//! the pipeline shape actually runs, even though every stage is currently
//! a placeholder per its own crate's docs.

fn main() {
    let input = "<p>hello</p>";

    let tokens = html::tokenize(input);
    let document = html::build_tree(&tokens);
    let _stylesheet = css::parse_stylesheet("p { color: red; }");
    let fragments = layout::layout();
    let _display_list = paint::build_display_list(&fragments);

    println!("DeChromed Engine -- pipeline smoke test");
    println!("input: {input:?}");
    println!("tokens: {tokens:?}");
    println!("dom:\n{document}");
    println!("(css/layout/paint stages ran but are placeholders -- see ROADMAP.md Track A/B)");
}

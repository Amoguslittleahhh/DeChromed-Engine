//! A8/A9: the WHATWG HTML parsing spec's "foreign content" machinery --
//! the tag-name/attribute-name adjustment tables and the breakout/
//! integration-point predicates `tree_builder.rs` needs to implement
//! <https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-inforeign>.
//!
//! Kept in its own module because it's almost entirely static spec tables
//! (transcribed by hand from the spec's own listings, not generated --
//! see the "known gap" note below) plus small pure predicate functions,
//! distinct from `tree_builder.rs`'s stateful insertion-mode logic.
//!
//! Known gap: [`adjust_svg_tag_name`] and [`adjust_svg_attributes`] aim to
//! cover the spec's full tables but were transcribed from memory of
//! the spec text rather than generated from it -- real vendored-corpus
//! test cases (`svg.dat`, `tests9.dat`-`tests12.dat`, `namespace-
//! sensitivity.dat`, ...) are what actually validates them, the same
//! "measure against the real corpus, fix what it finds" loop as A2/A3.
//! Rarely-used filter-primitive tag/attribute names are more likely to
//! have gaps than the common ones (`viewBox`, `foreignObject`,
//! `linearGradient`, ...), which are covered and tested.

/// <https://html.spec.whatwg.org/multipage/parsing.html#adjust-svg-tag-names>
/// Maps the tokenizer's all-lowercase tag name back to SVG's real
/// (partially camelCase) local name. Tags not listed here pass through
/// unchanged -- most SVG tag names (`path`, `rect`, `g`, `svg`, ...) are
/// already all-lowercase and need no adjustment.
const SVG_TAG_ADJUSTMENTS: &[(&str, &str)] = &[
    ("altglyph", "altGlyph"),
    ("altglyphdef", "altGlyphDef"),
    ("altglyphitem", "altGlyphItem"),
    ("animatecolor", "animateColor"),
    ("animatemotion", "animateMotion"),
    ("animatetransform", "animateTransform"),
    ("clippath", "clipPath"),
    ("feblend", "feBlend"),
    ("fecolormatrix", "feColorMatrix"),
    ("fecomponenttransfer", "feComponentTransfer"),
    ("fecomposite", "feComposite"),
    ("feconvolvematrix", "feConvolveMatrix"),
    ("fediffuselighting", "feDiffuseLighting"),
    ("fedisplacementmap", "feDisplacementMap"),
    ("fedistantlight", "feDistantLight"),
    ("fedropshadow", "feDropShadow"),
    ("feflood", "feFlood"),
    ("fefunca", "feFuncA"),
    ("fefuncb", "feFuncB"),
    ("fefuncg", "feFuncG"),
    ("fefuncr", "feFuncR"),
    ("fegaussianblur", "feGaussianBlur"),
    ("feimage", "feImage"),
    ("femerge", "feMerge"),
    ("femergenode", "feMergeNode"),
    ("femorphology", "feMorphology"),
    ("feoffset", "feOffset"),
    ("fepointlight", "fePointLight"),
    ("fespecularlighting", "feSpecularLighting"),
    ("fespotlight", "feSpotLight"),
    ("fetile", "feTile"),
    ("feturbulence", "feTurbulence"),
    ("foreignobject", "foreignObject"),
    ("glyphref", "glyphRef"),
    ("lineargradient", "linearGradient"),
    ("radialgradient", "radialGradient"),
    ("textpath", "textPath"),
];

/// <https://html.spec.whatwg.org/multipage/parsing.html#adjust-svg-attributes>
/// Same idea as [`SVG_TAG_ADJUSTMENTS`] but for attribute *local* names
/// (no namespace change -- that's [`FOREIGN_ATTR_NAMESPACES`]).
const SVG_ATTR_ADJUSTMENTS: &[(&str, &str)] = &[
    ("attributename", "attributeName"),
    ("attributetype", "attributeType"),
    ("basefrequency", "baseFrequency"),
    ("baseprofile", "baseProfile"),
    ("calcmode", "calcMode"),
    ("clippathunits", "clipPathUnits"),
    ("diffuseconstant", "diffuseConstant"),
    ("edgemode", "edgeMode"),
    ("filterunits", "filterUnits"),
    ("glyphref", "glyphRef"),
    ("gradienttransform", "gradientTransform"),
    ("gradientunits", "gradientUnits"),
    ("kernelmatrix", "kernelMatrix"),
    ("kernelunitlength", "kernelUnitLength"),
    ("keypoints", "keyPoints"),
    ("keysplines", "keySplines"),
    ("keytimes", "keyTimes"),
    ("lengthadjust", "lengthAdjust"),
    ("limitingconeangle", "limitingConeAngle"),
    ("markerheight", "markerHeight"),
    ("markerunits", "markerUnits"),
    ("markerwidth", "markerWidth"),
    ("maskcontentunits", "maskContentUnits"),
    ("maskunits", "maskUnits"),
    ("numoctaves", "numOctaves"),
    ("pathlength", "pathLength"),
    ("patterncontentunits", "patternContentUnits"),
    ("patterntransform", "patternTransform"),
    ("patternunits", "patternUnits"),
    ("pointsatx", "pointsAtX"),
    ("pointsaty", "pointsAtY"),
    ("pointsatz", "pointsAtZ"),
    ("preservealpha", "preserveAlpha"),
    ("preserveaspectratio", "preserveAspectRatio"),
    ("primitiveunits", "primitiveUnits"),
    ("refx", "refX"),
    ("refy", "refY"),
    ("repeatcount", "repeatCount"),
    ("repeatdur", "repeatDur"),
    ("requiredextensions", "requiredExtensions"),
    ("requiredfeatures", "requiredFeatures"),
    ("specularconstant", "specularConstant"),
    ("specularexponent", "specularExponent"),
    ("spreadmethod", "spreadMethod"),
    ("startoffset", "startOffset"),
    ("stddeviation", "stdDeviation"),
    ("stitchtiles", "stitchTiles"),
    ("surfacescale", "surfaceScale"),
    ("systemlanguage", "systemLanguage"),
    ("tablevalues", "tableValues"),
    ("targetx", "targetX"),
    ("targety", "targetY"),
    ("textlength", "textLength"),
    ("viewbox", "viewBox"),
    ("viewtarget", "viewTarget"),
    ("xchannelselector", "xChannelSelector"),
    ("ychannelselector", "yChannelSelector"),
    ("zoomandpan", "zoomAndPan"),
];

/// <https://html.spec.whatwg.org/multipage/parsing.html#adjust-foreign-attributes>
/// Attributes namespace-qualified regardless of which foreign namespace
/// they appear in: `(attribute name as written, namespace prefix, local
/// name)`. Used only for the html5lib-tests dump format (`xlink href`
/// instead of `xlink:href`) -- see `html5lib_harness`'s serializer -- since
/// this crate's `dom::ElementData` doesn't model per-attribute namespaces,
/// only element namespaces (a documented simplification: real attribute
/// namespace resolution is deferred, same spirit as A3's namespace gap
/// this phase otherwise closes for elements).
pub const FOREIGN_ATTR_NAMESPACES: &[(&str, &str, &str)] = &[
    ("xlink:actuate", "xlink", "actuate"),
    ("xlink:arcrole", "xlink", "arcrole"),
    ("xlink:href", "xlink", "href"),
    ("xlink:role", "xlink", "role"),
    ("xlink:show", "xlink", "show"),
    ("xlink:title", "xlink", "title"),
    ("xlink:type", "xlink", "type"),
    ("xml:lang", "xml", "lang"),
    ("xml:space", "xml", "space"),
    ("xmlns", "xmlns", "xmlns"),
    ("xmlns:xlink", "xmlns", "xlink"),
];

pub fn adjust_svg_tag_name(name: &str) -> &str {
    SVG_TAG_ADJUSTMENTS
        .iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| *v)
        .unwrap_or(name)
}

/// <https://html.spec.whatwg.org/multipage/parsing.html#adjust-mathml-attributes>
/// -- a single-entry table, unlike SVG's.
pub fn adjust_mathml_attributes(attrs: Vec<(String, String)>) -> Vec<(String, String)> {
    attrs
        .into_iter()
        .map(|(k, v)| {
            if k == "definitionurl" {
                ("definitionURL".to_string(), v)
            } else {
                (k, v)
            }
        })
        .collect()
}

pub fn adjust_svg_attributes(attrs: Vec<(String, String)>) -> Vec<(String, String)> {
    attrs
        .into_iter()
        .map(|(k, v)| {
            let adjusted = SVG_ATTR_ADJUSTMENTS
                .iter()
                .find(|(from, _)| *from == k)
                .map(|(_, to)| (*to).to_string())
                .unwrap_or(k);
            (adjusted, v)
        })
        .collect()
}

/// <https://html.spec.whatwg.org/multipage/parsing.html#parsing-main-inforeign>'s
/// "any other start tag" step: these tag names (plus `font` with a
/// `color`/`face`/`size` attribute) force a "breakout" back to HTML
/// content even while inside foreign content.
const BREAKOUT_TAGS: &[&str] = &[
    "b",
    "big",
    "blockquote",
    "body",
    "br",
    "center",
    "code",
    "dd",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "hr",
    "i",
    "img",
    "li",
    "listing",
    "menu",
    "meta",
    "nobr",
    "ol",
    "p",
    "pre",
    "ruby",
    "s",
    "small",
    "span",
    "strong",
    "strike",
    "sub",
    "sup",
    "table",
    "tt",
    "u",
    "ul",
    "var",
];

pub fn is_breakout_start_tag(name: &str, attributes: &[(String, String)]) -> bool {
    if BREAKOUT_TAGS.contains(&name) {
        return true;
    }
    name == "font"
        && attributes
            .iter()
            .any(|(k, _)| matches!(k.as_str(), "color" | "face" | "size"))
}

/// MathML namespace elements whose start tag (other than `mglyph`/
/// `malignmark`) or immediate character content is processed with the
/// current (HTML) insertion mode rather than foreign-content rules.
pub fn is_mathml_text_integration_point(namespace: &str, local_name: &str) -> bool {
    namespace == dom::MATHML_NS && matches!(local_name, "mi" | "mo" | "mn" | "ms" | "mtext")
}

/// The extra, foreign-namespace entries in the "has an element in the
/// specific scope" algorithm's boundary list (used for the *default*
/// scope and everything built on it -- list item, button):
/// <https://html.spec.whatwg.org/multipage/parsing.html#has-an-element-in-the-specific-scope>.
/// Unlike [`is_html_integration_point`], `annotation-xml` counts here
/// unconditionally (no `encoding` attribute check) -- scope boundaries and
/// integration points are similar but not the same predicate.
pub fn is_scope_boundary_foreign_element(namespace: &str, local_name: &str) -> bool {
    is_mathml_text_integration_point(namespace, local_name)
        || (namespace == dom::MATHML_NS && local_name == "annotation-xml")
        || (namespace == dom::SVG_NS && matches!(local_name, "foreignObject" | "desc" | "title"))
}

/// SVG's own HTML integration points (`foreignObject`/`desc`/`title`), plus
/// MathML's `annotation-xml` when its `encoding` attribute says its content
/// is HTML/XHTML -- both let ordinary HTML markup appear directly inside
/// foreign content without another namespace switch.
pub fn is_html_integration_point(
    namespace: &str,
    local_name: &str,
    attributes: &[(String, String)],
) -> bool {
    if namespace == dom::SVG_NS && matches!(local_name, "foreignObject" | "desc" | "title") {
        return true;
    }
    if namespace == dom::MATHML_NS && local_name == "annotation-xml" {
        return attributes.iter().any(|(k, v)| {
            k == "encoding"
                && (v.eq_ignore_ascii_case("text/html")
                    || v.eq_ignore_ascii_case("application/xhtml+xml"))
        });
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_tag_name_adjustment() {
        assert_eq!(adjust_svg_tag_name("foreignobject"), "foreignObject");
        assert_eq!(adjust_svg_tag_name("lineargradient"), "linearGradient");
        assert_eq!(adjust_svg_tag_name("path"), "path");
    }

    #[test]
    fn svg_attribute_adjustment() {
        let adjusted = adjust_svg_attributes(vec![
            ("viewbox".to_string(), "0 0 1 1".to_string()),
            ("d".to_string(), "M0 0".to_string()),
        ]);
        assert_eq!(adjusted[0].0, "viewBox");
        assert_eq!(adjusted[1].0, "d");
    }

    #[test]
    fn breakout_tags() {
        assert!(is_breakout_start_tag("div", &[]));
        assert!(is_breakout_start_tag(
            "font",
            &[("color".to_string(), "red".to_string())]
        ));
        assert!(!is_breakout_start_tag("font", &[]));
        assert!(!is_breakout_start_tag("path", &[]));
    }

    #[test]
    fn integration_points() {
        assert!(is_html_integration_point(dom::SVG_NS, "foreignObject", &[]));
        assert!(is_html_integration_point(
            dom::MATHML_NS,
            "annotation-xml",
            &[("encoding".to_string(), "text/html".to_string())]
        ));
        assert!(!is_html_integration_point(
            dom::MATHML_NS,
            "annotation-xml",
            &[]
        ));
        assert!(is_mathml_text_integration_point(dom::MATHML_NS, "mi"));
        assert!(!is_mathml_text_integration_point(dom::SVG_NS, "mi"));
    }
}

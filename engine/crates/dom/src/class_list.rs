//! C1: real `DOMTokenList` semantics for `class` (`Element.classList`) --
//! a live, order-preserving, duplicate-free space-separated token set
//! backed directly by the element's own `class` attribute string, per the
//! spec's own "ordered set parser"/"ordered set serializer" algorithms.
//!
//! Reference: <https://dom.spec.whatwg.org/#concept-ordered-set-parser>,
//! <https://dom.spec.whatwg.org/#interface-domtokenlist>.
//!
//! There's no separate cached token list to go stale here: every
//! operation reads and writes the attribute string directly, live by the
//! same "no cache to begin with" reasoning `api.rs`'s own module doc
//! gives for `NodeList`/`HTMLCollection`.
//!
//! Known gap: the real ordered-set parser splits on any ASCII whitespace
//! (space, tab, newline, form feed, CR) -- this uses `str::
//! split_ascii_whitespace`, which matches that definition exactly for
//! well-formed input, but doesn't separately validate tokens against
//! `DOMTokenList`'s own "empty string" / "contains ASCII whitespace"
//! `SyntaxError`/`InvalidCharacterError` checks (this crate has no
//! exception type to raise; `add_class("")` or a token containing a
//! space would currently just be filtered out / treated per-token by
//! the whitespace splitter rather than rejected outright).

use crate::ElementData;

fn parse_tokens(value: &str) -> Vec<&str> {
    value.split_ascii_whitespace().collect()
}

impl ElementData {
    /// `classList.contains(token)`.
    pub fn has_class(&self, token: &str) -> bool {
        self.attr("class")
            .is_some_and(|v| parse_tokens(v).contains(&token))
    }

    /// `classList.add(token)`: a real ordered-*set* insert -- a no-op if
    /// `token` is already present, rather than appending a duplicate or
    /// moving the existing occurrence.
    pub fn add_class(&mut self, token: &str) {
        if token.is_empty() || self.has_class(token) {
            return;
        }
        let mut tokens: Vec<String> = self
            .attr("class")
            .map(|v| parse_tokens(v).into_iter().map(str::to_string).collect())
            .unwrap_or_default();
        tokens.push(token.to_string());
        self.set_class_tokens(&tokens);
    }

    /// `classList.remove(token)`.
    pub fn remove_class(&mut self, token: &str) {
        let Some(value) = self.attr("class") else {
            return;
        };
        let tokens: Vec<String> = parse_tokens(value)
            .into_iter()
            .filter(|t| *t != token)
            .map(str::to_string)
            .collect();
        self.set_class_tokens(&tokens);
    }

    /// `classList.toggle(token)`, returning whether `token` is present
    /// afterward -- matching `DOMTokenList.toggle`'s own return value,
    /// which real code commonly branches on.
    pub fn toggle_class(&mut self, token: &str) -> bool {
        if self.has_class(token) {
            self.remove_class(token);
            false
        } else {
            self.add_class(token);
            true
        }
    }

    /// The real ordered-set serializer: tokens joined by a single space,
    /// in their current (insertion/removal-preserving) order.
    fn set_class_tokens(&mut self, tokens: &[String]) {
        let serialized = tokens.join(" ");
        match self.attributes.iter_mut().find(|(k, _)| k == "class") {
            Some((_, v)) => *v = serialized,
            None => self.attributes.push(("class".to_string(), serialized)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el() -> ElementData {
        ElementData::html("div", vec![])
    }

    #[test]
    fn add_appends_a_new_token() {
        let mut e = el();
        e.add_class("a");
        e.add_class("b");
        assert_eq!(e.attr("class"), Some("a b"));
        assert!(e.has_class("a"));
        assert!(e.has_class("b"));
        assert!(!e.has_class("c"));
    }

    #[test]
    fn add_is_a_no_op_for_an_existing_token() {
        let mut e = el();
        e.add_class("a");
        e.add_class("a");
        assert_eq!(e.attr("class"), Some("a"));
    }

    #[test]
    fn remove_drops_only_the_named_token_preserving_order() {
        let mut e = el();
        e.add_class("a");
        e.add_class("b");
        e.add_class("c");
        e.remove_class("b");
        assert_eq!(e.attr("class"), Some("a c"));
    }

    #[test]
    fn remove_of_an_absent_token_or_with_no_class_attribute_is_a_no_op() {
        let mut e = el();
        e.remove_class("a");
        assert_eq!(e.attr("class"), None);
        e.add_class("a");
        e.remove_class("nonexistent");
        assert_eq!(e.attr("class"), Some("a"));
    }

    #[test]
    fn toggle_adds_when_absent_and_removes_when_present() {
        let mut e = el();
        assert!(e.toggle_class("a"));
        assert!(e.has_class("a"));
        assert!(!e.toggle_class("a"));
        assert!(!e.has_class("a"));
    }

    #[test]
    fn multiple_whitespace_separated_tokens_in_source_all_parse() {
        let mut e = el();
        e.attributes
            .push(("class".to_string(), "a  b\tc".to_string()));
        assert!(e.has_class("a"));
        assert!(e.has_class("b"));
        assert!(e.has_class("c"));
    }
}

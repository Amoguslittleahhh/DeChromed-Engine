//! A7 (part 1): a mutable CSSOM-shaped object graph.
//!
//! Reference: <https://www.w3.org/TR/cssom-1/>.
//!
//! `parse_stylesheet()` (`lib.rs`) produces an immutable, already-flattened
//! `Stylesheet` -- fine for A4-A6, but the real
//! [`CSSStyleSheet`](https://www.w3.org/TR/cssom-1/#the-cssstylesheet-interface)/
//! [`CSSRule`](https://www.w3.org/TR/cssom-1/#the-cssrule-interface) object
//! graph is mutable: script inserts/deletes rules, and edits a rule's
//! declaration block in place, and each of those mutations needs to be
//! individually reflected without reparsing the whole sheet. `CssomSheet`/
//! `CssomRule` here are that mutable graph at the Rust level.
//!
//! Known gap: this is the object graph a JS binding layer would wrap, not
//! the binding layer itself -- there's no IDL/JS-visible `CSSStyleSheet`
//! constructor or `document.styleSheets` yet, since that needs Track C8
//! (DOM<->JS bindings), which doesn't exist. `@media`/`@supports` rules are
//! still flattened away rather than kept as live `CSSConditionRule`
//! objects, inheriting A4's "condition not evaluated" simplification;
//! other at-rules (`@font-face`, `@import`, ...) round-trip as opaque text
//! via `AtRuleSummary`, same as `Stylesheet`.

use crate::{AtRuleSummary, Declaration};

/// A mutable, in-memory analogue of `CSSStyleSheet`: an ordered list of
/// rules that can be inserted/deleted in place.
#[derive(Debug, Clone, Default)]
pub struct CssomSheet {
    pub rules: Vec<CssomRule>,
}

#[derive(Debug, Clone)]
pub enum CssomRule {
    Style(CssomStyleRule),
    At(AtRuleSummary),
}

/// A mutable analogue of `CSSStyleRule`/`CSSStyleDeclaration` combined --
/// real CSSOM splits these into two objects connected by `.style`, but
/// nothing here needs that indirection yet.
#[derive(Debug, Clone)]
pub struct CssomStyleRule {
    pub selector_text: String,
    pub declarations: Vec<Declaration>,
}

impl CssomStyleRule {
    /// <https://www.w3.org/TR/cssom-1/#dom-cssstyledeclaration-getpropertyvalue>
    pub fn get_property_value(&self, name: &str) -> Option<&str> {
        self.declarations
            .iter()
            .find(|d| d.property == name)
            .map(|d| d.value.as_str())
    }

    /// <https://www.w3.org/TR/cssom-1/#dom-cssstyledeclaration-setproperty>
    /// Replaces the value (and importance) of an existing declaration for
    /// `name`, or appends a new one if none exists yet.
    pub fn set_property(&mut self, name: &str, value: &str, important: bool) {
        if let Some(decl) = self.declarations.iter_mut().find(|d| d.property == name) {
            decl.value = value.to_string();
            decl.important = important;
        } else {
            self.declarations.push(Declaration {
                property: name.to_string(),
                value: value.to_string(),
                important,
            });
        }
    }

    /// <https://www.w3.org/TR/cssom-1/#dom-cssstyledeclaration-removeproperty>
    pub fn remove_property(&mut self, name: &str) -> Option<String> {
        let pos = self.declarations.iter().position(|d| d.property == name)?;
        Some(self.declarations.remove(pos).value)
    }

    /// <https://www.w3.org/TR/cssom-1/#serialize-a-css-declaration-block>
    /// (simplified: doesn't attempt shorthand reconstruction).
    pub fn css_text(&self) -> String {
        let mut out = format!("{} {{ ", self.selector_text);
        for decl in &self.declarations {
            out.push_str(&decl.property);
            out.push_str(": ");
            out.push_str(&decl.value);
            if decl.important {
                out.push_str(" !important");
            }
            out.push_str("; ");
        }
        out.push('}');
        out
    }
}

#[derive(Debug)]
pub struct CssomError(pub String);

impl CssomSheet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses `css_text` as a full stylesheet and builds a `CssomSheet`
    /// from it (qualified rules become `CssomRule::Style`, `@media`/
    /// `@supports`/... contents get flattened the same way
    /// `parse_stylesheet` does -- see module docs).
    pub fn parse(css_text: &str) -> Self {
        let sheet = crate::parse_stylesheet(css_text);
        let mut rules: Vec<CssomRule> = sheet
            .rules
            .into_iter()
            .map(|r| {
                CssomRule::Style(CssomStyleRule {
                    selector_text: r.selector,
                    declarations: r.declarations,
                })
            })
            .collect();
        rules.extend(sheet.other_at_rules.into_iter().map(CssomRule::At));
        CssomSheet { rules }
    }

    /// <https://www.w3.org/TR/cssom-1/#dom-cssstylesheet-insertrule>
    /// Parses `rule_text` as a single rule and inserts it at `index`.
    pub fn insert_rule(&mut self, rule_text: &str, index: usize) -> Result<(), CssomError> {
        if index > self.rules.len() {
            return Err(CssomError(format!(
                "index {index} out of bounds for {} rules",
                self.rules.len()
            )));
        }
        let sheet = crate::parse_stylesheet(rule_text);
        let rule = if let Some(r) = sheet.rules.into_iter().next() {
            CssomRule::Style(CssomStyleRule {
                selector_text: r.selector,
                declarations: r.declarations,
            })
        } else if let Some(at) = sheet.other_at_rules.into_iter().next() {
            CssomRule::At(at)
        } else {
            return Err(CssomError("no rule found in rule text".to_string()));
        };
        self.rules.insert(index, rule);
        Ok(())
    }

    /// <https://www.w3.org/TR/cssom-1/#dom-cssstylesheet-deleterule>
    pub fn delete_rule(&mut self, index: usize) -> Result<(), CssomError> {
        if index >= self.rules.len() {
            return Err(CssomError(format!(
                "index {index} out of bounds for {} rules",
                self.rules.len()
            )));
        }
        self.rules.remove(index);
        Ok(())
    }

    /// Re-flattens this sheet's style rules into the plain [`crate::Stylesheet`]
    /// shape A6's cascade operates on.
    pub fn to_stylesheet(&self) -> crate::Stylesheet {
        let mut sheet = crate::Stylesheet::default();
        for rule in &self.rules {
            match rule {
                CssomRule::Style(s) => sheet.rules.push(crate::Rule {
                    selector: s.selector_text.clone(),
                    declarations: s.declarations.clone(),
                }),
                CssomRule::At(a) => sheet.other_at_rules.push(a.clone()),
            }
        }
        sheet
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_serialize() {
        let sheet = CssomSheet::parse("p { color: red; }");
        assert_eq!(sheet.rules.len(), 1);
        match &sheet.rules[0] {
            CssomRule::Style(s) => assert_eq!(s.css_text(), "p { color: red; }"),
            _ => panic!("expected style rule"),
        }
    }

    #[test]
    fn insert_and_delete_rule() {
        let mut sheet = CssomSheet::parse("p { color: red; }");
        sheet.insert_rule("div { color: blue; }", 0).unwrap();
        assert_eq!(sheet.rules.len(), 2);
        match &sheet.rules[0] {
            CssomRule::Style(s) => assert_eq!(s.selector_text, "div"),
            _ => panic!("expected style rule"),
        }
        sheet.delete_rule(0).unwrap();
        assert_eq!(sheet.rules.len(), 1);
        assert!(sheet.delete_rule(5).is_err());
    }

    #[test]
    fn set_and_remove_property() {
        let mut sheet = CssomSheet::parse("p { color: red; }");
        let CssomRule::Style(rule) = &mut sheet.rules[0] else {
            panic!()
        };
        rule.set_property("color", "green", true);
        assert_eq!(rule.get_property_value("color"), Some("green"));
        rule.set_property("margin", "0", false);
        assert_eq!(rule.declarations.len(), 2);
        assert_eq!(rule.remove_property("margin"), Some("0".to_string()));
        assert_eq!(rule.declarations.len(), 1);
    }
}

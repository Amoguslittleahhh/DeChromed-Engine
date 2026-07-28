//! A5: Selectors Level 4 -- parsing selector text into a real AST, and
//! matching that AST against a [`dom::Document`] tree.
//!
//! Reference: <https://www.w3.org/TR/selectors-4/>
//!
//! Implemented: type/universal/id/class/attribute selectors (with all six
//! attribute matchers and the `i`/`s` case-sensitivity flag), all four
//! combinators (descendant, child `>`, next-sibling `+`, subsequent-sibling
//! `~`), structural pseudo-classes (`:first-child`, `:last-child`,
//! `:only-child`, the `-of-type` family, `:nth-child`/`:nth-last-child`
//! with full `An+B` parsing and the `of <selector>` extension, `:root`,
//! `:empty`), and the logical pseudo-classes `:not()`, `:is()`, `:where()`,
//! `:has()` (with nested selector lists, matched via real tree traversal).
//!
//! Not implemented: pseudo-*elements* (`::before`/`::after`/etc: parsed
//! without erroring, but never match anything, since they don't correspond
//! to real DOM nodes) and interaction/document-state-dependent
//! pseudo-classes (`:hover`, `:focus`, `:checked`, `:disabled`, ...) --
//! there's no interaction or form state modeled yet for these to reflect,
//! so they parse successfully but always evaluate to "not matched" rather
//! than erroring, which is the same "known gap, not silently wrong" spirit
//! as A2/A3's documented gaps.

use crate::tokenizer::{Token, Tokenizer};
use dom::{Document, NodeData, NodeId};

#[derive(Debug, Clone, PartialEq)]
pub struct SelectorList(pub Vec<ComplexSelector>);

#[derive(Debug, Clone, PartialEq)]
pub struct ComplexSelector {
    /// Left-to-right compound selectors as written (`div`, `p`, `span` for
    /// `"div p span"`), each paired with the combinator connecting it to
    /// the *previous* step -- `steps[0].combinator` is always `None`.
    /// Matching walks this backward from `steps.last()` (the subject
    /// element actually being tested) toward `steps[0]`, since each
    /// combinator constrains what's to its *left*.
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub combinator: Option<Combinator>,
    pub compound: CompoundSelector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    Descendant,
    Child,
    NextSibling,
    SubsequentSibling,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CompoundSelector {
    pub type_selector: Option<TypeSelector>,
    pub subclasses: Vec<SubclassSelector>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeSelector {
    Universal,
    Named(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SubclassSelector {
    Id(String),
    Class(String),
    Attribute(AttrSelector),
    PseudoClass(PseudoClass),
    PseudoElement(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AttrSelector {
    pub name: String,
    pub matcher: Option<(AttrMatcher, String, bool)>, // (op, value, case_insensitive)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrMatcher {
    Equals,
    Includes,
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnB {
    pub a: i32,
    pub b: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PseudoClass {
    FirstChild,
    LastChild,
    OnlyChild,
    FirstOfType,
    LastOfType,
    OnlyOfType,
    NthChild(AnB, Option<Box<SelectorList>>),
    NthLastChild(AnB, Option<Box<SelectorList>>),
    NthOfType(AnB),
    NthLastOfType(AnB),
    Root,
    Empty,
    Not(SelectorList),
    Is(SelectorList),
    Where(SelectorList),
    Has(SelectorList),
    /// Any other pseudo-class name (`:hover`, `:checked`, ...): parses
    /// successfully, but see module docs -- it never matches, since no
    /// interaction/form state is modeled to check it against.
    Unsupported(String),
}

#[derive(Debug)]
pub struct SelectorParseError(pub String);

pub fn parse_selector_list(input: &str) -> Result<SelectorList, SelectorParseError> {
    let mut p = SelectorParser::new(input);
    p.parse_selector_list()
}

struct SelectorParser {
    tokens: Vec<Token>,
    pos: usize,
}

impl SelectorParser {
    fn new(input: &str) -> Self {
        let mut tokenizer = Tokenizer::new(input);
        let mut tokens = Vec::new();
        loop {
            let t = tokenizer.next_token();
            let done = matches!(t, Token::Eof);
            if !matches!(t, Token::Whitespace) || !tokens.is_empty() {
                tokens.push(t);
            }
            if done {
                break;
            }
        }
        // Trim leading whitespace we couldn't skip above without losing
        // "first token" bookkeeping, and any trailing whitespace before EOF.
        while tokens.len() > 1 && matches!(tokens.first(), Some(Token::Whitespace)) {
            tokens.remove(0);
        }
        SelectorParser { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> Token {
        let t = self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof);
        if !matches!(t, Token::Eof) {
            self.pos += 1;
        }
        t
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Token::Whitespace) {
            self.advance();
        }
    }

    fn err(&self, msg: &str) -> SelectorParseError {
        SelectorParseError(format!("{msg} at token {:?}", self.peek()))
    }

    fn parse_selector_list(&mut self) -> Result<SelectorList, SelectorParseError> {
        let mut list = vec![self.parse_complex_selector()?];
        loop {
            self.skip_whitespace();
            if matches!(self.peek(), Token::Comma) {
                self.advance();
                self.skip_whitespace();
                list.push(self.parse_complex_selector()?);
            } else {
                break;
            }
        }
        Ok(SelectorList(list))
    }

    fn parse_complex_selector(&mut self) -> Result<ComplexSelector, SelectorParseError> {
        let first = self.parse_compound_selector()?;
        let mut steps = vec![Step {
            combinator: None,
            compound: first,
        }];
        loop {
            let combinator = self.parse_optional_combinator();
            match combinator {
                Some(c) => {
                    self.skip_whitespace();
                    let compound = self.parse_compound_selector()?;
                    steps.push(Step {
                        combinator: Some(c),
                        compound,
                    });
                }
                None => break,
            }
        }
        Ok(ComplexSelector { steps })
    }

    /// Looks for a combinator between compound selectors: an explicit `>`,
    /// `+`, `~`, or (if there's whitespace followed by something that isn't
    /// a comma/EOF/closing-paren) an implicit descendant combinator.
    fn parse_optional_combinator(&mut self) -> Option<Combinator> {
        let had_whitespace = matches!(self.peek(), Token::Whitespace);
        if had_whitespace {
            self.skip_whitespace();
        }
        match self.peek() {
            Token::Delim('>') => {
                self.advance();
                self.skip_whitespace();
                Some(Combinator::Child)
            }
            Token::Delim('+') => {
                self.advance();
                self.skip_whitespace();
                Some(Combinator::NextSibling)
            }
            Token::Delim('~') => {
                self.advance();
                self.skip_whitespace();
                Some(Combinator::SubsequentSibling)
            }
            Token::Comma | Token::Eof | Token::RightParen => None,
            _ if had_whitespace => Some(Combinator::Descendant),
            _ => None,
        }
    }

    fn parse_compound_selector(&mut self) -> Result<CompoundSelector, SelectorParseError> {
        let mut compound = CompoundSelector::default();
        if let Some(ts) = self.try_parse_type_selector() {
            compound.type_selector = Some(ts);
        }
        loop {
            match self.peek().clone() {
                Token::Hash { value, .. } => {
                    self.advance();
                    compound.subclasses.push(SubclassSelector::Id(value));
                }
                Token::Delim('.') => {
                    self.advance();
                    match self.advance() {
                        Token::Ident(name) => {
                            compound.subclasses.push(SubclassSelector::Class(name))
                        }
                        _ => return Err(self.err("expected class name after '.'")),
                    }
                }
                Token::LeftSquare => {
                    compound
                        .subclasses
                        .push(SubclassSelector::Attribute(self.parse_attr_selector()?));
                }
                Token::Colon => {
                    compound.subclasses.push(self.parse_pseudo()?);
                }
                _ => break,
            }
        }
        if compound.type_selector.is_none() && compound.subclasses.is_empty() {
            return Err(self.err("expected a selector"));
        }
        Ok(compound)
    }

    fn try_parse_type_selector(&mut self) -> Option<TypeSelector> {
        match self.peek().clone() {
            Token::Delim('*') => {
                self.advance();
                Some(TypeSelector::Universal)
            }
            Token::Ident(name) => {
                self.advance();
                Some(TypeSelector::Named(name.to_ascii_lowercase()))
            }
            _ => None,
        }
    }

    fn parse_attr_selector(&mut self) -> Result<AttrSelector, SelectorParseError> {
        self.advance(); // `[`
        self.skip_whitespace();
        let name = match self.advance() {
            Token::Ident(n) => n,
            _ => return Err(self.err("expected attribute name")),
        };
        self.skip_whitespace();
        let matcher = match self.peek().clone() {
            Token::RightSquare => None,
            Token::Delim('=') => {
                self.advance();
                Some(AttrMatcher::Equals)
            }
            Token::Delim(c @ ('~' | '|' | '^' | '$' | '*')) => {
                self.advance();
                if !matches!(self.advance(), Token::Delim('=')) {
                    return Err(self.err("expected '=' in attribute matcher"));
                }
                Some(match c {
                    '~' => AttrMatcher::Includes,
                    '|' => AttrMatcher::DashMatch,
                    '^' => AttrMatcher::Prefix,
                    '$' => AttrMatcher::Suffix,
                    _ => AttrMatcher::Substring,
                })
            }
            _ => return Err(self.err("expected attribute matcher or ']'")),
        };
        let full_matcher = if let Some(op) = matcher {
            self.skip_whitespace();
            let value = match self.advance() {
                Token::Str(s) => s,
                Token::Ident(s) => s,
                _ => return Err(self.err("expected attribute value")),
            };
            self.skip_whitespace();
            let case_insensitive = match self.peek() {
                Token::Ident(flag) if flag.eq_ignore_ascii_case("i") => {
                    self.advance();
                    true
                }
                Token::Ident(flag) if flag.eq_ignore_ascii_case("s") => {
                    self.advance();
                    false
                }
                _ => false,
            };
            self.skip_whitespace();
            Some((op, value, case_insensitive))
        } else {
            None
        };
        if !matches!(self.advance(), Token::RightSquare) {
            return Err(self.err("expected ']'"));
        }
        Ok(AttrSelector {
            name,
            matcher: full_matcher,
        })
    }

    fn parse_pseudo(&mut self) -> Result<SubclassSelector, SelectorParseError> {
        self.advance(); // first `:`
        let is_element = matches!(self.peek(), Token::Colon);
        if is_element {
            self.advance();
        }
        match self.advance() {
            Token::Ident(name) => {
                let lower = name.to_ascii_lowercase();
                if is_element
                    || matches!(
                        lower.as_str(),
                        "before" | "after" | "first-line" | "first-letter"
                    )
                {
                    return Ok(SubclassSelector::PseudoElement(lower));
                }
                Ok(SubclassSelector::PseudoClass(
                    self.simple_pseudo_class(&lower),
                ))
            }
            Token::Function(name) => {
                let lower = name.to_ascii_lowercase();
                let pc = self.functional_pseudo_class(&lower)?;
                if !matches!(self.advance(), Token::RightParen) {
                    return Err(self.err("expected ')' to close pseudo-class"));
                }
                Ok(SubclassSelector::PseudoClass(pc))
            }
            _ => Err(self.err("expected pseudo-class/element name")),
        }
    }

    fn simple_pseudo_class(&self, name: &str) -> PseudoClass {
        match name {
            "first-child" => PseudoClass::FirstChild,
            "last-child" => PseudoClass::LastChild,
            "only-child" => PseudoClass::OnlyChild,
            "first-of-type" => PseudoClass::FirstOfType,
            "last-of-type" => PseudoClass::LastOfType,
            "only-of-type" => PseudoClass::OnlyOfType,
            "root" => PseudoClass::Root,
            "empty" => PseudoClass::Empty,
            other => PseudoClass::Unsupported(other.to_string()),
        }
    }

    fn functional_pseudo_class(&mut self, name: &str) -> Result<PseudoClass, SelectorParseError> {
        match name {
            "not" => Ok(PseudoClass::Not(self.parse_selector_list()?)),
            "is" => Ok(PseudoClass::Is(self.parse_selector_list()?)),
            "where" => Ok(PseudoClass::Where(self.parse_selector_list()?)),
            "has" => Ok(PseudoClass::Has(self.parse_relative_selector_list()?)),
            "nth-child" => {
                let anb = self.parse_anb()?;
                self.skip_whitespace();
                let of_list = if matches!(self.peek(), Token::Ident(s) if s.eq_ignore_ascii_case("of"))
                {
                    self.advance();
                    self.skip_whitespace();
                    Some(Box::new(self.parse_selector_list()?))
                } else {
                    None
                };
                Ok(PseudoClass::NthChild(anb, of_list))
            }
            "nth-last-child" => {
                let anb = self.parse_anb()?;
                self.skip_whitespace();
                let of_list = if matches!(self.peek(), Token::Ident(s) if s.eq_ignore_ascii_case("of"))
                {
                    self.advance();
                    self.skip_whitespace();
                    Some(Box::new(self.parse_selector_list()?))
                } else {
                    None
                };
                Ok(PseudoClass::NthLastChild(anb, of_list))
            }
            "nth-of-type" => Ok(PseudoClass::NthOfType(self.parse_anb()?)),
            "nth-last-of-type" => Ok(PseudoClass::NthLastOfType(self.parse_anb()?)),
            _ => {
                // Unsupported functional pseudo-class (:lang(), :dir(),
                // ...): consume up to the matching ')' so the rest of the
                // selector list can still parse, but record it as never-
                // matching, same policy as simple unsupported pseudo-classes.
                let mut depth = 0i32;
                loop {
                    match self.peek() {
                        Token::RightParen if depth == 0 => break,
                        Token::LeftParen => {
                            depth += 1;
                            self.advance();
                        }
                        Token::RightParen => {
                            depth -= 1;
                            self.advance();
                        }
                        Token::Eof => break,
                        _ => {
                            self.advance();
                        }
                    }
                }
                Ok(PseudoClass::Unsupported(name.to_string()))
            }
        }
    }

    /// `:has()`'s argument is a *relative* selector list -- each one may
    /// start with an explicit combinator (meaning "relative to the anchor
    /// element"), defaulting to descendant if none is given. We normalize
    /// by parsing it as an ordinary selector list where the first compound
    /// is allowed to be empty (matching "any descendant"), then matching
    /// treats `first` specially when it's the wildcard placeholder.
    fn parse_relative_selector_list(&mut self) -> Result<SelectorList, SelectorParseError> {
        self.skip_whitespace();
        let mut list = Vec::new();
        loop {
            self.skip_whitespace();
            let leading_combinator = match self.peek() {
                Token::Delim('>') => {
                    self.advance();
                    Some(Combinator::Child)
                }
                Token::Delim('+') => {
                    self.advance();
                    Some(Combinator::NextSibling)
                }
                Token::Delim('~') => {
                    self.advance();
                    Some(Combinator::SubsequentSibling)
                }
                _ => None,
            };
            self.skip_whitespace();
            let combinator = leading_combinator.unwrap_or(Combinator::Descendant);
            let mut complex = self.parse_complex_selector()?;
            // Fold the relative combinator in as a synthetic first hop from
            // an empty "anchor" compound, so `has_matches_from` can walk it
            // forward from the `:has()` subject the same way for every case.
            complex.steps[0].combinator = Some(combinator);
            complex.steps.insert(
                0,
                Step {
                    combinator: None,
                    compound: CompoundSelector::default(),
                },
            );
            list.push(complex);
            self.skip_whitespace();
            if matches!(self.peek(), Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        Ok(SelectorList(list))
    }

    /// `<An+B>` micro-syntax: `odd`, `even`, `<integer>`, `n`, `-n`, `An`,
    /// `An+B`, `An-B`, with optional whitespace around the sign per spec.
    fn parse_anb(&mut self) -> Result<AnB, SelectorParseError> {
        self.skip_whitespace();
        if let Token::Ident(kw) = self.peek() {
            if kw.eq_ignore_ascii_case("odd") {
                self.advance();
                self.skip_whitespace();
                return Ok(AnB { a: 2, b: 1 });
            }
            if kw.eq_ignore_ascii_case("even") {
                self.advance();
                self.skip_whitespace();
                return Ok(AnB { a: 2, b: 0 });
            }
        }
        // Collect the raw "an+b" text by re-serializing the remaining
        // tokens up to (but not including) the closing paren or `of`
        // keyword, then parse it with a small dedicated scanner -- the
        // tokenizer already split things like "2n+1" into a Dimension
        // token (value=2, unit="n") followed by a Number(1) with an
        // explicit sign, or similar combinations, so it's easiest to
        // reason about on the already-tokenized form directly.
        let mut a = 0i32;
        let mut b = 0i32;
        let mut seen_n = false;
        match self.peek().clone() {
            Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("n") => {
                a = value as i32;
                seen_n = true;
                self.advance();
            }
            Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("n-") => {
                // e.g. "3n-1" tokenizes as Dimension(3, "n-") Number(1)
                a = value as i32;
                seen_n = true;
                self.advance();
                self.skip_whitespace();
                if let Token::Number { value, .. } = self.peek().clone() {
                    self.advance();
                    b = -(value as i32);
                    self.skip_whitespace();
                    return Ok(AnB { a, b });
                }
            }
            Token::Ident(s) if s.eq_ignore_ascii_case("n") => {
                a = 1;
                seen_n = true;
                self.advance();
            }
            Token::Ident(s) if s.eq_ignore_ascii_case("-n") => {
                a = -1;
                seen_n = true;
                self.advance();
            }
            Token::Number { value, .. } => {
                b = value as i32;
                self.advance();
                self.skip_whitespace();
                return Ok(AnB { a: 0, b });
            }
            Token::Delim('-') => {
                self.advance();
                if let Token::Ident(s) = self.peek().clone() {
                    if s.eq_ignore_ascii_case("n") {
                        a = -1;
                        seen_n = true;
                        self.advance();
                    }
                }
            }
            _ => return Err(self.err("expected An+B expression")),
        }
        self.skip_whitespace();
        if seen_n {
            match self.peek().clone() {
                Token::Number { value, .. } if value < 0.0 => {
                    b = value as i32;
                    self.advance();
                }
                Token::Delim('+') => {
                    self.advance();
                    self.skip_whitespace();
                    if let Token::Number { value, .. } = self.advance() {
                        b = value as i32;
                    } else {
                        return Err(self.err("expected integer after '+' in An+B"));
                    }
                }
                Token::Delim('-') => {
                    self.advance();
                    self.skip_whitespace();
                    if let Token::Number { value, .. } = self.advance() {
                        b = -(value as i32);
                    } else {
                        return Err(self.err("expected integer after '-' in An+B"));
                    }
                }
                _ => {}
            }
        }
        self.skip_whitespace();
        Ok(AnB { a, b })
    }
}

// ---------------------------- matching ----------------------------

fn element_children(doc: &Document, id: NodeId) -> Vec<NodeId> {
    doc.children(id)
        .iter()
        .copied()
        .filter(|&c| matches!(doc.data(c), NodeData::Element(_)))
        .collect()
}

fn index_among_element_siblings(doc: &Document, id: NodeId) -> Option<(usize, usize)> {
    let parent = doc.parent(id)?;
    let siblings = element_children(doc, parent);
    let pos = siblings.iter().position(|&s| s == id)?;
    Some((pos, siblings.len()))
}

fn index_among_same_type_siblings(doc: &Document, id: NodeId) -> Option<(usize, usize)> {
    let parent = doc.parent(id)?;
    let name = element_name(doc, id)?;
    let siblings: Vec<NodeId> = element_children(doc, parent)
        .into_iter()
        .filter(|&s| element_name(doc, s).as_deref() == Some(name.as_str()))
        .collect();
    let pos = siblings.iter().position(|&s| s == id)?;
    Some((pos, siblings.len()))
}

fn element_name(doc: &Document, id: NodeId) -> Option<String> {
    match doc.data(id) {
        NodeData::Element(el) => Some(el.local_name.clone()),
        _ => None,
    }
}

fn anb_matches(anb: AnB, one_indexed_pos: i32) -> bool {
    if anb.a == 0 {
        return one_indexed_pos == anb.b;
    }
    let k = one_indexed_pos - anb.b;
    k % anb.a == 0 && k / anb.a >= 0
}

/// Does `id` match `list` (any complex selector in it)?
pub fn matches(doc: &Document, id: NodeId, list: &SelectorList) -> bool {
    list.0.iter().any(|cs| matches_complex(doc, id, cs))
}

fn matches_complex(doc: &Document, id: NodeId, cs: &ComplexSelector) -> bool {
    match_step(doc, id, &cs.steps, cs.steps.len() - 1)
}

/// `steps` is left-to-right as written (`div`, `p`, `span` for
/// `"div p span"`), but `id` is always the element the *last* step
/// describes -- matching walks backward from there: check `steps[i]`
/// against the current node, then use `steps[i].combinator` to find which
/// node(s) `steps[i-1]` must match.
fn match_step(doc: &Document, id: NodeId, steps: &[Step], i: usize) -> bool {
    if !matches_compound(doc, id, &steps[i].compound) {
        return false;
    }
    if i == 0 {
        return true;
    }
    match steps[i]
        .combinator
        .expect("only steps[0] has no combinator")
    {
        Combinator::Child => match doc.parent(id) {
            Some(p) if matches!(doc.data(p), NodeData::Element(_)) => {
                match_step(doc, p, steps, i - 1)
            }
            _ => false,
        },
        Combinator::Descendant => {
            let mut cur = id;
            loop {
                match doc.parent(cur) {
                    Some(p) if matches!(doc.data(p), NodeData::Element(_)) => {
                        if match_step(doc, p, steps, i - 1) {
                            return true;
                        }
                        cur = p;
                    }
                    _ => return false,
                }
            }
        }
        Combinator::NextSibling => match previous_element_sibling(doc, id) {
            Some(p) => match_step(doc, p, steps, i - 1),
            None => false,
        },
        Combinator::SubsequentSibling => {
            let mut cur = id;
            loop {
                match previous_element_sibling(doc, cur) {
                    Some(p) => {
                        if match_step(doc, p, steps, i - 1) {
                            return true;
                        }
                        cur = p;
                    }
                    None => return false,
                }
            }
        }
    }
}

fn previous_element_sibling(doc: &Document, id: NodeId) -> Option<NodeId> {
    let parent = doc.parent(id)?;
    let siblings = element_children(doc, parent);
    let pos = siblings.iter().position(|&s| s == id)?;
    if pos == 0 {
        None
    } else {
        Some(siblings[pos - 1])
    }
}

fn matches_compound(doc: &Document, id: NodeId, compound: &CompoundSelector) -> bool {
    let el = match doc.data(id) {
        NodeData::Element(el) => el,
        _ => return false,
    };
    if let Some(ts) = &compound.type_selector {
        match ts {
            TypeSelector::Universal => {}
            TypeSelector::Named(name) => {
                if &el.local_name != name {
                    return false;
                }
            }
        }
    }
    compound
        .subclasses
        .iter()
        .all(|s| matches_subclass(doc, id, s))
}

fn matches_subclass(doc: &Document, id: NodeId, sub: &SubclassSelector) -> bool {
    let el = match doc.data(id) {
        NodeData::Element(el) => el,
        _ => return false,
    };
    match sub {
        SubclassSelector::Id(want) => el.attr("id") == Some(want.as_str()),
        SubclassSelector::Class(want) => el
            .attr("class")
            .map(|c| c.split_ascii_whitespace().any(|token| token == want))
            .unwrap_or(false),
        SubclassSelector::Attribute(attr) => matches_attr(el, attr),
        SubclassSelector::PseudoElement(_) => false,
        SubclassSelector::PseudoClass(pc) => matches_pseudo_class(doc, id, pc),
    }
}

fn matches_attr(el: &dom::ElementData, attr: &AttrSelector) -> bool {
    let Some(value) = el.attr(&attr.name) else {
        return false;
    };
    let Some((op, want, ci)) = &attr.matcher else {
        return true;
    };
    let (value, want): (String, String) = if *ci {
        (value.to_ascii_lowercase(), want.to_ascii_lowercase())
    } else {
        (value.to_string(), want.clone())
    };
    match op {
        AttrMatcher::Equals => value == want,
        AttrMatcher::Includes => value.split_ascii_whitespace().any(|t| t == want),
        AttrMatcher::DashMatch => value == want || value.starts_with(&format!("{want}-")),
        AttrMatcher::Prefix => !want.is_empty() && value.starts_with(&want),
        AttrMatcher::Suffix => !want.is_empty() && value.ends_with(&want),
        AttrMatcher::Substring => !want.is_empty() && value.contains(&want),
    }
}

fn matches_pseudo_class(doc: &Document, id: NodeId, pc: &PseudoClass) -> bool {
    match pc {
        PseudoClass::FirstChild => {
            index_among_element_siblings(doc, id).is_some_and(|(i, _)| i == 0)
        }
        PseudoClass::LastChild => {
            index_among_element_siblings(doc, id).is_some_and(|(i, n)| i + 1 == n)
        }
        PseudoClass::OnlyChild => {
            index_among_element_siblings(doc, id).is_some_and(|(_, n)| n == 1)
        }
        PseudoClass::FirstOfType => {
            index_among_same_type_siblings(doc, id).is_some_and(|(i, _)| i == 0)
        }
        PseudoClass::LastOfType => {
            index_among_same_type_siblings(doc, id).is_some_and(|(i, n)| i + 1 == n)
        }
        PseudoClass::OnlyOfType => {
            index_among_same_type_siblings(doc, id).is_some_and(|(_, n)| n == 1)
        }
        PseudoClass::Root => doc
            .parent(id)
            .map(|p| doc.parent(p).is_none())
            .unwrap_or(false),
        PseudoClass::Empty => doc.children(id).iter().all(|&c| match doc.data(c) {
            NodeData::Text(t) => t.is_empty(),
            NodeData::Element(_) => false,
            _ => true,
        }),
        PseudoClass::NthChild(anb, of_list) => match of_list {
            None => index_among_element_siblings(doc, id)
                .is_some_and(|(i, _)| anb_matches(*anb, i as i32 + 1)),
            Some(list) => {
                if !matches(doc, id, list) {
                    return false;
                }
                let Some(parent) = doc.parent(id) else {
                    return false;
                };
                let matching_siblings: Vec<NodeId> = element_children(doc, parent)
                    .into_iter()
                    .filter(|&s| matches(doc, s, list))
                    .collect();
                matching_siblings
                    .iter()
                    .position(|&s| s == id)
                    .is_some_and(|i| anb_matches(*anb, i as i32 + 1))
            }
        },
        PseudoClass::NthLastChild(anb, of_list) => match of_list {
            None => index_among_element_siblings(doc, id)
                .is_some_and(|(i, n)| anb_matches(*anb, (n - i) as i32)),
            Some(list) => {
                if !matches(doc, id, list) {
                    return false;
                }
                let Some(parent) = doc.parent(id) else {
                    return false;
                };
                let matching_siblings: Vec<NodeId> = element_children(doc, parent)
                    .into_iter()
                    .filter(|&s| matches(doc, s, list))
                    .collect();
                matching_siblings
                    .iter()
                    .position(|&s| s == id)
                    .is_some_and(|i| anb_matches(*anb, (matching_siblings.len() - i) as i32))
            }
        },
        PseudoClass::NthOfType(anb) => index_among_same_type_siblings(doc, id)
            .is_some_and(|(i, _)| anb_matches(*anb, i as i32 + 1)),
        PseudoClass::NthLastOfType(anb) => index_among_same_type_siblings(doc, id)
            .is_some_and(|(i, n)| anb_matches(*anb, (n - i) as i32)),
        PseudoClass::Not(list) => !matches(doc, id, list),
        PseudoClass::Is(list) | PseudoClass::Where(list) => matches(doc, id, list),
        PseudoClass::Has(list) => list.0.iter().any(|cs| has_matches_from(doc, id, cs)),
        PseudoClass::Unsupported(_) => false,
    }
}

/// `:has()` support: `cs.first` is always the empty placeholder compound
/// (see `parse_relative_selector_list`), so we start from `id` and walk
/// `cs.rest`'s combinators *forward* from the anchor -- the reverse
/// direction from ordinary complex-selector matching, since `:has()`'s
/// argument describes what must exist relative to (typically below) the
/// subject, not what the subject's ancestors must look like.
fn has_matches_from(doc: &Document, id: NodeId, cs: &ComplexSelector) -> bool {
    has_match_hops(doc, id, &cs.steps[1..], 0)
}

fn has_match_hops(doc: &Document, anchor: NodeId, hops: &[Step], idx: usize) -> bool {
    if idx >= hops.len() {
        return true;
    }
    let compound = &hops[idx].compound;
    let combinator = hops[idx]
        .combinator
        .expect("has() hops always carry a combinator");
    match combinator {
        Combinator::Child => element_children(doc, anchor)
            .into_iter()
            .any(|c| matches_compound(doc, c, compound) && has_match_hops(doc, c, hops, idx + 1)),
        Combinator::Descendant => {
            fn walk(
                doc: &Document,
                node: NodeId,
                compound: &CompoundSelector,
                hops: &[Step],
                idx: usize,
            ) -> bool {
                for child in element_children(doc, node) {
                    if matches_compound(doc, child, compound)
                        && has_match_hops(doc, child, hops, idx + 1)
                    {
                        return true;
                    }
                    if walk(doc, child, compound, hops, idx) {
                        return true;
                    }
                }
                false
            }
            walk(doc, anchor, compound, hops, idx)
        }
        Combinator::NextSibling => next_element_sibling(doc, anchor).is_some_and(|s| {
            matches_compound(doc, s, compound) && has_match_hops(doc, s, hops, idx + 1)
        }),
        Combinator::SubsequentSibling => {
            let mut cur = anchor;
            while let Some(s) = next_element_sibling(doc, cur) {
                if matches_compound(doc, s, compound) && has_match_hops(doc, s, hops, idx + 1) {
                    return true;
                }
                cur = s;
            }
            false
        }
    }
}

fn next_element_sibling(doc: &Document, id: NodeId) -> Option<NodeId> {
    let parent = doc.parent(id)?;
    let siblings = element_children(doc, parent);
    let pos = siblings.iter().position(|&s| s == id)?;
    siblings.get(pos + 1).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dom::{Document, ElementData, NodeData};

    fn el(doc: &mut Document, parent: NodeId, tag: &str, attrs: &[(&str, &str)]) -> NodeId {
        doc.append(
            parent,
            NodeData::Element(ElementData {
                local_name: tag.to_string(),
                attributes: attrs
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            }),
        )
    }

    #[test]
    fn type_and_class_and_id() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[("id", "main"), ("class", "a b")]);

        assert!(matches(&doc, div, &parse_selector_list("div").unwrap()));
        assert!(matches(&doc, div, &parse_selector_list("#main").unwrap()));
        assert!(matches(&doc, div, &parse_selector_list(".a").unwrap()));
        assert!(matches(&doc, div, &parse_selector_list(".b").unwrap()));
        assert!(matches(
            &doc,
            div,
            &parse_selector_list("div.a#main").unwrap()
        ));
        assert!(!matches(&doc, div, &parse_selector_list("span").unwrap()));
        assert!(!matches(&doc, div, &parse_selector_list(".c").unwrap()));
    }

    #[test]
    fn descendant_and_child_combinators() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[]);
        let p = el(&mut doc, div, "p", &[]);
        let span = el(&mut doc, p, "span", &[]);

        assert!(matches(
            &doc,
            span,
            &parse_selector_list("div span").unwrap()
        ));
        assert!(matches(
            &doc,
            span,
            &parse_selector_list("p > span").unwrap()
        ));
        assert!(!matches(
            &doc,
            span,
            &parse_selector_list("div > span").unwrap()
        ));
        assert!(matches(
            &doc,
            span,
            &parse_selector_list("div p span").unwrap()
        ));
    }

    #[test]
    fn sibling_combinators() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[]);
        let a = el(&mut doc, div, "a", &[]);
        let b = el(&mut doc, div, "b", &[]);
        let c = el(&mut doc, div, "c", &[]);
        let _ = a;

        assert!(matches(&doc, c, &parse_selector_list("b + c").unwrap()));
        assert!(matches(&doc, c, &parse_selector_list("a ~ c").unwrap()));
        assert!(!matches(&doc, b, &parse_selector_list("a + c").unwrap()));
    }

    #[test]
    fn attribute_selectors() {
        let mut doc = Document::new();
        let root = doc.root();
        let a = el(&mut doc, root, "a", &[("href", "https://example.com/foo")]);

        assert!(matches(&doc, a, &parse_selector_list("[href]").unwrap()));
        assert!(matches(
            &doc,
            a,
            &parse_selector_list("[href^=\"https\"]").unwrap()
        ));
        assert!(matches(
            &doc,
            a,
            &parse_selector_list("[href$=\"foo\"]").unwrap()
        ));
        assert!(matches(
            &doc,
            a,
            &parse_selector_list("[href*=\"example\"]").unwrap()
        ));
        assert!(!matches(
            &doc,
            a,
            &parse_selector_list("[href^=\"ftp\"]").unwrap()
        ));
    }

    #[test]
    fn nth_child_and_structural() {
        let mut doc = Document::new();
        let root = doc.root();
        let ul = el(&mut doc, root, "ul", &[]);
        let li1 = el(&mut doc, ul, "li", &[]);
        let li2 = el(&mut doc, ul, "li", &[]);
        let li3 = el(&mut doc, ul, "li", &[]);

        assert!(matches(
            &doc,
            li1,
            &parse_selector_list("li:first-child").unwrap()
        ));
        assert!(matches(
            &doc,
            li3,
            &parse_selector_list("li:last-child").unwrap()
        ));
        assert!(matches(
            &doc,
            li2,
            &parse_selector_list("li:nth-child(2)").unwrap()
        ));
        assert!(matches(
            &doc,
            li1,
            &parse_selector_list("li:nth-child(odd)").unwrap()
        ));
        assert!(matches(
            &doc,
            li3,
            &parse_selector_list("li:nth-child(odd)").unwrap()
        ));
        assert!(!matches(
            &doc,
            li2,
            &parse_selector_list("li:nth-child(odd)").unwrap()
        ));
        assert!(matches(
            &doc,
            li2,
            &parse_selector_list("li:nth-child(2n)").unwrap()
        ));
    }

    #[test]
    fn logical_pseudo_classes() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[("class", "a")]);
        let span = el(&mut doc, root, "span", &[]);

        assert!(matches(
            &doc,
            div,
            &parse_selector_list(":is(div, span)").unwrap()
        ));
        assert!(matches(
            &doc,
            span,
            &parse_selector_list(":is(div, span)").unwrap()
        ));
        assert!(matches(
            &doc,
            div,
            &parse_selector_list("div:not(.b)").unwrap()
        ));
        assert!(!matches(
            &doc,
            div,
            &parse_selector_list("div:not(.a)").unwrap()
        ));
    }

    #[test]
    fn has_pseudo_class() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[]);
        el(&mut doc, div, "span", &[]);
        let empty_div = el(&mut doc, root, "div", &[]);
        let _ = empty_div;

        assert!(matches(
            &doc,
            div,
            &parse_selector_list("div:has(span)").unwrap()
        ));
        assert!(matches(
            &doc,
            div,
            &parse_selector_list("div:has(> span)").unwrap()
        ));
    }
}

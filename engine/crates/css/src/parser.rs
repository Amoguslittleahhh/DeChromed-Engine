//! A4: parsing tokens into rules, per the CSS Syntax Module Level 3's
//! "5. Parsing" algorithms: component values, qualified rules, at-rules,
//! and declarations.
//!
//! Reference: <https://www.w3.org/TR/css-syntax-3/#parsing>
//!
//! Deliberately not implemented: CSS's error-recovery corner cases around
//! deeply malformed nested blocks get a simplified (but still
//! spec-consistent for well-formed input) treatment rather than an exact
//! transcription of every "consume the remnants of a bad declaration"
//! branch -- CSS parsing *must* degrade gracefully for genuinely malformed
//! input (that's the spec's whole point), and this parser does recover
//! rather than fail outright, just not always identically to the letter of
//! every edge case.
//!
//! **Nesting depth guard:** `consume_component_value`/`consume_simple_block`/
//! `consume_function_args` are mutually recursive by block/function nesting
//! depth, matching the spec's own recursive grammar -- but that means
//! pathologically deep input (`a{color:` + a million `(` + a million `)` +
//! `}`, found by stress-testing this crate) recurses the Rust call stack
//! deep enough to overflow it and abort the whole process, an unrecoverable
//! crash no `catch_unwind` can stop. [`MAX_NESTING_DEPTH`] caps recursion:
//! past it, a block's contents are consumed as a flat, unstructured token
//! run (see [`Parser::skip_balanced_flat`]) instead of recursing further.
//! Real content never nests anywhere close to this deep, so this only
//! changes behavior on the kind of adversarial input a real browser's own
//! (typically similar-order-of-magnitude) internal limits also exist to
//! defend against.

use crate::tokenizer::{Token, Tokenizer};

/// See the module docs' "Nesting depth guard" note. 256 is generously
/// above anything real (even heavily-nested preprocessor output rarely
/// exceeds a few dozen levels) while staying comfortably within a debug
/// build's default stack size.
const MAX_NESTING_DEPTH: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub enum ComponentValue {
    Token(Token),
    Function {
        name: String,
        args: Vec<ComponentValue>,
    },
    Block {
        open: BlockKind,
        contents: Vec<ComponentValue>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    Curly,
    Square,
    Paren,
}

#[derive(Debug, Clone)]
pub struct QualifiedRule {
    pub prelude: Vec<ComponentValue>,
    pub block: Vec<ComponentValue>,
}

#[derive(Debug, Clone)]
pub struct AtRule {
    pub name: String,
    pub prelude: Vec<ComponentValue>,
    pub block: Option<Vec<ComponentValue>>,
}

#[derive(Debug, Clone)]
pub enum Rule {
    Qualified(QualifiedRule),
    At(AtRule),
}

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
}

impl Parser {
    pub fn new(input: &str) -> Self {
        let mut tokenizer = Tokenizer::new(input);
        let mut tokens = Vec::new();
        loop {
            let t = tokenizer.next_token();
            let done = matches!(t, Token::Eof);
            tokens.push(t);
            if done {
                break;
            }
        }
        Parser {
            tokens,
            pos: 0,
            depth: 0,
        }
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

    /// <https://www.w3.org/TR/css-syntax-3/#consume-list-of-rules>
    pub fn consume_rules(&mut self, top_level: bool) -> Vec<Rule> {
        let mut rules = Vec::new();
        loop {
            match self.peek() {
                Token::Whitespace => {
                    self.advance();
                }
                Token::Eof => break,
                Token::Cdo | Token::Cdc => {
                    if top_level {
                        self.advance();
                    } else if let Some(r) = self.consume_qualified_rule() {
                        rules.push(Rule::Qualified(r));
                    }
                }
                Token::AtKeyword(_) => {
                    rules.push(Rule::At(self.consume_at_rule()));
                }
                _ => {
                    if let Some(r) = self.consume_qualified_rule() {
                        rules.push(Rule::Qualified(r));
                    }
                }
            }
        }
        rules
    }

    /// <https://www.w3.org/TR/css-syntax-3/#consume-at-rule>
    fn consume_at_rule(&mut self) -> AtRule {
        let name = match self.advance() {
            Token::AtKeyword(n) => n,
            _ => String::new(),
        };
        let mut prelude = Vec::new();
        loop {
            match self.peek() {
                Token::Semicolon | Token::Eof => {
                    self.advance();
                    return AtRule {
                        name,
                        prelude,
                        block: None,
                    };
                }
                Token::LeftCurly => {
                    let block = self.consume_simple_block(BlockKind::Curly);
                    return AtRule {
                        name,
                        prelude,
                        block: Some(block),
                    };
                }
                _ => prelude.push(self.consume_component_value()),
            }
        }
    }

    /// <https://www.w3.org/TR/css-syntax-3/#consume-qualified-rule>
    fn consume_qualified_rule(&mut self) -> Option<QualifiedRule> {
        let mut prelude = Vec::new();
        loop {
            match self.peek() {
                Token::Eof => return None, // parse error: EOF before a block
                Token::LeftCurly => {
                    let block = self.consume_simple_block(BlockKind::Curly);
                    return Some(QualifiedRule { prelude, block });
                }
                _ => prelude.push(self.consume_component_value()),
            }
        }
    }

    /// <https://www.w3.org/TR/css-syntax-3/#consume-component-value>
    fn consume_component_value(&mut self) -> ComponentValue {
        match self.peek().clone() {
            Token::LeftCurly => ComponentValue::Block {
                open: BlockKind::Curly,
                contents: self.consume_simple_block(BlockKind::Curly),
            },
            Token::LeftSquare => ComponentValue::Block {
                open: BlockKind::Square,
                contents: self.consume_simple_block(BlockKind::Square),
            },
            Token::LeftParen => ComponentValue::Block {
                open: BlockKind::Paren,
                contents: self.consume_simple_block(BlockKind::Paren),
            },
            Token::Function(name) => {
                self.advance();
                ComponentValue::Function {
                    name,
                    args: self.consume_function_args(),
                }
            }
            other => {
                self.advance();
                ComponentValue::Token(other)
            }
        }
    }

    fn consume_simple_block(&mut self, kind: BlockKind) -> Vec<ComponentValue> {
        self.advance(); // the opening bracket
        let closing = match kind {
            BlockKind::Curly => Token::RightCurly,
            BlockKind::Square => Token::RightSquare,
            BlockKind::Paren => Token::RightParen,
        };
        if self.depth >= MAX_NESTING_DEPTH {
            return self.skip_balanced_flat(&closing);
        }
        self.depth += 1;
        let mut contents = Vec::new();
        loop {
            match self.peek() {
                t if *t == closing => {
                    self.advance();
                    break;
                }
                Token::Eof => break,
                _ => contents.push(self.consume_component_value()),
            }
        }
        self.depth -= 1;
        contents
    }

    fn consume_function_args(&mut self) -> Vec<ComponentValue> {
        if self.depth >= MAX_NESTING_DEPTH {
            return self.skip_balanced_flat(&Token::RightParen);
        }
        self.depth += 1;
        let mut contents = Vec::new();
        loop {
            match self.peek() {
                Token::RightParen => {
                    self.advance();
                    break;
                }
                Token::Eof => break,
                _ => contents.push(self.consume_component_value()),
            }
        }
        self.depth -= 1;
        contents
    }

    /// Non-recursive fallback once [`MAX_NESTING_DEPTH`] is hit: consumes
    /// tokens as flat, unstructured `ComponentValue::Token`s (no nested
    /// `Block`/`Function` values) until `closing` is seen with no
    /// still-open bracket/function between it and here, or EOF. `nested`
    /// counts *any* opening token seen (regardless of kind) against *any*
    /// closing token, rather than exactly pairing bracket kinds -- exact
    /// pairing would itself need unbounded state to replicate what
    /// recursion normally tracks on the call stack. That's a fidelity
    /// trade only observable on already-pathological, deliberately-
    /// adversarial input (this path is otherwise unreachable): it never
    /// panics or infinite-loops, which is the actual property this guard
    /// exists for.
    fn skip_balanced_flat(&mut self, closing: &Token) -> Vec<ComponentValue> {
        let mut contents = Vec::new();
        let mut nested = 0u32;
        loop {
            match self.peek().clone() {
                Token::Eof => break,
                t if nested == 0 && t == *closing => {
                    self.advance();
                    break;
                }
                t @ (Token::LeftCurly
                | Token::LeftSquare
                | Token::LeftParen
                | Token::Function(_)) => {
                    nested += 1;
                    self.advance();
                    contents.push(ComponentValue::Token(t));
                }
                t @ (Token::RightCurly | Token::RightSquare | Token::RightParen) => {
                    nested = nested.saturating_sub(1);
                    self.advance();
                    contents.push(ComponentValue::Token(t));
                }
                other => {
                    self.advance();
                    contents.push(ComponentValue::Token(other));
                }
            }
        }
        contents
    }

    /// <https://www.w3.org/TR/css-syntax-3/#consume-declaration>, applied
    /// to every semicolon-separated chunk in a qualified rule's block. This
    /// is the "declaration list" parse used for ordinary style-rule bodies.
    pub fn parse_declaration_list(block: &[ComponentValue]) -> Vec<crate::Declaration> {
        let mut declarations = Vec::new();
        for chunk in split_on_top_level_semicolons(block) {
            if let Some(decl) = parse_one_declaration(&chunk) {
                declarations.push(decl);
            }
        }
        declarations
    }

    /// Splits a style rule's block into its plain declarations and any
    /// rules nested directly inside it (CSS Nesting Module), in source
    /// order. Unlike [`Self::parse_declaration_list`] (which assumes every
    /// semicolon-separated chunk is `prop: value`), nesting interleaves
    /// declarations with nested qualified rules (`& .child { ... }`) and
    /// nested at-rules (`@media (...) { ... }`) in the same block --
    /// <https://www.w3.org/TR/css-nesting-1/#syntax>. A run of component
    /// values is a nested rule if a `{`-block immediately follows it
    /// (the run becomes that rule's prelude/selector); otherwise, once a
    /// `;` or the block's end is reached, it's parsed as an ordinary
    /// declaration. `css::collect_rules` resolves each nested qualified
    /// rule's selector against its parent (substituting `&`) and recurses.
    ///
    /// Takes `block` *by value* (not `&[ComponentValue]`) deliberately: a
    /// nested block's own `contents` need to become the child rule's owned
    /// `block` field, and since `collect_rules` calls this once per
    /// nesting level, cloning that (potentially-still-deeply-nested)
    /// remainder at every level -- rather than moving it -- is quadratic
    /// in nesting depth. Stress-testing with `.a{.a{.a{...` nested tens of
    /// thousands deep (well past where the parser's own depth guard caps
    /// *its* recursion, but not past where this function used to reprocess
    /// the same leftover tokens over and over) took over 7 seconds at only
    /// 20,000 levels before this was fixed to move instead of clone.
    pub fn parse_style_block(block: Vec<ComponentValue>) -> (Vec<crate::Declaration>, Vec<Rule>) {
        let mut declarations = Vec::new();
        let mut nested = Vec::new();
        let mut iter = block.into_iter();
        while let Some(first) = iter.next() {
            match first {
                ComponentValue::Token(Token::Whitespace)
                | ComponentValue::Token(Token::Semicolon) => {}
                ComponentValue::Token(Token::AtKeyword(name)) => {
                    let mut prelude = Vec::new();
                    let mut at_block = None;
                    for item in iter.by_ref() {
                        match item {
                            ComponentValue::Token(Token::Semicolon) => break,
                            ComponentValue::Block {
                                open: BlockKind::Curly,
                                contents,
                            } => {
                                at_block = Some(contents);
                                break;
                            }
                            other => prelude.push(other),
                        }
                    }
                    nested.push(Rule::At(AtRule {
                        name,
                        prelude,
                        block: at_block,
                    }));
                }
                first_of_run => {
                    let mut run = Vec::new();
                    let mut found_block = None;
                    for item in std::iter::once(first_of_run).chain(iter.by_ref()) {
                        match item {
                            ComponentValue::Token(Token::Semicolon) => break,
                            ComponentValue::Block {
                                open: BlockKind::Curly,
                                contents,
                            } => {
                                found_block = Some(contents);
                                break;
                            }
                            other => run.push(other),
                        }
                    }
                    if let Some(contents) = found_block {
                        nested.push(Rule::Qualified(QualifiedRule {
                            prelude: run,
                            block: contents,
                        }));
                    } else if let Some(decl) = parse_one_declaration(&run) {
                        declarations.push(decl);
                    }
                }
            }
        }
        (declarations, nested)
    }
}

/// Splits a selector prelude on top-level commas -- safe to do directly on
/// already-parsed `ComponentValue`s (unlike raw tokens) because anything
/// inside parentheses/brackets is already grouped into its own nested
/// `ComponentValue::Block`, so a `Comma` appearing in this flat slice is
/// always a real selector-list separator, never one buried inside e.g.
/// `:is(a, b)`.
pub fn split_top_level_commas(values: &[ComponentValue]) -> Vec<Vec<ComponentValue>> {
    let mut parts = Vec::new();
    let mut current = Vec::new();
    for v in values {
        if matches!(v, ComponentValue::Token(Token::Comma)) {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(v.clone());
        }
    }
    parts.push(current);
    parts
}

fn split_on_top_level_semicolons(values: &[ComponentValue]) -> Vec<Vec<ComponentValue>> {
    let mut chunks = Vec::new();
    let mut current = Vec::new();
    for v in values {
        if matches!(v, ComponentValue::Token(Token::Semicolon)) {
            chunks.push(std::mem::take(&mut current));
        } else {
            current.push(v.clone());
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn parse_one_declaration(chunk: &[ComponentValue]) -> Option<crate::Declaration> {
    let mut i = 0;
    while matches!(chunk.get(i), Some(ComponentValue::Token(Token::Whitespace))) {
        i += 1;
    }
    let name = match chunk.get(i) {
        Some(ComponentValue::Token(Token::Ident(n))) => n.clone(),
        _ => return None, // parse error: declarations must start with an ident
    };
    i += 1;
    while matches!(chunk.get(i), Some(ComponentValue::Token(Token::Whitespace))) {
        i += 1;
    }
    if !matches!(chunk.get(i), Some(ComponentValue::Token(Token::Colon))) {
        return None; // parse error: no colon
    }
    i += 1;

    let mut value_tokens = &chunk[i..];
    while matches!(
        value_tokens.first(),
        Some(ComponentValue::Token(Token::Whitespace))
    ) {
        value_tokens = &value_tokens[1..];
    }
    let mut value_tokens = value_tokens.to_vec();
    while matches!(
        value_tokens.last(),
        Some(ComponentValue::Token(Token::Whitespace))
    ) {
        value_tokens.pop();
    }

    // Check for a trailing "!important", per the spec's declaration value
    // post-processing step.
    let mut important = false;
    if value_tokens.len() >= 2 {
        let last = &value_tokens[value_tokens.len() - 1];
        let mut j = value_tokens.len() - 1;
        if matches!(last, ComponentValue::Token(Token::Ident(v)) if v.eq_ignore_ascii_case("important"))
        {
            let mut k = j;
            while k > 0
                && matches!(
                    value_tokens[k - 1],
                    ComponentValue::Token(Token::Whitespace)
                )
            {
                k -= 1;
            }
            if k > 0
                && matches!(
                    &value_tokens[k - 1],
                    ComponentValue::Token(Token::Delim('!'))
                )
            {
                j = k - 1;
                important = true;
            }
        }
        if important {
            value_tokens.truncate(j);
            while matches!(
                value_tokens.last(),
                Some(ComponentValue::Token(Token::Whitespace))
            ) {
                value_tokens.pop();
            }
        }
    }

    let value = serialize(&value_tokens);
    Some(crate::Declaration {
        property: name,
        value,
        important,
    })
}

/// Re-serializes component values back to CSS text -- used both for
/// declaration values (kept as plain strings in the public `Declaration`
/// API) and for a qualified rule's prelude (the selector text A5 parses).
pub fn serialize(values: &[ComponentValue]) -> String {
    let mut out = String::new();
    for v in values {
        serialize_one(v, &mut out);
    }
    out.trim().to_string()
}

fn serialize_one(v: &ComponentValue, out: &mut String) {
    match v {
        ComponentValue::Token(t) => serialize_token(t, out),
        ComponentValue::Function { name, args } => {
            out.push_str(name);
            out.push('(');
            for a in args {
                serialize_one(a, out);
            }
            out.push(')');
        }
        ComponentValue::Block { open, contents } => {
            let (o, c) = match open {
                BlockKind::Curly => ('{', '}'),
                BlockKind::Square => ('[', ']'),
                BlockKind::Paren => ('(', ')'),
            };
            out.push(o);
            for cv in contents {
                serialize_one(cv, out);
            }
            out.push(c);
        }
    }
}

fn serialize_token(t: &Token, out: &mut String) {
    match t {
        Token::Ident(s) | Token::Str(s) => out.push_str(s),
        Token::Function(s) => {
            out.push_str(s);
            out.push('(');
        }
        Token::AtKeyword(s) => {
            out.push('@');
            out.push_str(s);
        }
        Token::Hash { value, .. } => {
            out.push('#');
            out.push_str(value);
        }
        Token::Url(s) => {
            out.push_str("url(");
            out.push_str(s);
            out.push(')');
        }
        Token::Delim(c) => out.push(*c),
        Token::Number { repr, .. } => out.push_str(repr),
        Token::Percentage { repr, .. } => {
            out.push_str(repr);
            out.push('%');
        }
        Token::Dimension { repr, unit, .. } => {
            out.push_str(repr);
            out.push_str(unit);
        }
        Token::Whitespace => out.push(' '),
        Token::Colon => out.push(':'),
        Token::Semicolon => out.push(';'),
        Token::Comma => out.push(','),
        Token::LeftSquare => out.push('['),
        Token::RightSquare => out.push(']'),
        Token::LeftParen => out.push('('),
        Token::RightParen => out.push(')'),
        Token::LeftCurly => out.push('{'),
        Token::RightCurly => out.push('}'),
        Token::Cdo => out.push_str("<!--"),
        Token::Cdc => out.push_str("-->"),
        Token::BadString | Token::BadUrl | Token::Eof => {}
    }
}

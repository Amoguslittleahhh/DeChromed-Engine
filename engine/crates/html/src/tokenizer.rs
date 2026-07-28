//! A2: the WHATWG HTML tokenizer state machine.
//!
//! Reference: <https://html.spec.whatwg.org/multipage/parsing.html#tokenization>
//!
//! `TokenizerState` lists every state from the spec. Implemented: Data, tag
//! open/close, all attribute-value quoting forms, self-closing tags,
//! comments (including the bogus-comment and markup-declaration-open bad
//! paths), DOCTYPE, RCDATA/RAWTEXT/PLAINTEXT (including their end-tag
//! matching against the last start tag emitted), CDATA sections, and both
//! numeric and named character references (including the legacy
//! without-semicolon named-reference forms and the Windows-1252 control
//! code remapping table for numeric references).
//!
//! Not implemented yet: the ScriptData escaped/double-escaped states
//! (`<script>`'s `<!--` / nested-script escaping mechanism) -- `ScriptData`
//! itself works, but `escapeFlag.test` specifically exercises the escaped
//! variants and is expected to fail until that's filled in.
//!
//! `tokenize()`/`tokenize_from()`/`tokenize_with()` run the tokenizer to
//! completion in one call, matching html5lib-tests' tokenizer-only test
//! format (which specifies a fixed `initialStates` up front). A3's tree
//! builder needs something more dynamic than that: the real spec has tree
//! construction tell the tokenizer *while parsing* to switch into RCDATA/
//! RAWTEXT/ScriptData/PLAINTEXT the moment it sees a `<title>`/`<style>`/
//! `<script>`/`<plaintext>` start tag, and switch back to Data afterward --
//! [`Tokenizer`] (the struct, made public for this) plus [`Tokenizer::next_token`]
//! and [`Tokenizer::set_state`] expose exactly that: pull one token at a
//! time, and change state in between pulls.

use crate::entities;
use crate::Token;
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Not all states are reachable until ScriptData escaping is implemented.
pub enum TokenizerState {
    Data,
    RcData,
    RawText,
    ScriptData,
    PlainText,
    TagOpen,
    EndTagOpen,
    TagName,
    RcDataLessThanSign,
    RcDataEndTagOpen,
    RcDataEndTagName,
    RawTextLessThanSign,
    RawTextEndTagOpen,
    RawTextEndTagName,
    ScriptDataLessThanSign,
    ScriptDataEndTagOpen,
    ScriptDataEndTagName,
    ScriptDataEscapeStart,
    ScriptDataEscapeStartDash,
    ScriptDataEscaped,
    ScriptDataEscapedDash,
    ScriptDataEscapedDashDash,
    ScriptDataEscapedLessThanSign,
    ScriptDataEscapedEndTagOpen,
    ScriptDataEscapedEndTagName,
    ScriptDataDoubleEscapeStart,
    ScriptDataDoubleEscaped,
    ScriptDataDoubleEscapedDash,
    ScriptDataDoubleEscapedDashDash,
    ScriptDataDoubleEscapedLessThanSign,
    ScriptDataDoubleEscapeEnd,
    BeforeAttributeName,
    AttributeName,
    AfterAttributeName,
    BeforeAttributeValue,
    AttributeValueDoubleQuoted,
    AttributeValueSingleQuoted,
    AttributeValueUnquoted,
    AfterAttributeValueQuoted,
    SelfClosingStartTag,
    BogusComment,
    MarkupDeclarationOpen,
    CommentStart,
    CommentStartDash,
    Comment,
    CommentLessThanSign,
    CommentLessThanSignBang,
    CommentLessThanSignBangDash,
    CommentLessThanSignBangDashDash,
    CommentEndDash,
    CommentEnd,
    CommentEndBang,
    Doctype,
    BeforeDoctypeName,
    DoctypeName,
    AfterDoctypeName,
    AfterDoctypePublicKeyword,
    BeforeDoctypePublicIdentifier,
    DoctypePublicIdentifierDoubleQuoted,
    DoctypePublicIdentifierSingleQuoted,
    AfterDoctypePublicIdentifier,
    BetweenDoctypePublicAndSystemIdentifiers,
    AfterDoctypeSystemKeyword,
    BeforeDoctypeSystemIdentifier,
    DoctypeSystemIdentifierDoubleQuoted,
    DoctypeSystemIdentifierSingleQuoted,
    AfterDoctypeSystemIdentifier,
    BogusDoctype,
    CdataSection,
    CdataSectionBracket,
    CdataSectionEnd,
    CharacterReference,
    NamedCharacterReference,
    AmbiguousAmpersand,
    NumericCharacterReference,
    HexadecimalCharacterReferenceStart,
    DecimalCharacterReferenceStart,
    HexadecimalCharacterReference,
    DecimalCharacterReference,
    NumericCharacterReferenceEnd,
}

/// Tokenize `input`, starting in [`TokenizerState::Data`].
pub fn tokenize(input: &str) -> Vec<Token> {
    tokenize_from(input, TokenizerState::Data)
}

/// Tokenize `input`, starting from an explicit state -- for the handful of
/// RCDATA/RAWTEXT/PLAINTEXT/CDATA test cases that specify `initialStates`
/// (see module docs for why this is needed instead of the tokenizer
/// figuring it out itself).
pub fn tokenize_from(input: &str, start_state: TokenizerState) -> Vec<Token> {
    tokenize_with(input, start_state, None)
}

/// Like [`tokenize_from`], but also primes the "last start tag emitted"
/// used by RCDATA/RAWTEXT/ScriptData's "appropriate end tag token" check --
/// needed because without a tree builder driving the tokenizer, there's no
/// earlier start tag in this same call to have set it naturally. Matches
/// html5lib-tests' `lastStartTag` field.
pub fn tokenize_with(
    input: &str,
    start_state: TokenizerState,
    last_start_tag: Option<&str>,
) -> Vec<Token> {
    let mut tok = Tokenizer::new(input, start_state);
    tok.last_start_tag_name = last_start_tag.map(str::to_string);
    tok.run();
    Vec::from(tok.tokens)
}

/// Codepoints 0x80-0x9F map to these characters instead of themselves when
/// they appear in a numeric character reference, per the spec's table in
/// the `NumericCharacterReferenceEnd` state (a legacy accommodation for
/// documents that used Windows-1252 byte values as if they were Unicode
/// scalar values).
const C1_REPLACEMENTS: [char; 32] = [
    '\u{20AC}', '\u{0081}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008D}', '\u{017D}', '\u{008F}',
    '\u{0090}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{009D}', '\u{017E}', '\u{0178}',
];

pub struct Tokenizer {
    chars: Vec<char>,
    pos: usize,
    state: TokenizerState,
    return_state: TokenizerState,
    tokens: VecDeque<Token>,

    // Current tag being built (start or end).
    tag_name: String,
    tag_is_end: bool,
    tag_self_closing: bool,
    attrs: Vec<(String, String)>,
    attr_name: String,
    attr_value: String,

    // Current comment/doctype being built.
    comment: String,
    doctype_name: Option<String>,
    doctype_public_id: Option<String>,
    doctype_system_id: Option<String>,
    doctype_force_quirks: bool,

    // Character-reference scratch space.
    temp_buffer: String,
    char_ref_code: u32,

    last_start_tag_name: Option<String>,
}

/// Spec preprocessing step, applied before tokenization even starts:
/// every CRLF pair, and every remaining lone CR, is normalized to a single
/// LF. See <https://html.spec.whatwg.org/multipage/parsing.html#preprocessing-the-input-stream>.
fn normalize_newlines(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

impl Tokenizer {
    pub fn new(input: &str, start_state: TokenizerState) -> Self {
        let normalized = normalize_newlines(input);
        Tokenizer {
            chars: normalized.chars().collect(),
            pos: 0,
            state: start_state,
            return_state: TokenizerState::Data,
            tokens: VecDeque::new(),
            tag_name: String::new(),
            tag_is_end: false,
            tag_self_closing: false,
            attrs: Vec::new(),
            attr_name: String::new(),
            attr_value: String::new(),
            comment: String::new(),
            doctype_name: None,
            doctype_public_id: None,
            doctype_system_id: None,
            doctype_force_quirks: false,
            temp_buffer: String::new(),
            char_ref_code: 0,
            last_start_tag_name: None,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn rest(&self) -> String {
        self.chars[self.pos..].iter().collect()
    }

    fn starts_with_ignore_case(&self, s: &str) -> bool {
        let n = s.chars().count();
        if self.pos + n > self.chars.len() {
            return false;
        }
        self.chars[self.pos..self.pos + n]
            .iter()
            .zip(s.chars())
            .all(|(a, b)| a.eq_ignore_ascii_case(&b))
    }

    fn consume_n(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.chars.len());
    }

    fn emit_char(&mut self, c: char) {
        self.tokens.push_back(Token::Character(c));
    }

    fn emit_str(&mut self, s: &str) {
        for c in s.chars() {
            self.emit_char(c);
        }
    }

    fn start_new_tag(&mut self, is_end: bool) {
        self.tag_name.clear();
        self.tag_is_end = is_end;
        self.tag_self_closing = false;
        self.attrs.clear();
        // Also reset the in-progress attribute scratch buffers -- without
        // this, a tag with no attributes of its own inherits and re-emits
        // the previous tag's last in-progress attribute name/value.
        self.attr_name.clear();
        self.attr_value.clear();
    }

    fn start_new_attribute(&mut self) {
        self.finish_attribute();
        self.attr_name.clear();
        self.attr_value.clear();
    }

    fn finish_attribute(&mut self) {
        if !self.attr_name.is_empty() && !self.attrs.iter().any(|(k, _)| k == &self.attr_name) {
            self.attrs
                .push((self.attr_name.clone(), self.attr_value.clone()));
        }
    }

    fn emit_tag(&mut self) {
        self.finish_attribute();
        if self.tag_is_end {
            self.tokens.push_back(Token::EndTag {
                name: self.tag_name.clone(),
            });
        } else {
            self.last_start_tag_name = Some(self.tag_name.clone());
            self.tokens.push_back(Token::StartTag {
                name: self.tag_name.clone(),
                attributes: self.attrs.clone(),
                self_closing: self.tag_self_closing,
            });
        }
    }

    fn emit_comment(&mut self) {
        self.tokens.push_back(Token::Comment(self.comment.clone()));
    }

    fn emit_doctype(&mut self) {
        self.tokens.push_back(Token::Doctype {
            name: self.doctype_name.clone(),
            public_id: self.doctype_public_id.clone(),
            system_id: self.doctype_system_id.clone(),
            force_quirks: self.doctype_force_quirks,
        });
    }

    fn is_appropriate_end_tag(&self) -> bool {
        matches!(&self.last_start_tag_name, Some(n) if n == &self.tag_name)
    }

    /// Rewinds by one character so it's re-read under `state`. Must only be
    /// called right after `advance()` returned `Some` -- calling it after a
    /// `None` (nothing consumed) would rewind past the end and re-read the
    /// last real character forever. `saturating_sub` is a safety net, not a
    /// substitute for calling this correctly.
    fn reconsume(&mut self, state: TokenizerState) {
        self.pos = self.pos.saturating_sub(1);
        self.state = state;
    }

    fn run(&mut self) {
        // Defensive cap: no correct state ever needs more than a small
        // constant number of steps per input character (character
        // references are the worst case, at a handful of steps per char).
        // If this trips, it means a state transition failed to consume
        // input or terminate -- fail loudly in CI instead of hanging.
        let max_steps = self.chars.len().saturating_mul(64) + 1000;
        let mut steps = 0usize;
        loop {
            steps += 1;
            assert!(
                steps <= max_steps,
                "tokenizer exceeded {max_steps} steps (likely an infinite loop) in state {:?} at pos {}",
                self.state,
                self.pos
            );
            if self.step() {
                break;
            }
        }
    }

    /// Pulls the next token, running the state machine just far enough to
    /// produce one. Used by the tree builder (A3), which needs to inspect
    /// each token (and possibly call [`Tokenizer::set_state`] in response)
    /// before the next one is produced -- unlike `run()`, which tokenizes
    /// an entire input up front assuming a fixed state throughout.
    pub fn next_token(&mut self) -> Token {
        if let Some(t) = self.tokens.pop_front() {
            return t;
        }
        let max_steps = 200_000usize;
        let mut steps = 0usize;
        loop {
            steps += 1;
            assert!(
                steps <= max_steps,
                "tokenizer exceeded {max_steps} steps producing one token (likely an infinite loop) in state {:?} at pos {}",
                self.state,
                self.pos
            );
            let done = self.step();
            if let Some(t) = self.tokens.pop_front() {
                return t;
            }
            if done {
                return Token::Eof;
            }
        }
    }

    /// Switches the tokenizer's state -- how the tree builder (A3) tells it
    /// "the content of the element you're about to see is RCDATA/RAWTEXT/
    /// ScriptData/PLAINTEXT, not ordinary markup," per the tree
    /// construction algorithm's various "switch the tokenizer to the ...
    /// state" steps.
    pub fn set_state(&mut self, state: TokenizerState) {
        self.state = state;
    }

    /// Sets the "last start tag emitted," used by RCDATA/RAWTEXT/ScriptData's
    /// "appropriate end tag token" check. The tree builder calls this
    /// whenever it processes a start tag, so the tokenizer's later matching
    /// end-tag check reflects real parsing rather than a value primed once
    /// up front (which is all `tokenize_with` can do without a tree builder
    /// driving it).
    pub fn set_last_start_tag(&mut self, name: Option<String>) {
        self.last_start_tag_name = name;
    }

    /// Runs one state's worth of work. Returns `true` when tokenization is
    /// complete (EOF has been emitted).
    fn step(&mut self) -> bool {
        use TokenizerState::*;
        match self.state {
            Data => {
                match self.advance() {
                    None => {
                        self.tokens.push_back(Token::Eof);
                        return true;
                    }
                    Some('&') => {
                        self.return_state = Data;
                        self.state = CharacterReference;
                    }
                    Some('<') => self.state = TagOpen,
                    // Unlike RCDATA/RAWTEXT/ScriptData below, the spec's
                    // Data state does NOT replace NUL with U+FFFD -- it
                    // just emits the NUL character as-is (with a parse
                    // error). Only the content-text states replace it.
                    Some(c) => self.emit_char(c),
                }
            }
            RcData => match self.advance() {
                None => {
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some('&') => {
                    self.return_state = RcData;
                    self.state = CharacterReference;
                }
                Some('<') => self.state = RcDataLessThanSign,
                Some('\0') => self.emit_char('\u{FFFD}'),
                Some(c) => self.emit_char(c),
            },
            RawText => match self.advance() {
                None => {
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some('<') => self.state = RawTextLessThanSign,
                Some('\0') => self.emit_char('\u{FFFD}'),
                Some(c) => self.emit_char(c),
            },
            ScriptData => match self.advance() {
                None => {
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some('<') => self.state = ScriptDataLessThanSign,
                Some('\0') => self.emit_char('\u{FFFD}'),
                Some(c) => self.emit_char(c),
            },
            PlainText => match self.advance() {
                None => {
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some('\0') => self.emit_char('\u{FFFD}'),
                Some(c) => self.emit_char(c),
            },

            TagOpen => match self.peek() {
                Some('!') => {
                    self.advance();
                    self.state = MarkupDeclarationOpen;
                }
                Some('/') => {
                    self.advance();
                    self.state = EndTagOpen;
                }
                Some(c) if c.is_ascii_alphabetic() => {
                    self.start_new_tag(false);
                    self.state = TagName;
                }
                Some('?') => {
                    self.comment.clear();
                    self.state = BogusComment;
                }
                None => {
                    self.emit_char('<');
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some(_) => {
                    self.emit_char('<');
                    self.state = Data;
                }
            },
            EndTagOpen => match self.peek() {
                Some(c) if c.is_ascii_alphabetic() => {
                    self.start_new_tag(true);
                    self.state = TagName;
                }
                Some('>') => {
                    self.advance();
                    self.state = Data;
                }
                None => {
                    self.emit_str("</");
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some(_) => {
                    self.comment.clear();
                    self.state = BogusComment;
                }
            },
            TagName => match self.advance() {
                Some(c) if c.is_ascii_whitespace() => self.state = BeforeAttributeName,
                Some('/') => self.state = SelfClosingStartTag,
                Some('>') => {
                    self.emit_tag();
                    self.state = Data;
                }
                Some(c) if c.is_ascii_uppercase() => {
                    self.tag_name.push(c.to_ascii_lowercase());
                }
                Some('\0') => self.tag_name.push('\u{FFFD}'),
                Some(c) => self.tag_name.push(c),
                None => {
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
            },

            RcDataLessThanSign => match self.peek() {
                Some('/') => {
                    self.advance();
                    self.temp_buffer.clear();
                    self.state = RcDataEndTagOpen;
                }
                _ => {
                    self.emit_char('<');
                    self.state = RcData;
                }
            },
            RcDataEndTagOpen => match self.peek() {
                Some(c) if c.is_ascii_alphabetic() => {
                    self.start_new_tag(true);
                    self.state = RcDataEndTagName;
                }
                _ => {
                    self.emit_str("</");
                    self.state = RcData;
                }
            },
            RcDataEndTagName => self.tag_end_name_step(RcData),

            RawTextLessThanSign => match self.peek() {
                Some('/') => {
                    self.advance();
                    self.temp_buffer.clear();
                    self.state = RawTextEndTagOpen;
                }
                _ => {
                    self.emit_char('<');
                    self.state = RawText;
                }
            },
            RawTextEndTagOpen => match self.peek() {
                Some(c) if c.is_ascii_alphabetic() => {
                    self.start_new_tag(true);
                    self.state = RawTextEndTagName;
                }
                _ => {
                    self.emit_str("</");
                    self.state = RawText;
                }
            },
            RawTextEndTagName => self.tag_end_name_step(RawText),

            ScriptDataLessThanSign => match self.peek() {
                Some('/') => {
                    self.advance();
                    self.temp_buffer.clear();
                    self.state = ScriptDataEndTagOpen;
                }
                _ => {
                    self.emit_char('<');
                    self.state = ScriptData;
                }
            },
            ScriptDataEndTagOpen => match self.peek() {
                Some(c) if c.is_ascii_alphabetic() => {
                    self.start_new_tag(true);
                    self.state = ScriptDataEndTagName;
                }
                _ => {
                    self.emit_str("</");
                    self.state = ScriptData;
                }
            },
            ScriptDataEndTagName => self.tag_end_name_step(ScriptData),

            // Escaped/double-escaped ScriptData states: not implemented yet
            // (see module docs). Treat as plain ScriptData so we don't get
            // stuck in an unreachable state.
            ScriptDataEscapeStart
            | ScriptDataEscapeStartDash
            | ScriptDataEscaped
            | ScriptDataEscapedDash
            | ScriptDataEscapedDashDash
            | ScriptDataEscapedLessThanSign
            | ScriptDataEscapedEndTagOpen
            | ScriptDataEscapedEndTagName
            | ScriptDataDoubleEscapeStart
            | ScriptDataDoubleEscaped
            | ScriptDataDoubleEscapedDash
            | ScriptDataDoubleEscapedDashDash
            | ScriptDataDoubleEscapedLessThanSign
            | ScriptDataDoubleEscapeEnd => {
                self.state = ScriptData;
            }

            BeforeAttributeName => match self.peek() {
                Some(c) if c.is_ascii_whitespace() => {
                    self.advance();
                }
                Some('/') | Some('>') | None => {
                    self.start_new_attribute();
                    self.state = AfterAttributeName;
                }
                Some('=') => {
                    self.advance();
                    self.start_new_attribute();
                    self.attr_name.push('=');
                    self.state = AttributeName;
                }
                Some(_) => {
                    self.start_new_attribute();
                    self.state = AttributeName;
                }
            },
            AttributeName => match self.advance() {
                Some(c) if c.is_ascii_whitespace() || c == '/' || c == '>' => {
                    self.reconsume(AfterAttributeName);
                }
                Some('=') => self.state = BeforeAttributeValue,
                Some(c) if c.is_ascii_uppercase() => {
                    self.attr_name.push(c.to_ascii_lowercase());
                }
                Some('\0') => self.attr_name.push('\u{FFFD}'),
                Some(c) => self.attr_name.push(c),
                // Nothing was actually consumed here (advance() returned
                // None), so this must NOT call reconsume() -- doing so
                // would rewind `pos` past the end and re-read the last
                // real character forever (an infinite loop, not just a
                // wrong token).
                None => self.state = AfterAttributeName,
            },
            AfterAttributeName => match self.peek() {
                Some(c) if c.is_ascii_whitespace() => {
                    self.advance();
                }
                Some('/') => {
                    self.advance();
                    self.state = SelfClosingStartTag;
                }
                Some('=') => {
                    self.advance();
                    self.state = BeforeAttributeValue;
                }
                Some('>') => {
                    self.advance();
                    self.emit_tag();
                    self.state = Data;
                }
                None => {
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some(_) => {
                    self.start_new_attribute();
                    self.state = AttributeName;
                }
            },
            BeforeAttributeValue => match self.peek() {
                Some(c) if c.is_ascii_whitespace() => {
                    self.advance();
                }
                Some('"') => {
                    self.advance();
                    self.state = AttributeValueDoubleQuoted;
                }
                Some('\'') => {
                    self.advance();
                    self.state = AttributeValueSingleQuoted;
                }
                Some('>') => {
                    self.advance();
                    self.emit_tag();
                    self.state = Data;
                }
                _ => self.state = AttributeValueUnquoted,
            },
            AttributeValueDoubleQuoted => match self.advance() {
                Some('"') => self.state = AfterAttributeValueQuoted,
                Some('&') => {
                    self.return_state = AttributeValueDoubleQuoted;
                    self.state = CharacterReference;
                }
                Some('\0') => self.attr_value.push('\u{FFFD}'),
                Some(c) => self.attr_value.push(c),
                None => {
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
            },
            AttributeValueSingleQuoted => match self.advance() {
                Some('\'') => self.state = AfterAttributeValueQuoted,
                Some('&') => {
                    self.return_state = AttributeValueSingleQuoted;
                    self.state = CharacterReference;
                }
                Some('\0') => self.attr_value.push('\u{FFFD}'),
                Some(c) => self.attr_value.push(c),
                None => {
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
            },
            AttributeValueUnquoted => match self.advance() {
                Some(c) if c.is_ascii_whitespace() => self.state = BeforeAttributeName,
                Some('&') => {
                    self.return_state = AttributeValueUnquoted;
                    self.state = CharacterReference;
                }
                Some('>') => {
                    self.emit_tag();
                    self.state = Data;
                }
                Some('\0') => self.attr_value.push('\u{FFFD}'),
                Some(c) => self.attr_value.push(c),
                None => {
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
            },
            AfterAttributeValueQuoted => match self.peek() {
                Some(c) if c.is_ascii_whitespace() => {
                    self.advance();
                    self.state = BeforeAttributeName;
                }
                Some('/') => {
                    self.advance();
                    self.state = SelfClosingStartTag;
                }
                Some('>') => {
                    self.advance();
                    self.emit_tag();
                    self.state = Data;
                }
                None => {
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some(_) => self.state = BeforeAttributeName,
            },
            SelfClosingStartTag => match self.peek() {
                Some('>') => {
                    self.advance();
                    self.tag_self_closing = true;
                    self.emit_tag();
                    self.state = Data;
                }
                None => {
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some(_) => self.state = BeforeAttributeName,
            },

            BogusComment => match self.advance() {
                Some('>') => {
                    self.emit_comment();
                    self.state = Data;
                }
                Some('\0') => self.comment.push('\u{FFFD}'),
                Some(c) => self.comment.push(c),
                None => {
                    self.emit_comment();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
            },
            MarkupDeclarationOpen => {
                if self.starts_with_ignore_case("--") {
                    self.consume_n(2);
                    self.comment.clear();
                    self.state = CommentStart;
                } else if self.starts_with_ignore_case("doctype") {
                    self.consume_n(7);
                    self.state = Doctype;
                } else if self.starts_with_ignore_case("[CDATA[") {
                    // Spec: this only enters CDATA section state when the
                    // current node is foreign (SVG/MathML) content: HTML
                    // content treats `<![CDATA[` as a bogus comment whose
                    // text starts with "[CDATA[". We have no tree builder
                    // yet to know "current node", so default to the
                    // ordinary-HTML-content behavior (bogus comment) --
                    // the far more common case, and what a tokenizer-only
                    // test run assumes absent other signal.
                    self.comment.clear();
                    self.state = BogusComment;
                } else {
                    self.comment.clear();
                    self.state = BogusComment;
                }
            }

            CommentStart => match self.peek() {
                Some('-') => {
                    self.advance();
                    self.state = CommentStartDash;
                }
                Some('>') => {
                    self.advance();
                    self.emit_comment();
                    self.state = Data;
                }
                _ => self.state = Comment,
            },
            CommentStartDash => match self.peek() {
                Some('-') => {
                    self.advance();
                    self.state = CommentEnd;
                }
                Some('>') => {
                    self.advance();
                    self.emit_comment();
                    self.state = Data;
                }
                None => {
                    self.emit_comment();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some(_) => {
                    self.comment.push('-');
                    self.state = Comment;
                }
            },
            Comment => match self.advance() {
                Some('<') => {
                    self.comment.push('<');
                    self.state = CommentLessThanSign;
                }
                Some('-') => self.state = CommentEndDash,
                Some('\0') => self.comment.push('\u{FFFD}'),
                Some(c) => self.comment.push(c),
                None => {
                    self.emit_comment();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
            },
            CommentLessThanSign => match self.peek() {
                Some('!') => {
                    self.advance();
                    self.comment.push('!');
                    self.state = CommentLessThanSignBang;
                }
                Some('<') => {
                    self.advance();
                    self.comment.push('<');
                }
                _ => self.state = Comment,
            },
            CommentLessThanSignBang => match self.peek() {
                Some('-') => {
                    self.advance();
                    self.state = CommentLessThanSignBangDash;
                }
                _ => self.state = Comment,
            },
            CommentLessThanSignBangDash => match self.peek() {
                Some('-') => {
                    self.advance();
                    self.state = CommentLessThanSignBangDashDash;
                }
                _ => self.state = CommentEndDash,
            },
            CommentLessThanSignBangDashDash => {
                self.state = CommentEnd;
            }
            CommentEndDash => match self.peek() {
                Some('-') => {
                    self.advance();
                    self.state = CommentEnd;
                }
                None => {
                    self.emit_comment();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some(_) => {
                    self.comment.push('-');
                    self.state = Comment;
                }
            },
            CommentEnd => match self.peek() {
                Some('>') => {
                    self.advance();
                    self.emit_comment();
                    self.state = Data;
                }
                Some('!') => {
                    self.advance();
                    self.state = CommentEndBang;
                }
                Some('-') => {
                    self.advance();
                    self.comment.push('-');
                }
                None => {
                    self.emit_comment();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some(_) => {
                    self.comment.push_str("--");
                    self.state = Comment;
                }
            },
            CommentEndBang => match self.peek() {
                Some('-') => {
                    self.advance();
                    self.comment.push_str("--!");
                    self.state = CommentEndDash;
                }
                Some('>') => {
                    self.advance();
                    self.emit_comment();
                    self.state = Data;
                }
                None => {
                    self.emit_comment();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some(_) => {
                    self.comment.push_str("--!");
                    self.state = Comment;
                }
            },

            Doctype => match self.peek() {
                Some(c) if c.is_ascii_whitespace() => {
                    self.advance();
                    self.state = BeforeDoctypeName;
                }
                Some('>') => self.state = BeforeDoctypeName,
                None => {
                    self.doctype_name = None;
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some(_) => self.state = BeforeDoctypeName,
            },
            BeforeDoctypeName => match self.advance() {
                Some(c) if c.is_ascii_whitespace() => {}
                Some(c) if c.is_ascii_uppercase() => {
                    self.doctype_name = Some(c.to_ascii_lowercase().to_string());
                    self.state = DoctypeName;
                }
                Some('\0') => {
                    self.doctype_name = Some('\u{FFFD}'.to_string());
                    self.state = DoctypeName;
                }
                Some('>') => {
                    self.doctype_name = None;
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.state = Data;
                }
                None => {
                    self.doctype_name = None;
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some(c) => {
                    self.doctype_name = Some(c.to_string());
                    self.state = DoctypeName;
                }
            },
            DoctypeName => match self.advance() {
                Some(c) if c.is_ascii_whitespace() => self.state = AfterDoctypeName,
                Some('>') => {
                    self.emit_doctype();
                    self.state = Data;
                }
                Some(c) if c.is_ascii_uppercase() => {
                    self.doctype_name
                        .get_or_insert_with(String::new)
                        .push(c.to_ascii_lowercase());
                }
                Some('\0') => self
                    .doctype_name
                    .get_or_insert_with(String::new)
                    .push('\u{FFFD}'),
                Some(c) => self.doctype_name.get_or_insert_with(String::new).push(c),
                None => {
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
            },
            AfterDoctypeName => {
                if self.starts_with_ignore_case("public") {
                    self.consume_n(6);
                    self.state = AfterDoctypePublicKeyword;
                } else if self.starts_with_ignore_case("system") {
                    self.consume_n(6);
                    self.state = AfterDoctypeSystemKeyword;
                } else {
                    match self.advance() {
                        Some(c) if c.is_ascii_whitespace() => {}
                        Some('>') => {
                            self.emit_doctype();
                            self.state = Data;
                        }
                        None => {
                            self.doctype_force_quirks = true;
                            self.emit_doctype();
                            self.tokens.push_back(Token::Eof);
                            return true;
                        }
                        Some(_) => {
                            self.doctype_force_quirks = true;
                            self.state = BogusDoctype;
                        }
                    }
                }
            }
            AfterDoctypePublicKeyword => match self.peek() {
                Some(c) if c.is_ascii_whitespace() => {
                    self.advance();
                    self.state = BeforeDoctypePublicIdentifier;
                }
                Some('"') => {
                    self.advance();
                    self.doctype_public_id = Some(String::new());
                    self.state = DoctypePublicIdentifierDoubleQuoted;
                }
                Some('\'') => {
                    self.advance();
                    self.doctype_public_id = Some(String::new());
                    self.state = DoctypePublicIdentifierSingleQuoted;
                }
                Some('>') => {
                    self.advance();
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.state = Data;
                }
                None => {
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some(_) => {
                    self.doctype_force_quirks = true;
                    self.state = BogusDoctype;
                }
            },
            BeforeDoctypePublicIdentifier => match self.peek() {
                Some(c) if c.is_ascii_whitespace() => {
                    self.advance();
                }
                Some('"') => {
                    self.advance();
                    self.doctype_public_id = Some(String::new());
                    self.state = DoctypePublicIdentifierDoubleQuoted;
                }
                Some('\'') => {
                    self.advance();
                    self.doctype_public_id = Some(String::new());
                    self.state = DoctypePublicIdentifierSingleQuoted;
                }
                Some('>') => {
                    self.advance();
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.state = Data;
                }
                None => {
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some(_) => {
                    self.doctype_force_quirks = true;
                    self.state = BogusDoctype;
                }
            },
            DoctypePublicIdentifierDoubleQuoted => match self.advance() {
                Some('"') => self.state = AfterDoctypePublicIdentifier,
                Some('\0') => self
                    .doctype_public_id
                    .get_or_insert_with(String::new)
                    .push('\u{FFFD}'),
                Some('>') => {
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.state = Data;
                }
                Some(c) => self
                    .doctype_public_id
                    .get_or_insert_with(String::new)
                    .push(c),
                None => {
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
            },
            DoctypePublicIdentifierSingleQuoted => match self.advance() {
                Some('\'') => self.state = AfterDoctypePublicIdentifier,
                Some('\0') => self
                    .doctype_public_id
                    .get_or_insert_with(String::new)
                    .push('\u{FFFD}'),
                Some('>') => {
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.state = Data;
                }
                Some(c) => self
                    .doctype_public_id
                    .get_or_insert_with(String::new)
                    .push(c),
                None => {
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
            },
            AfterDoctypePublicIdentifier => match self.peek() {
                Some(c) if c.is_ascii_whitespace() => {
                    self.advance();
                    self.state = BetweenDoctypePublicAndSystemIdentifiers;
                }
                Some('>') => {
                    self.advance();
                    self.emit_doctype();
                    self.state = Data;
                }
                Some('"') => {
                    self.advance();
                    self.doctype_system_id = Some(String::new());
                    self.state = DoctypeSystemIdentifierDoubleQuoted;
                }
                Some('\'') => {
                    self.advance();
                    self.doctype_system_id = Some(String::new());
                    self.state = DoctypeSystemIdentifierSingleQuoted;
                }
                None => {
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some(_) => {
                    self.doctype_force_quirks = true;
                    self.state = BogusDoctype;
                }
            },
            BetweenDoctypePublicAndSystemIdentifiers => match self.peek() {
                Some(c) if c.is_ascii_whitespace() => {
                    self.advance();
                }
                Some('>') => {
                    self.advance();
                    self.emit_doctype();
                    self.state = Data;
                }
                Some('"') => {
                    self.advance();
                    self.doctype_system_id = Some(String::new());
                    self.state = DoctypeSystemIdentifierDoubleQuoted;
                }
                Some('\'') => {
                    self.advance();
                    self.doctype_system_id = Some(String::new());
                    self.state = DoctypeSystemIdentifierSingleQuoted;
                }
                None => {
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some(_) => {
                    self.doctype_force_quirks = true;
                    self.state = BogusDoctype;
                }
            },
            AfterDoctypeSystemKeyword => match self.peek() {
                Some(c) if c.is_ascii_whitespace() => {
                    self.advance();
                    self.state = BeforeDoctypeSystemIdentifier;
                }
                Some('"') => {
                    self.advance();
                    self.doctype_system_id = Some(String::new());
                    self.state = DoctypeSystemIdentifierDoubleQuoted;
                }
                Some('\'') => {
                    self.advance();
                    self.doctype_system_id = Some(String::new());
                    self.state = DoctypeSystemIdentifierSingleQuoted;
                }
                Some('>') => {
                    self.advance();
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.state = Data;
                }
                None => {
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some(_) => {
                    self.doctype_force_quirks = true;
                    self.state = BogusDoctype;
                }
            },
            BeforeDoctypeSystemIdentifier => match self.peek() {
                Some(c) if c.is_ascii_whitespace() => {
                    self.advance();
                }
                Some('"') => {
                    self.advance();
                    self.doctype_system_id = Some(String::new());
                    self.state = DoctypeSystemIdentifierDoubleQuoted;
                }
                Some('\'') => {
                    self.advance();
                    self.doctype_system_id = Some(String::new());
                    self.state = DoctypeSystemIdentifierSingleQuoted;
                }
                Some('>') => {
                    self.advance();
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.state = Data;
                }
                None => {
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some(_) => {
                    self.doctype_force_quirks = true;
                    self.state = BogusDoctype;
                }
            },
            DoctypeSystemIdentifierDoubleQuoted => match self.advance() {
                Some('"') => self.state = AfterDoctypeSystemIdentifier,
                Some('\0') => self
                    .doctype_system_id
                    .get_or_insert_with(String::new)
                    .push('\u{FFFD}'),
                Some('>') => {
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.state = Data;
                }
                Some(c) => self
                    .doctype_system_id
                    .get_or_insert_with(String::new)
                    .push(c),
                None => {
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
            },
            DoctypeSystemIdentifierSingleQuoted => match self.advance() {
                Some('\'') => self.state = AfterDoctypeSystemIdentifier,
                Some('\0') => self
                    .doctype_system_id
                    .get_or_insert_with(String::new)
                    .push('\u{FFFD}'),
                Some('>') => {
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.state = Data;
                }
                Some(c) => self
                    .doctype_system_id
                    .get_or_insert_with(String::new)
                    .push(c),
                None => {
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
            },
            AfterDoctypeSystemIdentifier => match self.peek() {
                Some(c) if c.is_ascii_whitespace() => {
                    self.advance();
                }
                Some('>') => {
                    self.advance();
                    self.emit_doctype();
                    self.state = Data;
                }
                None => {
                    self.doctype_force_quirks = true;
                    self.emit_doctype();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
                Some(_) => self.state = BogusDoctype,
            },
            BogusDoctype => match self.advance() {
                Some('>') => {
                    self.emit_doctype();
                    self.state = Data;
                }
                Some(_) => {}
                None => {
                    self.emit_doctype();
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
            },

            CdataSection => match self.advance() {
                Some(']') => self.state = CdataSectionBracket,
                Some(c) => self.emit_char(c),
                None => {
                    self.tokens.push_back(Token::Eof);
                    return true;
                }
            },
            CdataSectionBracket => match self.peek() {
                Some(']') => {
                    self.advance();
                    self.state = CdataSectionEnd;
                }
                _ => {
                    self.emit_char(']');
                    self.state = CdataSection;
                }
            },
            CdataSectionEnd => match self.peek() {
                Some(']') => {
                    self.advance();
                    self.emit_char(']');
                }
                Some('>') => {
                    self.advance();
                    self.state = Data;
                }
                _ => {
                    self.emit_str("]]");
                    self.state = CdataSection;
                }
            },

            CharacterReference => {
                self.temp_buffer.clear();
                self.temp_buffer.push('&');
                match self.peek() {
                    Some(c) if c.is_ascii_alphanumeric() => {
                        self.state = NamedCharacterReference;
                    }
                    Some('#') => {
                        self.advance();
                        self.temp_buffer.push('#');
                        self.state = NumericCharacterReference;
                    }
                    _ => {
                        self.flush_temp_buffer_as_characters();
                        self.state = self.return_state;
                    }
                }
            }
            NamedCharacterReference => {
                let remaining = self.rest();
                if let Some((len, codepoints)) = entities::longest_match(&remaining) {
                    let matched: String = remaining.chars().take(len).collect();
                    let next_char_after = remaining.chars().nth(len);
                    let in_attribute = matches!(
                        self.return_state,
                        AttributeValueDoubleQuoted
                            | AttributeValueSingleQuoted
                            | AttributeValueUnquoted
                    );
                    let ends_with_semicolon = matched.ends_with(';');

                    // Spec: if we're in an attribute, the match doesn't end
                    // in ';', and the next char is '=' or alphanumeric,
                    // don't consume it as a reference -- treat the whole
                    // thing as plain text (avoids e.g. breaking `&notin=x`
                    // in an unquoted attribute value).
                    let suppress = in_attribute
                        && !ends_with_semicolon
                        && matches!(next_char_after, Some(c) if c == '=' || c.is_ascii_alphanumeric());

                    if suppress {
                        self.flush_temp_buffer_as_characters();
                        self.state = self.return_state;
                    } else {
                        self.consume_n(len);
                        for &cp in codepoints {
                            if let Some(c) = char::from_u32(cp) {
                                self.append_char_ref_output(c);
                            }
                        }
                        self.state = self.return_state;
                    }
                } else {
                    // No match at all: still flush the '&' (temp_buffer)
                    // before falling into ambiguous-ampersand handling --
                    // otherwise the leading '&' is silently dropped.
                    self.flush_temp_buffer_as_characters();
                    self.state = AmbiguousAmpersand;
                }
            }
            AmbiguousAmpersand => match self.peek() {
                Some(c) if c.is_ascii_alphanumeric() => {
                    self.advance();
                    let in_attribute = matches!(
                        self.return_state,
                        AttributeValueDoubleQuoted
                            | AttributeValueSingleQuoted
                            | AttributeValueUnquoted
                    );
                    if in_attribute {
                        self.attr_value.push(c);
                    } else {
                        self.emit_char(c);
                    }
                }
                Some(';') => {
                    self.state = self.return_state;
                }
                _ => {
                    self.state = self.return_state;
                }
            },
            NumericCharacterReference => {
                self.char_ref_code = 0;
                match self.peek() {
                    Some(c @ ('x' | 'X')) => {
                        self.advance();
                        // Must be remembered in case there turn out to be
                        // no hex digits after it (e.g. "&#x" at EOF), in
                        // which case the whole "&#x" gets flushed as plain
                        // text.
                        self.temp_buffer.push(c);
                        self.state = HexadecimalCharacterReferenceStart;
                    }
                    _ => self.state = DecimalCharacterReferenceStart,
                }
            }
            HexadecimalCharacterReferenceStart => match self.peek() {
                Some(c) if c.is_ascii_hexdigit() => self.state = HexadecimalCharacterReference,
                _ => {
                    self.flush_temp_buffer_as_characters();
                    self.state = self.return_state;
                }
            },
            DecimalCharacterReferenceStart => match self.peek() {
                Some(c) if c.is_ascii_digit() => self.state = DecimalCharacterReference,
                _ => {
                    self.flush_temp_buffer_as_characters();
                    self.state = self.return_state;
                }
            },
            HexadecimalCharacterReference => match self.peek() {
                Some(c) if c.is_ascii_hexdigit() => {
                    self.advance();
                    // Saturating, not wrapping: a too-large reference must
                    // stay detectably out-of-range (> 0x10FFFF) so
                    // NumericCharacterReferenceEnd maps it to U+FFFD,
                    // rather than wrapping back into a valid codepoint.
                    self.char_ref_code = self
                        .char_ref_code
                        .saturating_mul(16)
                        .saturating_add(c.to_digit(16).unwrap());
                }
                Some(';') => {
                    self.advance();
                    self.state = NumericCharacterReferenceEnd;
                }
                _ => self.state = NumericCharacterReferenceEnd,
            },
            DecimalCharacterReference => match self.peek() {
                Some(c) if c.is_ascii_digit() => {
                    self.advance();
                    self.char_ref_code = self
                        .char_ref_code
                        .saturating_mul(10)
                        .saturating_add(c.to_digit(10).unwrap());
                }
                Some(';') => {
                    self.advance();
                    self.state = NumericCharacterReferenceEnd;
                }
                _ => self.state = NumericCharacterReferenceEnd,
            },
            NumericCharacterReferenceEnd => {
                let code = self.char_ref_code;
                let resolved = if (0x80..=0x9F).contains(&code) {
                    C1_REPLACEMENTS[(code - 0x80) as usize]
                } else if code == 0 || code > 0x10FFFF || (0xD800..=0xDFFF).contains(&code) {
                    '\u{FFFD}'
                } else {
                    char::from_u32(code).unwrap_or('\u{FFFD}')
                };
                self.append_char_ref_output(resolved);
                self.state = self.return_state;
            }
        }
        false
    }

    /// Shared body for `RcDataEndTagName`/`RawTextEndTagName`/
    /// `ScriptDataEndTagName`: only treat `</name` as an end tag if `name`
    /// matches the last start tag emitted (the "appropriate end tag token"
    /// check) -- otherwise it's just text.
    fn tag_end_name_step(&mut self, text_state: TokenizerState) {
        match self.peek() {
            Some(c) if c.is_ascii_whitespace() && self.is_appropriate_end_tag() => {
                self.advance();
                self.state = TokenizerState::BeforeAttributeName;
            }
            Some('/') if self.is_appropriate_end_tag() => {
                self.advance();
                self.state = TokenizerState::SelfClosingStartTag;
            }
            Some('>') if self.is_appropriate_end_tag() => {
                self.advance();
                self.emit_tag();
                self.state = TokenizerState::Data;
            }
            Some(c) if c.is_ascii_alphabetic() => {
                self.advance();
                self.tag_name.push(c.to_ascii_lowercase());
                self.temp_buffer.push(c);
            }
            _ => {
                self.emit_str("</");
                self.emit_str(&self.temp_buffer.clone());
                self.state = text_state;
            }
        }
    }

    fn flush_temp_buffer_as_characters(&mut self) {
        let buf = self.temp_buffer.clone();
        let in_attribute = matches!(
            self.return_state,
            TokenizerState::AttributeValueDoubleQuoted
                | TokenizerState::AttributeValueSingleQuoted
                | TokenizerState::AttributeValueUnquoted
        );
        if in_attribute {
            self.attr_value.push_str(&buf);
        } else {
            self.emit_str(&buf);
        }
    }

    fn append_char_ref_output(&mut self, c: char) {
        let in_attribute = matches!(
            self.return_state,
            TokenizerState::AttributeValueDoubleQuoted
                | TokenizerState::AttributeValueSingleQuoted
                | TokenizerState::AttributeValueUnquoted
        );
        if in_attribute {
            self.attr_value.push(c);
        } else {
            self.emit_char(c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag_names(tokens: &[Token]) -> Vec<&str> {
        tokens
            .iter()
            .filter_map(|t| match t {
                Token::StartTag { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn simple_tag() {
        let tokens = tokenize("<p>hi</p>");
        assert_eq!(tag_names(&tokens), vec!["p"]);
        assert!(matches!(tokens[1], Token::Character('h')));
        assert!(matches!(tokens.last(), Some(Token::Eof)));
    }

    #[test]
    fn attributes() {
        let tokens = tokenize("<h a='b' c=\"d\" e>");
        match &tokens[0] {
            Token::StartTag {
                name, attributes, ..
            } => {
                assert_eq!(name, "h");
                assert_eq!(
                    attributes,
                    &vec![
                        ("a".to_string(), "b".to_string()),
                        ("c".to_string(), "d".to_string()),
                        ("e".to_string(), "".to_string()),
                    ]
                );
            }
            other => panic!("unexpected token: {other:?}"),
        }
    }

    #[test]
    fn named_character_reference() {
        let tokens = tokenize("&amp;");
        assert_eq!(tokens, vec![Token::Character('&'), Token::Eof]);
    }

    #[test]
    fn numeric_character_reference() {
        let tokens = tokenize("&#65;");
        assert_eq!(tokens, vec![Token::Character('A'), Token::Eof]);
    }

    #[test]
    fn doctype() {
        let tokens = tokenize("<!DOCTYPE html>");
        assert_eq!(
            tokens,
            vec![
                Token::Doctype {
                    name: Some("html".to_string()),
                    public_id: None,
                    system_id: None,
                    force_quirks: false,
                },
                Token::Eof
            ]
        );
    }

    #[test]
    fn comment() {
        let tokens = tokenize("<!-- hi -->");
        assert_eq!(tokens, vec![Token::Comment(" hi ".to_string()), Token::Eof]);
    }
}

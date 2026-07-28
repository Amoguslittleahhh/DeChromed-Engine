//! A2: the WHATWG HTML tokenizer state machine.
//!
//! `TokenizerState` lists every state from the spec so the eventual
//! implementation has a checklist with a canonical name for each state to
//! implement against -- transcribed from
//! <https://html.spec.whatwg.org/multipage/parsing.html#tokenization>.
//!
//! `tokenize()` is **not implemented yet**. It currently returns just an EOF
//! token for any input, which is deliberately, obviously wrong: the point is
//! for the html5lib-tests harness (see crates/html5lib_harness) to have a
//! real target to measure a near-zero pass rate against per A1's exit
//! criterion, rather than the harness not existing at all.

use crate::Token;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Not all states are referenced until the state machine is implemented.
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

/// Tokenize `input` per the WHATWG algorithm.
///
/// TODO(A2): this is a placeholder. Replace with the real state machine
/// driven by [`TokenizerState`], starting from `TokenizerState::Data`.
pub fn tokenize(_input: &str) -> Vec<Token> {
    vec![Token::Eof]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_always_ends_in_eof() {
        assert_eq!(tokenize("<p>hi</p>"), vec![Token::Eof]);
    }
}

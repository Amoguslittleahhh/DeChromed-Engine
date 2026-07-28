//! Token types produced by the tokenizer (A2), matching the WHATWG spec's
//! token kinds: <https://html.spec.whatwg.org/multipage/parsing.html#tokenization>

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Doctype {
        name: Option<String>,
        public_id: Option<String>,
        system_id: Option<String>,
        force_quirks: bool,
    },
    StartTag {
        name: String,
        attributes: Vec<(String, String)>,
        self_closing: bool,
    },
    EndTag {
        name: String,
    },
    Comment(String),
    Character(char),
    Eof,
}

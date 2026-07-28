//! A4: the CSS tokenizer, per the CSS Syntax Module Level 3 spec.
//!
//! Reference: <https://www.w3.org/TR/css-syntax-3/#tokenization>
//!
//! Implements the full token set (ident, function, at-keyword, hash,
//! string/bad-string, url/bad-url, delimiters, numeric tokens with unit/
//! percentage suffixes, whitespace, CDO/CDC, and all the bracket/
//! punctuation tokens), input preprocessing (newline normalization, NUL
//! replacement), and escaped-code-point handling in idents/strings/urls.
//! Comments are consumed and discarded during tokenization, matching the
//! spec (they're not emitted as tokens at all).

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Ident(String),
    Function(String),
    AtKeyword(String),
    Hash {
        value: String,
        is_id: bool,
    },
    Str(String),
    BadString,
    Url(String),
    BadUrl,
    Delim(char),
    Number {
        value: f64,
        repr: String,
    },
    Percentage {
        value: f64,
        repr: String,
    },
    Dimension {
        value: f64,
        repr: String,
        unit: String,
    },
    Whitespace,
    Cdo,
    Cdc,
    Colon,
    Semicolon,
    Comma,
    LeftSquare,
    RightSquare,
    LeftParen,
    RightParen,
    LeftCurly,
    RightCurly,
    Eof,
}

/// Preprocessing step, applied before tokenization: CRLF/CR/FF -> LF, and
/// NUL -> U+FFFD. See <https://www.w3.org/TR/css-syntax-3/#input-preprocessing>.
fn preprocess(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\u{0C}' => out.push('\n'),
            '\0' => out.push('\u{FFFD}'),
            c => out.push(c),
        }
    }
    out
}

fn is_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n')
}

fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || !c.is_ascii()
}

fn is_name_char(c: char) -> bool {
    is_name_start(c) || c.is_ascii_digit() || c == '-'
}

pub struct Tokenizer {
    chars: Vec<char>,
    pos: usize,
}

impl Tokenizer {
    pub fn new(input: &str) -> Self {
        Tokenizer {
            chars: preprocess(input).chars().collect(),
            pos: 0,
        }
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn peek(&self) -> Option<char> {
        self.peek_at(0)
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn starts_with(&self, s: &str) -> bool {
        let n = s.chars().count();
        if self.pos + n > self.chars.len() {
            return false;
        }
        self.chars[self.pos..self.pos + n]
            .iter()
            .zip(s.chars())
            .all(|(a, b)| *a == b)
    }

    /// <https://www.w3.org/TR/css-syntax-3/#starts-with-a-valid-escape>
    fn would_start_escape(&self, offset: usize) -> bool {
        self.peek_at(offset) == Some('\\') && self.peek_at(offset + 1) != Some('\n')
    }

    /// <https://www.w3.org/TR/css-syntax-3/#would-start-an-identifier>
    fn would_start_ident(&self, offset: usize) -> bool {
        match self.peek_at(offset) {
            Some('-') => {
                matches!(self.peek_at(offset + 1), Some(c) if is_name_start(c) || c == '-')
                    || self.would_start_escape(offset + 1)
            }
            Some(c) if is_name_start(c) => true,
            Some('\\') => self.would_start_escape(offset),
            _ => false,
        }
    }

    /// <https://www.w3.org/TR/css-syntax-3/#starts-with-a-number>
    fn would_start_number(&self, offset: usize) -> bool {
        match self.peek_at(offset) {
            Some('+') | Some('-') => match self.peek_at(offset + 1) {
                Some(c) if c.is_ascii_digit() => true,
                Some('.') => matches!(self.peek_at(offset + 2), Some(c) if c.is_ascii_digit()),
                _ => false,
            },
            Some('.') => matches!(self.peek_at(offset + 1), Some(c) if c.is_ascii_digit()),
            Some(c) if c.is_ascii_digit() => true,
            _ => false,
        }
    }

    fn consume_whitespace(&mut self) {
        while matches!(self.peek(), Some(c) if is_whitespace(c)) {
            self.advance();
        }
    }

    fn consume_comment(&mut self) {
        // Caller has already checked `starts_with("/*")`.
        self.pos += 2;
        while let Some(c) = self.peek() {
            if c == '*' && self.peek_at(1) == Some('/') {
                self.pos += 2;
                return;
            }
            self.advance();
        }
        // EOF inside a comment: parse error, just stop (spec: nothing emitted).
    }

    /// <https://www.w3.org/TR/css-syntax-3/#consume-escaped-code-point>
    /// Caller has already consumed the leading `\`.
    fn consume_escaped(&mut self) -> char {
        match self.advance() {
            Some(c) if c.is_ascii_hexdigit() => {
                let mut hex = String::new();
                hex.push(c);
                for _ in 0..5 {
                    match self.peek() {
                        Some(h) if h.is_ascii_hexdigit() => {
                            hex.push(h);
                            self.advance();
                        }
                        _ => break,
                    }
                }
                if matches!(self.peek(), Some(c) if is_whitespace(c)) {
                    self.advance();
                }
                let code = u32::from_str_radix(&hex, 16).unwrap_or(0xFFFD);
                if code == 0 || code > 0x10FFFF || (0xD800..=0xDFFF).contains(&code) {
                    '\u{FFFD}'
                } else {
                    char::from_u32(code).unwrap_or('\u{FFFD}')
                }
            }
            Some(c) => c,
            None => '\u{FFFD}',
        }
    }

    /// <https://www.w3.org/TR/css-syntax-3/#consume-name>
    fn consume_name(&mut self) -> String {
        let mut out = String::new();
        loop {
            match self.peek() {
                Some(c) if is_name_char(c) => {
                    out.push(c);
                    self.advance();
                }
                Some('\\') if self.would_start_escape(0) => {
                    self.advance();
                    out.push(self.consume_escaped());
                }
                _ => break,
            }
        }
        out
    }

    /// <https://www.w3.org/TR/css-syntax-3/#consume-string-token>
    fn consume_string(&mut self, quote: char) -> Token {
        let mut out = String::new();
        loop {
            // An unescaped newline is a parse error that ends the string
            // *without* consuming the newline -- it's reconsumed as the
            // start of the next token (typically whitespace).
            if self.peek() == Some('\n') {
                return Token::BadString;
            }
            match self.advance() {
                None => return Token::Str(out),
                Some(c) if c == quote => return Token::Str(out),
                Some('\\') => match self.peek() {
                    None => {}
                    Some('\n') => {
                        self.advance();
                    }
                    _ => out.push(self.consume_escaped()),
                },
                Some(c) => out.push(c),
            }
        }
    }

    /// <https://www.w3.org/TR/css-syntax-3/#consume-a-number>
    fn consume_number(&mut self) -> (String, f64) {
        let mut repr = String::new();
        if matches!(self.peek(), Some('+') | Some('-')) {
            repr.push(self.advance().unwrap());
        }
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            repr.push(self.advance().unwrap());
        }
        if self.peek() == Some('.') && matches!(self.peek_at(1), Some(c) if c.is_ascii_digit()) {
            repr.push(self.advance().unwrap());
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                repr.push(self.advance().unwrap());
            }
        }
        if matches!(self.peek(), Some('e') | Some('E')) {
            let exp_sign_ok = matches!(self.peek_at(1), Some(c) if c.is_ascii_digit())
                || (matches!(self.peek_at(1), Some('+') | Some('-'))
                    && matches!(self.peek_at(2), Some(c) if c.is_ascii_digit()));
            if exp_sign_ok {
                repr.push(self.advance().unwrap());
                if matches!(self.peek(), Some('+') | Some('-')) {
                    repr.push(self.advance().unwrap());
                }
                while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                    repr.push(self.advance().unwrap());
                }
            }
        }
        let value = repr.parse().unwrap_or(0.0);
        (repr, value)
    }

    /// <https://www.w3.org/TR/css-syntax-3/#consume-numeric-token>
    fn consume_numeric(&mut self) -> Token {
        let (repr, value) = self.consume_number();
        if self.would_start_ident(0) {
            let unit = self.consume_name();
            Token::Dimension { value, repr, unit }
        } else if self.peek() == Some('%') {
            self.advance();
            Token::Percentage { value, repr }
        } else {
            Token::Number { value, repr }
        }
    }

    /// <https://www.w3.org/TR/css-syntax-3/#consume-url-token>
    fn consume_url(&mut self) -> Token {
        let mut out = String::new();
        self.consume_whitespace();
        loop {
            match self.peek() {
                None => return Token::Url(out),
                Some(')') => {
                    self.advance();
                    return Token::Url(out);
                }
                Some(c) if is_whitespace(c) => {
                    self.consume_whitespace();
                    match self.peek() {
                        Some(')') | None => {
                            self.advance();
                            return Token::Url(out);
                        }
                        _ => return self.consume_bad_url(),
                    }
                }
                Some('"') | Some('\'') | Some('(') => return self.consume_bad_url(),
                Some('\\') if self.would_start_escape(0) => {
                    self.advance();
                    out.push(self.consume_escaped());
                }
                Some('\\') => return self.consume_bad_url(),
                Some(c) => {
                    out.push(c);
                    self.advance();
                }
            }
        }
    }

    fn consume_bad_url(&mut self) -> Token {
        loop {
            match self.advance() {
                None => break,
                Some(')') => break,
                Some('\\') if self.would_start_escape(0) => {
                    self.consume_escaped();
                }
                _ => {}
            }
        }
        Token::BadUrl
    }

    /// <https://www.w3.org/TR/css-syntax-3/#consume-ident-like-token>
    fn consume_ident_like(&mut self) -> Token {
        let name = self.consume_name();
        if name.eq_ignore_ascii_case("url") && self.peek() == Some('(') {
            self.advance();
            // Skip whitespace, then check for a quoted url (parsed as a
            // string, per spec, rather than the unquoted url() path).
            let mut lookahead = self.pos;
            while matches!(self.chars.get(lookahead), Some(c) if is_whitespace(*c)) {
                lookahead += 1;
            }
            match self.chars.get(lookahead) {
                Some('"') | Some('\'') => {
                    self.pos = lookahead;
                    let quote = self.advance().unwrap();
                    let s = self.consume_string(quote);
                    self.consume_whitespace();
                    match self.advance() {
                        Some(')') | None => match s {
                            Token::Str(v) => Token::Url(v),
                            _ => Token::BadUrl,
                        },
                        _ => self.consume_bad_url(),
                    }
                }
                _ => self.consume_url(),
            }
        } else if self.peek() == Some('(') {
            self.advance();
            Token::Function(name)
        } else {
            Token::Ident(name)
        }
    }

    /// Pulls the next token. <https://www.w3.org/TR/css-syntax-3/#consume-token>
    pub fn next_token(&mut self) -> Token {
        while self.starts_with("/*") {
            self.consume_comment();
        }
        let c = match self.peek() {
            None => return Token::Eof,
            Some(c) => c,
        };
        if is_whitespace(c) {
            self.consume_whitespace();
            return Token::Whitespace;
        }
        match c {
            '"' | '\'' => {
                self.advance();
                self.consume_string(c)
            }
            '#' => {
                if matches!(self.peek_at(1), Some(c) if is_name_char(c))
                    || self.would_start_escape(1)
                {
                    let is_id = self.would_start_ident(1);
                    self.advance();
                    let value = self.consume_name();
                    Token::Hash { value, is_id }
                } else {
                    self.advance();
                    Token::Delim(c)
                }
            }
            '(' => {
                self.advance();
                Token::LeftParen
            }
            ')' => {
                self.advance();
                Token::RightParen
            }
            '+' => {
                if self.would_start_number(0) {
                    self.consume_numeric()
                } else {
                    self.advance();
                    Token::Delim(c)
                }
            }
            ',' => {
                self.advance();
                Token::Comma
            }
            '-' => {
                if self.would_start_number(0) {
                    self.consume_numeric()
                } else if self.starts_with("-->") {
                    self.pos += 3;
                    Token::Cdc
                } else if self.would_start_ident(0) {
                    self.consume_ident_like()
                } else {
                    self.advance();
                    Token::Delim(c)
                }
            }
            '.' => {
                if self.would_start_number(0) {
                    self.consume_numeric()
                } else {
                    self.advance();
                    Token::Delim(c)
                }
            }
            ':' => {
                self.advance();
                Token::Colon
            }
            ';' => {
                self.advance();
                Token::Semicolon
            }
            '<' => {
                if self.starts_with("<!--") {
                    self.pos += 4;
                    Token::Cdo
                } else {
                    self.advance();
                    Token::Delim(c)
                }
            }
            '@' => {
                if self.would_start_ident(1) {
                    self.advance();
                    let name = self.consume_name();
                    Token::AtKeyword(name)
                } else {
                    self.advance();
                    Token::Delim(c)
                }
            }
            '[' => {
                self.advance();
                Token::LeftSquare
            }
            '\\' => {
                if self.would_start_escape(0) {
                    self.consume_ident_like()
                } else {
                    self.advance();
                    Token::Delim(c)
                }
            }
            ']' => {
                self.advance();
                Token::RightSquare
            }
            '{' => {
                self.advance();
                Token::LeftCurly
            }
            '}' => {
                self.advance();
                Token::RightCurly
            }
            c if c.is_ascii_digit() => self.consume_numeric(),
            c if is_name_start(c) => self.consume_ident_like(),
            _ => {
                self.advance();
                Token::Delim(c)
            }
        }
    }
}

/// Tokenizes `input` to completion, discarding nothing (whitespace and
/// EOF are still real entries) except comments, which the spec never
/// surfaces as tokens at all.
pub fn tokenize(input: &str) -> Vec<Token> {
    let mut tok = Tokenizer::new(input);
    let mut out = Vec::new();
    loop {
        let t = tok.next_token();
        let done = matches!(t, Token::Eof);
        out.push(t);
        if done {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(input: &str) -> Vec<Token> {
        let mut v = tokenize(input);
        v.pop(); // drop trailing Eof for readability in assertions
        v
    }

    #[test]
    fn basic_rule_tokens() {
        let t = toks("p { color: red; }");
        assert_eq!(
            t,
            vec![
                Token::Ident("p".into()),
                Token::Whitespace,
                Token::LeftCurly,
                Token::Whitespace,
                Token::Ident("color".into()),
                Token::Colon,
                Token::Whitespace,
                Token::Ident("red".into()),
                Token::Semicolon,
                Token::Whitespace,
                Token::RightCurly,
            ]
        );
    }

    #[test]
    fn strings() {
        assert_eq!(toks("\"hi\""), vec![Token::Str("hi".into())]);
        assert_eq!(toks("'hi'"), vec![Token::Str("hi".into())]);
        assert_eq!(
            toks("\"unterminated\n"),
            vec![Token::BadString, Token::Whitespace]
        );
    }

    #[test]
    fn numbers_and_dimensions() {
        assert_eq!(
            toks("10px"),
            vec![Token::Dimension {
                value: 10.0,
                repr: "10".into(),
                unit: "px".into()
            }]
        );
        assert_eq!(
            toks("50%"),
            vec![Token::Percentage {
                value: 50.0,
                repr: "50".into()
            }]
        );
        assert_eq!(
            toks("-1.5e2"),
            vec![Token::Number {
                value: -150.0,
                repr: "-1.5e2".into()
            }]
        );
    }

    #[test]
    fn hash_and_at_keyword() {
        assert_eq!(
            toks("#main"),
            vec![Token::Hash {
                value: "main".into(),
                is_id: true
            }]
        );
        assert_eq!(toks("@media"), vec![Token::AtKeyword("media".into())]);
    }

    #[test]
    fn function_and_url() {
        assert_eq!(toks("rgb("), vec![Token::Function("rgb".into())]);
        assert_eq!(toks("url(foo.png)"), vec![Token::Url("foo.png".into())]);
        assert_eq!(toks("url(\"foo.png\")"), vec![Token::Url("foo.png".into())]);
    }

    #[test]
    fn comments_are_discarded() {
        assert_eq!(
            toks("a/* comment */b"),
            vec![Token::Ident("a".into()), Token::Ident("b".into())]
        );
    }

    #[test]
    fn cdo_cdc() {
        assert_eq!(
            toks("<!-- -->"),
            vec![Token::Cdo, Token::Whitespace, Token::Cdc]
        );
    }
}

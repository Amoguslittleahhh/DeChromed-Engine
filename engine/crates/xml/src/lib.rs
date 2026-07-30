//! A10: standalone XML document parsing.
//!
//! Reference: <https://www.w3.org/TR/xml/> (XML 1.0), <https://www.w3.org/TR/xml-names/>
//! (Namespaces in XML 1.0).
//!
//! Unlike A2/A3's HTML parser, XML's error-handling model is the opposite
//! of permissive: a **fatal well-formedness error** must stop the parser,
//! not silently recover (that's XML's whole design point -- see the
//! module's `parse_document`). This parser enforces the well-formedness
//! constraints that are checkable without a DTD (matched tags, unique
//! attribute names per element, a single root element, correctly nested
//! markup, defined character/entity references) and returns [`XmlError`]
//! on the first one it hits, rather than trying to recover and guess like
//! the HTML parser does.
//!
//! Namespace resolution (`xmlns`/`xmlns:prefix`) is implemented: each
//! element's [`dom::ElementData::namespace`] is resolved from the nearest
//! enclosing declaration, and its `local_name` has any prefix stripped.
//!
//! Known gaps, documented rather than silently wrong:
//! - **Not a validating parser.** No DTD content-model or attribute-list
//!   validation. A `<!DOCTYPE ...>`'s internal subset is recognized
//!   (bracket-balanced) and skipped, not parsed -- so custom entity
//!   declarations (`<!ENTITY foo "...">`) aren't honored; only the XML
//!   spec's 5 predefined entities (`amp`, `lt`, `gt`, `apos`, `quot`) and
//!   numeric character references (`&#NN;`/`&#xHH;`) are recognized.
//!   Using an undeclared named entity is (correctly) a well-formedness
//!   error, so documents that rely on custom DTD entities will fail to
//!   parse here -- a real gap, not a silent wrong-answer.
//! - **Attribute values aren't normalized** per the DTD-driven attribute
//!   value normalization algorithm (which needs attribute-list
//!   declarations this parser doesn't process) -- only the
//!   always-applicable whitespace-to-space and character-reference
//!   substitution rules run.
//! - **XSLT is out of scope entirely**, per `ROADMAP.md`'s explicit
//!   pre-authorization to drop it if scope needs trimming -- this crate
//!   only produces a [`dom::Document`], nothing consumes it as a stylesheet.
//! - **No external entity/DTD fetching** -- `SYSTEM`/`PUBLIC` identifiers
//!   in the DOCTYPE are parsed and stored on the `Doctype` node but never
//!   dereferenced (no network/filesystem access from a parser).

pub mod svg;

use dom::{DoctypeData, Document, ElementData, NodeData, NodeId};

#[derive(Debug, Clone, PartialEq)]
pub struct XmlError {
    pub message: String,
    pub pos: usize,
}

impl std::fmt::Display for XmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "XML parse error at byte {}: {}", self.pos, self.message)
    }
}

impl std::error::Error for XmlError {}

/// Parses `input` as a well-formed XML document, per the module docs.
pub fn parse_document(input: &str) -> Result<Document, XmlError> {
    let mut p = Parser::new(input);
    p.parse()
}

struct NsScope {
    default: Option<String>,
    prefixes: Vec<(String, String)>,
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
    doc: Document,
    ns_stack: Vec<NsScope>,
    depth: usize,
}

/// `parse_element` recurses once per nesting level, matching the grammar's
/// own recursive `element := ... content ...` production -- but stress-
/// testing with a few hundred thousand levels of `<a><a><a>...` overflowed
/// the Rust call stack and aborted the process before this guard existed.
/// Returning a well-formedness error past this depth is the correct XML
/// behavior anyway (this parser is meant to fail fast, not recover -- see
/// module docs), so the fix is just enforcing it explicitly rather than
/// letting the stack do it uncontrolled. 512 is far beyond any real
/// document's nesting (even deeply-nested generated XML like SOAP
/// envelopes or nested `<div>`-equivalents rarely exceeds a few dozen).
const MAX_ELEMENT_NESTING_DEPTH: usize = 512;

const PREDEFINED_ENTITIES: &[(&str, char)] = &[
    ("amp", '&'),
    ("lt", '<'),
    ("gt", '>'),
    ("apos", '\''),
    ("quot", '"'),
];

impl Parser {
    fn new(input: &str) -> Self {
        Parser {
            chars: input.chars().collect(),
            pos: 0,
            doc: Document::new(),
            ns_stack: vec![NsScope {
                default: None,
                prefixes: vec![(
                    "xml".to_string(),
                    "http://www.w3.org/XML/1998/namespace".to_string(),
                )],
            }],
            depth: 0,
        }
    }

    fn err(&self, message: impl Into<String>) -> XmlError {
        XmlError {
            message: message.into(),
            pos: self.pos,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn starts_with(&self, s: &str) -> bool {
        s.chars()
            .enumerate()
            .all(|(i, c)| self.peek_at(i) == Some(c))
    }

    fn consume_str(&mut self, s: &str) -> Result<(), XmlError> {
        if self.starts_with(s) {
            self.pos += s.chars().count();
            Ok(())
        } else {
            Err(self.err(format!("expected {s:?}")))
        }
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_ascii_whitespace()) {
            self.pos += 1;
        }
    }

    fn eof(&self) -> bool {
        self.pos >= self.chars.len()
    }

    /// Name := NameStartChar (NameChar)* -- simplified to a practical
    /// ASCII-plus-common-Unicode-identifier-char subset rather than the
    /// spec's exhaustive Unicode character-class tables.
    fn parse_name(&mut self) -> Result<String, XmlError> {
        let start = self.pos;
        match self.peek() {
            Some(c) if c.is_alphabetic() || c == '_' || c == ':' => {
                self.advance();
            }
            _ => return Err(self.err("expected a name")),
        }
        while matches!(self.peek(), Some(c) if c.is_alphanumeric() || matches!(c, '_' | ':' | '-' | '.'))
        {
            self.advance();
        }
        Ok(self.chars[start..self.pos].iter().collect())
    }

    fn parse(&mut self) -> Result<Document, XmlError> {
        self.skip_prolog();
        self.skip_misc()?;
        if self.starts_with("<!DOCTYPE") {
            self.parse_doctype()?;
            self.skip_misc()?;
        }
        if !self.starts_with("<") || self.peek_at(1) == Some('/') {
            return Err(self.err("expected root element"));
        }
        let root = self.doc.root();
        self.parse_element(root)?;
        self.skip_misc()?;
        if !self.eof() {
            return Err(self.err("content after root element's end tag"));
        }
        Ok(std::mem::replace(&mut self.doc, Document::new()))
    }

    /// `<?xml version="1.0" ...?>` -- recognized and discarded (no
    /// document-node-visible effect; encoding/standalone aren't acted on
    /// since input already arrives as a Rust `&str`, i.e. already-decoded
    /// Unicode).
    fn skip_prolog(&mut self) {
        if self.starts_with("<?xml")
            && matches!(self.peek_at(5), Some(c) if c.is_ascii_whitespace() || c == '?')
        {
            while !self.eof() && !self.starts_with("?>") {
                self.advance();
            }
            let _ = self.consume_str("?>");
        }
    }

    /// Misc := Comment | PI | S -- appended as siblings of the eventual
    /// root element under the document node.
    fn skip_misc(&mut self) -> Result<(), XmlError> {
        loop {
            self.skip_whitespace();
            if self.starts_with("<!--") {
                let text = self.parse_comment_text()?;
                self.doc.append(self.doc.root(), NodeData::Comment(text));
            } else if self.starts_with("<?") {
                let (target, data) = self.parse_pi()?;
                self.doc.append(
                    self.doc.root(),
                    NodeData::ProcessingInstruction { target, data },
                );
            } else {
                break;
            }
        }
        Ok(())
    }

    fn parse_comment_text(&mut self) -> Result<String, XmlError> {
        self.consume_str("<!--")?;
        let start = self.pos;
        while !self.starts_with("-->") {
            if self.eof() {
                return Err(self.err("unterminated comment"));
            }
            if self.starts_with("--") {
                return Err(self.err("comments must not contain \"--\""));
            }
            self.advance();
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        self.consume_str("-->")?;
        Ok(text)
    }

    fn parse_pi(&mut self) -> Result<(String, String), XmlError> {
        self.consume_str("<?")?;
        let target = self.parse_name()?;
        if target.eq_ignore_ascii_case("xml") {
            return Err(self.err("processing instruction target must not be \"xml\""));
        }
        self.skip_whitespace();
        let start = self.pos;
        while !self.eof() && !self.starts_with("?>") {
            self.advance();
        }
        if self.eof() {
            return Err(self.err("unterminated processing instruction"));
        }
        let data: String = self.chars[start..self.pos].iter().collect();
        self.consume_str("?>")?;
        Ok((target, data))
    }

    /// `<!DOCTYPE Name (ExternalID)? (S? '[' intSubset ']' S?)? S? '>'`.
    /// The internal subset (if any) is only bracket-balance-skipped, not
    /// parsed -- see the module's "not a validating parser" gap note.
    fn parse_doctype(&mut self) -> Result<(), XmlError> {
        self.consume_str("<!DOCTYPE")?;
        self.skip_whitespace();
        let name = self.parse_name()?;
        self.skip_whitespace();
        let (mut public_id, mut system_id) = (None, None);
        if self.starts_with("PUBLIC") {
            self.consume_str("PUBLIC")?;
            self.skip_whitespace();
            public_id = Some(self.parse_quoted()?);
            self.skip_whitespace();
            system_id = Some(self.parse_quoted()?);
        } else if self.starts_with("SYSTEM") {
            self.consume_str("SYSTEM")?;
            self.skip_whitespace();
            system_id = Some(self.parse_quoted()?);
        }
        self.skip_whitespace();
        if self.peek() == Some('[') {
            self.advance();
            let mut depth = 1;
            while depth > 0 {
                match self.advance() {
                    Some('[') => depth += 1,
                    Some(']') => depth -= 1,
                    Some(_) => {}
                    None => return Err(self.err("unterminated internal subset")),
                }
            }
        }
        self.skip_whitespace();
        self.consume_str(">")?;
        self.doc.append(
            self.doc.root(),
            NodeData::Doctype(DoctypeData {
                name,
                public_id,
                system_id,
            }),
        );
        Ok(())
    }

    fn parse_quoted(&mut self) -> Result<String, XmlError> {
        let quote = match self.advance() {
            Some(c @ ('"' | '\'')) => c,
            _ => return Err(self.err("expected a quoted string")),
        };
        let start = self.pos;
        while self.peek() != Some(quote) {
            if self.eof() {
                return Err(self.err("unterminated quoted string"));
            }
            self.advance();
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        self.advance();
        Ok(s)
    }

    /// element := EmptyElemTag | STag content ETag. `parent` is where the
    /// new element (and, recursively, everything under it) gets attached.
    fn parse_element(&mut self, parent: NodeId) -> Result<(), XmlError> {
        if self.depth >= MAX_ELEMENT_NESTING_DEPTH {
            return Err(self.err("element nesting too deep"));
        }
        self.depth += 1;
        self.consume_str("<")?;
        let qname = self.parse_name()?;
        let mut attrs: Vec<(String, String)> = Vec::new();
        let mut xmlns_default: Option<String> = None;
        let mut xmlns_prefixes: Vec<(String, String)> = Vec::new();
        loop {
            let had_space = matches!(self.peek(), Some(c) if c.is_ascii_whitespace());
            self.skip_whitespace();
            if self.starts_with("/>") || self.peek() == Some('>') {
                break;
            }
            if !had_space {
                return Err(self.err("expected whitespace before attribute"));
            }
            let aname = self.parse_name()?;
            self.skip_whitespace();
            self.consume_str("=")?;
            self.skip_whitespace();
            let avalue = self.parse_attr_value()?;
            if aname == "xmlns" {
                xmlns_default = Some(avalue);
                continue;
            }
            if let Some(prefix) = aname.strip_prefix("xmlns:") {
                xmlns_prefixes.push((prefix.to_string(), avalue));
                continue;
            }
            if attrs.iter().any(|(k, _)| k == &aname) {
                return Err(self.err(format!("duplicate attribute {aname:?}")));
            }
            attrs.push((aname, avalue));
        }

        self.ns_stack.push(NsScope {
            default: xmlns_default,
            prefixes: xmlns_prefixes,
        });

        let (prefix, local) = split_qname(&qname);
        let namespace = self.resolve_element_namespace(prefix);
        let id = self.doc.append(
            parent,
            NodeData::Element(ElementData::new(namespace, local, attrs)),
        );

        if self.starts_with("/>") {
            self.pos += 2;
            self.ns_stack.pop();
            self.depth -= 1;
            return Ok(());
        }
        self.consume_str(">")?;

        loop {
            if self.eof() {
                return Err(self.err(format!("unclosed element <{qname}>")));
            }
            if self.starts_with("</") {
                break;
            }
            if self.starts_with("<![CDATA[") {
                self.pos += 9;
                let start = self.pos;
                while !self.starts_with("]]>") {
                    if self.eof() {
                        return Err(self.err("unterminated CDATA section"));
                    }
                    self.advance();
                }
                for &c in &self.chars[start..self.pos] {
                    self.doc.append_char(id, c);
                }
                self.pos += 3;
            } else if self.starts_with("<!--") {
                let text = self.parse_comment_text()?;
                self.doc.append(id, NodeData::Comment(text));
            } else if self.starts_with("<?") {
                let (target, data) = self.parse_pi()?;
                self.doc
                    .append(id, NodeData::ProcessingInstruction { target, data });
            } else if self.starts_with("<") {
                self.parse_element(id)?;
            } else {
                let c = self.parse_char_data_char()?;
                self.doc.append_char(id, c);
            }
        }

        self.consume_str("</")?;
        let close_name = self.parse_name()?;
        if close_name != qname {
            return Err(self.err(format!(
                "mismatched closing tag: expected </{qname}>, found </{close_name}>"
            )));
        }
        self.skip_whitespace();
        self.consume_str(">")?;
        self.ns_stack.pop();
        self.depth -= 1;
        Ok(())
    }

    fn resolve_element_namespace(&self, prefix: Option<&str>) -> String {
        match prefix {
            Some(p) => self
                .ns_stack
                .iter()
                .rev()
                .find_map(|scope| {
                    scope
                        .prefixes
                        .iter()
                        .find(|(k, _)| k == p)
                        .map(|(_, v)| v.clone())
                })
                .unwrap_or_default(),
            None => self
                .ns_stack
                .iter()
                .rev()
                .find_map(|scope| scope.default.clone())
                .unwrap_or_default(),
        }
    }

    fn parse_attr_value(&mut self) -> Result<String, XmlError> {
        let quote = match self.advance() {
            Some(c @ ('"' | '\'')) => c,
            _ => return Err(self.err("expected a quoted attribute value")),
        };
        let mut out = String::new();
        loop {
            match self.peek() {
                Some(c) if c == quote => {
                    self.advance();
                    break;
                }
                None => return Err(self.err("unterminated attribute value")),
                Some('<') => return Err(self.err("attribute values must not contain '<'")),
                Some('&') => out.push(self.parse_entity_or_char_ref()?),
                Some(c) if c.is_ascii_whitespace() => {
                    // Whitespace-to-space normalization (the always-
                    // applicable part; see module docs' normalization gap).
                    self.advance();
                    out.push(' ');
                }
                Some(c) => {
                    out.push(c);
                    self.advance();
                }
            }
        }
        Ok(out)
    }

    fn parse_char_data_char(&mut self) -> Result<char, XmlError> {
        match self.peek() {
            Some('&') => self.parse_entity_or_char_ref(),
            Some('>') if self.starts_with("]]>") => {
                Err(self.err("\"]]>\" not allowed in character data"))
            }
            Some(c) => {
                self.advance();
                Ok(c)
            }
            None => Err(self.err("unexpected end of input")),
        }
    }

    /// `&#NN;` / `&#xHH;` (numeric) or a predefined named entity. Any
    /// other name is a well-formedness error (undeclared entity) since
    /// this parser doesn't process `<!ENTITY>` declarations -- see module
    /// docs.
    fn parse_entity_or_char_ref(&mut self) -> Result<char, XmlError> {
        self.consume_str("&")?;
        if self.peek() == Some('#') {
            self.advance();
            let hex = self.peek() == Some('x');
            if hex {
                self.advance();
            }
            let start = self.pos;
            while matches!(self.peek(), Some(c) if c.is_ascii_hexdigit() && (hex || c.is_ascii_digit()))
            {
                self.advance();
            }
            let digits: String = self.chars[start..self.pos].iter().collect();
            self.consume_str(";")?;
            let code = u32::from_str_radix(&digits, if hex { 16 } else { 10 })
                .map_err(|_| self.err("invalid character reference"))?;
            return char::from_u32(code)
                .ok_or_else(|| self.err("invalid character reference codepoint"));
        }
        let name = self.parse_name()?;
        self.consume_str(";")?;
        PREDEFINED_ENTITIES
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, c)| *c)
            .ok_or_else(|| self.err(format!("undeclared entity &{name};")))
    }
}

fn split_qname(qname: &str) -> (Option<&str>, &str) {
    match qname.split_once(':') {
        Some((prefix, local)) => (Some(prefix), local),
        None => (None, qname),
    }
}

/// Re-serializes `doc` back to XML text, escaping per
/// <https://www.w3.org/TR/xml/#syntax>. Doesn't attempt to reproduce the
/// original document's exact formatting/whitespace or re-emit `xmlns`
/// declarations (namespaces are already resolved on each `ElementData`,
/// and re-deriving a minimal, correct set of `xmlns` attributes to
/// reproduce them is a separate, unimplemented "namespace un-resolution"
/// problem) -- documents parsed and re-serialized round-trip in content
/// and tree shape, not in original namespace-declaration placement.
pub fn serialize(doc: &Document) -> String {
    let mut out = String::new();
    for &child in doc.children(doc.root()) {
        serialize_node(doc, child, &mut out);
    }
    out
}

fn serialize_node(doc: &Document, id: NodeId, out: &mut String) {
    match doc.data(id) {
        NodeData::Document => {}
        NodeData::Doctype(d) => {
            out.push_str("<!DOCTYPE ");
            out.push_str(&d.name);
            out.push('>');
        }
        NodeData::Element(el) => {
            out.push('<');
            out.push_str(&el.local_name);
            for (k, v) in &el.attributes {
                out.push(' ');
                out.push_str(k);
                out.push_str("=\"");
                out.push_str(&escape_attr(v));
                out.push('"');
            }
            let children = doc.children(id);
            if children.is_empty() {
                out.push_str("/>");
                return;
            }
            out.push('>');
            for &child in children {
                serialize_node(doc, child, out);
            }
            out.push_str("</");
            out.push_str(&el.local_name);
            out.push('>');
        }
        NodeData::Text(t) => out.push_str(&escape_text(t)),
        NodeData::Comment(c) => {
            out.push_str("<!--");
            out.push_str(c);
            out.push_str("-->");
        }
        NodeData::ProcessingInstruction { target, data } => {
            out.push_str("<?");
            out.push_str(target);
            out.push(' ');
            out.push_str(data);
            out.push_str("?>");
        }
    }
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_attr(s: &str) -> String {
    escape_text(s).replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_document() {
        let doc = parse_document("<root>hello</root>").unwrap();
        let root = doc.children(doc.root())[0];
        match doc.data(root) {
            NodeData::Element(el) => assert_eq!(el.local_name, "root"),
            _ => panic!("expected element"),
        }
        let text = doc.children(root)[0];
        assert!(matches!(doc.data(text), NodeData::Text(t) if t == "hello"));
    }

    #[test]
    fn parses_nested_elements_and_attributes() {
        let doc = parse_document("<a x=\"1\"><b y=\"2\"/></a>").unwrap();
        let a = doc.children(doc.root())[0];
        let NodeData::Element(a_el) = doc.data(a) else {
            panic!()
        };
        assert_eq!(a_el.attr("x"), Some("1"));
        let b = doc.children(a)[0];
        let NodeData::Element(b_el) = doc.data(b) else {
            panic!()
        };
        assert_eq!(b_el.local_name, "b");
        assert_eq!(b_el.attr("y"), Some("2"));
        assert!(doc.children(b).is_empty());
    }

    #[test]
    fn parses_prolog_doctype_comment_pi() {
        let doc = parse_document(
            "<?xml version=\"1.0\"?>\n<!DOCTYPE root SYSTEM \"x.dtd\">\n<!-- hi --><?target data?><root/>",
        )
        .unwrap();
        let kinds: Vec<_> = doc
            .children(doc.root())
            .iter()
            .map(|&id| match doc.data(id) {
                NodeData::Doctype(_) => "doctype",
                NodeData::Comment(_) => "comment",
                NodeData::ProcessingInstruction { .. } => "pi",
                NodeData::Element(_) => "element",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["doctype", "comment", "pi", "element"]);
    }

    #[test]
    fn resolves_default_and_prefixed_namespaces() {
        let doc = parse_document("<root xmlns=\"urn:default\" xmlns:x=\"urn:x\"><x:child/></root>")
            .unwrap();
        let root = doc.children(doc.root())[0];
        let NodeData::Element(root_el) = doc.data(root) else {
            panic!()
        };
        assert_eq!(root_el.namespace, "urn:default");
        let child = doc.children(root)[0];
        let NodeData::Element(child_el) = doc.data(child) else {
            panic!()
        };
        assert_eq!(child_el.namespace, "urn:x");
        assert_eq!(child_el.local_name, "child");
    }

    #[test]
    fn cdata_section_is_literal_text() {
        let doc = parse_document("<root><![CDATA[<not a tag> & stuff]]></root>").unwrap();
        let root = doc.children(doc.root())[0];
        let text = doc.children(root)[0];
        assert!(matches!(doc.data(text), NodeData::Text(t) if t == "<not a tag> & stuff"));
    }

    #[test]
    fn entity_and_character_references() {
        let doc = parse_document("<root>&amp;&#65;&#x42;</root>").unwrap();
        let root = doc.children(doc.root())[0];
        let text = doc.children(root)[0];
        assert!(matches!(doc.data(text), NodeData::Text(t) if t == "&AB"));
    }

    #[test]
    fn mismatched_end_tag_is_a_fatal_error() {
        let err = parse_document("<a><b></a></b>").unwrap_err();
        assert!(err.message.contains("mismatched"));
    }

    #[test]
    fn duplicate_attribute_is_a_fatal_error() {
        let err = parse_document("<a x=\"1\" x=\"2\"/>").unwrap_err();
        assert!(err.message.contains("duplicate"));
    }

    #[test]
    fn undeclared_entity_is_a_fatal_error() {
        let err = parse_document("<a>&undefined;</a>").unwrap_err();
        assert!(err.message.contains("undeclared entity"));
    }

    #[test]
    fn multiple_root_elements_is_a_fatal_error() {
        let err = parse_document("<a/><b/>").unwrap_err();
        assert!(err.message.contains("content after root"));
    }

    #[test]
    fn round_trips_through_serialize() {
        let doc = parse_document("<root a=\"1\"><child>text &amp; more</child></root>").unwrap();
        let text = serialize(&doc);
        let reparsed = parse_document(&text).unwrap();
        assert_eq!(serialize(&reparsed), text);
    }
}

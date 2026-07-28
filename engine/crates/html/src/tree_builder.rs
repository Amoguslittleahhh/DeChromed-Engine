//! A3: HTML tree construction -- the insertion-mode state machine, the
//! adoption agency algorithm, implicit end-tag generation, and foster
//! parenting for stray table content.
//!
//! Reference: <https://html.spec.whatwg.org/multipage/parsing.html#tree-construction>
//!
//! Implemented insertion modes: Initial, BeforeHtml, BeforeHead, InHead,
//! AfterHead, InBody (the workhorse -- headings, `<p>` auto-close, list
//! items, formatting elements + adoption agency, buttons), Text (RCDATA/
//! RAWTEXT/script content), InTable/InTableText/InCaption/InColumnGroup/
//! InTableBody/InRow/InCell (with foster parenting for content that
//! doesn't belong in a table), InSelect, InTemplate (simplified), AfterBody,
//! AfterAfterBody.
//!
//! Known gaps, same spirit as A2's documented ScriptData-escaping gap:
//! - **No foreign content** (SVG/MathML namespace switching). `<svg>`/
//!   `<math>` are parsed as ordinary unknown HTML elements. `svg.dat`,
//!   `math.dat`, `namespace-sensitivity.dat`, `foreign-fragment.dat` are
//!   expected to fail because of this.
//! - **No fragment-context parsing** (`parseFragment`/innerHTML setter
//!   algorithm) -- only full-document parsing. Test cases with a
//!   `#document-fragment` context are skipped by the harness, not attempted.
//! - **`<frameset>` documents** aren't handled (InFrameset/AfterFrameset/
//!   AfterAfterFrameset modes don't exist) -- frameset content parses as
//!   ordinary elements instead of switching modes.
//! - The Noah's Ark clause (deduplicating the active formatting elements
//!   list when 3+ identical entries pile up) isn't implemented -- a rare
//!   edge case, not load-bearing for most real documents.

use crate::tokenizer::{Tokenizer, TokenizerState};
use crate::Token;
use dom::{DoctypeData, Document, ElementData, NodeData, NodeId};

/// Elements that establish a "formatting" context re-applied via the
/// adoption agency algorithm and active-formatting-element reconstruction.
const FORMATTING_TAGS: &[&str] = &[
    "a", "b", "big", "code", "em", "font", "i", "nobr", "s", "small", "strike", "strong", "tt", "u",
];

/// The HTML "special" category, used to find the adoption agency
/// algorithm's "furthest block" and to bound `is_special`-based scope
/// checks. MathML/SVG specials are omitted (no foreign content support).
const SPECIAL_TAGS: &[&str] = &[
    "address",
    "applet",
    "area",
    "article",
    "aside",
    "base",
    "basefont",
    "bgsound",
    "blockquote",
    "body",
    "br",
    "button",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dir",
    "div",
    "dl",
    "dt",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "iframe",
    "img",
    "input",
    "isindex",
    "li",
    "link",
    "listing",
    "main",
    "marquee",
    "menu",
    "meta",
    "nav",
    "noembed",
    "noframes",
    "noscript",
    "object",
    "ol",
    "p",
    "param",
    "plaintext",
    "pre",
    "script",
    "section",
    "select",
    "source",
    "style",
    "summary",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "track",
    "ul",
    "wbr",
];

fn is_special(tag: &str) -> bool {
    SPECIAL_TAGS.contains(&tag)
}

const RCDATA_TAGS: &[&str] = &["title", "textarea"];
const RAWTEXT_TAGS: &[&str] = &[
    "style", "xmp", "iframe", "noembed", "noframes", "script", "noscript",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InsertionMode {
    Initial,
    BeforeHtml,
    BeforeHead,
    InHead,
    InHeadNoscript,
    AfterHead,
    InBody,
    Text,
    InTable,
    InTableText,
    InCaption,
    InColumnGroup,
    InTableBody,
    InRow,
    InCell,
    InSelect,
    InTemplate,
    AfterBody,
    AfterAfterBody,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Default,
    ListItem,
    Button,
    Table,
    Select,
}

#[derive(Debug, Clone)]
enum Afe {
    Marker,
    Element(NodeId, String, Vec<(String, String)>),
}

impl Afe {
    fn node_id(&self) -> Option<NodeId> {
        match self {
            Afe::Marker => None,
            Afe::Element(id, ..) => Some(*id),
        }
    }
}

pub struct TreeBuilder {
    doc: Document,
    open_elements: Vec<NodeId>,
    afe: Vec<Afe>,
    mode: InsertionMode,
    orig_mode: InsertionMode,
    head_element: Option<NodeId>,
    foster_parenting: bool,
    template_modes: Vec<InsertionMode>,
    pending_table_chars: String,
    /// Set right after inserting `<pre>`/`<listing>`/`<textarea>`: per
    /// spec, a single immediately-following LF is silently dropped (an
    /// accommodation for the common `<pre>\ncontent` authoring pattern,
    /// where the newline right after the tag is not meant to be content).
    ignore_next_lf: bool,
    done: bool,
}

impl TreeBuilder {
    fn new() -> Self {
        TreeBuilder {
            doc: Document::new(),
            open_elements: Vec::new(),
            afe: Vec::new(),
            mode: InsertionMode::Initial,
            orig_mode: InsertionMode::Initial,
            head_element: None,
            foster_parenting: false,
            template_modes: Vec::new(),
            pending_table_chars: String::new(),
            ignore_next_lf: false,
            done: false,
        }
    }

    fn tag_name(&self, id: NodeId) -> String {
        match self.doc.data(id) {
            NodeData::Element(el) => el.local_name.clone(),
            _ => String::new(),
        }
    }

    fn current_node(&self) -> NodeId {
        *self
            .open_elements
            .last()
            .expect("stack of open elements is never empty once parsing has started")
    }

    fn current_tag(&self) -> String {
        self.tag_name(self.current_node())
    }

    // ---- insertion location & basic insertion primitives ----

    fn appropriate_insertion_location(
        &self,
        override_target: Option<NodeId>,
    ) -> (NodeId, Option<NodeId>) {
        let target = override_target.unwrap_or_else(|| self.current_node());
        if self.foster_parenting
            && matches!(
                self.tag_name(target).as_str(),
                "table" | "tbody" | "tfoot" | "thead" | "tr"
            )
        {
            if let Some(&table_id) = self
                .open_elements
                .iter()
                .rev()
                .find(|&&id| self.tag_name(id) == "table")
            {
                if let Some(parent) = self.doc.parent(table_id) {
                    return (parent, Some(table_id));
                }
                let idx = self
                    .open_elements
                    .iter()
                    .position(|&id| id == table_id)
                    .unwrap_or(1);
                let prev = self.open_elements[idx.saturating_sub(1)];
                return (prev, None);
            }
            return (self.open_elements[0], None);
        }
        (target, None)
    }

    fn insert_element_at(
        &mut self,
        name: &str,
        attrs: Vec<(String, String)>,
        location: (NodeId, Option<NodeId>),
    ) -> NodeId {
        let data = NodeData::Element(ElementData {
            local_name: name.to_string(),
            attributes: attrs,
        });
        let (parent, before) = location;
        self.doc.insert_before(parent, before, data)
    }

    fn insert_html_element(&mut self, name: &str, attrs: Vec<(String, String)>) -> NodeId {
        let location = self.appropriate_insertion_location(None);
        let id = self.insert_element_at(name, attrs, location);
        self.open_elements.push(id);
        id
    }

    /// The "Noah's Ark clause": if 3 entries with the same tag name and
    /// attributes already exist since the last marker (or list start),
    /// drop the earliest one. Without this, something like four
    /// back-to-back identical `<b>` tags reconstructs all four after an
    /// intervening block element closes them, when real engines only
    /// reconstruct three.
    fn apply_noahs_ark(&mut self, name: &str, attrs: &[(String, String)]) {
        let last_marker = self
            .afe
            .iter()
            .rposition(|e| matches!(e, Afe::Marker))
            .map(|i| i + 1)
            .unwrap_or(0);
        let mut matches_found = Vec::new();
        for i in (last_marker..self.afe.len()).rev() {
            if let Afe::Element(_, n, a) = &self.afe[i] {
                if n == name && a == attrs {
                    matches_found.push(i);
                }
            }
        }
        if matches_found.len() >= 3 {
            let earliest = *matches_found.last().unwrap();
            self.afe.remove(earliest);
        }
    }

    fn insert_formatting_element(&mut self, name: &str, attrs: Vec<(String, String)>) -> NodeId {
        self.apply_noahs_ark(name, &attrs);
        let id = self.insert_html_element(name, attrs.clone());
        self.afe.push(Afe::Element(id, name.to_string(), attrs));
        id
    }

    fn insert_character(&mut self, c: char) {
        let (parent, before) = self.appropriate_insertion_location(None);
        match before {
            Some(b) => self.doc.insert_char_before(parent, Some(b), c),
            None => self.doc.append_char(parent, c),
        }
    }

    fn insert_comment(&mut self, text: &str) {
        let (parent, before) = self.appropriate_insertion_location(None);
        self.doc
            .insert_before(parent, before, NodeData::Comment(text.to_string()));
    }

    // ---- scope checks ----

    fn scope_boundary(&self, tag: &str, scope: Scope) -> bool {
        match scope {
            Scope::Default => matches!(
                tag,
                "applet"
                    | "caption"
                    | "html"
                    | "table"
                    | "td"
                    | "th"
                    | "marquee"
                    | "object"
                    | "template"
            ),
            Scope::ListItem => {
                self.scope_boundary(tag, Scope::Default) || matches!(tag, "ol" | "ul")
            }
            Scope::Button => self.scope_boundary(tag, Scope::Default) || tag == "button",
            Scope::Table => matches!(tag, "html" | "table" | "template"),
            Scope::Select => !matches!(tag, "optgroup" | "option"),
        }
    }

    fn has_element_in_scope(&self, target_tag: &str, scope: Scope) -> bool {
        for &id in self.open_elements.iter().rev() {
            let tag = self.tag_name(id);
            if tag == target_tag {
                return true;
            }
            if self.scope_boundary(&tag, scope) {
                return false;
            }
        }
        false
    }

    fn node_in_scope(&self, target: NodeId, scope: Scope) -> bool {
        for &id in self.open_elements.iter().rev() {
            if id == target {
                return true;
            }
            if self.scope_boundary(&self.tag_name(id), scope) {
                return false;
            }
        }
        false
    }

    // ---- implied end tags ----

    fn generate_implied_end_tags(&mut self, except: Option<&str>) {
        loop {
            let tag = self.current_tag();
            let closable = matches!(
                tag.as_str(),
                "dd" | "dt" | "li" | "optgroup" | "option" | "p" | "rb" | "rp" | "rt" | "rtc"
            );
            if closable && Some(tag.as_str()) != except {
                self.open_elements.pop();
            } else {
                break;
            }
        }
    }

    // ---- active formatting elements ----

    fn reconstruct_active_formatting_elements(&mut self) {
        if self.afe.is_empty() {
            return;
        }
        let last = self.afe.len() - 1;
        if matches!(self.afe[last], Afe::Marker)
            || self.afe[last]
                .node_id()
                .is_some_and(|id| self.open_elements.contains(&id))
        {
            return;
        }
        let mut idx = last;
        loop {
            if idx == 0 {
                break;
            }
            idx -= 1;
            let entry_ok = matches!(self.afe[idx], Afe::Marker)
                || self.afe[idx]
                    .node_id()
                    .is_some_and(|id| self.open_elements.contains(&id));
            if entry_ok {
                idx += 1;
                break;
            }
        }
        for i in idx..=last {
            if let Afe::Element(_, name, attrs) = self.afe[i].clone() {
                let new_id = self.insert_html_element(&name, attrs.clone());
                self.afe[i] = Afe::Element(new_id, name, attrs);
            }
        }
    }

    fn clear_afe_to_last_marker(&mut self) {
        while let Some(entry) = self.afe.pop() {
            if matches!(entry, Afe::Marker) {
                break;
            }
        }
    }

    // ---- adoption agency algorithm ----

    fn adoption_agency(&mut self, subject: &str) {
        for _ in 0..8 {
            let last_marker = self
                .afe
                .iter()
                .rposition(|e| matches!(e, Afe::Marker))
                .map(|i| i + 1)
                .unwrap_or(0);
            let formatting_pos = self.afe[last_marker..]
                .iter()
                .rposition(|e| matches!(e, Afe::Element(_, name, _) if name == subject))
                .map(|i| i + last_marker);

            let Some(fpos) = formatting_pos else {
                self.any_other_end_tag(subject);
                return;
            };
            let formatting_node = self.afe[fpos].node_id().unwrap();

            if !self.open_elements.contains(&formatting_node) {
                self.afe.remove(fpos);
                return;
            }
            if !self.node_in_scope(formatting_node, Scope::Default) {
                return;
            }

            let stack_pos = self
                .open_elements
                .iter()
                .position(|&id| id == formatting_node)
                .unwrap();
            let furthest_block = self.open_elements[stack_pos + 1..]
                .iter()
                .position(|&id| is_special(&self.tag_name(id)))
                .map(|i| stack_pos + 1 + i);

            let Some(furthest_block_pos) = furthest_block else {
                self.open_elements.truncate(stack_pos);
                self.afe.remove(fpos);
                return;
            };
            let furthest_block = self.open_elements[furthest_block_pos];
            let common_ancestor = self.open_elements[stack_pos - 1];

            let mut bookmark = fpos + 1;
            let mut node_pos = furthest_block_pos;
            let mut last_node = furthest_block;

            for inner in 1..=1000u32 {
                if node_pos == 0 {
                    break;
                }
                node_pos -= 1;
                let node = self.open_elements[node_pos];
                if node == formatting_node {
                    break;
                }
                let node_afe_pos = self.afe.iter().position(|e| e.node_id() == Some(node));
                if inner > 3 {
                    if let Some(p) = node_afe_pos {
                        self.afe.remove(p);
                        if p < bookmark {
                            bookmark -= 1;
                        }
                    }
                }
                let Some(node_afe_pos) = self.afe.iter().position(|e| e.node_id() == Some(node))
                else {
                    self.open_elements.remove(node_pos);
                    if furthest_block_pos > node_pos {
                        // positions after node_pos shift down by one; recompute
                    }
                    continue;
                };
                let (name, attrs) = match &self.afe[node_afe_pos] {
                    Afe::Element(_, n, a) => (n.clone(), a.clone()),
                    Afe::Marker => unreachable!(),
                };
                let new_node =
                    self.insert_element_at(&name, attrs.clone(), (common_ancestor, None));
                self.doc.detach(new_node);
                self.afe[node_afe_pos] = Afe::Element(new_node, name, attrs);
                self.open_elements[node_pos] = new_node;
                if last_node == furthest_block {
                    bookmark = node_afe_pos + 1;
                }
                self.doc.append_existing(new_node, last_node);
                last_node = new_node;
            }

            let insert_loc = self.appropriate_insertion_location(Some(common_ancestor));
            match insert_loc {
                (parent, Some(before)) => {
                    self.doc
                        .insert_existing_before(parent, Some(before), last_node)
                }
                (parent, None) => self.doc.append_existing(parent, last_node),
            }

            let (fname, fattrs) = match &self.afe[fpos] {
                Afe::Element(_, n, a) => (n.clone(), a.clone()),
                Afe::Marker => unreachable!(),
            };
            let new_formatting =
                self.insert_element_at(&fname, fattrs.clone(), (furthest_block, None));
            let children: Vec<NodeId> = self.doc.children(furthest_block).to_vec();
            for child in children {
                self.doc.append_existing(new_formatting, child);
            }
            self.doc.append_existing(furthest_block, new_formatting);

            self.afe.remove(fpos);
            let bookmark = bookmark.min(self.afe.len());
            self.afe
                .insert(bookmark, Afe::Element(new_formatting, fname, fattrs));

            let stack_pos = self
                .open_elements
                .iter()
                .position(|&id| id == formatting_node)
                .unwrap();
            self.open_elements.remove(stack_pos);
            let furthest_block_pos = self
                .open_elements
                .iter()
                .position(|&id| id == furthest_block)
                .unwrap();
            self.open_elements
                .insert(furthest_block_pos + 1, new_formatting);
        }
    }

    fn any_other_end_tag(&mut self, tag: &str) {
        for i in (0..self.open_elements.len()).rev() {
            let node = self.open_elements[i];
            let node_tag = self.tag_name(node);
            if node_tag == tag {
                self.generate_implied_end_tags(Some(tag));
                self.open_elements.truncate(i);
                return;
            }
            if is_special(&node_tag) {
                return;
            }
        }
    }

    fn pop_until_including(&mut self, tag: &str) {
        while let Some(&id) = self.open_elements.last() {
            let t = self.tag_name(id);
            self.open_elements.pop();
            if t == tag {
                break;
            }
        }
    }

    fn close_p_element(&mut self) {
        self.generate_implied_end_tags(Some("p"));
        self.pop_until_including("p");
    }
}

/// Parses a full HTML document, driving the tokenizer (A2) and this tree
/// builder together -- the tokenizer's state is switched dynamically as
/// content elements (`<title>`, `<script>`, etc) are encountered, matching
/// how a real parser's two stages cooperate.
pub fn parse_document(input: &str) -> Document {
    let mut tb = TreeBuilder::new();
    let mut tokenizer = Tokenizer::new(input, TokenizerState::Data);

    loop {
        if tb.done {
            break;
        }
        let token = tokenizer.next_token();
        let is_eof = matches!(token, Token::Eof);
        tb.process(token, &mut tokenizer);
        if is_eof {
            break;
        }
    }
    tb.doc
}

/// Back-compat entry point matching the old placeholder's signature, used
/// by anything that already has a pre-tokenized stream (mostly historical
/// at this point -- prefer [`parse_document`], which drives the tokenizer
/// itself and can therefore switch its state).
pub fn build_tree(tokens: &[Token]) -> Document {
    // Re-synthesize a document from a fixed token stream: since we can't
    // switch tokenizer state after the fact, this path can't handle RCDATA/
    // RAWTEXT/script content correctly -- fine for the simple cases this
    // function was originally used for (see shell's smoke test).
    let mut tb = TreeBuilder::new();
    let mut tokenizer = Tokenizer::new("", TokenizerState::Data);
    for token in tokens {
        tb.process(token.clone(), &mut tokenizer);
    }
    tb.doc
}

enum Action {
    Continue,
    Reprocess,
}

impl TreeBuilder {
    fn process(&mut self, token: Token, tokenizer: &mut Tokenizer) {
        if self.ignore_next_lf {
            self.ignore_next_lf = false;
            if matches!(token, Token::Character('\n')) {
                return;
            }
        }
        loop {
            match self.step(&token, tokenizer) {
                Action::Continue => break,
                Action::Reprocess => continue,
            }
        }
    }

    fn step(&mut self, token: &Token, tokenizer: &mut Tokenizer) -> Action {
        use InsertionMode::*;
        match self.mode {
            Initial => self.in_initial(token),
            BeforeHtml => self.in_before_html(token),
            BeforeHead => self.in_before_head(token),
            InHead => self.in_head(token, tokenizer),
            InHeadNoscript => self.in_head_noscript(token, tokenizer),
            AfterHead => self.in_after_head(token, tokenizer),
            InBody => self.in_body(token, tokenizer),
            Text => self.in_text(token, tokenizer),
            InTable => self.in_table(token, tokenizer),
            InTableText => self.in_table_text(token, tokenizer),
            InCaption => self.in_caption(token, tokenizer),
            InColumnGroup => self.in_column_group(token),
            InTableBody => self.in_table_body(token, tokenizer),
            InRow => self.in_row(token, tokenizer),
            InCell => self.in_cell(token, tokenizer),
            InSelect => self.in_select(token),
            InTemplate => self.in_template(token, tokenizer),
            AfterBody => self.in_after_body(token),
            AfterAfterBody => self.in_after_after_body(token),
        }
    }

    fn switch_text_mode(&mut self, tokenizer: &mut Tokenizer, tag: &str) {
        self.orig_mode = self.mode;
        self.mode = InsertionMode::Text;
        if RCDATA_TAGS.contains(&tag) {
            tokenizer.set_state(TokenizerState::RcData);
        } else {
            tokenizer.set_state(TokenizerState::RawText);
        }
    }

    // ---------------- Initial ----------------
    fn in_initial(&mut self, token: &Token) -> Action {
        match token {
            Token::Character(c) if c.is_ascii_whitespace() => Action::Continue,
            Token::Comment(text) => {
                self.insert_comment_at_document(text);
                Action::Continue
            }
            Token::Doctype {
                name,
                public_id,
                system_id,
                ..
            } => {
                let id = self.doc.append(
                    self.doc.root(),
                    NodeData::Doctype(DoctypeData {
                        name: name.clone().unwrap_or_default(),
                        public_id: public_id.clone(),
                        system_id: system_id.clone(),
                    }),
                );
                let _ = id;
                self.mode = InsertionMode::BeforeHtml;
                Action::Continue
            }
            _ => {
                self.mode = InsertionMode::BeforeHtml;
                Action::Reprocess
            }
        }
    }

    fn insert_comment_at_document(&mut self, text: &str) {
        self.doc
            .append(self.doc.root(), NodeData::Comment(text.to_string()));
    }

    // ---------------- BeforeHtml ----------------
    fn in_before_html(&mut self, token: &Token) -> Action {
        match token {
            Token::Character(c) if c.is_ascii_whitespace() => Action::Continue,
            Token::Comment(text) => {
                self.insert_comment_at_document(text);
                Action::Continue
            }
            Token::Doctype { .. } => Action::Continue,
            Token::StartTag {
                name, attributes, ..
            } if name == "html" => {
                let id = self.doc.append(
                    self.doc.root(),
                    NodeData::Element(ElementData {
                        local_name: "html".into(),
                        attributes: attributes.clone(),
                    }),
                );
                self.open_elements.push(id);
                self.mode = InsertionMode::BeforeHead;
                Action::Continue
            }
            Token::EndTag { name } if !matches!(name.as_str(), "head" | "body" | "html" | "br") => {
                Action::Continue
            }
            _ => {
                let id = self.doc.append(
                    self.doc.root(),
                    NodeData::Element(ElementData {
                        local_name: "html".into(),
                        attributes: vec![],
                    }),
                );
                self.open_elements.push(id);
                self.mode = InsertionMode::BeforeHead;
                Action::Reprocess
            }
        }
    }

    // ---------------- BeforeHead ----------------
    fn in_before_head(&mut self, token: &Token) -> Action {
        match token {
            Token::Character(c) if c.is_ascii_whitespace() => Action::Continue,
            Token::Comment(text) => {
                self.insert_comment(text);
                Action::Continue
            }
            Token::Doctype { .. } => Action::Continue,
            Token::StartTag { name, .. } if name == "html" => self.in_body_stub_html(token),
            Token::StartTag {
                name, attributes, ..
            } if name == "head" => {
                let id = self.insert_html_element("head", attributes.clone());
                self.head_element = Some(id);
                self.mode = InsertionMode::InHead;
                Action::Continue
            }
            Token::EndTag { name } if !matches!(name.as_str(), "head" | "body" | "html" | "br") => {
                Action::Continue
            }
            _ => {
                let id = self.insert_html_element("head", vec![]);
                self.head_element = Some(id);
                self.mode = InsertionMode::InHead;
                Action::Reprocess
            }
        }
    }

    fn in_body_stub_html(&mut self, token: &Token) -> Action {
        // "html" start tag seen again outside Initial/BeforeHtml: merge new
        // attributes onto the existing root element (matches the spec's
        // "a start tag whose tag name is 'html'" handling repeated across
        // several insertion modes).
        if let Token::StartTag { attributes, .. } = token {
            let html_id = self.open_elements[0];
            if let NodeData::Element(el) = self.doc.data_mut(html_id) {
                for (k, v) in attributes {
                    el.set_if_absent(k, v);
                }
            }
        }
        Action::Continue
    }

    // ---------------- InHead ----------------
    fn in_head(&mut self, token: &Token, tokenizer: &mut Tokenizer) -> Action {
        match token {
            Token::Character(c) if c.is_ascii_whitespace() => {
                self.insert_character(*c);
                Action::Continue
            }
            Token::Comment(text) => {
                self.insert_comment(text);
                Action::Continue
            }
            Token::Doctype { .. } => Action::Continue,
            Token::StartTag { name, .. } if name == "html" => self.in_body_stub_html(token),
            Token::StartTag {
                name,
                attributes,
                self_closing,
            } if matches!(
                name.as_str(),
                "base" | "basefont" | "bgsound" | "link" | "meta"
            ) =>
            {
                self.insert_html_element(name, attributes.clone());
                self.open_elements.pop();
                let _ = self_closing;
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "title" => {
                self.insert_html_element(name, attributes.clone());
                self.switch_text_mode(tokenizer, "title");
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "noframes" | "style") => {
                self.insert_html_element(name, attributes.clone());
                self.switch_text_mode(tokenizer, "style");
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "noscript" => {
                self.insert_html_element(name, attributes.clone());
                self.mode = InsertionMode::InHeadNoscript;
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "script" => {
                self.insert_html_element(name, attributes.clone());
                self.switch_text_mode(tokenizer, "script");
                Action::Continue
            }
            Token::EndTag { name } if name == "head" => {
                self.open_elements.pop();
                self.mode = InsertionMode::AfterHead;
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "template" => {
                self.insert_html_element(name, attributes.clone());
                self.afe.push(Afe::Marker);
                self.template_modes.push(InsertionMode::InTemplate);
                self.mode = InsertionMode::InTemplate;
                Action::Continue
            }
            Token::EndTag { name } if name == "template" => {
                if self
                    .open_elements
                    .iter()
                    .any(|&id| self.tag_name(id) == "template")
                {
                    self.generate_implied_end_tags(None);
                    self.pop_until_including("template");
                    self.clear_afe_to_last_marker();
                    self.template_modes.pop();
                    self.mode = self
                        .template_modes
                        .last()
                        .copied()
                        .unwrap_or(InsertionMode::InBody);
                }
                Action::Continue
            }
            Token::EndTag { name } if !matches!(name.as_str(), "body" | "html" | "br") => {
                Action::Continue
            }
            _ => {
                self.open_elements.pop();
                self.mode = InsertionMode::AfterHead;
                Action::Reprocess
            }
        }
    }

    // ---------------- InHeadNoscript ----------------
    fn in_head_noscript(&mut self, token: &Token, tokenizer: &mut Tokenizer) -> Action {
        match token {
            Token::Doctype { .. } => Action::Continue,
            Token::StartTag { name, .. } if name == "html" => self.in_body(token, tokenizer),
            Token::EndTag { name } if name == "noscript" => {
                self.open_elements.pop();
                self.mode = InsertionMode::InHead;
                Action::Continue
            }
            Token::Character(c) if c.is_ascii_whitespace() => self.in_head(token, tokenizer),
            Token::Comment(_) => self.in_head(token, tokenizer),
            Token::StartTag { name, .. }
                if matches!(
                    name.as_str(),
                    "basefont" | "bgsound" | "link" | "meta" | "noframes" | "style"
                ) =>
            {
                self.in_head(token, tokenizer)
            }
            Token::StartTag { name, .. } if matches!(name.as_str(), "head" | "noscript") => {
                Action::Continue
            }
            Token::EndTag { name } if name == "br" => self.leave_head_noscript_and_reprocess(),
            _ => self.leave_head_noscript_and_reprocess(),
        }
    }

    fn leave_head_noscript_and_reprocess(&mut self) -> Action {
        self.open_elements.pop();
        self.mode = InsertionMode::InHead;
        Action::Reprocess
    }

    // ---------------- AfterHead ----------------
    fn in_after_head(&mut self, token: &Token, tokenizer: &mut Tokenizer) -> Action {
        match token {
            Token::Character(c) if c.is_ascii_whitespace() => {
                self.insert_character(*c);
                Action::Continue
            }
            Token::Comment(text) => {
                self.insert_comment(text);
                Action::Continue
            }
            Token::Doctype { .. } => Action::Continue,
            Token::StartTag { name, .. } if name == "html" => self.in_body_stub_html(token),
            Token::StartTag {
                name, attributes, ..
            } if name == "body" => {
                self.insert_html_element(name, attributes.clone());
                self.mode = InsertionMode::InBody;
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "frameset" => {
                self.insert_html_element(name, attributes.clone());
                self.mode = InsertionMode::InBody;
                Action::Continue
            }
            Token::StartTag { name, .. }
                if matches!(
                    name.as_str(),
                    "base"
                        | "basefont"
                        | "bgsound"
                        | "link"
                        | "meta"
                        | "noframes"
                        | "script"
                        | "style"
                        | "template"
                        | "title"
                ) =>
            {
                if let Some(head) = self.head_element {
                    self.open_elements.push(head);
                    let action = self.in_head(token, tokenizer);
                    // Pop the temporarily-reopened head back off unless
                    // in_head itself already popped it (e.g. base/link/meta
                    // insert-then-immediately-pop themselves).
                    if self.open_elements.last() == Some(&head) {
                        self.open_elements.pop();
                    }
                    return action;
                }
                Action::Continue
            }
            Token::EndTag { name } if name == "template" => self.in_head(token, tokenizer),
            Token::EndTag { name } if !matches!(name.as_str(), "body" | "html" | "br") => {
                Action::Continue
            }
            _ => {
                self.insert_html_element("body", vec![]);
                self.mode = InsertionMode::InBody;
                Action::Reprocess
            }
        }
    }

    // ---------------- InBody ----------------
    fn in_body(&mut self, token: &Token, tokenizer: &mut Tokenizer) -> Action {
        match token {
            Token::Character('\0') => Action::Continue,
            Token::Character(c) => {
                self.reconstruct_active_formatting_elements();
                self.insert_character(*c);
                Action::Continue
            }
            Token::Comment(text) => {
                self.insert_comment(text);
                Action::Continue
            }
            Token::Doctype { .. } => Action::Continue,
            Token::StartTag { name, .. } if name == "html" => self.in_body_stub_html(token),
            Token::StartTag {
                name, attributes, ..
            } if matches!(
                name.as_str(),
                "base"
                    | "basefont"
                    | "bgsound"
                    | "link"
                    | "meta"
                    | "noframes"
                    | "script"
                    | "style"
                    | "template"
                    | "title"
            ) =>
            {
                let _ = attributes;
                self.in_head(token, tokenizer)
            }
            Token::EndTag { name } if name == "template" => self.in_head(token, tokenizer),
            Token::StartTag { name, .. } if name == "head" => {
                // Parse error, ignore: a second `<head>` start tag once
                // already past the head section doesn't reopen or
                // reinsert one.
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "body" => {
                if let Some(&body) = self.open_elements.get(1) {
                    if self.tag_name(body) == "body" {
                        if let NodeData::Element(el) = self.doc.data_mut(body) {
                            for (k, v) in attributes {
                                el.set_if_absent(k, v);
                            }
                        }
                    }
                }
                Action::Continue
            }
            Token::StartTag { name, .. } if name == "frameset" => Action::Continue,
            Token::Eof => {
                self.done = true;
                Action::Continue
            }
            Token::EndTag { name } if name == "body" => {
                self.mode = InsertionMode::AfterBody;
                Action::Continue
            }
            Token::EndTag { name } if name == "html" => {
                self.mode = InsertionMode::AfterBody;
                Action::Reprocess
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(
                name.as_str(),
                "address"
                    | "article"
                    | "aside"
                    | "blockquote"
                    | "center"
                    | "details"
                    | "dialog"
                    | "dir"
                    | "div"
                    | "dl"
                    | "fieldset"
                    | "figcaption"
                    | "figure"
                    | "footer"
                    | "header"
                    | "hgroup"
                    | "main"
                    | "menu"
                    | "nav"
                    | "ol"
                    | "p"
                    | "section"
                    | "summary"
                    | "ul"
            ) =>
            {
                if self.has_element_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }
                self.insert_html_element(name, attributes.clone());
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6") => {
                if self.has_element_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }
                if matches!(
                    self.current_tag().as_str(),
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                ) {
                    self.open_elements.pop();
                }
                self.insert_html_element(name, attributes.clone());
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "pre" | "listing") => {
                if self.has_element_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }
                self.insert_html_element(name, attributes.clone());
                self.ignore_next_lf = true;
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "form" => {
                self.insert_html_element(name, attributes.clone());
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "rb" | "rtc") => {
                if self.has_element_in_scope("ruby", Scope::Default) {
                    self.generate_implied_end_tags(None);
                }
                self.insert_html_element(name, attributes.clone());
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "rp" | "rt") => {
                if self.has_element_in_scope("ruby", Scope::Default) {
                    self.generate_implied_end_tags(Some("rtc"));
                }
                self.insert_html_element(name, attributes.clone());
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "li") => {
                for &id in self.open_elements.iter().rev() {
                    let tag = self.tag_name(id);
                    if tag == "li" {
                        self.generate_implied_end_tags(Some("li"));
                        self.pop_until_including("li");
                        break;
                    }
                    if is_special(&tag) && !matches!(tag.as_str(), "address" | "div" | "p") {
                        break;
                    }
                }
                if self.has_element_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }
                self.insert_html_element(name, attributes.clone());
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "dd" | "dt") => {
                for &id in self.open_elements.iter().rev() {
                    let tag = self.tag_name(id);
                    if tag == "dd" || tag == "dt" {
                        self.generate_implied_end_tags(Some(&tag));
                        self.pop_until_including(&tag);
                        break;
                    }
                    if is_special(&tag) && !matches!(tag.as_str(), "address" | "div" | "p") {
                        break;
                    }
                }
                if self.has_element_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }
                self.insert_html_element(name, attributes.clone());
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "plaintext" => {
                if self.has_element_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }
                self.insert_html_element(name, attributes.clone());
                tokenizer.set_state(TokenizerState::PlainText);
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "textarea" => {
                // Spec also sets frameset-ok to "not ok" -- skipped as a
                // minor approximation (frameset documents aren't handled;
                // see this file's module docs).
                self.insert_html_element(name, attributes.clone());
                self.switch_text_mode(tokenizer, "textarea");
                self.ignore_next_lf = true;
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name != "noscript" && RAWTEXT_TAGS.contains(&name.as_str()) => {
                // "noscript" is deliberately excluded: its RAWTEXT-vs-markup
                // tokenization depends on the scripting flag, and this
                // engine assumes scripting disabled throughout (see
                // in_head's noscript handling) -- so its content parses as
                // ordinary markup, not RAWTEXT, consistently in both modes.
                self.insert_html_element(name, attributes.clone());
                self.switch_text_mode(tokenizer, "style");
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "button" => {
                if self.has_element_in_scope("button", Scope::Default) {
                    self.generate_implied_end_tags(None);
                    self.pop_until_including("button");
                }
                self.reconstruct_active_formatting_elements();
                self.insert_html_element(name, attributes.clone());
                Action::Continue
            }
            Token::EndTag { name }
                if matches!(
                    name.as_str(),
                    "address"
                        | "article"
                        | "aside"
                        | "blockquote"
                        | "button"
                        | "center"
                        | "details"
                        | "dialog"
                        | "dir"
                        | "div"
                        | "dl"
                        | "fieldset"
                        | "figcaption"
                        | "figure"
                        | "footer"
                        | "header"
                        | "hgroup"
                        | "listing"
                        | "main"
                        | "menu"
                        | "nav"
                        | "ol"
                        | "pre"
                        | "section"
                        | "summary"
                        | "ul"
                ) =>
            {
                if self.has_element_in_scope(name, Scope::Default) {
                    self.generate_implied_end_tags(None);
                    self.pop_until_including(name);
                }
                Action::Continue
            }
            Token::EndTag { name } if name == "form" => {
                if self.has_element_in_scope("form", Scope::Default) {
                    self.generate_implied_end_tags(None);
                    self.pop_until_including("form");
                }
                Action::Continue
            }
            Token::EndTag { name } if name == "p" => {
                if !self.has_element_in_scope("p", Scope::Button) {
                    self.insert_html_element("p", vec![]);
                }
                self.close_p_element();
                Action::Continue
            }
            Token::EndTag { name } if name == "li" => {
                if self.has_element_in_scope("li", Scope::ListItem) {
                    self.generate_implied_end_tags(Some("li"));
                    self.pop_until_including("li");
                }
                Action::Continue
            }
            Token::EndTag { name } if matches!(name.as_str(), "dd" | "dt") => {
                if self.has_element_in_scope(name, Scope::Default) {
                    self.generate_implied_end_tags(Some(name));
                    self.pop_until_including(name);
                }
                Action::Continue
            }
            Token::EndTag { name }
                if matches!(name.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6") =>
            {
                if ["h1", "h2", "h3", "h4", "h5", "h6"]
                    .iter()
                    .any(|h| self.has_element_in_scope(h, Scope::Default))
                {
                    self.generate_implied_end_tags(None);
                    self.pop_until_including(name);
                }
                Action::Continue
            }
            Token::EndTag { name } if FORMATTING_TAGS.contains(&name.as_str()) => {
                self.adoption_agency(name);
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "nobr" => {
                self.reconstruct_active_formatting_elements();
                // "nobr" gets a pre-check the other formatting tags don't:
                // if one's already open, run the adoption agency on it
                // (and reconstruct again) *before* inserting a new one --
                // without this, back-to-back `<nobr>`s nest instead of
                // becoming siblings the way real browsers render them.
                if self.has_element_in_scope("nobr", Scope::Default) {
                    self.adoption_agency("nobr");
                    self.reconstruct_active_formatting_elements();
                }
                self.insert_formatting_element(name, attributes.clone());
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if FORMATTING_TAGS.contains(&name.as_str()) => {
                self.reconstruct_active_formatting_elements();
                self.insert_formatting_element(name, attributes.clone());
                Action::Continue
            }
            Token::StartTag { name, .. } if name == "a" => {
                self.reconstruct_active_formatting_elements();
                self.insert_formatting_element(name, vec![]);
                Action::Continue
            }
            Token::StartTag { name, .. }
                if matches!(
                    name.as_str(),
                    "br" | "img" | "embed" | "area" | "keygen" | "wbr"
                ) =>
            {
                self.reconstruct_active_formatting_elements();
                self.insert_html_element(name, vec![]);
                self.open_elements.pop();
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "input") => {
                self.reconstruct_active_formatting_elements();
                self.insert_html_element(name, attributes.clone());
                self.open_elements.pop();
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "hr") => {
                if self.has_element_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }
                self.insert_html_element(name, attributes.clone());
                self.open_elements.pop();
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "table") => {
                if self.has_element_in_scope("p", Scope::Button) {
                    self.close_p_element();
                }
                self.insert_html_element(name, attributes.clone());
                self.mode = InsertionMode::InTable;
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "select") => {
                self.reconstruct_active_formatting_elements();
                self.insert_html_element(name, attributes.clone());
                self.mode = InsertionMode::InSelect;
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "optgroup" | "option") => {
                if self.current_tag() == "option" {
                    self.open_elements.pop();
                }
                self.reconstruct_active_formatting_elements();
                self.insert_html_element(name, attributes.clone());
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(
                name.as_str(),
                "td" | "th" | "tr" | "tbody" | "tfoot" | "thead" | "caption" | "colgroup" | "col"
            ) =>
            {
                // Stray table-part tags encountered directly in body (no
                // enclosing table): treat as ordinary elements rather than
                // spec's full "foster or ignore" handling -- a pragmatic
                // approximation, most real markup reaches these via InTable.
                self.insert_html_element(name, attributes.clone());
                Action::Continue
            }
            Token::EndTag { name } if name == "br" => {
                self.reconstruct_active_formatting_elements();
                self.insert_html_element("br", vec![]);
                self.open_elements.pop();
                Action::Continue
            }
            Token::StartTag {
                name,
                attributes,
                self_closing: _,
            } => {
                self.reconstruct_active_formatting_elements();
                self.insert_html_element(name, attributes.clone());
                Action::Continue
            }
            Token::EndTag { name } => {
                self.any_other_end_tag(name);
                Action::Continue
            }
        }
    }

    // ---------------- Text ----------------
    fn in_text(&mut self, token: &Token, tokenizer: &mut Tokenizer) -> Action {
        match token {
            Token::Character(c) => {
                self.insert_character(*c);
                Action::Continue
            }
            Token::EndTag { .. } => {
                self.open_elements.pop();
                self.mode = self.orig_mode;
                let _ = tokenizer;
                Action::Continue
            }
            Token::Eof => {
                self.open_elements.pop();
                self.mode = self.orig_mode;
                Action::Reprocess
            }
            _ => Action::Continue,
        }
    }

    // ---------------- Table family ----------------
    fn clear_stack_to_table_context(&mut self) {
        while !matches!(self.current_tag().as_str(), "table" | "template" | "html") {
            self.open_elements.pop();
        }
    }

    fn in_table(&mut self, token: &Token, tokenizer: &mut Tokenizer) -> Action {
        match token {
            Token::Character(_)
                if matches!(
                    self.current_tag().as_str(),
                    "table" | "tbody" | "tfoot" | "thead" | "tr"
                ) =>
            {
                self.pending_table_chars.clear();
                self.orig_mode = self.mode;
                self.mode = InsertionMode::InTableText;
                Action::Reprocess
            }
            Token::Comment(text) => {
                self.insert_comment(text);
                Action::Continue
            }
            Token::Doctype { .. } => Action::Continue,
            Token::StartTag {
                name, attributes, ..
            } if name == "caption" => {
                self.clear_stack_to_table_context();
                self.afe.push(Afe::Marker);
                self.insert_html_element(name, attributes.clone());
                self.mode = InsertionMode::InCaption;
                Action::Continue
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "colgroup" => {
                self.clear_stack_to_table_context();
                self.insert_html_element(name, attributes.clone());
                self.mode = InsertionMode::InColumnGroup;
                Action::Continue
            }
            Token::StartTag { name, .. } if name == "col" => {
                self.clear_stack_to_table_context();
                self.insert_html_element("colgroup", vec![]);
                self.mode = InsertionMode::InColumnGroup;
                Action::Reprocess
            }
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "tbody" | "tfoot" | "thead") => {
                self.clear_stack_to_table_context();
                self.insert_html_element(name, attributes.clone());
                self.mode = InsertionMode::InTableBody;
                Action::Continue
            }
            Token::StartTag { name, .. } if matches!(name.as_str(), "td" | "th" | "tr") => {
                self.clear_stack_to_table_context();
                self.insert_html_element("tbody", vec![]);
                self.mode = InsertionMode::InTableBody;
                Action::Reprocess
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "table" => {
                let _ = attributes;
                if self.has_element_in_scope("table", Scope::Table) {
                    self.generate_implied_end_tags(None);
                    self.pop_until_including("table");
                    // Reset based on the new current node (this is what
                    // actually guarantees termination for nested
                    // `<table><table>`: it moves the mode away from
                    // InTable when the popped table was the only one,
                    // rather than reprocessing the same token in the same
                    // mode forever).
                    self.reset_insertion_mode();
                    Action::Reprocess
                } else {
                    Action::Continue
                }
            }
            Token::EndTag { name } if name == "table" => {
                if self.has_element_in_scope("table", Scope::Table) {
                    self.pop_until_including("table");
                    self.reset_insertion_mode();
                }
                Action::Continue
            }
            Token::EndTag { name }
                if matches!(
                    name.as_str(),
                    "body"
                        | "caption"
                        | "col"
                        | "colgroup"
                        | "html"
                        | "tbody"
                        | "td"
                        | "tfoot"
                        | "th"
                        | "thead"
                        | "tr"
                ) =>
            {
                Action::Continue
            }
            Token::StartTag { name, .. }
                if matches!(name.as_str(), "style" | "script" | "template") =>
            {
                self.in_head(token, tokenizer)
            }
            Token::EndTag { name } if name == "template" => self.in_head(token, tokenizer),
            Token::StartTag {
                name, attributes, ..
            } if name == "input" => {
                let is_hidden = attributes.iter().any(|(k, v)| {
                    k.eq_ignore_ascii_case("type") && v.eq_ignore_ascii_case("hidden")
                });
                if is_hidden {
                    self.insert_html_element("input", attributes.clone());
                    self.open_elements.pop();
                    Action::Continue
                } else {
                    // Not type=hidden: falls through to the "anything
                    // else" foster-parenting path, same as any other
                    // unrecognized element here.
                    self.foster_parenting = true;
                    let action = self.in_body(token, tokenizer);
                    self.foster_parenting = false;
                    action
                }
            }
            Token::StartTag {
                name, attributes, ..
            } if name == "form" => {
                self.insert_html_element(name, attributes.clone());
                self.open_elements.pop();
                Action::Continue
            }
            Token::Eof => self.in_body(token, tokenizer),
            _ => {
                // "anything else": parse error. Spec: "Enable foster
                // parenting, process the token using the rules for the
                // 'in body' insertion mode, and then disable foster
                // parenting." This is the *only* place InTable enables
                // it -- structural table children (tbody/tr/td/caption/
                // colgroup, handled by their own dedicated branches
                // above) are inserted normally, not foster-parented away
                // from the table they belong in.
                self.foster_parenting = true;
                let action = self.in_body(token, tokenizer);
                self.foster_parenting = false;
                action
            }
        }
    }

    fn reset_insertion_mode(&mut self) {
        for &id in self.open_elements.iter().rev() {
            match self.tag_name(id).as_str() {
                "select" => {
                    self.mode = InsertionMode::InSelect;
                    return;
                }
                "td" | "th" => {
                    self.mode = InsertionMode::InCell;
                    return;
                }
                "tr" => {
                    self.mode = InsertionMode::InRow;
                    return;
                }
                "tbody" | "thead" | "tfoot" => {
                    self.mode = InsertionMode::InTableBody;
                    return;
                }
                "caption" => {
                    self.mode = InsertionMode::InCaption;
                    return;
                }
                "colgroup" => {
                    self.mode = InsertionMode::InColumnGroup;
                    return;
                }
                "table" => {
                    self.mode = InsertionMode::InTable;
                    return;
                }
                "template" => {
                    self.mode = self
                        .template_modes
                        .last()
                        .copied()
                        .unwrap_or(InsertionMode::InBody);
                    return;
                }
                "head" | "body" => {
                    self.mode = InsertionMode::InBody;
                    return;
                }
                "html" => {
                    self.mode = InsertionMode::BeforeHead;
                    return;
                }
                _ => {}
            }
        }
        self.mode = InsertionMode::InBody;
    }

    fn in_table_text(&mut self, token: &Token, tokenizer: &mut Tokenizer) -> Action {
        match token {
            Token::Character('\0') => Action::Continue,
            Token::Character(c) => {
                self.pending_table_chars.push(*c);
                Action::Continue
            }
            _ => {
                let only_whitespace = self
                    .pending_table_chars
                    .chars()
                    .all(|c| c.is_ascii_whitespace());
                let chars = std::mem::take(&mut self.pending_table_chars);
                if only_whitespace {
                    for c in chars.chars() {
                        self.insert_character(c);
                    }
                } else {
                    // Non-whitespace character data inside a table: parse
                    // error. Spec routes this through InTable's "anything
                    // else" (foster parenting enabled, processed via
                    // in-body rules) -- crucially *not* a direct
                    // `insert_character` call, since in-body's character
                    // handling also reconstructs the active formatting
                    // elements first (e.g. re-opening an `<a>` that was
                    // left open before the table).
                    self.foster_parenting = true;
                    for c in chars.chars() {
                        self.in_body(&Token::Character(c), tokenizer);
                    }
                    self.foster_parenting = false;
                }
                self.mode = self.orig_mode;
                Action::Reprocess
            }
        }
    }

    fn in_caption(&mut self, token: &Token, tokenizer: &mut Tokenizer) -> Action {
        match token {
            Token::EndTag { name } if name == "caption" => {
                if self.has_element_in_scope("caption", Scope::Table) {
                    self.generate_implied_end_tags(None);
                    self.pop_until_including("caption");
                    self.clear_afe_to_last_marker();
                    self.mode = InsertionMode::InTable;
                }
                Action::Continue
            }
            Token::StartTag { name, .. }
                if matches!(
                    name.as_str(),
                    "caption"
                        | "col"
                        | "colgroup"
                        | "tbody"
                        | "td"
                        | "tfoot"
                        | "th"
                        | "thead"
                        | "tr"
                ) =>
            {
                if self.has_element_in_scope("caption", Scope::Table) {
                    self.pop_until_including("caption");
                    self.clear_afe_to_last_marker();
                    self.mode = InsertionMode::InTable;
                    Action::Reprocess
                } else {
                    Action::Continue
                }
            }
            Token::EndTag { name }
                if matches!(
                    name.as_str(),
                    "table"
                        | "body"
                        | "col"
                        | "colgroup"
                        | "html"
                        | "tbody"
                        | "td"
                        | "tfoot"
                        | "th"
                        | "thead"
                        | "tr"
                ) =>
            {
                if name == "table" && self.has_element_in_scope("caption", Scope::Table) {
                    self.pop_until_including("caption");
                    self.clear_afe_to_last_marker();
                    self.mode = InsertionMode::InTable;
                    Action::Reprocess
                } else {
                    Action::Continue
                }
            }
            _ => self.in_body(token, tokenizer),
        }
    }

    fn in_column_group(&mut self, token: &Token) -> Action {
        match token {
            Token::Character(c) if c.is_ascii_whitespace() => {
                self.insert_character(*c);
                Action::Continue
            }
            Token::Comment(text) => {
                self.insert_comment(text);
                Action::Continue
            }
            Token::Doctype { .. } => Action::Continue,
            Token::StartTag { name, .. } if name == "html" => self.in_body_stub_html(token),
            Token::StartTag {
                name, attributes, ..
            } if name == "col" => {
                self.insert_html_element(name, attributes.clone());
                self.open_elements.pop();
                Action::Continue
            }
            Token::EndTag { name } if name == "colgroup" => {
                if self.current_tag() == "colgroup" {
                    self.open_elements.pop();
                    self.mode = InsertionMode::InTable;
                }
                Action::Continue
            }
            Token::EndTag { name } if name == "col" => Action::Continue,
            _ => {
                if self.current_tag() == "colgroup" {
                    self.open_elements.pop();
                    self.mode = InsertionMode::InTable;
                    Action::Reprocess
                } else {
                    Action::Continue
                }
            }
        }
    }

    fn in_table_body(&mut self, token: &Token, tokenizer: &mut Tokenizer) -> Action {
        match token {
            Token::StartTag {
                name, attributes, ..
            } if name == "tr" => {
                self.clear_stack_to_table_body_context();
                self.insert_html_element(name, attributes.clone());
                self.mode = InsertionMode::InRow;
                Action::Continue
            }
            Token::StartTag { name, .. } if matches!(name.as_str(), "th" | "td") => {
                self.clear_stack_to_table_body_context();
                self.insert_html_element("tr", vec![]);
                self.mode = InsertionMode::InRow;
                Action::Reprocess
            }
            Token::EndTag { name } if matches!(name.as_str(), "tbody" | "tfoot" | "thead") => {
                if self.has_element_in_scope(name, Scope::Table) {
                    self.clear_stack_to_table_body_context();
                    self.open_elements.pop();
                    self.mode = InsertionMode::InTable;
                }
                Action::Continue
            }
            Token::StartTag { name, .. }
                if matches!(
                    name.as_str(),
                    "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead"
                ) =>
            {
                if self.has_element_in_scope("tbody", Scope::Table)
                    || self.has_element_in_scope("thead", Scope::Table)
                    || self.has_element_in_scope("tfoot", Scope::Table)
                {
                    self.clear_stack_to_table_body_context();
                    self.open_elements.pop();
                    self.mode = InsertionMode::InTable;
                    Action::Reprocess
                } else {
                    Action::Continue
                }
            }
            Token::EndTag { name } if name == "table" => {
                if self.has_element_in_scope("tbody", Scope::Table)
                    || self.has_element_in_scope("thead", Scope::Table)
                    || self.has_element_in_scope("tfoot", Scope::Table)
                {
                    self.clear_stack_to_table_body_context();
                    self.open_elements.pop();
                    self.mode = InsertionMode::InTable;
                    Action::Reprocess
                } else {
                    Action::Continue
                }
            }
            Token::EndTag { name }
                if matches!(
                    name.as_str(),
                    "body" | "caption" | "col" | "colgroup" | "html" | "td" | "th" | "tr"
                ) =>
            {
                Action::Continue
            }
            _ => self.in_table(token, tokenizer),
        }
    }

    fn clear_stack_to_table_body_context(&mut self) {
        while !matches!(
            self.current_tag().as_str(),
            "tbody" | "tfoot" | "thead" | "template" | "html"
        ) {
            self.open_elements.pop();
        }
    }

    fn clear_stack_to_row_context(&mut self) {
        while !matches!(self.current_tag().as_str(), "tr" | "template" | "html") {
            self.open_elements.pop();
        }
    }

    fn in_row(&mut self, token: &Token, tokenizer: &mut Tokenizer) -> Action {
        match token {
            Token::StartTag {
                name, attributes, ..
            } if matches!(name.as_str(), "th" | "td") => {
                self.clear_stack_to_row_context();
                self.insert_html_element(name, attributes.clone());
                self.mode = InsertionMode::InCell;
                self.afe.push(Afe::Marker);
                Action::Continue
            }
            Token::EndTag { name } if name == "tr" => {
                if self.has_element_in_scope("tr", Scope::Table) {
                    self.clear_stack_to_row_context();
                    self.open_elements.pop();
                    self.mode = InsertionMode::InTableBody;
                }
                Action::Continue
            }
            Token::StartTag { name, .. }
                if matches!(
                    name.as_str(),
                    "caption" | "col" | "colgroup" | "tbody" | "tfoot" | "thead" | "tr"
                ) =>
            {
                if self.has_element_in_scope("tr", Scope::Table) {
                    self.clear_stack_to_row_context();
                    self.open_elements.pop();
                    self.mode = InsertionMode::InTableBody;
                    Action::Reprocess
                } else {
                    Action::Continue
                }
            }
            Token::EndTag { name } if name == "table" => {
                if self.has_element_in_scope("tr", Scope::Table) {
                    self.clear_stack_to_row_context();
                    self.open_elements.pop();
                    self.mode = InsertionMode::InTableBody;
                    Action::Reprocess
                } else {
                    Action::Continue
                }
            }
            Token::EndTag { name } if matches!(name.as_str(), "tbody" | "tfoot" | "thead") => {
                if self.has_element_in_scope(name, Scope::Table)
                    && self.has_element_in_scope("tr", Scope::Table)
                {
                    self.clear_stack_to_row_context();
                    self.open_elements.pop();
                    self.mode = InsertionMode::InTableBody;
                    Action::Reprocess
                } else {
                    Action::Continue
                }
            }
            Token::EndTag { name }
                if matches!(
                    name.as_str(),
                    "body" | "caption" | "col" | "colgroup" | "html" | "td" | "th"
                ) =>
            {
                Action::Continue
            }
            _ => self.in_table(token, tokenizer),
        }
    }

    fn in_cell(&mut self, token: &Token, tokenizer: &mut Tokenizer) -> Action {
        match token {
            Token::EndTag { name } if matches!(name.as_str(), "td" | "th") => {
                if self.has_element_in_scope(name, Scope::Table) {
                    self.generate_implied_end_tags(None);
                    self.pop_until_including(name);
                    self.clear_afe_to_last_marker();
                    self.mode = InsertionMode::InRow;
                }
                Action::Continue
            }
            Token::StartTag { name, .. }
                if matches!(
                    name.as_str(),
                    "caption"
                        | "col"
                        | "colgroup"
                        | "tbody"
                        | "td"
                        | "tfoot"
                        | "th"
                        | "thead"
                        | "tr"
                ) =>
            {
                if self.has_element_in_scope("td", Scope::Table)
                    || self.has_element_in_scope("th", Scope::Table)
                {
                    self.close_cell();
                    Action::Reprocess
                } else {
                    Action::Continue
                }
            }
            Token::EndTag { name }
                if matches!(
                    name.as_str(),
                    "body" | "caption" | "col" | "colgroup" | "html"
                ) =>
            {
                Action::Continue
            }
            Token::EndTag { name }
                if matches!(name.as_str(), "table" | "tbody" | "tfoot" | "thead" | "tr") =>
            {
                if self.has_element_in_scope(name, Scope::Table) {
                    self.close_cell();
                    Action::Reprocess
                } else {
                    Action::Continue
                }
            }
            _ => self.in_body(token, tokenizer),
        }
    }

    fn close_cell(&mut self) {
        self.generate_implied_end_tags(None);
        if matches!(self.current_tag().as_str(), "td" | "th") {
            self.open_elements.pop();
        }
        self.clear_afe_to_last_marker();
        self.mode = InsertionMode::InRow;
    }

    // ---------------- Select ----------------
    fn in_select(&mut self, token: &Token) -> Action {
        match token {
            Token::Character('\0') => Action::Continue,
            Token::Character(c) => {
                self.insert_character(*c);
                Action::Continue
            }
            Token::Comment(text) => {
                self.insert_comment(text);
                Action::Continue
            }
            Token::Doctype { .. } => Action::Continue,
            Token::StartTag { name, .. } if name == "html" => self.in_body_stub_html(token),
            Token::StartTag { name, .. } if name == "option" => {
                if self.current_tag() == "option" {
                    self.open_elements.pop();
                }
                self.insert_html_element(name, vec![]);
                Action::Continue
            }
            Token::StartTag { name, .. } if name == "optgroup" => {
                if self.current_tag() == "option" {
                    self.open_elements.pop();
                }
                if self.current_tag() == "optgroup" {
                    self.open_elements.pop();
                }
                self.insert_html_element(name, vec![]);
                Action::Continue
            }
            Token::StartTag { name, .. } if name == "hr" => {
                // Spec: unlike most content in a <select>, <hr> is
                // genuinely inserted (as a void element), after closing
                // any open option/optgroup -- not ignored.
                if self.current_tag() == "option" {
                    self.open_elements.pop();
                }
                if self.current_tag() == "optgroup" {
                    self.open_elements.pop();
                }
                self.insert_html_element(name, vec![]);
                self.open_elements.pop();
                Action::Continue
            }
            Token::EndTag { name } if name == "optgroup" => {
                if self.current_tag() == "option" && self.open_elements.len() >= 2 {
                    let parent_tag =
                        self.tag_name(self.open_elements[self.open_elements.len() - 2]);
                    if parent_tag == "optgroup" {
                        self.open_elements.pop();
                    }
                }
                if self.current_tag() == "optgroup" {
                    self.open_elements.pop();
                }
                Action::Continue
            }
            Token::EndTag { name } if name == "option" => {
                if self.current_tag() == "option" {
                    self.open_elements.pop();
                }
                Action::Continue
            }
            Token::EndTag { name } if name == "select" => {
                if self.has_element_in_scope("select", Scope::Select) {
                    self.pop_until_including("select");
                    self.reset_insertion_mode();
                }
                Action::Continue
            }
            Token::StartTag { name, .. } if name == "select" => {
                if self.has_element_in_scope("select", Scope::Select) {
                    self.pop_until_including("select");
                    self.reset_insertion_mode();
                }
                Action::Continue
            }
            Token::StartTag { name, .. }
                if matches!(name.as_str(), "input" | "keygen" | "textarea") =>
            {
                if self.has_element_in_scope("select", Scope::Select) {
                    self.pop_until_including("select");
                    self.reset_insertion_mode();
                }
                Action::Reprocess
            }
            Token::Eof => Action::Continue,
            _ => Action::Continue,
        }
    }

    // ---------------- Template ----------------
    fn in_template(&mut self, token: &Token, tokenizer: &mut Tokenizer) -> Action {
        match token {
            Token::Eof => {
                if self
                    .open_elements
                    .iter()
                    .any(|&id| self.tag_name(id) == "template")
                {
                    self.pop_until_including("template");
                    self.clear_afe_to_last_marker();
                    self.template_modes.pop();
                    self.mode = self
                        .template_modes
                        .last()
                        .copied()
                        .unwrap_or(InsertionMode::InBody);
                    Action::Reprocess
                } else {
                    self.done = true;
                    Action::Continue
                }
            }
            _ => self.in_body(token, tokenizer),
        }
    }

    // ---------------- AfterBody ----------------
    fn in_after_body(&mut self, token: &Token) -> Action {
        match token {
            Token::Character(c) if c.is_ascii_whitespace() => self.in_body_delegate_char(*c),
            Token::Comment(text) => {
                // Appended as a child of the <html> element, not the body.
                let html = self.open_elements[0];
                self.doc.append(html, NodeData::Comment(text.to_string()));
                Action::Continue
            }
            Token::Doctype { .. } => Action::Continue,
            Token::StartTag { name, .. } if name == "html" => self.in_body_stub_html(token),
            Token::EndTag { name } if name == "html" => {
                self.mode = InsertionMode::AfterAfterBody;
                Action::Continue
            }
            Token::Eof => {
                self.done = true;
                Action::Continue
            }
            _ => {
                self.mode = InsertionMode::InBody;
                Action::Reprocess
            }
        }
    }

    fn in_body_delegate_char(&mut self, c: char) -> Action {
        self.reconstruct_active_formatting_elements();
        self.insert_character(c);
        Action::Continue
    }

    // ---------------- AfterAfterBody ----------------
    fn in_after_after_body(&mut self, token: &Token) -> Action {
        match token {
            Token::Comment(text) => {
                self.insert_comment_at_document(text);
                Action::Continue
            }
            Token::Doctype { .. } => Action::Continue,
            Token::Character(c) if c.is_ascii_whitespace() => self.in_body_delegate_char(*c),
            Token::StartTag { name, .. } if name == "html" => self.in_body_stub_html(token),
            Token::Eof => {
                self.done = true;
                Action::Continue
            }
            _ => {
                self.mode = InsertionMode::InBody;
                Action::Reprocess
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dump(input: &str) -> String {
        parse_document(input).to_string()
    }

    #[test]
    fn minimal_document() {
        let out = dump("<!DOCTYPE html><html><head></head><body>hi</body></html>");
        assert_eq!(
            out,
            "#document\n  <!DOCTYPE html>\n  <html>\n    <head>\n    <body>\n      \"hi\"\n"
        );
    }

    #[test]
    fn implicit_head_and_body() {
        let out = dump("Test");
        assert_eq!(
            out,
            "#document\n  <html>\n    <head>\n    <body>\n      \"Test\"\n"
        );
    }

    #[test]
    fn paragraph_auto_close() {
        let out = dump("<p>One<p>Two");
        assert_eq!(
            out,
            "#document\n  <html>\n    <head>\n    <body>\n      <p>\n        \"One\"\n      <p>\n        \"Two\"\n"
        );
    }

    #[test]
    fn formatting_element_survives_across_block() {
        // A simple adoption-agency-triggering case: <b> left open across a
        // <div>, per html5lib's adoption01.dat "misnesting" style cases.
        let out = dump("<b>1<div>2</b>3</div>");
        // Just check it doesn't panic and produces two <b> elements
        // (the reparented clone inside the div), which is the hallmark of
        // adoption agency actually having run rather than being a no-op.
        assert_eq!(out.matches("<b>").count(), 2);
    }
}

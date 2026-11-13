//! A parser for XML 1.0 with namespaces that builds a read-only tree.
//!
//! It is written for the XML office documents are made of. Text and
//! attribute values point into the source wherever they need no decoding;
//! comments and processing instructions are skipped; a document type
//! declaration is refused, as Word refuses it.

use std::borrow::Cow;
use std::fmt;

const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
const NONE: u32 = u32::MAX;

/// Why a text is not well-formed XML, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// The text ends inside a tag, or before every element is closed.
    UnexpectedEnd,
    /// Something other than what the grammar allows at this point.
    Expected(&'static str),
    /// An end tag that does not close the element that is open.
    MismatchedTag {
        open: String,
        close: String,
    },
    DuplicateAttribute(String),
    /// A prefix with no namespace declaration in scope.
    UnknownPrefix(String),
    /// An entity other than the five predefined ones, or a reference to a character XML does not allow.
    UnknownEntity(String),
    /// A character XML does not allow, such as a control character.
    InvalidCharacter,
    /// A document type declaration.
    Dtd,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ErrorKind::UnexpectedEnd => f.write_str("the text ends too early")?,
            ErrorKind::Expected(what) => write!(f, "expected {what}")?,
            ErrorKind::MismatchedTag { open, close } => write!(f, "expected the end of '{open}', not of '{close}'")?,
            ErrorKind::DuplicateAttribute(name) => write!(f, "attribute '{name}' is given twice")?,
            ErrorKind::UnknownPrefix(prefix) => write!(f, "no namespace is declared for the prefix '{prefix}'")?,
            ErrorKind::UnknownEntity(name) => write!(f, "unknown entity '&{name};'")?,
            ErrorKind::InvalidCharacter => f.write_str("a character that XML does not allow")?,
            ErrorKind::Dtd => f.write_str("a document type declaration is not allowed")?,
        }
        write!(f, " at {}:{}", self.line, self.column)
    }
}

impl std::error::Error for Error {}

impl fmt::Debug for Document<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Document of {} nodes", self.nodes.len() - 1)
    }
}

pub struct Document<'i> {
    /// In document order, so that a node's descendants directly follow it.
    nodes: Vec<NodeData<'i>>,
    attributes: Vec<AttributeData<'i>>,
    /// Every namespace declaration, each linked to the one in scope before it.
    scopes: Vec<Scope<'i>>,
}

struct NodeData<'i> {
    kind: Kind<'i>,
    /// One past the node's last descendant.
    end: u32,
}

enum Kind<'i> {
    Root,
    Element {
        name: &'i str,
        /// The declaration that gives the element its namespace, or `NONE`.
        namespace: u32,
        attributes: (u32, u32),
        /// The innermost declaration in scope.
        scope: u32,
    },
    Text(Cow<'i, str>),
}

struct AttributeData<'i> {
    name: &'i str,
    namespace: u32,
    value: Cow<'i, str>,
}

struct Scope<'i> {
    prefix: &'i str,
    uri: Cow<'i, str>,
    outer: u32,
}

impl<'i> Document<'i> {
    pub fn parse(text: &'i str) -> Result<Self, Error> {
        let mut parser = Parser {
            text,
            bytes: text.as_bytes(),
            pos: 0,
            document: Document {
                nodes: vec![NodeData { kind: Kind::Root, end: 0 }],
                attributes: Vec::new(),
                scopes: vec![Scope { prefix: "xml", uri: Cow::Borrowed(XML_NS), outer: NONE }],
            },
            scope: 0,
            resolved: Vec::new(),
            open: Vec::new(),
            text_node: None,
            pending: Vec::new(),
        };
        match parser.run() {
            Ok(()) => {
                parser.document.nodes[0].end = parser.document.nodes.len() as u32;
                Ok(parser.document)
            }
            Err((kind, at)) => {
                let before = &text[..floor_char_boundary(text, at.min(text.len()))];
                let line = before.bytes().filter(|&b| b == b'\n').count() as u32 + 1;
                let column = before.rsplit('\n').next().map_or(0, |last| last.chars().count()) as u32 + 1;
                Err(Error { kind, line, column })
            }
        }
    }

    /// The document's one element at the top.
    pub fn root_element(&self) -> Node<'_, 'i> {
        Node { document: self, index: 1 }
    }

    /// Every node, in document order.
    pub fn descendants(&self) -> impl Iterator<Item = Node<'_, 'i>> {
        (1..self.nodes.len() as u32).map(move |index| Node { document: self, index })
    }
}

fn floor_char_boundary(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// A name to look up: a bare local name, which only matches where there is
/// no namespace, or a namespace and a local name.
pub trait Named {
    fn parts(&self) -> (Option<&str>, &str);
}

impl Named for &str {
    fn parts(&self) -> (Option<&str>, &str) {
        (None, *self)
    }
}

impl Named for (&str, &str) {
    fn parts(&self) -> (Option<&str>, &str) {
        (Some(self.0), self.1)
    }
}

/// An element or a run of text. Cheap to copy: a reference and an index.
#[derive(Clone, Copy)]
pub struct Node<'a, 'i> {
    document: &'a Document<'i>,
    index: u32,
}

/// An element's name: its local part and the namespace it is in.
#[derive(Clone, Copy)]
pub struct Name<'a, 'i> {
    name: &'i str,
    namespace: Option<&'a str>,
}

impl<'a, 'i> Name<'a, 'i> {
    pub fn name(&self) -> &'i str {
        self.name
    }

    pub fn namespace(&self) -> Option<&'a str> {
        self.namespace
    }
}

#[derive(Clone, Copy)]
pub struct Attribute<'a, 'i> {
    document: &'a Document<'i>,
    data: &'a AttributeData<'i>,
}

impl<'a, 'i> Attribute<'a, 'i> {
    pub fn name(&self) -> &'i str {
        self.data.name
    }

    pub fn namespace(&self) -> Option<&'a str> {
        self.document.uri(self.data.namespace)
    }

    pub fn value(&self) -> &'a str {
        &self.data.value
    }
}

impl<'i> Document<'i> {
    fn uri(&self, scope: u32) -> Option<&str> {
        self.scopes.get(scope as usize).map(|scope| &*scope.uri)
    }
}

impl<'a, 'i> Node<'a, 'i> {
    fn data(&self) -> &'a NodeData<'i> {
        &self.document.nodes[self.index as usize]
    }

    pub fn is_element(&self) -> bool {
        matches!(self.data().kind, Kind::Element { .. })
    }

    pub fn is_text(&self) -> bool {
        matches!(self.data().kind, Kind::Text(_))
    }

    /// The name of an element; empty for text.
    pub fn tag_name(&self) -> Name<'a, 'i> {
        match self.data().kind {
            Kind::Element { name, namespace, .. } => Name { name, namespace: self.document.uri(namespace) },
            _ => Name { name: "", namespace: None },
        }
    }

    /// Whether this is the element with that namespace and local name.
    pub fn has_tag_name(&self, name: (&str, &str)) -> bool {
        let tag = self.tag_name();
        self.is_element() && tag.name == name.1 && tag.namespace == Some(name.0)
    }

    pub fn attributes(&self) -> impl Iterator<Item = Attribute<'a, 'i>> {
        let (start, end) = match self.data().kind {
            Kind::Element { attributes, .. } => attributes,
            _ => (0, 0),
        };
        let document = self.document;
        document.attributes[start as usize..end as usize].iter().map(move |data| Attribute { document, data })
    }

    pub fn attribute(&self, name: impl Named) -> Option<&'a str> {
        let (namespace, local) = name.parts();
        self.attributes().find(|a| a.name() == local && a.namespace() == namespace).map(|a| a.value())
    }

    /// The elements and runs of text directly inside this node.
    pub fn children(&self) -> Children<'a, 'i> {
        Children { document: self.document, next: self.index + 1, end: self.data().end }
    }

    /// The text of a text node. For an element, the text it starts with, if it starts with any.
    pub fn text(&self) -> Option<&'a str> {
        match &self.data().kind {
            Kind::Text(text) => Some(text),
            Kind::Element { .. } => self.children().next().filter(Node::is_text).and_then(|first| first.text()),
            Kind::Root => None,
        }
    }

    /// The namespace a prefix stands for at this element; `None` asks for the default namespace.
    pub fn lookup_namespace_uri(&self, prefix: Option<&str>) -> Option<&'a str> {
        let Kind::Element { scope, .. } = self.data().kind else {
            return None;
        };
        let prefix = prefix.unwrap_or_default();
        let mut at = scope;
        while let Some(scope) = self.document.scopes.get(at as usize) {
            if scope.prefix == prefix {
                return Some(&*scope.uri).filter(|uri| !uri.is_empty());
            }
            at = scope.outer;
        }
        None
    }
}

pub struct Children<'a, 'i> {
    document: &'a Document<'i>,
    next: u32,
    end: u32,
}

impl<'a, 'i> Iterator for Children<'a, 'i> {
    type Item = Node<'a, 'i>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.end {
            return None;
        }
        let node = Node { document: self.document, index: self.next };
        self.next = node.data().end;
        Some(node)
    }
}

type Failure = (ErrorKind, usize);

struct Parser<'i> {
    text: &'i str,
    bytes: &'i [u8],
    pos: usize,
    document: Document<'i>,
    /// The innermost namespace declaration in scope.
    scope: u32,
    /// Prefixes already looked up under the current `scope`.
    resolved: Vec<(&'i str, u32)>,
    /// The elements not yet closed: node index, name as written, and the scope outside.
    open: Vec<(u32, &'i str, u32)>,
    /// The text node that further text of the current element is added to.
    text_node: Option<u32>,
    /// Attributes of the tag being read: name, where it stands, and where its value starts and ends.
    pending: Vec<(&'i str, usize, usize, usize)>,
}

fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

impl<'i> Parser<'i> {
    fn run(&mut self) -> Result<(), Failure> {
        self.pos = if self.text.starts_with('\u{FEFF}') { 3 } else { 0 };
        let mut seen_root = false;
        loop {
            let start = self.pos;
            let Some(offset) = self.bytes[start..].iter().position(|&b| b == b'<') else {
                self.pos = self.bytes.len();
                self.characters(start, self.bytes.len())?;
                break;
            };
            self.pos = start + offset;
            self.characters(start, self.pos)?;
            let rest = &self.bytes[self.pos..];
            if rest.starts_with(b"</") {
                self.end_tag()?;
            } else if rest.starts_with(b"<!--") {
                self.skip_past(4, "-->")?;
            } else if rest.starts_with(b"<![CDATA[") {
                let from = self.pos + 9;
                self.skip_past(9, "]]>")?;
                if self.open.is_empty() {
                    return Err((ErrorKind::Expected("an element"), from));
                }
                self.push_text(from, self.pos - 3, false)?;
            } else if rest.starts_with(b"<?") {
                self.skip_past(2, "?>")?;
            } else if rest.starts_with(b"<!") {
                return Err((ErrorKind::Dtd, self.pos));
            } else {
                if self.open.is_empty() && std::mem::replace(&mut seen_root, true) {
                    return Err((ErrorKind::Expected("the end of the document"), self.pos));
                }
                self.start_tag()?;
            }
        }
        match (self.open.is_empty(), seen_root) {
            (true, true) => Ok(()),
            (false, _) => Err((ErrorKind::UnexpectedEnd, self.bytes.len())),
            (true, false) => Err((ErrorKind::Expected("an element"), self.bytes.len())),
        }
    }

    fn skip_past(&mut self, opening: usize, closing: &str) -> Result<(), Failure> {
        match self.text[self.pos + opening..].find(closing) {
            Some(at) => {
                self.pos += opening + at + closing.len();
                Ok(())
            }
            None => Err((ErrorKind::UnexpectedEnd, self.bytes.len())),
        }
    }

    /// Text between two tags. Outside the top element only white space may stand.
    fn characters(&mut self, start: usize, end: usize) -> Result<(), Failure> {
        if start == end {
            return Ok(());
        }
        if self.open.is_empty() {
            return match self.bytes[start..end].iter().position(|&b| !is_space(b)) {
                Some(at) => Err((ErrorKind::Expected("an element"), start + at)),
                None => Ok(()),
            };
        }
        self.push_text(start, end, true)
    }

    fn push_text(&mut self, start: usize, end: usize, references: bool) -> Result<(), Failure> {
        let text = decode(self.text, start, end, references, false)?;
        match self.text_node {
            Some(index) => {
                let Kind::Text(existing) = &mut self.document.nodes[index as usize].kind else { unreachable!() };
                existing.to_mut().push_str(&text);
            }
            None => {
                let index = self.document.nodes.len() as u32;
                self.document.nodes.push(NodeData { kind: Kind::Text(text), end: index + 1 });
                self.text_node = Some(index);
            }
        }
        Ok(())
    }

    /// Reads a name and returns it, leaving `pos` after it.
    fn name(&mut self) -> Result<&'i str, Failure> {
        let start = self.pos;
        let length = self.bytes[start..]
            .iter()
            .position(|&b| is_space(b) || matches!(b, b'/' | b'>' | b'=' | b'<' | b'"' | b'\''))
            .unwrap_or(self.bytes.len() - start);
        let name = &self.text[start..start + length];
        let starts_well =
            name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_' || c == ':' || !c.is_ascii());
        if !starts_well {
            return Err((ErrorKind::Expected("a name"), start));
        }
        self.pos += length;
        Ok(name)
    }

    fn skip_space(&mut self) {
        while self.bytes.get(self.pos).copied().is_some_and(is_space) {
            self.pos += 1;
        }
    }

    fn start_tag(&mut self) -> Result<(), Failure> {
        let at = self.pos;
        self.pos += 1;
        let name = self.name()?;
        self.pending.clear();
        let empty = loop {
            let spaced = self.bytes.get(self.pos).copied().is_some_and(is_space);
            self.skip_space();
            match self.bytes.get(self.pos) {
                None => return Err((ErrorKind::UnexpectedEnd, self.pos)),
                Some(b'>') => {
                    self.pos += 1;
                    break false;
                }
                Some(b'/') => {
                    if self.bytes.get(self.pos + 1) != Some(&b'>') {
                        return Err((ErrorKind::Expected("'>'"), self.pos + 1));
                    }
                    self.pos += 2;
                    break true;
                }
                Some(_) if !spaced => return Err((ErrorKind::Expected("a space before the attribute"), self.pos)),
                Some(_) => {
                    let position = self.pos;
                    let attribute = self.name()?;
                    self.skip_space();
                    if self.bytes.get(self.pos) != Some(&b'=') {
                        return Err((ErrorKind::Expected("'='"), self.pos));
                    }
                    self.pos += 1;
                    self.skip_space();
                    let quote = match self.bytes.get(self.pos) {
                        Some(&quote @ (b'"' | b'\'')) => quote,
                        _ => return Err((ErrorKind::Expected("a quoted value"), self.pos)),
                    };
                    let start = self.pos + 1;
                    let Some(length) = self.bytes[start..].iter().position(|&b| b == quote) else {
                        return Err((ErrorKind::UnexpectedEnd, self.bytes.len()));
                    };
                    self.pos = start + length + 1;
                    self.pending.push((attribute, position, start, start + length));
                }
            }
        };

        // Declarations first: they are in force for the tag they are written on.
        let outside = self.scope;
        for &(attribute, _, start, end) in &self.pending {
            let prefix = match attribute.strip_prefix("xmlns") {
                Some("") => "",
                Some(rest) => match rest.strip_prefix(':') {
                    Some(prefix) => prefix,
                    None => continue,
                },
                None => continue,
            };
            let uri = decode(self.text, start, end, true, true)?;
            self.document.scopes.push(Scope { prefix, uri, outer: self.scope });
            self.scope = self.document.scopes.len() as u32 - 1;
            self.resolved.clear();
        }

        let (prefix, local) = name.split_once(':').unwrap_or(("", name));
        let namespace =
            self.resolve(prefix, true).ok_or_else(|| (ErrorKind::UnknownPrefix(prefix.to_owned()), at + 1))?;
        let first = self.document.attributes.len() as u32;
        for i in 0..self.pending.len() {
            let (attribute, position, start, end) = self.pending[i];
            if attribute == "xmlns" || attribute.starts_with("xmlns:") {
                continue;
            }
            let (prefix, local) = attribute.split_once(':').unwrap_or(("", attribute));
            // An attribute without a prefix is in no namespace, whatever the default is.
            let namespace = match prefix {
                "" => NONE,
                _ => self
                    .resolve(prefix, false)
                    .ok_or_else(|| (ErrorKind::UnknownPrefix(prefix.to_owned()), position))?,
            };
            let uri = self.document.uri(namespace);
            let twice = self.document.attributes[first as usize..]
                .iter()
                .any(|other| other.name == local && self.document.uri(other.namespace) == uri);
            if twice {
                return Err((ErrorKind::DuplicateAttribute(attribute.to_owned()), position));
            }
            let value = decode(self.text, start, end, true, true)?;
            self.document.attributes.push(AttributeData { name: local, namespace, value });
        }
        let last = self.document.attributes.len() as u32;

        let index = self.document.nodes.len() as u32;
        let kind = Kind::Element { name: local, namespace, attributes: (first, last), scope: self.scope };
        self.document.nodes.push(NodeData { kind, end: index + 1 });
        self.text_node = None;
        if empty {
            self.leave(outside);
        } else {
            self.open.push((index, name, outside));
        }
        Ok(())
    }

    fn end_tag(&mut self) -> Result<(), Failure> {
        let at = self.pos;
        self.pos += 2;
        let name = self.name()?;
        self.skip_space();
        if self.bytes.get(self.pos) != Some(&b'>') {
            let kind = if self.pos < self.bytes.len() { ErrorKind::Expected("'>'") } else { ErrorKind::UnexpectedEnd };
            return Err((kind, self.pos));
        }
        self.pos += 1;
        let Some((index, open, outside)) = self.open.pop() else {
            return Err((ErrorKind::Expected("an element"), at));
        };
        if open != name {
            return Err((ErrorKind::MismatchedTag { open: open.to_owned(), close: name.to_owned() }, at));
        }
        self.document.nodes[index as usize].end = self.document.nodes.len() as u32;
        self.text_node = None;
        self.leave(outside);
        Ok(())
    }

    /// Puts the namespace scope back to what it was outside an element.
    fn leave(&mut self, outside: u32) {
        if self.scope != outside {
            self.scope = outside;
            self.resolved.clear();
        }
    }

    /// The declaration a prefix refers to. With no prefix, an element takes
    /// the default namespace if there is one; `NONE` when there is not.
    fn resolve(&mut self, prefix: &'i str, element: bool) -> Option<u32> {
        if let Some(&(_, found)) = self.resolved.iter().find(|(known, _)| *known == prefix) {
            return Some(found);
        }
        let mut at = self.scope;
        let found = loop {
            match self.document.scopes.get(at as usize) {
                Some(scope) if scope.prefix == prefix => break if scope.uri.is_empty() { NONE } else { at },
                Some(scope) => at = scope.outer,
                None if prefix.is_empty() && element => break NONE,
                None => return None,
            }
        };
        // An undeclared prefix (`xmlns:p=""` is not allowed, but harmless) is as good as unknown.
        if found == NONE && !prefix.is_empty() {
            return None;
        }
        self.resolved.push((prefix, found));
        Some(found)
    }
}

/// Turns the source text of character data or of an attribute value into
/// what it stands for: references resolved, line ends made uniform and, in
/// an attribute, white space turned into spaces.
fn decode(text: &str, start: usize, end: usize, references: bool, attribute: bool) -> Result<Cow<'_, str>, Failure> {
    let bytes = text.as_bytes();
    // U+FFFE and U+FFFF are the two characters above the controls that XML leaves out.
    let mut rest = start;
    while let Some(offset) = bytes[rest..end].iter().position(|&b| b == 0xEF) {
        rest += offset + 1;
        if bytes[rest..end].starts_with(&[0xBF, 0xBE]) || bytes[rest..end].starts_with(&[0xBF, 0xBF]) {
            return Err((ErrorKind::InvalidCharacter, rest - 1));
        }
    }
    // Line feeds and tabs stand for themselves in text, for a space in an attribute.
    let special = |b: u8| match b {
        b'\n' | b'\t' => attribute,
        _ => b < 0x20 || b == b'&' || b == b'<',
    };
    let Some(first) = bytes[start..end].iter().position(|&b| special(b)) else {
        return Ok(Cow::Borrowed(&text[start..end]));
    };
    let mut out = String::with_capacity(end - start);
    out.push_str(&text[start..start + first]);
    let mut at = start + first;
    while at < end {
        let byte = bytes[at];
        let from = at;
        at += 1;
        match byte {
            b'\r' | b'\n' | b'\t' => {
                if byte == b'\r' && at < end && bytes[at] == b'\n' {
                    at += 1;
                }
                out.push(match byte {
                    _ if attribute => ' ',
                    b'\t' => '\t',
                    _ => '\n',
                });
            }
            0..0x20 => return Err((ErrorKind::InvalidCharacter, from)),
            b'<' if attribute => return Err((ErrorKind::Expected("no '<' in an attribute value"), from)),
            b'&' if references => {
                let Some(length) = bytes[at..end].iter().position(|&b| b == b';') else {
                    return Err((ErrorKind::Expected("';' to end the reference"), from));
                };
                let name = &text[at..at + length];
                at += length + 1;
                out.push(match name {
                    "lt" => '<',
                    "gt" => '>',
                    "amp" => '&',
                    "apos" => '\'',
                    "quot" => '"',
                    _ => {
                        let code = match name.strip_prefix("#x") {
                            Some(hex) => u32::from_str_radix(hex, 16).ok(),
                            None => name.strip_prefix('#').and_then(|decimal| decimal.parse().ok()),
                        };
                        code.and_then(char::from_u32)
                            .filter(|&c| is_xml_char(c))
                            .ok_or_else(|| (ErrorKind::UnknownEntity(name.to_owned()), from))?
                    }
                });
            }
            _ => {
                // Copy up to the next byte that needs a look.
                let length = bytes[at..end].iter().position(|&b| special(b)).unwrap_or(end - at);
                out.push_str(&text[from..at + length]);
                at += length;
            }
        }
    }
    Ok(Cow::Owned(out))
}

/// Writes `text` so that it can stand as character data or in a quoted
/// attribute value and reads back as itself: the markup characters become
/// references, as do tabs and line ends, which an attribute would otherwise
/// turn into spaces. Characters XML cannot hold at all are left out.
pub fn escape(text: &str) -> Cow<'_, str> {
    let plain = |c: char| !matches!(c, '&' | '<' | '>' | '"' | '\t' | '\n' | '\r') && is_xml_char(c);
    if text.chars().all(plain) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 16);
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            c if is_xml_char(c) => out.push(c),
            _ => {}
        }
    }
    Cow::Owned(out)
}

fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: &str = "urn:w";

    #[test]
    fn builds_the_tree_with_names_in_their_namespaces() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- a comment --><w:body xmlns:w="urn:w" xmlns="urn:default" w:id="1" plain="2"><w:p><w:r>one</w:r><r/></w:p>tail</w:body>
"#;
        let document = Document::parse(xml).unwrap();
        let body = document.root_element();
        assert_eq!((body.tag_name().name(), body.tag_name().namespace()), ("body", Some(W)));
        assert_eq!(body.attribute((W, "id")), Some("1"));
        assert_eq!(body.attribute("plain"), Some("2"));
        // A bare name only matches an attribute in no namespace, and the default namespace is not for attributes.
        assert_eq!(body.attribute("id"), None);
        assert_eq!(body.attribute(("urn:default", "plain")), None);
        assert_eq!(body.attributes().count(), 2);

        let children: Vec<_> = body.children().collect();
        assert_eq!(children.len(), 2);
        assert!(children[0].has_tag_name((W, "p")) && children[1].is_text());
        assert_eq!(children[1].text(), Some("tail"));
        let runs: Vec<_> = children[0].children().collect();
        assert_eq!(runs[0].text(), Some("one"));
        assert_eq!(runs[1].tag_name().namespace(), Some("urn:default"));
        assert_eq!(runs[1].lookup_namespace_uri(Some("w")), Some(W));
        assert_eq!(runs[1].lookup_namespace_uri(None), Some("urn:default"));
        assert_eq!(runs[1].lookup_namespace_uri(Some("x")), None);
        assert_eq!(document.descendants().filter(Node::is_element).count(), 4);
    }

    #[test]
    fn a_declaration_holds_only_inside_its_element() {
        let document = Document::parse(r#"<a><b xmlns="urn:b" xmlns:p="urn:p"><p:c/></b><d/></a>"#).unwrap();
        let names: Vec<_> = document.descendants().map(|n| (n.tag_name().name(), n.tag_name().namespace())).collect();
        assert_eq!(names, [("a", None), ("b", Some("urn:b")), ("c", Some("urn:p")), ("d", None)]);
        let error = Document::parse(r#"<a><b xmlns:p="urn:p"/><p:c/></a>"#).unwrap_err();
        assert_eq!(error.kind, ErrorKind::UnknownPrefix("p".to_owned()));
    }

    #[test]
    fn decodes_text_and_attribute_values() {
        let xml =
            "<a v=\"1 &lt; 2 &amp;&#x4E2D;&#20013;\ttab\r\nline\">x &gt; y<![CDATA[ <raw> & ]]>\r\nend&apos;&quot;</a>";
        let document = Document::parse(xml).unwrap();
        let a = document.root_element();
        assert_eq!(a.attribute("v"), Some("1 < 2 &中中 tab line"));
        // Text, a CDATA section and more text are one run; line ends become a single line feed.
        assert_eq!(a.children().count(), 1);
        assert_eq!(a.text(), Some("x > y <raw> & \nend'\""));
        // Text that needs no decoding is not copied.
        let document = Document::parse("<a>plain</a>").unwrap();
        assert!(matches!(document.nodes[2].kind, Kind::Text(Cow::Borrowed("plain"))));
    }

    #[test]
    fn says_what_is_wrong_and_where() {
        let kind = |xml: &str| Document::parse(xml).unwrap_err().kind;
        assert_eq!(kind("<a><b></a>"), ErrorKind::MismatchedTag { open: "b".into(), close: "a".into() });
        assert_eq!(kind("<a>"), ErrorKind::UnexpectedEnd);
        assert_eq!(kind("<a"), ErrorKind::UnexpectedEnd);
        assert_eq!(kind("<a x='1' x='2'/>"), ErrorKind::DuplicateAttribute("x".into()));
        assert_eq!(kind("<a xmlns:p='u' xmlns:q='u' p:x='1' q:x='2'/>"), ErrorKind::DuplicateAttribute("q:x".into()));
        assert_eq!(kind("<p:a/>"), ErrorKind::UnknownPrefix("p".into()));
        assert_eq!(kind("<a>&nbsp;</a>"), ErrorKind::UnknownEntity("nbsp".into()));
        assert_eq!(kind("<a>&#1;</a>"), ErrorKind::UnknownEntity("#1".into()));
        assert_eq!(kind("<a>\u{1}</a>"), ErrorKind::InvalidCharacter);
        assert_eq!(kind("<a>\u{FFFE}</a>"), ErrorKind::InvalidCharacter);
        assert_eq!(kind("<!DOCTYPE a><a/>"), ErrorKind::Dtd);
        assert_eq!(kind("<a/><b/>"), ErrorKind::Expected("the end of the document"));
        assert_eq!(kind("text<a/>"), ErrorKind::Expected("an element"));
        assert_eq!(kind(""), ErrorKind::Expected("an element"));
        assert_eq!(kind("<a x=1/>"), ErrorKind::Expected("a quoted value"));
        assert_eq!(kind("<a x='<'/>"), ErrorKind::Expected("no '<' in an attribute value"));

        let error = Document::parse("<a>\n  <b>\n</a>").unwrap_err();
        assert_eq!((error.line, error.column), (3, 1));
        assert_eq!(error.to_string(), "expected the end of 'b', not of 'a' at 3:1");
    }

    #[test]
    fn what_is_escaped_reads_back_as_itself() {
        let text = "a < b && \"c\" > 'd'\ttab\r\nline \u{1}中";
        assert_eq!(escape("plain 中文"), "plain 中文");
        assert_eq!(escape(text), "a &lt; b &amp;&amp; &quot;c&quot; &gt; 'd'&#9;tab&#13;&#10;line 中");
        let xml = format!("<a v=\"{0}\">{0}</a>", escape(text));
        let document = Document::parse(&xml).unwrap();
        let expected = text.replace('\u{1}', "");
        assert_eq!(document.root_element().attribute("v"), Some(expected.as_str()));
        assert_eq!(document.root_element().text(), Some(expected.as_str()));
    }

    #[test]
    fn survives_any_text() {
        let seed = "<a xmlns:w='u' w:b=\"c&amp;\"><!--x--><w:d>t&#x41;</w:d><![CDATA[z]]><?pi ?></a>";
        assert!(Document::parse(seed).is_ok());
        for end in 0..seed.len() {
            let _ = Document::parse(&seed[..end]);
        }
        for at in 0..seed.len() {
            for replacement in ["<", ">", "&", "\"", "'", "/", " ", ":", "=", "\u{1}", "中"] {
                let mut changed = seed.to_owned();
                changed.replace_range(at..at + 1, replacement);
                let _ = Document::parse(&changed);
            }
        }
    }
}

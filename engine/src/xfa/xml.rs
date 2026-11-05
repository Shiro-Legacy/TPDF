// SPDX-License-Identifier: MIT
//! A small XML reader and writer for the XDP packets: elements,
//! attributes, text, CDATA, comments and processing instructions (both
//! skipped), the five predefined entities and numeric character
//! references, namespace prefixes kept on names (`local` strips them).
//! Bounded by `limits::MAX_XML_ELEMENTS` and `MAX_XML_DEPTH`. Not a
//! validating parser: it reads what Designer and Acrobat write.
//! Line ends are read as `\n`; literal tabs and line ends in attribute
//! values as spaces. Whitespace between elements is kept as text, so a
//! value's content is what was written. An explicit stack, never
//! recursion, so depth costs no call stack.

use std::fmt;

use super::limits::{MAX_XML_DEPTH, MAX_XML_ELEMENTS};

/// An element's child.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XmlNode {
    Element(Element),
    /// Character data, entities decoded, whitespace as written.
    Text(String),
}

/// An element.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Element {
    /// The qualified name as written (`xfa:data`, `subform`).
    pub name: String,
    /// Attributes in document order, qualified names as written, values
    /// with entities decoded.
    pub attrs: Vec<(String, String)>,
    pub children: Vec<XmlNode>,
}

/// Why XML could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XmlError {
    /// Byte offset of the problem.
    pub offset: usize,
    pub message: &'static str,
}

impl fmt::Display for XmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.message, self.offset)
    }
}

impl Element {
    /// The name without its prefix (`data` for `xfa:data`).
    pub fn local(&self) -> &str {
        match self.name.rfind(':') {
            Some(i) => &self.name[i + 1..],
            None => &self.name,
        }
    }
    /// An attribute by its qualified name, or by local name when no
    /// qualified attribute matches.
    pub fn attr(&self, name: &str) -> Option<&str> {
        if let Some((_, v)) = self.attrs.iter().find(|(k, _)| k == name) {
            return Some(v);
        }
        self.attrs
            .iter()
            .find(|(k, _)| k.rsplit(':').next() == Some(name))
            .map(|(_, v)| v.as_str())
    }
    /// The child elements, in order.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|c| match c {
            XmlNode::Element(e) => Some(e),
            XmlNode::Text(_) => None,
        })
    }
    pub fn elements_mut(&mut self) -> impl Iterator<Item = &mut Element> {
        self.children.iter_mut().filter_map(|c| match c {
            XmlNode::Element(e) => Some(e),
            XmlNode::Text(_) => None,
        })
    }
    /// The first child element with this local name.
    pub fn child(&self, local: &str) -> Option<&Element> {
        self.elements().find(|e| e.local() == local)
    }
    pub fn child_mut(&mut self, local: &str) -> Option<&mut Element> {
        self.elements_mut().find(|e| e.local() == local)
    }
    /// Every child element with this local name, in order.
    pub fn children_named<'a>(&'a self, local: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.elements().filter(move |e| e.local() == local)
    }
    /// The element's direct text content, joined.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for c in &self.children {
            if let XmlNode::Text(t) = c {
                out.push_str(t);
            }
        }
        out
    }
    /// Replaces the element's content with one text node.
    pub fn set_text(&mut self, text: &str) {
        self.children.clear();
        if !text.is_empty() {
            self.children.push(XmlNode::Text(text.to_string()));
        }
    }
    /// The element's children re-serialized as XML (for `exData` rich
    /// text, which `richtext` reads from markup).
    pub fn inner_xml(&self) -> String {
        let mut out = String::new();
        for c in &self.children {
            write_node(c, &mut out);
        }
        out
    }
    /// Whether the element has any child element.
    pub fn has_elements(&self) -> bool {
        self.elements().next().is_some()
    }
}

/// Reads a document's root element. A leading XML declaration, comments
/// and processing instructions anywhere are skipped.
pub fn parse(text: &str) -> Result<Element, XmlError> {
    Parser {
        text,
        bytes: text.as_bytes(),
        pos: 0,
    }
    .run()
}

/// Writes `root` as XML with an `<?xml version="1.0" encoding="UTF-8"?>`
/// declaration. Attribute values and text are escaped.
pub fn serialize(root: &Element) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    write_element(root, &mut out);
    out
}

/// Writes `element` as XML without a declaration (a packet of an XDP
/// document, a fragment).
pub fn serialize_fragment(element: &Element) -> String {
    let mut out = String::new();
    write_element(element, &mut out);
    out
}

fn write_node(node: &XmlNode, out: &mut String) {
    match node {
        XmlNode::Text(t) => escape_text(t, out),
        XmlNode::Element(e) => write_element(e, out),
    }
}

/// Writes an element and its content with an explicit stack, so a deep
/// tree built in code cannot exhaust the call stack.
fn write_element(root: &Element, out: &mut String) {
    // (element, index of the next child to write)
    let mut stack: Vec<(&Element, usize)> = Vec::new();
    open_tag(root, out);
    if root.children.is_empty() {
        return;
    }
    stack.push((root, 0));
    while let Some((element, next)) = stack.last_mut() {
        let element: &Element = element;
        let Some(child) = element.children.get(*next) else {
            out.push_str("</");
            out.push_str(&element.name);
            out.push('>');
            stack.pop();
            continue;
        };
        *next += 1;
        match child {
            XmlNode::Text(t) => escape_text(t, out),
            XmlNode::Element(e) => {
                open_tag(e, out);
                if !e.children.is_empty() {
                    stack.push((e, 0));
                }
            }
        }
    }
}

/// `<name attrs>`, or `<name attrs/>` when the element is empty.
fn open_tag(e: &Element, out: &mut String) {
    out.push('<');
    out.push_str(&e.name);
    for (k, v) in &e.attrs {
        out.push(' ');
        out.push_str(k);
        out.push_str("=\"");
        escape_attr(v, out);
        out.push('"');
    }
    out.push_str(if e.children.is_empty() { "/>" } else { ">" });
}

struct Parser<'t> {
    text: &'t str,
    bytes: &'t [u8],
    pos: usize,
}

fn err(offset: usize, message: &'static str) -> XmlError {
    XmlError { offset, message }
}

impl<'t> Parser<'t> {
    fn starts(&self, s: &[u8]) -> bool {
        self.bytes[self.pos..].starts_with(s)
    }

    /// Moves past the next `end`, or fails with `message` at the start.
    fn skip_past(&mut self, end: &[u8], message: &'static str) -> Result<usize, XmlError> {
        let start = self.pos;
        match find(&self.bytes[start..], end) {
            Some(i) => {
                self.pos = start + i + end.len();
                Ok(start + i)
            }
            None => Err(err(start, message)),
        }
    }

    fn skip_space(&mut self) {
        while self
            .bytes
            .get(self.pos)
            .is_some_and(|b| b.is_ascii_whitespace())
        {
            self.pos += 1;
        }
    }

    /// A name: everything up to whitespace or markup.
    fn name(&mut self) -> Result<&'t str, XmlError> {
        let start = self.pos;
        while let Some(&b) = self.bytes.get(self.pos) {
            if b.is_ascii_whitespace() || matches!(b, b'/' | b'>' | b'=' | b'<' | b'"' | b'\'') {
                break;
            }
            self.pos += 1;
        }
        if self.pos == start {
            return Err(err(start, "expected a name"));
        }
        Ok(&self.text[start..self.pos])
    }

    fn run(mut self) -> Result<Element, XmlError> {
        let mut stack: Vec<Element> = Vec::new();
        let mut root: Option<Element> = None;
        let mut elements = 0usize;
        while self.pos < self.bytes.len() {
            let at = self.pos;
            if self.starts(b"<?") {
                self.skip_past(b"?>", "unterminated processing instruction")?;
            } else if self.starts(b"<!--") {
                self.skip_past(b"-->", "unterminated comment")?;
            } else if self.starts(b"<![CDATA[") {
                self.pos += 9;
                let end = self.skip_past(b"]]>", "unterminated CDATA section")?;
                let data = &self.text[at + 9..end];
                match stack.last_mut() {
                    Some(parent) => push_text(parent, normalize_newlines(data)),
                    None => return Err(err(at, "character data outside the root element")),
                }
            } else if self.starts(b"<!") {
                self.skip_declaration()?;
            } else if self.starts(b"</") {
                self.pos += 2;
                let name = self.name()?;
                self.skip_space();
                if self.bytes.get(self.pos) != Some(&b'>') {
                    return Err(err(self.pos, "expected '>' after an end tag"));
                }
                self.pos += 1;
                let Some(element) = stack.pop() else {
                    return Err(err(at, "end tag without a start tag"));
                };
                if element.name != name {
                    return Err(err(at, "end tag does not match the start tag"));
                }
                match stack.last_mut() {
                    Some(parent) => parent.children.push(XmlNode::Element(element)),
                    None => root = Some(element),
                }
            } else if self.starts(b"<") {
                if root.is_some() && stack.is_empty() {
                    return Err(err(at, "content after the root element"));
                }
                if stack.len() >= MAX_XML_DEPTH {
                    return Err(err(at, "elements nested too deeply"));
                }
                elements += 1;
                if elements > MAX_XML_ELEMENTS {
                    return Err(err(at, "too many elements"));
                }
                self.pos += 1;
                let (element, empty) = self.start_tag()?;
                if empty {
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(XmlNode::Element(element)),
                        None => root = Some(element),
                    }
                } else {
                    stack.push(element);
                }
            } else {
                let end = find(&self.bytes[at..], b"<").map_or(self.bytes.len(), |i| at + i);
                let raw = &self.text[at..end];
                self.pos = end;
                match stack.last_mut() {
                    Some(parent) => {
                        let decoded = decode(raw, at)?;
                        push_text(parent, normalize_newlines(&decoded));
                    }
                    None if raw
                        .trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
                        .is_empty() => {}
                    None => return Err(err(at, "character data outside the root element")),
                }
            }
        }
        if !stack.is_empty() {
            return Err(err(self.bytes.len(), "unclosed element"));
        }
        root.ok_or_else(|| err(self.bytes.len(), "no root element"))
    }

    /// `<!DOCTYPE ...>` and other declarations, an internal subset in
    /// brackets included.
    fn skip_declaration(&mut self) -> Result<(), XmlError> {
        let start = self.pos;
        let mut in_subset = false;
        let mut quote: Option<u8> = None;
        while let Some(&b) = self.bytes.get(self.pos) {
            self.pos += 1;
            match (quote, b) {
                (Some(q), _) if b == q => quote = None,
                (Some(_), _) => {}
                (None, b'"' | b'\'') => quote = Some(b),
                (None, b'[') => in_subset = true,
                (None, b']') => in_subset = false,
                (None, b'>') if !in_subset => return Ok(()),
                _ => {}
            }
        }
        Err(err(start, "unterminated declaration"))
    }

    /// After `<`: the name, the attributes, and whether the tag closes
    /// itself (`/>`).
    fn start_tag(&mut self) -> Result<(Element, bool), XmlError> {
        let name = self.name()?;
        let mut element = Element {
            name: name.to_string(),
            attrs: Vec::new(),
            children: Vec::new(),
        };
        loop {
            let before = self.pos;
            self.skip_space();
            match self.bytes.get(self.pos) {
                None => return Err(err(self.pos, "unterminated start tag")),
                Some(b'>') => {
                    self.pos += 1;
                    return Ok((element, false));
                }
                Some(b'/') => {
                    if self.bytes.get(self.pos + 1) != Some(&b'>') {
                        return Err(err(self.pos, "expected '>' after '/'"));
                    }
                    self.pos += 2;
                    return Ok((element, true));
                }
                Some(_) if self.pos == before => {
                    return Err(err(self.pos, "expected whitespace before an attribute"));
                }
                Some(_) => {
                    let key = self.name()?;
                    self.skip_space();
                    if self.bytes.get(self.pos) != Some(&b'=') {
                        return Err(err(self.pos, "expected '=' after an attribute name"));
                    }
                    self.pos += 1;
                    self.skip_space();
                    let quote = match self.bytes.get(self.pos) {
                        Some(&q @ (b'"' | b'\'')) => q,
                        _ => return Err(err(self.pos, "expected a quoted attribute value")),
                    };
                    self.pos += 1;
                    let start = self.pos;
                    let Some(len) = self.bytes[start..].iter().position(|&b| b == quote) else {
                        return Err(err(start, "unterminated attribute value"));
                    };
                    let raw = &self.text[start..start + len];
                    if let Some(i) = raw.find('<') {
                        return Err(err(start + i, "'<' in an attribute value"));
                    }
                    self.pos = start + len + 1;
                    let value = decode(&raw.replace(['\t', '\n', '\r'], " "), start)?;
                    if !element.attrs.iter().any(|(k, _)| k == key) {
                        element.attrs.push((key.to_string(), value));
                    }
                }
            }
        }
    }
}

/// Appends text to `parent`, joined with a text node just before it.
fn push_text(parent: &mut Element, text: String) {
    if text.is_empty() {
        return;
    }
    if let Some(XmlNode::Text(last)) = parent.children.last_mut() {
        last.push_str(&text);
    } else {
        parent.children.push(XmlNode::Text(text));
    }
}

/// `\r\n` and lone `\r` as `\n`, as XML reads line ends.
fn normalize_newlines(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_string();
    }
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Decodes the five predefined entities and numeric references. An
/// unknown entity or a lone `&` is kept as written (Designer never writes
/// one; other writers sometimes do). `offset` is where `raw` starts.
fn decode(raw: &str, offset: usize) -> Result<String, XmlError> {
    if !raw.contains('&') {
        return Ok(raw.to_string());
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let window = &rest.as_bytes()[..rest.len().min(12)];
        let Some(semi) = window.iter().position(|&b| b == b';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..semi];
        let ch = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => {
                if let Some(num) = entity.strip_prefix('#') {
                    let code = match num.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => num.parse::<u32>().ok(),
                    };
                    let Some(code) = code else {
                        let at = offset + (raw.len() - rest.len());
                        return Err(err(at, "bad character reference"));
                    };
                    Some(
                        char::from_u32(code)
                            .filter(|&c| c != '\0')
                            .unwrap_or('\u{fffd}'),
                    )
                } else {
                    None
                }
            }
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// `&`, `<` and `>` escaped.
pub fn escape_text(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

/// `escape_text` plus `"`.
pub fn escape_attr(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_elements_attributes_and_text() {
        let root = parse(
            "<?xml version='1.0'?>\n<!-- head --><a x=\"1\" y='two' p:z=\"&lt;&amp;&gt;&quot;&apos;\">\
             <b/>text &#65;&#x42;<![CDATA[<raw>&amp;]]><?pi skipped?><c k=\"v\">in</c><!-- c --></a>\n",
        )
        .unwrap();
        assert_eq!(root.name, "a");
        assert_eq!(root.attr("x"), Some("1"));
        assert_eq!(root.attr("y"), Some("two"));
        assert_eq!(root.attr("p:z"), Some("<&>\"'"));
        assert_eq!(root.attr("z"), Some("<&>\"'"));
        assert_eq!(root.text(), "text AB<raw>&amp;");
        assert_eq!(root.child("c").map(Element::text), Some("in".to_string()));
        assert_eq!(root.elements().count(), 2);
    }

    #[test]
    fn keeps_prefixes_whitespace_and_newlines_inside_tags() {
        let root = parse("<xfa:data\n><f\n>  x  </f\n>\n<g/></xfa:data\n>").unwrap();
        assert_eq!(root.name, "xfa:data");
        assert_eq!(root.local(), "data");
        assert_eq!(root.child("f").unwrap().text(), "  x  ");
        // Whitespace between elements is content.
        assert_eq!(root.children.len(), 3);
        assert_eq!(root.children[1], XmlNode::Text("\n".into()));
    }

    #[test]
    fn line_ends_and_attribute_whitespace_normalize() {
        let root = parse("<a v=\"x\ny\">1\r\n2\r3</a>").unwrap();
        assert_eq!(root.attr("v"), Some("x y"));
        assert_eq!(root.text(), "1\n2\n3");
    }

    #[test]
    fn unknown_entities_are_kept() {
        let root = parse("<a>&nbsp; & b</a>").unwrap();
        assert_eq!(root.text(), "&nbsp; & b");
        assert!(parse("<a>&#xZZ;</a>").is_err());
    }

    #[test]
    fn malformed_input_is_an_error_with_an_offset() {
        let cases: &[(&str, usize)] = &[
            ("<a><b></a>", 6),
            ("<a>", 3),
            ("<a x=\"<\"/>", 6),
            ("<a x=1/>", 5),
            ("</a>", 0),
            ("<a/><b/>", 4),
            ("text<a/>", 0),
            ("<a", 2),
            ("<a x=\"1\"y=\"2\"/>", 8),
            ("<!-- open", 0),
            ("", 0),
        ];
        for (text, offset) in cases {
            let e = parse(text).unwrap_err();
            assert_eq!(e.offset, *offset, "{text}: {e}");
        }
    }

    #[test]
    fn depth_and_count_are_bounded() {
        let deep = "<a>".repeat(600) + &"</a>".repeat(600);
        let e = parse(&deep).unwrap_err();
        assert_eq!(e.message, "elements nested too deeply");
        let ok = "<a>".repeat(MAX_XML_DEPTH) + &"</a>".repeat(MAX_XML_DEPTH);
        assert!(parse(&ok).is_ok());
    }

    #[test]
    fn garbage_never_panics() {
        let samples = [
            "<",
            "<<",
            "<a",
            "<a b",
            "<a b=",
            "<a b='",
            "<a></",
            "<a></a",
            "<?",
            "<!",
            "<![CDATA[",
            "&",
            "<a>&#",
            "<a>&#x110000;</a>",
            "<a>&#0;</a>",
            "\u{feff}<a/>",
            "<a>é&amp</a>",
            "<é/>",
            "<!DOCTYPE a [<!ENTITY x 'y'>]><a/>",
        ];
        for s in samples {
            let _ = parse(s);
        }
        let template = "<template><subform name=\"a\"><field name=\"b\"/></subform></template>";
        for cut in 0..template.len() {
            let _ = parse(&template[..cut]);
        }
        assert_eq!(parse("<a>&#x110000;</a>").unwrap().text(), "\u{fffd}");
        assert!(parse("\u{feff}<a/>").is_ok());
        assert!(parse("<!DOCTYPE a [<!ENTITY x 'y'>]><a/>").is_ok());
    }

    #[test]
    fn serialize_round_trips() {
        let text =
            "<t:a k=\"&quot;q&quot; &amp; &lt;\"><b>x &amp; y &lt; z</b>\n  <c/><d>é</d></t:a>";
        let root = parse(text).unwrap();
        let written = serialize(&root);
        assert!(written.starts_with("<?xml"));
        assert_eq!(parse(&written).unwrap(), root);
        assert_eq!(serialize_fragment(&root), text);
        let rich = parse("<exData><body><p>a<span>b</span></p></body></exData>").unwrap();
        assert_eq!(rich.inner_xml(), "<body><p>a<span>b</span></p></body>");
    }

    #[test]
    fn deep_trees_built_in_code_serialize() {
        let mut e = Element {
            name: "x".into(),
            ..Default::default()
        };
        for _ in 0..5000 {
            e = Element {
                name: "x".into(),
                attrs: Vec::new(),
                children: vec![XmlNode::Element(e)],
            };
        }
        let text = serialize_fragment(&e);
        assert_eq!(text.len(), 5001 * 3 + 5000 * 4 + 1);
        // Taken apart by hand, so the drop does not recurse 5000 deep.
        let mut node = Some(e);
        while let Some(mut e) = node {
            node = match e.children.pop() {
                Some(XmlNode::Element(c)) => Some(c),
                _ => None,
            };
        }
    }
}

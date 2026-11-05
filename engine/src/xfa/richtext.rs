// SPDX-License-Identifier: MIT
//! XFA rich text (`exData contentType="text/html"`, an XHTML fragment)
//! read into a `text::Block`: paragraphs of styled runs.
//!
//! Elements: `p`, `div`, `li`, `ul`, `ol` and `h1`–`h6` are paragraphs
//! (an `li` starts with "• ", or "N. " inside an `ol`); `br` ends the
//! line and continues with a paragraph of the same settings (no space
//! between the two halves, no indent on the second); `b`/`strong` bold,
//! `i`/`em` italic, `u` underline, `s`/`strike`/`del` line-through,
//! `sup`/`sub` two thirds of the size (the baseline is not shifted: the
//! model has no rise); `html`, `body`, `span`, `a` and any other element
//! are transparent. A trailing `br` in a paragraph adds no line; an empty
//! `<p/>` is one empty line.
//!
//! `style` on any element: `font-family` (the first of a list, quotes
//! stripped), `font-size` (`pt`, `px`, `in`, `mm`, `cm`, `em` and `%` of
//! the inherited size), `font-weight` (`bold`, `bolder`, 600 and up),
//! `font-style` (`italic`, `oblique`), `text-decoration` (`underline`,
//! `line-through`, `none`), `color` (`#rrggbb`, `#rgb`, `rgb(r,g,b)`, the
//! sixteen HTML names), `letter-spacing` (a length, `em` of the size,
//! `normal`), `vertical-align` (`super`, `sub`), `xfa-spacerun:yes`
//! (spaces kept as written), `xfa-tab-count:N` (N tabs, at most 64). On
//! paragraph elements (and `body`/`html`, whose settings paragraphs
//! inherit) also `text-align`, `margin` and `margin-*` (top and bottom are
//! space above and below), `text-indent` and `line-height` (a length, or
//! a number times the size). Styles inherit down the tree from the
//! field's `font` and `para`. Ignored: `tab-stops`, `tab-interval`,
//! `xfa-tab-stops`, `kerning-mode`, `font-stretch`, `text-valign`,
//! `xfa-font-*`, CSS classes and every other property.
//!
//! Whitespace: outside `xfa-spacerun:yes`, a run of spaces, tabs and
//! newlines is one space, and a paragraph's leading and trailing
//! whitespace is dropped; U+00A0 (`&#160;`, `&nbsp;`) is kept. Floating
//! fields: a `span` with `xfa:embed="#id"` shows `embed("id")` (nothing
//! when it is `None`) in place of its content; `embedType` and
//! `embedMode` are ignored.
//!
//! The markup is read with `xml::parse`, the fragment wrapped in one root
//! element, HTML's named entities rewritten as character references
//! first. When it is not well formed, a tag stripper reads its text.
//! Bounded: `limits::MAX_BLOCK_CHARS` characters and paragraphs together,
//! `limits::MAX_XML_DEPTH` levels.

use super::limits::{MAX_BLOCK_CHARS, MAX_XML_DEPTH};
use super::text::{Block, Paragraph, Run};
use super::types::{self, Font, HAlign, Para, Pt, Rgb};
use super::xml::{self, Element, XmlNode};

/// Reads an XHTML fragment into paragraphs of styled runs. `base` and
/// `para` are the field's or draw's own settings, which the markup
/// overrides. `embed` gives the text of a floating field by its `id`.
pub fn parse(
    html: &str,
    base: &Font,
    para: &Para,
    embed: &dyn Fn(&str) -> Option<String>,
) -> Block {
    match read_markup(html) {
        Some(root) => from_element(&root, base, para, embed),
        None => {
            let text = strip_tags(html);
            if text.is_empty() {
                return Block::default();
            }
            Block {
                paragraphs: text
                    .split('\n')
                    .map(|line| Paragraph {
                        runs: vec![Run {
                            text: line.to_string(),
                            font: base.clone(),
                        }],
                        para: para.clone(),
                    })
                    .collect(),
            }
        }
    }
}

/// The fragment's text with its tags removed, paragraphs separated by
/// newlines, entities decoded: `parse(html, &Font::default(),
/// &Para::default(), &|_| None).plain()`. Markup that is not well formed
/// goes through a tag stripper instead.
pub fn plain_text(html: &str) -> String {
    match read_markup(html) {
        Some(root) => from_element(&root, &Font::default(), &Para::default(), &|_| None).plain(),
        None => strip_tags(html),
    }
}

/// The paragraphs of an element tree: the element and its content as
/// `parse` reads them (the element itself is transparent unless it is a
/// paragraph element).
pub fn from_element(
    root: &Element,
    base: &Font,
    para: &Para,
    embed: &dyn Fn(&str) -> Option<String>,
) -> Block {
    let mut b = Builder {
        paragraphs: Vec::new(),
        open: None,
        budget: MAX_BLOCK_CHARS,
        serial: 0,
        embed,
    };
    let style = Style {
        font: base.clone(),
        para: para.clone(),
        spacerun: false,
    };
    b.walk(root, &style, 0, None);
    b.finish()
}

/// The fragment as one element tree, or `None` when it is not well
/// formed (or empty).
fn read_markup(html: &str) -> Option<Element> {
    let mut text = html.trim_start_matches('\u{feff}').trim_start();
    if text.starts_with("<?xml")
        && let Some(end) = text.find("?>")
    {
        text = &text[end + 2..];
    }
    if text.trim().is_empty() {
        return None;
    }
    let mut wrapped = String::with_capacity(text.len() + 32);
    wrapped.push_str("<tpdf-rich>");
    wrapped.push_str(&html_entities_to_references(text));
    wrapped.push_str("</tpdf-rich>");
    xml::parse(&wrapped).ok()
}

/// HTML named entities XML lacks, as their code points.
const HTML_ENTITIES: &[(&str, u32)] = &[
    ("nbsp", 0xa0),
    ("copy", 0xa9),
    ("reg", 0xae),
    ("deg", 0xb0),
    ("plusmn", 0xb1),
    ("middot", 0xb7),
    ("sect", 0xa7),
    ("para", 0xb6),
    ("laquo", 0xab),
    ("raquo", 0xbb),
    ("shy", 0xad),
    ("times", 0xd7),
    ("divide", 0xf7),
    ("eacute", 0xe9),
    ("egrave", 0xe8),
    ("agrave", 0xe0),
    ("ccedil", 0xe7),
    ("ndash", 0x2013),
    ("mdash", 0x2014),
    ("lsquo", 0x2018),
    ("rsquo", 0x2019),
    ("sbquo", 0x201a),
    ("ldquo", 0x201c),
    ("rdquo", 0x201d),
    ("bdquo", 0x201e),
    ("bull", 0x2022),
    ("hellip", 0x2026),
    ("euro", 0x20ac),
    ("trade", 0x2122),
    ("dagger", 0x2020),
    ("Dagger", 0x2021),
    ("permil", 0x2030),
];

fn html_entity(name: &str) -> Option<char> {
    HTML_ENTITIES
        .iter()
        .find(|e| e.0 == name)
        .and_then(|e| char::from_u32(e.1))
}

/// `&name;` for an HTML entity XML does not predefine, rewritten as
/// `&#N;`; everything else as written.
fn html_entities_to_references(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let name_end = rest[1..]
            .char_indices()
            .take(12)
            .find(|(_, c)| !c.is_ascii_alphanumeric())
            .map(|(i, c)| (i + 1, c));
        match name_end {
            Some((end, ';')) if end > 1 => {
                let name = &rest[1..end];
                match html_entity(name) {
                    Some(ch) => {
                        out.push_str("&#");
                        out.push_str(&u32::from(ch).to_string());
                        out.push(';');
                    }
                    None => out.push_str(&rest[..=end]),
                }
                rest = &rest[end + 1..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Inherited settings.
#[derive(Clone)]
struct Style {
    font: Font,
    para: Para,
    spacerun: bool,
}

/// The paragraph being filled.
struct Open {
    paragraph: Paragraph,
    /// The serial of the paragraph element that opened it (`None`: text
    /// outside one, or the continuation after a `br`).
    owner: Option<usize>,
    /// Opened by a `br`.
    continuation: bool,
    /// Any character (a collapsed paragraph's first visible one).
    content: bool,
    /// A collapsed space waiting for the next character, in this font.
    pending: Option<Font>,
}

struct Builder<'a> {
    paragraphs: Vec<Paragraph>,
    open: Option<Open>,
    /// Characters and paragraphs left.
    budget: usize,
    /// Elements visited, to tell paragraph elements apart.
    serial: usize,
    embed: &'a dyn Fn(&str) -> Option<String>,
}

/// What an element is.
#[derive(Clone, Copy, PartialEq)]
enum Role {
    Paragraph,
    Break,
    Inline,
}

fn role(local: &str) -> Role {
    match local {
        "p" | "div" | "li" | "ul" | "ol" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
        | "blockquote" => Role::Paragraph,
        "br" => Role::Break,
        _ => Role::Inline,
    }
}

/// The list an `li` is in: `None` outside one, `Some(None)` in a `ul`,
/// `Some(Some(n))` for the next number in an `ol`.
type ListState = Option<Option<usize>>;

impl Builder<'_> {
    fn finish(mut self) -> Block {
        if let Some(open) = self.open.take()
            && open.content
        {
            self.push(open);
        }
        Block {
            paragraphs: self.paragraphs,
        }
    }

    fn walk(&mut self, e: &Element, inherited: &Style, depth: usize, list: ListState) {
        if self.budget == 0 || depth > MAX_XML_DEPTH {
            return;
        }
        self.serial += 1;
        let id = self.serial;
        let local = e.local().to_ascii_lowercase();
        let role = role(&local);
        let mut style = inherited.clone();
        match local.as_str() {
            "b" | "strong" => style.font.bold = true,
            "i" | "em" => style.font.italic = true,
            "u" => style.font.underline = true,
            "s" | "strike" | "del" => style.font.line_through = true,
            "sup" | "sub" => style.font.size *= 2.0 / 3.0,
            _ => {}
        }
        let para_props = role == Role::Paragraph || matches!(local.as_str(), "body" | "html");
        let mut tabs = 0;
        if let Some(css) = e.attr("style") {
            tabs = apply_css(css, inherited, &mut style, para_props);
        }

        match role {
            Role::Break => {
                self.line_break(&style.para);
                return;
            }
            Role::Paragraph => {
                self.begin_paragraph(id, &style.para);
                if local == "li" {
                    let marker = match list {
                        Some(Some(n)) => format!("{n}. "),
                        _ => "\u{2022} ".to_string(),
                    };
                    self.push_literal(&marker, &style);
                }
            }
            Role::Inline => {}
        }
        if tabs > 0 {
            self.push_literal(&"\t".repeat(tabs), &style);
        }

        let embed = e.attr("xfa:embed").or_else(|| e.attr("embed"));
        if let Some(target) = embed {
            let id_text = target.trim().trim_start_matches('#');
            let value = (self.embed)(id_text).unwrap_or_default();
            let value: String = value
                .chars()
                .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
                .collect();
            self.push_literal(&value, &style);
        } else {
            // Children of a paragraph inherit its settings without its
            // spacing (that belongs to the paragraph itself).
            let mut child = style.clone();
            if role == Role::Paragraph {
                child.para.space_above = 0.0;
                child.para.space_below = 0.0;
            }
            let mut list_here: ListState = match local.as_str() {
                "ul" => Some(None),
                "ol" => Some(Some(1)),
                _ => list,
            };
            for node in &e.children {
                if self.budget == 0 {
                    break;
                }
                match node {
                    XmlNode::Text(t) => self.push_text(t, &child),
                    XmlNode::Element(c) => {
                        let item = matches!(list_here, Some(Some(_)))
                            && c.local().eq_ignore_ascii_case("li");
                        self.walk(c, &child, depth + 1, list_here);
                        if item && let Some(Some(n)) = list_here.as_mut() {
                            *n = n.saturating_add(1);
                        }
                    }
                }
            }
        }

        if role == Role::Paragraph {
            self.end_paragraph(id);
        }
    }

    fn push(&mut self, open: Open) {
        if self.budget == 0 {
            return;
        }
        self.budget -= 1;
        self.paragraphs.push(open.paragraph);
    }

    fn new_open(para: &Para, owner: Option<usize>, continuation: bool) -> Open {
        Open {
            paragraph: Paragraph {
                runs: Vec::new(),
                para: para.clone(),
            },
            owner,
            continuation,
            content: false,
            pending: None,
        }
    }

    /// Closes an open paragraph that has text (an empty one gives way) and
    /// opens the element's.
    fn begin_paragraph(&mut self, id: usize, para: &Para) {
        if let Some(open) = self.open.take() {
            self.close_or_drop(open);
        }
        self.open = Some(Self::new_open(para, Some(id), false));
    }

    /// Closes the element's paragraph (even empty) or whatever text
    /// followed inside it.
    fn end_paragraph(&mut self, id: usize) {
        if let Some(open) = self.open.take() {
            if open.owner == Some(id) {
                self.push(open);
            } else {
                self.close_or_drop(open);
            }
        }
    }

    /// Pushes a paragraph with text; drops an empty one (an empty
    /// continuation hands its space below to the paragraph before it).
    fn close_or_drop(&mut self, open: Open) {
        if open.content {
            self.push(open);
        } else if open.continuation
            && let Some(last) = self.paragraphs.last_mut()
        {
            last.para.space_below = open.paragraph.para.space_below;
        }
    }

    fn line_break(&mut self, para: &Para) {
        let mut open = match self.open.take() {
            Some(o) => o,
            None => Self::new_open(para, None, false),
        };
        let mut next = open.paragraph.para.clone();
        open.paragraph.para.space_below = 0.0;
        next.space_above = 0.0;
        next.text_indent = 0.0;
        self.push(open);
        self.open = Some(Self::new_open(&next, None, true));
    }

    fn open_mut(&mut self, para: &Para) -> &mut Open {
        self.open
            .get_or_insert_with(|| Self::new_open(para, None, false))
    }

    /// Appends one character in `font`, merging into the last run when
    /// the font is the same.
    fn push_char(&mut self, ch: char, font: &Font, para: &Para) {
        if self.budget == 0 {
            return;
        }
        self.budget -= 1;
        let open = self.open_mut(para);
        open.content = true;
        let runs = &mut open.paragraph.runs;
        match runs.last_mut() {
            Some(run) if run.font == *font => run.text.push(ch),
            _ => runs.push(Run {
                text: ch.to_string(),
                font: font.clone(),
            }),
        }
    }

    fn flush_pending(&mut self, para: &Para) {
        if let Some(font) = self.open.as_mut().and_then(|o| o.pending.take()) {
            self.push_char(' ', &font, para);
        }
    }

    /// Text as written (list markers, tabs, floating fields).
    fn push_literal(&mut self, text: &str, style: &Style) {
        if text.is_empty() {
            return;
        }
        self.flush_pending(&style.para);
        for ch in text.chars() {
            if self.budget == 0 {
                break;
            }
            self.push_char(ch, &style.font, &style.para);
        }
    }

    /// Character data: collapsed, or kept as written in a spacerun.
    fn push_text(&mut self, text: &str, style: &Style) {
        if text.is_empty() {
            return;
        }
        if style.spacerun {
            let starts_white = text.chars().next().is_some_and(is_white);
            if starts_white {
                if let Some(open) = self.open.as_mut() {
                    open.pending = None;
                }
            } else {
                self.flush_pending(&style.para);
            }
            for ch in text.chars() {
                if self.budget == 0 {
                    break;
                }
                let ch = if ch == '\n' || ch == '\r' || ch == '\t' {
                    ' '
                } else {
                    ch
                };
                self.push_char(ch, &style.font, &style.para);
            }
            return;
        }
        for ch in text.chars() {
            if self.budget == 0 {
                break;
            }
            if is_white(ch) {
                let open = self.open_mut(&style.para);
                if open.content && open.pending.is_none() {
                    open.pending = Some(style.font.clone());
                }
            } else {
                self.flush_pending(&style.para);
                self.push_char(ch, &style.font, &style.para);
            }
        }
    }
}

/// Collapsible whitespace (not U+00A0).
fn is_white(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\n' | '\r' | '\u{c}')
}

/// Applies a `style` attribute: font properties to `style.font`, the
/// paragraph ones to `style.para` when `para_props`. Returns the tab
/// count. `parent` gives the sizes `em` and `%` are relative to.
fn apply_css(css: &str, parent: &Style, style: &mut Style, para_props: bool) -> usize {
    let mut tabs = 0;
    for decl in css.split(';').take(64) {
        let Some((key, value)) = decl.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        let lower = value.to_ascii_lowercase();
        match key.as_str() {
            "font-family" => {
                let first = value.split(',').next().unwrap_or("").trim();
                let family = first.trim_matches(|c| c == '"' || c == '\'').trim();
                if !family.is_empty() {
                    style.font.family = family.to_string();
                }
            }
            "font-size" => {
                if let Some(size) = css_length(&lower, parent.font.size)
                    && size > 0.0
                    && size <= 10_000.0
                {
                    style.font.size = size;
                }
            }
            "font-weight" => {
                style.font.bold = match lower.as_str() {
                    "bold" | "bolder" => true,
                    "normal" | "lighter" => false,
                    n => n.parse::<u32>().map_or(style.font.bold, |w| w >= 600),
                }
            }
            "font-style" => style.font.italic = matches!(lower.as_str(), "italic" | "oblique"),
            "text-decoration" => {
                if lower.split_whitespace().any(|w| w == "none") {
                    style.font.underline = false;
                    style.font.line_through = false;
                }
                for word in lower.split_whitespace() {
                    match word {
                        "underline" => style.font.underline = true,
                        "line-through" => style.font.line_through = true,
                        _ => {}
                    }
                }
            }
            "color" => {
                if let Some(c) = css_color(&lower) {
                    style.font.color = c;
                }
            }
            "letter-spacing" => {
                if lower == "normal" {
                    style.font.letter_spacing = 0.0;
                } else if let Some(v) = css_length(&lower, style.font.size) {
                    style.font.letter_spacing = v;
                }
            }
            "vertical-align" => {
                if matches!(lower.as_str(), "super" | "sub") {
                    style.font.size = parent.font.size * 2.0 / 3.0;
                }
            }
            "xfa-spacerun" => style.spacerun = lower == "yes",
            "xfa-tab-count" => tabs = lower.parse::<usize>().unwrap_or(0).min(64),
            _ if para_props => apply_para_css(&key, &lower, style),
            _ => {}
        }
    }
    tabs
}

fn apply_para_css(key: &str, value: &str, style: &mut Style) {
    let size = style.font.size;
    let len = |v: &str| css_length(v, size);
    let para = &mut style.para;
    match key {
        "text-align" => {
            para.h_align = match value {
                "center" => HAlign::Center,
                "right" | "end" => HAlign::Right,
                "justify" | "justify-all" => HAlign::Justify,
                "left" | "start" => HAlign::Left,
                _ => para.h_align,
            }
        }
        "margin-left" => para.margin_left = len(value).unwrap_or(para.margin_left),
        "margin-right" => para.margin_right = len(value).unwrap_or(para.margin_right),
        "margin-top" => para.space_above = len(value).unwrap_or(para.space_above),
        "margin-bottom" => para.space_below = len(value).unwrap_or(para.space_below),
        "margin" => {
            let v: Vec<Pt> = value.split_whitespace().take(4).filter_map(len).collect();
            let (t, r, b, l) = match v.as_slice() {
                [a] => (*a, *a, *a, *a),
                [a, b] => (*a, *b, *a, *b),
                [a, b, c] => (*a, *b, *c, *b),
                [a, b, c, d] => (*a, *b, *c, *d),
                _ => return,
            };
            para.space_above = t;
            para.margin_right = r;
            para.space_below = b;
            para.margin_left = l;
        }
        "text-indent" => para.text_indent = len(value).unwrap_or(para.text_indent),
        "line-height" => {
            if value == "normal" {
                para.line_height = None;
            } else if let Ok(factor) = value.parse::<f64>() {
                if factor.is_finite() && factor >= 0.0 {
                    para.line_height = Some(factor * size);
                }
            } else if let Some(v) = len(value) {
                para.line_height = Some(v);
            }
        }
        _ => {}
    }
}

/// A CSS length in points: the units `types::length` reads, `em` and `%`
/// of `em`.
fn css_length(value: &str, em: Pt) -> Option<Pt> {
    let value = value.trim();
    let v = if let Some(n) = value.strip_suffix("em") {
        n.trim().parse::<f64>().ok()? * em
    } else if let Some(n) = value.strip_suffix('%') {
        n.trim().parse::<f64>().ok()? * em / 100.0
    } else {
        types::length(value)?
    };
    v.is_finite().then_some(v)
}

/// A CSS colour: `#rrggbb`, `#rgb`, `rgb(r, g, b)`, the sixteen HTML
/// names.
fn css_color(value: &str) -> Option<Rgb> {
    if let Some(hex) = value.strip_prefix('#') {
        let digit = |i: usize| hex.get(i..=i).and_then(|d| u8::from_str_radix(d, 16).ok());
        return match hex.len() {
            6 => {
                let byte = |i: usize| {
                    hex.get(i..i + 2)
                        .and_then(|d| u8::from_str_radix(d, 16).ok())
                };
                Some(Rgb(byte(0)?, byte(2)?, byte(4)?))
            }
            3 => Some(Rgb(digit(0)? * 17, digit(1)? * 17, digit(2)? * 17)),
            _ => None,
        };
    }
    if let Some(inner) = value.strip_prefix("rgb(").and_then(|v| v.strip_suffix(')')) {
        let mut parts = inner.split(',').map(|p| {
            let p = p.trim();
            match p.strip_suffix('%') {
                Some(n) => n.trim().parse::<f64>().ok().map(|v| v * 255.0 / 100.0),
                None => p.parse::<f64>().ok(),
            }
            .filter(|v| v.is_finite())
            .map(|v| v.round().clamp(0.0, 255.0) as u8)
        });
        let r = parts.next()??;
        let g = parts.next()??;
        let b = parts.next()??;
        return parts.next().is_none().then_some(Rgb(r, g, b));
    }
    let named = match value {
        "black" => Rgb(0, 0, 0),
        "silver" => Rgb(192, 192, 192),
        "gray" | "grey" => Rgb(128, 128, 128),
        "white" => Rgb(255, 255, 255),
        "maroon" => Rgb(128, 0, 0),
        "red" => Rgb(255, 0, 0),
        "purple" => Rgb(128, 0, 128),
        "fuchsia" => Rgb(255, 0, 255),
        "green" => Rgb(0, 128, 0),
        "lime" => Rgb(0, 255, 0),
        "olive" => Rgb(128, 128, 0),
        "yellow" => Rgb(255, 255, 0),
        "navy" => Rgb(0, 0, 128),
        "blue" => Rgb(0, 0, 255),
        "teal" => Rgb(0, 128, 128),
        "aqua" => Rgb(0, 255, 255),
        _ => return None,
    };
    Some(named)
}

/// Decodes an entity at the start of `text` (after its `&`): the XML five,
/// the HTML names above, `#N` and `#xH`. Returns the character and the
/// bytes read after the `&`.
fn decode_entity(text: &str) -> Option<(char, usize)> {
    let end = text.char_indices().take(12).find(|(_, c)| *c == ';')?.0;
    let name = &text[..end];
    let ch = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        _ => {
            if let Some(num) = name.strip_prefix('#') {
                let code = match num.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                    None => num.parse::<u32>().ok()?,
                };
                char::from_u32(code)?
            } else {
                html_entity(name)?
            }
        }
    };
    Some((ch, end + 1))
}

/// The text of markup that is not well formed: tags removed (`p`, `div`,
/// `li` and `br` end lines as `parse` would), entities decoded,
/// whitespace collapsed, lines trimmed. Bounded by `MAX_BLOCK_CHARS`.
fn strip_tags(html: &str) -> String {
    struct Line {
        text: String,
        owned: bool,
        content: bool,
        pending: bool,
    }
    let fresh = |owned| Line {
        text: String::new(),
        owned,
        content: false,
        pending: false,
    };
    let mut lines: Vec<String> = Vec::new();
    let mut cur: Option<Line> = None;
    let mut budget = MAX_BLOCK_CHARS;
    let mut rest = html;
    while !rest.is_empty() && budget > 0 {
        if let Some(tag) = rest.strip_prefix('<') {
            let end = tag.find('>').unwrap_or(tag.len());
            let inner = &tag[..end];
            rest = tag.get(end + 1..).unwrap_or("");
            if inner.starts_with('!') || inner.starts_with('?') {
                continue;
            }
            let closing = inner.starts_with('/');
            let self_closing = inner.ends_with('/');
            let name: String = inner
                .trim_start_matches('/')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == ':')
                .collect();
            let local = name.rsplit(':').next().unwrap_or("").to_ascii_lowercase();
            match role(&local) {
                Role::Break => {
                    let line = cur.take().unwrap_or_else(|| fresh(false));
                    lines.push(line.text);
                    budget = budget.saturating_sub(1);
                    cur = Some(fresh(false));
                }
                Role::Paragraph => {
                    if !closing {
                        if let Some(line) = cur.take()
                            && line.content
                        {
                            lines.push(line.text);
                            budget = budget.saturating_sub(1);
                        }
                        cur = Some(fresh(true));
                    }
                    if closing || self_closing {
                        if let Some(line) = cur.take()
                            && (line.content || line.owned)
                        {
                            lines.push(line.text);
                            budget = budget.saturating_sub(1);
                        }
                    }
                }
                Role::Inline => {}
            }
            continue;
        }
        let (ch, len) = match rest.strip_prefix('&').and_then(decode_entity) {
            Some((ch, n)) => (ch, n + 1),
            None => {
                let ch = rest.chars().next().unwrap_or(' ');
                (ch, ch.len_utf8())
            }
        };
        rest = rest.get(len..).unwrap_or("");
        let line = cur.get_or_insert_with(|| fresh(false));
        if is_white(ch) {
            line.pending = line.content;
        } else {
            if line.pending {
                line.text.push(' ');
                line.pending = false;
            }
            line.text.push(ch);
            line.content = true;
            budget -= 1;
        }
    }
    if let Some(line) = cur
        && line.content
    {
        lines.push(line.text);
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(name: &str, attrs: &[(&str, &str)], children: Vec<XmlNode>) -> XmlNode {
        XmlNode::Element(Element {
            name: name.into(),
            attrs: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            children,
        })
    }

    fn t(text: &str) -> XmlNode {
        XmlNode::Text(text.into())
    }

    fn styled(name: &str, style: &str, children: Vec<XmlNode>) -> XmlNode {
        el(name, &[("style", style)], children)
    }

    fn root(children: Vec<XmlNode>) -> Element {
        Element {
            name: "body".into(),
            attrs: vec![],
            children,
        }
    }

    fn read(children: Vec<XmlNode>) -> Block {
        from_element(&root(children), &Font::default(), &Para::default(), &|_| {
            None
        })
    }

    /// The font of the run holding `needle`.
    fn font_of<'a>(b: &'a Block, needle: &str) -> &'a Font {
        b.paragraphs
            .iter()
            .flat_map(|p| &p.runs)
            .find(|r| r.text.contains(needle))
            .map(|r| &r.font)
            .unwrap_or_else(|| panic!("no run with {needle:?} in {b:?}"))
    }

    #[test]
    fn paragraphs_and_breaks() {
        let b = read(vec![
            el("p", &[], vec![t("one")]),
            el("p", &[], vec![t("two"), el("br", &[], vec![]), t("three")]),
            el("p", &[], vec![]),
            el("p", &[], vec![el("br", &[], vec![])]),
            el("p", &[], vec![t("four"), el("br", &[], vec![])]),
            el("div", &[], vec![t("five")]),
        ]);
        assert_eq!(b.plain(), "one\ntwo\nthree\n\n\nfour\nfive");
        assert_eq!(b.paragraphs.len(), 7);
    }

    #[test]
    fn a_break_continues_the_paragraph_settings() {
        let b = read(vec![styled(
            "p",
            "text-align:center;margin-top:4pt;margin-bottom:6pt;text-indent:9pt;margin-left:3pt",
            vec![t("a"), el("br", &[], vec![]), t("b")],
        )]);
        let (first, second) = (&b.paragraphs[0].para, &b.paragraphs[1].para);
        assert_eq!(first.h_align, HAlign::Center);
        assert_eq!(second.h_align, HAlign::Center);
        assert_eq!((first.space_above, first.space_below), (4.0, 0.0));
        assert_eq!((second.space_above, second.space_below), (0.0, 6.0));
        assert_eq!((first.text_indent, second.text_indent), (9.0, 0.0));
        assert_eq!(second.margin_left, 3.0);
    }

    #[test]
    fn inline_elements_style_their_runs() {
        let b = read(vec![el(
            "p",
            &[],
            vec![
                t("plain "),
                el("b", &[], vec![t("bold ")]),
                el("strong", &[], vec![t("strong ")]),
                el("i", &[], vec![t("italic ")]),
                el("em", &[], vec![t("em ")]),
                el("u", &[], vec![t("under ")]),
                el("sup", &[], vec![t("up")]),
                el("sub", &[], vec![t("down")]),
                el("a", &[("href", "x")], vec![t(" link")]),
                el("unknown", &[], vec![t(" other")]),
            ],
        )]);
        assert_eq!(
            b.plain(),
            "plain bold strong italic em under updown link other"
        );
        assert!(!font_of(&b, "plain").bold);
        assert!(font_of(&b, "bold").bold);
        assert!(font_of(&b, "strong").bold);
        assert!(font_of(&b, "italic").italic);
        assert!(font_of(&b, "em").italic);
        assert!(font_of(&b, "under").underline);
        assert!((font_of(&b, "up").size - 20.0 / 3.0).abs() < 1e-9);
        assert!((font_of(&b, "down").size - 20.0 / 3.0).abs() < 1e-9);
        assert_eq!(font_of(&b, "link"), &Font::default());
    }

    #[test]
    fn lists_are_paragraphs_with_markers() {
        let b = read(vec![
            el(
                "ul",
                &[],
                vec![el("li", &[], vec![t("a")]), el("li", &[], vec![t("b")])],
            ),
            el(
                "ol",
                &[],
                vec![el("li", &[], vec![t("c")]), el("li", &[], vec![t("d")])],
            ),
        ]);
        assert_eq!(b.plain(), "\u{2022} a\n\u{2022} b\n1. c\n2. d");
    }

    #[test]
    fn style_properties() {
        let span = |style: &str, text: &str| styled("span", style, vec![t(text)]);
        let b = read(vec![el(
            "p",
            &[],
            vec![
                span("font-family:'Times New Roman', serif", "fam "),
                span("font-size:12pt", "pt "),
                span("font-size:16px", "px "),
                span("font-size:1.5em", "em "),
                span("font-size:200%", "pc "),
                span("font-weight:bold", "w1 "),
                span("font-weight:700", "w2 "),
                span("font-weight:bolder", "w3 "),
                span("font-style:oblique", "obl "),
                span("text-decoration:line-through", "lt "),
                span("text-decoration:underline line-through", "both "),
                span("color:#0000ff", "c1 "),
                span("color:#f00", "c2 "),
                span("color:rgb(0, 128, 255)", "c3 "),
                span("color:teal", "c4 "),
                span("letter-spacing:0in", "ls0 "),
                span("letter-spacing:0.5pt", "ls1 "),
                span("letter-spacing:-0.1em", "ls2 "),
                span("vertical-align:super", "va "),
            ],
        )]);
        assert_eq!(font_of(&b, "fam").family, "Times New Roman");
        assert_eq!(font_of(&b, "pt").size, 12.0);
        assert_eq!(font_of(&b, "px").size, 12.0);
        assert_eq!(font_of(&b, "em").size, 15.0);
        assert_eq!(font_of(&b, "pc").size, 20.0);
        assert!(font_of(&b, "w1").bold && font_of(&b, "w2").bold && font_of(&b, "w3").bold);
        assert!(font_of(&b, "obl").italic);
        assert!(font_of(&b, "lt").line_through && !font_of(&b, "lt").underline);
        assert!(font_of(&b, "both").line_through && font_of(&b, "both").underline);
        assert_eq!(font_of(&b, "c1").color, Rgb(0, 0, 255));
        assert_eq!(font_of(&b, "c2").color, Rgb(255, 0, 0));
        assert_eq!(font_of(&b, "c3").color, Rgb(0, 128, 255));
        assert_eq!(font_of(&b, "c4").color, Rgb(0, 128, 128));
        assert_eq!(font_of(&b, "ls0").letter_spacing, 0.0);
        assert_eq!(font_of(&b, "ls1").letter_spacing, 0.5);
        assert!((font_of(&b, "ls2").letter_spacing + 1.0).abs() < 1e-9);
        assert!((font_of(&b, "va").size - 20.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn paragraph_style_properties() {
        let b = read(vec![
            styled(
                "p",
                "text-align:right;margin-left:1in;margin-right:2pt;margin-top:3pt;margin-bottom:4pt;text-indent:-13.68pt;line-height:18pt",
                vec![t("a")],
            ),
            styled(
                "p",
                "text-align:justify;margin:1pt 2pt;line-height:1.5",
                vec![t("b")],
            ),
            styled("span", "text-align:center;margin-left:9pt", vec![t("c")]),
        ]);
        let p = &b.paragraphs[0].para;
        assert_eq!(p.h_align, HAlign::Right);
        assert_eq!(
            (p.margin_left, p.margin_right, p.space_above, p.space_below),
            (72.0, 2.0, 3.0, 4.0)
        );
        assert_eq!(p.text_indent, -13.68);
        assert_eq!(p.line_height, Some(18.0));
        let p = &b.paragraphs[1].para;
        assert_eq!(p.h_align, HAlign::Justify);
        assert_eq!(
            (p.space_above, p.margin_right, p.space_below, p.margin_left),
            (1.0, 2.0, 1.0, 2.0)
        );
        assert_eq!(p.line_height, Some(15.0));
        // Paragraph properties on a span are ignored.
        assert_eq!(b.paragraphs[2].para, Para::default());
    }

    #[test]
    fn styles_inherit_from_the_base_and_down_the_tree() {
        let base = Font {
            family: "Myriad Pro".into(),
            size: 9.0,
            ..Font::default()
        };
        let para = Para {
            h_align: HAlign::Center,
            margin_left: 5.0,
            ..Para::default()
        };
        let tree = root(vec![styled(
            "p",
            "color:#ff0000;font-size:12pt",
            vec![
                t("red "),
                el(
                    "b",
                    &[],
                    vec![styled("span", "font-style:italic", vec![t("deep")])],
                ),
            ],
        )]);
        let b = from_element(&tree, &base, &para, &|_| None);
        let deep = font_of(&b, "deep");
        assert_eq!(deep.family, "Myriad Pro");
        assert_eq!(deep.size, 12.0);
        assert_eq!(deep.color, Rgb(255, 0, 0));
        assert!(deep.bold && deep.italic);
        assert_eq!(b.paragraphs[0].para.h_align, HAlign::Center);
        assert_eq!(b.paragraphs[0].para.margin_left, 5.0);
        // The body's style reaches its paragraphs.
        let tree = Element {
            name: "body".into(),
            attrs: vec![("style".into(), "font-size:14pt;text-align:right".into())],
            children: vec![el("p", &[], vec![t("x")])],
        };
        let b = from_element(&tree, &base, &Para::default(), &|_| None);
        assert_eq!(b.paragraphs[0].runs[0].font.size, 14.0);
        assert_eq!(b.paragraphs[0].para.h_align, HAlign::Right);
    }

    #[test]
    fn whitespace_collapses_outside_spaceruns() {
        let b = read(vec![
            el(
                "p",
                &[],
                vec![
                    t("  \n one   two\t\n"),
                    el("b", &[], vec![t("  three ")]),
                    t(" four  "),
                ],
            ),
            el(
                "p",
                &[],
                vec![
                    t("3."),
                    styled("span", "xfa-spacerun:yes", vec![t("   ")]),
                    t("Name"),
                ],
            ),
            el(
                "p",
                &[],
                vec![styled("span", "xfa-spacerun:yes", vec![t("  lead")])],
            ),
            el(
                "p",
                &[],
                vec![
                    t("a "),
                    styled("span", "xfa-spacerun:yes", vec![t(" ")]),
                    t("b"),
                ],
            ),
        ]);
        assert_eq!(b.plain(), "one two three four\n3.   Name\n  lead\na b");
        // The collapsed space before "three" is in the plain run's font.
        assert!(!b.paragraphs[0].runs[0].font.bold);
        assert_eq!(b.paragraphs[0].runs[0].text, "one two ");
    }

    #[test]
    fn non_breaking_spaces_are_kept() {
        let b = read(vec![el("p", &[], vec![t(" a\u{a0}\u{a0}b\u{a0}")])]);
        assert_eq!(b.plain(), "a\u{a0}\u{a0}b\u{a0}");
    }

    #[test]
    fn tab_counts_insert_tabs() {
        let b = read(vec![el(
            "p",
            &[],
            vec![t("a"), styled("span", "xfa-tab-count:2", vec![]), t("b")],
        )]);
        assert_eq!(b.plain(), "a\t\tb");
    }

    #[test]
    fn floating_fields_show_their_values() {
        let tree = root(vec![el(
            "p",
            &[],
            vec![
                t("Paid "),
                el(
                    "span",
                    &[
                        ("xfa:embed", "#floatingField1"),
                        ("xfa:embedMode", "raw"),
                        ("xfa:embedType", "uri"),
                    ],
                    vec![],
                ),
                t(" on "),
                el("span", &[("xfa:embed", "#missing")], vec![t("ignored")]),
                t("."),
            ],
        )]);
        let embed = |id: &str| (id == "floatingField1").then(|| "$1,200".to_string());
        let b = from_element(&tree, &Font::default(), &Para::default(), &embed);
        assert_eq!(b.plain(), "Paid $1,200 on .");
    }

    #[test]
    fn markup_through_the_xml_reader() {
        let html = "<body xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\"><p style=\"letter-spacing:0in\">Fees &amp; charges&nbsp;due<span style=\"xfa-spacerun:yes\">  </span><span style=\"font-weight:bold\">now</span></p><p style=\"letter-spacing:0in\"><br/></p><p>for <span xfa:embed=\"#f1\" xfa:embedMode=\"raw\" xfa:embedType=\"uri\"/>.</p></body>";
        let embed = |id: &str| (id == "f1").then(|| "Unit 4".to_string());
        let b = parse(html, &Font::default(), &Para::default(), &embed);
        assert_eq!(b.plain(), "Fees & charges\u{a0}due  now\n\nfor Unit 4.");
        assert!(font_of(&b, "now").bold);
        assert_eq!(plain_text(html), "Fees & charges\u{a0}due  now\n\nfor .");
        // More than one top-level element.
        assert_eq!(plain_text("<p>a</p><p>b</p>"), "a\nb");
        assert_eq!(plain_text("<?xml version=\"1.0\"?><p>x</p>"), "x");
        assert_eq!(plain_text(""), "");
    }

    #[test]
    fn plain_text_agrees_with_parse() {
        for html in [
            "<body><p>one <b>two</b></p><p/><p>a<br/>b</p></body>",
            "<p>x &lt;y&gt; &#8364;</p>",
            "<ul><li>a</li></ul>",
            "<p>broken <b>markup</p>",
            "plain &amp; simple",
            "<p>a</p><p",
        ] {
            let parsed = parse(html, &Font::default(), &Para::default(), &|_| None).plain();
            assert_eq!(plain_text(html), parsed, "{html}");
        }
    }

    #[test]
    fn malformed_markup_falls_back_to_the_tag_stripper() {
        let html = "<p>one  &amp;\n two<br>three</p><p>four <b>five</p><p></p>&nbsp;x &bogus;";
        assert_eq!(
            plain_text(html),
            "one & two\nthree\nfour five\n\n\u{a0}x &bogus;"
        );
        let b = parse(
            html,
            &Font {
                size: 7.0,
                ..Font::default()
            },
            &Para::default(),
            &|_| None,
        );
        assert_eq!(b.paragraphs.len(), 5);
        assert_eq!(b.paragraphs[0].runs[0].font.size, 7.0);
        assert_eq!(plain_text("<p>unclosed <"), "unclosed");
    }

    #[test]
    fn the_work_is_bounded() {
        let long = "a".repeat(MAX_BLOCK_CHARS + 100);
        let b = read(vec![el("p", &[], vec![t(&long)])]);
        assert!(b.plain().len() <= MAX_BLOCK_CHARS);
        assert!(strip_tags(&format!("<p>{long}")).len() <= MAX_BLOCK_CHARS);
        // Deep nesting stops at the depth limit without overflowing.
        let mut node = t("deep");
        for _ in 0..(MAX_XML_DEPTH + 50) {
            node = el("span", &[], vec![node]);
        }
        let b = read(vec![node]);
        assert_eq!(b.plain(), "");
    }

    #[test]
    fn colours_and_lengths() {
        assert_eq!(css_color("#00ff7f"), Some(Rgb(0, 255, 127)));
        assert_eq!(css_color("#abc"), Some(Rgb(0xaa, 0xbb, 0xcc)));
        assert_eq!(css_color("rgb(300,-4,50%)"), Some(Rgb(255, 0, 128)));
        assert_eq!(css_color("#12"), None);
        assert_eq!(css_color("chartreuse"), None);
        for (name, rgb) in [
            ("black", Rgb(0, 0, 0)),
            ("navy", Rgb(0, 0, 128)),
            ("fuchsia", Rgb(255, 0, 255)),
        ] {
            assert_eq!(css_color(name), Some(rgb));
        }
        assert_eq!(css_length("0in", 10.0), Some(0.0));
        assert_eq!(css_length("2em", 10.0), Some(20.0));
        assert_eq!(css_length("1in", 10.0), Some(72.0));
        assert_eq!(css_length("wide", 10.0), None);
    }
}

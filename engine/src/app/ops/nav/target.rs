// SPDX-License-Identifier: MIT
//! Where a link or a bookmark goes, read from its dictionary (/Dest, else
//! /A, and for links /AA /U or /D), with a link URI for what is not a page
//! here.
//!
//! A URI is never resolved back to a page: only a local destination that
//! names a page of this document is a page. Resolving the fragment of any
//! URI would make `https://x/#page=3`, a GoToR to another file and a
//! destination naming no page come back as pages of this one.
//!
//! Text strings are read from at most `MAX_TEXT_BYTES` bytes, so a string
//! shared by many links or bookmarks costs a bounded amount each time.
use std::cell::OnceCell;
use std::fmt::Write;

use crate::app::edit::get;
use crate::app::pagetree::{self, NamedDests, PageIndex};
use crate::document::Document;
use crate::object::{Dictionary, Object, decode_text_prefix};

/// A title, URI, file name or destination name is read from at most this
/// many bytes; the app writes at most 4096 characters (8194 bytes).
pub const MAX_TEXT_BYTES: usize = 64 * 1024;

/// What a clipped text ends with.
pub const CLIPPED: char = '\u{2026}';

/// A destination as the panels report it.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A page of this document, 0-based.
    Page(usize),
    /// Anything else, as a link URI: a web or mail address, another
    /// file, or a local destination that names no page.
    Uri(String),
    /// A URI whose text passed `MAX_TEXT_BYTES`: its start and `CLIPPED`.
    /// Listed as blocked, never followed.
    Clipped(String),
}

impl Target {
    /// The URI, whole or clipped.
    pub fn uri(&self) -> Option<&str> {
        match self {
            Target::Page(_) => None,
            Target::Uri(uri) | Target::Clipped(uri) => Some(uri),
        }
    }
}

/// `Uri`, or `Clipped` with its mark when a text in it was clipped.
fn uri_target(uri: String, clipped: bool) -> Target {
    if clipped {
        Target::Clipped(uri + &CLIPPED.to_string())
    } else {
        Target::Uri(uri)
    }
}

/// Reads targets against one document state.
pub struct Resolver<'d> {
    doc: &'d Document<'d>,
    pages: PageIndex,
    named: OnceCell<NamedDests<'d>>,
}

impl<'d> Resolver<'d> {
    pub fn new(doc: &'d Document<'d>) -> Self {
        Self {
            doc,
            pages: PageIndex::of(doc),
            named: OnceCell::new(),
        }
    }

    pub fn page_count(&self) -> usize {
        self.pages.count()
    }

    /// A link annotation's target: /Dest, else /A, else the /U or /D
    /// additional action; `page` is the link's own page, which PrevPage and
    /// NextPage count from. None when there is no link to list.
    pub fn link(&self, annot: &Dictionary, page: usize) -> Option<Target> {
        let doc = self.doc;
        let dest = doc.dict_get(annot, b"Dest");
        if !dest.is_null() {
            return self.dest(dest);
        }
        let mut action = doc.dict_get(annot, b"A");
        if action.is_null() {
            let additional = doc.dict_get(annot, b"AA");
            action = get(doc, additional, b"U");
            if action.is_null() {
                action = get(doc, additional, b"D");
            }
        }
        self.action(action, Some(page))
    }

    /// An outline item's target: /Dest, else /A.
    pub fn outline(&self, item: &Dictionary) -> Option<Target> {
        let dest = self.doc.dict_get(item, b"Dest");
        if !dest.is_null() {
            return self.dest(dest);
        }
        self.action(self.doc.dict_get(item, b"A"), None)
    }

    fn named(&self) -> &NamedDests<'d> {
        self.named.get_or_init(|| NamedDests::load(self.doc))
    }

    /// A local destination: an explicit array, or a name or string looked up
    /// in the named destinations. Anything else names nothing.
    fn dest(&self, dest: &Object) -> Option<Target> {
        let doc = self.doc;
        match doc.resolve(dest) {
            Object::Array(items) if !items.is_empty() => {
                let page = pagetree::dest_page(doc, &self.pages, &NamedDests::default(), dest);
                Some(match page {
                    Some(page) => Target::Page(page),
                    None => Target::Uri(explicit_uri("", doc, items)),
                })
            }
            Object::Name(_) | Object::String(_) => Some(
                match pagetree::dest_page(doc, &self.pages, self.named(), dest) {
                    Some(page) => Target::Page(page),
                    None => {
                        let (name, clipped) = dest_name(doc.resolve(dest));
                        uri_target(named_uri("", &name), clipped)
                    }
                },
            ),
            _ => None,
        }
    }

    /// An action's target; `page` is None for an outline item.
    fn action(&self, action: &Object, page: Option<usize>) -> Option<Target> {
        let doc = self.doc;
        let dict = doc.resolve(action).as_dict()?;
        let kind = doc.dict_get(dict, b"S").as_name()?;
        match kind {
            b"GoTo" => self.dest(doc.dict_get(dict, b"D")),
            b"URI" => {
                let (uri, clipped) = text(doc.dict_get(dict, b"URI"));
                if is_external(&uri) {
                    return Some(uri_target(uri, clipped));
                }
                let root = doc.resolve_id(doc.root());
                let base = get(doc, get(doc, root, b"URI"), b"Base");
                let (base, base_clipped) = if base.is_null() {
                    ("file://".to_string(), false)
                } else {
                    text(base)
                };
                Some(uri_target(base + &uri, clipped || base_clipped))
            }
            b"Launch" => file_uri(doc, doc.dict_get(dict, b"F"), None),
            b"GoToR" => {
                let dest = doc.dict_get(dict, b"D");
                file_uri(doc, doc.dict_get(dict, b"F"), Some(dest))
            }
            b"Named" => {
                let last = self.pages.count().checked_sub(1)?;
                match doc.dict_get(dict, b"N").as_name()? {
                    b"FirstPage" => Some(Target::Page(0)),
                    b"LastPage" => Some(Target::Page(last)),
                    b"PrevPage" => page.map(|page| Target::Page(page.saturating_sub(1))),
                    b"NextPage" => page.map(|page| Target::Page(page.saturating_add(1).min(last))),
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

/// A string's text, "" for anything else; `true`
/// when it was clipped.
fn text(value: &Object) -> (String, bool) {
    value
        .as_string()
        .map_or((String::new(), false), decode_text)
}

/// A text string, decoded from its first
/// `MAX_TEXT_BYTES` bytes, `true` when there were more.
pub fn decode_text(bytes: &[u8]) -> (String, bool) {
    decode_text_prefix(bytes, MAX_TEXT_BYTES)
}

/// A destination name: a name's bytes, a string's text; `true` when
/// clipped.
fn dest_name(value: &Object) -> (Vec<u8>, bool) {
    match value {
        Object::Name(name) => {
            let clipped = name.len() > MAX_TEXT_BYTES;
            (name.get(..MAX_TEXT_BYTES).unwrap_or(name).to_vec(), clipped)
        }
        _ => {
            let (name, clipped) = text(value);
            (name.into_bytes(), clipped)
        }
    }
}

/// `prefix` and a `nameddest` fragment.
fn named_uri(prefix: &str, name: &[u8]) -> String {
    let join = joiner(prefix);
    format!("{prefix}{join}nameddest={}", encode_component(name))
}

/// A fragment is appended with `&` to a URI that has one already.
fn joiner(prefix: &str) -> char {
    if prefix.contains('#') { '&' } else { '#' }
}

/// Whether the URI starts with an RFC 3986 scheme.
pub fn is_external(uri: &str) -> bool {
    let mut chars = uri.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    for c in chars {
        match c {
            ':' => return true,
            c if c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.') => {}
            _ => return false,
        }
    }
    false
}

/// A file specification and its destination as a link URI: `file:` and
/// the percent-encoded path (`file://` before an absolute one), then the
/// destination's fragment. /UF wins over /F; /Unix and /DOS are ignored. A
/// Launch (`dest` None) has no fragment. A specification with /FS /URL
/// names a URL, kept as written with the fragment joined to it; without a
/// usable destination (and for a Launch) it is the URL itself.
fn file_uri(doc: &Document<'_>, spec: &Object, dest: Option<&Object>) -> Option<Target> {
    let (name, url) = match spec {
        Object::String(_) => (spec, false),
        Object::Dictionary(dict) => {
            let unicode = doc.dict_get(dict, b"UF");
            let name = if unicode.is_null() {
                doc.dict_get(dict, b"F")
            } else {
                unicode
            };
            (name, doc.dict_get(dict, b"FS").as_name() == Some(b"URL"))
        }
        _ => return None,
    };
    let (path, clipped) = decode_text(name.as_string()?);
    let dest = dest.map(|dest| doc.resolve(dest));
    let file = if url {
        path
    } else {
        let scheme = if path.starts_with('/') {
            "file://"
        } else {
            "file:"
        };
        format!("{scheme}{}", encode(path.as_bytes(), true))
    };
    let (uri, name_clipped) = match dest {
        None => (file, false),
        Some(Object::Array(items)) => (explicit_uri(&file, doc, items), false),
        Some(name @ (Object::Name(_) | Object::String(_))) => {
            let (name, clipped) = dest_name(name);
            (named_uri(&file, &name), clipped)
        }
        Some(_) if url => (file, false),
        // A GoToR without a usable /D goes to page 1.
        Some(_) => (format!("{file}#page=1"), false),
    };
    Some(uri_target(uri, clipped || name_clipped))
}

/// The URI of a destination that is not a page of this document (another
/// file's, or a dead local one): the page number (1 when it is not a
/// number), then the view with the destination's own coordinates.
fn explicit_uri(prefix: &str, doc: &Document<'_>, items: &[Object]) -> String {
    let page = match items.first().map(|first| doc.resolve(first)) {
        Some(Object::Integer(page)) if *page >= 0 => page.saturating_add(1),
        _ => 1,
    };
    let number = |index: usize| -> Option<f32> {
        match items.get(index).map(|item| doc.resolve(item)) {
            None | Some(Object::Null) => None,
            Some(value) => Some(value.as_f64().unwrap_or(0.0) as f32),
        }
    };
    let real = |index: usize| number(index).unwrap_or(0.0);
    let start = format!("{prefix}{}page={page}", joiner(prefix));
    let kind = items
        .get(1)
        .map(|kind| doc.resolve(kind))
        .and_then(Object::as_name);
    match kind {
        Some(b"Fit") => format!("{start}&view=Fit"),
        Some(b"FitB") => format!("{start}&view=FitB"),
        Some(view @ (b"FitH" | b"FitBH" | b"FitV" | b"FitBV")) => {
            let view = String::from_utf8_lossy(view);
            match number(2) {
                Some(value) => format!("{start}&view={view},{}", g(value)),
                None => format!("{start}&view={view}"),
            }
        }
        Some(b"FitR") => {
            let (x0, y0, x1, y1) = (real(2), real(3), real(4), real(5));
            format!(
                "{start}&viewrect={},{},{},{}",
                g(x0.min(x1)),
                g(y0.min(y1)),
                g((x1 - x0).abs()),
                g((y1 - y0).abs())
            )
        }
        // XYZ, and an unknown or missing view, read as XYZ.
        _ => {
            let x = number(2).unwrap_or(f32::NAN);
            let y = number(3).unwrap_or(f32::NAN);
            let zoom = number(4)
                .filter(|zoom| *zoom > 0.0)
                .map_or(f32::NAN, |zoom| zoom * 100.0);
            if x.is_nan() && y.is_nan() && zoom.is_nan() {
                start
            } else {
                format!("{start}&zoom={},{},{}", g(zoom), g(x), g(y))
            }
        }
    }
}

/// A coordinate as a link URI prints it: the shortest digits that
/// read back as the same float, no leading zero before the point, `nan`.
fn g(value: f32) -> String {
    if value.is_nan() {
        return "nan".into();
    }
    let text = format!("{value}");
    if let Some(rest) = text.strip_prefix("0.") {
        format!(".{rest}")
    } else if let Some(rest) = text.strip_prefix("-0.") {
        format!("-.{rest}")
    } else {
        text
    }
}

/// Every byte but letters, digits and `-_.!~*'()` percent-encoded.
fn encode_component(bytes: &[u8]) -> String {
    encode(bytes, false)
}

/// Percent-encoding; a path keeps `/` as well.
fn encode(bytes: &[u8], path: bool) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &byte in bytes {
        let keep = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            )
            || (path && byte == b'/');
        if keep {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_print_in_shortest_form() {
        assert_eq!(g(123_456_789.0), "123456790");
        assert_eq!(g(0.000_001_234), ".000001234");
        assert_eq!(g(-0.1), "-.1");
        assert_eq!(g(150.0), "150");
        assert_eq!(g(f32::NAN), "nan");
    }

    #[test]
    fn schemes_and_encodings() {
        assert!(is_external("HTTPS://x"));
        assert!(is_external("a.b-c+d:e"));
        assert!(!is_external("1x:y"));
        assert!(!is_external("a_b:c"));
        assert!(!is_external(" https://x"));
        assert!(!is_external(""));
        assert_eq!(
            encode(b"a!$&'*+,;=:@-._~/b(c)[d]{e}#f?g<h>|i^j`k\\l\"m", true),
            "a!%24%26'*%2B%2C%3B%3D%3A%40-._~/b(c)%5Bd%5D%7Be%7D%23f%3Fg%3Ch%3E%7Ci%5Ej%60k%5Cl%22m"
        );
        assert_eq!(encode_component(b"n &=?#%()"), "n%20%26%3D%3F%23%25()");
        assert_eq!(encode_component("é".as_bytes()), "%C3%A9");
    }

    #[test]
    fn text_strings_decode_and_clip() {
        let text = |bytes: &[u8]| decode_text(bytes).0;
        assert_eq!(text(b"\xC3\xA9"), "é");
        assert_eq!(text(b"\xE9t\xE9"), "été");
        assert_eq!(text(b"\x18A"), "\u{2D8}A");
        assert_eq!(text(b"\x9FA\xAD"), "\u{9F}A\u{AD}");
        assert_eq!(
            text(b"\xED\xA0\xBD"),
            "í€½",
            "a UTF-8 surrogate is not UTF-8"
        );
        assert_eq!(text(b"\xFE\xFF\x00\xE9"), "é");
        // A clip inside a character keeps the characters before it.
        let mut long = "a".repeat(MAX_TEXT_BYTES - 1).into_bytes();
        long.extend_from_slice("éé".as_bytes());
        let (clipped, cut) = decode_text(&long);
        assert!(cut);
        assert_eq!(clipped, "a".repeat(MAX_TEXT_BYTES - 1));
        let mut wide = vec![0xFE, 0xFF];
        wide.extend(std::iter::repeat_n([0x00, 0xE9], MAX_TEXT_BYTES).flatten());
        let (clipped, cut) = decode_text(&wide);
        assert!(cut);
        assert_eq!(clipped, "é".repeat(MAX_TEXT_BYTES / 2 - 1));
        assert_eq!(decode_text(b"plain"), ("plain".to_string(), false));
    }

    #[test]
    fn fragments_join_with_an_ampersand_after_a_fragment() {
        assert_eq!(named_uri("", b"a b"), "#nameddest=a%20b");
        assert_eq!(
            named_uri("https://x/d#top", b"n"),
            "https://x/d#top&nameddest=n"
        );
    }
}

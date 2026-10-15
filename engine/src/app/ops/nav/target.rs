// SPDX-License-Identifier: AGPL-3.0-or-later
//! Where a link or a bookmark goes, read from its dictionary as MuPDF's
//! `pdf_load_link` and `pdf_load_outline` read it (/Dest, else /A, and for
//! links /AA /U or /D), with MuPDF's link URIs for what is not a page here.
//!
//! Unlike MuPDF, a URI is never resolved back to a page: only a local
//! destination that names a page of this document is a page. MuPDF resolves
//! the fragment of any URI, so `https://x/#page=3`, a GoToR to another file
//! and a destination naming no page all came back as pages of this one.
use std::cell::OnceCell;
use std::fmt::Write;

use crate::app::edit::get;
use crate::app::pagetree::{self, NamedDests, PageIndex};
use crate::document::Document;
use crate::object::{Dictionary, Object, decode_text_string};

/// A destination as the panels report it.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A page of this document, 0-based.
    Page(usize),
    /// Anything else, as MuPDF's link URI: a web or mail address, another
    /// file, or a local destination that names no page.
    Uri(String),
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

    /// A link annotation's target (`pdf_load_link`): /Dest, else /A, else
    /// the /U or /D additional action; `page` is the link's own page, which
    /// PrevPage and NextPage count from. None when MuPDF lists no link.
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
            Object::Name(name) => Some(self.named_dest(dest, name)),
            Object::String(text) => {
                let name = decode_text_string(text);
                Some(self.named_dest(dest, name.as_bytes()))
            }
            _ => None,
        }
    }

    fn named_dest(&self, dest: &Object, name: &[u8]) -> Target {
        match pagetree::dest_page(self.doc, &self.pages, self.named(), dest) {
            Some(page) => Target::Page(page),
            None => Target::Uri(format!("#nameddest={}", encode_component(name))),
        }
    }

    /// MuPDF's `pdf_parse_link_action`; `page` is None for an outline item.
    fn action(&self, action: &Object, page: Option<usize>) -> Option<Target> {
        let doc = self.doc;
        let dict = doc.resolve(action).as_dict()?;
        let kind = doc.dict_get(dict, b"S").as_name()?;
        match kind {
            b"GoTo" => self.dest(doc.dict_get(dict, b"D")),
            b"URI" => {
                let uri = text(doc.dict_get(dict, b"URI"));
                if is_external(&uri) {
                    return Some(Target::Uri(uri));
                }
                let root = doc.resolve_id(doc.root());
                let base = get(doc, get(doc, root, b"URI"), b"Base");
                let base = if base.is_null() {
                    "file://".to_string()
                } else {
                    text(base)
                };
                Some(Target::Uri(base + &uri))
            }
            b"Launch" => file_uri(doc, doc.dict_get(dict, b"F"), None).map(Target::Uri),
            b"GoToR" => {
                let dest = doc.dict_get(dict, b"D");
                file_uri(doc, doc.dict_get(dict, b"F"), Some(dest)).map(Target::Uri)
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

/// `pdf_to_text_string`: a string's text, "" for anything else.
fn text(value: &Object) -> String {
    value
        .as_string()
        .map(decode_text_string)
        .unwrap_or_default()
}

/// MuPDF's `fz_is_external_link`: the URI starts with an RFC 3986 scheme.
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

/// A file specification and its destination as MuPDF's link URI
/// (`pdf_parse_file_spec`): `file:` and the percent-encoded path (`file://`
/// before an absolute one), then the destination's fragment. /UF wins over
/// /F. A Launch (`dest` None) has no fragment; MuPDF adds `#page=1`.
fn file_uri(doc: &Document<'_>, spec: &Object, dest: Option<&Object>) -> Option<String> {
    let name = match spec {
        Object::String(_) => spec,
        Object::Dictionary(dict) => {
            let unicode = doc.dict_get(dict, b"UF");
            if unicode.is_null() {
                doc.dict_get(dict, b"F")
            } else {
                unicode
            }
        }
        _ => return None,
    };
    let path = decode_text_string(name.as_string()?);
    let scheme = if path.starts_with('/') {
        "file://"
    } else {
        "file:"
    };
    let file = format!("{scheme}{}", encode(path.as_bytes(), true));
    Some(match dest.map(|dest| doc.resolve(dest)) {
        None => file,
        Some(Object::Array(items)) => explicit_uri(&file, doc, items),
        Some(Object::Name(name)) => format!("{file}#nameddest={}", encode_component(name)),
        Some(Object::String(name)) => format!(
            "{file}#nameddest={}",
            encode_component(decode_text_string(name).as_bytes())
        ),
        // A GoToR without a usable /D: MuPDF's "no destination" is page 1.
        Some(_) => format!("{file}#page=1"),
    })
}

/// MuPDF's `format_explicit_dest_link_uri` for a destination that is not a
/// page of this document (another file's, or a dead local one): the page
/// number (1 when it is not a number, as MuPDF writes), then the view with
/// the destination's own coordinates.
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
    let start = format!("{prefix}#page={page}");
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
        // XYZ, and an unknown or missing view, which MuPDF reads as XYZ.
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

/// A coordinate as MuPDF prints it in a link URI: the shortest digits that
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

/// `fz_encode_uri_component`: every byte but letters, digits and
/// `-_.!~*'()` percent-encoded.
fn encode_component(bytes: &[u8]) -> String {
    encode(bytes, false)
}

/// `fz_encode_uri_pathname` keeps `/` as well.
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
    fn coordinates_print_as_mupdf_prints_them() {
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
}

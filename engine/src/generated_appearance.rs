// SPDX-License-Identifier: AGPL-3.0-or-later
//! Appearance streams generated for display when an annotation has none, or
//! when /NeedAppearances asks the viewer to regenerate field appearances.
//! Covered: text and choice fields, check boxes and radio buttons, signature
//! fields (decoration only) and Text (note) annotations. Layout constants are
//! measured against MuPDF 1.28.1. The output is never saved into the document.
//! ponytail: text uses built-in Helvetica (Times for Times DA fonts) whatever
//! the DA font is, WinAnsi characters only, no comb fields or rich text; add a
//! font pipeline for embedded DA fonts when a document needs one.

use std::collections::BTreeMap;

use crate::builtin_metrics::{BuiltinFace, BuiltinMetricLookup, BuiltinMetrics};
use crate::document::Document;
use crate::object::Object;
use crate::simple_encoding::{NamedEncoding, SimpleEncoding};

type Dictionary = BTreeMap<Vec<u8>, Object>;

/// A Form XObject (BBox [0 0 w h], mapped onto the annotation Rect).
#[derive(Clone, Debug)]
pub struct GeneratedAppearance {
    pub dictionary: Dictionary,
    pub content: Vec<u8>,
    /// A font for /Resources /Font /TPDFGen; the caller stores it indirectly
    /// because font outlines load only from indirect font objects.
    pub font: Option<Dictionary>,
}
pub const FONT_NAME: &[u8] = FONT;

const FONT: &[u8] = b"TPDFGen";
const PADDING: f64 = 2.;
const LINE_HEIGHT: f64 = 1.116;
const DESCENT: f64 = 0.21;
const MULTILINE: i64 = 1 << 12;
const PASSWORD: i64 = 1 << 13;
const RADIO: i64 = 1 << 15;
const PUSHBUTTON: i64 = 1 << 16;
const COMBO: i64 = 1 << 17;

fn resolve<'a>(doc: &'a Document<'_>, value: &'a Object) -> &'a Object {
    let mut value = value;
    for _ in 0..32 {
        match value {
            Object::Reference(id) => match doc.object(*id) {
                Ok(object) => value = &object.value,
                Err(_) => return &Object::Null,
            },
            _ => return value,
        }
    }
    &Object::Null
}
fn get<'a>(doc: &'a Document<'_>, dict: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    dict.get(key)
        .map(|v| resolve(doc, v))
        .filter(|v| !matches!(v, Object::Null))
}
/// A field attribute from the widget or its nearest ancestor.
fn inherited<'a>(
    doc: &'a Document<'_>,
    mut field: &'a Dictionary,
    key: &[u8],
) -> Option<&'a Object> {
    for _ in 0..64 {
        if let Some(value) = get(doc, field, key) {
            return Some(value);
        }
        match get(doc, field, b"Parent") {
            Some(Object::Dictionary(parent)) => field = parent,
            _ => return None,
        }
    }
    None
}
fn number(value: &Object) -> Option<f64> {
    match value {
        Object::Integer(v) => Some(*v as f64),
        Object::Real(v) if v.is_finite() => Some(*v),
        _ => None,
    }
}
/// A color array as a fill or stroke operator; empty arrays mean none.
fn color(doc: &Document<'_>, value: Option<&Object>, stroke: bool) -> Option<String> {
    let Some(Object::Array(items)) = value else {
        return None;
    };
    let values: Vec<f64> = items
        .iter()
        .filter_map(|v| number(resolve(doc, v)))
        .collect();
    let op = match (values.len(), stroke) {
        (1, false) => "g",
        (1, true) => "G",
        (3, false) => "rg",
        (3, true) => "RG",
        (4, false) => "k",
        (4, true) => "K",
        _ => return None,
    };
    let values: Vec<String> = values.iter().map(|v| fmt(v.clamp(0., 1.))).collect();
    Some(format!("{} {op}", values.join(" ")))
}
fn fmt(value: f64) -> String {
    let s = format!("{value:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}
/// PDF text string (PDFDocEncoding or UTF-16BE) to Unicode.
fn text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    bytes.iter().map(|&b| char::from(b)).collect()
}

/// The parsed default appearance: text color operator and font size.
struct Da {
    color: String,
    size: f64,
    times: bool,
}
fn parse_da(doc: &Document<'_>, da: &[u8], resources: Option<&Dictionary>) -> Da {
    let words: Vec<&str> = std::str::from_utf8(da)
        .unwrap_or("")
        .split_whitespace()
        .collect();
    let mut out = Da {
        color: "0 g".into(),
        size: 0.,
        times: false,
    };
    for (i, word) in words.iter().enumerate() {
        match *word {
            "Tf" if i >= 2 => {
                out.size = words[i - 1].parse().unwrap_or(0.);
                let name = words[i - 2].trim_start_matches('/').as_bytes();
                let base = resources
                    .and_then(|r| get(doc, r, b"Font"))
                    .and_then(|f| match f {
                        Object::Dictionary(f) => get(doc, f, name),
                        _ => None,
                    })
                    .and_then(|f| match f {
                        Object::Dictionary(f) => get(doc, f, b"BaseFont"),
                        _ => None,
                    });
                out.times = matches!(base, Some(Object::Name(n)) if n.starts_with(b"Times"))
                    || name.starts_with(b"TiRo")
                    || name.starts_with(b"Times");
            }
            "g" | "rg" | "k" => {
                let n = match *word {
                    "g" => 1,
                    "rg" => 3,
                    _ => 4,
                };
                if i >= n && words[i - n..i].iter().all(|w| w.parse::<f64>().is_ok()) {
                    out.color = words[i - n..=i].join(" ");
                }
            }
            _ => {}
        }
    }
    out
}

struct Writer {
    metrics: BuiltinMetrics,
    content: String,
}
impl Writer {
    fn new(times: bool) -> Self {
        let face = if times {
            BuiltinFace::TimesRoman
        } else {
            BuiltinFace::Helvetica
        };
        Self {
            metrics: BuiltinMetrics::from_encoding(
                face,
                SimpleEncoding::from_named(NamedEncoding::WinAnsi),
            ),
            content: String::new(),
        }
    }
    /// WinAnsi agrees with Latin-1 for printable ASCII and 0xA0-0xFF.
    fn encode(text: &str) -> Vec<u8> {
        text.chars()
            .map(|c| match c as u32 {
                0x20..=0x7E | 0xA0..=0xFF => c as u8,
                _ => b'?',
            })
            .collect()
    }
    fn width(&self, bytes: &[u8], size: f64) -> f64 {
        bytes
            .iter()
            .map(|&b| match self.metrics.lookup(b) {
                BuiltinMetricLookup::Advance(a) => a.value(),
                _ => 0.,
            })
            .sum::<f64>()
            * size
            / 1000.
    }
    fn show(&mut self, bytes: &[u8], x: f64, y: f64) {
        let hex: String = bytes.iter().map(|b| format!("{b:02X}")).collect();
        self.content
            .push_str(&format!("1 0 0 1 {} {} Tm <{hex}> Tj\n", fmt(x), fmt(y)));
    }
    /// Greedy word wrap to `width`; explicit line breaks are kept.
    fn wrap(&self, text: &str, size: f64, width: f64) -> Vec<Vec<u8>> {
        let mut lines = Vec::new();
        for paragraph in text.split(['\r', '\n']) {
            let mut line: Vec<u8> = Vec::new();
            for word in paragraph.split(' ') {
                let word = Self::encode(word);
                let candidate = if line.is_empty() {
                    word.clone()
                } else {
                    [line.as_slice(), b" ", &word].concat()
                };
                if !line.is_empty() && self.width(&candidate, size) > width {
                    lines.push(std::mem::replace(&mut line, word));
                } else {
                    line = candidate;
                }
            }
            lines.push(line);
        }
        lines
    }
}

/// Appearance for an annotation that has none (or must be regenerated).
/// Ok(None): the annotation correctly draws nothing. Err: unsupported here.
pub fn generate(
    doc: &Document<'_>,
    annot: &Dictionary,
    rect: [f64; 4],
) -> Result<Option<GeneratedAppearance>, &'static str> {
    let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
    let subtype = match get(doc, annot, b"Subtype") {
        Some(Object::Name(n)) => n.as_slice(),
        _ => return Err("annotation Subtype"),
    };
    let resources = Dictionary::new();
    let mut font = None;
    let content = match subtype {
        b"Text" => note(doc, annot, h),
        b"Widget" => match widget(doc, annot, w, h, &mut font)? {
            Some(content) => content,
            None => return Ok(None),
        },
        _ => return Err("annotation type without a generated appearance"),
    };
    if content.is_empty() {
        return Ok(None);
    }
    let mut dictionary = Dictionary::new();
    dictionary.insert(b"Type".to_vec(), Object::Name(b"XObject".to_vec()));
    dictionary.insert(b"Subtype".to_vec(), Object::Name(b"Form".to_vec()));
    dictionary.insert(
        b"BBox".to_vec(),
        Object::Array(vec![
            Object::Integer(0),
            Object::Integer(0),
            Object::Real(w),
            Object::Real(h),
        ]),
    );
    dictionary.insert(b"Resources".to_vec(), Object::Dictionary(resources));
    dictionary.insert(b"Length".to_vec(), Object::Integer(content.len() as i64));
    Ok(Some(GeneratedAppearance {
        dictionary,
        content: content.into_bytes(),
        font,
    }))
}

/// MuPDF's Note icon: a 16-unit square with four bars at the Rect's top left.
fn note(doc: &Document<'_>, annot: &Dictionary, h: f64) -> String {
    let fill = color(doc, get(doc, annot, b"C"), false)
        .map(|c| format!("{c} 0.5 {} 15 15 re f\n", fmt(h - 15.5)))
        .unwrap_or_default();
    let mut out = format!("q {fill}0 G 1 w 0.5 {} 15 15 re S\n", fmt(h - 15.5));
    for bar in [4.5, 6.5, 8.5, 10.5] {
        out.push_str(&format!("4 {y} m 12 {y} l S\n", y = fmt(h - bar)));
    }
    out.push_str("Q\n");
    out
}

fn widget(
    doc: &Document<'_>,
    annot: &Dictionary,
    w: f64,
    h: f64,
    resources: &mut Option<Dictionary>,
) -> Result<Option<String>, &'static str> {
    let field_type = match inherited(doc, annot, b"FT") {
        Some(Object::Name(n)) => n.clone(),
        _ => return Err("widget without a field type"),
    };
    let flags = inherited(doc, annot, b"Ff").and_then(number).unwrap_or(0.) as i64;
    let form = get(doc, doc_catalog(doc)?, b"AcroForm").and_then(|f| match f {
        Object::Dictionary(f) => Some(f),
        _ => None,
    });
    let mk = match get(doc, annot, b"MK") {
        Some(Object::Dictionary(mk)) => Some(mk),
        _ => None,
    };
    let mut out = String::new();
    if let Some(bg) = color(doc, mk.and_then(|m| get(doc, m, b"BG")), false) {
        out.push_str(&format!("q {bg} 0 0 {} {} re f Q\n", fmt(w), fmt(h)));
    }
    if let Some(bc) = color(doc, mk.and_then(|m| get(doc, m, b"BC")), true) {
        let border = match get(doc, annot, b"BS") {
            Some(Object::Dictionary(bs)) => get(doc, bs, b"W").and_then(number).unwrap_or(1.),
            _ => 1.,
        };
        if border > 0. {
            let half = border / 2.;
            out.push_str(&format!(
                "q {bc} {} w {} {} {} {} re S Q\n",
                fmt(border),
                fmt(half),
                fmt(half),
                fmt(w - border),
                fmt(h - border)
            ));
        }
    }
    let on = |value: Option<&Object>| matches!(value, Some(Object::Name(n)) if n != b"Off");
    match field_type.as_slice() {
        b"Btn" if flags & PUSHBUTTON != 0 => return Err("push button without an appearance"),
        b"Btn" => {
            if on(get(doc, annot, b"AS"))
                || (get(doc, annot, b"AS").is_none() && on(inherited(doc, annot, b"V")))
            {
                let s = w.min(h) / 20.;
                let (cx, cy) = ((w - 20. * s) / 2., (h - 20. * s) / 2.);
                let p = |x: f64, y: f64| format!("{} {}", fmt(cx + x * s), fmt(cy + y * s));
                if flags & RADIO != 0 {
                    // A filled dot, a quarter of the box, as four Bezier arcs.
                    let (r, k) = (5. * s, 5. * s * 0.5523);
                    let (x, y) = (w / 2., h / 2.);
                    out.push_str(&format!(
                        "q 0 g {} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c f Q\n",
                        fmt(x + r), fmt(y),
                        fmt(x + r), fmt(y + k), fmt(x + k), fmt(y + r), fmt(x), fmt(y + r),
                        fmt(x - k), fmt(y + r), fmt(x - r), fmt(y + k), fmt(x - r), fmt(y),
                        fmt(x - r), fmt(y - k), fmt(x - k), fmt(y - r), fmt(x), fmt(y - r),
                        fmt(x + k), fmt(y - r), fmt(x + r), fmt(y - k), fmt(x + r), fmt(y),
                    ));
                } else {
                    // The ZapfDingbats check MuPDF draws, as a stroked path.
                    out.push_str(&format!(
                        "q 0 G {} w 1 J 1 j {} m {} l {} l S Q\n",
                        fmt(2.2 * s),
                        p(4.8, 8.6),
                        p(7.4, 4.9),
                        p(16.3, 16.9)
                    ));
                }
            }
        }
        b"Sig" => {}
        b"Tx" | b"Ch" => {
            let dr = form.and_then(|f| match get(doc, f, b"DR") {
                Some(Object::Dictionary(dr)) => Some(dr),
                _ => None,
            });
            let da = inherited(doc, annot, b"DA").or_else(|| form.and_then(|f| get(doc, f, b"DA")));
            let da = match da {
                Some(Object::String(s)) => parse_da(doc, s, dr),
                _ => parse_da(doc, b"", None),
            };
            let quadding = inherited(doc, annot, b"Q")
                .or_else(|| form.and_then(|f| get(doc, f, b"Q")))
                .and_then(number)
                .unwrap_or(0.) as i64;
            let value = |v: Option<&Object>| -> Vec<String> {
                match v {
                    Some(Object::String(s)) => vec![text(s)],
                    Some(Object::Array(items)) => items
                        .iter()
                        .filter_map(|i| match resolve(doc, i) {
                            Object::String(s) => Some(text(s)),
                            _ => None,
                        })
                        .collect(),
                    _ => Vec::new(),
                }
            };
            let selected = value(inherited(doc, annot, b"V"));
            let writer = Writer::new(da.times);
            let mut lines: Vec<(Vec<u8>, bool)> = Vec::new();
            let list = field_type == b"Ch" && flags & COMBO == 0;
            let multiline = list || (field_type == b"Tx" && flags & MULTILINE != 0);
            if list {
                // Each option is a string or an [export display] pair.
                let options = match inherited(doc, annot, b"Opt") {
                    Some(Object::Array(items)) => items.as_slice(),
                    _ => &[],
                };
                let skip = inherited(doc, annot, b"TI")
                    .and_then(number)
                    .unwrap_or(0.)
                    .max(0.) as usize;
                for option in options.iter().skip(skip) {
                    let (export, display) = match resolve(doc, option) {
                        Object::String(s) => (text(s), text(s)),
                        Object::Array(pair) if pair.len() == 2 => {
                            match (resolve(doc, &pair[0]), resolve(doc, &pair[1])) {
                                (Object::String(e), Object::String(d)) => (text(e), text(d)),
                                _ => continue,
                            }
                        }
                        _ => continue,
                    };
                    lines.push((Writer::encode(&display), selected.contains(&export)));
                }
            } else {
                let mut shown = selected.first().cloned().unwrap_or_default();
                if field_type == b"Ch" {
                    // A combo box shows the display text of the selected export value.
                    if let Some(Object::Array(options)) = inherited(doc, annot, b"Opt") {
                        for option in options {
                            if let Object::Array(pair) = resolve(doc, option) {
                                if let [e, d] = pair.as_slice() {
                                    if let (Object::String(e), Object::String(d)) =
                                        (resolve(doc, e), resolve(doc, d))
                                    {
                                        if text(e) == shown {
                                            shown = text(d);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if flags & PASSWORD != 0 {
                    shown = "*".repeat(shown.chars().count());
                }
                if shown.is_empty() {
                    return Ok(Some(out));
                }
                let size = if da.size > 0. {
                    da.size
                } else if multiline {
                    12.
                } else {
                    ((h - 2. * PADDING) / LINE_HEIGHT).clamp(1., 12.)
                };
                if multiline {
                    lines = writer
                        .wrap(&shown, size, w - 2. * PADDING)
                        .into_iter()
                        .map(|l| (l, false))
                        .collect();
                } else {
                    let bytes = Writer::encode(&shown);
                    // Auto size also shrinks a long single line to fit.
                    let fit = writer.width(&bytes, 1.);
                    let size = if da.size > 0. || fit <= 0. {
                        size
                    } else {
                        size.min((w - 2. * PADDING) / fit).max(1.)
                    };
                    lines.push((bytes, false));
                    return Ok(Some(text_block(
                        out, writer, &da, size, lines, quadding, w, h, false, resources,
                    )));
                }
            }
            let size = if da.size > 0. { da.size } else { 12. };
            return Ok(Some(text_block(
                out, writer, &da, size, lines, quadding, w, h, true, resources,
            )));
        }
        _ => return Err("unknown field type"),
    }
    Ok(Some(out))
}

#[allow(clippy::too_many_arguments)]
fn text_block(
    mut out: String,
    mut writer: Writer,
    da: &Da,
    size: f64,
    lines: Vec<(Vec<u8>, bool)>,
    quadding: i64,
    w: f64,
    h: f64,
    multiline: bool,
    resources: &mut Option<Dictionary>,
) -> String {
    let base = if da.times {
        b"Times-Roman".as_slice()
    } else {
        b"Helvetica"
    };
    let mut font = Dictionary::new();
    font.insert(b"Type".to_vec(), Object::Name(b"Font".to_vec()));
    font.insert(b"Subtype".to_vec(), Object::Name(b"Type1".to_vec()));
    font.insert(b"BaseFont".to_vec(), Object::Name(base.to_vec()));
    font.insert(
        b"Encoding".to_vec(),
        Object::Name(b"WinAnsiEncoding".to_vec()),
    );
    *resources = Some(font);
    let step = LINE_HEIGHT * size;
    out.push_str(&format!(
        "/Tx BMC q 1 1 {} {} re W n\n",
        fmt(w - 2.),
        fmt(h - 2.)
    ));
    // Selected list items get MuPDF's highlight behind their line.
    for (i, (_, selected)) in lines.iter().enumerate() {
        if *selected {
            let top = h - PADDING - i as f64 * step;
            out.push_str(&format!(
                "q 0.6 0.757 0.855 rg 1 {} {} {} re f Q\n",
                fmt(top - step),
                fmt(w - 2.),
                fmt(step)
            ));
        }
    }
    out.push_str(&format!(
        "BT {} /{} {} Tf\n",
        da.color,
        String::from_utf8_lossy(FONT),
        fmt(size)
    ));
    writer.content.clear();
    for (i, (bytes, _)) in lines.iter().enumerate() {
        let y = if multiline {
            h - PADDING - (i as f64 + 1.) * step
        } else {
            (h - size) / 2. + DESCENT * size
        };
        let width = writer.width(bytes, size);
        let x = match quadding {
            1 => (w - width) / 2.,
            2 => w - PADDING - width,
            _ => PADDING,
        };
        writer.show(bytes, x, y);
    }
    out.push_str(&writer.content);
    out.push_str("ET Q EMC\n");
    out
}

fn doc_catalog<'a>(doc: &'a Document<'_>) -> Result<&'a Dictionary, &'static str> {
    match doc.object(doc.root()).map(|o| &o.value) {
        Ok(Object::Dictionary(d)) => Ok(d),
        _ => Err("catalog"),
    }
}

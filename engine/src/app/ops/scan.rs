// SPDX-License-Identifier: AGPL-3.0-or-later
//! `scan-compose` and `scan-measure`: the mobile scanner's PDF
//! (engine/design/app-m9-mobile-release.md, "The scanner"). No session
//! state is read or changed.
//!
//! `scan-compose` writes one page per request page: the page's JPEG stored
//! as it is (DCTDecode, an embedded ICC profile as ICCBased) filling the
//! page, stroked boxes, and text runs, invisible (render mode 3, a
//! searchable scan's OCR lines) or filled. Each character takes the first
//! font that draws it:
//!
//! 1. Helvetica or Helvetica-Bold in WinAnsiEncoding, not embedded;
//! 2. a Latin letter WinAnsi lacks: the same base font with the
//!    `Eight::Latin` /Differences (glyph names, not embedded), as FreeText
//!    and widget appearances draw it;
//! 3. a subset of the engine's own URW Nimbus Sans outlines (Greek,
//!    Cyrillic, more Latin, arrows, maths, currency), embedded
//!    as a Type1C simple font of at most 255 glyphs;
//! 4. a subset of the installed CJK fallback font (Droid Sans Fallback,
//!    `set_cjk_font`), a Type0 font with Identity-H; a character it lacks
//!    draws as its .notdef.
//!
//! Every font but the WinAnsi ones has a ToUnicode CMap, so search and
//! copy return the input text. `scan-measure` gives the advance each
//! character is drawn with, so the scanner can lay out rebuilt pages.
//!
//! When a character needs the CJK font and none is installed, the op fails
//! (`no-font`) and turns `cjk_font_wanted` on: the adapter installs the
//! font and dispatches again, as it does for a render.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::OnceLock;

use serde_json::{Value, json};

use crate::app::edit::{Builder, Edit};
use crate::app::forms::scripts::Eight;
use crate::app::forms::text::{Family, Metrics, win_ansi_code};
use crate::app::ops::pages::{MAX_IMAGE_BYTES, images, written};
use crate::app::pagetree::PageTree;
use crate::app::{AppError, Buffers, MAX_SAVE_BYTES, reply, request};
use crate::object::{Dictionary, Object, ObjectId};
use crate::view::scan_fonts::{self, SubsetError, TrueType};

#[path = "scan_bidi.rs"]
mod bidi;

/// Pages one scan may have (the scanner's 100-page output limit).
pub const MAX_SCAN_PAGES: usize = 100;
/// Bytes of all the images together: the file stores each one as it is,
/// so more could never be saved under the 50 MiB ceiling.
pub const MAX_SCAN_IMAGE_BYTES: usize = MAX_SAVE_BYTES;
/// Text runs, and boxes, on one page.
pub const MAX_PAGE_RUNS: usize = 4000;
/// Characters in one run (the scanner's longest OCR line).
pub const MAX_RUN_CHARS: usize = 4000;
/// Characters in all runs, or measured at once: 12 scans of 24,000.
pub const MAX_SCAN_TEXT: usize = 300_000;
/// The largest JPEG side, in pixels (the scanner's canvas makes at most 2,400).
pub const MAX_JPEG_SIDE: u32 = 16_384;
/// The largest page side and coordinate, in points (pdf-from-images').
const MAX_POINTS: f64 = 14_400.0;
/// The subset holds .notdef, one glyph per character and the components
/// of composite glyphs, at most 65,535 in all; this leaves room for the
/// components (Droid Sans Fallback's CJK glyphs have none).
pub const MAX_CJK_CHARS: usize = 60_000;
/// Runs and boxes in one request: each is a JSON object the adapter
/// encodes and the engine parses (400,000 boxes took 35 s).
pub const MAX_SCAN_ITEMS: usize = 40_000;

fn invalid(message: impl std::fmt::Display) -> AppError {
    AppError::reason("invalid", format!("scan-compose: {message}"))
}

/// A `too-large` refusal; `detail` names the limit (`pages`, `images`,
/// `image-size`, `runs`, `text`, `characters`, `output`) for the scanner's
/// message.
fn too_large(op: &str, detail: &str, message: impl std::fmt::Display) -> AppError {
    AppError::detail("too-large", detail, format!("{op}: {message}"))
}

/// Where a character is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    Helvetica(u8),
    Latin(u8),
    Nimbus(usize),
    Cjk,
}

fn helvetica(bold: bool) -> &'static Metrics {
    static METRICS: OnceLock<[Metrics; 2]> = OnceLock::new();
    &METRICS.get_or_init(|| {
        [
            Metrics::family(Family::Helvetica),
            Metrics::family(Family::HelveticaBold),
        ]
    })[usize::from(bold)]
}

fn family(bold: bool) -> Family {
    if bold {
        Family::HelveticaBold
    } else {
        Family::Helvetica
    }
}

fn latin_widths(bold: bool) -> &'static [f64; 256] {
    static WIDTHS: OnceLock<[[f64; 256]; 2]> = OnceLock::new();
    &WIDTHS.get_or_init(|| {
        [
            Eight::Latin.widths(Family::Helvetica),
            Eight::Latin.widths(Family::HelveticaBold),
        ]
    })[usize::from(bold)]
}

/// The first font of the module's list that draws `c`.
fn route(c: char, bold: bool) -> Route {
    if let Some(code) = win_ansi_code(c).filter(|&code| code >= 0x20) {
        return Route::Helvetica(code);
    }
    if !c.is_ascii()
        && let Some(code) = Eight::Latin.code(c)
        && latin_widths(bold)[usize::from(code)] > 0.0
    {
        return Route::Latin(code);
    }
    match scan_fonts::outlines(bold).glyph(c) {
        Some(glyph) => Route::Nimbus(glyph),
        None => Route::Cjk,
    }
}

/// A character's advance in 1/1000 em; `cjk` is the CJK font (needed only
/// for `Route::Cjk`).
fn advance(c: char, bold: bool, cjk: Option<&TrueType<'_>>) -> f64 {
    match route(c, bold) {
        Route::Helvetica(code) => helvetica(bold).advance(code),
        Route::Latin(code) => latin_widths(bold)[usize::from(code)],
        Route::Nimbus(glyph) => scan_fonts::outlines(bold).advance(glyph),
        Route::Cjk => cjk.map_or(0.0, |font| font.advance(font.glyph(c))),
    }
}

/// Text as the fonts take it: a tab is a space; other control characters
/// (C0, DEL, C1) are dropped, as no font draws them.
fn clean(text: &str) -> impl Iterator<Item = char> + '_ {
    text.chars().filter_map(|c| match c {
        '\t' => Some(' '),
        // A soft hyphen shows only where a line breaks at it, and runs are
        // lines already.
        '\u{AD}' => None,
        c if c.is_control() => None,
        c => Some(c),
    })
}

/// The installed CJK font's bytes when `needed`; `no-font` when it is
/// needed and not installed (the adapter's cue to install it).
fn cjk_bytes(op: &str, needed: bool) -> Result<Option<hayro_interpret::font::FontData>, AppError> {
    if !needed {
        return Ok(None);
    }
    scan_fonts::cjk_font().map(Some).ok_or_else(|| {
        AppError::reason(
            "no-font",
            format!("{op}: the text needs the CJK fallback font, which is not installed"),
        )
    })
}

fn read_cjk<'a>(op: &str, bytes: Option<&'a [u8]>) -> Result<Option<TrueType<'a>>, AppError> {
    match bytes {
        None => Ok(None),
        Some(bytes) => TrueType::read(bytes).map(Some).ok_or_else(|| {
            AppError::reason(
                "no-font",
                format!("{op}: the installed CJK fallback font is not a TrueType font"),
            )
        }),
    }
}

/// `scan-measure`: the advance (1/1000 em) of each distinct character of
/// `text` in the regular and the bold fonts `scan-compose` draws it with,
/// and which font that is.
pub fn measure(request: &Value) -> Result<Value, AppError> {
    let text = request::string(request, "text")
        .ok_or_else(|| AppError::reason("invalid", "scan-measure: text must be a string"))?;
    if text.len() > 4 * MAX_SCAN_TEXT || text.chars().count() > MAX_SCAN_TEXT {
        return Err(too_large(
            "scan-measure",
            "text",
            format!("the text is longer than {MAX_SCAN_TEXT} characters"),
        ));
    }
    let mut chars: Vec<char> = Vec::new();
    let mut seen = BTreeSet::new();
    for c in clean(text) {
        if seen.insert(c) {
            chars.push(c);
        }
    }
    let needed = chars
        .iter()
        .any(|&c| route(c, false) == Route::Cjk || route(c, true) == Route::Cjk);
    let data = cjk_bytes("scan-measure", needed)?;
    let cjk = read_cjk("scan-measure", data.as_ref().map(|data| (**data).as_ref()))?;
    let widths = |bold| -> Vec<Value> {
        (chars.iter())
            .map(|&c| request::js_number_value(advance(c, bold, cjk.as_ref())))
            .collect()
    };
    let (regular, bold) = (widths(false), widths(true));
    // Which font draws each character (regular style): `missing` is the
    // CJK font's .notdef, a box.
    let fonts: Vec<&str> = (chars.iter())
        .map(|&c| match route(c, false) {
            Route::Helvetica(_) => "helvetica",
            Route::Latin(_) => "latin",
            Route::Nimbus(_) => "nimbus",
            Route::Cjk if cjk.as_ref().is_some_and(|font| font.glyph(c) != 0) => "cjk",
            Route::Cjk => "missing",
        })
        .collect();
    let chars: Vec<String> = chars.iter().map(char::to_string).collect();
    Ok(reply(
        request,
        json!({ "chars": chars, "regular": regular, "bold": bold, "fonts": fonts }),
    ))
}

struct Run {
    text: Vec<char>,
    x: f64,
    y: f64,
    size: f64,
    bold: bool,
    invisible: bool,
    fit: Option<f64>,
    color: [f64; 3],
}

struct Frame {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    line: f64,
    color: [f64; 3],
}

struct Page {
    width: f64,
    height: f64,
    image: Option<usize>,
    runs: Vec<Run>,
    boxes: Vec<Frame>,
}

fn number(value: Option<&Value>, what: &str, range: (f64, f64)) -> Result<f64, AppError> {
    value
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v >= range.0 && *v <= range.1)
        .ok_or_else(|| {
            invalid(format!(
                "{what} must be a number in {}..{}",
                range.0, range.1
            ))
        })
}

fn color(value: Option<&Value>, what: &str) -> Result<[f64; 3], AppError> {
    if request::nullish(value) {
        return Ok([0.0; 3]);
    }
    let items = value
        .and_then(Value::as_array)
        .filter(|items| items.len() == 3);
    let items = items.ok_or_else(|| invalid(format!("{what} must be three numbers in 0..1")))?;
    let mut out = [0.0; 3];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = number(Some(item), what, (0.0, 1.0))?;
    }
    Ok(out)
}

fn flag(value: Option<&Value>, what: &str) -> Result<bool, AppError> {
    match value {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(flag)) => Ok(*flag),
        Some(_) => Err(invalid(format!("{what} must be a boolean"))),
    }
}

/// The request's pages, checked; `text` counts the characters of all runs.
fn read_pages(request: &Value, images: usize) -> Result<Vec<Page>, AppError> {
    let pages = request::array(request, "pages")
        .filter(|pages| !pages.is_empty())
        .ok_or_else(|| invalid(format!("pages must be 1..{MAX_SCAN_PAGES} pages")))?;
    if pages.len() > MAX_SCAN_PAGES {
        return Err(too_large(
            "scan-compose",
            "pages",
            format!("a scan has at most {MAX_SCAN_PAGES} pages"),
        ));
    }
    let items: usize = (pages.iter())
        .flat_map(|page| ["runs", "boxes"].map(|key| page.get(key).and_then(Value::as_array)))
        .map(|list| list.map_or(0, Vec::len))
        .sum();
    if items > MAX_SCAN_ITEMS {
        return Err(too_large(
            "scan-compose",
            "runs",
            format!("a scan has at most {MAX_SCAN_ITEMS} runs and boxes in all"),
        ));
    }
    let coordinate = (-MAX_POINTS, MAX_POINTS);
    let mut text = 0usize;
    let mut out = Vec::with_capacity(pages.len());
    for (index, page) in pages.iter().enumerate() {
        let at = |what: &str| format!("page {index}: {what}");
        let width = number(page.get("width"), &at("width"), (0.0, MAX_POINTS))?;
        let height = number(page.get("height"), &at("height"), (0.0, MAX_POINTS))?;
        if width <= 0.0 || height <= 0.0 {
            return Err(invalid(at("the page size must be positive")));
        }
        let image = match page.get("image") {
            value if request::nullish(value) => None,
            Some(value) => Some(
                value
                    .as_u64()
                    .and_then(|i| usize::try_from(i).ok())
                    .filter(|&i| i < images)
                    .ok_or_else(|| invalid(at("image must index images")))?,
            ),
            None => None,
        };
        let list = |key: &str| -> Result<&[Value], AppError> {
            match page.get(key) {
                value if request::nullish(value) => Ok(&[]),
                Some(Value::Array(items)) if items.len() <= MAX_PAGE_RUNS => Ok(items),
                Some(Value::Array(_)) => Err(too_large(
                    "scan-compose",
                    "runs",
                    at(&format!("more than {MAX_PAGE_RUNS} {key}")),
                )),
                _ => Err(invalid(at(&format!("{key} must be an array")))),
            }
        };
        let mut runs = Vec::new();
        for (r, run) in list("runs")?.iter().enumerate() {
            let what = |key: &str| at(&format!("run {r} {key}"));
            let value = run
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid(what("text must be a string")))?;
            if value.len() > 4 * MAX_RUN_CHARS || value.chars().count() > MAX_RUN_CHARS {
                return Err(too_large(
                    "scan-compose",
                    "text",
                    what(&format!("text is longer than {MAX_RUN_CHARS} characters")),
                ));
            }
            text += value.chars().count();
            if text > MAX_SCAN_TEXT {
                return Err(too_large(
                    "scan-compose",
                    "text",
                    format!("the text is longer than {MAX_SCAN_TEXT} characters"),
                ));
            }
            let fit = match run.get("width") {
                value if request::nullish(value) => None,
                value => Some(number(value, &what("width"), (0.0, 2.0 * MAX_POINTS))?),
            };
            let size = number(run.get("size"), &what("size"), (0.0, MAX_POINTS))?;
            if size <= 0.0 {
                return Err(invalid(what("size must be positive")));
            }
            runs.push(Run {
                text: clean(value).collect(),
                x: number(run.get("x"), &what("x"), coordinate)?,
                y: number(run.get("y"), &what("y"), coordinate)?,
                size,
                bold: flag(run.get("bold"), &what("bold"))?,
                invisible: flag(run.get("invisible"), &what("invisible"))?,
                fit: fit.filter(|&fit| fit > 0.0),
                color: color(run.get("color"), &what("color"))?,
            });
        }
        let mut boxes = Vec::new();
        for (b, frame) in list("boxes")?.iter().enumerate() {
            let what = |key: &str| at(&format!("box {b} {key}"));
            let line = match frame.get("lineWidth") {
                value if request::nullish(value) => 1.0,
                value => number(value, &what("lineWidth"), (0.0, 100.0))?,
            };
            boxes.push(Frame {
                x: number(frame.get("x"), &what("x"), coordinate)?,
                y: number(frame.get("y"), &what("y"), coordinate)?,
                width: number(frame.get("width"), &what("width"), (0.0, 2.0 * MAX_POINTS))?,
                height: number(
                    frame.get("height"),
                    &what("height"),
                    (0.0, 2.0 * MAX_POINTS),
                )?,
                line,
                color: color(frame.get("color"), &what("color"))?,
            });
        }
        out.push(Page {
            width,
            height,
            image,
            runs,
            boxes,
        });
    }
    Ok(out)
}

/// A font resource of the scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Key {
    Helvetica(bool),
    Latin(bool),
    /// The style and the font's number.
    Nimbus(bool, usize),
    Cjk,
}

impl Key {
    fn resource(self) -> String {
        let style = |bold: bool| if bold { 2 } else { 1 };
        match self {
            Self::Helvetica(bold) => format!("F{}", style(bold)),
            Self::Latin(bold) => format!("E{}", style(bold)),
            Self::Nimbus(bold, n) => format!("N{}_{n}", style(bold)),
            Self::Cjk => "F0".into(),
        }
    }
}

/// The Nimbus subsets of one style: each glyph's font and code, and each
/// font's glyphs (code `i + 1` is glyph `i`) with their characters.
#[derive(Default)]
struct NimbusFonts {
    codes: HashMap<usize, (usize, u8)>,
    fonts: Vec<Vec<(usize, char)>>,
}

/// The fonts a scan uses so far, and their objects.
struct Fonts<'f> {
    cjk: Option<TrueType<'f>>,
    cjk_cids: HashMap<char, u16>,
    cjk_chars: Vec<char>,
    nimbus: [NimbusFonts; 2],
    /// The codes used in the WinAnsi and Latin-name fonts, per style, with
    /// their characters (for ToUnicode).
    helvetica: [BTreeMap<u8, char>; 2],
    latin: [BTreeMap<u8, char>; 2],
    objects: BTreeMap<Key, ObjectId>,
}

impl Fonts<'_> {
    /// `c`'s font, code and advance (1/1000 em).
    fn encode(&mut self, c: char, bold: bool) -> Result<(Key, u32, f64), AppError> {
        let style = usize::from(bold);
        let width = advance(c, bold, self.cjk.as_ref());
        Ok(match route(c, bold) {
            Route::Helvetica(code) => {
                self.helvetica[style].insert(code, c);
                (Key::Helvetica(bold), u32::from(code), width)
            }
            Route::Latin(code) => {
                self.latin[style].insert(code, c);
                (Key::Latin(bold), u32::from(code), width)
            }
            Route::Nimbus(glyph) => {
                let set = &mut self.nimbus[style];
                let (font, code) = match set.codes.get(&glyph) {
                    Some(&found) => found,
                    None => {
                        if set.fonts.last().is_none_or(|font| font.len() >= 255) {
                            set.fonts.push(Vec::new());
                        }
                        let font = set.fonts.len() - 1;
                        let glyphs = &mut set.fonts[font];
                        glyphs.push((glyph, c));
                        let code = u8::try_from(glyphs.len()).unwrap_or(u8::MAX);
                        set.codes.insert(glyph, (font, code));
                        (font, code)
                    }
                };
                (Key::Nimbus(bold, font), u32::from(code), width)
            }
            Route::Cjk => {
                let cid = match self.cjk_cids.get(&c) {
                    Some(&cid) => cid,
                    None => {
                        if self.cjk_chars.len() >= MAX_CJK_CHARS {
                            return Err(too_large(
                                "scan-compose",
                                "characters",
                                format!(
                                    "more than {MAX_CJK_CHARS} different characters need the CJK font"
                                ),
                            ));
                        }
                        self.cjk_chars.push(c);
                        let cid = u16::try_from(self.cjk_chars.len()).unwrap_or(u16::MAX);
                        self.cjk_cids.insert(c, cid);
                        cid
                    }
                };
                (Key::Cjk, u32::from(cid), width)
            }
        })
    }

    /// The font's object, allocated on first use and written by `finish`.
    fn object(&mut self, out: &mut Builder, key: Key) -> Result<ObjectId, AppError> {
        if let Some(&id) = self.objects.get(&key) {
            return Ok(id);
        }
        let id = out.allocate(Object::Null)?;
        self.objects.insert(key, id);
        Ok(id)
    }
}

/// `value` with at most four decimals, as the scanner wrote numbers.
fn num(value: f64) -> String {
    let text = format!("{value:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    match text {
        "-0" | "" => "0".into(),
        text => text.into(),
    }
}

/// A PDF number object: an integer when the value is one.
fn pdf_number(value: f64) -> Object {
    let value = (value * 10_000.0).round() / 10_000.0;
    if value.fract() == 0.0 {
        Object::Integer(value as i64)
    } else {
        Object::real(value).unwrap_or(Object::Integer(0))
    }
}

/// A Flate-compressed stream object of `data` with `dictionary`.
fn stream(
    out: &mut Builder,
    mut dictionary: Dictionary,
    data: &[u8],
) -> Result<ObjectId, AppError> {
    let packed = images::deflate(data, MAX_SAVE_BYTES).map_err(|_| {
        too_large(
            "scan-compose",
            "output",
            format!("the PDF would exceed the {MAX_SAVE_BYTES} byte save ceiling"),
        )
    })?;
    dictionary.insert(b"Filter".to_vec(), Object::name("FlateDecode"));
    let id = out.allocate(Object::Null)?;
    out.put_stream_raw(id, dictionary, packed)?;
    Ok(id)
}

/// The six-letter subset tag (ISO 32000-1, 9.6.4) hashed from the
/// subset's characters, as the TypeScript scanner made it.
fn subset_tag(chars: impl IntoIterator<Item = char>) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for c in chars {
        let c = u32::from(c);
        for shift in [0, 8, 16, 24] {
            hash = (hash ^ ((c >> shift) & 0xFF)).wrapping_mul(0x0100_0193);
        }
    }
    let mut tag = String::new();
    for i in 0..6u32 {
        tag.push(char::from(b'A' + (hash % 26) as u8));
        let next = hash / 26;
        hash = if next != 0 {
            next
        } else {
            (hash ^ i).wrapping_mul(0x0100_0193)
        };
    }
    tag
}

/// A ToUnicode CMap mapping each code (one or two bytes) to its character.
fn to_unicode(entries: &[(u32, char)], two_bytes: bool) -> Vec<u8> {
    let digits = if two_bytes { 4 } else { 2 };
    let mut lines = vec![
        "/CIDInit /ProcSet findresource begin".to_string(),
        "12 dict begin".into(),
        "begincmap".into(),
        "/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def".into(),
        "/CMapName /Adobe-Identity-UCS def".into(),
        "/CMapType 2 def".into(),
        "1 begincodespacerange".into(),
        if two_bytes {
            "<0000> <FFFF>".into()
        } else {
            "<00> <FF>".into()
        },
        "endcodespacerange".into(),
    ];
    for chunk in entries.chunks(100) {
        lines.push(format!("{} beginbfchar", chunk.len()));
        for &(code, c) in chunk {
            let mut units = [0u16; 2];
            let utf16: String = (c.encode_utf16(&mut units).iter())
                .map(|unit| format!("{unit:04X}"))
                .collect();
            lines.push(format!("<{code:0digits$X}> <{utf16}>"));
        }
        lines.push("endbfchar".into());
    }
    for line in [
        "endcmap",
        "CMapName currentdict /CMap defineresource pop",
        "end",
        "end",
    ] {
        lines.push(line.into());
    }
    lines.join("\n").into_bytes()
}

/// `scan-compose`: the scanner's PDF, its bytes and page count.
pub fn compose(request: &Value, buffers: &mut Buffers) -> Result<Value, AppError> {
    let images = match request.get("images") {
        value if request::nullish(value) => &[][..],
        Some(Value::Array(images)) if images.len() <= MAX_SCAN_PAGES => images.as_slice(),
        _ => {
            return Err(invalid(format!(
                "images must be an array of at most {MAX_SCAN_PAGES} JPEG files"
            )));
        }
    };
    let mut total = 0usize;
    for (index, image) in images.iter().enumerate() {
        let size = buffers
            .input(Some(image))
            .map(<[u8]>::len)
            .ok_or_else(|| invalid(format!("image {index} must be an ArrayBuffer")))?;
        if size > MAX_IMAGE_BYTES {
            return Err(too_large(
                "scan-compose",
                "image-size",
                format!("image {index} is over the {MAX_IMAGE_BYTES} byte image ceiling"),
            ));
        }
        total = total.saturating_add(size);
    }
    if total > MAX_SCAN_IMAGE_BYTES {
        return Err(too_large(
            "scan-compose",
            "images",
            format!("the images total more than {MAX_SCAN_IMAGE_BYTES} bytes"),
        ));
    }
    let pages = read_pages(request, images.len())?;
    let producer = match request.get("producer") {
        value if request::nullish(value) => None,
        Some(Value::String(text)) if text.encode_utf16().count() <= 255 => Some(text.as_str()),
        _ => {
            return Err(invalid(
                "producer must be a string of at most 255 characters",
            ));
        }
    };

    // The CJK font, read only when a character needs it.
    let needed = (pages.iter().flat_map(|page| &page.runs))
        .any(|run| run.text.iter().any(|&c| route(c, run.bold) == Route::Cjk));
    let data = cjk_bytes("scan-compose", needed)?;
    let cjk = read_cjk("scan-compose", data.as_ref().map(|data| (**data).as_ref()))?;

    let mut out = Builder::new("1.7")?;
    // Each image once, in order; the same bytes twice are stored once.
    let mut stored: Vec<ObjectId> = Vec::with_capacity(images.len());
    let mut used = 0usize;
    for (index, image) in images.iter().enumerate() {
        let bytes = buffers.take(Some(image)).unwrap_or_default();
        if !bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            return Err(invalid(format!("image {index} is not a JPEG")));
        }
        let placed = images::read(&bytes, MAX_SAVE_BYTES.saturating_sub(used)).map_err(
            |error| match error {
                images::ImageError::TooLarge => too_large(
                    "scan-compose",
                    "output",
                    format!("the PDF would exceed the {MAX_SAVE_BYTES} byte save ceiling"),
                ),
                images::ImageError::Unreadable(error) => invalid(format!(
                    "image {index} is not a JPEG the app can read ({error})"
                )),
            },
        )?;
        drop(bytes);
        // A frame that says 0 (its height then comes in a DNL marker),
        // more than 16,384 pixels a side, or arithmetic coding, which the
        // scanner's canvas never writes and several viewers do not decode.
        let (width, height) = (placed.width, placed.height);
        if width == 0 || height == 0 || width > MAX_JPEG_SIDE || height > MAX_JPEG_SIDE {
            return Err(invalid(format!(
                "image {index} is {width} x {height} pixels; a scan image has 1 to {MAX_JPEG_SIDE} a side"
            )));
        }
        if placed.arithmetic {
            return Err(invalid(format!(
                "image {index} is an arithmetic-coded JPEG"
            )));
        }
        used = used.saturating_add(placed.stored());
        let id = images::store_shared(&mut out, placed, &stored)?;
        stored.push(id);
    }

    let mut fonts = Fonts {
        cjk,
        cjk_cids: HashMap::new(),
        cjk_chars: Vec::new(),
        nimbus: Default::default(),
        helvetica: Default::default(),
        latin: Default::default(),
        objects: BTreeMap::new(),
    };
    let mut kids = Vec::with_capacity(pages.len());
    for page in &pages {
        let (width, height) = (page.width, page.height);
        let mut content: Vec<String> = Vec::new();
        if page.image.is_some() {
            content.push(format!(
                "q {} 0 0 {} 0 0 cm /Im0 Do Q",
                num(width),
                num(height)
            ));
        }
        for frame in &page.boxes {
            let [r, g, b] = frame.color.map(num);
            content.push(format!(
                "q {} w 0 J 0 j 10 M {r} {g} {b} RG {} {} {} {} re S Q",
                num(frame.line),
                num(frame.x),
                num(height - frame.y - frame.height),
                num(frame.width),
                num(frame.height)
            ));
        }
        let mut used: BTreeSet<Key> = BTreeSet::new();
        for run in &page.runs {
            let mut parts: Vec<(Key, String)> = Vec::new();
            let mut total = 0.0;
            // Right-to-left text is drawn in visual order, which readers turn
            // back into the typed order. Brackets are mirrored only where they
            // are seen: PDFKit's copy does not mirror them back.
            let visual = bidi::visual(&run.text, !run.invisible);
            for &c in visual.as_deref().unwrap_or(&run.text) {
                let (key, code, width) = fonts.encode(c, run.bold)?;
                total += width;
                let hex = if key == Key::Cjk {
                    format!("{code:04x}")
                } else {
                    format!("{code:02x}")
                };
                match parts.last_mut() {
                    Some((last, codes)) if *last == key => codes.push_str(&hex),
                    _ => parts.push((key, hex)),
                }
            }
            if parts.is_empty() {
                continue;
            }
            let end = total / 1000.0 * run.size;
            let scale = match run.fit {
                Some(fit) if end > 0.0 => fit / end,
                _ => 1.0,
            };
            // The rendering mode is graphics state and outlives ET, so each
            // text object sets its own.
            let mode = if run.invisible {
                "3 Tr".to_string()
            } else {
                let [r, g, b] = run.color.map(num);
                format!("0 Tr {r} {g} {b} rg")
            };
            let mut ops = format!(
                "BT {mode} {} 0 0 {} {} {} Tm",
                num(run.size * scale),
                num(run.size),
                num(run.x),
                num(height - run.y)
            );
            for (key, codes) in parts {
                used.insert(key);
                ops.push_str(&format!(" /{} 1 Tf <{codes}> Tj", key.resource()));
            }
            ops.push_str(" ET");
            content.push(ops);
        }
        let contents = stream(&mut out, Dictionary::new(), content.join("\n").as_bytes())?;
        let mut resources = Dictionary::new();
        if let Some(image) = page.image.and_then(|index| stored.get(index)) {
            let xobjects = Object::dictionary([("Im0", Object::Reference(*image))]);
            resources.insert(b"XObject".to_vec(), xobjects);
        }
        if !used.is_empty() {
            let mut entries = Dictionary::new();
            for key in used {
                let id = fonts.object(&mut out, key)?;
                entries.insert(key.resource().into_bytes(), Object::Reference(id));
            }
            resources.insert(b"Font".to_vec(), Object::Dictionary(entries));
        }
        let media = [0.0, 0.0, width, height].map(pdf_number);
        let page = Object::dictionary([
            ("Type", Object::name("Page")),
            ("MediaBox", Object::array(media)),
            ("Resources", Object::Dictionary(resources)),
            ("Contents", Object::Reference(contents)),
        ]);
        kids.push(Object::Reference(out.allocate(page)?));
    }
    write_fonts(&mut out, &fonts)?;

    let count = kids.len();
    let tree = PageTree::read(out.document())
        .map_err(|error| AppError::plain(format!("scan-compose: {error}")))?;
    tree.insert(&mut out, 0, kids)?;
    let info = match producer {
        Some(producer) => {
            Some(out.allocate(Object::dictionary([("Producer", Object::text(producer))]))?)
        }
        None => None,
    };
    let mut doc = out.into_document();
    if let Some(info) = info {
        doc.set_trailer_entry(b"Info", Some(Object::Reference(info)))?;
    }
    let bytes = written(&doc, "scan-compose", "PDF").map_err(|mut error| {
        if error.reason.as_deref() == Some("too-large") {
            error.detail = Some("output".into());
        }
        error
    })?;
    let fields = json!({ "bytes": buffers.put(bytes), "pageCount": count });
    Ok(reply(request, fields))
}

/// Writes every font `fonts` allocated, now that their characters are known.
fn write_fonts(out: &mut Builder, fonts: &Fonts<'_>) -> Result<(), AppError> {
    for (&key, &id) in &fonts.objects {
        let font = match key {
            Key::Helvetica(bold) => {
                let metrics = helvetica(bold);
                let widths = (32..=255u8).map(|code| pdf_number(metrics.advance(code).round()));
                // The encoding alone reads U+00A0 as a space: ToUnicode keeps it.
                let entries: Vec<(u32, char)> = (fonts.helvetica[usize::from(bold)].iter())
                    .map(|(&code, &c)| (u32::from(code), c))
                    .collect();
                let unicode = stream(out, Dictionary::new(), &to_unicode(&entries, false))?;
                Object::dictionary([
                    ("ToUnicode", Object::Reference(unicode)),
                    ("Type", Object::name("Font")),
                    ("Subtype", Object::name("Type1")),
                    ("BaseFont", Object::name(family(bold).base_font())),
                    ("Encoding", Object::name("WinAnsiEncoding")),
                    ("FirstChar", Object::Integer(32)),
                    ("LastChar", Object::Integer(255)),
                    ("Widths", Object::array(widths)),
                ])
            }
            Key::Latin(bold) => {
                let family = family(bold);
                let mut font = Eight::Latin.font(family.base_font(), family);
                let entries: Vec<(u32, char)> = (fonts.latin[usize::from(bold)].iter())
                    .map(|(&code, &c)| (u32::from(code), c))
                    .collect();
                let unicode = stream(out, Dictionary::new(), &to_unicode(&entries, false))?;
                font.insert(b"ToUnicode".to_vec(), Object::Reference(unicode));
                Object::Dictionary(font)
            }
            Key::Nimbus(bold, n) => {
                nimbus_font(out, bold, &fonts.nimbus[usize::from(bold)].fonts[n])?
            }
            Key::Cjk => cjk_font(out, fonts)?,
        };
        out.put_value(id, font)?;
    }
    Ok(())
}

/// A Type1 simple font embedding the Nimbus Sans outlines of `glyphs`
/// (code `i + 1` draws glyph `i`) as a bare CFF program (FontFile3/Type1C).
fn nimbus_font(
    out: &mut Builder,
    bold: bool,
    glyphs: &[(usize, char)],
) -> Result<Object, AppError> {
    let outlines = scan_fonts::outlines(bold);
    let style = if bold { "Bold" } else { "Regular" };
    let name = format!(
        "{}+NimbusSans-{style}",
        subset_tag(glyphs.iter().map(|&(_, c)| c))
    );
    let ids: Vec<usize> = glyphs.iter().map(|&(glyph, _)| glyph).collect();
    let (program, bbox) = outlines
        .program(name.as_bytes(), &ids)
        .ok_or_else(|| AppError::plain("scan-compose: the Latin font subset could not be built"))?;
    let file = stream(
        out,
        Dictionary::from_iter([(b"Subtype".to_vec(), Object::name("Type1C"))]),
        &program,
    )?;
    // Nimbus Sans's AFM values (Helvetica's): ascender, descender, cap
    // height and dominant vertical stem.
    let descriptor = Object::dictionary([
        ("Type", Object::name("FontDescriptor")),
        ("FontName", Object::name(name.as_bytes())),
        ("Flags", Object::Integer(32)),
        (
            "FontBBox",
            Object::array(bbox.map(|v| Object::Integer(i64::from(v)))),
        ),
        ("ItalicAngle", Object::Integer(0)),
        ("Ascent", Object::Integer(718)),
        ("Descent", Object::Integer(-207)),
        ("CapHeight", Object::Integer(718)),
        ("StemV", Object::Integer(if bold { 140 } else { 88 })),
        ("FontFile3", Object::Reference(file)),
    ]);
    let descriptor = out.allocate(descriptor)?;
    let mut differences = vec![Object::Integer(1)];
    differences.extend(ids.iter().map(|&glyph| Object::name(outlines.name(glyph))));
    let widths = ids
        .iter()
        .map(|&glyph| pdf_number(outlines.advance(glyph).round()));
    let entries: Vec<(u32, char)> = (glyphs.iter().enumerate())
        .map(|(i, &(_, c))| (i as u32 + 1, c))
        .collect();
    let unicode = stream(out, Dictionary::new(), &to_unicode(&entries, false))?;
    Ok(Object::dictionary([
        ("Type", Object::name("Font")),
        ("Subtype", Object::name("Type1")),
        ("BaseFont", Object::name(name.as_bytes())),
        ("FirstChar", Object::Integer(1)),
        ("LastChar", Object::Integer(glyphs.len() as i64)),
        ("Widths", Object::array(widths)),
        (
            "Encoding",
            Object::dictionary([
                ("Type", Object::name("Encoding")),
                ("Differences", Object::Array(differences)),
            ]),
        ),
        ("FontDescriptor", Object::Reference(descriptor)),
        ("ToUnicode", Object::Reference(unicode)),
    ]))
}

/// The Type0 font of the CJK font's subset: one CID per distinct
/// character, CID `i` drawn by the subset's glyph `i` (Identity), each
/// mapped to its character.
fn cjk_font(out: &mut Builder, fonts: &Fonts<'_>) -> Result<Object, AppError> {
    let font = fonts
        .cjk
        .as_ref()
        .ok_or_else(|| AppError::plain("scan-compose: the CJK font is not loaded"))?;
    let chars = &fonts.cjk_chars;
    let glyphs: Vec<u16> = std::iter::once(0)
        .chain(chars.iter().map(|&c| font.glyph(c)))
        .collect();
    let program = font.subset(&glyphs).map_err(|error| match error {
        SubsetError::TooManyGlyphs => too_large(
            "scan-compose",
            "characters",
            "the CJK font subset would need more than 65,535 glyphs",
        ),
        SubsetError::TooLarge => too_large(
            "scan-compose",
            "characters",
            "the CJK font subset would be larger than the font",
        ),
        SubsetError::Unreadable => AppError::reason(
            "no-font",
            "scan-compose: the installed CJK font cannot be subset",
        ),
    })?;
    let name = format!("{}+DroidSansFallback", subset_tag(chars.iter().copied()));
    let length = Object::Integer(program.len() as i64);
    let file = stream(
        out,
        Dictionary::from_iter([(b"Length1".to_vec(), length)]),
        &program,
    )?;
    let metrics = font.metrics();
    let descriptor = Object::dictionary([
        ("Type", Object::name("FontDescriptor")),
        ("FontName", Object::name(name.as_bytes())),
        ("Flags", Object::Integer(4)),
        ("FontBBox", Object::array(metrics.bbox.map(Object::Integer))),
        ("ItalicAngle", Object::Integer(0)),
        ("Ascent", Object::Integer(metrics.ascent)),
        ("Descent", Object::Integer(metrics.descent)),
        ("CapHeight", Object::Integer(metrics.cap_height)),
        ("StemV", Object::Integer(80)),
        ("FontFile2", Object::Reference(file)),
    ]);
    let descriptor = out.allocate(descriptor)?;
    let widths = glyphs[1..]
        .iter()
        .map(|&glyph| Object::Integer((font.advance(glyph) + 0.5).floor() as i64));
    let descendant = Object::dictionary([
        ("Type", Object::name("Font")),
        ("Subtype", Object::name("CIDFontType2")),
        ("BaseFont", Object::name(name.as_bytes())),
        (
            "CIDSystemInfo",
            Object::dictionary([
                ("Registry", Object::string("Adobe")),
                ("Ordering", Object::string("Identity")),
                ("Supplement", Object::Integer(0)),
            ]),
        ),
        ("FontDescriptor", Object::Reference(descriptor)),
        (
            "W",
            Object::array([Object::Integer(1), Object::array(widths)]),
        ),
        ("DW", Object::Integer(1000)),
        ("CIDToGIDMap", Object::name("Identity")),
    ]);
    let descendant = out.allocate(descendant)?;
    let entries: Vec<(u32, char)> = (chars.iter().enumerate())
        .map(|(i, &c)| (i as u32 + 1, c))
        .collect();
    let unicode = stream(out, Dictionary::new(), &to_unicode(&entries, true))?;
    Ok(Object::dictionary([
        ("Type", Object::name("Font")),
        ("Subtype", Object::name("Type0")),
        ("BaseFont", Object::name(name.as_bytes())),
        ("Encoding", Object::name("Identity-H")),
        (
            "DescendantFonts",
            Object::array([Object::Reference(descendant)]),
        ),
        ("ToUnicode", Object::Reference(unicode)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_tags_and_routes() {
        assert_eq!(num(612.0), "612");
        assert_eq!(num(0.12), "0.12");
        assert_eq!(num(-0.00001), "0");
        assert_eq!(num(1.23456), "1.2346");
        // The TypeScript scanner's tags (FNV-1a over the characters' bytes).
        assert_eq!(subset_tag("中文".chars()).len(), 6);
        assert_ne!(subset_tag("中文".chars()), subset_tag("日本".chars()));
        assert_eq!(route('A', false), Route::Helvetica(b'A'));
        assert_eq!(route('é', false), Route::Helvetica(0xE9));
        assert!(matches!(route('ż', false), Route::Latin(_)));
        assert!(matches!(route('Ω', true), Route::Nimbus(_)));
        assert_eq!(route('中', false), Route::Cjk);
        assert_eq!(clean("a\tb\u{7}c\u{85}").collect::<String>(), "a bc");
    }
}

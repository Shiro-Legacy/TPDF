// SPDX-License-Identifier: AGPL-3.0-or-later
//! Appearance streams for the annotations the app writes and the view
//! draws without one (spec 06): Form XObjects (ISO 32000-2 §8.10, §12.5.5)
//! and the /Rect each needs.
//!
//! Every computed coordinate is clamped into a working range and rounded
//! to 1/10,000 pt before it is written (section 3.0.1), so content never
//! holds `NaN`, `inf` or runaway digits. All artwork here (note icons,
//! caret, line endings, squiggle, stamp plate) is this module's own
//! design. FreeText shares the variable-text layout of the form widgets
//! (`forms::scripts`, spec 07).
use crate::app::forms::scripts::{self, Cjk, Fonts, Glyph, Slot};
use crate::app::forms::text;
use crate::app::ops::annots::NOTE_SIZE;
use crate::object::{Dictionary, Object, ObjectId};
use crate::serialize::compact_real;

use super::fonts::{self, Base14};
use super::{Color, Point, Quad};

pub const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// Content coordinates are clamped into ±COORD (points): far beyond any
/// page, small enough that rounding to 1/10,000 stays exact.
const COORD: f64 = 1e7;
/// The widest stroke drawn (points).
pub(crate) const MAX_WIDTH: f64 = 1000.0;
/// A Squiggly's vertices over all its quads (section 3.7).
const MAX_SQUIGGLY_VERTICES: usize = 32_768;
/// Points of an Ink drawn at most (section 6).
const MAX_INK_POINTS: usize = 131_072;
/// FreeText lines laid out at most (section 6).
const MAX_LINES: usize = 65_536;
/// Baseline distance of FreeText lines, in font sizes. It exceeds the
/// glyph box of every standard face (at most 1.19 em in the Core 14 AFMs),
/// so lines never overlap.
const LINE_FACTOR: f64 = 1.2;
/// The icon stamp plate: 3:1.
const STAMP_W: f64 = 150.0;
const STAMP_H: f64 = 50.0;
/// The caret's design box: wider than tall.
const CARET_BOX: [f64; 4] = [0.0, 0.0, 30.0, 20.0];
/// The grid the tile icons are designed on (scaled to `NOTE_SIZE`).
const TILE: f64 = 20.0;

const BLACK: Color = Color::Gray(0.0);
/// Highlighter yellow: black text stays black under it (Multiply).
const HIGHLIGHT: Color = Color::Rgb([1.0, 0.9, 0.2]);
/// A note tile without /C: pale cream.
const TILE_BACKGROUND: Color = Color::Rgb([0.98, 0.95, 0.78]);
const TILE_OUTLINE: Color = Color::Gray(0.28);
/// Stamps without /C: a deep vermilion.
const STAMP_INK: Color = Color::Rgb([0.8, 0.13, 0.1]);
/// Redact outlines without /C.
const REDACT_INK: Color = Color::Rgb([0.9, 0.1, 0.1]);

/// A Form XObject appearance.
#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    pub bbox: [f64; 4],
    pub matrix: [f64; 6],
    /// Empty when the content needs none.
    pub resources: Dictionary,
    pub content: Vec<u8>,
}

impl Form {
    /// The stream dictionary: /Type /XObject, /Subtype /Form, /BBox,
    /// /Matrix, and /Resources when not empty. No /Length or /Filter
    /// (the writer adds those).
    pub fn dictionary(&self) -> Dictionary {
        let mut dict = Dictionary::new();
        dict.insert(b"Type".to_vec(), Object::name("XObject"));
        dict.insert(b"Subtype".to_vec(), Object::name("Form"));
        dict.insert(b"BBox".to_vec(), numbers(&self.bbox));
        dict.insert(b"Matrix".to_vec(), numbers(&self.matrix));
        if !self.resources.is_empty() {
            dict.insert(
                b"Resources".to_vec(),
                Object::Dictionary(self.resources.clone()),
            );
        }
        dict
    }
}

/// How a shape is painted: /C (stroke), /IC (fill), border width, /CA,
/// dash array (empty = solid).
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub stroke: Option<Color>,
    pub fill: Option<Color>,
    pub width: f64,
    pub opacity: f64,
    pub dash: Vec<f64>,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            stroke: None,
            fill: None,
            width: 1.0,
            opacity: 1.0,
            dash: Vec::new(),
        }
    }
}

// ------------------------------------------------------------------ numbers

/// A PDF number: Integer when `value` is integral and |value| < 1e15,
/// else Real; non-finite values become Integer 0.
pub fn number(value: f64) -> Object {
    if !value.is_finite() {
        return Object::Integer(0);
    }
    if value.fract() == 0.0 && value.abs() < 1e15 {
        Object::Integer(value as i64)
    } else {
        Object::Real(value)
    }
}

/// An array of `number`s.
pub fn numbers(values: &[f64]) -> Object {
    Object::Array(values.iter().map(|&v| number(v)).collect())
}

/// Axis-aligned bounds [x0 y0 x1 y1] of the points; None for none.
/// Points with a non-finite coordinate are left out.
pub fn bounds(points: impl IntoIterator<Item = Point>) -> Option<[f64; 4]> {
    let mut out: Option<[f64; 4]> = None;
    for (x, y) in points {
        if !(x.is_finite() && y.is_finite()) {
            continue;
        }
        let b = out.get_or_insert([x, y, x, y]);
        *b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
    }
    out
}

/// `rect` grown by `by` on every side (shrunk for negative `by`); a side
/// shrunk past the other collapses onto the middle.
pub fn grow(rect: [f64; 4], by: f64) -> [f64; 4] {
    let by = if by.is_finite() { by } else { 0.0 };
    let mut r = [rect[0] - by, rect[1] - by, rect[2] + by, rect[3] + by];
    for axis in 0..2 {
        if r[axis] > r[axis + 2] {
            let mid = rect[axis] / 2.0 + rect[axis + 2] / 2.0;
            r[axis] = mid;
            r[axis + 2] = mid;
        }
    }
    r
}

/// A coordinate as drawn: finite and clamped.
fn fin(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(-COORD, COORD)
    } else {
        0.0
    }
}

fn pt(p: Point) -> Point {
    (fin(p.0), fin(p.1))
}

/// A rect with finite, clamped, ordered coordinates.
fn sane_rect(r: [f64; 4]) -> [f64; 4] {
    let [a, b, c, d] = r.map(fin);
    [a.min(c), b.min(d), a.max(c), b.max(d)]
}

/// A stroke width as drawn: None (no stroke) for zero, negative or
/// non-finite widths (decision A4).
fn stroke_width(width: f64) -> Option<f64> {
    (width.is_finite() && width > 0.0).then(|| width.min(MAX_WIDTH))
}

/// A number as content writes it: finite, rounded to 1/10,000.
fn num(v: f64) -> String {
    let v = if v.is_finite() {
        v.clamp(-1e12, 1e12)
    } else {
        0.0
    };
    let r = (v * 1e4).round() / 1e4;
    if r == 0.0 {
        "0".into()
    } else {
        compact_real(r)
    }
}

/// A name token: delimiters, whitespace and other bytes outside the
/// printable range as `#xx`.
fn pdf_name(name: &[u8]) -> String {
    let mut out = String::from("/");
    for &b in name {
        if (0x21..=0x7E).contains(&b) && !b"()<>[]{}/%#".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("#{b:02X}"));
        }
    }
    out
}

fn add(a: Point, b: Point) -> Point {
    (a.0 + b.0, a.1 + b.1)
}

fn sub(a: Point, b: Point) -> Point {
    (a.0 - b.0, a.1 - b.1)
}

fn scale(a: Point, k: f64) -> Point {
    (a.0 * k, a.1 * k)
}

fn length(a: Point) -> f64 {
    a.0.hypot(a.1)
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

// ------------------------------------------------------------------ content

/// A content stream being written, with the resources it names.
struct Content {
    out: String,
    states: Dictionary,
    fonts: Dictionary,
}

impl Content {
    fn new() -> Self {
        Self {
            out: String::new(),
            states: Dictionary::new(),
            fonts: Dictionary::new(),
        }
    }

    fn op(&mut self, text: &str) {
        self.out.push_str(text);
        self.out.push('\n');
    }

    /// `values` then `op`.
    fn with(&mut self, values: &[f64], op: &str) {
        for &v in values {
            self.out.push_str(&num(v));
            self.out.push(' ');
        }
        self.op(op);
    }

    fn m(&mut self, p: Point) {
        self.with(&[p.0, p.1], "m");
    }

    fn l(&mut self, p: Point) {
        self.with(&[p.0, p.1], "l");
    }

    fn c(&mut self, a: Point, b: Point, p: Point) {
        self.with(&[a.0, a.1, b.0, b.1, p.0, p.1], "c");
    }

    fn re(&mut self, r: [f64; 4]) {
        self.with(&[r[0], r[1], r[2] - r[0], r[3] - r[1]], "re");
    }

    /// A closed polygon through `points`.
    fn polygon(&mut self, points: &[Point]) {
        for (i, &p) in points.iter().enumerate() {
            if i == 0 { self.m(p) } else { self.l(p) }
        }
        self.op("h");
    }

    /// An open path through `points`.
    fn polyline(&mut self, points: &[Point]) {
        for (i, &p) in points.iter().enumerate() {
            if i == 0 { self.m(p) } else { self.l(p) }
        }
    }

    /// The ellipse inscribed in `r`: four Béziers, under 0.03% off.
    fn ellipse(&mut self, r: [f64; 4]) {
        const K: f64 = 0.552_284_749_8;
        let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
        let (rx, ry) = ((r[2] - r[0]) / 2.0, (r[3] - r[1]) / 2.0);
        self.m((cx + rx, cy));
        self.c(
            (cx + rx, cy + K * ry),
            (cx + K * rx, cy + ry),
            (cx, cy + ry),
        );
        self.c(
            (cx - K * rx, cy + ry),
            (cx - rx, cy + K * ry),
            (cx - rx, cy),
        );
        self.c(
            (cx - rx, cy - K * ry),
            (cx - K * rx, cy - ry),
            (cx, cy - ry),
        );
        self.c(
            (cx + K * rx, cy - ry),
            (cx + rx, cy - K * ry),
            (cx + rx, cy),
        );
        self.op("h");
    }

    fn circle(&mut self, centre: Point, radius: f64) {
        let (x, y) = centre;
        self.ellipse([x - radius, y - radius, x + radius, y + radius]);
    }

    /// A rectangle with corners rounded by `radius`.
    fn rounded(&mut self, r: [f64; 4], radius: f64) {
        const K: f64 = 0.552_284_749_8;
        let [x0, y0, x1, y1] = r;
        let k = radius * (1.0 - K);
        self.m((x0 + radius, y0));
        self.l((x1 - radius, y0));
        self.c((x1 - k, y0), (x1, y0 + k), (x1, y0 + radius));
        self.l((x1, y1 - radius));
        self.c((x1, y1 - k), (x1 - k, y1), (x1 - radius, y1));
        self.l((x0 + radius, y1));
        self.c((x0 + k, y1), (x0, y1 - k), (x0, y1 - radius));
        self.l((x0, y0 + radius));
        self.c((x0, y0 + k), (x0 + k, y0), (x0 + radius, y0));
        self.op("h");
    }

    fn color(&mut self, color: &Color, stroke: bool) {
        match color {
            Color::Gray(g) => self.with(&[*g], if stroke { "G" } else { "g" }),
            Color::Rgb(v) => self.with(v, if stroke { "RG" } else { "rg" }),
            Color::Cmyk(v) => self.with(v, if stroke { "K" } else { "k" }),
        }
    }

    fn width(&mut self, width: f64) {
        self.with(&[width], "w");
    }

    fn dash(&mut self, dash: &[f64]) {
        if dash.is_empty() {
            return;
        }
        let items: Vec<String> = dash.iter().map(|&d| num(d.max(0.0))).collect();
        self.op(&format!("[{}] 0 d", items.join(" ")));
    }

    /// Opacity `alpha` below 1 (section 3.0.3) and, for a highlight, the
    /// Multiply blend mode: an ExtGState selected here, before any paint.
    fn alpha(&mut self, alpha: f64, multiply: bool) {
        let alpha = (alpha.is_finite() && alpha < 1.0).then(|| alpha.max(0.0));
        if alpha.is_none() && !multiply {
            return;
        }
        let mut state = Dictionary::new();
        state.insert(b"Type".to_vec(), Object::name("ExtGState"));
        if let Some(a) = alpha {
            state.insert(b"CA".to_vec(), number(a));
            state.insert(b"ca".to_vec(), number(a));
        }
        if multiply {
            state.insert(b"BM".to_vec(), Object::name("Multiply"));
        }
        let name = b"GS0".to_vec();
        self.states.insert(name.clone(), Object::Dictionary(state));
        self.op(&format!("{} gs", pdf_name(&name)));
    }

    fn finish(self, bbox: [f64; 4], matrix: [f64; 6]) -> Form {
        let mut resources = Dictionary::new();
        if !self.states.is_empty() {
            resources.insert(b"ExtGState".to_vec(), Object::Dictionary(self.states));
        }
        if !self.fonts.is_empty() {
            resources.insert(b"Font".to_vec(), Object::Dictionary(self.fonts));
        }
        Form {
            bbox,
            matrix,
            resources,
            content: self.out.into_bytes(),
        }
    }
}

/// `form` drawn onto `inner`, in a form whose /BBox is `outer` (the
/// view's way to draw a fixed design into part of a /Rect). With
/// `keep_aspect`, the largest centred box of the design's aspect is used.
pub(crate) fn fitted(form: Form, inner: [f64; 4], outer: [f64; 4], keep_aspect: bool) -> Form {
    let shown = super::transform_rect(&form.matrix, form.bbox);
    let (bw, bh) = (shown[2] - shown[0], shown[3] - shown[1]);
    let inner = sane_rect(inner);
    let (iw, ih) = (inner[2] - inner[0], inner[3] - inner[1]);
    let mut content = Content::new();
    if bw > 0.0 && bh > 0.0 && bw.is_finite() && bh.is_finite() {
        let (mut sx, mut sy) = (iw / bw, ih / bh);
        if keep_aspect {
            let s = sx.min(sy);
            (sx, sy) = (s, s);
        }
        let tx = inner[0] + (iw - bw * sx) / 2.0 - shown[0] * sx;
        let ty = inner[1] + (ih - bh * sy) / 2.0 - shown[1] * sy;
        let place = super::concat(&form.matrix, &[sx, 0.0, 0.0, sy, tx, ty]);
        content.op("q");
        content.with(&place, "cm");
        content
            .out
            .push_str(&String::from_utf8_lossy(&form.content));
        content.op("Q");
    }
    let mut out = content.finish(sane_rect(outer), IDENTITY);
    out.resources = form.resources;
    out
}

// ------------------------------------------------------------------ tile icons

/// The tile icons: Text annotation names (ISO 32000-2 §12.5.6.4), and the
/// FileAttachment and Sound icons the view draws (owner decision A5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Icon {
    Comment,
    Key,
    Note,
    Help,
    NewParagraph,
    Paragraph,
    Insert,
    Paperclip,
    Speaker,
}

impl Icon {
    fn of_note(name: &[u8]) -> Self {
        match name {
            b"Comment" => Self::Comment,
            b"Key" => Self::Key,
            b"Help" => Self::Help,
            b"NewParagraph" => Self::NewParagraph,
            b"Paragraph" => Self::Paragraph,
            b"Insert" => Self::Insert,
            _ => Self::Note,
        }
    }

    /// The glyph, on the 20-unit grid, in the current colours.
    fn draw(self, c: &mut Content) {
        match self {
            Self::Note => {
                // A sheet with its top right corner folded, three lines.
                c.width(1.2);
                c.polygon(&[
                    (5.5, 3.5),
                    (5.5, 16.5),
                    (11.8, 16.5),
                    (14.5, 13.8),
                    (14.5, 3.5),
                ]);
                c.op("S");
                c.polyline(&[(11.8, 16.5), (11.8, 13.8), (14.5, 13.8)]);
                c.op("S");
                for (y, x1) in [(11.0, 12.4), (8.4, 12.4), (5.8, 10.6)] {
                    c.polyline(&[(7.6, y), (x1, y)]);
                    c.op("S");
                }
            }
            Self::Comment => {
                // A speech bubble with a tail at its lower left, three dots.
                c.width(1.4);
                c.rounded([3.4, 7.2, 16.6, 16.4], 2.6);
                c.op("S");
                c.polyline(&[(6.4, 7.2), (5.0, 3.6), (9.8, 7.2)]);
                c.op("S");
                for x in [7.0, 10.0, 13.0] {
                    c.circle((x, 11.8), 0.95);
                    c.op("f");
                }
            }
            Self::Key => {
                // A ring bow, a diagonal shaft, two teeth.
                c.width(1.6);
                c.circle((6.6, 13.4), 3.0);
                c.op("S");
                c.polyline(&[(8.75, 11.25), (15.6, 4.4)]);
                c.op("S");
                c.polyline(&[(12.4, 7.6), (13.9, 9.1)]);
                c.op("S");
                c.polyline(&[(14.2, 5.8), (15.7, 7.3)]);
                c.op("S");
            }
            Self::Help => {
                // A question mark: a hook and a dot.
                c.width(1.8);
                c.m((6.7, 12.9));
                c.c((6.7, 16.9), (13.3, 16.9), (13.3, 12.9));
                c.c((13.3, 10.7), (10.0, 10.6), (10.0, 8.4));
                c.l((10.0, 7.4));
                c.op("S");
                c.circle((10.0, 4.6), 1.15);
                c.op("f");
            }
            Self::NewParagraph => {
                // A return arrow: down the right, then left to a head.
                c.width(1.6);
                c.polyline(&[(14.8, 15.6), (14.8, 9.2), (8.4, 9.2)]);
                c.op("S");
                c.polygon(&[(4.6, 9.2), (8.6, 12.1), (8.6, 6.3)]);
                c.op("f");
            }
            Self::Paragraph => {
                // A pilcrow: a solid bowl on two stems.
                c.width(1.4);
                c.m((10.2, 16.0));
                c.c((5.6, 16.0), (5.6, 9.6), (10.2, 9.6));
                c.op("h");
                c.op("f");
                c.polyline(&[(10.2, 16.0), (10.2, 3.8)]);
                c.op("S");
                c.polyline(&[(13.4, 16.0), (13.4, 3.8)]);
                c.op("S");
                c.polyline(&[(10.2, 16.0), (14.8, 16.0)]);
                c.op("S");
            }
            Self::Insert => {
                // A line of text with a gap, a caret pointing up into it.
                c.width(1.4);
                c.polyline(&[(3.4, 15.2), (7.6, 15.2)]);
                c.op("S");
                c.polyline(&[(12.4, 15.2), (16.6, 15.2)]);
                c.op("S");
                c.width(1.8);
                c.polyline(&[(5.0, 4.4), (10.0, 12.6), (15.0, 4.4)]);
                c.op("S");
            }
            Self::Paperclip => {
                c.width(1.3);
                c.m((9.8, 7.0));
                c.l((9.8, 13.6));
                c.c((9.8, 15.0), (11.8, 15.0), (11.8, 13.6));
                c.l((11.8, 5.2));
                c.c((11.8, 2.6), (7.0, 2.6), (7.0, 5.2));
                c.l((7.0, 14.2));
                c.c((7.0, 18.0), (14.6, 18.0), (14.6, 14.2));
                c.l((14.6, 7.4));
                c.op("S");
            }
            Self::Speaker => {
                // A speaker cone and two sound arcs.
                c.polygon(&[
                    (4.2, 8.2),
                    (7.2, 8.2),
                    (11.0, 4.8),
                    (11.0, 15.2),
                    (7.2, 11.8),
                    (4.2, 11.8),
                ]);
                c.op("f");
                c.width(1.3);
                for radius in [3.2, 6.0] {
                    arc(c, (11.0, 10.0), radius);
                    c.op("S");
                }
            }
        }
    }
}

/// A quarter circle about `centre`, from −45° to +45°.
fn arc(c: &mut Content, centre: Point, radius: f64) {
    const K: f64 = 0.552_284_749_8;
    let s = std::f64::consts::FRAC_1_SQRT_2 * radius;
    let k = K * radius * std::f64::consts::FRAC_1_SQRT_2;
    let p0 = add(centre, (s, -s));
    let p3 = add(centre, (s, s));
    c.m(p0);
    c.c(add(p0, (k, k)), add(p3, (k, -k)), p3);
}

/// A tile icon: `icon` on a rounded tile filled with `color` (a pale cream
/// without one), edged in dark grey, the glyph dark or light, whichever
/// stands out from the tile. /BBox [0 0 N N], N = `NOTE_SIZE`.
pub(crate) fn tile(icon: Icon, color: Option<&Color>, alpha: f64) -> Form {
    let background = color.cloned().unwrap_or(TILE_BACKGROUND);
    // Pure yellow (0.89) takes a dark glyph; blue (0.11) and dark red
    // (0.15) a light one.
    let glyph = if background.luminance() > 0.5 {
        Color::Gray(0.12)
    } else {
        Color::Gray(1.0)
    };
    let mut c = Content::new();
    c.alpha(alpha, false);
    let k = NOTE_SIZE / TILE;
    if (k - 1.0).abs() > 1e-12 && k.is_finite() && k > 0.0 {
        c.with(&[k, 0.0, 0.0, k, 0.0, 0.0], "cm");
    }
    c.color(&background, false);
    c.color(&TILE_OUTLINE, true);
    c.width(1.0);
    c.rounded([0.6, 0.6, TILE - 0.6, TILE - 0.6], 2.6);
    c.op("B");
    c.color(&glyph, false);
    c.color(&glyph, true);
    c.op("1 J 1 j");
    icon.draw(&mut c);
    c.finish([0.0, 0.0, NOTE_SIZE, NOTE_SIZE], IDENTITY)
}

/// A Text annotation's icon (section 3.1). `icon` is the /Name bytes; any
/// name but the seven of ISO 32000-2 §12.5.6.4 draws the Note icon.
pub fn note(icon: &[u8], color: Option<&Color>, alpha: f64) -> Form {
    tile(Icon::of_note(icon), color, alpha)
}

// ------------------------------------------------------------------ text markup

/// A quad with finite, clamped corners: (ul, ur, ll, lr).
fn sane_quad(q: &Quad) -> Quad {
    q.map(pt)
}

/// Highlight over `quads`; returns the form and the /Rect (section 3.2).
pub fn highlight(quads: &[Quad], color: Option<&Color>, alpha: f64) -> (Form, [f64; 4]) {
    let color = color.cloned().unwrap_or(HIGHLIGHT);
    let mut c = Content::new();
    c.alpha(alpha, true);
    c.color(&color, false);
    let mut rect: Option<[f64; 4]> = None;
    for q in quads {
        let [ul, ur, ll, lr] = sane_quad(q);
        c.polygon(&[ll, lr, ur, ul]);
        c.op("f");
        // A margin of a twentieth of the quad's height.
        let h = length(sub(ul, ll)).max(length(sub(ur, lr)));
        if let Some(b) = bounds([ul, ur, ll, lr]) {
            rect = union(rect, grow(b, h / 20.0));
        }
    }
    let rect = rect.unwrap_or([0.0; 4]);
    (c.finish(rect, IDENTITY), rect)
}

fn union(a: Option<[f64; 4]>, b: [f64; 4]) -> Option<[f64; 4]> {
    Some(match a {
        None => b,
        Some(a) => [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ],
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Underline,
    StrikeOut,
    Squiggly,
}

/// Stroke width of a text mark, per unit of quad height.
const MARK_WIDTH: f64 = 0.07;
/// Height of an underline, and of a strike-out, per unit of quad height.
const UNDERLINE_AT: f64 = 0.15;
const STRIKE_AT: f64 = 0.4;
/// A squiggle: its middle, amplitude, half period and stroke width per
/// unit of quad height.
const SQUIGGLE_AT: f64 = 0.09;
const SQUIGGLE_AMPLITUDE: f64 = 0.05;
const SQUIGGLE_HALF_PERIOD: f64 = 0.25;
const SQUIGGLE_WIDTH: f64 = 0.05;

/// Underline, StrikeOut or Squiggly; returns the form and the /Rect
/// (section 3.7).
pub fn text_line(
    mark: Mark,
    quads: &[Quad],
    color: Option<&Color>,
    alpha: f64,
) -> (Form, [f64; 4]) {
    let color = color.cloned().unwrap_or(BLACK);
    let quads: Vec<Quad> = quads.iter().map(sane_quad).collect();
    let mut c = Content::new();
    c.alpha(alpha, false);
    c.color(&color, true);
    let mut rect: Option<[f64; 4]> = None;
    let segments = match mark {
        Mark::Squiggly => {
            c.op("1 J 1 j");
            squiggle_segments(&quads)
        }
        _ => Vec::new(),
    };
    for (i, &[ul, ur, ll, lr]) in quads.iter().enumerate() {
        let (up_l, up_r) = (sub(ul, ll), sub(ur, lr));
        let h = length(up_l).max(length(up_r));
        if let Some(b) = bounds([ul, ur, ll, lr]) {
            rect = union(rect, b);
        }
        let at = |s: f64, t: f64| add(lerp(ll, lr, s), scale(lerp(up_l, up_r, s), t));
        let width = h * match mark {
            Mark::Squiggly => SQUIGGLE_WIDTH,
            _ => MARK_WIDTH,
        };
        let width = width.min(MAX_WIDTH);
        c.width(width);
        let mut drawn = Vec::new();
        match mark {
            Mark::Underline | Mark::StrikeOut => {
                let t = if mark == Mark::Underline {
                    UNDERLINE_AT
                } else {
                    STRIKE_AT
                };
                drawn.push(at(0.0, t));
                drawn.push(at(1.0, t));
            }
            Mark::Squiggly => {
                let n = segments.get(i).copied().unwrap_or(0);
                if n == 0 {
                    continue;
                }
                for k in 0..=n {
                    let side = if k % 2 == 0 { -1.0 } else { 1.0 };
                    drawn.push(at(
                        k as f64 / n as f64,
                        SQUIGGLE_AT + side * SQUIGGLE_AMPLITUDE,
                    ));
                }
            }
        }
        c.polyline(&drawn);
        c.op("S");
        let mut all = vec![ul, ur, ll, lr];
        all.extend(drawn);
        if let Some(b) = bounds(all) {
            rect = union(rect, grow(b, width / 2.0));
        }
    }
    let rect = rect.unwrap_or([0.0; 4]);
    (c.finish(rect, IDENTITY), rect)
}

/// Segments of each quad's squiggle: one per half period of a quarter of
/// the quad's height, at least two; when all quads together would pass
/// `MAX_SQUIGGLY_VERTICES`, the period widens uniformly. A quad past the
/// budget (more than about 10,900 quads) gets 0: no squiggle.
fn squiggle_segments(quads: &[Quad]) -> Vec<usize> {
    let natural: Vec<f64> = quads
        .iter()
        .map(|&[ul, ur, ll, lr]| {
            let h = length(sub(ul, ll)).max(length(sub(ur, lr)));
            let len = length(sub(lr, ll));
            let s = len / (SQUIGGLE_HALF_PERIOD * h);
            if s.is_finite() {
                s.clamp(0.0, 1e9)
            } else {
                2.0
            }
        })
        .collect();
    let count = quads.len() as f64;
    let budget = MAX_SQUIGGLY_VERTICES as f64;
    let total: f64 = natural.iter().map(|s| s.ceil().max(2.0) + 1.0).sum();
    let widen = if total <= budget {
        1.0
    } else if budget > 3.0 * count {
        (natural.iter().sum::<f64>() / (budget - 3.0 * count)).max(1.0)
    } else {
        f64::INFINITY
    };
    let mut left = MAX_SQUIGGLY_VERTICES;
    natural
        .iter()
        .map(|s| {
            let n = (s / widen).ceil();
            let n = if n.is_finite() {
                n.max(2.0) as usize
            } else {
                2
            };
            if n < left {
                left -= n + 1;
                n
            } else {
                0
            }
        })
        .collect()
}

// ------------------------------------------------------------------ shapes

/// The /RD each side of a Square, Circle (or Polygon) the app creates
/// from a rectangle with border width `width` (section 3.4): half the
/// border, and a point for anti-aliasing.
pub fn border_allowance(width: f64) -> f64 {
    allowance(width)
}

fn allowance(width: f64) -> f64 {
    stroke_width(width).unwrap_or(0.0) / 2.0 + 1.0
}

/// The fill and stroke a closed shape paints: /IC and /C; black stroke
/// when it has neither, so it stays visible (decision A5).
fn closed_paint(c: &mut Content, style: &Style) -> Option<&'static str> {
    let width = stroke_width(style.width);
    let stroke = match (&style.stroke, &style.fill) {
        (Some(s), _) => Some(s.clone()),
        (None, None) => Some(BLACK),
        (None, Some(_)) => None,
    }
    .filter(|_| width.is_some());
    if let Some(fill) = &style.fill {
        c.color(fill, false);
    }
    if let (Some(stroke), Some(width)) = (&stroke, width) {
        c.color(stroke, true);
        c.width(width);
        c.dash(&style.dash);
    }
    match (style.fill.is_some(), stroke.is_some()) {
        (true, true) => Some("B"),
        (true, false) => Some("f"),
        (false, true) => Some("S"),
        (false, false) => None,
    }
}

/// Square (`ellipse` false) or Circle whose border runs along `inner`,
/// in a form whose /BBox is `bbox`.
pub fn rectangle(inner: [f64; 4], bbox: [f64; 4], ellipse: bool, style: &Style) -> Form {
    let inner = sane_rect(inner);
    let mut c = Content::new();
    c.alpha(style.opacity, false);
    if let Some(paint) = closed_paint(&mut c, style) {
        if ellipse {
            c.ellipse(inner);
        } else {
            c.re(inner);
        }
        c.op(paint);
    }
    c.finish(sane_rect(bbox), IDENTITY)
}

/// Line from `a` to `b`; `endings` are the two /LE names (b"" or b"None"
/// for none). Returns the form and the /Rect (section 3.5).
pub fn line(a: Point, b: Point, endings: [&[u8]; 2], style: &Style) -> (Form, [f64; 4]) {
    let (a, b) = (pt(a), pt(b));
    let mut points = vec![a, b];
    let mut c = Content::new();
    let width = stroke_width(style.width);
    if let Some(width) = width {
        c.alpha(style.opacity, false);
        c.color(style.stroke.as_ref().unwrap_or(&BLACK), true);
        if let Some(fill) = &style.fill {
            c.color(fill, false);
        }
        c.width(width);
        c.op("1 j");
        c.dash(&style.dash);
        c.polyline(&[a, b]);
        c.op("S");
        if !style.dash.is_empty() {
            c.op("[] 0 d");
        }
        let d = sub(b, a);
        let len = length(d);
        let along = if len > 0.0 && len.is_finite() {
            scale(d, 1.0 / len)
        } else {
            (1.0, 0.0)
        };
        let size = 6.0 + 2.3 * width;
        let filled = style.fill.is_some();
        ending(
            &mut c,
            endings[0],
            a,
            scale(along, -1.0),
            size,
            filled,
            &mut points,
        );
        ending(&mut c, endings[1], b, along, size, filled, &mut points);
    }
    let reach = width.unwrap_or(0.0) / 2.0 + 0.5;
    let rect = bounds(points).map_or([0.0; 4], |r| grow(r, reach));
    (c.finish(rect, IDENTITY), rect)
}

/// A line ending (ISO 32000-2 Table 179) at `p`, `out` the unit direction
/// pointing away from the line, `size` its length; its points go to
/// `points`.
fn ending(
    c: &mut Content,
    name: &[u8],
    p: Point,
    out: Point,
    size: f64,
    filled: bool,
    points: &mut Vec<Point>,
) {
    let normal = (-out.1, out.0);
    // `along` units out of the line and `across` to its left.
    let at = |along: f64, across: f64| add(p, add(scale(out, along), scale(normal, across)));
    let r = size / 2.0;
    let closed = if filled { "b" } else { "s" };
    let (shape, paint): (Vec<Point>, &str) = match name {
        b"Square" => (vec![at(-r, -r), at(r, -r), at(r, r), at(-r, r)], closed),
        b"Diamond" => (
            vec![at(r, 0.0), at(0.0, r), at(-r, 0.0), at(0.0, -r)],
            closed,
        ),
        b"OpenArrow" => (vec![at(-size, r), p, at(-size, -r)], "S"),
        b"ClosedArrow" => (vec![at(-size, r), p, at(-size, -r)], closed),
        b"ROpenArrow" => (vec![at(size, r), p, at(size, -r)], "S"),
        b"RClosedArrow" => (vec![at(size, r), p, at(size, -r)], closed),
        b"Butt" => (vec![at(0.0, r), at(0.0, -r)], "S"),
        b"Slash" => {
            // A bar at 30° from the perpendicular.
            let (s, k) = (0.5 * r * 1.2, 0.866 * r * 1.2);
            (vec![at(s, k), at(-s, -k)], "S")
        }
        b"Circle" => {
            c.circle(p, r);
            c.op(closed);
            points.extend([(p.0 - r, p.1 - r), (p.0 + r, p.1 + r)]);
            return;
        }
        _ => return,
    };
    if paint == "S" {
        c.polyline(&shape);
    } else {
        c.polygon(&shape);
    }
    c.op(paint);
    points.extend(shape);
}

/// Polygon (`closed`) or PolyLine; returns the form and the /Rect.
pub fn poly(vertices: &[Point], closed: bool, style: &Style) -> (Form, [f64; 4]) {
    let vertices: Vec<Point> = vertices
        .iter()
        .take(MAX_INK_POINTS)
        .map(|&p| pt(p))
        .collect();
    let mut c = Content::new();
    let width = stroke_width(style.width);
    if vertices.len() >= 2 {
        c.alpha(style.opacity, false);
        c.op("1 J 1 j");
        if closed {
            if let Some(paint) = closed_paint(&mut c, style) {
                c.polygon(&vertices);
                c.op(paint);
            }
        } else if let Some(width) = width {
            c.color(style.stroke.as_ref().unwrap_or(&BLACK), true);
            c.width(width);
            c.dash(&style.dash);
            c.polyline(&vertices);
            c.op("S");
        }
    }
    let reach = width.unwrap_or(0.0) / 2.0 + 0.5;
    let rect = bounds(vertices).map_or([0.0; 4], |r| grow(r, reach));
    (c.finish(rect, IDENTITY), rect)
}

/// The /RD each side of an Ink with border width `width` (section 3.6):
/// half the pen, and a point for anti-aliasing.
pub fn ink_allowance(width: f64) -> f64 {
    allowance(width)
}

/// Ink strokes; returns the form and the /Rect (section 3.6). Straight
/// segments with round caps and joins, so nothing passes the points'
/// bounds by more than half the pen (owner decision A6).
pub fn ink(strokes: &[Vec<Point>], style: &Style) -> (Form, [f64; 4]) {
    let mut c = Content::new();
    let width = stroke_width(style.width);
    let mut all = Vec::new();
    let mut left = MAX_INK_POINTS;
    let mut started = false;
    for stroke in strokes {
        if left == 0 {
            break;
        }
        let points: Vec<Point> = stroke.iter().take(left).map(|&p| pt(p)).collect();
        left -= points.len();
        if points.is_empty() {
            continue;
        }
        if let Some(width) = width {
            if !started {
                c.alpha(style.opacity, false);
                c.color(style.stroke.as_ref().unwrap_or(&BLACK), true);
                c.width(width);
                c.op("1 J 1 j");
                started = true;
            }
            c.polyline(&points);
            if points.len() == 1 {
                // A dot of the pen's width.
                c.l(points[0]);
            }
            c.op("S");
        }
        all.extend(points);
    }
    let rect = bounds(all).map_or([0.0; 4], |r| grow(r, ink_allowance(style.width)));
    (c.finish(rect, IDENTITY), rect)
}

// ------------------------------------------------------------------ caret, stamps

/// A Caret mark (section 3.8): a solid peak whose base sweeps up into a
/// notch, filling a 30 × 20 /BBox (the view maps it onto /Rect less /RD).
pub fn caret(color: Option<&Color>, alpha: f64) -> Form {
    let [x0, y0, x1, y1] = CARET_BOX;
    let mid = (x0 + x1) / 2.0;
    let mut c = Content::new();
    c.alpha(alpha, false);
    c.color(color.unwrap_or(&BLACK), false);
    c.m((x0, y0));
    c.l((mid, y1));
    c.l((x1, y0));
    c.c((22.0, 2.0), (18.0, 4.0), (mid, 7.0));
    c.c((12.0, 4.0), (8.0, 2.0), (x0, y0));
    c.op("h");
    c.op("f");
    c.finish(CARET_BOX, IDENTITY)
}

/// An icon stamp's box fitted inside `rect` about its centre (section
/// 3.9): the largest rectangle of the stamp's 3:1 aspect.
pub fn icon_fit(rect: [f64; 4]) -> [f64; 4] {
    let [x0, y0, x1, y1] = rect.map(|v| if v.is_finite() { v } else { 0.0 });
    let (x0, x1) = (x0.min(x1), x0.max(x1));
    let (y0, y1) = (y0.min(y1), y0.max(y1));
    // Halves first: no overflow near the largest numbers.
    let (cx, cy) = (x0 / 2.0 + x1 / 2.0, y0 / 2.0 + y1 / 2.0);
    let (hw, hh) = (x1 / 2.0 - x0 / 2.0, y1 / 2.0 - y0 / 2.0);
    let aspect = STAMP_W / STAMP_H;
    let (hw, hh) = if hw > hh * aspect {
        (hh * aspect, hh)
    } else {
        (hw, hw / aspect)
    };
    let fit = [cx - hw, cy - hh, cx + hw, cy + hh];
    if hw == x1 / 2.0 - x0 / 2.0 {
        // Keep the request's own edges exactly.
        [x0, fit[1], x1, fit[3]]
    } else {
        [fit[0], y0, fit[2], y1]
    }
}

/// The label of an icon stamp (decision A2): the name's words in
/// capitals, a space at each lower-to-upper case change and before a
/// digit run after a letter; WinAnsi.
fn stamp_label(name: &[u8]) -> Vec<u8> {
    let name = if name.is_empty() {
        b"Draft".as_slice()
    } else {
        name
    };
    let mut label = String::new();
    let mut previous: Option<char> = None;
    for &byte in name.iter().take(1024) {
        let c = char::from(byte);
        if let Some(p) = previous {
            let word = c.is_ascii_uppercase() && p.is_ascii_lowercase();
            let digits = c.is_ascii_digit() && p.is_alphabetic();
            if word || digits {
                label.push(' ');
            }
        }
        label.push(c.to_ascii_uppercase());
        previous = Some(c);
    }
    fonts::encode(&label)
}

/// An icon stamp for /Name `name` (section 3.9): the label in
/// Helvetica-Bold (decision A2) on a plate whose border has its four
/// corners cut off at 45°, all in `color`; `matrix` is the form's /Matrix.
pub fn stamp(name: &[u8], color: Option<&Color>, alpha: f64, matrix: [f64; 6]) -> Form {
    const FONT: &[u8] = b"HeBo";
    let face = Base14::HelveticaBold;
    let ink = color.cloned().unwrap_or(STAMP_INK);
    let mut c = Content::new();
    c.alpha(alpha, false);
    c.color(&ink, true);
    c.color(&ink, false);
    c.width(FRAME);
    c.op("2 j");
    let (x0, y0) = (FRAME / 2.0, FRAME / 2.0);
    let (x1, y1) = (STAMP_W - FRAME / 2.0, STAMP_H - FRAME / 2.0);
    c.polygon(&[
        (x0 + CUT, y0),
        (x1 - CUT, y0),
        (x1, y0 + CUT),
        (x1, y1 - CUT),
        (x1 - CUT, y1),
        (x0 + CUT, y1),
        (x0, y1 - CUT),
        (x0, y0 + CUT),
    ]);
    c.op("S");

    let label = stamp_label(name);
    let lines = stamp_lines(&label, face);
    if !lines.is_empty() {
        let (top, bottom) = face.extent(&label);
        let ext = label_height(top, bottom);
        let rows = lines.len() as f64;
        let widest = lines.iter().map(|l| face.width(l, 1.0)).fold(0.0, f64::max);
        let size = label_size(ext, rows, widest);
        let pitch = LABEL_PITCH * ext * size;
        let first = STAMP_H / 2.0 + (rows - 1.0) * pitch / 2.0 - (top + bottom) / 2.0 * size;
        c.op("q");
        c.re([FRAME, FRAME, STAMP_W - FRAME, STAMP_H - FRAME]);
        c.op("W n");
        c.op("BT");
        c.op(&format!("{} {} Tf", pdf_name(FONT), num(size)));
        let mut origin = (0.0, 0.0);
        for (i, l) in lines.iter().enumerate() {
            let x = (STAMP_W - face.width(l, size)) / 2.0;
            let y = first - i as f64 * pitch;
            c.with(&[x - origin.0, y - origin.1], "Td");
            c.op(&format!("{} Tj", text::literal(l)));
            origin = (x, y);
        }
        c.op("ET");
        c.op("Q");
        c.fonts
            .insert(FONT.to_vec(), Object::Dictionary(face.dictionary()));
    }
    c.finish([0.0, 0.0, STAMP_W, STAMP_H], matrix)
}

/// The stamp plate's border width, the corners it cuts, and the margin
/// between the border and the label.
const FRAME: f64 = 3.6;
const CUT: f64 = 7.0;
const MARGIN: f64 = 4.4;
/// Two label lines sit 1.35 glyph heights apart.
const LABEL_PITCH: f64 = 1.35;

/// The height of a label's glyphs (em), capitals' when it has no ink.
fn label_height(top: f64, bottom: f64) -> f64 {
    if top - bottom > 0.0 {
        top - bottom
    } else {
        0.718
    }
}

/// The size a label of `rows` lines, glyphs `ext` em tall and its widest
/// line `widest` em, is set at: one line 62% of the height inside the
/// margins, two lines 85%, or less to fit the width.
fn label_size(ext: f64, rows: f64, widest: f64) -> f64 {
    let inset = FRAME + MARGIN;
    let (aw, ah) = (STAMP_W - 2.0 * inset, STAMP_H - 2.0 * inset);
    let share = if rows > 1.0 { 0.85 } else { 0.62 };
    let size = share * ah / (ext * (1.0 + (rows - 1.0) * LABEL_PITCH));
    if widest > 0.0 {
        size.min(aw / widest)
    } else {
        size
    }
}

/// The label on one line, or on two broken at a space when that sets it
/// notably larger.
fn stamp_lines(label: &[u8], face: Base14) -> Vec<Vec<u8>> {
    if label.iter().all(|&b| b == b' ') {
        return Vec::new();
    }
    let (top, bottom) = face.extent(label);
    let ext = label_height(top, bottom);
    let one = label_size(ext, 1.0, face.width(label, 1.0));
    // Prefix sums of the advances, for every break at a space.
    let mut sums = Vec::with_capacity(label.len() + 1);
    sums.push(0.0);
    for &b in label {
        sums.push(sums.last().copied().unwrap_or(0.0) + face.advance(b) / 1000.0);
    }
    let total = sums.last().copied().unwrap_or(0.0);
    let mut best: Option<(f64, usize)> = None;
    for (i, &b) in label.iter().enumerate() {
        if b != b' ' || i == 0 || i + 1 == label.len() {
            continue;
        }
        let widest = sums[i].max(total - sums[i + 1]);
        let s = label_size(ext, 2.0, widest);
        if best.is_none_or(|(t, _)| s > t) {
            best = Some((s, i));
        }
    }
    match best {
        Some((s, i)) if s > one * 1.15 => vec![label[..i].to_vec(), label[i + 1..].to_vec()],
        _ => vec![label.to_vec()],
    }
}

/// An image stamp or signature (section 3.10).
pub fn image_stamp(image: ObjectId, placement: [f64; 6], matrix: [f64; 6]) -> Form {
    let content = if placement == IDENTITY {
        b"/I Do\n".to_vec()
    } else {
        let values: Vec<String> = placement.iter().map(|&v| num(v)).collect();
        format!("q\n{} cm\n/I Do\nQ\n", values.join(" ")).into_bytes()
    };
    let mut xobjects = Dictionary::new();
    xobjects.insert(b"I".to_vec(), Object::Reference(image));
    let mut resources = Dictionary::new();
    resources.insert(b"XObject".to_vec(), Object::Dictionary(xobjects));
    Form {
        bbox: [0.0, 0.0, 1.0, 1.0],
        matrix,
        resources,
        content,
    }
}

// ------------------------------------------------------------------ redact

/// The width of a Redact mark's outline.
const REDACT_WIDTH: f64 = 1.0;

/// A Redact mark's /Rect for `quads`: their union grown by the outline's
/// reach; None without quads.
pub(crate) fn redact_rect(quads: &[Quad]) -> Option<[f64; 4]> {
    bounds(quads.iter().flatten().map(|&p| pt(p))).map(|b| grow(b, REDACT_WIDTH))
}

/// A Redact mark (section 3.11): returns the form and the /Rect.
pub fn redact(rect: [f64; 4], quads: &[Quad], color: Option<&Color>) -> (Form, [f64; 4]) {
    let mut c = Content::new();
    c.color(color.unwrap_or(&REDACT_INK), true);
    c.width(REDACT_WIDTH);
    match redact_rect(quads) {
        Some(outer) => {
            c.op("2 j");
            for q in quads {
                let [ul, ur, ll, lr] = sane_quad(q);
                c.polygon(&[ll, lr, ur, ul]);
                c.op("S");
            }
            (c.finish(outer, IDENTITY), outer)
        }
        None => {
            let r = sane_rect(rect);
            c.re(grow(r, -REDACT_WIDTH / 2.0));
            c.op("S");
            (c.finish(r, IDENTITY), r)
        }
    }
}

// ------------------------------------------------------------------ FreeText

/// What a FreeText draws.
#[derive(Clone, Debug, PartialEq)]
pub struct FreeText<'a> {
    pub text: &'a str,
    /// The /DA font name: the resource name of the main font.
    pub font_name: &'a [u8],
    pub font: Base14,
    /// The CID font for Han characters (spec 07 `scripts::han_font`).
    pub han: Cjk,
    pub size: f64,
    /// Text and border colour (/DA colour). None: black.
    pub color: Option<Color>,
    /// Border width (/BS /W or /Border); 0 or less: no border.
    pub border: f64,
    /// Dash array (/BS /S /D); empty: solid.
    pub dash: Vec<f64>,
    /// Background (/C); None: no fill.
    pub background: Option<Color>,
    /// /Q: 0 left, 1 centred, 2 right; anything else is 0.
    pub quadding: i64,
    /// /Rotate, a multiple of 90 in 0..360.
    pub rotate: i32,
    pub opacity: f64,
}

/// Characters of a FreeText laid out at most (app limit).
pub const MAX_FREE_TEXT_CHARS: usize = 65_536;

/// Distance between baselines of FreeText lines at font size `size`.
pub fn line_height(size: f64) -> f64 {
    LINE_FACTOR * size
}

/// The FreeText appearance: text laid out in `rect` (/Rect less /RD),
/// form /BBox `bbox` (the /Rect). Background, then the border inside
/// `rect`, then the text, clipped inside the border and padded from it by
/// the border width (none without a border, so a fit box is tight).
pub fn free_text(bbox: [f64; 4], rect: [f64; 4], box_: &FreeText<'_>) -> Form {
    let r = sane_rect(rect);
    let mut c = Content::new();
    c.alpha(box_.opacity, false);
    if let Some(background) = &box_.background {
        c.color(background, false);
        c.re(r);
        c.op("f");
    }
    let ink = box_.color.clone().unwrap_or(BLACK);
    let (w, h) = (r[2] - r[0], r[3] - r[1]);
    let border = stroke_width(box_.border)
        .map(|b| b.min(w.min(h) / 2.0))
        .filter(|&b| b > 0.0);
    if let Some(b) = border {
        c.color(&ink, true);
        c.width(b);
        c.dash(&box_.dash);
        c.re(grow(r, -b / 2.0));
        c.op("S");
    }
    let pad = border.unwrap_or(0.0);
    let clip = grow(r, -pad);
    free_text_lines(&mut c, box_, clip, pad, &ink);
    c.finish(sane_rect(bbox), IDENTITY)
}

/// The text of a FreeText, clipped to `clip` and laid out `pad` inside it.
fn free_text_lines(c: &mut Content, box_: &FreeText<'_>, clip: [f64; 4], pad: f64, ink: &Color) {
    let size = if box_.size.is_finite() && box_.size > 0.0 {
        box_.size.clamp(0.1, 1000.0)
    } else {
        12.0
    };
    let [x0, y0, x1, y1] = grow(clip, -pad);
    let (bw, bh) = (x1 - x0, y1 - y0);
    // The frame the text reads upright in (section 3.3.1): its width,
    // height, and the matrix from it to the form.
    let (tw, th, frame) = match box_.rotate.rem_euclid(360) {
        90 => (bh, bw, [0.0, 1.0, -1.0, 0.0, x1, y0]),
        180 => (bw, bh, [-1.0, 0.0, 0.0, -1.0, x1, y1]),
        270 => (bh, bw, [0.0, -1.0, 1.0, 0.0, x0, y1]),
        _ => (bw, bh, [1.0, 0.0, 0.0, 1.0, x0, y0]),
    };
    if !(tw > 0.0 && th > 0.0 && clip[2] > clip[0] && clip[3] > clip[1]) {
        return;
    }
    let metrics = box_.font.metrics();
    let fonts = Fonts {
        latin: &metrics,
        family: box_.font.family(),
        han: box_.han,
        latin_extended: true,
    };
    let text: String = box_.text.chars().take(MAX_FREE_TEXT_CHARS).collect();
    let lh = line_height(size);
    let room = ((th + pad) / lh).floor() + 2.0;
    let limit = if room.is_finite() && room < MAX_LINES as f64 {
        room.max(1.0) as usize
    } else {
        MAX_LINES
    };
    let (lines, _) = scripts::lines(&text, &fonts, size, tw, limit);
    if lines.iter().all(|line| line.glyphs.is_empty()) {
        return;
    }
    // The face's glyph box sits centred in each line's slot, the first
    // slot at the top: its middle is half a line below the top.
    let (top, bottom) = box_.font.glyph_box();
    let first = th - ((lh - (top - bottom) * size) / 2.0).max(0.0) - top * size;
    c.op("q");
    c.re(clip);
    c.op("W n");
    c.with(&frame, "cm");
    c.color(ink, false);
    c.op("BT");
    let mut shown = Shown::default();
    for (i, line) in lines.iter().enumerate() {
        let y = first - i as f64 * lh;
        if y < -pad {
            break;
        }
        let free = tw - line.width * size;
        let x = match box_.quadding {
            1 => free / 2.0,
            2 => free,
            _ => 0.0,
        };
        show(c, &mut shown, box_, &line.glyphs, (x, y), size);
    }
    c.op("ET");
    c.op("Q");
}

/// What the text object has set so far: the line origin and the font.
#[derive(Default)]
struct Shown {
    origin: Point,
    font: Option<Vec<u8>>,
}

/// Shows `glyphs` from `at`, one string per font run.
fn show(
    c: &mut Content,
    shown: &mut Shown,
    box_: &FreeText<'_>,
    glyphs: &[Glyph],
    at: Point,
    size: f64,
) {
    let mut x = at.0;
    let mut start = 0;
    while start < glyphs.len() {
        let slot = glyphs[start].slot;
        let end = glyphs[start..]
            .iter()
            .position(|g| g.slot != slot)
            .map_or(glyphs.len(), |n| start + n);
        let run = &glyphs[start..end];
        let name = font_resource(c, box_, slot);
        if shown.font.as_ref() != Some(&name) {
            c.op(&format!("{} {} Tf", pdf_name(&name), num(size)));
            shown.font = Some(name);
        }
        c.with(&[x - shown.origin.0, at.1 - shown.origin.1], "Td");
        c.op(&format!("{} Tj", scripts::string(run)));
        shown.origin = (x, at.1);
        x += text::advance(run) * size;
        start = end;
    }
}

/// The resource name of `slot`'s font, added to the resources on first
/// use: the /DA name for the main font, the /DA name and a suffix for the
/// fallbacks (spec 07 section 5), CJK fonts with their CIDFont and
/// descriptor direct (section 3.3.4).
fn font_resource(c: &mut Content, box_: &FreeText<'_>, slot: Slot) -> Vec<u8> {
    let main = box_.font_name.to_vec();
    if !c.fonts.contains_key(main.as_slice()) {
        c.fonts
            .insert(main.clone(), Object::Dictionary(box_.font.dictionary()));
    }
    let family = box_.font.family();
    let (name, font) = match slot {
        Slot::Latin => return main,
        Slot::Eight(eight) => (
            [&main[..], b"_", eight.suffix().as_bytes()].concat(),
            eight.font(family.base_font(), family),
        ),
        Slot::Cjk(cjk) => (
            [&main[..], b"_", cjk.name().as_bytes()].concat(),
            cjk.font(Object::Dictionary(
                cjk.descendant_font(Object::Dictionary(cjk.descriptor())),
            )),
        ),
    };
    if !c.fonts.contains_key(name.as_slice()) {
        c.fonts.insert(name.clone(), Object::Dictionary(font));
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_round_to_four_decimals_and_stay_finite() {
        assert_eq!(num(0.123_456), ".1235");
        assert_eq!(num(-2.0), "-2");
        assert_eq!(num(f64::NAN), "0");
        assert_eq!(num(f64::INFINITY), "0");
        assert_eq!(num(1e300), "1000000000000");
        assert_eq!(num(1e-9), "0");
        assert_eq!(pdf_name(b"A b/#"), "/A#20b#2F#23");
    }

    #[test]
    fn stamp_labels_split_words_and_digit_runs() {
        for (name, label) in [
            (&b"NotForPublicRelease"[..], &b"NOT FOR PUBLIC RELEASE"[..]),
            (b"Draft2", b"DRAFT 2"),
            (b"", b"DRAFT"),
            (b"approved", b"APPROVED"),
            (b"AB12cd", b"AB 12CD"),
            (b"a\xe9b", b"A\xe9B"),
        ] {
            assert_eq!(stamp_label(name), label);
        }
        let face = Base14::HelveticaBold;
        assert_eq!(stamp_lines(b"APPROVED", face).len(), 1);
        assert_eq!(stamp_lines(b"NOT FOR PUBLIC RELEASE", face).len(), 2);
        assert!(stamp_lines(b"   ", face).is_empty());
    }

    #[test]
    fn squiggles_keep_to_their_vertex_budget() {
        let thin = |n: usize, h: f64| -> Vec<Quad> {
            (0..n)
                .map(|i| {
                    let y = i as f64;
                    [(0.0, y + h), (590.0, y + h), (0.0, y), (590.0, y)]
                })
                .collect()
        };
        for (n, h) in [(1, 14.0), (400, 0.001), (16_384, 0.001), (3, 0.0)] {
            let segments = squiggle_segments(&thin(n, h));
            let vertices: usize = segments.iter().filter(|&&s| s > 0).map(|s| s + 1).sum();
            assert!(vertices <= MAX_SQUIGGLY_VERTICES, "{n}: {vertices}");
            if n <= 400 {
                assert!(segments.iter().all(|&s| s >= 2), "{n}");
            }
        }
    }

    #[test]
    fn fitted_forms_land_on_their_inner_box() {
        let form = caret(None, 1.0);
        let placed = fitted(
            form,
            [10.0, 10.0, 70.0, 30.0],
            [0.0, 0.0, 80.0, 40.0],
            false,
        );
        assert_eq!(placed.bbox, [0.0, 0.0, 80.0, 40.0]);
        assert!(placed.content.starts_with(b"q\n2 0 0 1 10 10 cm\n"));
        let icon = fitted(
            note(b"Key", None, 1.0),
            [0.0, 0.0, 40.0, 20.0],
            [0.0; 4],
            true,
        );
        assert!(icon.content.starts_with(b"q\n1 0 0 1 10 0 cm\n"));
    }

    #[test]
    fn hostile_free_text_stays_bounded() {
        let mut box_ = FreeText {
            text: "word ",
            font_name: b"Helv",
            font: Base14::Helvetica,
            han: Cjk::Gb1,
            size: 1e-30,
            color: None,
            border: f64::NAN,
            dash: Vec::new(),
            background: None,
            quadding: 7,
            rotate: -90,
            opacity: f64::NAN,
        };
        let text = "word ".repeat(20_000);
        box_.text = &text;
        let form = free_text([0.0, 0.0, 600.0, 800.0], [0.0, 0.0, 600.0, 800.0], &box_);
        assert!(form.content.len() < 4 << 20);
        assert!(form.resources.get(b"ExtGState".as_slice()).is_none());
    }
}

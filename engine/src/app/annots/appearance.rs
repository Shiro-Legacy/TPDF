// SPDX-License-Identifier: AGPL-3.0-or-later
//! Appearance streams drawn as MuPDF 1.28.1's `pdf_update_appearance` draws
//! them for the kinds it synthesizes: the same operators, /BBox, /Matrix
//! and /Resources (measured on MuPDF's output, see design/app-annots.md),
//! and the /Rect and /RD MuPDF derives from the geometry. Numbers are
//! written at single precision, as MuPDF computes them. All geometry is PDF
//! user space.
use crate::object::{Dictionary, Object, ObjectId};
use crate::serialize::compact_real;

use super::fonts::{self, Base14};
use super::{Color, Point, Quad};
use crate::app::forms::scripts::{self, Cjk, Eight, Fonts, Glyph, Slot};
use crate::app::forms::text::{Family, Metrics};

/// The Bézier constant of MuPDF's ellipses.
const KAPPA: f64 = 0.551915;
/// How far a highlight's rounded ends reach, per unit of quad height.
const BULGE: f64 = 0.23571;
/// The icon stamps' 0.6 degree tilt, as MuPDF prints it.
const TILT: &str = ".99994519 .0104717849 -.0104717849 .99994519 0 0 cm\n";
/// The size of an icon stamp's appearance; its rect takes this aspect.
pub const STAMP_BOX: [f64; 2] = [190.0, 50.0];
/// A stamp /Name, its label lines (text and baseline) and font size.
type StampLabel = (&'static [u8], &'static [(&'static str, f64)], f64);

/// MuPDF's icon stamp labels: the lines (text and baseline) and the font
/// size; any other /Name is drawn as itself at 20 points.
const STAMP_LABELS: [StampLabel; 15] = [
    (b"Approved", &[("APPROVED", 13.0)], 30.0),
    (b"AsIs", &[("AS IS", 13.0)], 30.0),
    (b"Confidential", &[("CONFIDENTIAL", 17.0)], 20.0),
    (b"Departmental", &[("DEPARTMENTAL", 17.0)], 20.0),
    (b"Experimental", &[("EXPERIMENTAL", 17.0)], 20.0),
    (b"Expired", &[("EXPIRED", 13.0)], 30.0),
    (b"Final", &[("FINAL", 13.0)], 30.0),
    (b"ForComment", &[("FOR COMMENT", 17.0)], 20.0),
    (
        b"ForPublicRelease",
        &[("FOR PUBLIC", 26.0), ("RELEASE", 8.5)],
        18.0,
    ),
    (b"NotApproved", &[("NOT APPROVED", 17.0)], 20.0),
    (
        b"NotForPublicRelease",
        &[("NOT FOR", 26.0), ("PUBLIC RELEASE", 8.5)],
        18.0,
    ),
    (b"Sold", &[("SOLD", 13.0)], 30.0),
    (b"TopSecret", &[("TOP SECRET", 14.0)], 26.0),
    (b"Draft", &[("DRAFT", 13.0)], 30.0),
    (b"", &[("DRAFT", 13.0)], 30.0),
];

pub const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

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
    /// The stream dictionary, keys in MuPDF's order.
    pub fn dictionary(&self) -> Dictionary {
        let mut form = Dictionary::new();
        form.insert(b"Type".to_vec(), Object::name("XObject"));
        form.insert(b"Subtype".to_vec(), Object::name("Form"));
        form.insert(b"BBox".to_vec(), numbers(&self.bbox));
        form.insert(b"Matrix".to_vec(), numbers(&self.matrix));
        if !self.resources.is_empty() {
            let resources = Object::Dictionary(self.resources.clone());
            form.insert(b"Resources".to_vec(), resources);
        }
        form
    }
}

/// How a shape is painted: /C, /IC, the border width and dash, /CA.
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

/// A PDF number object: an integer when the value is one.
pub fn number(value: f64) -> Object {
    let value = if value.is_finite() { value } else { 0.0 };
    if value.fract() == 0.0 && value.abs() < 1e15 {
        Object::Integer(value as i64)
    } else {
        Object::Real(value)
    }
}

pub fn numbers(values: &[f64]) -> Object {
    Object::Array(values.iter().map(|&v| number(v)).collect())
}

/// Content operators being written.
#[derive(Default)]
struct Ops(Vec<u8>);

impl Ops {
    fn num(&mut self, value: f64) {
        let value = if value.is_finite() { value } else { 0.0 };
        self.0.extend_from_slice(compact_real(value).as_bytes());
    }
    /// `values` then `op` on one line.
    fn op(&mut self, values: &[f64], op: &str) {
        for &value in values {
            self.num(value);
            self.0.push(b' ');
        }
        self.0.extend_from_slice(op.as_bytes());
        self.0.push(b'\n');
    }
    fn text(&mut self, text: &str) {
        self.0.extend_from_slice(text.as_bytes());
    }
    fn color(&mut self, color: &Color, stroke: bool) {
        let (values, op): (&[f64], _) = match color {
            Color::Gray(g) => (std::slice::from_ref(g), if stroke { "G" } else { "g" }),
            Color::Rgb(rgb) => (rgb, if stroke { "RG" } else { "rg" }),
            Color::Cmyk(cmyk) => (cmyk, if stroke { "K" } else { "k" }),
        };
        self.op(values, op);
    }
    fn point(&mut self, p: Point, op: &str) {
        self.op(&[p.0, p.1], op);
    }
    fn curve(&mut self, a: Point, b: Point, c: Point) {
        self.op(&[a.0, a.1, b.0, b.1, c.0, c.1], "c");
    }
    /// `/name ` with the bytes a name cannot hold written as `#xx`.
    fn name(&mut self, name: &[u8]) {
        self.0.push(b'/');
        for &byte in name {
            if (0x21..=0x7E).contains(&byte) && !b"()<>[]{}/%#".contains(&byte) {
                self.0.push(byte);
            } else {
                self.0.extend_from_slice(format!("#{byte:02X}").as_bytes());
            }
        }
        self.0.push(b' ');
    }
    /// A PDF literal string of WinAnsi bytes.
    fn string(&mut self, bytes: &[u8]) {
        self.0.push(b'(');
        for &byte in bytes {
            if matches!(byte, b'(' | b')' | b'\\') {
                self.0.push(b'\\');
            }
            self.0.push(byte);
        }
        self.0.push(b')');
    }
}

/// `/H gs` with its ExtGState when the annotation is translucent (or, for
/// a highlight, always: it multiplies).
fn opacity(ops: &mut Ops, resources: &mut Dictionary, opacity: f64, multiply: bool) {
    if opacity >= 1.0 && !multiply {
        return;
    }
    let mut state = Dictionary::new();
    state.insert(b"Type".to_vec(), Object::name("ExtGState"));
    if multiply {
        state.insert(b"BM".to_vec(), Object::name("Multiply"));
    }
    if opacity < 1.0 {
        state.insert(b"CA".to_vec(), number(opacity));
        state.insert(b"ca".to_vec(), number(opacity));
    }
    let mut states = Dictionary::new();
    states.insert(b"H".to_vec(), Object::Dictionary(state));
    resources.insert(b"ExtGState".to_vec(), Object::Dictionary(states));
    ops.text("/H gs\n");
}

/// The axis-aligned bounds of points; None for none.
pub fn bounds(points: impl IntoIterator<Item = Point>) -> Option<[f64; 4]> {
    points.into_iter().fold(None, |acc, (x, y)| match acc {
        None => Some([x, y, x, y]),
        Some([x0, y0, x1, y1]) => Some([x0.min(x), y0.min(y), x1.max(x), y1.max(y)]),
    })
}

pub fn grow(rect: [f64; 4], by: f64) -> [f64; 4] {
    [rect[0] - by, rect[1] - by, rect[2] + by, rect[3] + by]
}

fn union(a: Option<[f64; 4]>, b: [f64; 4]) -> [f64; 4] {
    match a {
        None => b,
        Some(a) => [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ],
    }
}

fn add(a: Point, b: Point) -> Point {
    (a.0 + b.0, a.1 + b.1)
}

fn sub(a: Point, b: Point) -> Point {
    (a.0 - b.0, a.1 - b.1)
}

fn scale(a: Point, s: f64) -> Point {
    (a.0 * s, a.1 * s)
}

fn length(a: Point) -> f64 {
    a.0.hypot(a.1)
}

/// `v / |v|`, zero for a zero vector.
fn unit(v: Point) -> Point {
    let h = length(v);
    if h > 0.0 {
        scale(v, 1.0 / h)
    } else {
        (0.0, 0.0)
    }
}

/// A Text annotation's icon in its 16 x 16 box: the background in /C (white
/// without one) under a black or white glyph, whichever reads on it.
pub fn note(icon: &[u8], color: Option<&Color>, alpha: f64) -> Form {
    let mut ops = Ops::default();
    let mut resources = Dictionary::new();
    opacity(&mut ops, &mut resources, alpha, false);
    match color {
        Some(color) => ops.color(color, false),
        None => ops.text("1 g\n"),
    }
    ops.text("1 w\n0.5 0.5 15 15 re\nb\n1 0 0 -1 4 12 cm\n");
    let dark = color.is_some_and(|color| color.luminance() < 0.25);
    ops.text(if dark { "1 g\n" } else { "0 g\n" });
    ops.text(match icon {
        b"Comment" => {
            ".09 0 m\n.03 0 0 .04 0 .09 c\n0 5.9 l\n0 5.95 .04 6 .09 6 c\n6 6 l\n8 8 l\n8 .08 l\n\
             8 .02 7.96 -.01 7.91 -.01 c\n.1 -.01 l\nh\n"
        }
        b"Key" => {
            "5.5 0 m\n4.12 0 3 1.12 3 2.5 c\n3 2.66 3 2.82 3.03 2.97 c\n0 6 l\n0 8 l\n3 8 l\n3 6 l\n\
             5 6 l\n5 5 l\n5.03 4.97 l\n5.18 5 5.34 5 5.5 5 c\n6.88 5 8 3.88 8 2.5 c\n\
             8 1.12 6.88 0 5.5 0 c\nh\n6 1 m\n6.55 1 7 1.45 7 2 c\n7 2.55 6.55 3 6 3 c\n\
             5.45 3 5 2.55 5 2 c\n5 1.45 5.45 1 6 1 c\nh\n"
        }
        b"Note" | b"" => "0 0 8 1 re\n0 2 8 1 re\n0 4 8 1 re\n0 6 8 1 re\n",
        b"Help" => {
            "1 0 0 1 1 0 cm\n2.47 0 m\n1.62 0 .99 .26 .59 .66 c\n.19 1.06 .05 1.56 0 1.94 c\n1 2.07 l\n\
             1.04 1.82 1.12 1.57 1.31 1.38 c\n1.50 1.19 1.80 1 2.47 1.00 c\n\
             3.13 1 3.49 1.16 3.69 1.34 c\n3.89 1.52 3.97 1.74 3.97 2 c\n\
             3.97 2.83 3.63 3.06 3.13 3.50 c\n2.63 3.94 1.97 4.58 1.97 5.75 c\n1.97 6 l\n\
             2.97 6 l\n2.97 5.75 l\n2.97 4.92 3.28 4.69 3.78 4.25 c\n\
             4.28 3.81 4.97 3.17 4.97 2 c\n4.97 1.52 4.80 .98 4.38 .59 c\n\
             3.95 .20 3.31 0 2.47 .00 c\nh\n1.97 7 m\n1.97 8 l\n2.97 8 l\n2.97 7 l\n1.97 7 l\nh\n"
        }
        b"NewParagraph" => "8 8 m\n4 0 l\n0 8 l\n",
        b"Paragraph" => {
            "7 0 m\n2 0 l\n1 0 0 1 0 2 c\n0 3 1 4 2 4 c\n3 4 l\n3 8 l\n4 8 l\n4 1 l\n5 1 l\n5 8 l\n\
             6 8 l\n6 1 l\n7 1 l\n"
        }
        b"Insert" => "8 5 m\n4 0 l\n0 5 l\n",
        _ => "4 0 m\n3 3 l\n0 3 l\n2.5 5 l\n1.5 8 l\n4 6 l\n6.5 8 l\n5.5 5 l\n8 3 l\n5 3 l\n4 0 l\nh\n",
    });
    ops.text("f\n");
    Form {
        bbox: [0.0, 0.0, 16.0, 16.0],
        matrix: IDENTITY,
        resources,
        content: ops.0,
    }
}

/// A Highlight: each quad filled with rounded ends, multiplied onto the
/// page. Returns the form and the /Rect MuPDF derives.
pub fn highlight(quads: &[Quad], color: Option<&Color>, alpha: f64) -> (Form, [f64; 4]) {
    let mut ops = Ops::default();
    let mut resources = Dictionary::new();
    opacity(&mut ops, &mut resources, alpha, true);
    if let Some(color) = color {
        ops.color(color, false);
    }
    let mut rect = None;
    for &[ul, ur, ll, lr] in quads {
        let side = sub(ul, ll);
        let h = length(side);
        let u = unit(side);
        let n = (-u.1, u.0);
        let k = h * BULGE;
        let c1 = add(ll, scale(add(n, u), k));
        let c2 = add(ul, scale(sub(n, u), k));
        let c3 = add(ur, scale((-n.0 - u.0, -n.1 - u.1), k));
        let c4 = add(lr, scale(sub(u, n), k));
        ops.point(ll, "m");
        ops.curve(c1, c2, ul);
        ops.point(ur, "l");
        ops.curve(c3, c4, lr);
        ops.text("f\n");
        let corners = bounds([ul, ur, ll, lr]).unwrap_or_default();
        let reach = bounds([c1, c2, c3, c4]).unwrap_or_default();
        rect = Some(union(Some(union(rect, grow(corners, h / 16.0))), reach));
    }
    let rect = rect.unwrap_or_default();
    let form = Form {
        bbox: rect,
        matrix: IDENTITY,
        resources,
        content: ops.0,
    };
    (form, rect)
}

/// Which line a text markup draws through its quads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Underline,
    StrikeOut,
    Squiggly,
}

/// An Underline, StrikeOut or Squiggly: a line h/16 wide through each quad
/// (1/7 and 3/7 of the height up; a zigzag of h/7 steps along the bottom).
pub fn text_line(
    mark: Mark,
    quads: &[Quad],
    color: Option<&Color>,
    alpha: f64,
) -> (Form, [f64; 4]) {
    let mut ops = Ops::default();
    let mut resources = Dictionary::new();
    opacity(&mut ops, &mut resources, alpha, false);
    if let Some(color) = color {
        ops.color(color, true);
    }
    let widen = match mark {
        Mark::Squiggly => zigzag_widening(quads),
        _ => 1.0,
    };
    let mut rect = None;
    for &[ul, ur, ll, lr] in quads {
        let side = sub(ul, ll);
        let h = length(side);
        ops.op(&[h / 16.0], "w");
        match mark {
            Mark::Underline | Mark::StrikeOut => {
                let at = if mark == Mark::Underline { 1.0 } else { 3.0 } / 7.0;
                ops.point(add(ll, scale(side, at)), "m");
                ops.point(add(lr, scale(side, at)), "l");
            }
            Mark::Squiggly => {
                let along = sub(lr, ll);
                let w = length(along) as f32;
                let (u, v) = (unit(along), unit(side));
                let step = (h / 7.0 * widen) as f32;
                // MuPDF steps in single precision until it passes the end.
                let mut x = 0f32;
                let mut up = false;
                let mut first = true;
                for _ in 0..MAX_ZIGZAG + 2 {
                    let rise = if up { h / 7.0 } else { 0.0 };
                    let p = add(add(ll, scale(u, f64::from(x))), scale(v, rise));
                    ops.point(p, if first { "m" } else { "l" });
                    first = false;
                    if x >= w || step <= 0.0 {
                        break;
                    }
                    x += step;
                    up = !up;
                }
            }
        }
        ops.text("S\n");
        let corners = bounds([ul, ur, ll, lr]).unwrap_or_default();
        rect = Some(union(rect, grow(corners, h / 16.0)));
    }
    let rect = rect.unwrap_or_default();
    let form = Form {
        bbox: rect,
        matrix: IDENTITY,
        resources,
        content: ops.0,
    };
    (form, rect)
}

/// The most zigzag points one Squiggly draws over all its quads.
const MAX_ZIGZAG: usize = 32_768;

/// How much wider than MuPDF's h/7 a Squiggly's zigzag steps are, so that
/// all its quads together take at most `MAX_ZIGZAG` points: 1 (MuPDF's
/// zigzag) unless they would take more.
fn zigzag_widening(quads: &[Quad]) -> f64 {
    let needed: f64 = quads
        .iter()
        .map(|&[ul, _, ll, lr]| {
            let steps = length(sub(lr, ll)) / (length(sub(ul, ll)) / 7.0);
            if steps.is_finite() { steps + 2.0 } else { 2.0 }
        })
        .sum();
    let most = MAX_ZIGZAG as f64;
    if needed > most { needed / most } else { 1.0 }
}

/// The border lines a shape strokes: width, dash and colours.
fn shape_style(ops: &mut Ops, resources: &mut Dictionary, style: &Style) {
    opacity(ops, resources, style.opacity, false);
    if !style.dash.is_empty() {
        ops.text("[");
        for (index, &dash) in style.dash.iter().enumerate() {
            if index > 0 {
                ops.text(" ");
            }
            ops.num(dash);
        }
        ops.text("]0 d\n");
    }
    ops.op(&[style.width], "w");
    if let Some(stroke) = &style.stroke {
        ops.color(stroke, true);
    }
    if let Some(fill) = &style.fill {
        ops.color(fill, false);
    }
}

/// The painting operator for a closed shape: stroke with /C, fill with /IC.
fn paint(style: &Style, closed: bool) -> &'static str {
    match (style.stroke.is_some(), style.fill.is_some(), closed) {
        (true, true, _) => "b\n",
        (false, true, _) => "f\n",
        (true, false, true) => "s\n",
        (true, false, false) => "S\n",
        (false, false, _) => "n\n",
    }
}

/// MuPDF's border allowance for a Square, Circle or Polygon set from a
/// rectangle: /RD on each side, half the width plus half a point.
pub fn border_allowance(width: f64) -> f64 {
    (width + 1.0) / 2.0
}

/// A Square or Circle whose border runs along `inner`, drawn in `bbox`.
pub fn rectangle(inner: [f64; 4], bbox: [f64; 4], ellipse: bool, style: &Style) -> Form {
    let mut ops = Ops::default();
    let mut resources = Dictionary::new();
    shape_style(&mut ops, &mut resources, style);
    let [x0, y0, x1, y1] = inner;
    if ellipse {
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (kx, ky) = ((x1 - x0) / 2.0 * KAPPA, (y1 - y0) / 2.0 * KAPPA);
        ops.point((cx, y1), "m");
        ops.curve((cx + kx, y1), (x1, cy + ky), (x1, cy));
        ops.curve((x1, cy - ky), (cx + kx, y0), (cx, y0));
        ops.curve((cx - kx, y0), (x0, cy - ky), (x0, cy));
        ops.curve((x0, cy + ky), (cx - kx, y1), (cx, y1));
    } else {
        ops.op(&[x0, y0, x1 - x0, y1 - y0], "re");
    }
    ops.text(paint(style, false));
    Form {
        bbox,
        matrix: IDENTITY,
        resources,
        content: ops.0,
    }
}

/// A Line from `a` to `b` with its two line endings; returns the /Rect.
pub fn line(a: Point, b: Point, endings: [&[u8]; 2], style: &Style) -> (Form, [f64; 4]) {
    let mut ops = Ops::default();
    let mut resources = Dictionary::new();
    shape_style(&mut ops, &mut resources, style);
    ops.point(a, "m");
    ops.point(b, "l");
    ops.text(if style.stroke.is_some() { "S\n" } else { "n\n" });
    let w = style.width;
    let mut rect = grow(bounds([a, b]).unwrap_or_default(), w);
    let d = unit(sub(b, a));
    for (point, direction, ending) in [(a, d, endings[0]), (b, scale(d, -1.0), endings[1])] {
        if let Some(reach) = line_ending(&mut ops, point, direction, ending, style) {
            rect = union(Some(rect), reach);
        }
    }
    let form = Form {
        bbox: rect,
        matrix: IDENTITY,
        resources,
        content: ops.0,
    };
    (form, rect)
}

/// One line ending at `p`, the line running on in direction `d`; returns
/// the area it covers.
fn line_ending(
    ops: &mut Ops,
    p: Point,
    d: Point,
    ending: &[u8],
    style: &Style,
) -> Option<[f64; 4]> {
    let w = style.width;
    let n = (-d.1, d.0);
    let stroke_fill = |ops: &mut Ops| ops.text(paint(style, true));
    let stroke = |ops: &mut Ops| ops.text(if style.stroke.is_some() { "S\n" } else { "n\n" });
    let arrow = |ops: &mut Ops, d: Point, closed: bool| {
        let r = w.max(1.0);
        let (l, h) = (8.8 * r, 4.5 * r);
        let n = (-d.1, d.0);
        let p1 = add(p, add(scale(d, l), scale(n, h)));
        let p2 = add(p, sub(scale(d, l), scale(n, h)));
        ops.point(p1, "m");
        ops.point(p, "l");
        ops.point(p2, "l");
        if closed {
            ops.text("h\n");
        }
        grow(bounds([p, p1, p2]).unwrap_or_default(), w)
    };
    let (x, y) = p;
    let reach = match ending {
        b"Square" | b"Circle" | b"Diamond" => {
            let r = (w * 2.5).max(2.5);
            match ending {
                b"Square" => ops.op(&[x - r, y - r, r * 2.0, r * 2.0], "re"),
                b"Circle" => {
                    let m = r * KAPPA;
                    ops.point((x, y + r), "m");
                    ops.curve((x + m, y + r), (x + r, y + m), (x + r, y));
                    ops.curve((x + r, y - m), (x + m, y - r), (x, y - r));
                    ops.curve((x - m, y - r), (x - r, y - m), (x - r, y));
                    ops.curve((x - r, y + m), (x - m, y + r), (x, y + r));
                }
                _ => {
                    ops.point((x, y + r), "m");
                    ops.point((x + r, y), "l");
                    ops.point((x, y - r), "l");
                    ops.point((x - r, y), "l");
                }
            }
            stroke_fill(ops);
            grow([x, y, x, y], r + w)
        }
        b"OpenArrow" | b"ROpenArrow" => {
            let reverse = ending == b"ROpenArrow";
            let reach = arrow(ops, if reverse { scale(d, -1.0) } else { d }, false);
            stroke(ops);
            reach
        }
        b"ClosedArrow" | b"RClosedArrow" => {
            let reverse = ending == b"RClosedArrow";
            let reach = arrow(ops, if reverse { scale(d, -1.0) } else { d }, true);
            stroke_fill(ops);
            reach
        }
        b"Butt" => {
            let r = (w * 3.0).max(3.0);
            ops.point(add(p, scale(n, r)), "m");
            ops.point(sub(p, scale(n, r)), "l");
            stroke(ops);
            grow([x, y, x, y], r + w)
        }
        b"Slash" => {
            let r = (w * 5.0).max(5.0);
            let angle = d.1.atan2(d.0) - 30f64.to_radians();
            let (c, s) = (angle.cos() * r, angle.sin() * r);
            ops.point((x - c, y - s), "m");
            ops.point((x + c, y + s), "l");
            stroke(ops);
            grow([x, y, x, y], r + w)
        }
        _ => return None,
    };
    Some(reach)
}

/// A Polygon (closed) or PolyLine through `vertices`; returns the /Rect.
pub fn poly(vertices: &[Point], closed: bool, style: &Style) -> (Form, [f64; 4]) {
    let mut ops = Ops::default();
    let mut resources = Dictionary::new();
    shape_style(&mut ops, &mut resources, style);
    for (index, &vertex) in vertices.iter().enumerate() {
        ops.point(vertex, if index == 0 { "m" } else { "l" });
    }
    if closed {
        ops.text("h\n");
        ops.text(paint(style, false));
    } else {
        ops.text(if style.stroke.is_some() { "S\n" } else { "n\n" });
    }
    let rect = grow(
        bounds(vertices.iter().copied()).unwrap_or_default(),
        border_allowance(style.width),
    );
    let form = Form {
        bbox: rect,
        matrix: IDENTITY,
        resources,
        content: ops.0,
    };
    (form, rect)
}

/// MuPDF's /RD of an Ink: the width plus six points on each side.
pub fn ink_allowance(width: f64) -> f64 {
    width + 6.0
}

/// An Ink: every stroke as straight segments with round caps and joins, in
/// one path. Returns the /Rect.
pub fn ink(strokes: &[Vec<Point>], style: &Style) -> (Form, [f64; 4]) {
    let mut ops = Ops::default();
    let mut resources = Dictionary::new();
    shape_style(&mut ops, &mut resources, style);
    ops.text("1 J\n1 j\n");
    for stroke in strokes {
        for (index, &point) in stroke.iter().enumerate() {
            ops.point(point, if index == 0 { "m" } else { "l" });
        }
    }
    ops.text(if style.stroke.is_some() { "S\n" } else { "n\n" });
    let points = strokes.iter().flatten().copied();
    let rect = grow(
        bounds(points).unwrap_or_default(),
        ink_allowance(style.width),
    );
    let form = Form {
        bbox: rect,
        matrix: IDENTITY,
        resources,
        content: ops.0,
    };
    (form, rect)
}

/// A Caret in its 20 x 14 box, filled with /C (black without one).
pub fn caret(color: Option<&Color>, alpha: f64) -> Form {
    let mut ops = Ops::default();
    let mut resources = Dictionary::new();
    opacity(&mut ops, &mut resources, alpha, false);
    if let Some(color) = color {
        ops.color(color, false);
    }
    ops.text("0 0 m\n10 0 10 7 10 14 c\n10 7 10 0 20 0 c\nf\n");
    Form {
        bbox: [0.0, 0.0, 20.0, 14.0],
        matrix: IDENTITY,
        resources,
        content: ops.0,
    }
}

/// An icon stamp's rect: the icon's 190:50 box scaled to fit inside
/// `rect` and centred in it, as MuPDF fits it (a rect wider than 190:50
/// keeps its height, a narrower one its width).
pub fn icon_fit(rect: [f64; 4]) -> [f64; 4] {
    let [width, height] = STAMP_BOX;
    let (w, h) = ((rect[2] - rect[0]).max(0.0), (rect[3] - rect[1]).max(0.0));
    let scale = (w / width).min(h / height);
    let (w, h) = (width * scale, height * scale);
    let (cx, cy) = ((rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0);
    [cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0]
}

/// An icon stamp: its label in Times-Bold inside a border, tilted 0.6
/// degrees, in a 190 x 50 box. `matrix` turns it upright on a rotated page.
pub fn stamp(name: &[u8], color: Option<&Color>, alpha: f64, matrix: [f64; 6]) -> Form {
    let mut ops = Ops::default();
    let mut resources = Dictionary::new();
    opacity(&mut ops, &mut resources, alpha, false);
    let mut font = Dictionary::new();
    font.insert(
        b"Times".to_vec(),
        Object::Dictionary(Base14::TimesBold.dictionary()),
    );
    resources.insert(b"Font".to_vec(), Object::Dictionary(font));
    if let Some(color) = color {
        ops.color(color, false);
        ops.color(color, true);
    }
    ops.text(TILT);
    ops.text("2 w\n2 2 186 44 re\nS\n");
    let own = String::from_utf8_lossy(name).into_owned();
    let (lines, size) = STAMP_LABELS
        .iter()
        .find(|(known, _, _)| *known == name)
        .map_or_else(
            || (vec![(own.as_str(), 17.0)], 20.0),
            |(_, lines, size)| (lines.to_vec(), *size),
        );
    for (text, y) in lines {
        let bytes = fonts::encode(text);
        let width = Base14::TimesBold.width(&bytes, size);
        ops.text("BT\n/Times ");
        ops.op(&[size], "Tf");
        ops.op(&[(STAMP_BOX[0] - width) / 2.0, y], "Td");
        ops.string(&bytes);
        ops.text(" Tj\nET\n");
    }
    Form {
        bbox: [0.0, 0.0, STAMP_BOX[0], STAMP_BOX[1]],
        matrix,
        resources,
        content: ops.0,
    }
}

/// An image stamp or signature: image `image` stretched over the unit box
/// (which maps onto /Rect), placed by `placement` (EXIF orientation).
pub fn image_stamp(image: ObjectId, placement: [f64; 6], matrix: [f64; 6]) -> Form {
    let mut ops = Ops::default();
    if placement == IDENTITY {
        ops.text("/I Do\n");
    } else {
        ops.text("q\n");
        ops.op(&placement, "cm");
        ops.text("/I Do\nQ\n");
    }
    let mut xobjects = Dictionary::new();
    xobjects.insert(b"I".to_vec(), Object::Reference(image));
    let mut resources = Dictionary::new();
    resources.insert(b"XObject".to_vec(), Object::Dictionary(xobjects));
    Form {
        bbox: [0.0, 0.0, 1.0, 1.0],
        matrix,
        resources,
        content: ops.0,
    }
}

/// A Redact mark: a red outline of each quad, or of the rect inset by a
/// point when it has none. Returns the /Rect.
pub fn redact(rect: [f64; 4], quads: &[Quad], color: Option<&Color>) -> (Form, [f64; 4]) {
    let mut ops = Ops::default();
    match color {
        Some(color) => ops.color(color, true),
        None => ops.text("1 0 0 RG\n"),
    }
    let mut bbox = rect;
    if quads.is_empty() {
        let [x0, y0, x1, y1] = rect;
        ops.point((x0 + 1.0, y0 + 1.0), "m");
        ops.point((x1 - 1.0, y0 + 1.0), "l");
        ops.point((x1 - 1.0, y1 - 1.0), "l");
        ops.point((x0 + 1.0, y1 - 1.0), "l");
        ops.text("s\n");
    } else {
        let mut all = None;
        for &[ul, ur, ll, lr] in quads {
            ops.point(ll, "m");
            ops.point(lr, "l");
            ops.point(ur, "l");
            ops.point(ul, "l");
            ops.text("s\n");
            all = Some(union(all, bounds([ul, ur, ll, lr]).unwrap_or_default()));
        }
        bbox = grow(all.unwrap_or(rect), 1.0);
    }
    let form = Form {
        bbox,
        matrix: IDENTITY,
        resources: Dictionary::new(),
        content: ops.0,
    };
    (form, bbox)
}

/// What a FreeText draws: its /DA, border, background and alignment.
#[derive(Clone, Debug, PartialEq)]
pub struct FreeText<'a> {
    pub text: &'a str,
    /// The /DA font name, the resource the text is shown with.
    pub font_name: &'a [u8],
    pub font: Base14,
    /// The CID font Han characters take (by /Lang, `scripts::han_font`).
    pub han: Cjk,
    pub size: f64,
    pub color: Option<Color>,
    pub border: f64,
    pub background: Option<Color>,
    /// /Q: 0 left, 1 centred, 2 right.
    pub quadding: i64,
    /// /Rotate, a multiple of 90.
    pub rotate: i32,
    pub opacity: f64,
}

/// Characters of a FreeText's text laid out at most; the app writes at
/// most 4,096 UTF-16 units, so only a foreign text is cut.
pub const MAX_FREE_TEXT_CHARS: usize = 65_536;
/// Lines of a FreeText laid out at most.
const MAX_FREE_TEXT_LINES: usize = 65_536;

/// MuPDF's line height for a font size.
pub fn line_height(size: f64) -> f64 {
    size * 1.2
}

/// The form family of a base 14 font: the Greek, Cyrillic and Latin
/// fallback fonts take its advances.
fn family(font: Base14) -> Family {
    match font {
        Base14::HelveticaBold | Base14::HelveticaBoldOblique => Family::HelveticaBold,
        Base14::TimesRoman | Base14::TimesBold | Base14::TimesItalic | Base14::TimesBoldItalic => {
            Family::Times
        }
        Base14::Courier
        | Base14::CourierBold
        | Base14::CourierOblique
        | Base14::CourierBoldOblique => Family::Courier,
        _ => Family::Helvetica,
    }
}

/// The lines of a FreeText as MuPDF's `write_variable_text` lays them out
/// (`forms::scripts::lines`): a line break at every line feed or carriage
/// return (a CRLF pair is one), and a break after the last space once a
/// line is wider than `width`; a word is never split. Each character is in
/// the DA font when WinAnsi has it, else in MuPDF's Greek or Cyrillic font
/// of its family, a CJK CID font, or (where MuPDF embeds a font program)
/// the Latin font `scripts::Eight::Latin`; `?` when none has it (then the
/// flag is set). At most `limit` lines, from the first
/// `MAX_FREE_TEXT_CHARS` characters.
pub(crate) fn free_text_lines(
    text: &str,
    font: Base14,
    han: Cjk,
    size: f64,
    width: f64,
    limit: usize,
) -> (Vec<scripts::Line>, bool) {
    let cut = match text.char_indices().nth(MAX_FREE_TEXT_CHARS) {
        Some((at, _)) => text.get(..at).unwrap_or(text),
        None => text,
    };
    let normalized = cut.replace("\r\n", "\n");
    let latin = Metrics::from_widths((0..=255u8).map(|code| font.advance(code)).collect());
    let fonts = Fonts {
        latin: &latin,
        family: family(font),
        han,
        latin_extended: true,
    };
    scripts::lines(&normalized, &fonts, size, width, limit)
}

/// A run's string: WinAnsi or 8-bit bytes as a literal (raw, as MuPDF
/// writes them), UTF-16 in hex for a CID font.
fn run_string(ops: &mut Ops, run: &[Glyph]) {
    match run.first().map(|glyph| glyph.slot) {
        Some(Slot::Cjk(_)) => ops.text(&scripts::string(run)),
        _ => {
            let bytes: Vec<u8> = run.iter().map(|glyph| glyph.code as u8).collect();
            ops.string(&bytes);
        }
    }
}

/// A FreeText's text box in `rect` (its /Rect less /RD), laid out in the
/// frame its /Rotate turns, in a form whose /BBox is `bbox` (its /Rect).
/// Lines wholly below the box, which its clip hides, are left out. The
/// resources name the DA font and each fallback font a run takes, under
/// MuPDF's names (`<DA>GRK`, `<DA>CYR`, `Song`, ...; `<DA>LAT` for the
/// Latin font); a CJK font's descendant and descriptor are direct here and
/// become objects when the form is stored (`annots::put_appearance`).
pub fn free_text(bbox: [f64; 4], rect: [f64; 4], box_: &FreeText<'_>) -> Form {
    let mut ops = Ops::default();
    let mut resources = Dictionary::new();
    opacity(&mut ops, &mut resources, box_.opacity, false);
    let [x0, y0, x1, y1] = rect;
    let b = box_.border.max(0.0);
    if let Some(background) = &box_.background {
        ops.color(background, false);
    }
    match &box_.color {
        Some(color) => ops.color(color, true),
        None => ops.text("0 G\n"),
    }
    ops.op(&[b], "w");
    if box_.background.is_some() {
        ops.op(&[x0, y0, x1 - x0, y1 - y0], "re");
        ops.text("f\n");
    }
    if b > 0.0 {
        let h = b / 2.0;
        ops.op(&[x0 + h, y0 + h, x1 - x0 - b, y1 - y0 - b], "re");
        ops.text("S\n");
    }
    ops.op(
        &[x0 + b, y0 + b, x1 - x0 - 2.0 * b, y1 - y0 - 2.0 * b],
        "re",
    );
    ops.text("W\nn\nq\n");
    let (w, h, frame) = match box_.rotate {
        90 => (y1 - y0, x1 - x0, [0.0, 1.0, -1.0, 0.0, x1, y0]),
        180 => (x1 - x0, y1 - y0, [-1.0, 0.0, 0.0, -1.0, x1, y1]),
        270 => (y1 - y0, x1 - x0, [0.0, -1.0, 1.0, 0.0, x0, y1]),
        _ => (x1 - x0, y1 - y0, [1.0, 0.0, -0.0, 1.0, x0, y0]),
    };
    ops.op(&frame, "cm");
    ops.text("BT\n");
    match &box_.color {
        Some(color) => ops.color(color, false),
        None => ops.text("0 g\n"),
    }
    let size = box_.size;
    let lh = line_height(size);
    let pad = 2.0 * b;
    // Lines whose baseline lies above `-2 size` (below it they are clipped).
    let visible = (h - pad + size * 0.4 + 2.0 * size.abs()) / lh;
    let limit = if visible.is_finite() {
        visible.clamp(0.0, MAX_FREE_TEXT_LINES as f64) as usize + 1
    } else {
        MAX_FREE_TEXT_LINES
    };
    let (lines, lossy) =
        free_text_lines(box_.text, box_.font, box_.han, size, w - 2.0 * pad, limit);
    // A text MuPDF draws with a font program (a character its simple and
    // CJK fonts lack) has its first baseline 0.9 size below the top, else
    // 0.8 size (measured, 1.28.1).
    let embedded = lossy
        || (lines.iter().flat_map(|line| &line.glyphs))
            .any(|glyph| glyph.slot == Slot::Eight(Eight::Latin));
    let top = h - pad + size * if embedded { 0.3 } else { 0.4 };
    ops.op(&[pad, top], "Td");
    let mut used: Vec<Slot> = Vec::new();
    let mut x = pad;
    let mut baseline = top;
    for line in &lines {
        baseline -= lh;
        if baseline + 2.0 * size.abs() < 0.0 {
            break;
        }
        let width = line.width * size;
        let at = match box_.quadding {
            1 => pad + (w - 2.0 * pad - width) / 2.0,
            2 => w - pad - width,
            _ => pad,
        };
        ops.op(&[at - x, -lh], "Td");
        x = at;
        for run in line.glyphs.chunk_by(|a, b| a.slot == b.slot) {
            let Some(first) = run.first() else {
                continue;
            };
            if !used.contains(&first.slot) {
                used.push(first.slot);
            }
            ops.name(&font_resource(box_, first.slot));
            ops.op(&[size], "Tf");
            run_string(&mut ops, run);
            ops.text(" Tj\n");
        }
    }
    ops.text("ET\nQ\n");
    let mut fonts = Dictionary::new();
    fonts.insert(
        box_.font_name.to_vec(),
        Object::Dictionary(box_.font.dictionary()),
    );
    for slot in used {
        let font = match slot {
            Slot::Latin => continue,
            Slot::Eight(eight) => eight.font(box_.font.base_font(), family(box_.font)),
            Slot::Cjk(cjk) => {
                let descriptor = Object::Dictionary(cjk.descriptor());
                cjk.font(Object::Dictionary(cjk.descendant_font(descriptor)))
            }
        };
        fonts.insert(font_resource(box_, slot), Object::Dictionary(font));
    }
    resources.insert(b"Font".to_vec(), Object::Dictionary(fonts));
    Form {
        bbox,
        matrix: IDENTITY,
        resources,
        content: ops.0,
    }
}

/// The resource name of the font a run is drawn in.
fn font_resource(box_: &FreeText<'_>, slot: Slot) -> Vec<u8> {
    match slot {
        Slot::Latin => box_.font_name.to_vec(),
        Slot::Eight(eight) => [box_.font_name, eight.suffix().as_bytes()].concat(),
        Slot::Cjk(cjk) => cjk.name().as_bytes().to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content(form: &Form) -> String {
        String::from_utf8(form.content.clone()).unwrap()
    }

    #[test]
    fn stamp_labels_are_mupdfs() {
        // MuPDF 1.28.1's output for each name (the label lines only).
        for (name, expected) in [
            (
                "TopSecret",
                "BT\n/Times 26 Tf\n13.009 14 Td\n(TOP SECRET) Tj\nET\n",
            ),
            ("AsIs", "BT\n/Times 30 Tf\n57.905 13 Td\n(AS IS) Tj\nET\n"),
            (
                "Departmental",
                "BT\n/Times 20 Tf\n10 17 Td\n(DEPARTMENTAL) Tj\nET\n",
            ),
            (
                "NotForPublicRelease",
                "BT\n/Times 18 Tf\n54.248 26 Td\n(NOT FOR) Tj\nET\nBT\n/Times 18 Tf\n16.736 8.5 Td\n(PUBLIC RELEASE) Tj\nET\n",
            ),
            ("Void", "BT\n/Times 20 Tf\n74.44 17 Td\n(Void) Tj\nET\n"),
        ] {
            let form = stamp(name.as_bytes(), None, 1.0, IDENTITY);
            let text = content(&form);
            assert!(text.ends_with(expected), "{name}: {text}");
            assert!(text.starts_with(TILT), "{name}");
        }
    }

    #[test]
    fn a_note_glyph_reads_on_its_background() {
        for (color, glyph) in [
            (Color::Rgb([1.0, 1.0, 0.0]), "0 g"),
            (Color::Rgb([0.0, 0.0, 1.0]), "1 g"),
            (Color::Rgb([0.8, 0.0, 0.0]), "1 g"),
            (Color::Gray(0.25), "0 g"),
            (Color::Gray(0.22), "1 g"),
            (Color::Cmyk([0.0, 1.0, 0.0, 0.0]), "0 g"),
        ] {
            let text = content(&note(b"Note", Some(&color), 1.0));
            assert!(
                text.contains(&format!("cm\n{glyph}\n")),
                "{color:?}: {text}"
            );
        }
        let plain = content(&note(b"Unknown", None, 0.5));
        assert!(plain.starts_with("/H gs\n1 g\n1 w\n"), "{plain}");
        assert!(
            plain.contains("4 0 m\n3 3 l\n"),
            "a Star for an unknown name"
        );
    }

    #[test]
    fn free_text_lines_break_as_mupdf_breaks_them() {
        let lines = |text: &str| -> Vec<String> {
            free_text_lines(text, Base14::Helvetica, Cjk::Song, 12.0, 160.0, 100)
                .0
                .into_iter()
                .map(|line| line.glyphs.iter().map(|g| g.ch).collect())
                .collect()
        };
        assert_eq!(
            lines("xxxxxxxxxxxxxxxxxxxxxxxxxx yyyy"),
            ["xxxxxxxxxxxxxxxxxxxxxxxxxx ", "yyyy"]
        );
        assert_eq!(
            lines("short word then averyveryverylongwordthatdoesnotfitatall end"),
            [
                "short word then ",
                "averyveryverylongwordthatdoesnotfitatall ",
                "end"
            ]
        );
        assert_eq!(
            lines("one two  three   four     five six seven eight nine ten"),
            [
                "one two  three   four     five ",
                "six seven eight nine ten"
            ]
        );
        // CR LF is one break; a trailing break and an empty text draw no
        // line (MuPDF 1.28.1).
        assert_eq!(lines("a\rb\n\nc"), ["a", "b", "", "c"]);
        assert_eq!(lines("a\r\nb"), ["a", "b"]);
        assert_eq!(lines("a\n"), ["a"]);
        assert!(lines("").is_empty());
        // A long word alone is never split.
        assert_eq!(lines(&"w".repeat(40)).len(), 1);
        // The line limit and the character cap.
        let many = "a\n".repeat(1000);
        let (cut, _) = free_text_lines(&many, Base14::Helvetica, Cjk::Song, 12.0, 160.0, 7);
        assert_eq!(cut.len(), 7);
        let huge = "a ".repeat(MAX_FREE_TEXT_CHARS);
        let (all, lossy) =
            free_text_lines(&huge, Base14::Helvetica, Cjk::Song, 12.0, 1e9, usize::MAX);
        assert!(!lossy);
        let drawn: usize = all.iter().map(|line| line.glyphs.len()).sum();
        assert_eq!(drawn, MAX_FREE_TEXT_CHARS);
    }

    #[test]
    fn free_text_runs_take_mupdfs_fallback_fonts() {
        let text = |text: &str, quadding: i64| {
            let box_ = FreeText {
                text,
                font_name: b"Helv",
                font: Base14::Helvetica,
                han: Cjk::Song,
                size: 12.0,
                color: Some(Color::Rgb([0.0, 0.0, 0.0])),
                border: 0.0,
                background: None,
                quadding,
                rotate: 0,
                opacity: 1.0,
            };
            let form = free_text(
                [40.0, 712.0, 200.0, 752.0],
                [40.0, 712.0, 200.0, 752.0],
                &box_,
            );
            let fonts = form.resources.get(b"Font".as_slice()).unwrap();
            let names: Vec<String> = (fonts.as_dict().unwrap().iter())
                .map(|(name, _)| String::from_utf8_lossy(name).into_owned())
                .collect();
            let content: String = form.content.iter().map(|&b| char::from(b)).collect();
            (names, content)
        };
        // MuPDF 1.28.1's runs and fonts for the same text (MuPDF prints the
        // line height in single precision, 14.400001).
        let (names, content) = text(
            "abc \u{3a9}mega \u{416}\u{443}\u{43a} \u{4e2d}\u{6587} end",
            0,
        );
        assert_eq!(names, ["Helv", "HelvGRK", "HelvCYR", "Song"]);
        assert!(
            content.ends_with(
                "0 44.8 Td\n0 -14.4 Td\n/Helv 12 Tf\n(abc ) Tj\n/HelvGRK 12 Tf\n(\u{d9}) Tj\n\
                 /Helv 12 Tf\n(mega ) Tj\n/HelvCYR 12 Tf\n(\u{f6}\u{d5}\u{cb} ) Tj\n\
                 /Song 12 Tf\n<4e2d65870020> Tj\n/Helv 12 Tf\n(end) Tj\nET\nQ\n"
            ),
            "{content}"
        );
        // Common characters before the first scripted one take its font.
        let (names, content) = text("12 \u{3a9}", 0);
        assert_eq!(names, ["Helv", "HelvGRK"]);
        assert!(
            content.contains("/HelvGRK 12 Tf\n(12 \u{d9}) Tj\n"),
            "{content}"
        );
        // Centred lines leave the break's trailing space out of the width.
        let (_, content) = text("xxxxxxxxxxxxxxxxxxxxxxxxxx yyyy", 1);
        assert!(content.contains("2 -14.4 Td\n"), "{content}");
        assert!(content.contains("66 -14.4 Td\n"), "{content}");
        // Latin letters WinAnsi lacks take the Latin font (MuPDF embeds a
        // font program); what no font has is `?`. Such a text's first
        // baseline is MuPDF's for a font program: 0.9 size below the top.
        let (names, content) = text("Za\u{17c}\u{f3}\u{142} \u{1ebf} \u{1f600}", 0);
        assert_eq!(names, ["Helv", "HelvLAT"]);
        assert!(
            content.contains("0 43.6 Td\n0 -14.4 Td\n/Helv 12 Tf\n(Za) Tj\n/HelvLAT 12 Tf\n(\u{f7}) Tj\n/Helv 12 Tf\n(\u{f3}) Tj\n/HelvLAT 12 Tf\n(\u{c2}) Tj\n/Helv 12 Tf\n( ? ?) Tj\n"),
            "{content}"
        );
        let (_, content) = text("\u{1f600}", 0);
        assert!(content.contains("0 43.6 Td\n"), "{content}");
    }

    #[test]
    fn an_icon_stamp_fits_inside_its_rect() {
        // MuPDF 1.28.1's rects (page space): a wide rect keeps its height,
        // a narrow one its width, both centred.
        assert_eq!(
            icon_fit([10.0, 10.0, 410.0, 60.0]),
            [115.0, 10.0, 305.0, 60.0]
        );
        assert_eq!(
            icon_fit([10.0, 10.0, 600.0, 100.0]),
            [134.0, 10.0, 476.0, 100.0]
        );
        assert_eq!(
            icon_fit([300.0, 600.0, 500.0, 650.0]),
            [305.0, 600.0, 495.0, 650.0]
        );
        let narrow = icon_fit([10.0, 10.0, 150.0, 60.0]);
        assert_eq!((narrow[0], narrow[2]), (10.0, 150.0));
        assert!((narrow[3] - narrow[1] - 140.0 * 50.0 / 190.0).abs() < 1e-9);
        assert!((narrow[1] + narrow[3] - 70.0).abs() < 1e-9);
        assert_eq!(icon_fit([5.0, 5.0, 5.0, 9.0]), [5.0, 7.0, 5.0, 7.0]);
        let huge = icon_fit([-1e30, -1e30, 1e30, 1e30]);
        assert!(huge.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn hostile_geometry_stays_finite_and_bounded() {
        let quad = [(0.0, 1e300), (1e300, 1e300), (0.0, 0.0), (1e300, 0.0)];
        let (form, rect) = text_line(Mark::Squiggly, &[quad], None, 1.0);
        assert!(form.content.len() < 4 << 20);
        assert!(rect.iter().all(|v| !v.is_nan()));
        let (form, _) = highlight(&[[(0.0, 0.0); 4]], None, 1.0);
        assert!(!content(&form).contains("NaN"));
    }
}

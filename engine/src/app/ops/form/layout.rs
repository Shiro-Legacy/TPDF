// SPDX-License-Identifier: MIT
//! The question form's page layout: pure geometry in page space (points, y
//! down from the page top) and the label ink each form page draws.
//!
//! Questions flow down one column inside a 54 pt margin, each block (its
//! label, help line and fields) kept whole on one page; a block that does
//! not fit starts the next form page. A question whose field was moved by
//! hand (pinned) keeps its place, its label just above it, and the flow
//! goes around it, as around any other field on the page.
//!
//! Beside one field and a row of choices, a block can be a rating (stars
//! or numbered buttons across), a grid (statements down, one row of
//! buttons each, the scale's words over the columns) or a section heading
//! (no fields; it can start a new page).

use crate::builtin_metrics::{BuiltinFace, BuiltinMetricLookup, BuiltinMetrics};
use crate::simple_encoding::{NamedEncoding, SimpleEncoding};

use super::super::stamp::win_ansi_byte;

/// The page margin on every side.
pub const MARGIN: f64 = 54.0;
/// Between two blocks.
const GAP: f64 = 16.0;
/// Between a label (or help line) and its fields.
const UNDER_LABEL: f64 = 4.0;
/// Room kept around an obstacle.
const CLEAR: f64 = 8.0;
pub const LABEL_SIZE: f64 = 10.0;
pub const HELP_SIZE: f64 = 9.0;
pub const CHOICE_SIZE: f64 = 10.0;
pub const TITLE_SIZE: f64 = 18.0;
pub const DESC_SIZE: f64 = 10.0;
pub const SECTION_SIZE: f64 = 14.0;
/// A grid's column words.
pub const HEADER_SIZE: f64 = 8.0;
/// The text box after an "Other" choice.
pub const OTHER_W: f64 = 160.0;
const OTHER_H: f64 = 18.0;
/// A rating star.
pub const STAR: f64 = 18.0;
/// A choice button or checkbox.
pub const BOX: f64 = 12.0;
/// One row of choices down the page.
const ROW: f64 = 18.0;

/// What a question puts under its label.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// One field box (text, drop-down, signature), at most as wide as the column.
    Field { w: f64, h: f64 },
    /// One small box per choice with its words beside it (choice buttons or checkboxes);
    /// `other`: the last choice is "Other", with a text box after its words (the last rect).
    Boxes { choices: Vec<String>, other: bool },
    /// One checkbox with the question itself beside it ("I agree").
    Agree,
    /// A rating across: `steps` stars, or numbered buttons.
    Rating { steps: usize, stars: bool },
    /// One row of buttons per statement under the columns' words (row by row).
    Grid {
        columns: Vec<String>,
        rows: Vec<String>,
    },
    /// A section heading (the label), its description (the help) and a rule;
    /// `new_page` starts it on a fresh form page.
    Section { new_page: bool },
}

impl Shape {
    pub fn boxes(choices: Vec<String>) -> Self {
        Self::Boxes {
            choices,
            other: false,
        }
    }
}

/// A field on a form page that is no question's: the flow keeps clear of it, and a
/// choice button's word (`words`) is drawn to its right.
#[derive(Clone, Debug)]
pub struct Fixed {
    pub slot: usize,
    pub rect: [f64; 4],
    pub words: String,
}

/// One question to lay out.
#[derive(Clone, Debug)]
pub struct Block {
    /// The label as drawn (` *` already added when required).
    pub label: String,
    pub help: String,
    pub shape: Shape,
    /// A hand-placed question: its form page slot and its widgets' rects (a section:
    /// one rect, the box its heading was moved to).
    pub pinned: Option<(usize, Vec<[f64; 4]>)>,
}

/// Something drawn on a form page.
#[derive(Clone, Debug, PartialEq)]
pub enum Ink {
    /// A line of text, its baseline at `y`.
    Text {
        x: f64,
        y: f64,
        size: f64,
        bold: bool,
        grey: bool,
        text: String,
    },
    /// A thin horizontal rule.
    Rule { x0: f64, x1: f64, y: f64 },
    /// A grey five-pointed star's outline (a rating star not picked), centred at (x, y).
    Star { x: f64, y: f64, r: f64 },
}

/// The layout: how many form pages it needs, each block's page slot and
/// widget rects (in order), and the ink of each slot.
#[derive(Clone, Debug, Default)]
pub struct Laid {
    pub slots: usize,
    pub placed: Vec<(usize, Vec<[f64; 4]>)>,
    pub ink: Vec<Vec<Ink>>,
    /// Each block's drawn words (label, help, choice words; a section's heading and
    /// rule): its slot and box, what a press on the page picks.
    pub boxes: Vec<Option<(usize, [f64; 4])>>,
    /// The title's box on the first form page.
    pub title_box: Option<[f64; 4]>,
}

/// Helvetica and Helvetica-Bold widths, as WinAnsi text.
pub struct Fonts {
    regular: BuiltinMetrics,
    bold: BuiltinMetrics,
}

impl Default for Fonts {
    fn default() -> Self {
        let encoding = || SimpleEncoding::from_named(NamedEncoding::WinAnsi);
        Self {
            regular: BuiltinMetrics::from_encoding(BuiltinFace::Helvetica, encoding()),
            bold: BuiltinMetrics::from_encoding(BuiltinFace::HelveticaBold, encoding()),
        }
    }
}

impl Fonts {
    /// The width of `text` at `size`, in points.
    pub fn width(&self, text: &str, size: f64, bold: bool) -> f64 {
        let metrics = if bold { &self.bold } else { &self.regular };
        let units: f64 = text
            .chars()
            .map(|c| match win_ansi_byte(c).map(|b| metrics.lookup(b)) {
                Some(BuiltinMetricLookup::Advance(advance)) => advance.value(),
                _ => 500.0,
            })
            .sum();
        units / 1000.0 * size
    }

    /// `text` broken into lines no wider than `width` (a word longer than a
    /// line is broken between letters).
    pub fn wrap(&self, text: &str, size: f64, bold: bool, width: f64) -> Vec<String> {
        let mut lines = Vec::new();
        for para in text.split('\n') {
            let mut line = String::new();
            for word in para.split_whitespace() {
                let joined = if line.is_empty() {
                    word.to_string()
                } else {
                    format!("{line} {word}")
                };
                if self.width(&joined, size, bold) <= width {
                    line = joined;
                    continue;
                }
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                // The word alone, broken when it is wider than a line.
                for c in word.chars() {
                    let mut next = line.clone();
                    next.push(c);
                    if !line.is_empty() && self.width(&next, size, bold) > width {
                        lines.push(std::mem::take(&mut line));
                        next = c.to_string();
                    }
                    line = next;
                }
            }
            if !line.is_empty() {
                lines.push(line);
            }
        }
        lines
    }
}

fn lead(size: f64) -> f64 {
    size * 1.3
}

/// The baseline of a line whose box starts at `top`.
fn baseline(top: f64, size: f64) -> f64 {
    top + size * 0.85
}

fn text(x: f64, y: f64, size: f64, bold: bool, grey: bool, line: String) -> Ink {
    Ink::Text {
        x,
        y,
        size,
        bold,
        grey,
        text: line,
    }
}

fn union(rects: &[[f64; 4]]) -> Option<[f64; 4]> {
    rects.iter().copied().reduce(|a, b| {
        [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]
    })
}

/// Whether the choices fit across one row: at most five short ones (and the
/// "Other" text box after them).
fn across(fonts: &Fonts, choices: &[String], other: bool, width: f64) -> bool {
    let total: f64 = choices
        .iter()
        .map(|c| BOX + 4.0 + fonts.width(c, CHOICE_SIZE, false) + 18.0)
        .sum::<f64>()
        + if other { OTHER_W } else { 0.0 };
    choices.len() <= 5 && choices.iter().all(|c| c.chars().count() <= 12) && total <= width
}

/// A line of text centred on `cx`.
fn centred(fonts: &Fonts, cx: f64, y: f64, size: f64, line: String) -> Ink {
    let w = fonts.width(&line, size, false);
    text(cx - w / 2.0, y, size, false, false, line)
}

/// A grid's geometry: the statements' column width and each column's width.
fn grid_columns(width: f64, count: usize) -> (f64, f64) {
    let words = (width * 0.4).max(120.0).min(width - 40.0 * count as f64);
    (words, (width - words) / count.max(1) as f64)
}

/// A grid from (x0, y): its header and rows' ink and rects, and its height.
fn grid(
    fonts: &Fonts,
    columns: &[String],
    rows: &[String],
    x0: f64,
    y0: f64,
    width: f64,
) -> (f64, Vec<[f64; 4]>, Vec<Ink>) {
    let mut ink = Vec::new();
    let mut rects = Vec::new();
    let (words_w, col_w) = grid_columns(width, columns.len());
    let mut header = 0.0_f64;
    for (i, column) in columns.iter().enumerate() {
        let cx = x0 + words_w + col_w * (i as f64 + 0.5);
        let lines = fonts.wrap(column, HEADER_SIZE, false, col_w - 4.0);
        let mut top = y0;
        for line in lines {
            ink.push(centred(
                fonts,
                cx,
                baseline(top, HEADER_SIZE),
                HEADER_SIZE,
                line,
            ));
            top += lead(HEADER_SIZE);
        }
        header = header.max(top - y0);
    }
    let mut y = y0 + header + 3.0;
    ink.push(Ink::Rule {
        x0,
        x1: x0 + width,
        y,
    });
    y += 3.0;
    for row in rows {
        let lines = fonts.wrap(row, CHOICE_SIZE, false, words_w - 6.0);
        let h = (lines.len() as f64 * lead(CHOICE_SIZE) + 6.0).max(ROW + 2.0);
        let mut top = y + (h - lines.len() as f64 * lead(CHOICE_SIZE)) / 2.0;
        for line in lines {
            ink.push(text(
                x0,
                baseline(top, CHOICE_SIZE),
                CHOICE_SIZE,
                false,
                false,
                line,
            ));
            top += lead(CHOICE_SIZE);
        }
        let by = y + (h - BOX) / 2.0;
        for i in 0..columns.len() {
            let cx = x0 + words_w + col_w * (i as f64 + 0.5);
            rects.push([cx - BOX / 2.0, by, cx + BOX / 2.0, by + BOX]);
        }
        y += h;
    }
    (y - y0, rects, ink)
}

/// A block laid out from (x0, 0): its height, widget rects and ink.
fn build(fonts: &Fonts, block: &Block, x0: f64, width: f64) -> (f64, Vec<[f64; 4]>, Vec<Ink>) {
    let mut ink = Vec::new();
    let mut rects = Vec::new();
    let mut y = 0.0;
    if let Shape::Section { .. } = block.shape {
        for line in fonts.wrap(&block.label, SECTION_SIZE, true, width) {
            ink.push(text(
                x0,
                baseline(y, SECTION_SIZE),
                SECTION_SIZE,
                true,
                false,
                line,
            ));
            y += lead(SECTION_SIZE);
        }
        for line in fonts.wrap(&block.help, DESC_SIZE, false, width) {
            ink.push(text(
                x0,
                baseline(y, DESC_SIZE),
                DESC_SIZE,
                false,
                true,
                line,
            ));
            y += lead(DESC_SIZE);
        }
        y += 3.0;
        ink.push(Ink::Rule {
            x0,
            x1: x0 + width,
            y,
        });
        return (y + 2.0, rects, ink);
    }
    if block.shape == Shape::Agree {
        rects.push([x0, 1.0, x0 + BOX, 1.0 + BOX]);
        let lines = fonts.wrap(&block.label, LABEL_SIZE, true, width - BOX - 6.0);
        let mut top = 0.0;
        for line in lines {
            ink.push(text(
                x0 + BOX + 6.0,
                baseline(top, LABEL_SIZE),
                LABEL_SIZE,
                true,
                false,
                line,
            ));
            top += lead(LABEL_SIZE);
        }
        y = f64::max(top, BOX + 2.0);
    } else {
        for line in fonts.wrap(&block.label, LABEL_SIZE, true, width) {
            ink.push(text(
                x0,
                baseline(y, LABEL_SIZE),
                LABEL_SIZE,
                true,
                false,
                line,
            ));
            y += lead(LABEL_SIZE);
        }
    }
    for line in fonts.wrap(&block.help, HELP_SIZE, false, width) {
        ink.push(text(
            x0,
            baseline(y, HELP_SIZE),
            HELP_SIZE,
            false,
            true,
            line,
        ));
        y += lead(HELP_SIZE);
    }
    match &block.shape {
        Shape::Agree | Shape::Section { .. } => {}
        Shape::Rating { steps, stars } => {
            y += UNDER_LABEL;
            let mut x = x0;
            for step in 1..=*steps {
                if *stars {
                    rects.push([x, y, x + STAR, y + STAR]);
                    ink.push(Ink::Star {
                        x: x + STAR / 2.0,
                        y: y + STAR / 2.0,
                        r: STAR * 0.46,
                    });
                    x += STAR + 6.0;
                } else {
                    let words = step.to_string();
                    let w = fonts.width(&words, CHOICE_SIZE, false);
                    rects.push([x, y + 1.0, x + BOX, y + 1.0 + BOX]);
                    ink.push(text(
                        x + BOX + 4.0,
                        y + 11.0,
                        CHOICE_SIZE,
                        false,
                        false,
                        words,
                    ));
                    x += BOX + 4.0 + w + 14.0;
                }
            }
            y += if *stars { STAR } else { ROW - 2.0 };
        }
        Shape::Grid { columns, rows } => {
            y += UNDER_LABEL;
            let (h, r, i) = grid(fonts, columns, rows, x0, y, width);
            rects.extend(r);
            ink.extend(i);
            y += h;
        }
        Shape::Field { w, h } => {
            y += UNDER_LABEL;
            rects.push([x0, y, x0 + w.min(width), y + h]);
            y += h;
        }
        Shape::Boxes { choices, other } => {
            y += UNDER_LABEL;
            if across(fonts, choices, *other, width) {
                let mut x = x0;
                for choice in choices {
                    rects.push([x, y + 1.0, x + BOX, y + 1.0 + BOX]);
                    let words = fonts.width(choice, CHOICE_SIZE, false);
                    ink.push(text(
                        x + BOX + 4.0,
                        y + 11.0,
                        CHOICE_SIZE,
                        false,
                        false,
                        choice.clone(),
                    ));
                    x += BOX + 4.0 + words + 18.0;
                }
                if *other {
                    // The text box right after "Other".
                    let x = x - 12.0;
                    rects.push([
                        x,
                        y - 2.0,
                        x + OTHER_W.min(x0 + width - x),
                        y - 2.0 + OTHER_H,
                    ]);
                }
                y += ROW - 2.0;
            } else {
                for choice in choices {
                    rects.push([x0, y + 1.0, x0 + BOX, y + 1.0 + BOX]);
                    ink.push(text(
                        x0 + BOX + 4.0,
                        y + 11.0,
                        CHOICE_SIZE,
                        false,
                        false,
                        choice.clone(),
                    ));
                    y += ROW;
                }
                if *other && let Some(last) = choices.last() {
                    let x = x0 + BOX + 4.0 + fonts.width(last, CHOICE_SIZE, false) + 6.0;
                    let y = y - ROW;
                    rects.push([
                        x,
                        y - 2.0,
                        x + OTHER_W.min(x0 + width - x),
                        y - 2.0 + OTHER_H,
                    ]);
                }
                y -= ROW - BOX - 2.0;
            }
        }
    }
    (y, rects, ink)
}

/// A pinned block's ink where its widgets are, and the box it keeps clear.
fn pinned_ink(
    fonts: &Fonts,
    block: &Block,
    rects: &[[f64; 4]],
    page_w: f64,
) -> (Option<[f64; 4]>, Vec<Ink>) {
    let mut ink = Vec::new();
    let Some(u) = union(rects) else {
        return (None, ink);
    };
    let mut keep = u;
    match &block.shape {
        Shape::Agree => {
            let x = u[2] + 6.0;
            let width = (page_w - MARGIN - x).max(100.0);
            let mut top = u[1] - 1.0;
            for line in fonts.wrap(&block.label, LABEL_SIZE, true, width) {
                let w = fonts.width(&line, LABEL_SIZE, true);
                keep[2] = keep[2].max(x + w);
                ink.push(text(
                    x,
                    baseline(top, LABEL_SIZE),
                    LABEL_SIZE,
                    true,
                    false,
                    line,
                ));
                top += lead(LABEL_SIZE);
            }
            keep[3] = keep[3].max(top);
        }
        shape => {
            let numbers: Vec<String>;
            let words: &[String] = match shape {
                Shape::Boxes { choices, .. } => choices,
                Shape::Rating {
                    steps,
                    stars: false,
                } => {
                    numbers = (1..=*steps).map(|n| n.to_string()).collect();
                    &numbers
                }
                _ => &[],
            };
            if let Shape::Rating { stars: true, .. } = shape {
                for r in rects {
                    let size = (r[2] - r[0]).min(r[3] - r[1]);
                    ink.push(Ink::Star {
                        x: (r[0] + r[2]) / 2.0,
                        y: (r[1] + r[3]) / 2.0,
                        r: size * 0.46,
                    });
                }
            }
            if let Shape::Grid { columns, rows } = shape {
                // The columns' words over the first row, each statement left of its row.
                let cols = columns.len().max(1);
                for (rect, column) in rects.iter().zip(columns) {
                    let cx = (rect[0] + rect[2]) / 2.0;
                    let y = rect[1] - 4.0;
                    let w = fonts.width(column, HEADER_SIZE, false);
                    keep[1] = keep[1].min(y - HEADER_SIZE);
                    keep[0] = keep[0].min(cx - w / 2.0);
                    keep[2] = keep[2].max(cx + w / 2.0);
                    ink.push(centred(fonts, cx, y, HEADER_SIZE, column.clone()));
                }
                for (row, statement) in rows.iter().enumerate() {
                    let Some(rect) = rects.get(row * cols) else {
                        break;
                    };
                    let w = fonts.width(statement, CHOICE_SIZE, false);
                    let x = rect[0] - 8.0 - w;
                    keep[0] = keep[0].min(x);
                    ink.push(text(
                        x,
                        rect[1] + 10.0,
                        CHOICE_SIZE,
                        false,
                        false,
                        statement.clone(),
                    ));
                }
            }
            for (rect, choice) in rects.iter().zip(words) {
                let x = rect[2] + 4.0;
                let w = fonts.width(choice, CHOICE_SIZE, false);
                keep[2] = keep[2].max(x + w);
                ink.push(text(
                    x,
                    rect[1] + 10.0,
                    CHOICE_SIZE,
                    false,
                    false,
                    choice.clone(),
                ));
            }
            let width = (page_w - MARGIN - u[0]).max(120.0);
            let lines = fonts.wrap(&block.label, LABEL_SIZE, true, width);
            let above = keep[1].min(u[1]);
            // As far above its field as in the flow (a label moved by hand does not jump).
            let gap = UNDER_LABEL + lead(LABEL_SIZE) - baseline(0.0, LABEL_SIZE);
            let mut base = above - gap - lead(LABEL_SIZE) * (lines.len().saturating_sub(1)) as f64;
            let label_x = keep[0].min(u[0]);
            keep[1] = keep[1].min(base - LABEL_SIZE);
            for line in lines {
                let w = fonts.width(&line, LABEL_SIZE, true);
                keep[2] = keep[2].max(label_x + w);
                ink.push(text(label_x, base, LABEL_SIZE, true, false, line));
                base += lead(LABEL_SIZE);
            }
        }
    }
    (Some(keep), ink)
}

/// The first top at or below `y` where a block `h` high and `x0..x1`
/// across overlaps no obstacle.
fn clear(obstacles: &[[f64; 4]], mut y: f64, h: f64, x0: f64, x1: f64) -> f64 {
    for _ in 0..=obstacles.len() {
        let hit = obstacles
            .iter()
            .filter(|o| o[1] < y + h && o[3] > y && o[0] < x1 && o[2] > x0)
            .map(|o| o[3])
            .reduce(f64::max);
        match hit {
            Some(bottom) => y = bottom + CLEAR,
            None => break,
        }
    }
    y
}

/// The box `ink` covers (a line of text from its ascent to its descent).
fn ink_box(fonts: &Fonts, ink: &[Ink]) -> Option<[f64; 4]> {
    let rects: Vec<[f64; 4]> = ink
        .iter()
        .map(|item| match item {
            Ink::Text {
                x,
                y,
                size,
                bold,
                text,
                ..
            } => [
                *x,
                y - size * 0.9,
                x + fonts.width(text, *size, *bold),
                y + size * 0.25,
            ],
            Ink::Rule { x0, x1, y } => [*x0, y - 1.0, *x1, y + 1.0],
            Ink::Star { x, y, r } => [x - r, y - r, x + r, y + r],
        })
        .collect();
    union(&rects)
}

fn shifted(ink: Vec<Ink>, dy: f64) -> impl Iterator<Item = Ink> {
    ink.into_iter().map(move |item| match item {
        Ink::Text {
            x,
            y,
            size,
            bold,
            grey,
            text,
        } => Ink::Text {
            x,
            y: y + dy,
            size,
            bold,
            grey,
            text,
        },
        Ink::Rule { x0, x1, y } => Ink::Rule { x0, x1, y: y + dy },
        Ink::Star { x, y, r } => Ink::Star { x, y: y + dy, r },
    })
}

/// Lays out the form: `sizes` are the form pages' (width, height) in order
/// (a page added after them takes the last one's size), `fixed` the rects
/// of other fields on them (slot, rect) the flow keeps clear of.
/// `title_at`: the title's top left when it was moved by hand.
pub fn lay_out(
    sizes: &[(f64, f64)],
    title: &str,
    desc: &str,
    title_at: Option<[f64; 2]>,
    blocks: &[Block],
    fixed: &[Fixed],
) -> Laid {
    let fonts = Fonts::default();
    let size_of = |slot: usize| {
        sizes
            .get(slot)
            .or(sizes.last())
            .copied()
            .unwrap_or((612.0, 792.0))
    };
    let mut laid = Laid {
        slots: 1,
        placed: vec![(0, Vec::new()); blocks.len()],
        ink: Vec::new(),
        boxes: vec![None; blocks.len()],
        title_box: None,
    };
    let mut obstacles: Vec<Vec<[f64; 4]>> = Vec::new();
    let grow = |laid: &mut Laid, obstacles: &mut Vec<Vec<[f64; 4]>>, slot: usize| {
        laid.slots = laid.slots.max(slot + 1);
        while laid.ink.len() < laid.slots {
            laid.ink.push(Vec::new());
        }
        while obstacles.len() < laid.slots {
            obstacles.push(Vec::new());
        }
    };
    grow(&mut laid, &mut obstacles, 0);
    for f in fixed {
        let (slot, mut rect) = (f.slot, f.rect);
        grow(&mut laid, &mut obstacles, slot);
        if !f.words.is_empty() {
            let size = ((rect[3] - rect[1]) * 0.75).clamp(8.0, 14.0);
            let x = rect[2] + 4.0;
            let y = (rect[1] + rect[3]) / 2.0 + size * 0.35;
            rect[2] = rect[2].max(x + fonts.width(&f.words, size, false));
            laid.ink[slot].push(text(x, y, size, false, false, f.words.clone()));
        }
        obstacles[slot].push(rect);
    }
    for (index, block) in blocks.iter().enumerate() {
        if let Some((slot, rects)) = &block.pinned {
            grow(&mut laid, &mut obstacles, *slot);
            if let (Shape::Section { .. }, Some(at)) = (&block.shape, rects.first()) {
                // A heading moved by hand: drawn where it was put, its width kept.
                let page_w = size_of(*slot).0;
                let width = (at[2] - at[0]).clamp(72.0, (page_w - at[0]).max(72.0));
                let (h, _, ink) = build(&fonts, block, at[0], width);
                // Its drawn box's top where the box was put.
                let top = ink_box(&fonts, &ink).map_or(0.0, |b| b[1]);
                let ink: Vec<Ink> = shifted(ink, at[1] - top).collect();
                let at = [at[0], at[1] - top, at[2], at[3]];
                laid.boxes[index] = ink_box(&fonts, &ink).map(|b| (*slot, b));
                obstacles[*slot].push([
                    at[0] - 4.0,
                    at[1] - 4.0,
                    at[0] + width + 4.0,
                    at[1] + h + 4.0,
                ]);
                laid.ink[*slot].extend(ink);
                laid.placed[index] = (*slot, Vec::new());
                continue;
            }
            let (keep, ink) = pinned_ink(&fonts, block, rects, size_of(*slot).0);
            laid.boxes[index] = ink_box(&fonts, &ink).map(|b| (*slot, b));
            obstacles[*slot].extend(keep.map(|k| [k[0] - 4.0, k[1] - 4.0, k[2] + 4.0, k[3] + 4.0]));
            laid.ink[*slot].extend(ink);
            laid.placed[index] = (*slot, rects.clone());
        }
    }
    let mut slot = 0;
    let mut y = MARGIN;
    // Whether anything flowed onto the slot yet (a block taller than a page
    // still goes on a fresh one).
    let mut fresh = true;
    if !title.trim().is_empty() || !desc.trim().is_empty() {
        let (w, _) = size_of(0);
        let x0 = title_at.map_or(MARGIN, |at| at[0]);
        let width = (w - MARGIN - x0).max(120.0);
        // Laid out from 0, then put at the first top clear of fields placed by hand.
        let mut ink = Vec::new();
        let mut h = 0.0;
        for line in fonts.wrap(title, TITLE_SIZE, true, width) {
            ink.push(text(
                x0,
                baseline(h, TITLE_SIZE),
                TITLE_SIZE,
                true,
                false,
                line,
            ));
            h += lead(TITLE_SIZE);
        }
        for line in fonts.wrap(desc, DESC_SIZE, false, width) {
            ink.push(text(
                x0,
                baseline(h, DESC_SIZE),
                DESC_SIZE,
                false,
                true,
                line,
            ));
            h += lead(DESC_SIZE);
        }
        h += 4.0;
        ink.push(Ink::Rule {
            x0,
            x1: x0 + width,
            y: h,
        });
        match title_at {
            Some(at) => {
                // Where it was put; the flow keeps clear of it.
                let ink: Vec<Ink> = shifted(ink, at[1]).collect();
                laid.title_box = ink_box(&fonts, &ink);
                obstacles[0].push([x0 - 4.0, at[1] - 4.0, x0 + width + 4.0, at[1] + h + 5.0]);
                laid.ink[0].extend(ink);
            }
            None => {
                let top = clear(&obstacles[0], y, h + 1.0, x0, x0 + width);
                let ink: Vec<Ink> = shifted(ink, top).collect();
                laid.title_box = ink_box(&fonts, &ink);
                laid.ink[0].extend(ink);
                y = top + h + GAP;
                fresh = false;
            }
        }
    }
    for (index, block) in blocks.iter().enumerate() {
        if block.pinned.is_some() {
            continue;
        }
        if matches!(block.shape, Shape::Section { new_page: true }) && !fresh {
            slot += 1;
            y = MARGIN;
            fresh = true;
        }
        loop {
            grow(&mut laid, &mut obstacles, slot);
            let (w, h) = size_of(slot);
            let width = (w - 2.0 * MARGIN).max(72.0);
            let (height, rects, ink) = build(&fonts, block, MARGIN, width);
            let top = clear(&obstacles[slot], y, height, MARGIN, MARGIN + width);
            if top + height <= h - MARGIN || (fresh && top <= MARGIN) {
                let rects = rects
                    .into_iter()
                    .map(|r| [r[0], r[1] + top, r[2], r[3] + top])
                    .collect();
                let ink: Vec<Ink> = shifted(ink, top).collect();
                laid.boxes[index] = ink_box(&fonts, &ink).map(|b| (slot, b));
                laid.ink[slot].extend(ink);
                laid.placed[index] = (slot, rects);
                y = top + height + GAP;
                fresh = false;
                break;
            }
            slot += 1;
            y = MARGIN;
            fresh = true;
        }
    }
    laid
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(label: &str) -> Block {
        Block {
            label: label.into(),
            help: String::new(),
            shape: Shape::Field { w: 300.0, h: 22.0 },
            pinned: None,
        }
    }

    #[test]
    fn blocks_flow_down_and_onto_a_new_page() {
        let blocks: Vec<Block> = (0..20).map(|i| field(&format!("Question {i}"))).collect();
        let laid = lay_out(&[(612.0, 792.0)], "", "", None, &blocks, &[]);
        // Label 13 + 4 + 22 = 39, then a 16 gap: 55 a block; 684 fit 12 of them.
        assert_eq!(laid.slots, 2);
        assert_eq!(laid.placed[0], (0, vec![[54.0, 71.0, 354.0, 93.0]]));
        assert_eq!(laid.placed[12].0, 1);
        assert_eq!(laid.placed[12].1[0][1], 71.0);
        assert!(laid.placed.iter().all(|(_, r)| r[0][3] <= 792.0 - MARGIN));
    }

    #[test]
    fn short_choices_go_across_and_long_ones_down() {
        let yes_no = Block {
            label: "Do you drive?".into(),
            help: String::new(),
            shape: Shape::boxes(vec!["Yes".into(), "No".into()]),
            pinned: None,
        };
        let laid = lay_out(&[(612.0, 792.0)], "", "", None, &[yes_no], &[]);
        let rects = &laid.placed[0].1;
        assert_eq!(rects[0][1], rects[1][1]);
        assert!(rects[1][0] > rects[0][2]);
        let long = Block {
            label: "Which?".into(),
            help: String::new(),
            shape: Shape::boxes(vec!["A very long answer".into(), "Another long one".into()]),
            pinned: None,
        };
        let laid = lay_out(&[(612.0, 792.0)], "", "", None, &[long], &[]);
        let rects = &laid.placed[0].1;
        assert_eq!(rects[0][0], rects[1][0]);
        assert_eq!(rects[1][1] - rects[0][1], ROW);
    }

    #[test]
    fn the_flow_goes_around_a_pinned_question_and_other_fields() {
        let mut pinned = field("Pinned");
        pinned.pinned = Some((0, vec![[54.0, 60.0, 354.0, 82.0]]));
        let laid = lay_out(
            &[(612.0, 792.0)],
            "",
            "",
            None,
            &[pinned, field("Next")],
            &[Fixed {
                slot: 0,
                rect: [54.0, 120.0, 200.0, 140.0],
                words: String::new(),
            }],
        );
        assert_eq!(laid.placed[0].1, vec![[54.0, 60.0, 354.0, 82.0]]);
        // Below the pinned field and below the other field.
        let next = laid.placed[1].1[0];
        assert!(next[1] > 140.0, "{next:?}");
        // The pinned label sits above its field.
        let label = laid.ink[0].iter().find_map(|i| match i {
            Ink::Text { text, y, .. } if text == "Pinned" => Some(*y),
            _ => None,
        });
        assert_eq!(label, Some(51.5));
    }

    #[test]
    fn a_title_keeps_clear_of_a_field_placed_by_hand() {
        let mut pinned = field("Placed");
        pinned.pinned = Some((0, vec![[54.0, 60.0, 66.0, 72.0]]));
        let laid = lay_out(&[(612.0, 792.0)], "Sign-up", "", None, &[pinned], &[]);
        let title = laid.ink[0].iter().find_map(|i| match i {
            Ink::Text { text, y, .. } if text == "Sign-up" => Some(*y),
            _ => None,
        });
        assert!(title.is_some_and(|y| y - TITLE_SIZE > 72.0), "{title:?}");
    }

    #[test]
    fn a_title_comes_first_and_long_words_wrap() {
        let laid = lay_out(
            &[(612.0, 792.0)],
            "Sign-up",
            "Fill in every part.",
            None,
            &[field("Name")],
            &[],
        );
        let texts: Vec<&str> = laid.ink[0]
            .iter()
            .filter_map(|i| match i {
                Ink::Text { text, .. } => Some(text.as_str()),
                Ink::Rule { .. } | Ink::Star { .. } => None,
            })
            .collect();
        assert_eq!(texts, ["Sign-up", "Fill in every part.", "Name"]);
        let fonts = Fonts::default();
        let lines = fonts.wrap(&"x".repeat(200), 10.0, false, 100.0);
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|l| fonts.width(l, 10.0, false) <= 100.0));
    }
}

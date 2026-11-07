// SPDX-License-Identifier: MIT
//! The question form's page layout: pure geometry in page space (points, y
//! down from the page top) and the label ink each form page draws.
//!
//! Questions flow down one column inside a 54 pt margin, each block (its
//! label, help line and fields) kept whole on one page; a block that does
//! not fit starts the next form page. A question whose field was moved by
//! hand (pinned) keeps its place, its label just above it, and the flow
//! goes around it, as around any other field on the page.

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
/// A choice button or checkbox.
pub const BOX: f64 = 12.0;
/// One row of choices down the page.
const ROW: f64 = 18.0;

/// What a question puts under its label.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// One field box (text, drop-down, signature), at most as wide as the column.
    Field { w: f64, h: f64 },
    /// One small box per choice with its words beside it (choice buttons or checkboxes).
    Boxes(Vec<String>),
    /// One checkbox with the question itself beside it ("I agree").
    Agree,
}

/// One question to lay out.
#[derive(Clone, Debug)]
pub struct Block {
    /// The label as drawn (` *` already added when required).
    pub label: String,
    pub help: String,
    pub shape: Shape,
    /// A hand-placed question: its form page slot and its widgets' rects.
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
}

/// The layout: how many form pages it needs, each block's page slot and
/// widget rects (in order), and the ink of each slot.
#[derive(Clone, Debug, Default)]
pub struct Laid {
    pub slots: usize,
    pub placed: Vec<(usize, Vec<[f64; 4]>)>,
    pub ink: Vec<Vec<Ink>>,
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

/// Whether the choices fit across one row: at most five short ones.
fn across(fonts: &Fonts, choices: &[String], width: f64) -> bool {
    let total: f64 = choices
        .iter()
        .map(|c| BOX + 4.0 + fonts.width(c, CHOICE_SIZE, false) + 18.0)
        .sum();
    choices.len() <= 5 && choices.iter().all(|c| c.chars().count() <= 12) && total <= width
}

/// A block laid out from (x0, 0): its height, widget rects and ink.
fn build(fonts: &Fonts, block: &Block, x0: f64, width: f64) -> (f64, Vec<[f64; 4]>, Vec<Ink>) {
    let mut ink = Vec::new();
    let mut rects = Vec::new();
    let mut y = 0.0;
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
        Shape::Agree => {}
        Shape::Field { w, h } => {
            y += UNDER_LABEL;
            rects.push([x0, y, x0 + w.min(width), y + h]);
            y += h;
        }
        Shape::Boxes(choices) => {
            y += UNDER_LABEL;
            if across(fonts, choices, width) {
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
            if let Shape::Boxes(choices) = shape {
                for (rect, choice) in rects.iter().zip(choices) {
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
            }
            let width = (page_w - MARGIN - u[0]).max(120.0);
            let lines = fonts.wrap(&block.label, LABEL_SIZE, true, width);
            let mut base = u[1] - 5.0 - lead(LABEL_SIZE) * (lines.len().saturating_sub(1)) as f64;
            keep[1] = keep[1].min(base - LABEL_SIZE);
            for line in lines {
                let w = fonts.width(&line, LABEL_SIZE, true);
                keep[2] = keep[2].max(u[0] + w);
                ink.push(text(u[0], base, LABEL_SIZE, true, false, line));
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
    })
}

/// Lays out the form: `sizes` are the form pages' (width, height) in order
/// (a page added after them takes the last one's size), `fixed` the rects
/// of other fields on them (slot, rect) the flow keeps clear of.
pub fn lay_out(
    sizes: &[(f64, f64)],
    title: &str,
    desc: &str,
    blocks: &[Block],
    fixed: &[(usize, [f64; 4])],
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
    for &(slot, rect) in fixed {
        grow(&mut laid, &mut obstacles, slot);
        obstacles[slot].push(rect);
    }
    for (index, block) in blocks.iter().enumerate() {
        if let Some((slot, rects)) = &block.pinned {
            grow(&mut laid, &mut obstacles, *slot);
            let (keep, ink) = pinned_ink(&fonts, block, rects, size_of(*slot).0);
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
        let width = w - 2.0 * MARGIN;
        for line in fonts.wrap(title, TITLE_SIZE, true, width) {
            laid.ink[0].push(text(
                MARGIN,
                baseline(y, TITLE_SIZE),
                TITLE_SIZE,
                true,
                false,
                line,
            ));
            y += lead(TITLE_SIZE);
        }
        for line in fonts.wrap(desc, DESC_SIZE, false, width) {
            laid.ink[0].push(text(
                MARGIN,
                baseline(y, DESC_SIZE),
                DESC_SIZE,
                false,
                true,
                line,
            ));
            y += lead(DESC_SIZE);
        }
        y += 4.0;
        laid.ink[0].push(Ink::Rule {
            x0: MARGIN,
            x1: w - MARGIN,
            y,
        });
        y += GAP;
        fresh = false;
    }
    for (index, block) in blocks.iter().enumerate() {
        if block.pinned.is_some() {
            continue;
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
                laid.ink[slot].extend(shifted(ink, top));
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
        let laid = lay_out(&[(612.0, 792.0)], "", "", &blocks, &[]);
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
            shape: Shape::Boxes(vec!["Yes".into(), "No".into()]),
            pinned: None,
        };
        let laid = lay_out(&[(612.0, 792.0)], "", "", &[yes_no], &[]);
        let rects = &laid.placed[0].1;
        assert_eq!(rects[0][1], rects[1][1]);
        assert!(rects[1][0] > rects[0][2]);
        let long = Block {
            label: "Which?".into(),
            help: String::new(),
            shape: Shape::Boxes(vec!["A very long answer".into(), "Another long one".into()]),
            pinned: None,
        };
        let laid = lay_out(&[(612.0, 792.0)], "", "", &[long], &[]);
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
            &[pinned, field("Next")],
            &[(0, [54.0, 120.0, 200.0, 140.0])],
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
        assert_eq!(label, Some(55.0));
    }

    #[test]
    fn a_title_comes_first_and_long_words_wrap() {
        let laid = lay_out(
            &[(612.0, 792.0)],
            "Sign-up",
            "Fill in every part.",
            &[field("Name")],
            &[],
        );
        let texts: Vec<&str> = laid.ink[0]
            .iter()
            .filter_map(|i| match i {
                Ink::Text { text, .. } => Some(text.as_str()),
                Ink::Rule { .. } => None,
            })
            .collect();
        assert_eq!(texts, ["Sign-up", "Fill in every part.", "Name"]);
        let fonts = Fonts::default();
        let lines = fonts.wrap(&"x".repeat(200), 10.0, false, 100.0);
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|l| fonts.width(l, 10.0, false) <= 100.0));
    }
}

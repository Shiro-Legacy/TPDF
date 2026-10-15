// SPDX-License-Identifier: AGPL-3.0-or-later
//! Paragraphs (content-edit.ts `editFindParagraphs`, `editPlanParagraph`):
//! consecutive shows of one text object whose operators between compose to
//! exactly one leading down, rewritten as one span that wraps the new text
//! greedily and restores the line matrix and leading exactly.
use serde_json::{Value, json};

use super::font::Codec;
use super::geometry::{edit_number, invert, number_value, round2, writable};
use super::plan::{Measure, collapse_spaces, literal, name_source};
use super::scan::Record;
use super::{MAX_EDIT_TEXT_CHARS, edit_error, js_len};
use crate::app::AppError;
use crate::app::request::{self, js_number};

/// content-edit.ts `MAX_PARAGRAPH_LINES`.
pub const MAX_PARAGRAPH_LINES: usize = 200;

/// One paragraph: the reply item and the proof its rewrite depends on.
#[derive(Clone, Debug)]
pub struct Paragraph {
    pub id: String,
    /// Indices of its line records in the scan.
    pub records: Vec<usize>,
    /// The text-space leading and the translation the original span leaves.
    pub leading_text: f64,
    pub drift: [f64; 2],
    pub bounds: [f64; 4],
    pub left: f64,
    pub width: f64,
    pub leading: f64,
    pub font_size: f64,
    pub kerned: bool,
    pub tagged: bool,
    pub target_ids: Vec<String>,
}

impl Paragraph {
    pub fn item(&self) -> Value {
        json!({
            "paragraphId": self.id, "targetIds": self.target_ids, "bounds": self.bounds,
            "left": self.left, "width": self.width, "leading": self.leading,
            "fontSize": self.font_size, "lines": self.records.len(), "kerned": self.kerned,
            "tagged": self.tagged,
        })
    }
}

/// Can this run be a paragraph line at all?
fn paragraph_line(record: &Record) -> bool {
    let Some(linear) = record.text_linear else {
        return false;
    };
    record.is_text()
        && record.candidate
        && record.bounds.is_some()
        && record.text.as_deref().is_some_and(|t| !t.is_empty())
        && record.first_on_line
        && !record.tail_follows
        && record.cid_write.is_none()
        && !record.font_type0
        && record.actual_text.is_none()
        && record.struct_actual_text.is_none()
        && record.tf_name.as_deref().is_some_and(|n| !n.is_empty())
        && record.tf_size.is_some_and(|s| s != 0.0 && !s.is_nan())
        && record.leading.is_some()
        && linear[1].abs() < 1e-6
        && linear[2].abs() < 1e-6
        && linear[0] > 0.0
        && linear[3] < 0.0
        && (record.cow.as_ref().is_some_and(|c| c.form.is_some()) || record.element.is_some())
}

fn same_line_state(left: &Record, right: &Record) -> bool {
    let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
    let (Some(a), Some(b)) = (left.text_linear, right.text_linear) else {
        return false;
    };
    left.path == right.path
        && left.tf_name == right.tf_name
        && close(left.tf_size.unwrap_or(0.0), right.tf_size.unwrap_or(0.0))
        && close(left.character_spacing, right.character_spacing)
        && close(left.word_spacing, right.word_spacing)
        && close(left.horizontal_scale, right.horizontal_scale)
        && left.cow.is_some() == right.cow.is_some()
        && left.codec == right.codec
        && a.iter().zip(b).all(|(x, y)| close(*x, y))
}

/// content-edit.ts `editFindParagraphs`; marks each line's record.
pub fn find(records: &mut [Record]) -> Vec<Paragraph> {
    let mut found = Vec::new();
    let mut chain: Vec<usize> = Vec::new();
    let mut leading = 0.0;
    let flush =
        |chain: &mut Vec<usize>, leading: f64, records: &[Record], found: &mut Vec<Paragraph>| {
            let lines = std::mem::take(chain);
            let joined: usize = lines
                .iter()
                .map(|&i| records[i].text.as_deref().map_or(0, js_len) + 1)
                .sum();
            if lines.len() < 2 || lines.len() > MAX_PARAGRAPH_LINES || joined > MAX_EDIT_TEXT_CHARS
            {
                return;
            }
            let first = &records[lines[0]];
            let linear = first.text_linear.unwrap_or([1.0, 0.0, 0.0, -1.0]);
            let scale = linear[2].hypot(linear[3]);
            let boxes: Vec<[f64; 4]> = lines.iter().filter_map(|&i| records[i].bounds).collect();
            let left = boxes.iter().map(|b| b[0]).fold(f64::INFINITY, f64::min);
            let bounds = [
                left,
                boxes.iter().map(|b| b[1]).fold(f64::INFINITY, f64::min),
                boxes.iter().map(|b| b[2]).fold(f64::NEG_INFINITY, f64::max),
                boxes.iter().map(|b| b[3]).fold(f64::NEG_INFINITY, f64::max),
            ];
            let drift = lines[1..].iter().fold([0.0, 0.0], |total, &i| {
                let between = records[i].between.unwrap_or([0.0, 0.0]);
                [total[0] + between[0], total[1] + between[1]]
            });
            found.push(Paragraph {
                id: format!("para.{}", first.target_id),
                target_ids: lines
                    .iter()
                    .map(|&i| records[i].target_id.clone())
                    .collect(),
                leading_text: leading,
                drift,
                bounds: bounds.map(round2),
                left: round2(left),
                width: round2(bounds[2] - left),
                leading: round2(leading * scale),
                font_size: round2(first.tf_size.unwrap_or(0.0).abs() * scale),
                kerned: lines.iter().any(|&i| records[i].kerned),
                tagged: lines.iter().any(|&i| records[i].mcid.is_some()),
                records: lines,
            });
        };
    for index in 0..records.len() {
        if !records[index].is_text() {
            continue;
        }
        if !paragraph_line(&records[index]) {
            flush(&mut chain, leading, records, &mut found);
            continue;
        }
        let record = &records[index];
        let linked = match (chain.last(), record.between) {
            (Some(&previous), Some(step)) => {
                let previous = &records[previous];
                record.prev_show.as_deref() == Some(previous.target_id.as_str())
                    && step[0].abs() < 1e-6
                    && step[1] < 0.0
                    && same_line_state(previous, record)
                    && (chain.len() == 1 || (-step[1] - leading).abs() < 1e-4)
            }
            _ => false,
        };
        if linked {
            if chain.len() == 1 {
                leading = -record.between.map_or(0.0, |s| s[1]);
            }
            chain.push(index);
            continue;
        }
        flush(&mut chain, leading, records, &mut found);
        chain.push(index);
    }
    flush(&mut chain, leading, records, &mut found);
    for paragraph in &found {
        for &i in &paragraph.records {
            records[i].paragraph_id = Some(paragraph.id.clone());
        }
    }
    found
}

/// One planned paragraph rewrite.
#[derive(Clone, Debug)]
pub struct ParagraphPlan {
    pub lines: Vec<String>,
    pub noop: bool,
    pub element: usize,
    /// The first line's record, the span and its replacement.
    pub record: usize,
    pub start: usize,
    pub end: usize,
    pub replacement: Vec<u8>,
}

fn fail(detail: &str, message: impl AsRef<str>) -> AppError {
    edit_error(detail, format!("edit-paragraph: {}", message.as_ref()))
}

fn number_in(
    value: Option<&Value>,
    what: &str,
    low: f64,
    high: f64,
) -> Result<Option<f64>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };
    request::as_number(value)
        .filter(|v| (low..=high).contains(v))
        .map(Some)
        .ok_or_else(|| {
            fail(
                "invalid",
                format!("{what} is a number {}..{}", js_number(low), js_number(high)),
            )
        })
}

/// content-edit.ts `editPlanParagraph`: wrap the text greedily, prove the
/// box clears every obstacle and the page, and build the one span rewrite.
pub fn plan(
    measure: &Measure<'_>,
    records: &[Record],
    paragraph: &Paragraph,
    request: &Value,
) -> Result<ParagraphPlan, AppError> {
    let lines_of = |i: &usize| &records[*i];
    let (Some(first), Some(last)) = (
        paragraph.records.first().map(lines_of),
        paragraph.records.last().map(lines_of),
    ) else {
        return Err(fail("stale-target", "the paragraph has no lines"));
    };
    let linear = first.text_linear.unwrap_or([1.0, 0.0, 0.0, -1.0]);
    let scale = linear[2].hypot(linear[3]);
    let text = request
        .get("text")
        .and_then(Value::as_str)
        .filter(|t| js_len(t) <= MAX_EDIT_TEXT_CHARS)
        .ok_or_else(|| {
            fail(
                "invalid",
                format!("text is a string of at most {MAX_EDIT_TEXT_CHARS} characters"),
            )
        })?;
    let width = number_in(request.get("width"), "width", 1.0, 14400.0)?.unwrap_or(paragraph.width);
    let size = number_in(request.get("size"), "size", 1.0, 400.0)?;
    let leading_page = number_in(request.get("leading"), "leading", 0.1, 2000.0)?;
    let mut blocks: Vec<String> = text.split('\n').map(collapse_spaces).collect();
    while blocks.len() > 1 && blocks.last().is_some_and(String::is_empty) {
        blocks.pop();
    }
    while blocks.len() > 1 && blocks.first().is_some_and(String::is_empty) {
        blocks.remove(0);
    }
    if !blocks.iter().any(|b| !b.is_empty()) {
        return Err(fail("invalid", "the text is empty"));
    }
    let codec = first.codec.unwrap_or_default();
    let bad: Vec<String> = blocks
        .concat()
        .chars()
        .filter(|&c| c < ' ' || codec.encode(c).is_none())
        .take(20)
        .map(String::from)
        .collect();
    if !bad.is_empty() {
        let message = match codec {
            Codec::WinAnsi => "text is not WinAnsi-encodable",
            Codec::Ascii => "only printable ASCII can be written",
            Codec::Named(_) => "text is not in this font's encoding",
        };
        return Err(fail("unsupported-encoding", message).with_extra(json!({"missing": bad})));
    }
    let old_tf = first.tf_size.unwrap_or(f64::NAN);
    let new_tf = match size {
        None => edit_number(old_tf),
        Some(size) => edit_number(old_tf.signum() * size / scale),
    };
    if number_value(&new_tf) == 0.0 {
        return Err(fail("invalid", "size rounds to zero"));
    }
    let ratio = number_value(&new_tf) / old_tf;
    let leading_text = edit_number(match leading_page {
        Some(page) => page / scale,
        None => paragraph.leading_text * ratio,
    });
    // Advances of every distinct character, measured with the run's font.
    let mut distinct: Vec<char> = Vec::new();
    for c in blocks.concat().chars().chain([' ']) {
        if !distinct.contains(&c) {
            distinct.push(c);
        }
    }
    let unmeasurable = || {
        fail(
            "unmeasurable-width",
            "this paragraph's advance cannot be measured in this build",
        )
    };
    let strings: Vec<Vec<u8>> = distinct
        .iter()
        .map(|&c| codec.encode(c).map(|b| vec![b]).unwrap_or_default())
        .collect();
    let glyphs = measure.glyphs(first, &strings).ok_or_else(unmeasurable)?;
    let x_scale = linear[0].hypot(linear[1]);
    let size_text = number_value(&new_tf).abs();
    let mut advances: Vec<(char, f64, bool)> = Vec::new();
    for (&c, glyph) in distinct.iter().zip(&glyphs) {
        let Some(glyph) = glyph.first() else {
            return Err(unmeasurable());
        };
        let advance = glyph.advance.ok_or_else(unmeasurable)?;
        // A character the font cannot draw counts no width, as engine.ts measured it.
        let advance = if glyph.missing { 0.0 } else { advance };
        advances.push((c, advance / 1000.0 * size_text * x_scale, glyph.missing));
    }
    let glyph = |c: char| {
        advances
            .iter()
            .find(|(d, _, _)| *d == c)
            .map_or(0.0, |a| a.1)
    };
    let th = first.horizontal_scale;
    let measure_line = |line: &str| -> f64 {
        line.chars()
            .map(|c| {
                let spacing =
                    first.character_spacing + if c == ' ' { first.word_spacing } else { 0.0 };
                glyph(c) + spacing * x_scale
            })
            .sum::<f64>()
            * th
    };
    let mut missing: Vec<String> = Vec::new();
    let note_missing = |c: char, missing: &mut Vec<String>| {
        if advances.iter().any(|(d, _, m)| *d == c && *m) && !missing.contains(&c.to_string()) {
            missing.push(c.to_string());
        }
    };
    let mut lines: Vec<String> = Vec::new();
    for block in &blocks {
        let mut line = String::new();
        let words: Vec<&str> = if block.is_empty() {
            Vec::new()
        } else {
            block.split(' ').collect()
        };
        for word in words {
            for c in word.chars() {
                note_missing(c, &mut missing);
            }
            if measure_line(word) > width + 0.01 {
                return Err(fail(
                    "word-too-wide",
                    format!("\"{word}\" is wider than {} pt", js_number(width)),
                )
                .with_extra(json!({"word": word, "width": width})));
            }
            let next = if line.is_empty() {
                word.to_string()
            } else {
                note_missing(' ', &mut missing);
                format!("{line} {word}")
            };
            if !line.is_empty() && measure_line(&next) > width + 0.01 {
                lines.push(std::mem::replace(&mut line, word.to_string()));
            } else {
                line = next;
            }
        }
        lines.push(line);
    }
    if !missing.is_empty() {
        missing.truncate(20);
        return Err(fail(
            "unsupported-encoding",
            "the font has no glyph for some characters",
        )
        .with_extra(json!({"missing": missing})));
    }
    if lines.len() > MAX_PARAGRAPH_LINES {
        return Err(fail(
            "too-large",
            format!("a paragraph holds at most {MAX_PARAGRAPH_LINES} lines"),
        ));
    }
    let (mut tx, mut ty) = ("0".to_string(), "0".to_string());
    if let Some(moved) = request.get("move") {
        let pair = moved.as_object().and_then(|m| {
            Some((
                m.get("dx").and_then(request::as_number)?,
                m.get("dy").and_then(request::as_number)?,
            ))
        });
        let (dx, dy) = pair.ok_or_else(|| fail("invalid", "move is { dx, dy } in page points"))?;
        let inverse = invert(&linear).unwrap_or([1.0, 0.0, 0.0, 1.0]);
        tx = edit_number(dx * inverse[0] + dy * inverse[2]);
        ty = edit_number(dx * inverse[1] + dy * inverse[3]);
    }
    let (x, y) = (number_value(&tx), number_value(&ty));
    let offset = [x * linear[0] + y * linear[2], x * linear[1] + y * linear[3]];
    let unchanged_leading = (number_value(&leading_text) - paragraph.leading_text).abs() < 1e-4;
    let noop = tx == "0"
        && ty == "0"
        && new_tf == edit_number(old_tf)
        && unchanged_leading
        && lines.len() == paragraph.records.len()
        && lines
            .iter()
            .zip(&paragraph.records)
            .all(|(line, &i)| records[i].text.as_deref() == Some(line.as_str()));
    let old_leading_page = paragraph.leading_text * scale;
    let new_leading_page = number_value(&leading_text) * scale;
    let first_box = first.bounds.unwrap_or_default();
    let last_box = last.bounds.unwrap_or_default();
    let count = paragraph.records.len() as f64;
    let box_of = |lines: usize| {
        [
            paragraph.left + offset[0],
            first_box[3] - (first_box[3] - first_box[1]) * ratio.abs() + offset[1],
            paragraph.left + width + offset[0],
            last_box[3] + (lines as f64 - 1.0) * new_leading_page
                - (count - 1.0) * old_leading_page
                + offset[1],
        ]
    };
    let own = paragraph.bounds;
    let hits = |candidate: [f64; 4]| {
        records.iter().enumerate().any(|(index, record)| {
            let other = if record.is_text() {
                record.bounds
            } else {
                record.placement.as_ref().map(|p| p.view)
            };
            let Some(other) = other else { return false };
            if paragraph.records.contains(&index) {
                return false;
            }
            if other[0] < own[2] && own[0] < other[2] && other[1] < own[3] && own[1] < other[3] {
                return false;
            }
            other[0] < candidate[2] + 1.0
                && candidate[0] < other[2] + 1.0
                && other[1] < candidate[3] + 1.0
                && candidate[1] < other[3] + 1.0
        })
    };
    if !noop {
        let clip = measure.bounds;
        let target = box_of(lines.len());
        if target[0] < clip[0] - 0.5 || target[2] > clip[2] + 0.5 || target[1] < clip[1] - 0.5 {
            return Err(fail("outside-page", "the paragraph would leave the page")
                .with_extra(json!({"bounds": target.map(round2)})));
        }
        let fits = |n: usize| !hits(box_of(n)) && box_of(n)[3] <= clip[3] + 0.5;
        if !fits(lines.len()) {
            let mut max_lines = lines.len() - 1;
            while max_lines > 0 && !fits(max_lines) {
                max_lines -= 1;
            }
            return Err(fail(
                "overflow",
                format!("the paragraph needs {} lines; {max_lines} fit", lines.len()),
            )
            .with_extra(json!({"lines": lines.len(), "maxLines": max_lines})));
        }
    }
    let font = name_source(first.tf_name.as_deref().unwrap_or_default());
    let sized = new_tf != edit_number(old_tf);
    let neg_leading = edit_number(-number_value(&leading_text));
    let mut out: Vec<u8> = Vec::new();
    if sized {
        out.extend_from_slice(format!("{font} {new_tf} Tf ").as_bytes());
    }
    if tx != "0" || ty != "0" {
        out.extend_from_slice(format!("{tx} {ty} Td ").as_bytes());
    }
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            out.extend_from_slice(format!("0 {neg_leading} Td ").as_bytes());
        }
        if line.is_empty() {
            continue;
        }
        let bytes: Vec<u8> = line.chars().filter_map(|c| codec.encode(c)).collect();
        out.extend_from_slice(&literal(&bytes));
        out.extend_from_slice(b" Tj ");
    }
    let ex = edit_number(paragraph.drift[0] - x);
    let ey = edit_number(
        paragraph.drift[1] - y + (lines.len() as f64 - 1.0) * number_value(&leading_text),
    );
    if ex != "0" || ey != "0" {
        out.extend_from_slice(format!("{ex} {ey} Td").as_bytes());
    }
    if sized {
        out.extend_from_slice(format!(" {font} {} Tf", edit_number(old_tf)).as_bytes());
    }
    let (first_leading, last_leading) = (
        edit_number(first.leading.unwrap_or(0.0)),
        edit_number(last.leading.unwrap_or(0.0)),
    );
    // Every number written stays plain (engine.ts wrote exponents past 1e21).
    let mut written = vec![tx.as_str(), ty.as_str(), ex.as_str(), ey.as_str()];
    let old_text = edit_number(old_tf);
    if sized {
        written.extend([new_tf.as_str(), old_text.as_str()]);
    }
    if lines.len() > 1 {
        written.push(neg_leading.as_str());
    }
    if first_leading != last_leading {
        written.push(last_leading.as_str());
    }
    if !noop && !written.iter().all(|text| writable(number_value(text))) {
        return Err(fail(
            "not-candidate",
            "this paragraph's numbers would be out of range",
        ));
    }
    if first_leading != last_leading {
        out.extend_from_slice(format!(" {last_leading} TL").as_bytes());
    }
    let element = match &first.cow {
        Some(super::scan::Cow {
            form: Some(form), ..
        }) => Some(form.element),
        _ => first.element,
    };
    let element =
        element.ok_or_else(|| fail("not-candidate", "only top-level page content is editable"))?;
    Ok(ParagraphPlan {
        lines,
        noop,
        element,
        record: paragraph.records[0],
        start: first.operand.start,
        end: last.op.end,
        replacement: out,
    })
}

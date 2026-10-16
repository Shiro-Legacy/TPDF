// SPDX-License-Identifier: AGPL-3.0-or-later
//! Text plans (content-edit.ts `editPlanText`, `editPlanGeometry`,
//! `editPlanWidth`, `editPlanGlyphLine`): every check a text write must pass
//! and the bytes it will splice, decided before any journal operation opens.
//! Widths are measured with the run's own font as the view draws it, in
//! page points: glyph advances, character and word spacing and horizontal
//! scaling all go through the text matrix.
use serde_json::{Value, json};

use super::font::win_ansi_byte;
use super::geometry::{edit_number, invert, number_value, round2, writable};
use super::scan::{Record, move_refusal, text_scale};
use super::{MAX_EDIT_TEXT_CHARS, edit_error, js_len};
use crate::app::AppError;
use crate::app::request;
use crate::view::{GlyphMeasure, Measurer};

/// Where text plans measure: the page as the view draws it, and its bounds
/// (MuPDF's `page.getBounds()`).
pub struct Measure<'v> {
    pub measurer: Option<Measurer<'v>>,
    pub bounds: [f64; 4],
}

impl Measure<'_> {
    /// The glyphs of each string in the record's font, None when the view
    /// cannot draw them code for code.
    pub fn glyphs(&self, record: &Record, strings: &[Vec<u8>]) -> Option<Vec<Vec<GlyphMeasure>>> {
        let measurer = self.measurer.as_ref()?;
        let font = record.tf_name.as_deref()?;
        let measured = measurer.measure(&record.font_path.forms, font, strings);
        measured.into_iter().collect()
    }
}

/// The optional geometry of one `edit-text`: absent (`undefined`) or the
/// request's value.
#[derive(Clone, Copy, Default)]
pub struct Geometry<'r> {
    pub moved: Option<&'r Value>,
    pub size: Option<&'r Value>,
}

impl Geometry<'_> {
    fn any(&self) -> bool {
        self.moved.is_some() || self.size.is_some()
    }
}

/// One validated text replacement, ready to write.
#[derive(Clone, Debug)]
pub struct TextPlan {
    /// The record's index in its scan.
    pub record: usize,
    pub next: String,
    pub element: usize,
    pub operand: Vec<u8>,
    pub tag: Vec<u8>,
    pub old_width: f64,
    pub new_width: f64,
    pub limit: f64,
    pub shift: f64,
    pub prefix: String,
    pub suffix: String,
    /// A glyph line whose bytes change (text or rounded geometry).
    pub rewrite: bool,
}

struct Placed {
    prefix: String,
    suffix: String,
    ratio: f64,
    offset: [f64; 2],
}

/// content-edit.ts `editNameSource`: a name as PDF source, `#xx`-escaping
/// every byte that is not regular (raw bytes; engine.ts escaped the UTF-8
/// of the Latin-1 name).
pub fn name_source(name: &[u8]) -> String {
    let mut out = String::from("/");
    for &b in name {
        let regular = b > 0x20
            && b < 0x7f
            && !matches!(
                b,
                b'#' | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
            );
        if regular {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("#{b:02x}"));
        }
    }
    out
}

/// A literal string operand of `bytes`, escaping `(`, `)` and `\`.
pub fn literal(bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![b'('];
    for &b in bytes {
        if matches!(b, b'(' | b')' | b'\\') {
            out.push(b'\\');
        }
        out.push(b);
    }
    out.push(b')');
    out
}

/// content-edit.ts `editActualTextBytes`: printable ASCII as a literal,
/// anything else UTF-16BE hex with a byte order mark.
pub fn actual_text_bytes(text: &str) -> Vec<u8> {
    if text.chars().all(|c| (' '..='~').contains(&c)) {
        return literal(text.as_bytes());
    }
    let mut hex = String::from("<feff");
    for unit in text.encode_utf16() {
        hex.push_str(&format!("{unit:04x}"));
    }
    hex.push('>');
    hex.into_bytes()
}

/// content-edit.ts `editHexOperand`.
pub fn hex_operand(codes: &[u32], bytes_per_code: usize) -> Vec<u8> {
    let mut out = String::from("<");
    for code in codes {
        if bytes_per_code == 1 {
            out.push_str(&format!("{code:02x}"));
        } else {
            out.push_str(&format!("{code:04x}"));
        }
    }
    out.push('>');
    out.into_bytes()
}

fn finite_move(value: &Value) -> Option<(f64, f64)> {
    let object = value.as_object()?;
    let dx = object.get("dx").and_then(request::as_number)?;
    let dy = object.get("dy").and_then(request::as_number)?;
    Some((dx, dy))
}

fn negate(value: &str) -> String {
    if value == "0" {
        "0".into()
    } else if let Some(rest) = value.strip_prefix('-') {
        rest.into()
    } else {
        format!("-{value}")
    }
}

/// A run whose text matrix is degenerate: no scale, or a size or move whose
/// operands would pass `MAX_WRITTEN` (engine.ts wrote exponents).
fn unproven() -> AppError {
    edit_error(
        "not-candidate",
        "edit-text: this run's text matrix is not proven",
    )
    .with_extra(json!({"reason": "matrix"}))
}

/// content-edit.ts `editPlanGeometry`.
fn plan_geometry(record: &Record, geometry: Geometry<'_>) -> Result<Placed, AppError> {
    let mut placed = Placed {
        prefix: String::new(),
        suffix: String::new(),
        ratio: 1.0,
        offset: [0.0, 0.0],
    };
    if !geometry.any() {
        return Ok(placed);
    }
    if let Some(size) = geometry.size {
        let size = request::as_number(size)
            .filter(|s| (1.0..=400.0).contains(s))
            .ok_or_else(|| {
                edit_error(
                    "invalid",
                    "edit-text: size is a number of page points, 1..400",
                )
            })?;
        let scale = text_scale(record).ok_or_else(unproven)?;
        let old = record.tf_size.unwrap_or(f64::NAN);
        let next = edit_number(old.signum() * size / scale);
        if !writable(number_value(&next)) {
            return Err(unproven());
        }
        if number_value(&next) == 0.0 {
            return Err(edit_error("invalid", "edit-text: size rounds to zero"));
        }
        if next != edit_number(old) {
            placed.ratio = number_value(&next) / old;
            let font = name_source(record.tf_name.as_deref().unwrap_or_default());
            placed.prefix = format!("{font} {next} Tf ");
            placed.suffix = format!(" {font} {} Tf", edit_number(old));
        }
    }
    if let Some(moved) = geometry.moved {
        let (dx, dy) = finite_move(moved)
            .ok_or_else(|| edit_error("invalid", "edit-text: move is { dx, dy } in page points"))?;
        if let Some((detail, reason)) = move_refusal(record) {
            return Err(edit_error(
                detail,
                format!("edit-text: this run cannot be moved ({reason})"),
            )
            .with_extra(json!({"reason": reason})));
        }
        let linear = record.text_linear.unwrap_or([1.0, 0.0, 0.0, 1.0]);
        let inverse = invert(&linear).unwrap_or([1.0, 0.0, 0.0, 1.0]);
        let tx = edit_number(dx * inverse[0] + dy * inverse[2]);
        let ty = edit_number(dx * inverse[1] + dy * inverse[3]);
        if !writable(number_value(&tx)) || !writable(number_value(&ty)) {
            return Err(unproven());
        }
        if tx != "0" || ty != "0" {
            placed.prefix = format!("{tx} {ty} Td {}", placed.prefix);
            placed.suffix = format!("{} {} {} Td", placed.suffix, negate(&tx), negate(&ty));
            let (x, y) = (number_value(&tx), number_value(&ty));
            placed.offset = [x * linear[0] + y * linear[2], x * linear[1] + y * linear[3]];
        }
    }
    Ok(placed)
}

/// A replacement's old and new widths in page points, and the characters
/// its font cannot draw.
struct Measured {
    old: f64,
    new: f64,
    missing: Vec<char>,
}

/// The run's horizontal text-space-to-page scale.
fn x_scale(record: &Record) -> Option<f64> {
    let linear = record.text_linear?;
    Some(linear[0].hypot(linear[1]))
}

/// The width of a run of codes in page points.
fn run_width(
    record: &Record,
    advance: f64,
    codes: usize,
    spaces: usize,
    adjustment: f64,
) -> Option<f64> {
    let size = record.tf_size?.abs();
    let text = size * (advance - adjustment) / 1000.0
        + record.character_spacing * codes as f64
        + record.word_spacing * spaces as f64;
    Some(text * record.horizontal_scale * x_scale(record)?)
}

/// content-edit.ts `editMeasureCid`, through the text matrix.
fn measure_cid(record: &Record, next: &str, ratio: f64) -> Option<Measured> {
    let write = record.cid_write.as_ref()?;
    record.font_size.filter(|s| s.is_finite())?;
    let mut missing = Vec::new();
    let mut codes = Vec::new();
    for c in next.chars() {
        match write.code(c) {
            Some(code) => codes.push(code),
            None => missing.push(c),
        }
    }
    let one_byte = write.bytes_per_code == 1;
    let advance = |codes: &[u32]| codes.iter().map(|&c| write.width(c)).sum::<f64>();
    let spaces = |codes: &[u32]| {
        if one_byte {
            codes.iter().filter(|&&c| c == 32).count()
        } else {
            0
        }
    };
    let old_codes = super::font::codes_of(
        record.payload.as_deref().unwrap_or_default(),
        write.bytes_per_code,
    );
    let old = run_width(
        record,
        advance(&old_codes),
        old_codes.len(),
        spaces(&old_codes),
        record.tj_adjustment,
    )?;
    let new = if missing.is_empty() {
        let sized = Record {
            tf_size: record.tf_size.map(|s| s * ratio),
            ..record.clone()
        };
        run_width(&sized, advance(&codes), codes.len(), spaces(&codes), 0.0)?
    } else {
        0.0
    };
    Some(Measured { old, new, missing })
}

/// content-edit.ts `editMeasureRun`: a simple font's old and new advances
/// as the view draws them.
fn measure_run(measure: &Measure<'_>, record: &Record, next: &str, ratio: f64) -> Option<Measured> {
    let codec = record.codec?;
    let old_codes = record.payload.clone()?;
    let new_codes: Vec<u8> = next.chars().filter_map(|c| codec.encode(c)).collect();
    let measured = measure.glyphs(record, &[old_codes.clone(), new_codes.clone()])?;
    let [old, new] = measured.as_slice() else {
        return None;
    };
    let old_advance = old.iter().map(|g| g.advance).sum::<Option<f64>>()?;
    let mut missing = Vec::new();
    let mut new_advance = 0.0;
    for (c, glyph) in next.chars().zip(new) {
        if glyph.missing {
            missing.push(c);
        } else {
            new_advance += glyph.advance?;
        }
    }
    let old_text = record.text.as_deref().unwrap_or_default();
    let old = run_width(
        record,
        old_advance,
        old_text.chars().count(),
        old_text.chars().filter(|&c| c == ' ').count(),
        record.tj_adjustment,
    )?;
    let sized = Record {
        tf_size: record.tf_size.map(|s| s * ratio),
        ..record.clone()
    };
    let new = run_width(
        &sized,
        new_advance,
        next.chars().count(),
        next.chars().filter(|&c| c == ' ').count(),
        0.0,
    )?;
    Some(Measured { old, new, missing })
}

fn missing_extra(missing: &[char]) -> Value {
    json!({"missing": missing.iter().take(20).map(|c| c.to_string()).collect::<Vec<_>>()})
}

/// content-edit.ts `editPlanText`: the one text validator edit-text, the
/// replace preview and apply share.
pub fn plan_text(
    measure: &Measure<'_>,
    index: usize,
    record: &Record,
    next: &str,
    geometry: Geometry<'_>,
) -> Result<TextPlan, AppError> {
    if record.glyph_line.is_some() {
        return plan_glyph_line(measure, index, record, next, geometry);
    }
    let placed = plan_geometry(record, geometry)?;
    if js_len(next) > MAX_EDIT_TEXT_CHARS {
        return Err(edit_error(
            "too-large",
            format!("edit-text: text must be at most {MAX_EDIT_TEXT_CHARS} characters"),
        ));
    }
    let cid = record.cid_write.is_some();
    if !cid && record.font_type0 {
        return Err(edit_error(
            "not-candidate",
            "edit-text: this run's font has no proven repertoire",
        )
        .with_extra(json!({"reason": "unsupported-encoding"})));
    }
    let codec = record.codec.unwrap_or_default();
    if !cid {
        let bad: Vec<char> = next
            .chars()
            .filter(|&c| c < ' ' || codec.encode(c).is_none())
            .collect();
        if !bad.is_empty() {
            let message = match codec {
                super::font::Codec::WinAnsi => "edit-text: text is not WinAnsi-encodable",
                super::font::Codec::Ascii => "edit-text: only printable ASCII can be written",
                super::font::Codec::Named(_) => "edit-text: text is not in this font's encoding",
            };
            return Err(edit_error("unsupported-encoding", message).with_extra(missing_extra(&bad)));
        }
    }
    let measured = if cid {
        measure_cid(record, next, placed.ratio)
    } else {
        measure_run(measure, record, next, placed.ratio)
    };
    let clip = (!placed.prefix.is_empty()).then_some(measure.bounds);
    let measured = measured.ok_or_else(|| {
        edit_error(
            "unmeasurable-width",
            "edit-text: this run's advance cannot be measured in this build",
        )
    })?;
    if !measured.missing.is_empty() {
        return Err(edit_error(
            "unsupported-encoding",
            "edit-text: the run's font has no glyph for some characters",
        )
        .with_extra(missing_extra(&measured.missing)));
    }
    let width = plan_width(
        record,
        measured.old,
        measured.new,
        !placed.prefix.is_empty(),
        placed.offset,
        placed.ratio,
        clip,
    )?;
    let operand = match (&record.cid_write, codec) {
        (Some(write), _) => {
            let codes: Vec<u32> = next.chars().filter_map(|c| write.code(c)).collect();
            hex_operand(&codes, write.bytes_per_code)
        }
        (None, codec) => {
            let bytes: Vec<u8> = next.chars().filter_map(|c| codec.encode(c)).collect();
            literal(&bytes)
        }
    };
    Ok(TextPlan {
        record: index,
        next: next.to_string(),
        element: width.element,
        operand,
        tag: actual_text_bytes(next),
        old_width: width.old,
        new_width: width.new,
        limit: width.limit,
        shift: width.shift,
        prefix: placed.prefix,
        suffix: placed.suffix,
        rewrite: false,
    })
}

/// JavaScript `/\s/`.
pub fn is_js_space(c: char) -> bool {
    matches!(c, '\u{9}'..='\u{D}' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}'
        | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}')
}

/// `text.replace(/\s+/g, " ").trim()`.
pub fn collapse_spaces(text: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in text.chars() {
        if is_js_space(c) {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    super::scan::js_trim(&out).to_string()
}

/// content-edit.ts `editPlanGlyphLine`: one unclipped block per glyph.
fn plan_glyph_line(
    measure: &Measure<'_>,
    index: usize,
    record: &Record,
    raw: &str,
    geometry: Geometry<'_>,
) -> Result<TextPlan, AppError> {
    let Some(line) = record.glyph_line.as_deref() else {
        return Err(edit_error("not-candidate", "edit-text: not a glyph line"));
    };
    if js_len(raw) > MAX_EDIT_TEXT_CHARS {
        return Err(edit_error(
            "too-large",
            format!("edit-text: text must be at most {MAX_EDIT_TEXT_CHARS} characters"),
        ));
    }
    let next = collapse_spaces(raw);
    let mut scale = line.scale;
    let (mut dx, mut dy) = (0.0, 0.0);
    if let Some(size) = geometry.size {
        scale = request::as_number(size)
            .filter(|s| (1.0..=400.0).contains(s))
            .ok_or_else(|| {
                edit_error(
                    "invalid",
                    "edit-text: size is a number of page points, 1..400",
                )
            })?;
    }
    if let Some(moved) = geometry.moved {
        (dx, dy) = finite_move(moved)
            .ok_or_else(|| edit_error("invalid", "edit-text: move is { dx, dy } in page points"))?;
    }
    let Some(write) = record.cid_write.as_deref() else {
        return Err(edit_error("not-candidate", "edit-text: not a glyph line"));
    };
    let missing: Vec<char> = next
        .chars()
        .filter(|&c| c != ' ' && write.code(c).is_none())
        .collect();
    if !missing.is_empty() {
        return Err(edit_error(
            "unsupported-encoding",
            "edit-text: the run's font has no glyph for some characters",
        )
        .with_extra(missing_extra(&missing)));
    }
    let ratio = scale / line.scale;
    let x0 = line.glyphs.first().map_or(0.0, |g| g.1);
    let left = x0 + dx;
    let y = line.y - dy;
    let same = Some(next.as_str()) == record.text.as_deref();
    let mut placed: Vec<(u32, f64, f64)> = Vec::new();
    if same {
        for &(code, x, adv) in &line.glyphs {
            placed.push((code, left + (x - x0) * ratio, adv * ratio));
        }
    } else {
        let mut pen = left;
        let mut gap = false;
        for c in next.chars() {
            if c == ' ' {
                gap = true;
                continue;
            }
            if gap {
                pen += line.space * scale;
            }
            gap = false;
            let code = write.code(c).unwrap_or(0);
            let adv = write.width(code) / 1000.0 * scale;
            placed.push((code, pen, adv));
            pen += adv;
        }
    }
    let geometric = edit_number(scale) != edit_number(line.scale)
        || edit_number(dx) != "0"
        || edit_number(dy) != "0";
    let clip = geometric.then_some(measure.bounds);
    let bounds = record.bounds.unwrap_or([x0, 0.0, x0, 0.0]);
    let old = bounds[2] - bounds[0];
    let new = placed.last().map_or(0.0, |&(_, x, adv)| x + adv - left);
    let width = plan_width(record, old, new, geometric, [dx, dy], ratio, clip)?;
    let font = name_source(record.tf_name.as_deref().unwrap_or_default());
    let colour = if line.colour.is_empty() {
        String::new()
    } else {
        format!("{} ", line.colour)
    };
    let size = edit_number(scale);
    let mut numbers = placed.iter().flat_map(|&(_, x, _)| [x, y]).chain([scale]);
    if !numbers.all(writable) {
        return Err(unproven());
    }
    let blocks: Vec<String> = placed
        .iter()
        .map(|&(code, x, _)| {
            format!(
                "q {colour}q {size} 0 0 {size} {} {} cm BT 1 0 0 1 0 0 Tm {font} 1 Tf <{code:02x}> Tj ET Q Q",
                edit_number(x),
                edit_number(y)
            )
        })
        .collect();
    Ok(TextPlan {
        record: index,
        tag: actual_text_bytes(&next),
        next,
        element: width.element,
        operand: blocks.join("\n").into_bytes(),
        old_width: width.old,
        new_width: width.new,
        limit: width.limit,
        shift: width.shift,
        prefix: String::new(),
        suffix: String::new(),
        rewrite: !same || geometric,
    })
}

struct Width {
    old: f64,
    new: f64,
    limit: f64,
    shift: f64,
    element: usize,
}

/// content-edit.ts `editPlanWidth`: the width, growth, page and element
/// rules every text plan runs.
fn plan_width(
    record: &Record,
    old: f64,
    new: f64,
    geometric: bool,
    offset: [f64; 2],
    ratio: f64,
    clip: Option<[f64; 4]>,
) -> Result<Width, AppError> {
    let clip_right = clip.map(|c| c[2] - offset[0]);
    let old_width = round2(old);
    let new_width = round2(new);
    let delta = new_width - old_width;
    let geometry_limit = if offset[0] != 0.0 || offset[1] != 0.0 {
        clip_right
    } else {
        record.obstacle_limit.or(clip_right)
    };
    let limit = round2(
        geometric
            .then_some(geometry_limit)
            .flatten()
            .or(record.growth_limit)
            .or(record.bounds.map(|b| b[2]))
            .unwrap_or(old_width),
    );
    let extra = || json!({"oldWidth": old_width, "newWidth": new_width, "limit": limit});
    if delta > 0.5 {
        if record.tail_follows && record.left_to_right_tail == Some(false) {
            return Err(edit_error(
                "tail-would-move",
                "edit-text: following text is not left-to-right",
            )
            .with_extra(extra()));
        }
        let right = if record.tail_follows {
            record
                .tail_right
                .or(record.bounds.map(|b| b[2]))
                .unwrap_or(old_width)
                + delta
        } else {
            record.bounds.map_or(0.0, |b| b[0]) + new_width
        };
        if right > limit {
            let detail = if record.tail_follows {
                "tail-would-move"
            } else {
                "overflow"
            };
            return Err(edit_error(
                detail,
                format!(
                    "edit-text: the replacement would cross the {} pt free-space limit",
                    crate::app::request::js_number(limit)
                ),
            )
            .with_extra(extra()));
        }
    }
    let shift = if record.tail_follows {
        round2(delta)
    } else {
        0.0
    };
    if let (true, Some(bounds), Some(clip)) = (geometric, record.bounds, clip) {
        let [x0, y0, _, y1] = checked_bounds(record, bounds);
        let moved = [
            x0 + offset[0],
            y1 - (y1 - y0) * ratio.abs() + offset[1],
            x0 + new_width + offset[0],
            y1 + offset[1],
        ];
        if moved[0] < clip[0] - 0.5
            || moved[1] < clip[1] - 0.5
            || moved[2] > clip[2] + 0.5
            || moved[3] > clip[3] + 0.5
        {
            return Err(edit_error(
                "outside-page",
                "edit-text: the moved text would leave the page",
            )
            .with_extra(json!({"bounds": moved.map(round2)})));
        }
    }
    let element = match &record.cow {
        Some(super::scan::Cow {
            form: Some(form), ..
        }) => Some(form.element),
        _ => record.element,
    };
    let element = element.ok_or_else(|| {
        edit_error(
            "not-candidate",
            "edit-text: only top-level page content is editable",
        )
    })?;
    Ok(Width {
        old: old_width,
        new: new_width,
        limit,
        shift,
        element,
    })
}

/// The box a move or size change keeps on the page: the run's structured
/// text bounds, unless they start away from the run's first glyph (its
/// proven origin, past a leading TJ number) or off its line (its baseline,
/// raised by its Ts). Then the structured text had no characters of its
/// own for the run and its one match on the page is other characters (a
/// run of spaces spread by a large Tc), and the box starts at the run's
/// first glyph instead (on its raised baseline too when off the line);
/// engine.ts kept the other characters' box. Upright text with a positive
/// size only.
fn checked_bounds(record: &Record, bounds: [f64; 4]) -> [f64; 4] {
    let (Some([ox, oy]), Some([a, b, c, d]), Some(size)) =
        (record.origin, record.text_linear, record.tf_size)
    else {
        return bounds;
    };
    let th = record.horizontal_scale;
    let upright = a > 0.0 && d < 0.0 && b.abs() <= a * 1e-9 && c.abs() <= -d * 1e-9;
    // Ts moves the glyphs along the text y axis, unscaled by the size.
    let start = ox + record.rise * c - record.tj_lead / 1000.0 * size * th * a;
    let baseline = oy + record.rise * d;
    if !upright || size <= 0.0 || th <= 0.0 || !start.is_finite() || !baseline.is_finite() {
        return bounds;
    }
    let [x0, y0, x1, y1] = bounds;
    let height = y1 - y0;
    // A line's height of room around the glyph box for odd font metrics.
    let on_line = baseline >= y0 - height.max(1.0) && baseline <= y1 + height.max(1.0);
    if on_line && (x0 - start).abs() <= (height / 2.0).max(1.0) {
        return bounds;
    }
    let width = x1 - x0;
    match on_line {
        true => [start, y0, start + width, y1],
        false => [start, baseline - height, start + width, baseline],
    }
}

/// The WinAnsi bytes of `text` (callers check every character first).
pub fn win_ansi_bytes(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| win_ansi_byte(c).unwrap_or(b'?'))
        .collect()
}

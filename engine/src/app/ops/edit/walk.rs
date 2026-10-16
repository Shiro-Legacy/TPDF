// SPDX-License-Identifier: AGPL-3.0-or-later
//! The edit-target scanner's walk (content-edit.ts `scanEditTargets`,
//! `walkEditContent`): one bounded pass over a page's /Contents and the
//! Forms they invoke, tracking the graphics and text state each record
//! needs, then the passes that complete the records.
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

use super::font::{Cid, Fonts, at, dict_at, latin1, number};
use super::glyph::GlyphBlock;
use super::lex::{Container, Lexer, Operand, Token, next_q_end};
use super::scan::{
    ActualText, CidWrite, Clip, Cow, FontPath, FormCow, ImageFacts, Kind, PageScan, Placement,
    Record, Run, attach_geometry, attach_growth_limits, element_index, is_rtl_or_shaping,
    match_runs, parent_entry, runs_of, struct_element_in, target_id,
};
use super::{
    Budget, MAX_EDIT_FORM_DEPTH, MAX_EDIT_NESTING, MAX_EDIT_OPERANDS, MAX_EDIT_RAW_TARGETS,
    MAX_EDIT_REPLY_TEXT_CHARS, MAX_EDIT_SCAN_BYTES, MAX_EDIT_TARGETS, MAX_EDIT_TEXT_CHARS,
    MAX_EDIT_TRAVERSALS, decode, geometry::*, js_len, scan_error,
};
use crate::app::AppError;
use crate::app::info;
use crate::app::pagetree;
use crate::app::view_contents::encodes_samples;
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId, decode_text_string};
use crate::view::ViewDocument;

/// A `cm` since its own `q`, with its operand span.
#[derive(Clone, Debug)]
struct Cm {
    span: Range<usize>,
    matrix: Option<[f64; 6]>,
}

/// content-edit.ts `EditTextState`.
#[derive(Clone, Debug)]
struct TextState {
    font_name: Option<Vec<u8>>,
    font_size: Option<f64>,
    horizontal_scale: f64,
    character_spacing: f64,
    word_spacing: f64,
    in_text: bool,
    render_mode: f64,
    cms: Vec<Cm>,
    clean: bool,
    q_token: Option<Range<usize>>,
    q_path: Option<String>,
    lin: Option<[f64; 4]>,
    tm: Option<[f64; 4]>,
    leading: Option<f64>,
    ctm: Option<[f64; 6]>,
    tlm: Option<[f64; 6]>,
    clip_stage: i32,
    clip_pending: Option<Clip>,
    clip_re: Option<(Clip, String)>,
}

impl TextState {
    fn fresh(lin: Option<[f64; 4]>, ctm: Option<[f64; 6]>) -> Self {
        Self {
            font_name: None,
            font_size: None,
            horizontal_scale: 1.0,
            character_spacing: 0.0,
            word_spacing: 0.0,
            in_text: false,
            render_mode: 0.0,
            cms: Vec::new(),
            clean: true,
            q_token: None,
            q_path: None,
            lin,
            tm: Some([1.0, 0.0, 0.0, 1.0]),
            leading: Some(0.0),
            ctm,
            tlm: Some(IDENTITY),
            clip_stage: -1,
            clip_pending: None,
            clip_re: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Scope {
    optional: bool,
    actual_text: Option<ActualText>,
    shared_actual_text: bool,
    mcid: Option<i64>,
}

/// What a /Contents walk carries from one stream to the next.
struct Carried {
    current: TextState,
    saved: Vec<TextState>,
    scopes: Vec<Scope>,
    /// A Form walk under an ActualText (inline, shared or its MCID's
    /// StructElem) open around the `Do`: its runs' text is not their own.
    enclosing_actual_text: bool,
}

/// The page facts a scan starts from.
pub struct PageInput<'d> {
    pub index: usize,
    pub dict: &'d Dictionary,
    pub transform: [f64; 6],
    /// The page's width and height (the view's bounds).
    pub size: (f64, f64),
}

pub(super) struct Scanner<'d, 'v> {
    pub doc: &'d Document<'static>,
    pub view: Option<&'v ViewDocument>,
    pub page: &'d PageInput<'d>,
    pub records: Vec<Record>,
    pub budget: Budget,
    contents_refs: BTreeMap<ObjectId, usize>,
    runs: Option<Vec<Run>>,
    active_forms: BTreeSet<ObjectId>,
    pub fonts: Fonts,
    repertoire: BTreeMap<ObjectId, BTreeSet<u32>>,
    cid_pending: Vec<(usize, Arc<Cid>)>,
    pub geometry_blocked: bool,
    pub page_linear: [f64; 4],
    /// The decoded bytes of each depth-0 stream, by path.
    pub streams: BTreeMap<String, Arc<Vec<u8>>>,
    /// Every stream walked, by object: what the records' spans index.
    decoded: BTreeMap<ObjectId, Arc<Vec<u8>>>,
    pub glyph_blocks: BTreeMap<String, BTreeMap<usize, GlyphBlock>>,
    ordinary: usize,
    count_only: bool,
    /// The page's ParentTree entry, looked up once.
    parent: Option<Option<&'d [Object]>>,
}

/// content-edit.ts `scanEditTargets`.
pub fn scan_page(
    doc: &Document<'static>,
    view: Option<&ViewDocument>,
    page: &PageInput<'_>,
) -> Result<PageScan, AppError> {
    let own = page
        .dict
        .get(b"Resources".as_slice())
        .map(|v| doc.resolve(v))
        .filter(|v| !v.is_null());
    let resources = match own {
        Some(value) => value.as_dict(),
        None => {
            pagetree::inherited(doc, page.dict, b"Resources").and_then(|v| doc.resolve(v).as_dict())
        }
    };
    let rotate = pagetree::inherited(doc, page.dict, b"Rotate").map(|v| doc.resolve(v));
    let rotated = rotate.is_some_and(|r| number(Some(r)).is_some_and(|r| r.trunc() % 360.0 != 0.0));
    let unit = at(doc, Some(page.dict), b"UserUnit");
    let scaled = unit.is_some_and(|u| number(Some(u)) != Some(1.0));
    let geometry_blocked = rotated || scaled;

    // Sharing is counted from page dictionaries only.
    let mut contents_refs: BTreeMap<ObjectId, usize> = BTreeMap::new();
    for other in info::pages(doc) {
        let Some(contents) = other.dict.get(b"Contents".as_slice()) else {
            continue;
        };
        let resolved = doc.resolve(contents);
        match resolved.as_array() {
            Some(items) => {
                for item in items {
                    if let Some(id) = item.as_reference() {
                        *contents_refs.entry(id).or_default() += 1;
                    }
                }
            }
            None => {
                if let Some(id) = contents.as_reference() {
                    *contents_refs.entry(id).or_default() += 1;
                }
            }
        }
    }
    let mut elements: Vec<&Object> = Vec::new();
    if let Some(contents) = at(doc, Some(page.dict), b"Contents") {
        match contents.as_array() {
            Some(items) => elements.extend(items.iter()),
            None => elements.push(
                page.dict
                    .get(b"Contents".as_slice())
                    .unwrap_or(&Object::Null),
            ),
        }
    }
    let new = |count_only: bool| Scanner {
        doc,
        view,
        page,
        records: Vec::new(),
        budget: Budget::default(),
        contents_refs: contents_refs.clone(),
        runs: None,
        active_forms: BTreeSet::new(),
        fonts: Fonts::default(),
        repertoire: BTreeMap::new(),
        cid_pending: Vec::new(),
        geometry_blocked,
        page_linear: [
            page.transform[0],
            page.transform[1],
            page.transform[2],
            page.transform[3],
        ],
        streams: BTreeMap::new(),
        decoded: BTreeMap::new(),
        glyph_blocks: BTreeMap::new(),
        ordinary: 0,
        count_only,
        parent: None,
    };
    let walk_page = |scanner: &mut Scanner<'_, '_>| -> Result<Carried, AppError> {
        let mut carried = Carried {
            current: TextState::fresh(Some([1.0, 0.0, 0.0, 1.0]), Some(IDENTITY)),
            saved: Vec::new(),
            scopes: Vec::new(),
            enclosing_actual_text: false,
        };
        for (index, element) in elements.iter().enumerate() {
            let object = element.as_reference();
            let shared =
                object.is_some_and(|id| scanner.contents_refs.get(&id).copied().unwrap_or(0) > 1);
            let cow = Cow {
                inherited_resources: own.is_none(),
                shared_contents: shared.then_some(index),
                form: None,
            };
            scanner.walk(
                element,
                &format!("c{index}"),
                0,
                resources,
                &FontPath::default(),
                own.is_none() || shared,
                Some(cow),
                &mut carried,
            )?;
        }
        Ok(carried)
    };
    // Count first without matching, so a page that can never fit refuses fast.
    walk_page(&mut new(true))?;
    let mut scanner = new(false);
    let carried = walk_page(&mut scanner)?;
    scanner.attach_repertoires();
    let mut map_order: Vec<usize> = (0..scanner.records.len()).collect();
    super::glyph::collapse(&mut scanner, resources, &mut map_order)?;
    if scanner.records.len() > MAX_EDIT_TARGETS {
        return Err(scan_error(
            "too-large",
            format!("edit-targets: more than {MAX_EDIT_TARGETS} targets on this page"),
        ));
    }
    let mut records = scanner.records;
    attach_growth_limits(&mut records, geometry_blocked, page.size.0);
    attach_geometry(&mut records);
    let paragraphs = super::paragraph::find(&mut records);
    Ok(PageScan {
        page: page.index,
        records,
        map_order,
        paragraphs,
        balanced: carried.saved.is_empty() && carried.current.cms.is_empty(),
        transform: page.transform,
        budget_bytes: scanner.budget.bytes,
        streams: scanner.decoded,
    })
}

impl<'d> Scanner<'d, '_> {
    /// The /ActualText of the StructElem a page MCID names.
    fn tagged_actual_text(&mut self, mcid: i64) -> Option<&'d Object> {
        let (doc, page) = (self.doc, self.page.dict);
        let entry = *self.parent.get_or_insert_with(|| parent_entry(doc, page));
        let (_, element) = struct_element_in(doc, entry?, mcid)?;
        at(doc, Some(element), b"ActualText")
    }

    fn runs(&mut self) -> &[Run] {
        if self.runs.is_none() {
            self.runs = Some(match self.view {
                Some(view) => runs_of(view, self.page.index),
                None => Vec::new(),
            });
        }
        self.runs.as_deref().unwrap_or(&[])
    }

    /// content-edit.ts `editAttachRepertoires`.
    fn attach_repertoires(&mut self) {
        for (index, cid) in std::mem::take(&mut self.cid_pending) {
            let Some(record) = self.records.get_mut(index) else {
                continue;
            };
            let codes = record.cid_key.and_then(|key| self.repertoire.get(&key));
            let repertoire = codes.and_then(|codes| super::font::repertoire(&cid, codes));
            let Some(chars) = repertoire else {
                record.candidate = false;
                record.reason = Some("unsupported-encoding");
                record.cow = None;
                continue;
            };
            record.repertoire = Some(chars.iter().map(|(c, _)| *c).collect());
            record.cid_write = Some(Arc::new(CidWrite {
                chars,
                widths: cid.widths.clone(),
                dw: cid.dw,
                bytes_per_code: cid.bytes_per_code,
            }));
        }
    }

    /// content-edit.ts `emit`.
    fn emit(&mut self, record: Record) -> Result<usize, AppError> {
        if self.records.len() >= MAX_EDIT_RAW_TARGETS {
            return Err(scan_error(
                "too-large",
                format!("edit-targets: more than {MAX_EDIT_RAW_TARGETS} targets on this page"),
            ));
        }
        let joinable = record.is_text()
            && record.depth == 0
            && record.payload.as_ref().is_some_and(|p| p.len() == 1)
            && record.element.is_some()
            && self
                .glyph_blocks_of(&record.path)
                .contains_key(&record.operand.start);
        if !joinable {
            self.ordinary += 1;
            if self.ordinary > MAX_EDIT_TARGETS {
                return Err(scan_error(
                    "too-large",
                    format!("edit-targets: more than {MAX_EDIT_TARGETS} targets on this page"),
                ));
            }
        }
        if let Some(text) = &record.text {
            let length = js_len(text);
            if length > MAX_EDIT_TEXT_CHARS {
                return Err(scan_error(
                    "too-large",
                    format!(
                        "edit-targets: a text target is over the {MAX_EDIT_TEXT_CHARS} character ceiling"
                    ),
                ));
            }
            if self.budget.reply_text + length > MAX_EDIT_REPLY_TEXT_CHARS {
                return Err(scan_error(
                    "too-large",
                    format!(
                        "edit-targets: the reply text is over the {MAX_EDIT_REPLY_TEXT_CHARS} character ceiling"
                    ),
                ));
            }
            self.budget.reply_text += length;
        }
        self.records.push(record);
        Ok(self.records.len() - 1)
    }

    pub(super) fn glyph_blocks_of(&mut self, path: &str) -> &BTreeMap<usize, GlyphBlock> {
        if !self.glyph_blocks.contains_key(path) {
            let blocks = self
                .streams
                .get(path)
                .map(|bytes| super::glyph::blocks(bytes))
                .unwrap_or_default();
            self.glyph_blocks.insert(path.to_string(), blocks);
        }
        static EMPTY: BTreeMap<usize, GlyphBlock> = BTreeMap::new();
        self.glyph_blocks.get(path).unwrap_or(&EMPTY)
    }

    /// content-edit.ts `walkEditContent`.
    #[allow(clippy::too_many_arguments)]
    fn walk(
        &mut self,
        element: &Object,
        path: &str,
        depth: usize,
        resources: Option<&Dictionary>,
        font_path: &FontPath,
        structural_unproven: bool,
        cow: Option<Cow>,
        carried: &mut Carried,
    ) -> Result<(), AppError> {
        self.budget.traversals += 1;
        if self.budget.traversals > MAX_EDIT_TRAVERSALS {
            return Err(scan_error(
                "too-large",
                format!(
                    "edit-targets: more than {MAX_EDIT_TRAVERSALS} content streams on this page"
                ),
            ));
        }
        let doc = self.doc;
        let stream = element
            .as_reference()
            .and_then(|id| doc.resolve_id_with_holder(id).0)
            .filter(|id| doc.object(*id).is_ok_and(|o| o.stream.is_some()))
            .ok_or_else(|| {
                scan_error("invalid", "edit-targets: a /Contents entry is not a stream")
            })?;
        let room = MAX_EDIT_SCAN_BYTES.saturating_sub(self.budget.bytes);
        let bytes = match decode(doc, stream, room) {
            Ok(bytes) => bytes,
            Err(super::DecodeError::TooLarge) => {
                return Err(scan_error(
                    "too-large",
                    format!(
                        "edit-targets: the decoded streams exceed the {MAX_EDIT_SCAN_BYTES} byte scan ceiling"
                    ),
                ));
            }
            // What the view draws: hayro reads damage the strict decoder
            // refuses (a bad checksum, a truncated stream), as MuPDF does,
            // and draws nothing from a stream it cannot read either (an LZW
            // stream without a clear code), which MuPDF reads as empty too:
            // such a stream holds no targets and is never written.
            Err(super::DecodeError::Invalid(message)) => match self.view {
                // Image samples, not operators: the view leaves such a
                // stream out, and hayro would decode it whole (a JBIG2 bomb
                // page held about 1.5 GB).
                Some(_) if encodes_samples(doc, &Object::Reference(stream)) => Vec::new(),
                Some(view) => view
                    .decoded_stream(stream.number(), stream.generation())
                    .unwrap_or_default(),
                None => {
                    return Err(scan_error(
                        "invalid",
                        format!("edit-targets: a content stream cannot be read ({message})"),
                    ));
                }
            },
        };
        if self.budget.bytes + bytes.len() > MAX_EDIT_SCAN_BYTES {
            return Err(scan_error(
                "too-large",
                format!(
                    "edit-targets: the decoded streams exceed the {MAX_EDIT_SCAN_BYTES} byte scan ceiling"
                ),
            ));
        }
        self.budget.bytes += bytes.len();
        let bytes = Arc::new(bytes);
        if depth == 0 {
            self.streams.insert(path.to_string(), bytes.clone());
        }
        self.decoded.entry(stream).or_insert_with(|| bytes.clone());
        let mut walk = Walk {
            bytes: &bytes,
            lexer: Lexer::new(&bytes),
            path,
            depth,
            stream,
            resources,
            font_path,
            unproven: structural_unproven,
            cow,
            current: carried.current.clone(),
            saved: std::mem::take(&mut carried.saved),
            last_show: None,
            positioned_since_show: false,
            prev_show: None,
            between: Some([0.0, 0.0]),
            pending_image: false,
        };
        let mut stack: Vec<Vec<Operand>> = vec![Vec::new()];
        let mut openers: Vec<(Container, Range<usize>)> = Vec::new();
        let mut pending = 0usize;
        let push = |stack: &mut Vec<Vec<Operand>>, pending: &mut usize, value: Operand| {
            *pending += 1;
            if *pending > MAX_EDIT_OPERANDS {
                return Err(scan_error(
                    "too-large",
                    format!("edit-targets: more than {MAX_EDIT_OPERANDS} pending operands"),
                ));
            }
            if let Some(top) = stack.last_mut() {
                top.push(value);
            }
            Ok(())
        };
        loop {
            let token = walk.lexer.next(&mut self.budget.tokens)?;
            let Some(token) = token else { break };
            match token {
                Token::Operand(value) => push(&mut stack, &mut pending, value)?,
                Token::Open(container, span) => {
                    if stack.len() >= MAX_EDIT_NESTING {
                        return Err(scan_error(
                            "too-large",
                            format!("edit-targets: container nesting over {MAX_EDIT_NESTING}"),
                        ));
                    }
                    stack.push(Vec::new());
                    openers.push((container, span));
                }
                Token::Close(container, span) => {
                    if stack.len() < 2 {
                        let close = if container == Container::Array {
                            "]"
                        } else {
                            ">>"
                        };
                        return Err(scan_error(
                            "invalid",
                            format!("edit-targets: an unmatched '{close}'"),
                        ));
                    }
                    let items = stack.pop().unwrap_or_default();
                    let (opener, start) = openers.pop().unwrap_or((Container::Array, 0..0));
                    pending = pending.saturating_sub(items.len());
                    let value = match opener {
                        Container::Array => Operand::Arr(items, start.start..span.end),
                        Container::Dict => Operand::Dict(start.start..span.end),
                    };
                    push(&mut stack, &mut pending, value)?;
                }
                Token::Op(span) => {
                    if stack.len() != 1 {
                        return Err(scan_error(
                            "invalid",
                            format!(
                                "edit-targets: the operator {} sits inside an unclosed container",
                                latin1(&bytes[span.clone()])
                            ),
                        ));
                    }
                    let operands = std::mem::take(&mut stack[0]);
                    pending = pending.saturating_sub(operands.len());
                    walk.apply(self, span, operands, carried)?;
                }
            }
        }
        if stack.len() != 1 {
            return Err(scan_error(
                "invalid",
                "edit-targets: a content stream ends inside a container",
            ));
        }
        carried.current = walk.current;
        carried.saved = walk.saved;
        Ok(())
    }
}

/// One stream's walk state (the locals of `walkEditContent`).
struct Walk<'b, 'p> {
    bytes: &'b [u8],
    lexer: Lexer<'b>,
    path: &'p str,
    depth: usize,
    stream: ObjectId,
    resources: Option<&'p Dictionary>,
    font_path: &'p FontPath,
    unproven: bool,
    cow: Option<Cow>,
    current: TextState,
    saved: Vec<TextState>,
    last_show: Option<usize>,
    positioned_since_show: bool,
    prev_show: Option<String>,
    between: Option<[f64; 2]>,
    pending_image: bool,
}

fn numbers<const N: usize>(operands: &[Operand]) -> Option<[f64; N]> {
    if operands.len() != N {
        return None;
    }
    let mut out = [0.0; N];
    for (slot, operand) in out.iter_mut().zip(operands) {
        *slot = operand.number()?;
    }
    Some(out)
}

fn translate(between: Option<[f64; 2]>, operands: &[Operand]) -> Option<[f64; 2]> {
    let [x, y] = numbers::<2>(operands)?;
    let between = between?;
    Some([between[0] + x, between[1] + y])
}

fn line_move(tlm: Option<[f64; 6]>, by: Option<[f64; 2]>) -> Option<[f64; 6]> {
    let (tlm, by) = (tlm?, by?);
    Some(affine(&[1.0, 0.0, 0.0, 1.0, by[0], by[1]], &tlm))
}

impl Walk<'_, '_> {
    fn op_name(&self, span: &Range<usize>) -> &[u8] {
        &self.bytes[span.clone()]
    }

    /// content-edit.ts `applyOperator`.
    fn apply(
        &mut self,
        scanner: &mut Scanner<'_, '_>,
        span: Range<usize>,
        operands: Vec<Operand>,
        carried: &mut Carried,
    ) -> Result<(), AppError> {
        let op = self.op_name(&span).to_vec();
        let op = op.as_slice();
        let stage = self.current.clip_stage;
        let mut clip_step = false;
        if op == b"re" && stage == 0 && self.current.clean && self.current.cms.is_empty() {
            if let (Some([x, y, w, h]), [first, .., last]) =
                (numbers::<4>(&operands), operands.as_slice())
            {
                self.current.clip_pending = Some(Clip {
                    rect: [x.min(x + w), y.min(y + h), x.max(x + w), y.max(y + h)],
                    start: self.current.q_token.as_ref().map_or(span.start, |q| q.end),
                    end: span.end,
                    ops: first.span().start..last.span().end,
                });
                clip_step = true;
            }
        } else if op == b"W" && stage == 1 && operands.is_empty() {
            clip_step = true;
        } else if op == b"n" && stage == 2 && operands.is_empty() {
            if let Some(pending) = self.current.clip_pending.clone() {
                self.current.clip_re = Some((
                    Clip {
                        end: span.end,
                        ..pending
                    },
                    self.path.to_string(),
                ));
                clip_step = true;
            }
        }
        if clip_step {
            self.current.clip_stage = stage + 1;
        } else {
            if stage == 1 || stage == 2 {
                self.current.clean = false;
            }
            self.current.clip_stage = -1;
        }
        if !clip_step && !matches!(op, b"q" | b"Q" | b"cm" | b"Do") {
            self.current.clean = false;
        }
        if self.between.is_some() && !matches!(op, b"Td" | b"TD" | b"T*" | b"Tj" | b"TJ") {
            self.between = None;
        }
        match op {
            b"q" => {
                if self.saved.len() >= MAX_EDIT_NESTING {
                    return Err(scan_error(
                        "too-large",
                        format!("edit-targets: graphics-state nesting over {MAX_EDIT_NESTING}"),
                    ));
                }
                self.saved.push(self.current.clone());
                self.current = TextState {
                    cms: Vec::new(),
                    clean: true,
                    q_token: Some(span),
                    q_path: Some(self.path.to_string()),
                    clip_stage: 0,
                    clip_pending: None,
                    clip_re: None,
                    ..self.current.clone()
                };
            }
            b"Q" => {
                let Some(popped) = self.saved.pop() else {
                    return Err(scan_error(
                        "invalid",
                        "edit-targets: a 'Q' has no matching 'q'",
                    ));
                };
                self.current = popped;
            }
            b"cm" => {
                let six = numbers::<6>(&operands);
                let span = match (operands.first(), operands.last(), six) {
                    (Some(first), Some(last), Some(_)) => first.span().start..last.span().end,
                    _ => span,
                };
                self.current.cms.push(Cm { span, matrix: six });
                self.current.lin = match (six, self.current.lin) {
                    (Some(m), Some(lin)) => Some(multiply(&[m[0], m[1], m[2], m[3]], &lin)),
                    _ => None,
                };
                self.current.ctm = match (six, self.current.ctm) {
                    (Some(m), Some(ctm)) => Some(affine(&m, &ctm)),
                    _ => None,
                };
            }
            b"BT" => {
                self.current.in_text = true;
                self.current.tm = Some([1.0, 0.0, 0.0, 1.0]);
                self.current.tlm = Some(IDENTITY);
                self.prev_show = None;
                self.last_show = None;
                self.positioned_since_show = false;
            }
            b"ET" => {
                self.current.in_text = false;
                self.prev_show = None;
                self.last_show = None;
            }
            b"Tm" => {
                let six = numbers::<6>(&operands);
                self.current.tm = six.map(|m| [m[0], m[1], m[2], m[3]]);
                self.current.tlm = six;
                self.positioned_since_show = true;
            }
            b"TD" => {
                self.current.leading = match operands.as_slice() {
                    [_, Operand::Num(y, _)] => Some(-y),
                    _ => None,
                };
                self.between = translate(self.between, &operands);
                self.current.tlm =
                    line_move(self.current.tlm, translate(Some([0.0, 0.0]), &operands));
                self.positioned_since_show = true;
            }
            b"TL" => {
                self.current.leading = match operands.as_slice() {
                    [Operand::Num(l, _)] => Some(*l),
                    _ => None,
                };
            }
            b"Td" => {
                self.between = translate(self.between, &operands);
                self.current.tlm =
                    line_move(self.current.tlm, translate(Some([0.0, 0.0]), &operands));
                self.positioned_since_show = true;
            }
            b"T*" => {
                let leading = self.current.leading;
                self.between = match (self.between, leading) {
                    (Some(b), Some(l)) => Some([b[0], b[1] - l]),
                    _ => None,
                };
                self.current.tlm = line_move(self.current.tlm, leading.map(|l| [0.0, -l]));
                self.positioned_since_show = true;
            }
            b"BDC" => {
                let scope = self.marked_scope(scanner, &operands)?;
                carried.scopes.push(scope);
            }
            b"BMC" => carried.scopes.push(Scope::default()),
            b"EMC" => {
                carried.scopes.pop();
            }
            b"Tf" => match operands.as_slice() {
                [Operand::Name(name, _), Operand::Num(size, _), ..] => {
                    self.current.font_name = Some(name.clone());
                    self.current.font_size = Some(*size);
                }
                _ => {
                    self.current.font_name = None;
                    self.current.font_size = None;
                }
            },
            b"Tr" => {
                self.current.render_mode = match operands.first() {
                    Some(Operand::Num(mode, _)) if mode.fract() == 0.0 => *mode,
                    _ => -1.0,
                };
            }
            b"Tz" => {
                self.current.horizontal_scale = match operands.first() {
                    Some(Operand::Num(v, _)) if v.is_finite() => v / 100.0,
                    _ => 1.0,
                };
            }
            b"Tc" => {
                self.current.character_spacing = match operands.first() {
                    Some(Operand::Num(v, _)) if v.is_finite() => *v,
                    _ => 0.0,
                };
            }
            b"Tw" => {
                self.current.word_spacing = match operands.first() {
                    Some(Operand::Num(v, _)) if v.is_finite() => *v,
                    _ => 0.0,
                };
            }
            b"Tj" | b"TJ" => self.classify_text(scanner, &operands, span, carried)?,
            b"'" | b"\"" => {
                if op == b"\"" {
                    // `aw ac string "` sets Tw and Tc first.
                    let spacing = |index: usize| match operands.get(index) {
                        Some(Operand::Num(v, _)) if v.is_finite() => *v,
                        _ => 0.0,
                    };
                    self.current.word_spacing = spacing(0);
                    self.current.character_spacing = spacing(1);
                }
                self.classify_text(scanner, &[], span, carried)?;
                self.current.tlm = None;
                self.last_show = None;
                self.positioned_since_show = true;
            }
            b"Do" => {
                self.classify_xobject(scanner, &operands, span, carried)?;
                self.current.clean = false;
            }
            b"BI" => self.pending_image = true,
            b"ID" if self.pending_image => {
                self.pending_image = false;
                self.skip_inline_image(span.end, &operands)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// content-edit.ts `declaredInlineLength`; `/IM true` counts (engine.ts
    /// lexed `true` as an operator and never saw it).
    fn declared_inline_length(bytes: &[u8], operands: &[Operand]) -> Option<usize> {
        let mut filtered = false;
        let mut values: BTreeMap<&[u8], f64> = BTreeMap::new();
        let mut color_space: Option<&[u8]> = None;
        let mut image_mask = false;
        for pair in operands.windows(2) {
            let Operand::Name(key, _) = &pair[0] else {
                continue;
            };
            let value = &pair[1];
            if key == b"F" || key == b"Filter" {
                filtered = true;
            }
            if key == b"L" || key == b"Length" {
                if let Operand::Num(n, _) = value {
                    if n.fract() == 0.0 && *n >= 0.0 {
                        return (*n <= usize::MAX as f64).then_some(*n as usize);
                    }
                }
            }
            if let Operand::Num(n, _) = value {
                if n.is_finite() {
                    values.insert(key, *n);
                }
            }
            if key == b"CS" {
                if let Operand::Name(name, _) = value {
                    color_space = Some(name);
                }
            }
            if key == b"IM" {
                image_mask = matches!(value, Operand::Word(span) if bytes.get(span.clone()) == Some(b"true".as_slice()));
            }
        }
        if filtered {
            return None;
        }
        let (width, height) = (*values.get(b"W".as_slice())?, *values.get(b"H".as_slice())?);
        if width.fract() != 0.0 || height.fract() != 0.0 || width < 0.0 || height < 0.0 {
            return None;
        }
        if width == 0.0 || height == 0.0 {
            return Some(0);
        }
        let components = if image_mask {
            1.0
        } else {
            match color_space? {
                b"G" | b"DeviceGray" | b"I" | b"Indexed" => 1.0,
                b"RGB" | b"DeviceRGB" => 3.0,
                b"CMYK" | b"DeviceCMYK" => 4.0,
                _ => return None,
            }
        };
        let depth = if image_mask {
            1.0
        } else {
            let bits = *values.get(b"BPC".as_slice())?;
            if bits.fract() != 0.0 || bits <= 0.0 {
                return None;
            }
            bits
        };
        let length = ((width * components * depth) / 8.0).ceil() * height;
        (length.is_finite() && length >= 0.0 && length <= usize::MAX as f64)
            .then_some(length as usize)
    }

    /// content-edit.ts `skipInlineImage`.
    fn skip_inline_image(&mut self, from: usize, operands: &[Operand]) -> Result<(), AppError> {
        let Some(declared) = Self::declared_inline_length(self.bytes, operands) else {
            return Err(scan_error(
                "invalid",
                "edit-targets: an inline image has no declared /L, so its boundary is ambiguous",
            ));
        };
        let bytes = self.bytes;
        if !super::lex::is_white(bytes.get(from).copied()) {
            return Err(scan_error(
                "invalid",
                "edit-targets: an inline image's ID is not followed by whitespace",
            ));
        }
        let mut start = from + 1;
        if bytes.get(from) == Some(&13) && bytes.get(start) == Some(&10) {
            start += 1;
        }
        let end = start.saturating_add(declared);
        if !super::lex::is_white(bytes.get(end).copied()) {
            return Err(scan_error(
                "invalid",
                "edit-targets: an inline image length does not end at a boundary",
            ));
        }
        let mut cursor = end;
        while cursor < bytes.len() && super::lex::is_white(bytes.get(cursor).copied()) {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'E') || bytes.get(cursor + 1) != Some(&b'I') {
            return Err(scan_error(
                "invalid",
                "edit-targets: an inline image has no EI after its declared length",
            ));
        }
        self.lexer.pos = cursor + 2;
        Ok(())
    }

    /// content-edit.ts `markedScope`.
    fn marked_scope(
        &mut self,
        scanner: &mut Scanner<'_, '_>,
        operands: &[Operand],
    ) -> Result<Scope, AppError> {
        let mut scope = Scope::default();
        if matches!(operands.first(), Some(Operand::Name(tag, _)) if tag == b"OC") {
            scope.optional = true;
        }
        let properties = if operands.len() > 1 {
            operands.last()
        } else {
            None
        };
        match properties {
            Some(Operand::Dict(span)) => {
                let mut lexer = Lexer::new(&self.bytes[..span.end.min(self.bytes.len())]);
                lexer.pos = span.start + 2;
                let mut depth = 0usize;
                let mut last_name: Option<Vec<u8>> = None;
                while lexer.pos < span.end {
                    let Some(token) = lexer.next(&mut scanner.budget.tokens)? else {
                        break;
                    };
                    match token {
                        Token::Open(..) => depth += 1,
                        Token::Close(..) => {
                            if depth == 0 {
                                break;
                            }
                            depth -= 1;
                        }
                        Token::Operand(value) if depth == 0 => match value {
                            Operand::Name(name, _) => {
                                if last_name.as_deref() == Some(b"Type")
                                    && (name == b"OCG" || name == b"OCMD")
                                {
                                    scope.optional = true;
                                }
                                last_name = Some(name);
                            }
                            Operand::Str(bytes, span)
                                if last_name.as_deref() == Some(b"ActualText") =>
                            {
                                scope.actual_text = Some(ActualText {
                                    path: self.path.to_string(),
                                    span,
                                    bytes,
                                });
                                last_name = None;
                            }
                            Operand::Num(n, _) if last_name.as_deref() == Some(b"MCID") => {
                                scope.mcid = Some(n.trunc() as i64);
                                last_name = None;
                            }
                            _ => last_name = None,
                        },
                        _ => {}
                    }
                }
            }
            Some(Operand::Name(name, _)) => {
                let doc = scanner.doc;
                let dict = dict_at(doc, self.resources, b"Properties")
                    .and_then(|props| props.get(name.as_slice()))
                    .and_then(|value| doc.resolve(value).as_dict());
                if let Some(dict) = dict {
                    let kind = at(doc, Some(dict), b"Type").and_then(Object::as_name);
                    if matches!(kind, Some(b"OCG" | b"OCMD")) {
                        scope.optional = true;
                    }
                    if at(doc, Some(dict), b"ActualText").is_some() {
                        scope.shared_actual_text = true;
                    }
                    if let Some(mcid) = number(at(doc, Some(dict), b"MCID")) {
                        scope.mcid = Some(mcid.trunc() as i64);
                    }
                }
            }
            _ => {}
        }
        Ok(scope)
    }

    /// content-edit.ts `classifyText`.
    fn classify_text(
        &mut self,
        scanner: &mut Scanner<'_, '_>,
        operands: &[Operand],
        token: Range<usize>,
        carried: &Carried,
    ) -> Result<(), AppError> {
        let doc = scanner.doc;
        let op = self.op_name(&token).to_vec();
        let tj = op == b"TJ";
        let lone = match operands {
            [Operand::Str(bytes, span)] => Some((bytes.clone(), span.clone())),
            _ => None,
        };
        let valid_array = match operands {
            [Operand::Arr(items, span)] if tj => items
                .iter()
                .all(|item| matches!(item, Operand::Str(..) | Operand::Num(..)))
                .then_some((items, span)),
            _ => None,
        };
        let payload = lone.or_else(|| {
            valid_array.map(|(items, span)| {
                let mut joined = Vec::new();
                for item in items.iter() {
                    if let Operand::Str(bytes, _) = item {
                        joined.extend_from_slice(bytes);
                    }
                }
                (joined, span.clone())
            })
        });
        let tj_adjustment = valid_array.map_or(0.0, |(items, _)| {
            items.iter().filter_map(Operand::number).sum::<f64>()
        });
        let font = scanner.fonts.resolve(
            doc,
            self.resources,
            self.current.font_name.as_deref(),
            &mut scanner.budget,
        );
        let cid = font.as_ref().and_then(|f| f.cid.clone());
        let cid_decoded = match (&cid, &payload) {
            (Some(cid), Some((bytes, _))) => cid.decode(bytes),
            _ => None,
        };
        if let (Some(cid), Some((bytes, _)), Some(object)) =
            (&cid, &payload, font.as_ref().and_then(|f| f.object))
        {
            scanner
                .repertoire
                .entry(object)
                .or_default()
                .extend(cid.codes(bytes));
        }
        let oc_depth = carried.scopes.iter().filter(|s| s.optional).count();
        let actuals: Vec<&ActualText> = carried
            .scopes
            .iter()
            .filter_map(|s| s.actual_text.as_ref())
            .collect();
        let shared_actual_text = carried.scopes.iter().any(|s| s.shared_actual_text);
        let mcid = carried.scopes.iter().rev().find_map(|s| s.mcid);
        let mut reason: Option<&'static str> = None;
        let mode = self.current.render_mode;
        if !self.current.in_text || !(mode == 0.0 || mode == 1.0 || mode == 2.0) || oc_depth > 0 {
            reason = Some("unsupported-graphics-state");
        } else if payload.as_ref().is_none_or(|(bytes, _)| bytes.is_empty()) {
            reason = Some("unsupported-text-operator");
        } else if (self.depth > 0 || self.unproven) && self.cow.is_none() {
            reason = Some("shared-or-unproven-sharing");
        }
        let cowable = if reason.is_none() && (self.depth > 0 || self.unproven) {
            self.cow.clone()
        } else {
            None
        };
        let first_on_line = self.last_show.is_none() || self.positioned_since_show;
        let run_origin = match (first_on_line, self.current.tlm, self.current.ctm) {
            (true, Some(tlm), Some(ctm)) => {
                let m = affine(&affine(&tlm, &ctm), &scanner.page.transform);
                Some([m[4], m[5]])
            }
            _ => None,
        };
        if reason.is_none() {
            if let Some((bytes, _)) = &payload {
                reason = match font.as_deref() {
                    None => Some("unsupported-encoding"),
                    Some(f) if f.writing_mode == Some(1.0) => Some("vertical-writing"),
                    Some(f) if f.is_subtype(b"Type3") => Some("type3"),
                    Some(f) if f.is_subtype(b"Type0") => match (&f.cid, &cid_decoded) {
                        (None, _) if f.subset == Some(true) => Some("subset-font"),
                        (None, _) | (Some(_), None) => Some("unsupported-encoding"),
                        (Some(_), Some((text, _))) if is_rtl_or_shaping(text) => {
                            Some("rtl-or-shaping")
                        }
                        _ => None,
                    },
                    Some(f) if f.cid.is_some() => match &cid_decoded {
                        None => Some("unsupported-encoding"),
                        Some((text, _)) if is_rtl_or_shaping(text) => Some("rtl-or-shaping"),
                        _ => None,
                    },
                    Some(f) if !f.simple || f.encoding_unknown => Some("unsupported-encoding"),
                    Some(f) if f.codec.decode(bytes).is_none() => Some("unsupported-encoding"),
                    _ => None,
                };
            }
        }
        let text: Option<String> = match (&cid_decoded, &payload, font.as_deref()) {
            (Some((text, _)), _, _) => Some(text.clone()),
            (None, Some((bytes, _)), Some(f))
                if f.simple && !f.encoding_unknown && f.cid.is_none() =>
            {
                f.codec.decode(bytes)
            }
            _ => None,
        };
        let mut bounds = None;
        if reason.is_none() && !scanner.count_only {
            if let Some(text) = &text {
                // By the name the view gives the font, not the display name.
                let name = font.as_ref().and_then(|f| f.view_name.clone());
                let found = match_runs(
                    scanner.runs(),
                    text,
                    name.as_deref(),
                    cowable.is_some(),
                    tj,
                    run_origin,
                    self.depth,
                );
                match found {
                    None => reason = Some("ambiguous-text-association"),
                    Some((found, _)) => bounds = found,
                }
            }
        }
        let mut actual_text = None;
        let mut struct_actual_text = None;
        if reason.is_none() && payload.is_some() {
            if let Some(text) = &text {
                if shared_actual_text || actuals.len() > 1 || carried.enclosing_actual_text {
                    reason = Some("ambiguous-text-association");
                } else if let Some(actual) = actuals.first() {
                    // A tag in another stream cannot be rewritten with this one.
                    if actual.path == self.path && decode_text_string(&actual.bytes) == *text {
                        actual_text = Some((*actual).clone());
                    } else {
                        reason = Some("ambiguous-text-association");
                    }
                }
                if reason.is_none() && mcid.is_some() && self.depth > 0 {
                    reason = Some("ambiguous-text-association");
                }
                if reason.is_none() {
                    if let Some(mcid) = mcid {
                        let tagged = scanner.tagged_actual_text(mcid).and_then(Object::as_string);
                        if let Some(tagged) = tagged {
                            if decode_text_string(tagged) != *text || actual_text.is_some() {
                                reason = Some("ambiguous-text-association");
                            } else {
                                struct_actual_text = Some(text.clone());
                            }
                        }
                    }
                }
            }
        }
        let mut hashed = op.clone();
        hashed.push(b'|');
        if let Some((bytes, _)) = &payload {
            hashed.extend_from_slice(bytes);
        }
        let id = target_id(scanner.page.index, self.path, token.start, &hashed);
        let mut record = Record::new(id.clone(), Kind::Text, self.path, self.stream, self.depth);
        record.op = token.clone();
        record.operand = payload
            .as_ref()
            .map_or(token.clone(), |(_, span)| span.clone());
        record.candidate = reason.is_none();
        record.reason = reason;
        record.mcid = mcid;
        record.oc_depth = oc_depth;
        record.horizontal_scale = self.current.horizontal_scale;
        record.character_spacing = self.current.character_spacing;
        record.word_spacing = self.current.word_spacing;
        record.render_mode = self.current.render_mode;
        record.tj_adjustment = tj_adjustment;
        record.kerned = tj;
        record.codec = font.as_ref().map(|f| f.codec);
        record.font_type0 = font.as_ref().is_some_and(|f| f.is_subtype(b"Type0"));
        record.text_linear = match (self.current.tm, self.current.lin) {
            (Some(tm), Some(lin)) => Some(multiply(&multiply(&tm, &lin), &scanner.page_linear)),
            _ => None,
        };
        record.first_on_line = first_on_line;
        record.origin = run_origin;
        record.tf_name = self.current.font_name.clone();
        record.tf_size = self.current.font_size;
        record.leading = self.current.leading;
        record.prev_show = self.prev_show.clone();
        record.between = if self.prev_show.is_some() {
            self.between
        } else {
            None
        };
        record.text = text;
        record.font = Some(font.as_ref().map(|f| f.summary.clone()).unwrap_or_default());
        record.bounds = bounds;
        record.payload = payload.map(|(bytes, _)| bytes);
        record.actual_text = actual_text;
        record.struct_actual_text = struct_actual_text;
        record.font_path = self.font_path.clone();
        if reason.is_none() {
            record.cow = cowable;
        }
        let pending_cid = match (&cid_decoded, &cid) {
            (Some(_), Some(cid)) if reason.is_none() => {
                record.font_size = self.current.font_size;
                record.cid_key = font.as_ref().and_then(|f| f.object);
                Some(cid.clone())
            }
            _ => None,
        };
        if let (Some(last), false) = (self.last_show, self.positioned_since_show) {
            if let Some(previous) = scanner.records.get_mut(last) {
                previous.tail_follows = true;
                previous.tail_target = Some(id.clone());
            }
        }
        let index = scanner.emit(record)?;
        if let Some(cid) = pending_cid {
            scanner.cid_pending.push((index, cid));
        }
        self.last_show = Some(index);
        self.positioned_since_show = false;
        self.prev_show = Some(id);
        self.between = Some([0.0, 0.0]);
        Ok(())
    }

    /// content-edit.ts `simplePlacement`.
    fn simple_placement(
        &self,
        scanner: &Scanner<'_, '_>,
        token: &Range<usize>,
        rect: [f64; 4],
    ) -> Option<Placement> {
        let q = self.current.q_token.as_ref()?;
        if self.saved.len() != 1 || self.current.q_path.as_deref() != Some(self.path) {
            return None;
        }
        if !self.current.clean || self.current.cms.len() != 1 {
            return None;
        }
        if !self.saved.iter().all(|s| s.cms.is_empty()) {
            return None;
        }
        let cm = self.current.cms.first()?;
        let matrix = cm.matrix?;
        let orient = orient_of(&matrix)?;
        let q_end = next_q_end(self.bytes, token.end)?;
        let full = map_rect(&matrix, &rect);
        let mut visible = full;
        let mut clip = None;
        if let Some((re, path)) = &self.current.clip_re {
            if path == self.path {
                visible = [
                    full[0].max(re.rect[0]),
                    full[1].max(re.rect[1]),
                    full[2].min(re.rect[2]),
                    full[3].min(re.rect[3]),
                ];
                let narrow = |side: f64| side.partial_cmp(&1.0).is_none_or(|o| o.is_lt());
                if narrow(visible[2] - visible[0]) || narrow(visible[3] - visible[1]) {
                    return None;
                }
                clip = Some(re.clone());
            }
        }
        Some(Placement {
            cm: cm.span.clone(),
            q_start: q.start,
            q_end,
            full,
            rect,
            view: view_box(&visible, &scanner.page.transform),
            matrix,
            orient,
            clip,
            visible,
        })
    }

    /// content-edit.ts `classifyXObject`.
    fn classify_xobject(
        &mut self,
        scanner: &mut Scanner<'_, '_>,
        operands: &[Operand],
        token: Range<usize>,
        carried: &Carried,
    ) -> Result<(), AppError> {
        let doc = scanner.doc;
        let payload = match operands.first() {
            Some(Operand::Name(name, span)) => Some((name.clone(), span.clone())),
            _ => None,
        };
        let page = scanner.page.index;
        let base = |id: String, kind: Kind| {
            let mut record = Record::new(id, kind, self.path, self.stream, self.depth);
            record.op = token.clone();
            record.operand = payload
                .as_ref()
                .map_or(token.clone(), |(_, span)| span.clone());
            record
        };
        let Some((name, _)) = payload.clone() else {
            let mut record = base(target_id(page, self.path, token.start, b"Do|"), Kind::Form);
            record.reason = Some("unsupported-graphics-state");
            scanner.emit(record)?;
            return Ok(());
        };
        let mut hashed = b"Do|".to_vec();
        hashed.extend_from_slice(&name);
        let id = target_id(page, self.path, token.start, &hashed);
        let entry = dict_at(doc, self.resources, b"XObject").and_then(|x| x.get(name.as_slice()));
        let target = entry
            .map(|value| doc.resolve_with_holder(value))
            .filter(|(_, value)| !value.is_null());
        let Some((holder, target_value)) = target else {
            let mut record = base(id, Kind::Form);
            record.reason = Some("unresolved-resource");
            scanner.emit(record)?;
            return Ok(());
        };
        let reference = holder.filter(|_| entry.is_some_and(|e| e.as_reference().is_some()));
        let target = target_value.as_dict();
        let subtype = at(doc, target, b"Subtype").and_then(Object::as_name);
        let oc_depth = carried.scopes.iter().filter(|s| s.optional).count();
        if subtype == Some(b"Image") {
            let int = |key: &[u8]| number(at(doc, target, key)).map_or(0, |v| v.trunc() as i64);
            let smask = at(doc, target, b"SMask");
            let interpolate = at(doc, target, b"Interpolate");
            let image = ImageFacts {
                width: int(b"Width"),
                height: int(b"Height"),
                smask: smask.is_some_and(|v| v.as_name() != Some(b"None")),
                mask: at(doc, target, b"Mask").is_some(),
                decode: at(doc, target, b"Decode").is_some(),
                interpolate: interpolate.is_some_and(|v| v.as_bool().unwrap_or(true)),
                oc: at(doc, target, b"OC").is_some() || oc_depth > 0,
            };
            let clean =
                !image.smask && !image.mask && !image.decode && !image.interpolate && !image.oc;
            let placement = self.simple_placement(scanner, &token, [0.0, 0.0, 1.0, 1.0]);
            let (ops, reason): (Vec<&'static str>, Option<&'static str>) =
                if scanner.geometry_blocked {
                    (vec![], Some("unsupported-transform"))
                } else if self.depth > 0 {
                    (vec![], Some("shared-or-unproven-sharing"))
                } else if !clean {
                    let ops = if placement.is_some() {
                        vec!["move", "resize", "delete", "duplicate", "transform", "crop"]
                    } else {
                        vec![]
                    };
                    (ops, Some("unsupported-image"))
                } else if placement.is_some() {
                    (
                        vec![
                            "move",
                            "resize",
                            "delete",
                            "replace",
                            "duplicate",
                            "transform",
                            "crop",
                        ],
                        None,
                    )
                } else {
                    (vec!["replace"], Some("unsupported-transform"))
                };
            let mut record = base(id, Kind::Image);
            record.object = reference;
            record.candidate = !ops.is_empty();
            record.object_name = Some(name);
            record.image = Some(image);
            record.reason = reason;
            if !ops.is_empty() {
                if let Some(placement) = placement {
                    record.bounds = Some(placement.visible);
                    record.placement = Some(placement);
                }
                if self.unproven {
                    record.cow = self.cow.clone();
                }
                record.object_ops = Some(ops);
            }
            scanner.emit(record)?;
            return Ok(());
        }
        if subtype != Some(b"Form") {
            let mut record = base(id, Kind::Form);
            record.object = reference;
            record.reason = Some("unsupported-xobject");
            record.object_name = Some(name);
            scanner.emit(record)?;
            return Ok(());
        }
        let rect = form_rect(doc, target);
        let placement = rect.and_then(|rect| self.simple_placement(scanner, &token, rect));
        let (ops, reason): (Vec<&'static str>, Option<&'static str>) = if scanner.geometry_blocked {
            (vec![], Some("unsupported-transform"))
        } else if self.depth > 0 {
            (vec![], Some("shared-or-unproven-sharing"))
        } else if placement.is_none() {
            (vec![], Some("unsupported-transform"))
        } else {
            (
                vec!["move", "resize", "delete", "duplicate", "transform", "crop"],
                None,
            )
        };
        let mut record = base(id, Kind::Form);
        record.object = reference;
        record.candidate = !ops.is_empty();
        record.object_name = Some(name.clone());
        record.reason = reason;
        if !ops.is_empty() {
            if let Some(placement) = placement {
                record.bounds = Some(placement.visible);
                record.placement = Some(placement);
            }
            if self.unproven {
                record.cow = self.cow.clone();
            }
            record.object_ops = Some(ops);
        }
        scanner.emit(record)?;
        if self.depth + 1 > MAX_EDIT_FORM_DEPTH {
            return Err(scan_error(
                "too-large",
                format!("edit-targets: Form nesting over {MAX_EDIT_FORM_DEPTH}"),
            ));
        }
        if let Some(reference) = reference {
            if !scanner.active_forms.insert(reference) {
                return Ok(());
            }
        }
        let child_cow = match (&self.cow, self.depth, reference, element_index(self.path)) {
            (Some(cow), 0, Some(reference), Some(element)) => Some(Cow {
                form: Some(FormCow {
                    name: name.clone(),
                    reference,
                    element,
                    operand: payload.map_or(0..0, |(_, span)| span),
                }),
                ..cow.clone()
            }),
            _ => None,
        };
        let form_numbers = match at(doc, target, b"Matrix") {
            None => Some(IDENTITY),
            Some(matrix) => numbers_of(doc, matrix),
        };
        let form_linear = match (form_numbers, self.current.lin) {
            (Some(m), Some(lin)) => Some(multiply(&[m[0], m[1], m[2], m[3]], &lin)),
            _ => None,
        };
        let form_ctm = match (form_numbers, self.current.ctm) {
            (Some(m), Some(ctm)) => Some(affine(&m, &ctm)),
            _ => None,
        };
        let page_mcid = carried
            .scopes
            .iter()
            .rev()
            .find_map(|s| s.mcid)
            .filter(|_| self.depth == 0);
        let enclosing_actual_text = carried.enclosing_actual_text
            || carried
                .scopes
                .iter()
                .any(|s| s.actual_text.is_some() || s.shared_actual_text)
            || page_mcid.is_some_and(|mcid| scanner.tagged_actual_text(mcid).is_some());
        let mut carried = Carried {
            current: TextState {
                tlm: Some(IDENTITY),
                ..TextState::fresh(form_linear, form_ctm)
            },
            saved: Vec::new(),
            scopes: Vec::new(),
            enclosing_actual_text,
        };
        let child_path = format!("{}/{}@{}", self.path, latin1(&name), token.start);
        let mut font_path = self.font_path.clone();
        font_path.forms.push(name);
        let resources = dict_at(doc, target, b"Resources");
        let element = entry.cloned().unwrap_or(Object::Null);
        let result = scanner.walk(
            &element,
            &child_path,
            self.depth + 1,
            resources,
            &font_path,
            self.unproven,
            child_cow,
            &mut carried,
        );
        if let Some(reference) = reference {
            scanner.active_forms.remove(&reference);
        }
        result
    }
}

/// An array of exactly six finite numbers.
fn numbers_of(doc: &Document<'_>, value: &Object) -> Option<[f64; 6]> {
    let items = value.as_array()?;
    if items.len() != 6 {
        return None;
    }
    let mut out = [0.0; 6];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = number(Some(doc.resolve(item))).filter(|v| v.is_finite())?;
    }
    Some(out)
}

/// content-edit.ts `editRectOf`: four finite numbers, normalized.
pub fn rect_of(doc: &Document<'_>, value: Option<&Object>) -> Option<[f64; 4]> {
    let items = value?.as_array()?;
    if items.len() != 4 {
        return None;
    }
    let mut v = [0.0; 4];
    for (slot, item) in v.iter_mut().zip(items) {
        *slot = number(Some(doc.resolve(item)))?;
    }
    if v.iter().any(|n| !n.is_finite()) {
        return None;
    }
    Some([
        v[0].min(v[2]),
        v[1].min(v[3]),
        v[0].max(v[2]),
        v[1].max(v[3]),
    ])
}

/// content-edit.ts `formRect`: the /BBox under a positive scale-and-translate /Matrix.
fn form_rect(doc: &Document<'_>, target: Option<&Dictionary>) -> Option<[f64; 4]> {
    let bbox = rect_of(doc, at(doc, target, b"BBox"))?;
    let matrix = match at(doc, target, b"Matrix") {
        None => IDENTITY,
        Some(value) => {
            let items = value.as_array()?;
            if items.len() != 6 {
                return None;
            }
            let mut m = [0.0; 6];
            for (slot, item) in m.iter_mut().zip(items) {
                *slot = number(Some(doc.resolve(item)))?;
            }
            if m.iter().any(|v| !v.is_finite())
                || m[1] != 0.0
                || m[2] != 0.0
                || m[0].partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
                || m[3].partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater)
            {
                return None;
            }
            m
        }
    };
    Some([
        matrix[0] * bbox[0] + matrix[4],
        matrix[3] * bbox[1] + matrix[5],
        matrix[0] * bbox[2] + matrix[4],
        matrix[3] * bbox[3] + matrix[5],
    ])
}

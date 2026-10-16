// SPDX-License-Identifier: AGPL-3.0-or-later
//! A dry run of the content hayro would interpret for one page. hayro has no
//! cycle check and recurses into patterns and Type3 glyphs without a depth
//! limit, so a page whose content invokes itself, nests too deeply, expands
//! past a work budget or shows more text than the view can hold is refused
//! before interpretation.
use super::ViewError;
use hayro_interpret::CacheKey;
use hayro_syntax::content::TypedIter;
use hayro_syntax::content::ops::TypedInstruction;
use hayro_syntax::object::dict::keys::{
    ANNOTS, AP, AS, BBOX, CHAR_PROCS, CONTENTS, F, FILTER, FONT, FORM, G, N, PATTERN_TYPE,
    RESOURCES, SMASK, SUBTYPE, TYPE3,
};
use hayro_syntax::object::{Array, Dict, Name, Object, ObjectIdentifier, Stream};
use hayro_syntax::page::{Page, Resources};
use std::collections::HashMap;

/// Content-stream runs (page, forms, pattern cells, soft masks, Type3 glyphs)
/// one page may cost.
pub const MAX_CONTENT_RUNS: u64 = 1 << 20;
/// String-operand bytes the text operators of one page may show, counted
/// once per time each is drawn. hayro holds a glyph per byte of a show
/// operation and the text device a character or more per glyph, about 330
/// bytes of WASM memory per glyph in all.
pub const MAX_SHOWN_BYTES: u64 = 1 << 20;
/// hayro skips a form drawn at this nesting depth.
pub(crate) const FORM_DEPTH: u32 = 50;

/// What one walk covers: forms only (text extraction), or everything the
/// rasterizer interprets.
#[derive(Clone, Copy)]
pub(crate) struct Scope {
    pub(crate) paint: bool,
    pub(crate) annotations: bool,
}

pub(crate) fn check_page(page: &Page<'_>, scope: Scope) -> Result<(), ViewError> {
    let mut walk = Walk {
        scope,
        active: Vec::new(),
        memo: HashMap::new(),
    };
    let resources = page.resources();
    let contents = match page.raw().get::<Object<'_>>(CONTENTS) {
        Some(Object::Stream(stream)) => vec![stream],
        Some(Object::Array(array)) => array.iter::<Stream<'_>>().collect(),
        _ => Vec::new(),
    };
    if contents.iter().any(encodes_samples) {
        return Err(SAMPLES);
    }
    let mut total = match page.page_stream() {
        Some(data) => walk.content(data, resources, 0)?,
        None => Cost::RUN,
    };
    if scope.annotations
        && let Some(annots) = page.raw().get::<Array<'_>>(ANNOTS)
    {
        for annot in annots.iter::<Dict<'_>>() {
            if annot.get::<u32>(F).unwrap_or(0) & 2 != 0 {
                continue;
            }
            if let Some(stream) = appearance(&annot) {
                total = total.plus(walk.form(&stream, resources, 0)?);
            }
        }
    }
    budget(total).map(drop)
}

/// The refusal of content encoded as image samples.
const SAMPLES: ViewError = ViewError::TooComplex("page content is encoded as an image");

/// Whether `stream` goes through a filter that decodes image samples. hayro
/// decodes content whole before reading it, and such a stream yields
/// samples, not operators: three JBIG2 page streams declaring 100,000 ×
/// 100,000 pixels held about 1.5 GB (pdf.js bomb_giant.pdf). The app's view
/// leaves such page streams out (`app::view_contents`).
fn encodes_samples(stream: &Stream<'_>) -> bool {
    match stream.dict().get::<Object<'_>>(FILTER) {
        Some(Object::Name(name)) => is_image_filter(name.as_ref()),
        Some(Object::Array(filters)) => filters
            .iter::<Name<'_>>()
            .any(|name| is_image_filter(name.as_ref())),
        _ => false,
    }
}

/// Filters that decode image samples.
pub(crate) fn is_image_filter(name: &[u8]) -> bool {
    matches!(
        name,
        b"JBIG2Decode" | b"JPXDecode" | b"DCTDecode" | b"DCT" | b"CCITTFaxDecode" | b"CCF"
    )
}

/// What interpreting some content costs: runs, and text bytes shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Cost {
    runs: u64,
    shown: u64,
}

impl Cost {
    const RUN: Self = Self { runs: 1, shown: 0 };

    fn plus(self, other: Self) -> Self {
        Self {
            runs: self.runs.saturating_add(other.runs),
            shown: self.shown.saturating_add(other.shown),
        }
    }

    fn times(self, n: u64) -> Self {
        Self {
            runs: self.runs.saturating_mul(n),
            shown: self.shown.saturating_mul(n),
        }
    }

    fn max(self, other: Self) -> Self {
        Self {
            runs: self.runs.max(other.runs),
            shown: self.shown.max(other.shown),
        }
    }

    fn over(self) -> bool {
        self.runs > MAX_CONTENT_RUNS || self.shown > MAX_SHOWN_BYTES
    }
}

fn budget(cost: Cost) -> Result<Cost, ViewError> {
    if cost.runs > MAX_CONTENT_RUNS {
        Err(ViewError::TooComplex(
            "page content expands past the work limit",
        ))
    } else if cost.shown > MAX_SHOWN_BYTES {
        Err(ViewError::TooComplex("page shows more text than the limit"))
    } else {
        Ok(cost)
    }
}

/// The normal appearance hayro draws for an annotation.
fn appearance<'a>(annot: &Dict<'a>) -> Option<Stream<'a>> {
    match annot.get::<Dict<'_>>(AP)?.get::<Object<'_>>(N)? {
        Object::Stream(stream) => Some(stream),
        Object::Dict(states) => annot
            .get::<Name<'_>>(AS)
            .and_then(|state| states.get::<Stream<'_>>(state))
            .or_else(|| states.get::<Stream<'_>>(b"Off")),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Node {
    Object(ObjectIdentifier),
    /// A dictionary that is not an indirect object, by its position.
    Direct(usize),
}

impl Node {
    fn of_dict(dict: &Dict<'_>) -> Self {
        dict.obj_id()
            .map_or(Self::Direct(dict.data().as_ptr().addr()), Self::Object)
    }
}

struct Walk {
    scope: Scope,
    /// Nodes being walked, outermost first.
    active: Vec<Node>,
    /// Cost of a node at a nesting depth under given inherited resources.
    memo: HashMap<(Node, u32, [u128; 4]), Cost>,
}

/// Paint state the cost of a paint operation depends on.
#[derive(Clone, Copy, Default)]
struct PaintState {
    fill: Cost,
    stroke: Cost,
    /// Cost of the costliest glyph of the current Type3 font.
    glyph: Cost,
}

fn resources_key(r: &Resources<'_>) -> [u128; 4] {
    [
        r.x_objects.cache_key(),
        r.patterns.cache_key(),
        r.ext_g_states.cache_key(),
        r.fonts.cache_key(),
    ]
}

impl Walk {
    /// Runs `body` for `node` with cycle detection and memoization.
    fn enter(
        &mut self,
        node: Node,
        depth: u32,
        inherited: &Resources<'_>,
        body: impl FnOnce(&mut Self) -> Result<Cost, ViewError>,
    ) -> Result<Cost, ViewError> {
        let key = (node, depth, resources_key(inherited));
        if let Some(cost) = self.memo.get(&key) {
            return Ok(*cost);
        }
        if self.active.contains(&node) {
            return Err(ViewError::TooComplex("page content refers to itself"));
        }
        self.active.push(node);
        let cost = body(self);
        self.active.pop();
        let cost = budget(cost?)?;
        self.memo.insert(key, cost);
        Ok(cost)
    }

    /// A form XObject drawn at nesting `depth`, as hayro's `FormXObject::draw`.
    fn form(
        &mut self,
        stream: &Stream<'_>,
        parent: &Resources<'_>,
        depth: u32,
    ) -> Result<Cost, ViewError> {
        let dict = stream.dict();
        if depth >= FORM_DEPTH || dict.get::<[f32; 4]>(BBOX).is_none() {
            return Ok(Cost::default());
        }
        let own = dict.get::<Dict<'_>>(RESOURCES).map(Resources::new);
        let empty = Resources::new(Dict::default());
        let inherited = if own.is_some() { &empty } else { parent };
        if encodes_samples(stream) {
            return Err(SAMPLES);
        }
        self.enter(Node::Object(stream.obj_id()), depth, inherited, |walk| {
            let Ok(data) = stream.decoded() else {
                return Ok(Cost::default());
            };
            walk.content(&data, own.as_ref().unwrap_or(parent), depth + 1)
        })
    }

    /// Nested interpretation hayro does without a depth check.
    fn unchecked(&self, depth: u32) -> Result<(), ViewError> {
        if depth >= FORM_DEPTH {
            Err(ViewError::TooComplex("page content nests too deeply"))
        } else {
            Ok(())
        }
    }

    fn pattern(&mut self, object: Object<'_>, depth: u32) -> Result<Cost, ViewError> {
        let Object::Stream(stream) = object else {
            return Ok(Cost::default());
        };
        if stream.dict().get::<u8>(PATTERN_TYPE) != Some(1) {
            return Ok(Cost::default());
        }
        self.unchecked(depth)?;
        if encodes_samples(&stream) {
            return Err(SAMPLES);
        }
        let resources =
            Resources::new(stream.dict().get::<Dict<'_>>(RESOURCES).unwrap_or_default());
        self.enter(
            Node::Object(stream.obj_id()),
            depth,
            &Resources::new(Dict::default()),
            |walk| {
                let Ok(data) = stream.decoded() else {
                    return Ok(Cost::default());
                };
                walk.content(&data, &resources, depth + 1)
            },
        )
    }

    fn soft_mask(
        &mut self,
        gs: &Dict<'_>,
        parent: &Resources<'_>,
        depth: u32,
    ) -> Result<Cost, ViewError> {
        let Some(group) = gs
            .get::<Dict<'_>>(SMASK)
            .and_then(|m| m.get::<Stream<'_>>(G))
        else {
            return Ok(Cost::default());
        };
        self.unchecked(depth)?;
        self.form(&group, parent, depth + 1)
    }

    /// The costliest glyph of a Type3 font; every glyph procedure is walked.
    fn type3(
        &mut self,
        font: &Dict<'_>,
        parent: &Resources<'_>,
        depth: u32,
    ) -> Result<Cost, ViewError> {
        self.unchecked(depth)?;
        let own = font.get::<Dict<'_>>(RESOURCES).map(Resources::new);
        let empty = Resources::new(Dict::default());
        let inherited = if own.is_some() { &empty } else { parent };
        self.enter(Node::of_dict(font), depth, inherited, |walk| {
            let procs = font.get::<Dict<'_>>(CHAR_PROCS).unwrap_or_default();
            let mut most = Cost::default();
            for name in procs.keys() {
                let Some(proc) = procs.get::<Stream<'_>>(name.as_ref()) else {
                    continue;
                };
                if encodes_samples(&proc) {
                    return Err(SAMPLES);
                }
                let resources = proc
                    .dict()
                    .get::<Dict<'_>>(RESOURCES)
                    .map(Resources::new)
                    .or_else(|| own.clone());
                let inherited = if resources.is_some() { &empty } else { parent };
                let cost = walk.enter(Node::Object(proc.obj_id()), depth, inherited, |walk| {
                    let Ok(data) = proc.decoded() else {
                        return Ok(Cost::default());
                    };
                    walk.content(&data, resources.as_ref().unwrap_or(parent), depth + 1)
                })?;
                most = most.max(cost);
            }
            Ok(most)
        })
    }

    fn font(
        &mut self,
        font: Option<Dict<'_>>,
        resources: &Resources<'_>,
        depth: u32,
    ) -> Result<Cost, ViewError> {
        match font {
            Some(font)
                if self.scope.paint
                    && font
                        .get::<Name<'_>>(SUBTYPE)
                        .is_some_and(|n| n.as_ref() == TYPE3) =>
            {
                self.type3(&font, resources, depth)
            }
            _ => Ok(Cost::default()),
        }
    }

    /// The cost of one content stream run at `depth`, including what it invokes.
    fn content(
        &mut self,
        data: &[u8],
        resources: &Resources<'_>,
        depth: u32,
    ) -> Result<Cost, ViewError> {
        let paint = self.scope.paint;
        let mut cost = Cost::RUN;
        let mut stack = vec![PaintState::default()];
        let mut ops = TypedIter::new(data);
        while let Some(op) = ops.next() {
            let Some(state) = stack.last().copied() else {
                break;
            };
            let add = match op {
                TypedInstruction::SaveState(_) => {
                    stack.push(state);
                    Cost::default()
                }
                TypedInstruction::RestoreState(_) => {
                    if stack.len() > 1 {
                        stack.pop();
                    }
                    Cost::default()
                }
                TypedInstruction::XObject(x) => match resources.get_x_object(x.0) {
                    Some(stream)
                        if stream
                            .dict()
                            .get::<Name<'_>>(SUBTYPE)
                            .is_some_and(|n| n.as_ref() == FORM) =>
                    {
                        self.form(&stream, resources, depth)?
                    }
                    _ => Cost::default(),
                },
                TypedInstruction::NonStrokeColorNamed(c) if paint => {
                    let pattern = match c.1 {
                        Some(name) => match resources.get_pattern(name) {
                            Some(p) => self.pattern(p, depth)?,
                            None => Cost::default(),
                        },
                        None => Cost::default(),
                    };
                    if let Some(top) = stack.last_mut() {
                        top.fill = pattern;
                    }
                    Cost::default()
                }
                TypedInstruction::StrokeColorNamed(c) if paint => {
                    let pattern = match c.1 {
                        Some(name) => match resources.get_pattern(name) {
                            Some(p) => self.pattern(p, depth)?,
                            None => Cost::default(),
                        },
                        None => Cost::default(),
                    };
                    if let Some(top) = stack.last_mut() {
                        top.stroke = pattern;
                    }
                    Cost::default()
                }
                TypedInstruction::ColorSpaceNonStroke(_)
                | TypedInstruction::NonStrokeColor(_)
                | TypedInstruction::NonStrokeColorDeviceGray(_)
                | TypedInstruction::NonStrokeColorDeviceRgb(_)
                | TypedInstruction::NonStrokeColorCmyk(_) => {
                    if let Some(top) = stack.last_mut() {
                        top.fill = Cost::default();
                    }
                    Cost::default()
                }
                TypedInstruction::ColorSpaceStroke(_)
                | TypedInstruction::StrokeColor(_)
                | TypedInstruction::StrokeColorDeviceGray(_)
                | TypedInstruction::StrokeColorDeviceRgb(_)
                | TypedInstruction::StrokeColorCmyk(_) => {
                    if let Some(top) = stack.last_mut() {
                        top.stroke = Cost::default();
                    }
                    Cost::default()
                }
                TypedInstruction::SetGraphicsState(g) if paint => {
                    match resources.get_ext_g_state(g.0) {
                        Some(gs) => {
                            if let Some(font) = gs
                                .get::<Array<'_>>(FONT)
                                .and_then(|a| a.iter::<Dict<'_>>().next())
                            {
                                let glyph = self.font(Some(font), resources, depth)?;
                                if let Some(top) = stack.last_mut() {
                                    top.glyph = glyph;
                                }
                            }
                            self.soft_mask(&gs, resources, depth)?
                        }
                        None => Cost::default(),
                    }
                }
                TypedInstruction::TextFont(t) if paint => {
                    let glyph = self.font(resources.get_font(t.0), resources, depth)?;
                    if let Some(top) = stack.last_mut() {
                        top.glyph = glyph;
                    }
                    Cost::default()
                }
                TypedInstruction::ShowText(s) => glyphs(&state, s.0.as_bytes().len()),
                TypedInstruction::NextLineAndShowText(s) => glyphs(&state, s.0.as_bytes().len()),
                TypedInstruction::ShowTextWithParameters(t) => glyphs(&state, t.2.as_bytes().len()),
                TypedInstruction::ShowTexts(a) => {
                    let n =
                        a.0.iter::<Object<'_>>()
                            .map(|o| match o {
                                Object::String(s) => s.as_bytes().len(),
                                _ => 0,
                            })
                            .sum();
                    glyphs(&state, n)
                }
                TypedInstruction::FillPathNonZero(_)
                | TypedInstruction::FillPathNonZeroCompatibility(_)
                | TypedInstruction::FillPathEvenOdd(_) => state.fill,
                TypedInstruction::StrokePath(_) | TypedInstruction::CloseAndStrokePath(_) => {
                    state.stroke
                }
                TypedInstruction::FillAndStrokeNonZero(_)
                | TypedInstruction::FillAndStrokeEvenOdd(_)
                | TypedInstruction::CloseFillAndStrokeNonZero(_)
                | TypedInstruction::CloseFillAndStrokeEvenOdd(_) => state.fill.plus(state.stroke),
                _ => Cost::default(),
            };
            cost = cost.plus(add);
            if cost.over() {
                return budget(cost);
            }
        }
        Ok(cost)
    }
}

/// Glyph procedures and pattern cells painted for `n` shown bytes, and the
/// bytes themselves.
fn glyphs(state: &PaintState, n: usize) -> Cost {
    let n = u64::try_from(n).unwrap_or(u64::MAX);
    let per_glyph = state.glyph.plus(state.fill).plus(state.stroke);
    per_glyph.times(n).plus(Cost { runs: 0, shown: n })
}

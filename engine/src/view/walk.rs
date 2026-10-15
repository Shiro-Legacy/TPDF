// SPDX-License-Identifier: AGPL-3.0-or-later
//! A dry run of the content hayro would interpret for one page. hayro has no
//! cycle check and recurses into patterns and Type3 glyphs without a depth
//! limit, so a page whose content invokes itself, nests too deeply or expands
//! past a work budget is refused before interpretation.
use super::ViewError;
use hayro_interpret::CacheKey;
use hayro_syntax::content::TypedIter;
use hayro_syntax::content::ops::TypedInstruction;
use hayro_syntax::object::dict::keys::{
    ANNOTS, AP, AS, BBOX, CHAR_PROCS, F, FONT, FORM, G, N, PATTERN_TYPE, RESOURCES, SMASK, SUBTYPE,
    TYPE3,
};
use hayro_syntax::object::{Array, Dict, Name, Object, ObjectIdentifier, Stream};
use hayro_syntax::page::{Page, Resources};
use std::collections::HashMap;

/// Content-stream runs (page, forms, pattern cells, soft masks, Type3 glyphs)
/// one page may cost.
pub const MAX_CONTENT_RUNS: u64 = 1 << 20;
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
    let mut total = match page.page_stream() {
        Some(data) => walk.content(data, resources, 0)?,
        None => 1,
    };
    if scope.annotations
        && let Some(annots) = page.raw().get::<Array<'_>>(ANNOTS)
    {
        for annot in annots.iter::<Dict<'_>>() {
            if annot.get::<u32>(F).unwrap_or(0) & 2 != 0 {
                continue;
            }
            if let Some(stream) = appearance(&annot) {
                total = total.saturating_add(walk.form(&stream, resources, 0)?);
            }
        }
    }
    budget(total).map(drop)
}

fn budget(cost: u64) -> Result<u64, ViewError> {
    if cost > MAX_CONTENT_RUNS {
        Err(ViewError::TooComplex(
            "page content expands past the work limit",
        ))
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
    memo: HashMap<(Node, u32, [u128; 4]), u64>,
}

/// Paint state the cost of a paint operation depends on.
#[derive(Clone, Copy, Default)]
struct PaintState {
    fill: u64,
    stroke: u64,
    /// Cost of the costliest glyph of the current Type3 font.
    glyph: u64,
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
        body: impl FnOnce(&mut Self) -> Result<u64, ViewError>,
    ) -> Result<u64, ViewError> {
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
    ) -> Result<u64, ViewError> {
        let dict = stream.dict();
        if depth >= FORM_DEPTH || dict.get::<[f32; 4]>(BBOX).is_none() {
            return Ok(0);
        }
        let own = dict.get::<Dict<'_>>(RESOURCES).map(Resources::new);
        let empty = Resources::new(Dict::default());
        let inherited = if own.is_some() { &empty } else { parent };
        self.enter(Node::Object(stream.obj_id()), depth, inherited, |walk| {
            let Ok(data) = stream.decoded() else {
                return Ok(0);
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

    fn pattern(&mut self, object: Object<'_>, depth: u32) -> Result<u64, ViewError> {
        let Object::Stream(stream) = object else {
            return Ok(0);
        };
        if stream.dict().get::<u8>(PATTERN_TYPE) != Some(1) {
            return Ok(0);
        }
        self.unchecked(depth)?;
        let resources =
            Resources::new(stream.dict().get::<Dict<'_>>(RESOURCES).unwrap_or_default());
        self.enter(
            Node::Object(stream.obj_id()),
            depth,
            &Resources::new(Dict::default()),
            |walk| {
                let Ok(data) = stream.decoded() else {
                    return Ok(0);
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
    ) -> Result<u64, ViewError> {
        let Some(group) = gs
            .get::<Dict<'_>>(SMASK)
            .and_then(|m| m.get::<Stream<'_>>(G))
        else {
            return Ok(0);
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
    ) -> Result<u64, ViewError> {
        self.unchecked(depth)?;
        let own = font.get::<Dict<'_>>(RESOURCES).map(Resources::new);
        let empty = Resources::new(Dict::default());
        let inherited = if own.is_some() { &empty } else { parent };
        self.enter(Node::of_dict(font), depth, inherited, |walk| {
            let procs = font.get::<Dict<'_>>(CHAR_PROCS).unwrap_or_default();
            let mut most = 0;
            for name in procs.keys() {
                let Some(proc) = procs.get::<Stream<'_>>(name.as_ref()) else {
                    continue;
                };
                let resources = proc
                    .dict()
                    .get::<Dict<'_>>(RESOURCES)
                    .map(Resources::new)
                    .or_else(|| own.clone());
                let inherited = if resources.is_some() { &empty } else { parent };
                let cost = walk.enter(Node::Object(proc.obj_id()), depth, inherited, |walk| {
                    let Ok(data) = proc.decoded() else {
                        return Ok(0);
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
    ) -> Result<u64, ViewError> {
        match font {
            Some(font)
                if self.scope.paint
                    && font
                        .get::<Name<'_>>(SUBTYPE)
                        .is_some_and(|n| n.as_ref() == TYPE3) =>
            {
                self.type3(&font, resources, depth)
            }
            _ => Ok(0),
        }
    }

    /// The cost of one content stream run at `depth`, including what it invokes.
    fn content(
        &mut self,
        data: &[u8],
        resources: &Resources<'_>,
        depth: u32,
    ) -> Result<u64, ViewError> {
        let paint = self.scope.paint;
        let mut cost: u64 = 1;
        let mut stack = vec![PaintState::default()];
        let mut ops = TypedIter::new(data);
        while let Some(op) = ops.next() {
            let Some(state) = stack.last().copied() else {
                break;
            };
            let add = match op {
                TypedInstruction::SaveState(_) => {
                    stack.push(state);
                    0
                }
                TypedInstruction::RestoreState(_) => {
                    if stack.len() > 1 {
                        stack.pop();
                    }
                    0
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
                    _ => 0,
                },
                TypedInstruction::NonStrokeColorNamed(c) if paint => {
                    let pattern = match c.1 {
                        Some(name) => match resources.get_pattern(name) {
                            Some(p) => self.pattern(p, depth)?,
                            None => 0,
                        },
                        None => 0,
                    };
                    if let Some(top) = stack.last_mut() {
                        top.fill = pattern;
                    }
                    0
                }
                TypedInstruction::StrokeColorNamed(c) if paint => {
                    let pattern = match c.1 {
                        Some(name) => match resources.get_pattern(name) {
                            Some(p) => self.pattern(p, depth)?,
                            None => 0,
                        },
                        None => 0,
                    };
                    if let Some(top) = stack.last_mut() {
                        top.stroke = pattern;
                    }
                    0
                }
                TypedInstruction::ColorSpaceNonStroke(_)
                | TypedInstruction::NonStrokeColor(_)
                | TypedInstruction::NonStrokeColorDeviceGray(_)
                | TypedInstruction::NonStrokeColorDeviceRgb(_)
                | TypedInstruction::NonStrokeColorCmyk(_) => {
                    if let Some(top) = stack.last_mut() {
                        top.fill = 0;
                    }
                    0
                }
                TypedInstruction::ColorSpaceStroke(_)
                | TypedInstruction::StrokeColor(_)
                | TypedInstruction::StrokeColorDeviceGray(_)
                | TypedInstruction::StrokeColorDeviceRgb(_)
                | TypedInstruction::StrokeColorCmyk(_) => {
                    if let Some(top) = stack.last_mut() {
                        top.stroke = 0;
                    }
                    0
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
                        None => 0,
                    }
                }
                TypedInstruction::TextFont(t) if paint => {
                    let glyph = self.font(resources.get_font(t.0), resources, depth)?;
                    if let Some(top) = stack.last_mut() {
                        top.glyph = glyph;
                    }
                    0
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
                | TypedInstruction::CloseFillAndStrokeEvenOdd(_) => {
                    state.fill.saturating_add(state.stroke)
                }
                _ => 0,
            };
            cost = cost.saturating_add(add);
            if cost > MAX_CONTENT_RUNS {
                return budget(cost);
            }
        }
        Ok(cost)
    }
}

/// Glyph procedures and pattern cells painted for `n` shown bytes.
fn glyphs(state: &PaintState, n: usize) -> u64 {
    let per_glyph = state
        .glyph
        .saturating_add(state.fill)
        .saturating_add(state.stroke);
    per_glyph.saturating_mul(n as u64)
}

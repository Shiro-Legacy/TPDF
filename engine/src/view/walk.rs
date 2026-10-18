// SPDX-License-Identifier: MIT
//! A dry run of the content hayro would interpret for one page. hayro has no
//! cycle check and recurses into patterns and Type3 glyphs without a depth
//! limit, so a page whose content invokes itself, nests too deeply, expands
//! past a work budget, shows more text than the view can hold, keeps too
//! many graphics states, decodes past hayro-syntax's limit, holds or reads
//! more decoded content than the page may, or draws with fonts that decode
//! past theirs is refused before interpretation.
use super::ViewError;
use hayro_interpret::CacheKey;
use hayro_syntax::content::TypedIter;
use hayro_syntax::content::ops::TypedInstruction;
use hayro_syntax::object::dict::keys::{
    ANNOTS, AP, AS, BBOX, CHAR_PROCS, CONTENTS, DESCENDANT_FONTS, ENCODING, F, FILTER, FONT,
    FONT_DESC, FONT_FILE, FONT_FILE2, FONT_FILE3, FORM, G, N, PATTERN_TYPE, RESOURCES, SMASK,
    SUBTYPE, TO_UNICODE, TYPE3,
};
use hayro_syntax::object::stream::DecodeFailure;
use hayro_syntax::object::{Array, Dict, Name, Object, ObjectIdentifier, Stream};
use hayro_syntax::page::{Page, Resources};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, PoisonError};

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
/// Graphics states the content of one page may keep at once: `q` nesting
/// plus a state for each content run (page, form, pattern cell, glyph, soft
/// mask) along the way. hayro keeps about 2 KB a state and the text reader a
/// copy of its text parameters, so 600,000 nested `q` in a 2,806-byte file
/// held 1.2 GB. Reference viewers refuse a page past about 2,048 nested
/// graphics states.
pub const MAX_GRAPHICS_STATES: u64 = 1 << 12;
/// Elements one path may have when painted: hayro builds the whole path
/// first, about 56 bytes an element (`1 1 l` is 6 bytes of content), and
/// vello costs more again (hayro's `MAX_PATH_BYTES`).
pub const MAX_PATH_ELEMENTS: u64 = 1 << 20;
/// Decoded content bytes one page may hold at once: its own content and,
/// along each chain of runs it invokes (forms, pattern cells, glyphs, soft
/// masks, appearances), theirs. hayro decodes a run whole and holds it
/// while the runs it invokes are interpreted, so hayro-syntax's limit for
/// one stream did not bound a chain: forms of 120 MiB, each drawing the
/// next, held 128 MiB a level (1,587 bytes of file, 357 MB of WASM at two
/// levels, 613 MB at four; forms nest to 50, wasm32 aborts at 4 GiB).
pub const MAX_HELD_CONTENT_BYTES: u64 = hayro_interpret::MAX_HELD_CONTENT_BYTES as u64;
/// Content bytes one page may read, each run's counted every time it is
/// run: hayro decodes a form, pattern cell or glyph each time it draws it
/// (a form of 120 MiB drawn a thousand times is 120 GB to inflate).
pub const MAX_READ_CONTENT_BYTES: u64 = hayro_interpret::MAX_READ_CONTENT_BYTES;
/// Bytes the fonts one page selects may decode to together, their programs
/// and CMaps: hayro keeps every font of a render, its program copied whole
/// (three fonts padded to 100 MiB each held 490 MB of WASM).
pub const MAX_FONT_BYTES: u64 = 1 << 28;

/// What one walk covers: forms only (text extraction), or everything the
/// rasterizer interprets.
#[derive(Clone, Copy)]
pub(crate) struct Scope {
    pub(crate) paint: bool,
    pub(crate) annotations: bool,
}

/// What the font streams of one view decode to, by object, measured once:
/// a font program is read whole whenever a page selects its font.
#[derive(Default)]
pub(crate) struct FontSizes(Mutex<HashMap<ObjectIdentifier, u64>>);

impl FontSizes {
    /// The bytes hayro holds for `stream`: what it decodes to (copied even
    /// when hayro borrows it), nothing when it does not decode or decodes
    /// past hayro-syntax's limit (hayro does not read it then).
    fn of(&self, stream: &Stream<'_>) -> u64 {
        let lock = || self.0.lock().unwrap_or_else(PoisonError::into_inner);
        // Only an indirect stream has an identity to remember it by.
        let id = stream.dict().obj_id();
        if let Some(len) = id.and_then(|id| lock().get(&id).copied()) {
            return len;
        }
        let len = stream.decoded().map_or(0, |data| bytes(data.len()));
        if let Some(id) = id {
            lock().insert(id, len);
        }
        len
    }
}

/// The fonts one interpretation selects and what they decode to together,
/// held to [`MAX_FONT_BYTES`]: hayro keeps every font it loads with one
/// cache, its program copied whole.
#[derive(Default)]
pub(crate) struct FontBudget {
    fonts: HashSet<Node>,
    bytes: u64,
}

impl FontBudget {
    /// Counts the program and CMaps of `font` (once a font dictionary).
    /// hayro reads the first of FontFile2, FontFile3 and FontFile its font
    /// type takes; all are counted.
    pub(crate) fn select(&mut self, font: &Dict<'_>, sizes: &FontSizes) -> Result<(), ViewError> {
        if !self.fonts.insert(Node::of_dict(font)) {
            return Ok(());
        }
        let program = match font.get::<Name<'_>>(SUBTYPE) {
            Some(n) if n.as_ref() == b"Type0" => font
                .get::<Array<'_>>(DESCENDANT_FONTS)
                .and_then(|a| a.iter::<Dict<'_>>().next()),
            _ => Some(font.clone()),
        };
        let descriptor = program.and_then(|p| p.get::<Dict<'_>>(FONT_DESC));
        let streams = [
            descriptor
                .as_ref()
                .and_then(|d| d.get::<Stream<'_>>(FONT_FILE)),
            descriptor
                .as_ref()
                .and_then(|d| d.get::<Stream<'_>>(FONT_FILE2)),
            descriptor
                .as_ref()
                .and_then(|d| d.get::<Stream<'_>>(FONT_FILE3)),
            font.get::<Stream<'_>>(TO_UNICODE),
            font.get::<Stream<'_>>(ENCODING),
        ];
        for stream in streams.iter().flatten() {
            self.bytes = self.bytes.saturating_add(sizes.of(stream));
            if self.bytes > MAX_FONT_BYTES {
                return Err(FONTS_TOO_LARGE);
            }
        }
        Ok(())
    }
}

pub(crate) fn check_page(
    page: &Page<'_>,
    font_sizes: &FontSizes,
    scope: Scope,
) -> Result<(), ViewError> {
    let mut walk = Walk {
        font_sizes,
        scope,
        active: Vec::new(),
        memo: HashMap::new(),
        skipped: HashMap::new(),
        chain: 0,
        fonts: FontBudget::default(),
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
    let page_stream = page.page_stream();
    if page.content_too_large() {
        return Err(TOO_LARGE);
    }
    // hayro-syntax hands the page's content over as its own copy, kept or
    // decoded anew; it is held while everything the page invokes runs.
    let held = page_stream.as_ref().map_or(0, |data| bytes(data.len()));
    let mut total = match &page_stream {
        Some(data) => walk.content(data, held, resources, 0)?,
        None => Cost::RUN,
    };
    if scope.annotations
        && let Some(annots) = page.raw().get::<Array<'_>>(ANNOTS)
    {
        walk.chain = held;
        for annot in annots.iter::<Dict<'_>>() {
            if annot.get::<u32>(F).unwrap_or(0) & 2 != 0 {
                continue;
            }
            if let Some(stream) = appearance(&annot) {
                total = total.plus(walk.form(&stream, resources, 0)?.holding(held));
            }
        }
    }
    budget(total).map(drop)
}

/// A length as counted against the budgets.
fn bytes(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// The refusal of content encoded as image samples.
const SAMPLES: ViewError = ViewError::TooComplex("page content is encoded as an image");
/// The refusal of content that decodes past hayro-syntax's limit
/// (`MAX_DECODED_BYTES`), or that holds more than
/// [`MAX_HELD_CONTENT_BYTES`] at once: a few kilobytes of nested Flate
/// decoded to gigabytes, and hayro reads content whole.
pub(crate) const TOO_LARGE: ViewError =
    ViewError::TooComplex("page content decodes past the size limit");
/// The refusal of a page whose fonts decode past [`MAX_FONT_BYTES`].
const FONTS_TOO_LARGE: ViewError = ViewError::TooComplex("page fonts decode past the size limit");

/// Decoded content and the bytes of it a decoder allocated: hayro borrows
/// a stream without filters from the file.
struct Decoded<'a> {
    data: Cow<'a, [u8]>,
    held: u64,
}

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

/// What interpreting some content costs: runs, text bytes shown, the most
/// graphics states it keeps at once (above its caller's), the most decoded
/// content bytes it holds at once (above its caller's), and the content
/// bytes it reads, decoded each time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Cost {
    runs: u64,
    shown: u64,
    states: u64,
    held: u64,
    read: u64,
}

impl Cost {
    const RUN: Self = Self {
        runs: 1,
        shown: 0,
        states: 0,
        held: 0,
        read: 0,
    };

    /// Both, one after the other: the states and bytes held are not added.
    fn plus(self, other: Self) -> Self {
        Self {
            runs: self.runs.saturating_add(other.runs),
            shown: self.shown.saturating_add(other.shown),
            states: self.states.max(other.states),
            held: self.held.max(other.held),
            read: self.read.saturating_add(other.read),
        }
    }

    fn times(self, n: u64) -> Self {
        Self {
            runs: self.runs.saturating_mul(n),
            shown: self.shown.saturating_mul(n),
            read: self.read.saturating_mul(n),
            ..self
        }
    }

    fn max(self, other: Self) -> Self {
        Self {
            runs: self.runs.max(other.runs),
            shown: self.shown.max(other.shown),
            states: self.states.max(other.states),
            held: self.held.max(other.held),
            read: self.read.max(other.read),
        }
    }

    /// This, invoked with `held` graphics states kept.
    fn within(self, held: u64) -> Self {
        Self {
            states: self.states.saturating_add(held),
            ..self
        }
    }

    /// This, invoked while `bytes` of decoded content are held.
    fn holding(self, bytes: u64) -> Self {
        Self {
            held: self.held.saturating_add(bytes),
            ..self
        }
    }

    fn over(self) -> bool {
        self.runs > MAX_CONTENT_RUNS
            || self.shown > MAX_SHOWN_BYTES
            || self.states > MAX_GRAPHICS_STATES
            || self.held > MAX_HELD_CONTENT_BYTES
            || self.read > MAX_READ_CONTENT_BYTES
    }
}

/// The refusal of content that expands past the work limits.
pub(crate) const TOO_MUCH_WORK: ViewError =
    ViewError::TooComplex("page content expands past the work limit");

/// The refusal of a page whose content hayro refused while interpreting it
/// (`hayro_interpret::ContentRefusal`): what the walk cannot follow, such
/// as pattern cells and glyphs inherited into forms, is bounded there.
pub(crate) fn refused(why: hayro_interpret::ContentRefusal) -> ViewError {
    match why {
        hayro_interpret::ContentRefusal::Held => TOO_LARGE,
        hayro_interpret::ContentRefusal::Read => TOO_MUCH_WORK,
    }
}

fn budget(cost: Cost) -> Result<Cost, ViewError> {
    if cost.runs > MAX_CONTENT_RUNS || cost.read > MAX_READ_CONTENT_BYTES {
        Err(TOO_MUCH_WORK)
    } else if cost.held > MAX_HELD_CONTENT_BYTES {
        Err(TOO_LARGE)
    } else if cost.shown > MAX_SHOWN_BYTES {
        Err(ViewError::TooComplex("page shows more text than the limit"))
    } else if cost.states > MAX_GRAPHICS_STATES {
        Err(ViewError::TooComplex(
            "page content nests graphics states too deeply",
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

struct Walk<'v> {
    font_sizes: &'v FontSizes,
    scope: Scope,
    /// Nodes being walked, outermost first.
    active: Vec<Node>,
    /// Cost of a node at a nesting depth under given inherited resources.
    memo: HashMap<(Node, u32, [u128; 4]), Cost>,
    /// Cost of a form hayro decodes and does not draw.
    skipped: HashMap<ObjectIdentifier, Cost>,
    /// Decoded content bytes the runs being walked hold.
    chain: u64,
    /// Fonts the page selects.
    fonts: FontBudget,
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

impl Walk<'_> {
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
        // hayro saves a state, or starts one, for every run it invokes.
        let cost = budget(cost?.within(1))?;
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
        if encodes_samples(stream) {
            return Err(SAMPLES);
        }
        if depth >= FORM_DEPTH || dict.get::<[f32; 4]>(BBOX).is_none() {
            return self.skipped(stream);
        }
        let own = dict.get::<Dict<'_>>(RESOURCES).map(Resources::new);
        let empty = Resources::new(Dict::default());
        let inherited = if own.is_some() { &empty } else { parent };
        self.enter(Node::Object(stream.obj_id()), depth, inherited, |walk| {
            let Some(content) = walk.decoded(stream)? else {
                return Ok(Cost::default());
            };
            let resources = own.as_ref().unwrap_or(parent);
            walk.content(&content.data, content.held, resources, depth + 1)
        })
    }

    /// A form hayro decodes and then does not draw (it has no bounding box,
    /// or is drawn too deeply): it is read, and held a moment, each time.
    fn skipped(&mut self, stream: &Stream<'_>) -> Result<Cost, ViewError> {
        // Only an indirect stream has an identity to remember it by.
        let id = stream.dict().obj_id();
        if let Some(cost) = id.and_then(|id| self.skipped.get(&id)) {
            return Ok(*cost);
        }
        let cost = match self.decoded(stream)? {
            Some(content) => Cost {
                held: content.held,
                read: bytes(content.data.len()),
                ..Cost::default()
            },
            None => Cost::default(),
        };
        if let Some(id) = id {
            self.skipped.insert(id, cost);
        }
        Ok(cost)
    }

    /// `stream`'s decoded content: `None` when it does not decode (drawn as
    /// nothing, as hayro does), an error past the decoding limit or past
    /// what the runs being walked leave of [`MAX_HELD_CONTENT_BYTES`].
    fn decoded<'a>(&self, stream: &Stream<'a>) -> Result<Option<Decoded<'a>>, ViewError> {
        let room = MAX_HELD_CONTENT_BYTES.saturating_sub(self.chain);
        match stream.decoded_within(usize::try_from(room).unwrap_or(usize::MAX)) {
            Ok(data) => {
                let held = match &data {
                    Cow::Owned(data) => bytes(data.len()),
                    Cow::Borrowed(_) => 0,
                };
                Ok(Some(Decoded { data, held }))
            }
            Err(DecodeFailure::TooLarge) => Err(TOO_LARGE),
            Err(_) => Ok(None),
        }
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
                let Some(content) = walk.decoded(&stream)? else {
                    return Ok(Cost::default());
                };
                walk.content(&content.data, content.held, &resources, depth + 1)
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
                    let Some(content) = walk.decoded(&proc)? else {
                        return Ok(Cost::default());
                    };
                    let resources = resources.as_ref().unwrap_or(parent);
                    walk.content(&content.data, content.held, resources, depth + 1)
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
        if let Some(font) = &font {
            self.fonts.select(font, self.font_sizes)?;
        }
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

    /// The cost of one content stream run at `depth`, including what it
    /// invokes, `held` of its bytes decoded and held while they run.
    fn content(
        &mut self,
        data: &[u8],
        held: u64,
        resources: &Resources<'_>,
        depth: u32,
    ) -> Result<Cost, ViewError> {
        self.chain = self.chain.saturating_add(held);
        let cost = self.run(data, held, resources, depth);
        self.chain = self.chain.saturating_sub(held);
        cost
    }

    fn run(
        &mut self,
        data: &[u8],
        held: u64,
        resources: &Resources<'_>,
        depth: u32,
    ) -> Result<Cost, ViewError> {
        let paint = self.scope.paint;
        // hayro saves a state as each run starts.
        let mut cost = Cost {
            held,
            read: bytes(data.len()),
            ..Cost::RUN.within(1)
        };
        if cost.over() {
            return budget(cost);
        }
        let mut stack = vec![PaintState::default()];
        let mut elements = 0u64;
        let mut ops = TypedIter::new(data);
        while let Some(op) = ops.next() {
            let Some(state) = stack.last().copied() else {
                break;
            };
            if paint {
                elements = path_elements(&op, elements);
                if elements > MAX_PATH_ELEMENTS {
                    return Err(ViewError::TooComplex(
                        "page draws a path with too many segments",
                    ));
                }
            }
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
                TypedInstruction::SetGraphicsState(g) => match resources.get_ext_g_state(g.0) {
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
                        if paint {
                            self.soft_mask(&gs, resources, depth)?
                        } else {
                            Cost::default()
                        }
                    }
                    None => Cost::default(),
                },
                TypedInstruction::TextFont(t) => {
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
            let states = u64::try_from(stack.len()).unwrap_or(u64::MAX);
            cost = cost.plus(add.within(states).holding(held));
            if cost.over() {
                return budget(cost);
            }
        }
        Ok(cost)
    }
}

/// The elements of the path being built after `op`, `elements` before it.
fn path_elements(op: &TypedInstruction<'_, '_>, elements: u64) -> u64 {
    match op {
        TypedInstruction::MoveTo(_)
        | TypedInstruction::LineTo(_)
        | TypedInstruction::CubicTo(_)
        | TypedInstruction::CubicStartTo(_)
        | TypedInstruction::CubicEndTo(_)
        | TypedInstruction::ClosePath(_) => elements + 1,
        // hayro adds a rectangle as a move, three lines and a close.
        TypedInstruction::RectPath(_) => elements + 5,
        TypedInstruction::EndPath(_)
        | TypedInstruction::StrokePath(_)
        | TypedInstruction::CloseAndStrokePath(_)
        | TypedInstruction::FillPathNonZero(_)
        | TypedInstruction::FillPathNonZeroCompatibility(_)
        | TypedInstruction::FillPathEvenOdd(_)
        | TypedInstruction::FillAndStrokeNonZero(_)
        | TypedInstruction::FillAndStrokeEvenOdd(_)
        | TypedInstruction::CloseFillAndStrokeNonZero(_)
        | TypedInstruction::CloseFillAndStrokeEvenOdd(_) => 0,
        _ => elements,
    }
}

/// Glyph procedures and pattern cells painted for `n` shown bytes, and the
/// bytes themselves.
fn glyphs(state: &PaintState, n: usize) -> Cost {
    let n = u64::try_from(n).unwrap_or(u64::MAX);
    let per_glyph = state.glyph.plus(state.fill).plus(state.stroke);
    per_glyph.times(n).plus(Cost {
        shown: n,
        ..Cost::default()
    })
}

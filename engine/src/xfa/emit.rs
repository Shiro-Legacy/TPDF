// SPDX-License-Identifier: MIT
//! The materialized document: a new PDF whose pages are the laid-out
//! pages (content streams drawing every item, base 14 fonts and the fonts
//! the source embeds that the layout chose, `fonts`), whose
//! AcroForm holds a field tree built from the fields' SOM names (one
//! terminal field per SOM name, `/T` per SOM step, `DATA_KEY` on the
//! terminal) with /NeedAppearances true so the view and other readers draw
//! values, and whose catalog keeps the source's /AcroForm /XFA packets (the
//! `form` packet dropped), /Metadata, /Lang, /Extensions, /MarkInfo and
//! /ViewerPreferences, with /NeedsRendering true so Acrobat lays the form
//! out itself from the same data, and `MARKER_KEY`. The source's /Perms,
//! /Names /JavaScript, /OpenAction, /AA, /Outlines and /StructTreeRoot are
//! left out.
//!
//! Rules:
//! - Layout coordinates are y-down from the page's top-left; content and
//!   /Rect are PDF space: `y_pdf = height - y - h`.
//! - Every item is drawn inside its rectangle: edges are stroked inside the
//!   box, text is clipped to it. A positive corner radius is drawn square.
//! - A check box's mark is drawn in the content only when no widget sits on
//!   it (a fillable check box's widget shows its state through its own
//!   /AP, synthesized here so its on state is named by the on save value,
//!   its mark chosen by /MK /CA). A round check button is a circle.
//! - An image item is an image XObject (`image::prepare`), one per image
//!   value, drawn at its box's top-left corner as its `aspect` sizes it,
//!   clipped to the box; one that does not prepare draws nothing.
//! - A terminal field with one widget is merged with it; a SOM name used by
//!   several widgets (a field on a page master, a radio group) becomes one
//!   field with the widgets as kids. A name past `forms::MAX_FIELD_DEPTH`
//!   steps has its leading steps joined into one /T (dots as `_`).
//! - A piece in an embedded font shows its codes (`EmbeddedFont::encode`:
//!   bytes, or two-byte CIDs) under `/XE<n>`, the source's font object
//!   copied as it is (program, descriptor, /Widths or /W, /ToUnicode),
//!   once; without `Tz`. A text or choice field whose font a simple
//!   WinAnsi embedded font serves (`EmbeddedFonts::widget_font`) names it
//!   in /DA, and /AcroForm /DR lists it under the same name.
//! - Packet streams, /Metadata and other copied objects are copied raw,
//!   their filters kept; copies are bounded in count and depth. A datasets
//!   packet the scripts changed (`Extra::datasets`) is written anew,
//!   unfiltered, in the stored entry's place.
//! - The catalog carries `SIGNATURE_KEY` (`Extra::signature`): a fill
//!   compares the scripted state it produces with it to decide whether to
//!   lay the form out again. It also carries `DATA_HASH_KEY`, the hash
//!   (`xfa::data_hash`) of the datasets packet as written, which a reopen
//!   compares (`xfa::reopen`) to tell a fill elsewhere from our own.
//! - Bounded: pages and items by `xfa::limits`, content bytes by
//!   `MAX_CONTENT_BYTES`, the output by the app's input ceiling (the result
//!   must reopen). Never panics; refuses with `XfaError::Write` or `Limit`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;

use crate::app::MAX_PDF_BYTES;
use crate::app::edit::{Builder, Edit};
use crate::app::forms::write::{EditSink, base14_font, refresh};
use crate::app::forms::{
    COMB, COMBO, EDIT, MAX_FIELD_DEPTH, MULTILINE, PASSWORD, PUSHBUTTON, RADIO, READ_ONLY,
    rect_object,
};
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};
use crate::writer::{DocumentWriteLimits, WriteOptions, write_document};

use super::fonts::EmbeddedFonts;
use super::layout::{ItemKind, Page};
use super::packets::Packets;
use super::text::{self, Face, LaidBlock};
use super::types::{
    Aspect, Border, CheckMark, ChoiceOpen, DataPath, Edge, Field, Fill, Form, HAlign, ImageData,
    Node, Pt, Rect, Rgb, Stroke, Subform, Ui, VAlign,
};
use super::{
    CLICK_KEY, DATA_HASH_KEY, DATA_KEY, MARKER_KEY, MARKER_VERSION, SIGNATURE_KEY, XfaError,
    data_hash, limits,
};

/// Content bytes of every page together, at most.
pub const MAX_CONTENT_BYTES: usize = 256 << 20;
/// Objects one deep copy of a catalog entry or packet makes at most.
const MAX_COPIED_OBJECTS: usize = 100_000;
/// Nesting a deep copy follows at most.
const MAX_COPY_DEPTH: usize = 64;
/// The private key of a check box field holding its off save value, which
/// a fill writes into the datasets when the box is cleared.
pub const OFF_KEY: &[u8] = b"TPDFOff";
/// /Ff no-toggle-to-off, set on radio groups.
const NO_TOGGLE_TO_OFF: i64 = 1 << 14;
/// /Ff multi-select.
const MULTI_SELECT: i64 = 1 << 21;
/// Field tree nodes at most (each widget, each name step).
const MAX_TREE_NODES: usize = 4 * limits::MAX_ITEMS;

/// What `materialize_with` writes besides the layout.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extra {
    /// The datasets packet's XML to write in place of the stored stream
    /// (values the form's scripts set). Ignored when /XFA is one stream.
    pub datasets: Option<String>,
    /// The signature of the form's scripted state (`xfa::signature`),
    /// written as `SIGNATURE_KEY` in the catalog.
    pub signature: Option<u64>,
}

/// Writes the materialized document.
pub fn materialize(
    source: &Document<'_>,
    packets: &Packets,
    form: &Form,
    pages: &[Page],
) -> Result<Vec<u8>, XfaError> {
    materialize_with(source, packets, form, pages, &Extra::default())
}

/// `materialize` with `extra` written too.
pub fn materialize_with(
    source: &Document<'_>,
    packets: &Packets,
    form: &Form,
    pages: &[Page],
    extra: &Extra,
) -> Result<Vec<u8>, XfaError> {
    if pages.is_empty() {
        return Err(XfaError::Write("the layout produced no pages".into()));
    }
    if pages.len() > limits::MAX_PAGES {
        return Err(XfaError::Limit("pages"));
    }
    let items: usize = pages.iter().map(|p| p.items.len()).sum();
    if items > limits::MAX_ITEMS {
        return Err(XfaError::Limit("items"));
    }
    if packets.entries.is_empty() && packets.whole.is_none() {
        return Err(XfaError::NoXfa);
    }
    let version = source.version().max("1.7");
    let mut out = Builder::new(version).map_err(write_error)?;
    let mut fonts = Fonts::default();
    let helv = fonts.get(&mut out, Face::Helvetica)?;
    let zadb = fonts.get(&mut out, Face::ZapfDingbats)?;
    let embedded = form.fonts.as_deref();
    let mut copier = Copier {
        source,
        map: BTreeMap::new(),
        copied: 0,
    };
    let mut embedded_ids = EmbeddedIds::default();

    let mut page_ids = Vec::with_capacity(pages.len());
    for _ in pages {
        page_ids.push(out.allocate(Object::Null).map_err(write_error)?);
    }

    // Content and the widgets each page holds.
    let groups = group_data(form);
    let mut images = Images::default();
    let mut tree = Tree {
        fonts: embedded,
        ..Tree::default()
    };
    let mut content_bytes = 0usize;
    let mut contents = Vec::with_capacity(pages.len());
    for (index, page) in pages.iter().enumerate() {
        let (width, height) = page_size(page);
        let mut painter = Painter {
            out: String::new(),
            height,
            faces: BTreeSet::new(),
            fonts: embedded,
            embedded: BTreeSet::new(),
        };
        let mut xobjects: BTreeMap<Vec<u8>, ObjectId> = BTreeMap::new();
        for (at, item) in page.items.iter().enumerate() {
            match &item.kind {
                ItemKind::Widget(field) => {
                    let shown = super::picture::shown(form, field);
                    tree.add(field, &shown, item.rect, height, index, &groups)?;
                }
                ItemKind::Check {
                    border,
                    checked,
                    round,
                    mark,
                } => {
                    let widget_here = page.items.get(at + 1).is_some_and(|next| {
                        next.rect == item.rect && matches!(next.kind, ItemKind::Widget(_))
                    });
                    let mark = (*checked && !widget_here).then_some(*mark);
                    painter.check(item.rect, border, mark, *round);
                }
                ItemKind::Image(image) => {
                    if let Some(placed) = images.get(&mut out, image)? {
                        let name = format!("Im{}", placed.index);
                        painter.image(item.rect, &name, &placed, image.aspect);
                        xobjects.insert(name.into_bytes(), placed.id);
                    }
                }
                kind => painter.item(item.rect, kind),
            }
            if content_bytes.saturating_add(painter.out.len()) > MAX_CONTENT_BYTES {
                return Err(XfaError::Limit("content bytes"));
            }
        }
        content_bytes = content_bytes.saturating_add(painter.out.len());
        let mut font_resources = Dictionary::new();
        for face in &painter.faces {
            let id = fonts.get(&mut out, *face)?;
            font_resources.insert(face.resource_name().to_vec(), Object::Reference(id));
        }
        for &at in &painter.embedded {
            let id = embedded_ids.get(&mut out, &mut copier, embedded, at)?;
            font_resources.insert(
                EmbeddedFonts::resource_name(at).into_bytes(),
                Object::Reference(id),
            );
        }
        let mut xobject_resources = Dictionary::new();
        for (name, id) in xobjects {
            xobject_resources.insert(name, Object::Reference(id));
        }
        contents.push((
            width,
            height,
            painter.out.into_bytes(),
            font_resources,
            xobject_resources,
        ));
    }

    // The field tree: object numbers, then dictionaries.
    let written = tree.write(&mut out, &page_ids)?;

    for (index, (width, height, content, font_resources, xobject_resources)) in
        contents.into_iter().enumerate()
    {
        let content_id = out.allocate(Object::Null).map_err(write_error)?;
        out.put_stream_raw(content_id, Dictionary::new(), content)
            .map_err(write_error)?;
        let mut resources = Dictionary::new();
        if !font_resources.is_empty() {
            resources.insert(b"Font".to_vec(), Object::Dictionary(font_resources));
        }
        if !xobject_resources.is_empty() {
            resources.insert(b"XObject".to_vec(), Object::Dictionary(xobject_resources));
        }
        let mut dict = Dictionary::new();
        dict.insert(b"Type".to_vec(), Object::name("Page"));
        dict.insert(b"Parent".to_vec(), Object::Reference(pages_id(&out)));
        dict.insert(b"MediaBox".to_vec(), rect_object([0.0, 0.0, width, height]));
        dict.insert(b"Resources".to_vec(), Object::Dictionary(resources));
        dict.insert(b"Contents".to_vec(), Object::Reference(content_id));
        let annots: Vec<Object> = (written.annots.get(index).into_iter().flatten())
            .map(|&id| Object::Reference(id))
            .collect();
        if !annots.is_empty() {
            dict.insert(b"Annots".to_vec(), Object::Array(annots));
        }
        out.put_value(page_ids[index], Object::Dictionary(dict))
            .map_err(write_error)?;
    }
    let mut kids_dict = Dictionary::new();
    kids_dict.insert(b"Type".to_vec(), Object::name("Pages"));
    kids_dict.insert(
        b"Kids".to_vec(),
        Object::Array(page_ids.iter().map(|&id| Object::Reference(id)).collect()),
    );
    kids_dict.insert(b"Count".to_vec(), Object::Integer(page_ids.len() as i64));
    out.put_value(pages_id(&out), Object::Dictionary(kids_dict))
        .map_err(write_error)?;

    // The catalog: what is kept of the source's, the AcroForm, the marker.
    let source_root = source
        .resolve_id(source.root())
        .as_dict()
        .cloned()
        .unwrap_or_default();
    let source_acro = source
        .resolve(source.dict_get(&source_root, b"AcroForm"))
        .as_dict()
        .cloned();
    let xfa = match packets.whole {
        Some(whole) if packets.entries.is_empty() => copier.reference(&mut out, whole, 0)?,
        _ => {
            let mut array = Vec::with_capacity(packets.entries.len() * 2);
            let mut datasets = extra.datasets.as_deref();
            for (name, id) in &packets.entries {
                if name.as_slice() == b"form" {
                    continue;
                }
                array.push(Object::string(name.clone()));
                // The first datasets entry is the one `packets` read.
                let replaced = (name.as_slice() == b"datasets")
                    .then(|| datasets.take())
                    .flatten();
                array.push(match replaced {
                    Some(text) => {
                        let id = out.allocate(Object::Null).map_err(write_error)?;
                        out.put_stream_raw(id, Dictionary::new(), text.as_bytes().to_vec())
                            .map_err(write_error)?;
                        Object::Reference(id)
                    }
                    None => copier.reference(&mut out, *id, 0)?,
                });
            }
            Object::Array(array)
        }
    };
    let mut dr_fonts = Dictionary::new();
    if let Some(acro) = &source_acro {
        let dr = source.resolve(source.dict_get(acro, b"DR"));
        let font = dr
            .as_dict()
            .map(|dr| source.resolve(source.dict_get(dr, b"Font")));
        if let Some(font) = font.and_then(Object::as_dict) {
            for (key, value) in font.iter() {
                if dr_fonts.len() >= 256 {
                    break;
                }
                let copied = copier.value(&mut out, value, 1)?;
                dr_fonts.insert(key.clone(), copied);
            }
        }
    }
    for &at in &tree.widget_fonts {
        let id = embedded_ids.get(&mut out, &mut copier, embedded, at)?;
        dr_fonts.insert(
            EmbeddedFonts::resource_name(at).into_bytes(),
            Object::Reference(id),
        );
    }
    dr_fonts.insert(b"Helv".to_vec(), Object::Reference(helv));
    dr_fonts.insert(b"ZaDb".to_vec(), Object::Reference(zadb));
    let mut dr = Dictionary::new();
    dr.insert(b"Font".to_vec(), Object::Dictionary(dr_fonts));
    let mut acro = Dictionary::new();
    acro.insert(
        b"Fields".to_vec(),
        Object::Array(
            written
                .roots
                .iter()
                .map(|&id| Object::Reference(id))
                .collect(),
        ),
    );
    acro.insert(b"DR".to_vec(), Object::Dictionary(dr));
    acro.insert(b"DA".to_vec(), Object::string(b"/Helv 0 Tf 0 g".to_vec()));
    acro.insert(b"NeedAppearances".to_vec(), Object::Boolean(true));
    acro.insert(b"XFA".to_vec(), xfa);
    let acro_id = out
        .allocate(Object::Dictionary(acro))
        .map_err(write_error)?;

    let mut catalog = Dictionary::new();
    catalog.insert(b"Type".to_vec(), Object::name("Catalog"));
    catalog.insert(b"Pages".to_vec(), Object::Reference(pages_id(&out)));
    catalog.insert(b"AcroForm".to_vec(), Object::Reference(acro_id));
    for key in [
        &b"Metadata"[..],
        b"Lang",
        b"Extensions",
        b"MarkInfo",
        b"ViewerPreferences",
    ] {
        if let Some(value) = source_root.get(key) {
            if source.resolve(value).is_null() {
                continue;
            }
            let copied = copier.value(&mut out, value, 0)?;
            catalog.insert(key.to_vec(), copied);
        }
    }
    catalog.insert(b"NeedsRendering".to_vec(), Object::Boolean(true));
    catalog.insert(MARKER_KEY.to_vec(), Object::Integer(MARKER_VERSION));
    if let Some(signature) = extra.signature {
        // The bits as an integer: PDF integers are signed.
        catalog.insert(SIGNATURE_KEY.to_vec(), Object::Integer(signature as i64));
    }
    // The datasets packet as a reader reads it back: `extra`'s in an array
    // /XFA (which writes it), the stored one otherwise.
    let data_read = match packets.whole {
        Some(_) if packets.entries.is_empty() => packets.datasets.as_deref(),
        _ => extra.datasets.as_deref().or(packets.datasets.as_deref()),
    };
    catalog.insert(
        DATA_HASH_KEY.to_vec(),
        Object::Integer(data_hash(data_read) as i64),
    );
    let root = out.document().root();
    out.put_value(root, Object::Dictionary(catalog))
        .map_err(write_error)?;

    // Buttons get /AP /N states named by their on values, so a fill turns
    // on the state the datasets store.
    {
        let mut sink = EditSink::new(&mut out);
        for (widget, on) in &written.buttons {
            refresh(&mut sink, *widget, Some(on.as_slice()))
                .map_err(|e| XfaError::Write(format!("button appearance: {}", e.message)))?;
        }
    }

    let mut write_limits = DocumentWriteLimits::default();
    write_limits.syntax.max_output_bytes = MAX_PDF_BYTES;
    let bytes = write_document(&out.into_document(), write_limits, WriteOptions::APP)
        .map_err(|e| XfaError::Write(e.to_string()))?;
    if bytes.len() > MAX_PDF_BYTES {
        return Err(XfaError::Limit("output bytes"));
    }
    Ok(bytes)
}

fn write_error(error: crate::document::DocumentError) -> XfaError {
    XfaError::Write(error.to_string())
}

/// The page tree root `Builder::new` made.
fn pages_id(out: &Builder) -> ObjectId {
    let doc = out.document();
    let root = doc.resolve_id(doc.root());
    root.as_dict()
        .and_then(|d| d.get(b"Pages".as_slice()))
        .and_then(Object::as_reference)
        .unwrap_or_else(|| ObjectId::new(2, 0).unwrap_or(doc.root()))
}

fn page_size(page: &Page) -> (Pt, Pt) {
    let ok = |v: Pt| v.is_finite() && v > 0.0 && v <= 14_400.0;
    if ok(page.width) && ok(page.height) {
        (page.width, page.height)
    } else {
        (612.0, 792.0)
    }
}

// ------------------------------------------------------------------ fonts

#[derive(Default)]
struct Fonts {
    ids: BTreeMap<Face, ObjectId>,
}

impl Fonts {
    /// One font object per face per document.
    fn get(&mut self, out: &mut Builder, face: Face) -> Result<ObjectId, XfaError> {
        if let Some(id) = self.ids.get(&face) {
            return Ok(*id);
        }
        let id = out
            .allocate(Object::Dictionary(base14_font(face.base_font())))
            .map_err(write_error)?;
        self.ids.insert(face, id);
        Ok(id)
    }
}

/// The embedded fonts written so far: each source font copied once.
#[derive(Default)]
struct EmbeddedIds {
    ids: BTreeMap<usize, ObjectId>,
}

impl EmbeddedIds {
    /// The output object of embedded font `at`, copied on first use.
    fn get(
        &mut self,
        out: &mut Builder,
        copier: &mut Copier<'_, '_>,
        fonts: Option<&EmbeddedFonts>,
        at: usize,
    ) -> Result<ObjectId, XfaError> {
        if let Some(id) = self.ids.get(&at) {
            return Ok(*id);
        }
        let font = fonts
            .and_then(|f| f.get(at))
            .ok_or_else(|| XfaError::Write("an embedded font went missing".into()))?;
        let id = match copier.value(out, &font.object, 0)? {
            Object::Reference(id) => id,
            dict @ Object::Dictionary(_) => out.allocate(dict).map_err(write_error)?,
            _ => return Err(XfaError::Write("an embedded font did not copy".into())),
        };
        self.ids.insert(at, id);
        Ok(id)
    }
}

// ---------------------------------------------------------------- content

/// A PDF number: at most three decimals, no trailing zeros.
fn num(value: f64) -> String {
    if !value.is_finite() {
        return "0".into();
    }
    let rounded = (value * 1000.0).round() / 1000.0;
    let mut text = format!("{rounded:.3}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" { "0".into() } else { text }
}

fn rgb(color: Rgb) -> String {
    let c = |v: u8| num(f64::from(v) / 255.0);
    format!("{} {} {}", c(color.0), c(color.1), c(color.2))
}

/// A PDF literal string of `bytes`: `(`, `)` and `\` escaped, anything
/// outside printable ASCII as octal.
pub fn pdf_string(bytes: &[u8], out: &mut String) {
    out.push('(');
    for &b in bytes {
        match b {
            b'(' => out.push_str("\\("),
            b')' => out.push_str("\\)"),
            b'\\' => out.push_str("\\\\"),
            0x20..=0x7e => out.push(char::from(b)),
            _ => {
                let _ = write!(out, "\\{b:03o}");
            }
        }
    }
    out.push(')');
}

/// A rectangle in PDF space: left, bottom, width, height.
#[derive(Clone, Copy)]
struct Box2 {
    x: Pt,
    y: Pt,
    w: Pt,
    h: Pt,
}

struct Painter<'f> {
    out: String,
    height: Pt,
    faces: BTreeSet<Face>,
    /// The embedded fonts the layout used, and those this page shows.
    fonts: Option<&'f EmbeddedFonts>,
    embedded: BTreeSet<usize>,
}

impl Painter<'_> {
    fn pdf(&self, rect: Rect) -> Box2 {
        let w = if rect.w.is_finite() {
            rect.w.max(0.0)
        } else {
            0.0
        };
        let h = if rect.h.is_finite() {
            rect.h.max(0.0)
        } else {
            0.0
        };
        Box2 {
            x: rect.x,
            y: self.height - rect.y - h,
            w,
            h,
        }
    }

    fn item(&mut self, rect: Rect, kind: &ItemKind) {
        match kind {
            ItemKind::Border(border) | ItemKind::Rectangle(border) => {
                let b = self.pdf(rect);
                self.border(border, b);
            }
            ItemKind::Text { block, v_align } => {
                let laid = text::lay_out_with(block, Some(rect.w), self.fonts);
                self.text(rect, &laid, *v_align);
            }
            ItemKind::Line { slope_down, edge } => self.line(rect, *slope_down, edge),
            ItemKind::Arc {
                circular,
                edge,
                fill,
            } => self.arc(rect, *circular, edge, fill.as_ref()),
            ItemKind::Check {
                border,
                checked,
                round,
                mark,
            } => self.check(rect, border, checked.then_some(*mark), *round),
            ItemKind::Widget(_) | ItemKind::Image(_) => {}
        }
    }

    fn stroke_style(&mut self, edge: &Edge) {
        let dash = match edge.stroke {
            Stroke::Dashed => "[3 3] 0 d ",
            Stroke::Dotted => "[1 2] 0 d ",
            Stroke::DashDot => "[3 2 1 2] 0 d ",
            Stroke::DashDotDot => "[3 2 1 2 1 2] 0 d ",
            _ => "",
        };
        let _ = write!(
            self.out,
            "{} RG {} w {dash}",
            rgb(edge.color),
            num(edge.thickness)
        );
    }

    fn segment(&mut self, edge: &Edge, from: (Pt, Pt), to: (Pt, Pt)) {
        if !edge.drawn() {
            return;
        }
        self.out.push_str("q ");
        self.stroke_style(edge);
        let _ = writeln!(
            self.out,
            "{} {} m {} {} l S Q",
            num(from.0),
            num(from.1),
            num(to.0),
            num(to.1)
        );
    }

    fn border(&mut self, border: &Border, b: Box2) {
        if !border.presence.drawn() {
            return;
        }
        if let Some(fill) = border.fill.filter(|f| f.presence.drawn()) {
            let _ = writeln!(
                self.out,
                "q {} rg {} {} {} {} re f Q",
                rgb(fill.color),
                num(b.x),
                num(b.y),
                num(b.w),
                num(b.h)
            );
        }
        let (left, bottom, right, top) = (b.x, b.y, b.x + b.w, b.y + b.h);
        let half = |e: &Edge| e.thickness / 2.0;
        let t = *border.top();
        self.segment(&t, (left, top - half(&t)), (right, top - half(&t)));
        let r = *border.right();
        self.segment(&r, (right - half(&r), top), (right - half(&r), bottom));
        let bo = *border.bottom();
        self.segment(&bo, (right, bottom + half(&bo)), (left, bottom + half(&bo)));
        let l = *border.left();
        self.segment(&l, (left + half(&l), bottom), (left + half(&l), top));
    }

    fn line(&mut self, rect: Rect, slope_down: bool, edge: &Edge) {
        let b = self.pdf(rect);
        let (top, bottom) = (b.y + b.h, b.y);
        if slope_down {
            self.segment(edge, (b.x, top), (b.x + b.w, bottom));
        } else {
            self.segment(edge, (b.x, bottom), (b.x + b.w, top));
        }
    }

    /// An ellipse in `b` (a centred circle of the smaller side when
    /// `circular`) as four Béziers, without painting it.
    fn ellipse_path(&mut self, b: Box2, circular: bool) {
        const K: f64 = 0.552_284_75;
        let (mut rx, mut ry) = (b.w / 2.0, b.h / 2.0);
        if circular {
            let r = rx.min(ry);
            rx = r;
            ry = r;
        }
        let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
        let (kx, ky) = (rx * K, ry * K);
        let n = num;
        let _ = write!(
            self.out,
            "{} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c h ",
            n(cx + rx),
            n(cy),
            n(cx + rx),
            n(cy + ky),
            n(cx + kx),
            n(cy + ry),
            n(cx),
            n(cy + ry),
            n(cx - kx),
            n(cy + ry),
            n(cx - rx),
            n(cy + ky),
            n(cx - rx),
            n(cy),
            n(cx - rx),
            n(cy - ky),
            n(cx - kx),
            n(cy - ry),
            n(cx),
            n(cy - ry),
            n(cx + kx),
            n(cy - ry),
            n(cx + rx),
            n(cy - ky),
            n(cx + rx),
            n(cy),
        );
    }

    fn arc(&mut self, rect: Rect, circular: bool, edge: &Edge, fill: Option<&Fill>) {
        let fill = fill.filter(|f| f.presence.drawn());
        let stroke = edge.drawn();
        if fill.is_none() && !stroke {
            return;
        }
        let b = self.pdf(rect);
        self.out.push_str("q ");
        if let Some(fill) = fill {
            let _ = write!(self.out, "{} rg ", rgb(fill.color));
        }
        if stroke {
            self.stroke_style(edge);
        }
        self.ellipse_path(b, circular);
        let op = match (fill.is_some(), stroke) {
            (true, true) => "B",
            (true, false) => "f",
            _ => "S",
        };
        let _ = writeln!(self.out, "{op} Q");
    }

    /// A check button's box (a circle when `round`: its border's first
    /// drawn edge and its fill, else a thin black circle) and, when given,
    /// its mark (`mark_char`): a dot as a path, any other as its
    /// ZapfDingbats glyph.
    fn check(&mut self, rect: Rect, border: &Border, mark: Option<CheckMark>, round: bool) {
        let b = self.pdf(rect);
        if round {
            self.round_border(border, b);
        } else if border.drawn() {
            self.border(border, b);
        } else {
            let _ = writeln!(
                self.out,
                "q 0 0 0 RG 0.5 w {} {} {} {} re S Q",
                num(b.x + 0.25),
                num(b.y + 0.25),
                num((b.w - 0.5).max(0.0)),
                num((b.h - 0.5).max(0.0))
            );
        }
        let Some(mark) = mark else {
            return;
        };
        let side = b.w.min(b.h);
        if side <= 0.0 {
            return;
        }
        let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
        let glyph = mark_char(mark, round);
        if glyph == b'l' {
            // As the widget's synthesized dot: half the radius inside a
            // 1 pt border.
            let r = ((side / 2.0 - 1.0) * 0.5).max(side / 8.0);
            self.out.push_str("q 0 0 0 rg ");
            self.ellipse_path(
                Box2 {
                    x: cx - r,
                    y: cy - r,
                    w: 2.0 * r,
                    h: 2.0 * r,
                },
                true,
            );
            self.out.push_str("f Q\n");
        } else {
            // A dingbat taken as 0.85 em wide, its ink from about 0 to
            // 0.69 em above the baseline (a20, `4`, is 846 units wide).
            let size = side * 0.8;
            let x = cx - 0.846 * size / 2.0;
            let y = cy - 0.346 * size;
            self.faces.insert(Face::ZapfDingbats);
            let mut text = String::new();
            pdf_string(&[glyph], &mut text);
            let _ = writeln!(
                self.out,
                "q BT /{} {} Tf 0 0 0 rg {} {} Td {text} Tj ET Q",
                String::from_utf8_lossy(Face::ZapfDingbats.resource_name()),
                num(size),
                num(x),
                num(y)
            );
        }
    }

    /// A round check button's circle: the border's fill, then its first
    /// drawn edge stroked inside the box (3-D strokes as solid ones); a
    /// thin black circle when the border draws nothing.
    fn round_border(&mut self, border: &Border, b: Box2) {
        let edge = (border.presence.drawn())
            .then(|| border.edges.iter().find(|e| e.drawn()).copied())
            .flatten();
        let fill = border
            .fill
            .filter(|f| border.presence.drawn() && f.presence.drawn());
        if edge.is_none() && fill.is_none() {
            self.out.push_str("q 0 0 0 RG 0.5 w ");
            let inset = Box2 {
                x: b.x + 0.25,
                y: b.y + 0.25,
                w: (b.w - 0.5).max(0.0),
                h: (b.h - 0.5).max(0.0),
            };
            self.ellipse_path(inset, true);
            self.out.push_str("S Q\n");
            return;
        }
        if let Some(fill) = fill {
            let _ = write!(self.out, "q {} rg ", rgb(fill.color));
            self.ellipse_path(b, true);
            self.out.push_str("f Q\n");
        }
        if let Some(edge) = edge {
            let t = edge.thickness.min(b.w.min(b.h) / 2.0);
            self.out.push_str("q ");
            self.stroke_style(&edge);
            let inset = Box2 {
                x: b.x + t / 2.0,
                y: b.y + t / 2.0,
                w: (b.w - t).max(0.0),
                h: (b.h - t).max(0.0),
            };
            self.ellipse_path(inset, true);
            self.out.push_str("S Q\n");
        }
    }

    /// An image XObject `name` of `placed`'s natural size in `rect`, as
    /// `aspect` places it from the top-left corner, clipped to `rect`.
    fn image(&mut self, rect: Rect, name: &str, placed: &PlacedImage, aspect: Aspect) {
        let b = self.pdf(rect);
        let (w, h) = super::image::place(aspect, placed.w, placed.h, b.w, b.h);
        if !(w > 0.0 && h > 0.0 && w.is_finite() && h.is_finite()) {
            return;
        }
        let top = b.y + b.h;
        let _ = writeln!(
            self.out,
            "q {} {} {} {} re W n {} 0 0 {} {} {} cm /{name} Do Q",
            num(b.x),
            num(b.y),
            num(b.w),
            num(b.h),
            num(w),
            num(h),
            num(b.x),
            num(top - h)
        );
    }

    fn text(&mut self, rect: Rect, laid: &LaidBlock, v_align: VAlign) {
        if laid.lines.is_empty() {
            return;
        }
        let b = self.pdf(rect);
        let spare = (b.h - laid.height).max(0.0);
        let shift = match v_align {
            VAlign::Top => 0.0,
            VAlign::Middle => spare / 2.0,
            VAlign::Bottom => spare,
        };
        let top = b.y + b.h - shift;
        let _ = writeln!(
            self.out,
            "q {} {} {} {} re W n",
            num(b.x),
            num(b.y),
            num(b.w),
            num(b.h)
        );
        // Text state set so far in this `q`: character spacing and the
        // horizontal scale (`Tz` scales `Tc` too, so `Tc` is divided by it).
        let (mut spacing, mut scale) = (0.0, 1.0);
        for line in &laid.lines {
            let baseline = top - line.baseline;
            for piece in &line.pieces {
                if piece.text.is_empty() || !(piece.size.is_finite() && piece.size > 0.0) {
                    continue;
                }
                let x = b.x + piece.x;
                let embedded = piece.font.and_then(|at| Some((at, self.fonts?.get(at)?)));
                let (resource, bytes) = match embedded {
                    Some((at, font)) => {
                        self.embedded.insert(at);
                        (EmbeddedFonts::resource_name(at), font.encode(&piece.text))
                    }
                    None => {
                        self.faces.insert(piece.face);
                        let bytes = (piece.text.chars())
                            .map(|ch| text::win_ansi_code(ch).unwrap_or(b'?'))
                            .collect();
                        let name = String::from_utf8_lossy(piece.face.resource_name());
                        (name.into_owned(), bytes)
                    }
                };
                let _ = write!(
                    self.out,
                    "BT /{resource} {} Tf {} rg ",
                    num(piece.size),
                    rgb(piece.color)
                );
                let piece_scale = if piece.scale.is_finite() && piece.scale > 0.0 {
                    piece.scale
                } else {
                    1.0
                };
                if piece_scale != scale {
                    scale = piece_scale;
                    let _ = write!(self.out, "{} Tz ", num(scale * 100.0));
                    // The spacing below is written again for the new scale.
                    spacing = f64::NAN;
                }
                let wanted = if piece.letter_spacing.is_finite() {
                    piece.letter_spacing
                } else {
                    0.0
                };
                if wanted != spacing {
                    spacing = wanted;
                    let _ = write!(self.out, "{} Tc ", num(spacing / scale));
                }
                let _ = write!(self.out, "{} {} Td ", num(x), num(baseline));
                pdf_string(&bytes, &mut self.out);
                self.out.push_str(" Tj ET\n");
                let rule = |offset: Pt, out: &mut String| {
                    let thick = 0.05 * piece.size;
                    let _ = writeln!(
                        out,
                        "{} rg {} {} {} {} re f",
                        rgb(piece.color),
                        num(x),
                        num(baseline + offset - thick / 2.0),
                        num(piece.width),
                        num(thick)
                    );
                };
                if piece.underline {
                    rule(-0.1 * piece.size, &mut self.out);
                }
                if piece.line_through {
                    rule(0.3 * piece.size, &mut self.out);
                }
            }
        }
        self.out.push_str("Q\n");
    }
}

/// The ZapfDingbats character of a check button's mark (Acrobat's /MK
/// /CA): `l` (a dot) for a circle and for a round button's default, `4`
/// (a check) for a square one's.
fn mark_char(mark: CheckMark, round: bool) -> u8 {
    match mark {
        CheckMark::Default if round => b'l',
        CheckMark::Default | CheckMark::Check => b'4',
        CheckMark::Circle => b'l',
        CheckMark::Cross => b'8',
        CheckMark::Diamond => b'u',
        CheckMark::Square => b'n',
        CheckMark::Star => b'H',
    }
}

// ------------------------------------------------------------------ images

/// An image written: its object, its resource index and its natural size
/// in points.
#[derive(Clone, Copy)]
struct PlacedImage {
    id: ObjectId,
    index: usize,
    w: Pt,
    h: Pt,
}

/// The images written so far, each image value once (by its shared bytes).
#[derive(Default)]
struct Images {
    by_value: HashMap<usize, Option<PlacedImage>>,
    written: usize,
}

impl Images {
    /// `image` written into `out` (once), or `None` when it is not drawn
    /// (`image::prepare`).
    fn get(
        &mut self,
        out: &mut Builder,
        image: &std::sync::Arc<ImageData>,
    ) -> Result<Option<PlacedImage>, XfaError> {
        let key = std::sync::Arc::as_ptr(image) as usize;
        if let Some(found) = self.by_value.get(&key) {
            return Ok(*found);
        }
        let placed = super::image::prepare(image);
        let result = match placed {
            Some(placed) => {
                let dpi = if placed.dpi > 0 {
                    f64::from(placed.dpi)
                } else {
                    72.0
                };
                let (w, h) = (
                    f64::from(placed.width) * 72.0 / dpi,
                    f64::from(placed.height) * 72.0 / dpi,
                );
                let id = crate::app::ops::pages::images::store(out, placed).map_err(write_error)?;
                self.written += 1;
                Some(PlacedImage {
                    id,
                    index: self.written,
                    w,
                    h,
                })
            }
            None => None,
        };
        self.by_value.insert(key, result);
        Ok(result)
    }
}

// ------------------------------------------------------------- field tree

/// What a terminal field is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldType {
    Text,
    Check,
    Radio,
    Choice,
    /// A push button (`Field::clickable`): no value, no data path, its SOM
    /// name in `CLICK_KEY` and its caption in /TU.
    Push,
}

struct WidgetSpec {
    page: usize,
    rect: [f64; 4],
    /// A button widget's on state name and whether it is on.
    state: Option<(Vec<u8>, bool)>,
    /// A check button's mark as /MK /CA (`mark_char`).
    caption: Option<u8>,
}

struct Terminal {
    kind: FieldType,
    entries: Dictionary,
    widgets: Vec<WidgetSpec>,
}

struct TreeNode {
    name: String,
    parent: Option<usize>,
    children: Vec<usize>,
    terminal: Option<Terminal>,
}

#[derive(Default)]
struct Tree<'f> {
    nodes: Vec<TreeNode>,
    roots: Vec<usize>,
    by_name: HashMap<(Option<usize>, String), usize>,
    /// The embedded fonts a widget's /DA may name, and those named.
    fonts: Option<&'f EmbeddedFonts>,
    widget_fonts: BTreeSet<usize>,
}

/// The objects the tree became.
struct Written {
    roots: Vec<ObjectId>,
    /// Widget objects of each page, in paint order.
    annots: Vec<Vec<ObjectId>>,
    /// Button widgets and their on state names.
    buttons: Vec<(ObjectId, Vec<u8>)>,
}

/// The data path of every exclusion group, by SOM name.
fn group_data(form: &Form) -> HashMap<String, Option<DataPath>> {
    let mut out = HashMap::new();
    let mut stack: Vec<&Node> = Vec::new();
    let mut subforms: Vec<&Subform> = vec![&form.root];
    for area in &form.page_set.areas {
        stack.extend(area.statics.iter());
    }
    let mut visited = 0usize;
    while visited < limits::MAX_FORM_NODES {
        if let Some(subform) = subforms.pop() {
            stack.extend(subform.children.iter());
        } else if let Some(node) = stack.pop() {
            match node {
                Node::Subform(s) => subforms.push(s),
                Node::ExclGroup(g) => {
                    out.insert(g.som.clone(), g.data.clone());
                }
                Node::Field(_) | Node::Draw(_) => {}
            }
        } else {
            break;
        }
        visited += 1;
    }
    out
}

/// The SOM name's steps, the leading ones joined so that at most `max`
/// remain.
fn steps(som: &str, max: usize) -> Vec<String> {
    let mut parts: Vec<String> = som
        .split('.')
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().filter(|c| !c.is_control()).collect())
        .collect();
    if parts.is_empty() {
        parts.push("Field".into());
    }
    let max = max.max(1);
    if parts.len() > max {
        let joined = parts.len() - max + 1;
        let head = parts[..joined].join("_");
        let mut rest = vec![head];
        rest.extend(parts.drain(joined..));
        parts = rest;
    }
    parts
}

/// A button state name: the save value's bytes, never empty, never `Off`,
/// without null bytes.
pub(super) fn state_name(value: &str) -> Vec<u8> {
    let bytes: Vec<u8> = value.bytes().filter(|&b| b != 0).collect();
    if bytes.is_empty() || bytes == b"Off" {
        b"On".to_vec()
    } else {
        bytes
    }
}

/// The /DA of `field`: its size and colour in `/Helv` under the family's
/// `Tz`, or in the embedded font `/<embedded>` without one.
fn da(field: &Field, embedded: Option<&str>) -> Object {
    let size = if field.font.size.is_finite() && field.font.size >= 0.0 {
        field.font.size
    } else {
        0.0
    };
    let scale = text::width_scale(&field.font);
    let (font, tz) = match embedded {
        Some(name) => (name, String::new()),
        None if scale == 1.0 => ("Helv", String::new()),
        None => ("Helv", format!(" {} Tz", num(scale * 100.0))),
    };
    Object::string(
        format!("/{font} {} Tf{tz} {} rg", num(size), rgb(field.font.color)).into_bytes(),
    )
}

impl Tree<'_> {
    /// Adds a widget for `field` at `rect` (y-down) on page `page`.
    fn add(
        &mut self,
        field: &Field,
        shown: &str,
        rect: Rect,
        height: Pt,
        page: usize,
        groups: &HashMap<String, Option<DataPath>>,
    ) -> Result<(), XfaError> {
        if self.nodes.len() >= MAX_TREE_NODES {
            return Err(XfaError::Limit("fields"));
        }
        let pdf = [
            rect.x,
            height - rect.y - rect.h,
            rect.x + rect.w,
            height - rect.y,
        ];
        if pdf.iter().any(|v| !v.is_finite()) {
            return Ok(());
        }
        let radio = field.group.is_some() && field.is_check();
        let (som, max) = match &field.group {
            Some(group) if radio && !group.is_empty() => (group.as_str(), MAX_FIELD_DEPTH - 1),
            Some(_) if radio => (field.som.as_str(), MAX_FIELD_DEPTH - 1),
            _ => (field.som.as_str(), MAX_FIELD_DEPTH),
        };
        let mut path = steps(som, max);
        let kind = match &field.ui {
            Ui::CheckButton { .. } if radio => FieldType::Radio,
            Ui::CheckButton { .. } => FieldType::Check,
            Ui::ChoiceList { .. } => FieldType::Choice,
            Ui::Button { .. } => FieldType::Push,
            _ => FieldType::Text,
        };
        // Find or make the terminal, renaming the leaf when the name is
        // already a branch or a terminal of another kind.
        let mut suffix = 0usize;
        let terminal = loop {
            let (parent, leaf) = self.branch(&path)?;
            match self.by_name.get(&(parent, leaf.clone())).copied() {
                Some(at) => match &self.nodes[at].terminal {
                    Some(t) if t.kind == kind => {
                        break at;
                    }
                    _ => {}
                },
                None => {
                    let font = match kind {
                        FieldType::Text | FieldType::Choice => self
                            .fonts
                            .and_then(|fonts| fonts.widget_font(&field.font, shown)),
                        _ => None,
                    };
                    if let Some(at) = font {
                        self.widget_fonts.insert(at);
                    }
                    let name = font.map(EmbeddedFonts::resource_name);
                    let entries = entries(field, shown, kind, groups, name.as_deref());
                    break self.node(
                        parent,
                        leaf,
                        Some(Terminal {
                            kind,
                            entries,
                            widgets: Vec::new(),
                        }),
                    );
                }
            }
            suffix += 1;
            if suffix > 1000 {
                return Err(XfaError::Limit("field names"));
            }
            if let Some(last) = path.last_mut() {
                *last = format!(
                    "{}_{suffix}",
                    last.trim_end_matches(|c: char| c.is_ascii_digit() || c == '_')
                );
            }
        };
        let state = match kind {
            FieldType::Check | FieldType::Radio => {
                let on = state_name(&field.check_values().0);
                Some((on, field.checked))
            }
            _ => None,
        };
        if let Some(t) = self.nodes[terminal].terminal.as_mut() {
            if kind == FieldType::Radio && field.checked {
                if let Some((on, _)) = &state {
                    t.entries.insert(b"V".to_vec(), Object::Name(on.clone()));
                }
            }
            if kind == FieldType::Check && field.checked {
                if let Some((on, _)) = &state {
                    t.entries.insert(b"V".to_vec(), Object::Name(on.clone()));
                }
            }
            let caption = match &field.ui {
                Ui::CheckButton { shape, mark, .. } => {
                    Some(mark_char(*mark, *shape == super::types::CheckShape::Round))
                }
                _ => None,
            };
            t.widgets.push(WidgetSpec {
                page,
                rect: pdf,
                state,
                caption,
            });
        }
        Ok(())
    }

    /// The branch nodes for every step but the last, made as needed; the
    /// parent and the leaf's name.
    fn branch(&mut self, path: &[String]) -> Result<(Option<usize>, String), XfaError> {
        let Some((leaf, branch)) = path.split_last() else {
            return Err(XfaError::Write("an empty field name".into()));
        };
        let mut parent = None;
        for step in branch {
            let key = (parent, step.clone());
            let at = match self.by_name.get(&key).copied() {
                Some(at) if self.nodes[at].terminal.is_none() => at,
                Some(_) => {
                    // A terminal of that name: branch under a renamed node.
                    let renamed = (parent, format!("{step}_"));
                    match self.by_name.get(&renamed).copied() {
                        Some(at) => at,
                        None => self.node(parent, renamed.1, None),
                    }
                }
                None => self.node(parent, step.clone(), None),
            };
            parent = Some(at);
        }
        Ok((parent, leaf.clone()))
    }

    fn node(&mut self, parent: Option<usize>, name: String, terminal: Option<Terminal>) -> usize {
        let at = self.nodes.len();
        self.by_name.insert((parent, name.clone()), at);
        self.nodes.push(TreeNode {
            name,
            parent,
            children: Vec::new(),
            terminal,
        });
        match parent {
            Some(p) => self.nodes[p].children.push(at),
            None => self.roots.push(at),
        }
        at
    }

    /// Writes every node and widget.
    fn write(&self, out: &mut Builder, pages: &[ObjectId]) -> Result<Written, XfaError> {
        let mut ids = Vec::with_capacity(self.nodes.len());
        for _ in &self.nodes {
            ids.push(out.allocate(Object::Null).map_err(write_error)?);
        }
        let mut annots: Vec<Vec<ObjectId>> = vec![Vec::new(); pages.len()];
        let mut buttons = Vec::new();
        for (at, node) in self.nodes.iter().enumerate() {
            let mut dict = Dictionary::new();
            dict.insert(b"T".to_vec(), Object::text(&node.name));
            if let Some(parent) = node.parent {
                dict.insert(b"Parent".to_vec(), Object::Reference(ids[parent]));
            }
            let mut kids: Vec<Object> = node
                .children
                .iter()
                .map(|&c| Object::Reference(ids[c]))
                .collect();
            if let Some(terminal) = &node.terminal {
                for (key, value) in terminal.entries.iter() {
                    dict.insert(key.clone(), value.clone());
                }
                let merged = terminal.widgets.len() == 1 && terminal.kind != FieldType::Radio;
                for spec in &terminal.widgets {
                    let Some(&page) = pages.get(spec.page) else {
                        continue;
                    };
                    let mut widget = if merged {
                        std::mem::take(&mut dict)
                    } else {
                        Dictionary::new()
                    };
                    widget.insert(b"Type".to_vec(), Object::name("Annot"));
                    widget.insert(b"Subtype".to_vec(), Object::name("Widget"));
                    widget.insert(b"Rect".to_vec(), rect_object(spec.rect));
                    widget.insert(b"F".to_vec(), Object::Integer(4));
                    widget.insert(b"P".to_vec(), Object::Reference(page));
                    if let Some((on, checked)) = &spec.state {
                        let shown = if *checked {
                            on.clone()
                        } else {
                            b"Off".to_vec()
                        };
                        widget.insert(b"AS".to_vec(), Object::Name(shown));
                    }
                    if let Some(caption) = spec.caption {
                        let mut mk = Dictionary::new();
                        mk.insert(b"CA".to_vec(), Object::string(vec![caption]));
                        widget.insert(b"MK".to_vec(), Object::Dictionary(mk));
                    }
                    let id = if merged {
                        ids[at]
                    } else {
                        widget.insert(b"Parent".to_vec(), Object::Reference(ids[at]));
                        if let Some(value) = terminal.entries.get(b"DA".as_slice()) {
                            widget.insert(b"DA".to_vec(), value.clone());
                        }
                        let id = out.allocate(Object::Null).map_err(write_error)?;
                        kids.push(Object::Reference(id));
                        id
                    };
                    if let Some((on, _)) = &spec.state {
                        buttons.push((id, on.clone()));
                    }
                    if let Some(list) = annots.get_mut(spec.page) {
                        list.push(id);
                    }
                    if merged {
                        dict = widget;
                    } else {
                        out.put_value(id, Object::Dictionary(widget))
                            .map_err(write_error)?;
                    }
                }
            }
            if !kids.is_empty() {
                dict.insert(b"Kids".to_vec(), Object::Array(kids));
            }
            out.put_value(ids[at], Object::Dictionary(dict))
                .map_err(write_error)?;
        }
        let roots = self.roots.iter().map(|&r| ids[r]).collect();
        Ok(Written {
            roots,
            annots,
            buttons,
        })
    }
}

/// A terminal field's own entries (not its widgets'). A text field's /V
/// is `shown`, its value through its display picture.
fn entries(
    field: &Field,
    shown: &str,
    kind: FieldType,
    groups: &HashMap<String, Option<DataPath>>,
    font: Option<&str>,
) -> Dictionary {
    let mut d = Dictionary::new();
    let mut flags = 0i64;
    if field.common.read_only {
        flags |= READ_ONLY;
    }
    let mut data = field.data.clone();
    match (&field.ui, kind) {
        (_, FieldType::Push) => {
            d.insert(b"FT".to_vec(), Object::name("Btn"));
            flags |= PUSHBUTTON;
            d.insert(CLICK_KEY.to_vec(), Object::text(&field.som));
            data = None;
        }
        (_, FieldType::Radio) => {
            d.insert(b"FT".to_vec(), Object::name("Btn"));
            flags |= RADIO | NO_TOGGLE_TO_OFF;
            d.insert(b"V".to_vec(), Object::name("Off"));
            data = field
                .group
                .as_ref()
                .and_then(|g| groups.get(g).cloned().flatten());
        }
        (_, FieldType::Check) => {
            d.insert(b"FT".to_vec(), Object::name("Btn"));
            d.insert(b"V".to_vec(), Object::name("Off"));
            d.insert(OFF_KEY.to_vec(), Object::text(&field.check_values().1));
        }
        (Ui::ChoiceList { open, text_entry }, _) => {
            d.insert(b"FT".to_vec(), Object::name("Ch"));
            match open {
                ChoiceOpen::Always => {}
                ChoiceOpen::MultiSelect => flags |= MULTI_SELECT,
                _ => flags |= COMBO,
            }
            if *text_entry {
                flags |= EDIT;
            }
            let options: Vec<Object> = field
                .items
                .iter()
                .take(crate::app::ops::fields::MAX_LISTED_OPTIONS)
                .map(|item| {
                    let export = if item.save.is_empty() {
                        &item.display
                    } else {
                        &item.save
                    };
                    Object::Array(vec![Object::text(export), Object::text(&item.display)])
                })
                .collect();
            d.insert(b"Opt".to_vec(), Object::Array(options));
            let value = field.value.plain();
            if *open == ChoiceOpen::MultiSelect && value.contains('\n') {
                let values = value.split('\n').map(Object::text).collect();
                d.insert(b"V".to_vec(), Object::Array(values));
            } else {
                d.insert(b"V".to_vec(), Object::text(&value));
            }
        }
        (ui, _) => {
            d.insert(b"FT".to_vec(), Object::name("Tx"));
            match ui {
                Ui::TextEdit { multi_line, comb } => {
                    if *multi_line {
                        flags |= MULTILINE;
                    }
                    match comb {
                        Some(cells) if *cells > 0 && !*multi_line => {
                            flags |= COMB;
                            d.insert(b"MaxLen".to_vec(), Object::Integer(i64::from(*cells)));
                        }
                        _ => {}
                    }
                }
                Ui::PasswordEdit => flags |= PASSWORD,
                _ => {}
            }
            if field.max_chars > 0 && !d.contains_key(b"MaxLen".as_slice()) {
                d.insert(
                    b"MaxLen".to_vec(),
                    Object::Integer(i64::from(field.max_chars)),
                );
            }
            d.insert(b"V".to_vec(), Object::text(shown));
        }
    }
    if flags != 0 {
        d.insert(b"Ff".to_vec(), Object::Integer(flags));
    }
    d.insert(b"DA".to_vec(), da(field, font));
    let q = match field.para.h_align {
        HAlign::Center => 1,
        HAlign::Right => 2,
        HAlign::Left | HAlign::Justify => 0,
    };
    if q != 0 && matches!(kind, FieldType::Text | FieldType::Choice) {
        d.insert(b"Q".to_vec(), Object::Integer(q));
    }
    let caption = field.caption_text();
    let tip = if !field.common.tooltip.is_empty() {
        &field.common.tooltip
    } else if !field.common.speak.is_empty() || kind != FieldType::Push {
        &field.common.speak
    } else {
        &caption
    };
    if !tip.is_empty() {
        d.insert(b"TU".to_vec(), Object::text(tip));
    }
    if let Some(path) = data.filter(|p| !p.0.is_empty()) {
        d.insert(DATA_KEY.to_vec(), Object::text(&path.to_string()));
    }
    d
}

// ------------------------------------------------------------------ copies

/// Deep copies of source objects, each source object once, streams raw.
struct Copier<'s, 'd> {
    source: &'s Document<'d>,
    map: BTreeMap<ObjectId, ObjectId>,
    copied: usize,
}

impl Copier<'_, '_> {
    fn value(
        &mut self,
        out: &mut Builder,
        value: &Object,
        depth: usize,
    ) -> Result<Object, XfaError> {
        if depth > MAX_COPY_DEPTH {
            return Ok(Object::Null);
        }
        Ok(match value {
            Object::Reference(id) => self.reference(out, *id, depth + 1)?,
            Object::Array(items) => {
                let mut copy = Vec::with_capacity(items.len());
                for item in items {
                    copy.push(self.value(out, item, depth + 1)?);
                }
                Object::Array(copy)
            }
            Object::Dictionary(dict) => Object::Dictionary(self.dict(out, dict, depth + 1)?),
            other => other.clone(),
        })
    }

    fn dict(
        &mut self,
        out: &mut Builder,
        dict: &Dictionary,
        depth: usize,
    ) -> Result<Dictionary, XfaError> {
        let mut copy = Dictionary::new();
        for (key, value) in dict.iter() {
            // A /Parent or /P back into the page tree would drag the source in.
            if matches!(key.as_slice(), b"Parent" | b"P") {
                continue;
            }
            copy.insert(key.clone(), self.value(out, value, depth)?);
        }
        Ok(copy)
    }

    fn reference(
        &mut self,
        out: &mut Builder,
        id: ObjectId,
        depth: usize,
    ) -> Result<Object, XfaError> {
        if let Some(copy) = self.map.get(&id) {
            return Ok(Object::Reference(*copy));
        }
        if self.copied >= MAX_COPIED_OBJECTS {
            return Err(XfaError::Limit("copied objects"));
        }
        self.copied += 1;
        let Ok(object) = self.source.object(id) else {
            return Ok(Object::Null);
        };
        let new = out.allocate(Object::Null).map_err(write_error)?;
        self.map.insert(id, new);
        let is_stream = object.stream.is_some();
        let value = object.value.clone();
        if is_stream {
            let raw = self
                .source
                .stream_data(id)
                .map_err(write_error)?
                .map(<[u8]>::to_vec)
                .unwrap_or_default();
            let dict = match value.as_dict() {
                Some(dict) => self.dict(out, dict, depth)?,
                None => Dictionary::new(),
            };
            out.put_stream_raw(new, dict, raw).map_err(write_error)?;
        } else {
            let copy = self.value(out, &value, depth)?;
            out.put_value(new, copy).map_err(write_error)?;
        }
        Ok(Object::Reference(new))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_strings() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(1.23456), "1.235");
        assert_eq!(num(-0.0001), "0");
        assert_eq!(num(f64::NAN), "0");
        let mut s = String::new();
        pdf_string(b"a(b)\\c\n\xe9", &mut s);
        assert_eq!(s, "(a\\(b\\)\\\\c\\012\\351)");
    }

    #[test]
    fn deep_names_are_joined_at_the_top() {
        let som = "a[0].b[0].c[0].d[0].e[0].f[0].g[0].h[0].i[0].j[0]";
        let parts = steps(som, 8);
        assert_eq!(parts.len(), 8);
        assert_eq!(parts[0], "a[0]_b[0]_c[0]");
        assert_eq!(parts[7], "j[0]");
        assert_eq!(steps("", 8), vec!["Field".to_string()]);
    }

    #[test]
    fn scaled_families_draw_under_tz() {
        use super::super::types::{Font, Para};
        let calibri = Font {
            family: "Calibri".into(),
            size: 10.0,
            letter_spacing: 1.0,
            ..Font::default()
        };
        let block = text::Block {
            paragraphs: vec![text::Paragraph {
                runs: vec![
                    text::Run {
                        text: "narrow ".into(),
                        font: calibri.clone(),
                    },
                    text::Run {
                        text: "wide".into(),
                        font: Font {
                            family: "Arial".into(),
                            ..calibri.clone()
                        },
                    },
                ],
                para: Para::default(),
            }],
        };
        let laid = text::lay_out(&block, None);
        let mut painter = Painter {
            out: String::new(),
            height: 100.0,
            faces: BTreeSet::new(),
            fonts: None,
            embedded: BTreeSet::new(),
        };
        painter.text(Rect::new(0.0, 0.0, 200.0, 20.0), &laid, VAlign::Top);
        let out = painter.out;
        // Tz before the Calibri run, Tc divided by it; back to 100 after.
        let narrow = out.find("89 Tz 1.124 Tc").expect(&out);
        let wide = out.find("100 Tz 1 Tc").expect(&out);
        assert!(narrow < out.find("(narrow ) Tj").unwrap());
        assert!(narrow < wide && wide < out.find("(wide) Tj").unwrap());
        // A field's /DA carries the same scale.
        let field = Field {
            font: calibri,
            ..Field::default()
        };
        assert_eq!(
            da(&field, None).as_string().unwrap(),
            b"/Helv 10 Tf 89 Tz 0 0 0 rg"
        );
        let arial = Field::default();
        assert!(
            !da(&arial, None)
                .as_string()
                .unwrap()
                .windows(2)
                .any(|w| w == b"Tz")
        );
    }

    #[test]
    fn state_names_are_usable() {
        assert_eq!(state_name("1"), b"1");
        assert_eq!(state_name(""), b"On");
        assert_eq!(state_name("Off"), b"On");
    }
}

// SPDX-License-Identifier: AGPL-3.0-or-later
//! Appearances the view synthesizes, as MuPDF does for display only: for a
//! widget without /AP /N, and for every widget when /AcroForm
//! /NeedAppearances is true. They live in the view's private update (new
//! objects past every number the document can use, and redefined widget
//! dictionaries); saves and `snapshot_bytes` never contain them.
use std::collections::BTreeMap;

use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};

use super::appearance::{Context, Synth};
use super::write::{Sink, appearance_dict};
use super::{info, page_widgets};

/// Widgets one view synthesizes at most.
pub const MAX_WIDGETS: usize = 20_000;
/// Content bytes one view synthesizes at most.
pub const MAX_CONTENT_BYTES: usize = 32 << 20;

/// The view's new streams: object, dictionary and decoded content.
pub type Streams = BTreeMap<ObjectId, (Dictionary, Vec<u8>)>;

/// Adds the synthesized appearances to `fixes` (the view's redefined
/// objects, edited where it already redefines a widget) and returns the
/// new streams, numbered from `first_free`.
pub(crate) fn appearances(
    doc: &Document<'_>,
    fixes: &mut BTreeMap<ObjectId, Object>,
    first_free: u32,
) -> Streams {
    let context = Context::of(doc);
    let regenerate = context.need_appearances();
    let mut sink = ViewSink {
        next: first_free,
        streams: BTreeMap::new(),
        objects: Vec::new(),
        fonts: BTreeMap::new(),
        bytes: 0,
    };
    let mut done = 0usize;
    for page in info::pages(doc) {
        for widget in page_widgets(doc, page.dict) {
            let Some(id) = widget.id else {
                continue;
            };
            if done >= MAX_WIDGETS || sink.bytes >= MAX_CONTENT_BYTES {
                return sink.finish(fixes);
            }
            let normal = doc
                .dict_get(widget.dict, b"AP")
                .as_dict()
                .map(|ap| doc.dict_get(ap, b"N"));
            if !regenerate && normal.is_some_and(|normal| !normal.is_null()) {
                continue;
            }
            let Some(synth) = context.synthesize(widget.dict) else {
                continue;
            };
            let state = match &synth {
                Synth::States { on_name, .. } => Some(state(doc, widget.dict, on_name)),
                _ => None,
            };
            let Ok(ap) = appearance_dict(&mut sink, &synth) else {
                return sink.finish(fixes);
            };
            let value = match fixes.get(&id) {
                Some(value) => value.clone(),
                None => match doc.object(id) {
                    Ok(object) if object.stream.is_none() => object.value.clone(),
                    _ => continue,
                },
            };
            let mut value = value;
            if let Some(dict) = value.as_dict_mut() {
                dict.insert(b"AP".to_vec(), Object::Dictionary(ap));
                if let Some(state) = state {
                    dict.insert(b"AS".to_vec(), Object::Name(state));
                }
            }
            fixes.insert(id, value);
            done += 1;
        }
    }
    sink.finish(fixes)
}

/// The appearance state to show: the widget's own /AS name, else its on
/// state when the field's /V names it, else Off.
fn state(doc: &Document<'_>, widget: &Dictionary, on: &[u8]) -> Vec<u8> {
    if let Some(name) = doc.dict_get(widget, b"AS").as_name() {
        return name.to_vec();
    }
    match super::inherited(doc, widget, b"V").as_name() {
        Some(value) if value == on => on.to_vec(),
        _ => b"Off".to_vec(),
    }
}

struct ViewSink {
    next: u32,
    streams: Streams,
    /// New objects that are not streams (fonts).
    objects: Vec<(ObjectId, Object)>,
    fonts: BTreeMap<Vec<u8>, ObjectId>,
    bytes: usize,
}

impl ViewSink {
    /// The streams; the other new objects join the redefinitions.
    fn finish(self, fixes: &mut BTreeMap<ObjectId, Object>) -> Streams {
        fixes.extend(self.objects);
        self.streams
    }
}

impl ViewSink {
    fn allocate(&mut self) -> Result<ObjectId, ()> {
        let id = ObjectId::new(self.next, 0).map_err(|_| ())?;
        self.next = self.next.checked_add(1).ok_or(())?;
        Ok(id)
    }
}

impl Sink for ViewSink {
    type Error = ();
    fn stream(&mut self, dictionary: Dictionary, content: Vec<u8>) -> Result<ObjectId, ()> {
        let id = self.allocate()?;
        self.bytes = self.bytes.saturating_add(content.len());
        self.streams.insert(id, (dictionary, content));
        Ok(id)
    }
    fn font(&mut self, dictionary: Dictionary) -> Result<ObjectId, ()> {
        let base = (dictionary.get(b"BaseFont".as_slice()))
            .and_then(Object::as_name)
            .unwrap_or(b"")
            .to_vec();
        if let Some(id) = self.fonts.get(&base) {
            return Ok(*id);
        }
        let id = self.allocate()?;
        self.objects.push((id, Object::Dictionary(dictionary)));
        self.fonts.insert(base, id);
        Ok(id)
    }
}

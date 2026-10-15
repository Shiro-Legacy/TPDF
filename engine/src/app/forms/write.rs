// SPDX-License-Identifier: AGPL-3.0-or-later
//! Writing synthesized appearances: the streams and fonts an appearance
//! needs, allocated by a `Sink` (any `Edit` target, such as the journaled
//! document or a scratch `Builder`, or the view's private objects), and the
//! /AP dictionary that names them.
use std::collections::BTreeMap;

use crate::document::DocumentError;
use crate::object::{Dictionary, Object, ObjectId};

use super::appearance::{Context, FontSource, Form, Synth};
use crate::app::AppError;
use crate::app::edit::{Edit, Place};

/// Where new appearance objects go.
pub trait Sink {
    type Error;
    /// A new Form XObject stream with unfiltered `content`.
    fn stream(&mut self, dictionary: Dictionary, content: Vec<u8>)
    -> Result<ObjectId, Self::Error>;
    /// A new non-stream object.
    fn object(&mut self, value: Object) -> Result<ObjectId, Self::Error>;
    /// The font objects this sink made, by `FontSource` key, so one font
    /// object serves every appearance written through it.
    fn fonts(&mut self) -> &mut BTreeMap<Vec<u8>, ObjectId>;
}

/// The dictionary of a new simple base 14 font.
pub fn base14_font(base: &[u8]) -> Dictionary {
    let mut font = Dictionary::new();
    font.insert(b"Type".to_vec(), Object::name("Font"));
    font.insert(b"Subtype".to_vec(), Object::name("Type1"));
    font.insert(b"BaseFont".to_vec(), Object::name(base));
    if base != b"ZapfDingbats" && base != b"Symbol" {
        font.insert(b"Encoding".to_vec(), Object::name("WinAnsiEncoding"));
    }
    font
}

/// The font object `source` names: /DR's own, or one this sink makes once.
fn font<S: Sink>(sink: &mut S, source: &FontSource) -> Result<Object, S::Error> {
    let key = match source {
        FontSource::Existing(value) => return Ok(value.clone()),
        FontSource::New(base) => base.to_vec(),
        FontSource::Eight { family, eight } => {
            [family.base_font(), b" ", eight.suffix().as_bytes()].concat()
        }
        FontSource::Cjk(cjk) => [b"CID ", cjk.name().as_bytes()].concat(),
    };
    if let Some(id) = sink.fonts().get(&key) {
        return Ok(Object::Reference(*id));
    }
    let id = match source {
        FontSource::Existing(value) => return Ok(value.clone()),
        FontSource::New(base) => sink.object(Object::Dictionary(base14_font(base)))?,
        FontSource::Eight { family, eight } => {
            let dict = eight.font(family.base_font(), *family);
            sink.object(Object::Dictionary(dict))?
        }
        FontSource::Cjk(cjk) => {
            let descriptor = sink.object(Object::Dictionary(cjk.descriptor()))?;
            let descendant = cjk.descendant_font(Object::Reference(descriptor));
            let descendant = sink.object(Object::Dictionary(descendant))?;
            sink.object(Object::Dictionary(cjk.font(Object::Reference(descendant))))?
        }
    };
    sink.fonts().insert(key, id);
    Ok(Object::Reference(id))
}

fn form<S: Sink>(sink: &mut S, form: &Form) -> Result<Object, S::Error> {
    let mut refs = Vec::with_capacity(form.fonts.len());
    for (_, source) in &form.fonts {
        refs.push(font(sink, source)?);
    }
    let id = sink.stream(form.dictionary(&refs), form.content.clone())?;
    Ok(Object::Reference(id))
}

/// The /AP dictionary of `synth`, its streams allocated by `sink`.
pub fn appearance_dict<S: Sink>(sink: &mut S, synth: &Synth) -> Result<Dictionary, S::Error> {
    let mut ap = Dictionary::new();
    match synth {
        Synth::Single(normal) => {
            ap.insert(b"N".to_vec(), form(sink, normal)?);
        }
        Synth::States { off, on_name, on } => {
            let mut states = Dictionary::new();
            states.insert(b"Off".to_vec(), form(sink, off)?);
            states.insert(on_name.clone(), form(sink, on)?);
            ap.insert(b"N".to_vec(), Object::Dictionary(states));
        }
        Synth::Push { normal, down } => {
            ap.insert(b"N".to_vec(), form(sink, normal)?);
            ap.insert(b"D".to_vec(), form(sink, down)?);
        }
    }
    Ok(ap)
}

/// Allocations in an `Edit` target (the journaled `Transaction`, or a
/// `Builder` such as a redaction's scratch copy), one font object per font
/// for the whole operation.
pub struct EditSink<'e, E: Edit> {
    pub target: &'e mut E,
    fonts: BTreeMap<Vec<u8>, ObjectId>,
}

impl<'e, E: Edit> EditSink<'e, E> {
    pub fn new(target: &'e mut E) -> Self {
        Self {
            target,
            fonts: BTreeMap::new(),
        }
    }
}

/// The journaled document's sink.
pub type TxSink<'t, 'a> = EditSink<'t, crate::app::Transaction<'a>>;

impl<E: Edit> Sink for EditSink<'_, E> {
    type Error = DocumentError;
    fn stream(
        &mut self,
        dictionary: Dictionary,
        content: Vec<u8>,
    ) -> Result<ObjectId, Self::Error> {
        let id = self.target.allocate(Object::Null)?;
        self.target.put_stream_raw(id, dictionary, content)?;
        Ok(id)
    }
    fn object(&mut self, value: Object) -> Result<ObjectId, Self::Error> {
        self.target.allocate(value)
    }
    fn fonts(&mut self) -> &mut BTreeMap<Vec<u8>, ObjectId> {
        &mut self.fonts
    }
}

/// Regenerates the appearance of the widget object `widget` from its
/// current dictionary (engine.ts `refreshAppearance`, with MuPDF's
/// synthesis): /AP is replaced, and a button whose /AS is not a name gets
/// /AS /Off. A text the fonts cannot draw (where MuPDF embeds a font
/// program) gets no appearance instead: /AP goes and /AcroForm
/// /NeedAppearances asks viewers to draw it. Returns false, writing
/// nothing, for a widget MuPDF draws no appearance for (a signature
/// field). `on` names a button's on state when its /AP has none yet.
pub fn refresh<E: Edit>(
    sink: &mut EditSink<'_, E>,
    widget: ObjectId,
    on: Option<&[u8]>,
) -> Result<bool, AppError> {
    let (synth, named) = {
        let doc = sink.target.document();
        let Some(dict) = doc.resolve_id(widget).as_dict() else {
            return Ok(false);
        };
        let named = doc.dict_get(dict, b"AS").as_name().is_some();
        (Context::of(doc).synthesize_with(dict, on), named)
    };
    let Some(synth) = synth else {
        return Ok(false);
    };
    if synth.lossy() {
        sink.target.update(widget, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.remove(b"AP".as_slice());
            }
        })?;
        need_appearances(sink.target)?;
        return Ok(true);
    }
    let ap = appearance_dict(sink, &synth)?;
    let states = matches!(synth, Synth::States { .. });
    sink.target.update(widget, |value| {
        if let Some(dict) = value.as_dict_mut() {
            dict.insert(b"AP".to_vec(), Object::Dictionary(ap));
            if states && !named {
                dict.insert(b"AS".to_vec(), Object::name("Off"));
            }
        }
    })?;
    Ok(true)
}

/// Sets /AcroForm /NeedAppearances true (where the catalog keeps the
/// /AcroForm dictionary); nothing without one.
pub fn need_appearances<E: Edit>(target: &mut E) -> Result<(), AppError> {
    let doc = target.document();
    let (Some(catalog), _) = doc.resolve_id_with_holder(doc.root()) else {
        return Ok(());
    };
    let acro = Place::object(catalog).child(doc, b"AcroForm");
    let Some(dict) = acro.read(doc).as_dict() else {
        return Ok(());
    };
    if doc.dict_get(dict, b"NeedAppearances").as_bool() == Some(true) {
        return Ok(());
    }
    acro.update(target, |value| {
        if let Some(dict) = value.as_dict_mut() {
            dict.insert(b"NeedAppearances".to_vec(), Object::Boolean(true));
        }
    })?;
    Ok(())
}

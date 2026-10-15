// SPDX-License-Identifier: AGPL-3.0-or-later
//! Writing synthesized appearances: the streams and fonts an appearance
//! needs, allocated by a `Sink` (the journaled document, or the view's
//! private objects), and the /AP dictionary that names them.
use std::collections::BTreeMap;

use crate::document::DocumentError;
use crate::object::{Dictionary, Object, ObjectId};

use super::appearance::{Context, FontSource, Form, Synth};
use crate::app::{AppError, Transaction};

/// Where new appearance objects go.
pub trait Sink {
    type Error;
    /// A new Form XObject stream.
    fn stream(&mut self, dictionary: Dictionary, content: Vec<u8>)
    -> Result<ObjectId, Self::Error>;
    /// A new font object (the sink may share one per dictionary).
    fn font(&mut self, dictionary: Dictionary) -> Result<ObjectId, Self::Error>;
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

fn form<S: Sink>(sink: &mut S, form: &Form) -> Result<Object, S::Error> {
    let mut refs = Vec::with_capacity(form.fonts.len());
    for (_, source) in &form.fonts {
        refs.push(match source {
            FontSource::Existing(value) => value.clone(),
            FontSource::New(base) => Object::Reference(sink.font(base14_font(base))?),
        });
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

/// Allocations in the journaled document, one font object per dictionary
/// for the whole operation.
pub struct TxSink<'t, 'a> {
    pub tx: &'t mut Transaction<'a>,
    fonts: BTreeMap<Vec<u8>, ObjectId>,
}

impl<'t, 'a> TxSink<'t, 'a> {
    pub fn new(tx: &'t mut Transaction<'a>) -> Self {
        Self {
            tx,
            fonts: BTreeMap::new(),
        }
    }
}

impl Sink for TxSink<'_, '_> {
    type Error = DocumentError;
    fn stream(
        &mut self,
        dictionary: Dictionary,
        content: Vec<u8>,
    ) -> Result<ObjectId, Self::Error> {
        self.tx
            .allocate(Object::Dictionary(dictionary), Some(content))
    }
    fn font(&mut self, dictionary: Dictionary) -> Result<ObjectId, Self::Error> {
        let base = (dictionary.get(b"BaseFont".as_slice()))
            .and_then(Object::as_name)
            .unwrap_or(b"")
            .to_vec();
        if let Some(id) = self.fonts.get(&base) {
            return Ok(*id);
        }
        let id = self.tx.allocate(Object::Dictionary(dictionary), None)?;
        self.fonts.insert(base, id);
        Ok(id)
    }
}

/// Regenerates the appearance of the widget object `widget` from its
/// current dictionary (engine.ts `refreshAppearance`, with MuPDF's
/// synthesis): /AP is replaced, and a button whose /AS is not a name gets
/// /AS /Off. Returns false, writing nothing, for a widget MuPDF draws no
/// appearance for (a signature field). `on` names a button's on state
/// when its /AP has none yet.
pub fn refresh(
    sink: &mut TxSink<'_, '_>,
    widget: ObjectId,
    on: Option<&[u8]>,
) -> Result<bool, AppError> {
    let (synth, named) = {
        let doc = sink.tx.document();
        let Some(dict) = doc.resolve_id(widget).as_dict() else {
            return Ok(false);
        };
        let named = doc.dict_get(dict, b"AS").as_name().is_some();
        (Context::of(doc).synthesize_with(dict, on), named)
    };
    let Some(synth) = synth else {
        return Ok(false);
    };
    let ap = appearance_dict(sink, &synth)?;
    let states = matches!(synth, Synth::States { .. });
    sink.tx.update(widget, |value| {
        if let Some(dict) = value.as_dict_mut() {
            dict.insert(b"AP".to_vec(), Object::Dictionary(ap));
            if states && !named {
                dict.insert(b"AS".to_vec(), Object::name("Off"));
            }
        }
    })?;
    Ok(true)
}

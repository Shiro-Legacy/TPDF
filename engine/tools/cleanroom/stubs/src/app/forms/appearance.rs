use std::marker::PhantomData;

use crate::document::Document;
use crate::object::{Dictionary, Object};

use super::scripts::{Cjk, Eight};
use super::text::Family;

pub const MAX_DRAWN_CHARS: usize = 65_536;

#[derive(Clone, Debug, PartialEq)]
pub enum FontSource {
    Existing(Object),
    New(&'static [u8]),
    Eight { family: Family, eight: Eight },
    Cjk(Cjk),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    pub width: f64,
    pub height: f64,
    pub rotate: i64,
    pub content: Vec<u8>,
    pub fonts: Vec<(Vec<u8>, FontSource)>,
    pub lossy: bool,
}

impl Form {
    pub fn dictionary(&self, _font_refs: &[Object]) -> Dictionary {
        todo!("clean-room: spec 07")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Synth {
    Single(Form),
    States {
        off: Form,
        on_name: Vec<u8>,
        on: Form,
    },
    Push {
        normal: Form,
        down: Form,
    },
}

impl Synth {
    pub fn forms(&self) -> Vec<&Form> {
        todo!("clean-room: spec 07")
    }

    pub fn lossy(&self) -> bool {
        todo!("clean-room: spec 07")
    }
}

pub struct Context<'d, 'a> {
    _doc: PhantomData<&'d Document<'a>>,
}

impl<'d, 'a> Context<'d, 'a> {
    pub fn of(_doc: &'d Document<'a>) -> Self {
        todo!("clean-room: spec 07")
    }

    pub fn work(&self) -> usize {
        todo!("clean-room: spec 07")
    }

    pub fn need_appearances(&self) -> bool {
        todo!("clean-room: spec 07")
    }

    pub fn synthesize(&self, _widget: &Dictionary) -> Option<Synth> {
        todo!("clean-room: spec 07")
    }

    pub fn synthesize_with(&self, _widget: &Dictionary, _on: Option<&[u8]>) -> Option<Synth> {
        todo!("clean-room: spec 07")
    }
}

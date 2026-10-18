use crate::object::{Dictionary, Object};

use super::text::{Family, Metrics};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cjk {
    Gb1,
    Cns1,
    Japan1,
    Korea1,
}

impl Cjk {
    pub fn name(self) -> &'static str {
        todo!("clean-room: spec 07")
    }

    pub fn font(self, _descendant: Object) -> Dictionary {
        todo!("clean-room: spec 07")
    }

    pub fn descendant_font(self, _descriptor: Object) -> Dictionary {
        todo!("clean-room: spec 07")
    }

    pub fn descriptor(self) -> Dictionary {
        todo!("clean-room: spec 07")
    }
}

pub fn han_font(_lang: Option<&[u8]>) -> Cjk {
    todo!("clean-room: spec 07")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eight {
    Greek,
    Cyrillic,
    Latin,
}

impl Eight {
    pub fn suffix(self) -> &'static str {
        todo!("clean-room: spec 07")
    }

    pub(crate) fn code(self, _c: char) -> Option<u8> {
        todo!("clean-room: spec 07")
    }

    pub fn widths(self, _family: Family) -> [f64; 256] {
        todo!("clean-room: spec 07")
    }

    pub fn font(self, _base: &[u8], _family: Family) -> Dictionary {
        todo!("clean-room: spec 07")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    Latin,
    Eight(Eight),
    Cjk(Cjk),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Glyph {
    pub slot: Slot,
    pub code: u32,
    pub advance: f64,
    pub ch: char,
}

pub(crate) struct Fonts<'m> {
    pub latin: &'m Metrics,
    pub family: Family,
    pub han: Cjk,
    pub latin_extended: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Line {
    pub glyphs: Vec<Glyph>,
    pub width: f64,
}

pub(crate) fn encode(_text: &str, _fonts: &Fonts<'_>) -> (Vec<Glyph>, bool) {
    todo!("clean-room: spec 07")
}

pub(crate) fn lines(
    _text: &str,
    _fonts: &Fonts<'_>,
    _size: f64,
    _max_width: f64,
    _limit: usize,
) -> (Vec<Line>, bool) {
    todo!("clean-room: spec 07")
}

pub(crate) fn string(_run: &[Glyph]) -> String {
    todo!("clean-room: spec 07")
}

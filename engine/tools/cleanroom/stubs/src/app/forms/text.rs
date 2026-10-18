use crate::document::Document;
use crate::object::{Dictionary, Object};

use super::scripts::Glyph;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Helvetica,
    HelveticaBold,
    Times,
    Courier,
    Symbol,
    ZapfDingbats,
}

impl Family {
    pub(crate) fn of(_base: &[u8]) -> Self {
        todo!("clean-room: spec 07")
    }

    pub(crate) fn base_font(self) -> &'static [u8] {
        todo!("clean-room: spec 07")
    }

    pub(crate) fn symbolic(self) -> bool {
        todo!("clean-room: spec 07")
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Metrics {}

impl Metrics {
    pub(crate) fn family(_family: Family) -> Self {
        todo!("clean-room: spec 07")
    }

    pub(crate) fn font(_doc: &Document<'_>, _font: &Dictionary) -> Self {
        todo!("clean-room: spec 07")
    }

    pub(crate) fn from_widths(_widths: Vec<f64>) -> Self {
        todo!("clean-room: spec 07")
    }

    pub(crate) fn advance(&self, _byte: u8) -> f64 {
        todo!("clean-room: spec 07")
    }
}

pub(crate) fn win_ansi_code(_c: char) -> Option<u8> {
    todo!("clean-room: spec 07")
}

pub(crate) fn win_ansi(_text: &str) -> Vec<u8> {
    todo!("clean-room: spec 07")
}

pub(crate) fn advance(_glyphs: &[Glyph]) -> f64 {
    todo!("clean-room: spec 07")
}

pub(crate) fn literal(_bytes: &[u8]) -> String {
    todo!("clean-room: spec 07")
}

pub(crate) fn font_dict<'d>(_doc: &'d Document<'_>, _value: &'d Object) -> Option<&'d Dictionary> {
    todo!("clean-room: spec 07")
}

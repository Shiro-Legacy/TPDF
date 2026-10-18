use super::{ViewDocument, ViewError};
use std::marker::PhantomData;

pub const MAX_SEARCH_QUADS: usize = usize::MAX;

pub const MAX_PAGE_CHARS: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq)]
pub struct TextFont {
    pub name: String,
    pub ascender: f64,
    pub descender: f64,
    pub bold: bool,
    pub italic: bool,
    pub serif: bool,
    pub monospace: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextChar {
    pub c: char,
    pub origin: [f64; 2],
    pub quad: [f64; 8],
    pub size: f64,
    pub font: usize,
    pub synthetic: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextLine {
    pub bbox: [f64; 4],
    pub dir: [f64; 2],
    pub chars: Vec<TextChar>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextBlock {
    pub bbox: [f64; 4],
    pub lines: Vec<TextLine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineSummary {
    pub text: String,
    pub x: i32,
    pub y: i32,
    pub top: i32,
    pub w: i32,
    pub h: i32,
    pub size: i32,
    pub family: &'static str,
    pub weight: &'static str,
    pub style: &'static str,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchResult {
    pub hits: Vec<Vec<[f64; 8]>>,
    pub truncated: bool,
}

#[derive(Clone, Debug)]
pub struct StructuredText {}

impl ViewDocument {
    pub fn structured_text(&self, _page: usize) -> Result<StructuredText, ViewError> {
        todo!("clean-room: spec 04")
    }

    pub fn text_reader(&self) -> TextReader<'_> {
        todo!("clean-room: spec 04")
    }
}

pub struct TextReader<'a> {
    _view: PhantomData<&'a ViewDocument>,
}

impl TextReader<'_> {
    pub fn structured_text(&self, _page: usize) -> Result<StructuredText, ViewError> {
        todo!("clean-room: spec 04")
    }
}

impl StructuredText {
    pub fn blocks(&self) -> &[TextBlock] {
        todo!("clean-room: spec 04")
    }

    pub fn span_blocks(&self) -> &[TextBlock] {
        todo!("clean-room: spec 04")
    }

    pub fn unclipped_blocks(&self) -> &[TextBlock] {
        todo!("clean-room: spec 04")
    }

    pub fn fonts(&self) -> &[TextFont] {
        todo!("clean-room: spec 04")
    }

    pub fn as_text(&self) -> String {
        todo!("clean-room: spec 04")
    }

    pub fn spans_text(&self) -> String {
        todo!("clean-room: spec 04")
    }

    pub fn line_summaries(&self) -> Vec<LineSummary> {
        todo!("clean-room: spec 04")
    }

    pub fn search(&self, _needle: &str, _max_quads: usize) -> SearchResult {
        todo!("clean-room: spec 04")
    }

    pub fn search_hits(&self, _needle: &str, _max_hits: usize) -> SearchResult {
        todo!("clean-room: spec 04")
    }
}

pub(crate) fn text_font_name(_base: &[u8]) -> String {
    todo!("clean-room: spec 04")
}

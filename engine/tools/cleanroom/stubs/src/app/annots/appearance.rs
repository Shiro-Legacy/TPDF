use crate::object::{Dictionary, Object, ObjectId};

use super::fonts::Base14;
use super::{Color, Point, Quad};
use crate::app::forms::scripts::Cjk;

pub const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    pub bbox: [f64; 4],
    pub matrix: [f64; 6],
    pub resources: Dictionary,
    pub content: Vec<u8>,
}

impl Form {
    pub fn dictionary(&self) -> Dictionary {
        todo!("clean-room: spec 06")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub stroke: Option<Color>,
    pub fill: Option<Color>,
    pub width: f64,
    pub opacity: f64,
    pub dash: Vec<f64>,
}

impl Default for Style {
    fn default() -> Self {
        todo!("clean-room: spec 06")
    }
}

pub fn number(_value: f64) -> Object {
    todo!("clean-room: spec 06")
}

pub fn numbers(_values: &[f64]) -> Object {
    todo!("clean-room: spec 06")
}

pub fn bounds(_points: impl IntoIterator<Item = Point>) -> Option<[f64; 4]> {
    todo!("clean-room: spec 06")
}

pub fn grow(_rect: [f64; 4], _by: f64) -> [f64; 4] {
    todo!("clean-room: spec 06")
}

pub fn note(_icon: &[u8], _color: Option<&Color>, _alpha: f64) -> Form {
    todo!("clean-room: spec 06")
}

pub fn highlight(_quads: &[Quad], _color: Option<&Color>, _alpha: f64) -> (Form, [f64; 4]) {
    todo!("clean-room: spec 06")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Underline,
    StrikeOut,
    Squiggly,
}

pub fn text_line(
    _mark: Mark,
    _quads: &[Quad],
    _color: Option<&Color>,
    _alpha: f64,
) -> (Form, [f64; 4]) {
    todo!("clean-room: spec 06")
}

pub fn border_allowance(_width: f64) -> f64 {
    todo!("clean-room: spec 06")
}

pub fn rectangle(_inner: [f64; 4], _bbox: [f64; 4], _ellipse: bool, _style: &Style) -> Form {
    todo!("clean-room: spec 06")
}

pub fn line(_a: Point, _b: Point, _endings: [&[u8]; 2], _style: &Style) -> (Form, [f64; 4]) {
    todo!("clean-room: spec 06")
}

pub fn poly(_vertices: &[Point], _closed: bool, _style: &Style) -> (Form, [f64; 4]) {
    todo!("clean-room: spec 06")
}

pub fn ink_allowance(_width: f64) -> f64 {
    todo!("clean-room: spec 06")
}

pub fn ink(_strokes: &[Vec<Point>], _style: &Style) -> (Form, [f64; 4]) {
    todo!("clean-room: spec 06")
}

pub fn caret(_color: Option<&Color>, _alpha: f64) -> Form {
    todo!("clean-room: spec 06")
}

pub fn icon_fit(_rect: [f64; 4]) -> [f64; 4] {
    todo!("clean-room: spec 06")
}

pub fn stamp(_name: &[u8], _color: Option<&Color>, _alpha: f64, _matrix: [f64; 6]) -> Form {
    todo!("clean-room: spec 06")
}

pub fn image_stamp(_image: ObjectId, _placement: [f64; 6], _matrix: [f64; 6]) -> Form {
    todo!("clean-room: spec 06")
}

pub fn redact(_rect: [f64; 4], _quads: &[Quad], _color: Option<&Color>) -> (Form, [f64; 4]) {
    todo!("clean-room: spec 06")
}

#[derive(Clone, Debug, PartialEq)]
pub struct FreeText<'a> {
    pub text: &'a str,
    pub font_name: &'a [u8],
    pub font: Base14,
    pub han: Cjk,
    pub size: f64,
    pub color: Option<Color>,
    pub border: f64,
    pub dash: Vec<f64>,
    pub background: Option<Color>,
    pub quadding: i64,
    pub rotate: i32,
    pub opacity: f64,
}

pub const MAX_FREE_TEXT_CHARS: usize = 65_536;

pub fn line_height(_size: f64) -> f64 {
    todo!("clean-room: spec 06")
}

pub fn free_text(_bbox: [f64; 4], _rect: [f64; 4], _box_: &FreeText<'_>) -> Form {
    todo!("clean-room: spec 06")
}

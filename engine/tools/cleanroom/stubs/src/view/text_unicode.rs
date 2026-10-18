use hayro_interpret::hayro_cmap::CMap;
use hayro_syntax::object::Dict;
use std::collections::HashMap;
use std::rc::Rc;

pub(super) type CMaps = HashMap<Vec<u8>, Option<Rc<CMap>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Code {
    pub code: u32,
    pub cid: u32,
    pub bytes: u8,
}

pub(super) struct FontText {}

impl FontText {
    pub(super) fn new(_font: &Dict<'_>, _program: Option<&[u8]>, _cmaps: &mut CMaps) -> Self {
        todo!("clean-room: spec 05")
    }

    pub(super) fn codes(&self, _shown: &[u8], _out: &mut Vec<Code>) {
        todo!("clean-room: spec 05")
    }

    pub(super) fn text(&self, _code: Code) -> Option<Vec<char>> {
        todo!("clean-room: spec 05")
    }

    pub(super) fn vertical(&self) -> bool {
        todo!("clean-room: spec 05")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BidiClass {
    L,
    R,
    AL,
    EN,
    ES,
    ET,
    AN,
    CS,
    NSM,
    BN,
    B,
    S,
    WS,
    ON,
    LRE,
    LRO,
    RLE,
    RLO,
    PDF,
    LRI,
    RLI,
    FSI,
    PDI,
}

pub(super) fn encoding_names(_font: &Dict<'_>) -> Vec<Option<Vec<u8>>> {
    todo!("clean-room: spec 05")
}

pub(super) fn glyph_name(_name: &[u8], _dingbats: bool) -> Option<Vec<char>> {
    todo!("clean-room: spec 05")
}

pub(super) fn normalise_extracted(_c: char, _out: &mut Vec<char>) {
    todo!("clean-room: spec 05")
}

pub(super) fn bidi_class(_c: char) -> BidiClass {
    todo!("clean-room: spec 05")
}

pub(super) fn bidi_mirror(_c: char) -> Option<char> {
    todo!("clean-room: spec 05")
}

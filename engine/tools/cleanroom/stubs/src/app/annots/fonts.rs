use crate::object::Dictionary;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base14 {
    Helvetica,
    HelveticaBold,
    HelveticaOblique,
    HelveticaBoldOblique,
    TimesRoman,
    TimesBold,
    TimesItalic,
    TimesBoldItalic,
    Courier,
    CourierBold,
    CourierOblique,
    CourierBoldOblique,
}

impl Base14 {
    pub fn from_resource_name(_name: &[u8]) -> Self {
        todo!("clean-room: spec 06")
    }

    pub fn base_font(self) -> &'static [u8] {
        todo!("clean-room: spec 06")
    }

    pub fn advance(self, _byte: u8) -> f64 {
        todo!("clean-room: spec 06")
    }

    pub fn width(self, _bytes: &[u8], _size: f64) -> f64 {
        todo!("clean-room: spec 06")
    }

    pub fn dictionary(self) -> Dictionary {
        todo!("clean-room: spec 06")
    }
}

pub fn win_ansi(_c: char) -> Option<u8> {
    todo!("clean-room: spec 06")
}

pub fn encode(_text: &str) -> Vec<u8> {
    todo!("clean-room: spec 06")
}

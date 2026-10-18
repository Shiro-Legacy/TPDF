#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Exif {
    pub dpi: Option<u32>,
    pub orientation: Option<u8>,
}

pub(crate) fn exif(_payload: &[u8]) -> Exif {
    todo!("clean-room: spec 08")
}

pub(crate) fn photoshop_dpi(_payload: &[u8]) -> Option<u32> {
    todo!("clean-room: spec 08")
}

pub(crate) fn jfif_dpi(_payload: &[u8]) -> Option<u32> {
    todo!("clean-room: spec 08")
}

pub(crate) fn png_dpi(_pixels_per_unit_x: u32, _unit_is_metre: bool) -> Option<u32> {
    todo!("clean-room: spec 08")
}

pub(crate) fn jpeg_dpi(_exif: Option<u32>, _photoshop: Option<u32>, _jfif: Option<u32>) -> u32 {
    todo!("clean-room: spec 08")
}

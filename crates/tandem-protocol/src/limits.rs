//! The request bounds and fixed sizes the renderer shares with the engine.
//!
//! Mirrors the constants of `src/renderer/limits.ts` (the document-size
//! ceilings) and `src/renderer/engine-shared.ts` (request bounds and fixed
//! sizes). Byte sizes, counts and character bounds are `usize`; scales and
//! point sizes are `f64`.

// ---------------------------------------------------------------- limits.ts

/// The open ceiling for a document, shared by the picker, the save path and the engine.
pub const MAX_PDF_BYTES: usize = 120 * 1024 * 1024;
/// Save As ceiling, lower than the open ceiling: saving is the expensive direction.
pub const MAX_SAVE_BYTES: usize = 50 * 1024 * 1024;
/// Redaction holds several copies of the document, so apply fails closed above 12 MiB.
pub const MAX_REDACT_SOURCE_BYTES: usize = 12 * 1024 * 1024;
/// The largest signature `/Contents` (in bytes, before hex encoding).
pub const MAX_SIG_CONTENTS_BYTES: usize = 8192;
/// The longest text `text-add` takes, in characters.
pub const MAX_TEXT_ADD_CHARS: usize = 4096;
/// The most lines `text-add` takes.
pub const MAX_TEXT_ADD_LINES: usize = 200;

// ---------------------------------------------------------- engine-shared.ts

/// The longest rendered pixel edge.
pub const MAX_PIXEL_EDGE: usize = 4096;
/// The smallest render scale.
pub const MIN_SCALE: f64 = 0.25;
/// Thumbnail long edge, lower bound (pixels).
pub const THUMB_MIN_EDGE: usize = 32;
/// Thumbnail long edge, upper bound (pixels).
pub const THUMB_MAX_EDGE: usize = 256;
/// The sidebar thumbnail cache's entry bound.
pub const THUMB_CACHE_ENTRIES: usize = 64;
/// The sidebar thumbnail cache's byte bound.
pub const THUMB_CACHE_BYTES: usize = 24 * 1024 * 1024;
/// The largest render scale.
pub const MAX_SCALE: f64 = 4.0;

/// A sticky note's fixed size, in PDF points (the point is its top-left).
pub const NOTE_SIZE: f64 = 20.0;

/// A shape's keyboard box (the search hit, else the page centre), in points.
pub const SHAPE_SIZE: [f64; 2] = [160.0, 100.0];
/// A signature's keyboard box, in points.
pub const SIGNATURE_SIZE: [f64; 2] = [200.0, 60.0];
/// Ink bound: the most strokes.
pub const MAX_INK_STROKES: usize = 64;
/// Ink bound: the fewest points in a stroke.
pub const MIN_INK_POINTS: usize = 2;
/// Ink bound: the most points in a stroke.
pub const MAX_INK_POINTS: usize = 2048;
/// A stamp or signature image's ceiling, the bound the picker enforces on its side.
pub const MAX_STAMP_IMAGE_BYTES: usize = 4 * 1024 * 1024;
/// The `/Subj` of a signature appearance (a picture, never a `/Sig` field).
pub const SIGNATURE_SUBJECT: &str = "Signature appearance";
/// The `/Subj` of a drawn signature (a black Ink, no `/T`, no `/Contents`).
pub const DRAWN_SIGNATURE_SUBJECT: &str = "Drawn signature";

/// Bounds the annotation contents string on the engine seam.
pub const MAX_NOTE_CHARS: usize = 4096;
/// A bookmark title is user text like a note, so it shares the same ceiling.
pub const MAX_BOOKMARK_CHARS: usize = 4096;
/// A link URI bound; `open-link` is bounded at the same 2048.
pub const MAX_URI_CHARS: usize = 2048;

/// Password entry bound: long enough for any real password.
pub const MAX_PASSWORD_CHARS: usize = 1024;
/// The default `search` hit bound.
pub const MAX_HITS_DEFAULT: usize = 500;
/// The largest `search` hit bound a request may ask for.
pub const MAX_HITS_CAP: usize = 2000;

/// Form designer: a field name is 1..128 chars.
pub const MAX_FIELD_NAME_CHARS: usize = 128;
/// Form designer: the uniqueness walk visits at most this many nodes.
pub const MAX_FIELD_NODES: usize = 4096;
/// Form designer: the uniqueness walk descends at most this many levels.
pub const MAX_FIELD_DEPTH: usize = 8;
/// Form designer: the most radio kids.
pub const MAX_FIELD_KIDS: usize = 32;
/// Form designer: the most choice options.
pub const MAX_FIELD_OPTIONS: usize = 256;
/// Form designer: the longest choice option.
pub const MAX_FIELD_OPTION_CHARS: usize = 256;
/// Form designer: the longest radio kid export name.
pub const MAX_FIELD_KID_NAME: usize = 64;
/// Form designer: the longest text value.
pub const MAX_FIELD_TEXT_CHARS: usize = 4096;
/// The longest accessible label (`/TU`).
pub const MAX_FIELD_LABEL: usize = 256;
/// The smallest authored widget, in points.
pub const MIN_FIELD_RECT_PT: f64 = 4.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_match_the_typescript() {
        assert_eq!(MAX_PDF_BYTES, 120 * 1024 * 1024);
        assert_eq!(MAX_SAVE_BYTES, 52_428_800);
        assert_eq!(MAX_REDACT_SOURCE_BYTES, 12_582_912);
        assert_eq!(THUMB_CACHE_BYTES, 25_165_824);
        assert_eq!(MAX_STAMP_IMAGE_BYTES, 4_194_304);
        assert_eq!(MAX_PIXEL_EDGE, 4096);
        assert_eq!(MIN_SCALE, 0.25);
        assert_eq!(MAX_SCALE, 4.0);
        assert_eq!(SHAPE_SIZE, [160.0, 100.0]);
        assert_eq!(SIGNATURE_SIZE, [200.0, 60.0]);
        assert_eq!(SIGNATURE_SUBJECT, "Signature appearance");
        assert_eq!(DRAWN_SIGNATURE_SUBJECT, "Drawn signature");
        assert_eq!(MAX_HITS_DEFAULT, 500);
        assert_eq!(MAX_HITS_CAP, 2000);
        assert_eq!(MAX_FIELD_NAME_CHARS, 128);
    }
}

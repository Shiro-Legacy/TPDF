// SPDX-License-Identifier: MIT
//! The pinned outline tables, decoded once from the packed files beside this
//! module (`README.md` there: format, provenance and the OFL notice of the font
//! data). A face is decoded on first use; the decoded tables are immutable and
//! live for the rest of the process, so every reference the API hands out is
//! `'static`, as with the generated Rust tables these files replaced.
use super::{BuiltinOutline, BuiltinOutlineCommand, MAX_GLYPH_NAME_BYTES};
use crate::builtin_metrics::BuiltinFace;
use std::sync::OnceLock;

#[derive(Debug)]
pub(super) struct Face {
    pub(super) glyphs: Box<[BuiltinOutline]>,
    pub(super) commands: Box<[BuiltinOutlineCommand]>,
}

static HELVETICA: OnceLock<Face> = OnceLock::new();
static TIMES_ROMAN: OnceLock<Face> = OnceLock::new();
static HELVETICA_BOLD: OnceLock<Face> = OnceLock::new();
static HELVETICA_BOLD_OBLIQUE: OnceLock<Face> = OnceLock::new();

pub(super) fn face(face: BuiltinFace) -> &'static Face {
    match face {
        BuiltinFace::Helvetica => {
            HELVETICA.get_or_init(|| decode(face, include_bytes!("helvetica.bin")))
        }
        BuiltinFace::TimesRoman => {
            TIMES_ROMAN.get_or_init(|| decode(face, include_bytes!("times_roman.bin")))
        }
        BuiltinFace::HelveticaBold => {
            HELVETICA_BOLD.get_or_init(|| decode(face, include_bytes!("helvetica_bold.bin")))
        }
        BuiltinFace::HelveticaBoldOblique => HELVETICA_BOLD_OBLIQUE
            .get_or_init(|| decode(face, include_bytes!("helvetica_bold_oblique.bin"))),
    }
}

const MAGIC: &[u8; 8] = b"TPDFOUT1";

/// The files are compiled in, so a layout the decoder refuses is a build
/// defect, not an input error: it stops the process with the reason.
fn decode(face: BuiltinFace, bytes: &'static [u8]) -> Face {
    match try_decode(face, bytes) {
        Ok(decoded) => decoded,
        Err(what) => panic!(
            "packed outline data for {}: {what}",
            String::from_utf8_lossy(face.pdf_name())
        ),
    }
}

struct Reader {
    bytes: &'static [u8],
    at: usize,
}

impl Reader {
    fn take(&mut self, n: usize) -> Result<&'static [u8], &'static str> {
        let end = self.at.checked_add(n).ok_or("length overflow")?;
        let slice = self.bytes.get(self.at..end).ok_or("truncated")?;
        self.at = end;
        Ok(slice)
    }
    fn u8(&mut self) -> Result<u8, &'static str> {
        Ok(self.take(1)?[0])
    }
    fn i16(&mut self) -> Result<i32, &'static str> {
        let b = self.take(2)?;
        Ok(i16::from_le_bytes([b[0], b[1]]) as i32)
    }
    fn u16(&mut self) -> Result<u16, &'static str> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, &'static str> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

fn try_decode(face: BuiltinFace, bytes: &'static [u8]) -> Result<Face, &'static str> {
    let mut r = Reader { bytes, at: 0 };
    if r.take(MAGIC.len())? != MAGIC {
        return Err("unknown format");
    }
    let command_count = r.u32()? as usize;
    let glyph_count = r.u32()? as usize;
    // Every command and every glyph record takes at least one byte, which bounds
    // the allocations by the file before anything is read.
    if command_count > bytes.len() || glyph_count > bytes.len() {
        return Err("counts exceed the file");
    }
    let mut commands = Vec::with_capacity(command_count);
    for _ in 0..command_count {
        commands.push(match r.u8()? {
            0 => BuiltinOutlineCommand::MoveTo {
                x: r.i16()?,
                y: r.i16()?,
            },
            1 => BuiltinOutlineCommand::LineTo {
                x: r.i16()?,
                y: r.i16()?,
            },
            2 => BuiltinOutlineCommand::CubicTo {
                x1: r.i16()?,
                y1: r.i16()?,
                x2: r.i16()?,
                y2: r.i16()?,
                x: r.i16()?,
                y: r.i16()?,
            },
            3 => BuiltinOutlineCommand::Close,
            _ => return Err("unknown command tag"),
        });
    }
    let mut glyphs = Vec::with_capacity(glyph_count);
    let mut previous: Option<&'static [u8]> = None;
    for _ in 0..glyph_count {
        let name_len = r.u8()? as usize;
        if name_len == 0 || name_len > MAX_GLYPH_NAME_BYTES {
            return Err("glyph name length out of range");
        }
        let name = r.take(name_len)?;
        if previous.is_some_and(|p| p >= name) {
            return Err("glyph names not strictly ascending");
        }
        previous = Some(name);
        let native_gid = r.u16()?;
        let command_start = r.u32()?;
        let command_len = r.u32()?;
        let end = (command_start as usize)
            .checked_add(command_len as usize)
            .ok_or("command range overflow")?;
        if end > command_count {
            return Err("command range exceeds the commands");
        }
        glyphs.push(BuiltinOutline {
            face,
            name,
            native_gid,
            command_start,
            command_len,
        });
    }
    if r.at != bytes.len() {
        return Err("trailing bytes");
    }
    Ok(Face {
        glyphs: glyphs.into_boxed_slice(),
        commands: commands.into_boxed_slice(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_face_decodes_to_the_pinned_tables() {
        for (face, glyphs, commands) in [
            (BuiltinFace::Helvetica, 654, 12788),
            (BuiltinFace::TimesRoman, 654, 17838),
            (BuiltinFace::HelveticaBold, 654, 12835),
            (BuiltinFace::HelveticaBoldOblique, 654, 13469),
        ] {
            let decoded = super::face(face);
            assert_eq!(decoded.glyphs.len(), glyphs);
            assert_eq!(decoded.commands.len(), commands);
            assert!(decoded.glyphs.windows(2).all(|w| w[0].name < w[1].name));
        }
    }

    #[test]
    fn a_damaged_file_is_refused_with_a_reason() {
        let good = include_bytes!("helvetica.bin");
        let mut bad = good.to_vec();
        bad[0] ^= 1;
        assert_eq!(
            try_decode(BuiltinFace::Helvetica, bad.leak()).unwrap_err(),
            "unknown format"
        );
        assert_eq!(
            try_decode(
                BuiltinFace::Helvetica,
                good[..good.len() - 1].to_vec().leak()
            )
            .unwrap_err(),
            "truncated"
        );
        let mut counts = good.to_vec();
        counts[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            try_decode(BuiltinFace::Helvetica, counts.leak()).unwrap_err(),
            "counts exceed the file"
        );
    }
}

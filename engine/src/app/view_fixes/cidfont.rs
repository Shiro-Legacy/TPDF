// SPDX-License-Identifier: MIT
//! CID fonts without a program outside the CJK collections. hayro draws
//! such a font with a base 14 face, finding each glyph through the font's
//! Unicode and else taking the glyph numbered like the CID, which in a
//! substitute face is some other character. Reference viewers find the
//! glyph through
//! /ToUnicode, or without one (and without a /CIDToGIDMap stream) takes the
//! CID as an index into the standard Macintosh glyph order, as the
//! TrueType fonts these CIDs come from number their first 258 glyphs; it
//! draws nothing for a CID it cannot place. For a font with /Identity-H or
//! /Identity-V encoding the view gives the descendant a /CIDToGIDMap that
//! places no CID, and the font, when it has neither, a /ToUnicode of the
//! Macintosh order (which the text the view extracts then reads too).
use crate::object::{Dictionary, Object, ObjectId};

use super::Fixes;

/// The Unicode of each glyph of the standard Macintosh order (TrueType
/// 'post' format 1), 0 for the three without one.
const MAC_ORDER: [u16; 258] = [
    0, 0, 0, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x2B, 0x2C, 0x2D,
    0x2E, 0x2F, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D,
    0x3E, 0x3F, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x4B, 0x4C, 0x4D,
    0x4E, 0x4F, 0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x5B, 0x5C, 0x5D,
    0x5E, 0x5F, 0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x6B, 0x6C, 0x6D,
    0x6E, 0x6F, 0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x7B, 0x7C, 0x7D,
    0x7E, 0xC4, 0xC5, 0xC7, 0xC9, 0xD1, 0xD6, 0xDC, 0xE1, 0xE0, 0xE2, 0xE4, 0xE3, 0xE5, 0xE7, 0xE9,
    0xE8, 0xEA, 0xEB, 0xED, 0xEC, 0xEE, 0xEF, 0xF1, 0xF3, 0xF2, 0xF4, 0xF6, 0xF5, 0xFA, 0xF9, 0xFB,
    0xFC, 0x2020, 0xB0, 0xA2, 0xA3, 0xA7, 0x2022, 0xB6, 0xDF, 0xAE, 0xA9, 0x2122, 0xB4, 0xA8,
    0x2260, 0xC6, 0xD8, 0x221E, 0xB1, 0x2264, 0x2265, 0xA5, 0xB5, 0x2202, 0x2211, 0x220F, 0x3C0,
    0x222B, 0xAA, 0xBA, 0x2126, 0xE6, 0xF8, 0xBF, 0xA1, 0xAC, 0x221A, 0x192, 0x2248, 0x2206, 0xAB,
    0xBB, 0x2026, 0xA0, 0xC0, 0xC3, 0xD5, 0x152, 0x153, 0x2013, 0x2014, 0x201C, 0x201D, 0x2018,
    0x2019, 0xF7, 0x25CA, 0xFF, 0x178, 0x2044, 0xA4, 0x2039, 0x203A, 0xFB01, 0xFB02, 0x2021, 0xB7,
    0x201A, 0x201E, 0x2030, 0xC2, 0xCA, 0xC1, 0xCB, 0xC8, 0xCD, 0xCE, 0xCF, 0xCC, 0xD3, 0xD4,
    0xF8FF, 0xD2, 0xDA, 0xDB, 0xD9, 0x131, 0x2C6, 0x2DC, 0xAF, 0x2D8, 0x2D9, 0x2DA, 0xB8, 0x2DD,
    0x2DB, 0x2C7, 0x141, 0x142, 0x160, 0x161, 0x17D, 0x17E, 0xA6, 0xD0, 0xF0, 0xDD, 0xFD, 0xDE,
    0xFE, 0x2212, 0xD7, 0xB9, 0xB2, 0xB3, 0xBD, 0xBC, 0xBE, 0x20A3, 0x11E, 0x11F, 0x130, 0x15E,
    0x15F, 0x106, 0x107, 0x10C, 0x10D, 0x111,
];

/// The orderings hayro draws through a collection's own Unicode map.
const CJK: [&[u8]; 4] = [b"GB1", b"CNS1", b"Japan1", b"Korea1"];

impl Fixes<'_, '_> {
    /// Gives a Type0 `font` without a program outside the CJK collections
    /// the /CIDToGIDMap and /ToUnicode of the module, once.
    pub(super) fn fix_cid_fallback(&mut self, font: &Object) {
        let doc = self.doc;
        let Object::Reference(font_id) = font else {
            return;
        };
        let font_id = *font_id;
        let Some(mut value) = self.current(font_id) else {
            return;
        };
        let Some(type0) = value.as_dict_mut() else {
            return;
        };
        if doc.dict_get(type0, b"Subtype").as_name() != Some(b"Type0")
            || !matches!(
                doc.dict_get(type0, b"Encoding").as_name(),
                Some(b"Identity-H" | b"Identity-V")
            )
            || !self.recolor.done.insert(font_id)
        {
            return;
        }
        let Some(descendant) = (doc.dict_get(type0, b"DescendantFonts").as_array())
            .and_then(|fonts| fonts.first())
            .cloned()
        else {
            return;
        };
        let (descendant_id, cid_font) = match &descendant {
            Object::Reference(id) => (Some(*id), self.current(*id)),
            other => (None, Some(other.clone())),
        };
        let Some(mut cid_font) = cid_font else {
            return;
        };
        let Some(cid) = cid_font.as_dict_mut() else {
            return;
        };
        if !self.substituted(cid) {
            return;
        }
        let (Some(no_glyphs), Some(mac)) = (self.no_glyphs(), self.mac_to_unicode()) else {
            return;
        };
        let placed = match doc.dict_get(cid, b"CIDToGIDMap") {
            Object::Null => true,
            map => map.as_name() == Some(b"Identity"),
        };
        cid.insert(b"CIDToGIDMap".to_vec(), Object::Reference(no_glyphs));
        let mut font_changed = false;
        if placed && type0.get(b"ToUnicode".as_slice()).is_none() {
            type0.insert(b"ToUnicode".to_vec(), Object::Reference(mac));
            font_changed = true;
        }
        match descendant_id {
            Some(id) => {
                self.objects.insert(id, cid_font);
            }
            None => {
                let fonts = type0.get_mut(b"DescendantFonts".as_slice());
                // A direct descendant in a direct array is edited in place.
                match fonts.and_then(Object::as_array_mut) {
                    Some(fonts) if !fonts.is_empty() => fonts[0] = cid_font,
                    _ => return,
                }
                font_changed = true;
            }
        }
        if font_changed {
            self.objects.insert(font_id, value);
        }
    }

    /// Whether hayro draws a CID font with a substitute face and its own
    /// glyph order: no program, not a CJK collection.
    fn substituted(&self, cid: &Dictionary) -> bool {
        let doc = self.doc;
        if !matches!(
            doc.dict_get(cid, b"Subtype").as_name(),
            Some(b"CIDFontType0" | b"CIDFontType2")
        ) {
            return false;
        }
        let descriptor = doc.dict_get(cid, b"FontDescriptor").as_dict();
        let embedded = descriptor.is_some_and(|descriptor| {
            [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"]
                .iter()
                .any(|key| !doc.dict_get(descriptor, key).is_null())
        });
        let info = doc.dict_get(cid, b"CIDSystemInfo").as_dict();
        let string = |key: &[u8]| info.and_then(|info| doc.dict_get(info, key).as_string());
        let collection = string(b"Registry") == Some(b"Adobe".as_slice())
            && string(b"Ordering").is_some_and(|ordering| CJK.contains(&ordering));
        !embedded && !collection
    }

    /// A /CIDToGIDMap stream placing no CID, one per view.
    fn no_glyphs(&mut self) -> Option<ObjectId> {
        if let Some(id) = self.recolor.no_glyphs {
            return Some(id);
        }
        let id = self.next()?;
        self.streams.insert(id, (Dictionary::new(), vec![0, 0]));
        self.recolor.no_glyphs = Some(id);
        Some(id)
    }

    /// A /ToUnicode CMap of the Macintosh order for two-byte codes, one per
    /// view.
    fn mac_to_unicode(&mut self) -> Option<ObjectId> {
        if let Some(id) = self.recolor.mac_to_unicode {
            return Some(id);
        }
        let id = self.next()?;
        let mut cmap = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
            /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
            /CMapName /TPDF-Mac-UCS def /CMapType 2 def\n\
            1 begincodespacerange <0000> <FFFF> endcodespacerange\n"
            .to_vec();
        let entries: Vec<(usize, u16)> = (MAC_ORDER.iter().enumerate())
            .filter(|&(_, &unicode)| unicode != 0)
            .map(|(cid, &unicode)| (cid, unicode))
            .collect();
        // At most 100 entries per bfchar block.
        for block in entries.chunks(100) {
            cmap.extend_from_slice(format!("{} beginbfchar\n", block.len()).as_bytes());
            for (cid, unicode) in block {
                cmap.extend_from_slice(format!("<{cid:04X}> <{unicode:04X}>\n").as_bytes());
            }
            cmap.extend_from_slice(b"endbfchar\n");
        }
        cmap.extend_from_slice(b"endcmap CMapName currentdict /CMap defineresource pop end end\n");
        self.streams.insert(id, (Dictionary::new(), cmap));
        self.recolor.mac_to_unicode = Some(id);
        Some(id)
    }
}

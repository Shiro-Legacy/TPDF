// SPDX-License-Identifier: AGPL-3.0-or-later
//! Page display and structured text on PDF bytes, backed by hayro.
//! Page geometry is the visible page box: CropBox intersected with
//! MediaBox, snapped /Rotate and UserUnit, with the top-left at the origin.
use crate::document::{Document, DocumentLimits};
use crate::object::Object as CoreObject;
use hayro::kurbo::Affine;
use hayro_syntax::object::dict::keys::{
    ANNOTS, AP, BASE_STATE, D, F, OC, OCGS, OCMD, OCPROPERTIES, OFF, ON, P, POPUP, SUBTYPE, TYPE,
};
use hayro_syntax::object::{Array, Dict, Name, Object, ObjectIdentifier};
use hayro_syntax::page::Page;
use hayro_syntax::{LoadPdfError, Pdf, PdfData};
use md5::{Digest, Md5};
use std::collections::HashSet;
use std::fmt;

mod fonts;
mod image;
mod measure;
mod render;
mod stext;
mod text_unicode;
mod walk;

pub(crate) use fonts::scan as scan_fonts;
pub use fonts::{FontError, MAX_CJK_FONT_BYTES, cjk_font_wanted, has_cjk_font, set_cjk_font};
pub use image::{ImageParams, ImageSamples};
pub use measure::{GlyphMeasure, Measurer};
pub use render::{
    MAX_RENDER_EDGE, MAX_RENDER_PIXELS, RenderBudgets, RenderOptions, RenderStats, RgbaImage,
};
pub(crate) use stext::text_font_name;
pub use stext::{
    LineSummary, MAX_PAGE_CHARS, MAX_SEARCH_QUADS, SearchResult, StructuredText, TextBlock,
    TextChar, TextFont, TextLine, TextReader,
};
pub(crate) use walk::is_image_filter;
pub use walk::{
    MAX_CONTENT_RUNS, MAX_FONT_BYTES, MAX_GRAPHICS_STATES, MAX_HELD_CONTENT_BYTES,
    MAX_PATH_ELEMENTS, MAX_READ_CONTENT_BYTES, MAX_SHOWN_BYTES,
};

/// A parsed document that owns its bytes.
pub struct ViewDocument {
    pdf: Pdf,
    /// What its font streams decode to, measured once (`walk`).
    font_sizes: walk::FontSizes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewError {
    /// The document is encrypted and the password did not open it.
    NeedsPassword,
    /// The bytes are not a readable PDF, or its encryption is unsupported.
    Invalid(&'static str),
    PageOutOfRange {
        page: usize,
        count: usize,
    },
    /// The scale is not finite and positive, or the image would be larger
    /// than [`MAX_RENDER_PIXELS`] or could not be allocated.
    BadScale,
    /// The page's content invokes itself, nests too deeply, would cost more
    /// than [`MAX_CONTENT_RUNS`] content-stream runs, shows more than
    /// [`MAX_SHOWN_BYTES`] of text, yields more than [`MAX_PAGE_CHARS`]
    /// characters, keeps more than [`MAX_GRAPHICS_STATES`] graphics states,
    /// paints a path of more than [`MAX_PATH_ELEMENTS`] elements, decodes
    /// past hayro-syntax's limit, holds more than [`MAX_HELD_CONTENT_BYTES`]
    /// of decoded content at once, reads more than
    /// [`MAX_READ_CONTENT_BYTES`], selects fonts that decode past
    /// [`MAX_FONT_BYTES`], or its drawing passes a render budget.
    TooComplex(&'static str),
}

impl fmt::Display for ViewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NeedsPassword => f.write_str("document is encrypted, a password is required"),
            Self::Invalid(reason) => write!(f, "invalid document: {reason}"),
            Self::PageOutOfRange { page, count } => {
                write!(f, "page {page} is out of range (0..{count})")
            }
            Self::BadScale => f.write_str("scale gives an unusable image size"),
            Self::TooComplex(reason) => write!(f, "page is too complex: {reason}"),
        }
    }
}

impl std::error::Error for ViewError {}

impl ViewDocument {
    /// Parse `bytes`, trying `password` as the user and then the owner
    /// password. For revisions 2-4 the owner password is checked by the
    /// strict core parser, so it opens only documents that parser reads.
    pub fn open(bytes: Vec<u8>, password: &str) -> Result<Self, ViewError> {
        Self::open_with(bytes, password, true)
    }

    /// `open` of bytes whose annotations View usage leaves out are already
    /// marked Hidden wherever they are stored and whose page tree hayro
    /// reads as the app lists it (the app's view, `app::view_fixes`): no
    /// pass over every page's annotations.
    pub fn open_prepared(bytes: Vec<u8>) -> Result<Self, ViewError> {
        Self::open_with(bytes, "", false)
    }

    fn open_with(bytes: Vec<u8>, password: &str, hide: bool) -> Result<Self, ViewError> {
        let data = PdfData::from(bytes);
        let (pdf, password) = match parse(data.clone(), password) {
            Ok(pdf) => (pdf, password.to_string()),
            // hayro checks only the user password for revisions 2-4.
            Err(ViewError::NeedsPassword) => {
                let user = user_password_from_owner(data.as_ref(), password)
                    .ok_or(ViewError::NeedsPassword)?;
                (parse(data, &user)?, user)
            }
            Err(e) => return Err(e),
        };
        // hayro draws NoView and Popup annotations, which a viewer hides
        // (ISO 32000-2 12.5.3, 12.5.6.14), and ignores an annotation's /OC; drop their appearances in a
        // private copy of the bytes.
        Ok(match hide.then(|| hide_annotations(&pdf)).flatten() {
            Some(patched) => Self {
                pdf: parse(PdfData::from(patched), &password)?,
                font_sizes: walk::FontSizes::default(),
            },
            None => Self {
                pdf,
                font_sizes: walk::FontSizes::default(),
            },
        })
    }

    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    /// The length of the bytes the view reads.
    pub fn byte_len(&self) -> usize {
        self.pdf.data().as_ref().len()
    }

    /// Width and height of the visible page in points (`PageFrame`).
    pub fn page_size(&self, page: usize) -> Result<(f64, f64), ViewError> {
        let frame = PageFrame::new(self.page(page)?);
        Ok((f64::from(frame.width), f64::from(frame.height)))
    }

    fn page(&self, page: usize) -> Result<&Page<'_>, ViewError> {
        let pages = self.pdf.pages();
        pages.get(page).ok_or(ViewError::PageOutOfRange {
            page,
            count: pages.len(),
        })
    }
}

fn parse(bytes: PdfData, password: &str) -> Result<Pdf, ViewError> {
    use hayro_syntax::DecryptionError as D;
    Pdf::new_with_password(bytes, password).map_err(|e| match e {
        LoadPdfError::Decryption(D::PasswordProtected) => ViewError::NeedsPassword,
        LoadPdfError::Decryption(D::UnsupportedAlgorithm) => {
            ViewError::Invalid("unsupported encryption")
        }
        LoadPdfError::Decryption(_) => ViewError::Invalid("broken encryption"),
        LoadPdfError::Invalid => ViewError::Invalid("unreadable PDF"),
    })
}

const PASSWORD_PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

/// The user password of a revision 2-4 document, recovered from /O with the
/// owner password (Algorithm 7), when `password` is the owner password. The
/// strict core parser reads the encryption dictionary and checks the password.
fn user_password_from_owner(data: &[u8], password: &str) -> Option<String> {
    let doc =
        Document::open_with_password(data, DocumentLimits::default(), password.as_bytes()).ok()?;
    let security = doc.security()?;
    if !security.owner || security.revision > 4 {
        return None;
    }
    let dict = match doc.trailer().get(b"Encrypt".as_slice())? {
        CoreObject::Reference(id) => match &doc.object(*id).ok()?.value {
            CoreObject::Dictionary(d) => d.clone(),
            _ => return None,
        },
        CoreObject::Dictionary(d) => d.clone(),
        _ => return None,
    };
    let int = |key: &[u8]| match dict.get(key) {
        Some(CoreObject::Integer(v)) => Some(*v),
        _ => None,
    };
    let Some(CoreObject::String(o)) = dict.get(b"O".as_slice()) else {
        return None;
    };
    let revision = security.revision;
    let bits = match int(b"V") {
        Some(1) | None => 40,
        Some(4) => int(b"Length").unwrap_or(128),
        _ => int(b"Length").unwrap_or(40),
    };
    let n = usize::try_from(bits / 8)
        .ok()
        .filter(|n| (5..=16).contains(n))?;
    let mut padded = PASSWORD_PAD;
    let given = &password.as_bytes()[..password.len().min(32)];
    padded[..given.len()].copy_from_slice(given);
    padded[given.len()..].copy_from_slice(&PASSWORD_PAD[..32 - given.len()]);
    let mut key = Md5::digest(padded).to_vec();
    if revision >= 3 {
        for _ in 0..50 {
            key = Md5::digest(&key).to_vec();
        }
    }
    key.truncate(n);
    let mut user = o.get(..32)?.to_vec();
    if revision == 2 {
        user = rc4(&key, &user);
    } else {
        for i in (0..20u8).rev() {
            let round: Vec<u8> = key.iter().map(|b| b ^ i).collect();
            user = rc4(&round, &user);
        }
    }
    let len = (0..=32)
        .find(|&k| user[k..] == PASSWORD_PAD[..32 - k])
        .unwrap_or(32);
    String::from_utf8(user[..len].to_vec()).ok()
}

fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, usize::from(j));
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|byte| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[usize::from(i)]);
            s.swap(usize::from(i), usize::from(j));
            byte ^ s[usize::from(s[usize::from(i)].wrapping_add(s[usize::from(j)]))]
        })
        .collect()
}

/// The document bytes with the /AP key of every NoView or Popup annotation,
/// and of every annotation whose optional content is off, renamed so that
/// hayro draws nothing for it, or None when there is nothing to hide.
/// Same-length edits keep offsets valid. Annotations stored in object streams
/// are left as they are; the app's view marks them Hidden beforehand
/// (`app::view_fixes`).
fn hide_annotations(pdf: &Pdf) -> Option<Vec<u8>> {
    let data: &[u8] = pdf.data().as_ref();
    let mut patched: Option<Vec<u8>> = None;
    let off = hidden_groups(pdf);
    for page in pdf.pages().iter() {
        let Some(annots) = page.raw().get::<Array<'_>>(ANNOTS) else {
            continue;
        };
        for annot in annots.iter::<Object<'_>>().filter_map(Object::into_dict) {
            let flags = annot.get::<u32>(F).unwrap_or(0);
            let popup = annot
                .get::<Name<'_>>(SUBTYPE)
                .is_some_and(|n| n.as_ref() == POPUP);
            let hidden = flags & 32 != 0 || popup || content_off(&annot, &off);
            if flags & 2 != 0 || !hidden || !annot.contains_key(AP) {
                continue;
            }
            let raw = annot.data();
            let Some(start) = raw.as_ptr().addr().checked_sub(data.as_ptr().addr()) else {
                continue;
            };
            let Some(at) = top_level_key(raw, AP) else {
                continue;
            };
            if start + raw.len() <= data.len() {
                let bytes = patched.get_or_insert_with(|| data.to_vec());
                bytes[start + at + 1] = b'p';
            }
        }
    }
    patched
}

/// The optional-content groups the default configuration turns off, read
/// as hayro reads them: /BaseState /OFF turns off every group of /OCGs, then
/// /ON and /OFF apply in that order.
fn hidden_groups(pdf: &Pdf) -> HashSet<ObjectIdentifier> {
    let mut off = HashSet::new();
    let xref = pdf.xref();
    let Some(properties) = xref
        .get::<Dict<'_>>(xref.root_id())
        .and_then(|catalog| catalog.get::<Dict<'_>>(OCPROPERTIES))
    else {
        return off;
    };
    let Some(config) = properties.get::<Dict<'_>>(D) else {
        return off;
    };
    let refs = |dict: &Dict<'_>, key: &[u8]| -> Vec<ObjectIdentifier> {
        dict.get::<Array<'_>>(key).map_or_else(Vec::new, |items| {
            (items.raw_iter())
                .filter_map(|item| item.as_obj_ref().map(ObjectIdentifier::from))
                .collect()
        })
    };
    if config
        .get::<Name<'_>>(BASE_STATE)
        .is_some_and(|state| state.as_ref() == OFF)
    {
        off.extend(refs(&properties, OCGS));
    }
    for group in refs(&config, ON) {
        off.remove(&group);
    }
    off.extend(refs(&config, OFF));
    off
}

/// Whether an annotation's /OC hides it: a group that is off, or a
/// membership dictionary (/OCGs and /P, default AnyOn) that evaluates off.
fn content_off(annot: &Dict<'_>, off: &HashSet<ObjectIdentifier>) -> bool {
    let Some(oc) = annot.get::<Dict<'_>>(OC) else {
        return false;
    };
    let membership = oc.get::<Name<'_>>(TYPE).is_some_and(|t| t.as_ref() == OCMD);
    if let (false, Some(group)) = (membership, annot.get_ref(OC)) {
        return off.contains(&group.into());
    }
    let groups: Vec<bool> = match oc.get::<Array<'_>>(OCGS) {
        Some(items) => (items.raw_iter())
            .filter_map(|item| item.as_obj_ref())
            .map(|group| off.contains(&group.into()))
            .collect(),
        None => oc
            .get_ref(OCGS)
            .map(|group| vec![off.contains(&group.into())])
            .unwrap_or_default(),
    };
    if groups.is_empty() {
        return false;
    }
    match oc.get::<Name<'_>>(P).as_deref() {
        Some(b"AllOn") => groups.iter().any(|&hidden| hidden),
        Some(b"AnyOff") => !groups.iter().any(|&hidden| hidden),
        Some(b"AllOff") => !groups.iter().all(|&hidden| hidden),
        _ => groups.iter().all(|&hidden| hidden),
    }
}

/// Where the name of the top-level `key` starts in a raw dictionary (after
/// the slash), if it occurs there exactly once.
fn top_level_key(dict: &[u8], key: &[u8]) -> Option<usize> {
    let delimiter = |c: &u8| c.is_ascii_whitespace() || b"()<>[]{}/%".contains(c);
    let (mut found, mut depth, mut i) = (None, 0, 0);
    while i < dict.len() {
        match dict[i] {
            b'(' => {
                let mut nest = 0;
                while i < dict.len() {
                    match dict[i] {
                        b'\\' => i += 1,
                        b'(' => nest += 1,
                        b')' if nest == 1 => break,
                        b')' => nest -= 1,
                        _ => {}
                    }
                    i += 1;
                }
            }
            b'<' if dict.get(i + 1) == Some(&b'<') => {
                depth += 1;
                i += 1;
            }
            b'>' if dict.get(i + 1) == Some(&b'>') => {
                depth -= 1;
                i += 1;
            }
            b'<' => {
                while i < dict.len() && dict[i] != b'>' {
                    i += 1;
                }
            }
            b'[' => depth += 1,
            b']' => depth -= 1,
            b'/' if depth == 1
                && dict.get(i + 1..i + 1 + key.len()) == Some(key)
                && dict.get(i + 1 + key.len()).is_some_and(delimiter) =>
            {
                if found.is_some() {
                    return None;
                }
                found = Some(i + 1);
            }
            _ => {}
        }
        i += 1;
    }
    found
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PageFrame {
    pub(crate) ctm: Affine,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

impl PageFrame {
    pub(crate) fn new(_page: &Page<'_>) -> Self {
        todo!("clean-room: spec 04")
    }
}

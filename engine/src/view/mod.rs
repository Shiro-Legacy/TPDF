// SPDX-License-Identifier: AGPL-3.0-or-later
//! Page display and structured text on PDF bytes, backed by hayro.
//! Page geometry follows MuPDF's bound page: CropBox intersected with
//! MediaBox, snapped /Rotate and UserUnit, with the top-left at the origin.
use crate::document::{Document, DocumentLimits};
use crate::object::Object as CoreObject;
use hayro::kurbo::Affine;
use hayro_syntax::object::Rect;
use hayro_syntax::object::dict::keys::{
    ANNOTS, AP, CROP_BOX, F, MEDIA_BOX, PARENT, POPUP, ROTATE, SUBTYPE, USER_UNIT,
};
use hayro_syntax::object::{Array, Dict, Name, Object};
use hayro_syntax::page::Page;
use hayro_syntax::{LoadPdfError, Pdf, PdfData};
use md5::{Digest, Md5};
use std::fmt;

mod render;
mod stext;
mod walk;

pub use render::{MAX_RENDER_PIXELS, RenderOptions, RgbaImage};
pub use stext::{
    LineSummary, MAX_SEARCH_QUADS, SearchResult, StructuredText, TextBlock, TextChar, TextFont,
    TextLine,
};
pub use walk::MAX_CONTENT_RUNS;

/// A parsed document that owns its bytes.
pub struct ViewDocument {
    pdf: Pdf,
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
    /// The page's content invokes itself, nests too deeply or would cost more
    /// than [`MAX_CONTENT_RUNS`] content-stream runs.
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
        // hayro draws NoView and Popup annotations, which MuPDF's View usage
        // hides; drop their appearances in a private copy of the bytes.
        Ok(match hide_annotations(&pdf) {
            Some(patched) => Self {
                pdf: parse(PdfData::from(patched), &password)?,
            },
            None => Self { pdf },
        })
    }

    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    /// Width and height of the bound page in points, as MuPDF's `getBounds`.
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

/// The document bytes with the /AP key of every NoView or Popup annotation
/// renamed so that hayro draws nothing for it, or None when there is nothing
/// to hide. Same-length edits keep offsets valid. Annotations stored in object
/// streams are left as they are.
fn hide_annotations(pdf: &Pdf) -> Option<Vec<u8>> {
    let data: &[u8] = pdf.data().as_ref();
    let mut patched: Option<Vec<u8>> = None;
    for page in pdf.pages().iter() {
        let Some(annots) = page.raw().get::<Array<'_>>(ANNOTS) else {
            continue;
        };
        for annot in annots.iter::<Object<'_>>().filter_map(Object::into_dict) {
            let flags = annot.get::<u32>(F).unwrap_or(0);
            let popup = annot
                .get::<Name<'_>>(SUBTYPE)
                .is_some_and(|n| n.as_ref() == POPUP);
            if flags & 2 != 0 || !(flags & 32 != 0 || popup) || !annot.contains_key(AP) {
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

/// User space to page space (points, y down, origin top-left) for one page.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PageFrame {
    pub(crate) ctm: Affine,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl PageFrame {
    pub(crate) fn new(page: &Page<'_>) -> Self {
        let dict = page.raw();
        // An empty box falls back to US Letter and a sliver to the unit square, as in MuPDF.
        let letter = Rect::new(0.0, 0.0, 612.0, 792.0);
        let empty = |r: &Rect| !(r.x0 < r.x1 && r.y0 < r.y1);
        let mut area = inherited::<Array<'_>>(dict, MEDIA_BOX)
            .map(|a| box_of(&a))
            .filter(|r| !empty(r))
            .unwrap_or(letter);
        if let Some(crop) = inherited::<Array<'_>>(dict, CROP_BOX) {
            area = area.intersect(box_of(&crop));
        }
        if empty(&area) {
            area = letter;
        } else if area.x1 - area.x0 < 1.0 || area.y1 - area.y0 < 1.0 {
            area = Rect::new(0.0, 0.0, 1.0, 1.0);
        }
        let unit = dict
            .get::<f32>(USER_UNIT)
            .filter(|u| u.is_finite() && *u > 0.0)
            .unwrap_or(1.0);
        // MuPDF's pdf_to_int rounds a real half up.
        let mut rotate = inherited::<f32>(dict, ROTATE).map_or(0, |r| (r + 0.5).floor() as i32);
        if rotate < 0 {
            rotate = 360 - (rotate.unsigned_abs() % 360) as i32;
        }
        rotate = 90 * ((rotate % 360 + 45) / 90);
        if rotate >= 360 {
            rotate = 0;
        }
        // Page sizes are f32 in MuPDF; pixel rounding depends on it.
        let (x0, y0, x1, y1) = (
            area.x0 as f32,
            area.y0 as f32,
            area.x1 as f32,
            area.y1 as f32,
        );
        let w = x1 * unit - x0 * unit;
        let h = y1 * unit - y0 * unit;
        let (u, x0, y0, x1, y1) = (
            f64::from(unit),
            f64::from(x0),
            f64::from(y0),
            f64::from(x1),
            f64::from(y1),
        );
        let (coefficients, width, height) = match rotate {
            90 => ([0., u, u, 0., -y0 * u, -x0 * u], h, w),
            180 => ([-u, 0., 0., u, x1 * u, -y0 * u], w, h),
            270 => ([0., -u, -u, 0., y1 * u, x1 * u], h, w),
            _ => ([u, 0., 0., -u, -x0 * u, y1 * u], w, h),
        };
        Self {
            ctm: Affine::new(coefficients),
            width,
            height,
        }
    }
}

/// MuPDF's `pdf_to_rect`: missing or non-numeric entries read as zero.
fn box_of(array: &Array<'_>) -> Rect {
    let mut v = [0.0f64; 4];
    for (slot, item) in v.iter_mut().zip(array.iter::<Object<'_>>()) {
        *slot = item.into_f32().map_or(0.0, f64::from);
    }
    Rect::new(
        v[0].min(v[2]),
        v[1].min(v[3]),
        v[0].max(v[2]),
        v[1].max(v[3]),
    )
}

/// Look up `key` on the page and then its /Parent chain.
fn inherited<'a, T>(page: &Dict<'a>, key: &[u8]) -> Option<T>
where
    T: TryFrom<Object<'a>>,
{
    let mut node = page.clone();
    for _ in 0..64 {
        if let Some(value) = node
            .get::<Object<'a>>(key)
            .and_then(|o| T::try_from(o).ok())
        {
            return Some(value);
        }
        node = node.get::<Dict<'a>>(PARENT)?;
    }
    None
}

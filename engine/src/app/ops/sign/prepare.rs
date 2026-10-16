// SPDX-License-Identifier: AGPL-3.0-or-later
//! engine.ts `prepareSignature` (src/renderer/sign-prepare.ts): a signature
//! field on a copy of the document, either a new widget or an existing empty
//! field signed in place, with its appearance and a /V dictionary whose
//! /Contents and /ByteRange are placeholders the external signer fills in
//! place; then the output bytes and the placeholders' offsets
//! (`scanSignature`).
//!
//! A signed source gets an incremental update after its bytes, which stay
//! unchanged (the copy edited is that file, opened as it is or rebuilt from
//! its own xref); any other source is written in full. Only the copy is
//! edited, never the session's document.
use std::collections::BTreeSet;

use crate::app::document::{LowNumbers, MAX_OBJECT_NUMBER};
use crate::app::edit::{Edit, Place};
use crate::app::{AppError, MAX_SAVE_BYTES, info, overlay};
use crate::document::{Document, DocumentError, DocumentErrorKind};
use crate::incremental::{Original, write_incremental_after};
use crate::object::{Dictionary, Object, ObjectId, decode_text_string};
use crate::serialize::SerializeError;
use crate::writer::{DocumentWriteError, Encryption, WriteOptions, write_document};

use super::widgets::{Frame, inherited, real, widgets};
use crate::app::request::js_number;

/// sign-prepare.ts `SENTINEL`: the /ByteRange placeholder, overwritten in
/// place with fixed-width numbers, so no offset moves.
pub const SENTINEL: &[u8; 36] = b"[0 1000000000 1000000000 1000000000]";
/// limits.ts `MAX_SIG_CONTENTS_BYTES`: the bytes /Contents reserves, written
/// as twice as many hex digits.
pub const MAX_SIG_CONTENTS_BYTES: usize = 8192;
/// A sign-prepare request after its checks.
#[derive(Clone, Debug)]
pub(crate) struct Options<'a> {
    pub name: &'a str,
    pub reason: &'a str,
    pub page: usize,
    /// Page space; None for an invisible signature.
    pub rect: Option<[f64; 4]>,
    pub certify: bool,
    /// Milliseconds since the epoch, as `new Date(when)` truncates them.
    pub when: i64,
    /// The signer's local time minus UTC, in milliseconds.
    pub local_offset: i64,
    /// An existing empty field to sign in place.
    pub field: Option<&'a str>,
}

/// The output and where its placeholders are (`scanSignature`).
#[derive(Clone, Debug)]
pub(crate) struct Prepared {
    pub bytes: Vec<u8>,
    /// The first hex digit inside `/Contents<...>`.
    pub contents_at: usize,
    pub byte_range_at: usize,
    /// The field's rectangle in page space ([0 0 0 0] when invisible).
    pub rect: [f64; 4],
    pub revisions: usize,
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::reason("invalid", message)
}

fn failed(error: DocumentError) -> AppError {
    invalid(format!("sign-prepare: {error}"))
}

/// The copy being signed and every object the signing wrote, which an
/// incremental update writes.
struct Signing {
    doc: Document<'static>,
    /// Where new numbers started, and the next one.
    start: u32,
    next: u32,
    /// Numbers once `next` passes `MAX_OBJECT_NUMBER`.
    low: Option<LowNumbers>,
    touched: BTreeSet<ObjectId>,
}

impl Signing {
    /// New objects take numbers from the largest /Size on, as MuPDF
    /// allocates at its xref length: a number the file listed (even as
    /// free) is never redefined. `floor` is that bound for a rebuilt copy,
    /// whose own trailer no longer has it. Past `MAX_OBJECT_NUMBER` (the
    /// copy holds a larger number), new objects take the lowest numbers no
    /// object holds or references (`LowNumbers`), as the session does; only
    /// a full save gets there, since a signed source holding such a number
    /// is refused (`extendable`).
    fn new(doc: Document<'static>, floor: u32) -> Self {
        let sizes = doc.trailer_chain().filter_map(|trailer| {
            let size = trailer.get(b"Size".as_slice())?.as_i64()?;
            u32::try_from(size)
                .ok()
                .filter(|&size| size <= MAX_OBJECT_NUMBER + 1)
        });
        let next = doc.next_id().map_or(u32::MAX, |id| id.number());
        let next = sizes.chain([next, floor]).max().unwrap_or(next).max(1);
        Self {
            doc,
            start: next,
            next,
            low: None,
            touched: BTreeSet::new(),
        }
    }

    fn reserve(&mut self) -> Result<ObjectId, DocumentError> {
        let number = if self.next <= MAX_OBJECT_NUMBER {
            self.next += 1;
            Some(self.next - 1)
        } else {
            let low = match &mut self.low {
                Some(low) => low,
                low => {
                    // Numbers reserved before may not be in the copy yet.
                    let mut numbers = LowNumbers::new(&self.doc, None);
                    for number in self.start..self.next.min(MAX_OBJECT_NUMBER + 1) {
                        numbers.take(number);
                    }
                    low.insert(numbers)
                }
            };
            low.allocate()
        };
        number
            .and_then(|number| ObjectId::new(number, 0).ok())
            .ok_or(DocumentError {
                offset: 0,
                kind: DocumentErrorKind::LimitExceeded("object numbers"),
            })
    }

    /// A new stream with unfiltered bytes.
    fn stream(
        &mut self,
        dictionary: Dictionary,
        bytes: Vec<u8>,
    ) -> Result<ObjectId, DocumentError> {
        let id = self.reserve()?;
        self.doc.put_stream_decoded(id, dictionary, bytes)?;
        self.touched.insert(id);
        Ok(id)
    }
}

impl Edit for Signing {
    fn document(&self) -> &Document<'static> {
        &self.doc
    }
    fn update<R>(
        &mut self,
        id: ObjectId,
        edit: impl FnOnce(&mut Object) -> R,
    ) -> Result<R, DocumentError> {
        let object = self.doc.object(id)?;
        let stream = object.stream.is_some();
        let mut value = object.value.clone();
        let result = edit(&mut value);
        match (stream, value) {
            (true, Object::Dictionary(dictionary)) => self.doc.set_stream_dict(id, dictionary)?,
            (true, _) => {
                return Err(DocumentError {
                    offset: 0,
                    kind: DocumentErrorKind::Invalid("stream value must stay a dictionary"),
                });
            }
            (false, value) => self.doc.put_value(id, value)?,
        }
        self.touched.insert(id);
        Ok(result)
    }
    fn put_value(&mut self, id: ObjectId, value: Object) -> Result<(), DocumentError> {
        self.doc.put_value(id, value)?;
        self.touched.insert(id);
        Ok(())
    }
    fn put_stream_raw(
        &mut self,
        id: ObjectId,
        dictionary: Dictionary,
        raw: Vec<u8>,
    ) -> Result<(), DocumentError> {
        self.doc.put_stream_raw(id, dictionary, raw)?;
        self.touched.insert(id);
        Ok(())
    }
    fn allocate(&mut self, value: Object) -> Result<ObjectId, DocumentError> {
        let id = self.reserve()?;
        self.put_value(id, value)?;
        Ok(id)
    }
}

fn name(text: &str) -> Object {
    Object::name(text.as_bytes().to_vec())
}

/// A JavaScript number as mupdf.js stores it: an integer when it is an
/// int32, otherwise a float real.
fn number(value: f64) -> Result<Object, AppError> {
    if value.fract() == 0.0 && (f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&value) {
        return Ok(Object::Integer(value as i64));
    }
    let single = value as f32;
    Object::real(f64::from(single))
        .ok_or_else(|| invalid("sign-prepare: the signature rectangle is out of range"))
}

/// The page's inherited /Rotate as `getInheritable("Rotate").asNumber()`
/// reads it, turned into 0..360 the JavaScript way (45 stays 45).
fn rotation(doc: &Document<'_>, page: &Dictionary) -> f64 {
    let rotate = inherited(doc, page, b"Rotate")
        .and_then(|value| real(doc, value))
        .map_or(0.0, f64::from);
    ((rotate % 360.0) + 360.0) % 360.0
}

/// Prepares the signature on `doc`, a copy of the source. With `original`
/// (the signed file `doc` holds the objects of), the output is its bytes
/// plus an update; otherwise a full save of `doc`.
pub(crate) fn prepare(
    doc: Document<'static>,
    options: &Options<'_>,
    original: Option<Original<'_>>,
) -> Result<Prepared, AppError> {
    let source_length = original.map_or(0, |original| original.bytes.len());
    // Like each /Size, a floor past MuPDF's largest number is not used.
    let floor = original
        .and_then(|original| u32::try_from(original.size).ok())
        .filter(|&size| size <= MAX_OBJECT_NUMBER + 1)
        .unwrap_or(0);
    let mut signing = Signing::new(doc, floor);
    let rect = match options.field {
        Some(field) => sign_field(&mut signing, options, field)?,
        None => new_widget(&mut signing, options)?,
    };
    let mut limits = overlay::limits();
    limits.syntax.max_output_bytes = MAX_SAVE_BYTES;
    let written = if let Some(original) = original {
        write_incremental_after(&signing.doc, original, &signing.touched, limits, None)
    } else {
        let options = WriteOptions {
            encryption: Encryption::Plaintext,
            ..WriteOptions::APP
        };
        write_document(&signing.doc, limits, options)
    };
    let bytes = match written {
        Ok(bytes) if bytes.len() <= MAX_SAVE_BYTES => bytes,
        Ok(_) | Err(DocumentWriteError::Serialize(SerializeError::OutputLimit)) => {
            return Err(AppError::reason(
                "too-large",
                format!(
                    "sign-prepare: the signed copy would exceed the {MAX_SAVE_BYTES} byte save ceiling"
                ),
            ));
        }
        Err(error) => return Err(invalid(format!("sign-prepare: {error}"))),
    };
    let Some((contents_at, byte_range_at, revisions)) = scan(&bytes, source_length) else {
        return Err(invalid(
            "sign-prepare: the signature placeholders are not each written exactly once",
        ));
    };
    Ok(Prepared {
        bytes,
        contents_at,
        byte_range_at,
        rect,
        revisions,
    })
}

/// The page a new widget goes on: its object, transform and rotation.
fn page_of(doc: &Document<'_>, index: usize) -> Result<(ObjectId, Frame, f64), AppError> {
    let pages = info::pages(doc);
    let page = pages
        .get(index)
        .ok_or_else(|| invalid("sign-prepare: invalid page"))?;
    // MuPDF would copy a page written into /Kids into the widget's /P.
    let id = page
        .id
        .ok_or_else(|| invalid("sign-prepare: the page is not an indirect object"))?;
    Ok((id, Frame::of(doc, page.dict), rotation(doc, page.dict)))
}

/// sign-prepare.ts's field path: the empty top-level field `field` on the
/// page keeps its object number; its /AP is replaced by the signed
/// appearance and /V names the new signature dictionary.
fn sign_field(
    signing: &mut Signing,
    options: &Options<'_>,
    field: &str,
) -> Result<[f64; 4], AppError> {
    let (widget, rect, turn) = {
        let doc = &signing.doc;
        let pages = info::pages(doc);
        let page = pages
            .get(options.page)
            .ok_or_else(|| invalid("sign-prepare: invalid page"))?;
        let (frame, turn) = (Frame::of(doc, page.dict), rotation(doc, page.dict));
        let found = widgets(doc, page.dict)
            .into_iter()
            .find(|w| w.is_signature(doc) && w.name(doc).as_deref() == Some(field));
        let Some(found) = found else {
            return Err(invalid(format!(
                "sign-prepare: page {} has no signature field {field}",
                options.page
            )));
        };
        let id = found.id.ok_or_else(|| {
            invalid(format!(
                "sign-prepare: {field} is not in the document's /AcroForm /Fields"
            ))
        })?;
        let rect = frame.annotation_rect(doc, found.dict).map(f64::from);
        (id, rect, turn)
    };
    let ap = appearance(signing, options, rect, turn)?;
    let sig = signature(signing, options)?;
    signing
        .update(widget, |value| {
            let dict = value.as_dict_mut()?;
            dict.remove(b"AP".as_slice());
            let normal = Object::dictionary([("N", Object::Reference(ap))]);
            dict.insert(b"AP".to_vec(), normal);
            dict.insert(b"V".to_vec(), Object::Reference(sig));
            Some(())
        })
        .map_err(failed)?
        .ok_or_else(|| invalid(format!("sign-prepare: {field} is not a dictionary")))?;
    form(signing, widget, false)?;
    if options.certify {
        certify(signing, sig)?;
    }
    Ok(rect)
}

/// sign-prepare.ts's new widget path: a /Sig widget named `Signature<n>`
/// (the first number no top-level field uses), visible with an appearance
/// when `rect` is given (clamped to the page, at least 40x20), invisible
/// with /Rect [0 0 0 0] otherwise.
fn new_widget(signing: &mut Signing, options: &Options<'_>) -> Result<[f64; 4], AppError> {
    let (page, frame, turn) = page_of(&signing.doc, options.page)?;
    let title = free_name(&signing.doc);
    let widget = signing.reserve().map_err(failed)?;
    let mut rect = [0.0; 4];
    let mut placed = Object::array([0, 0, 0, 0].map(Object::Integer));
    let mut ap = None;
    if let Some([x0, y0, x1, y1]) = options.rect {
        let [bx0, by0, bx1, by1] = frame.bounds().map(f64::from);
        let clamp = |v: f64, low: f64, high: f64| v.min(high).max(low);
        rect = [
            clamp(x0, bx0, bx1),
            clamp(y0, by0, by1),
            clamp(x1, bx0, bx1),
            clamp(y1, by0, by1),
        ];
        if !(rect[2] - rect[0] >= 40.0 && rect[3] - rect[1] >= 20.0) {
            return Err(invalid(
                "sign-prepare: the signature needs at least 40x20 pt of the page",
            ));
        }
        let user = frame.user_rect(rect.map(|v| v as f32));
        let user: Option<Vec<Object>> = user.iter().map(|&v| Object::real(f64::from(v))).collect();
        placed = Object::Array(
            user.ok_or_else(|| invalid("sign-prepare: the signature rectangle is out of range"))?,
        );
        ap = Some(appearance(signing, options, rect, turn)?);
    }
    let sig = signature(signing, options)?;
    let mut dict = Dictionary::new();
    dict.insert(b"Type".to_vec(), name("Annot"));
    dict.insert(b"Subtype".to_vec(), name("Widget"));
    dict.insert(b"P".to_vec(), Object::Reference(page));
    dict.insert(b"F".to_vec(), Object::Integer(4));
    dict.insert(b"FT".to_vec(), name("Sig"));
    dict.insert(b"T".to_vec(), Object::text(&title));
    dict.insert(b"Rect".to_vec(), placed);
    if let Some(ap) = ap {
        let normal = Object::dictionary([("N", Object::Reference(ap))]);
        dict.insert(b"AP".to_vec(), normal);
    }
    dict.insert(b"V".to_vec(), Object::Reference(sig));
    signing
        .put_value(widget, Object::Dictionary(dict))
        .map_err(failed)?;
    annotate(signing, page, widget)?;
    form(signing, widget, true)?;
    if options.certify {
        certify(signing, sig)?;
    }
    Ok(rect)
}

/// `Signature<n>` for the first n no top-level /Fields /T names.
fn free_name(doc: &Document<'_>) -> String {
    let root = doc.resolve_id(doc.root());
    let acro = crate::app::edit::get(doc, root, b"AcroForm");
    let fields = crate::app::edit::get(doc, acro, b"Fields");
    let names: BTreeSet<String> = (doc.resolve(fields).as_array().unwrap_or_default())
        .iter()
        .map(|field| match crate::app::edit::get(doc, field, b"T") {
            Object::String(text) => decode_text_string(text),
            _ => String::new(),
        })
        .collect();
    (1..=names.len() + 1)
        .map(|n| format!("Signature{n}"))
        .find(|title| !names.contains(title))
        .unwrap_or_else(|| "Signature1".into())
}

/// Appends `widget` to the page's /Annots as MuPDF's `createAnnotation`
/// does: the page gets a direct array, its entries plus the widget, so an
/// array object it named (which other pages, or the form's /Fields, may
/// list too) stays as it is. A missing or non-array /Annots becomes
/// `[widget]`.
fn annotate(signing: &mut Signing, page: ObjectId, widget: ObjectId) -> Result<(), AppError> {
    let place = Place::object(page);
    let mut annots = (place.child(&signing.doc, b"Annots").read(&signing.doc))
        .as_array()
        .map(<[Object]>::to_vec)
        .unwrap_or_default();
    annots.push(Object::Reference(widget));
    put(signing, &place, b"Annots", Object::Array(annots))
}

/// The form part: a new widget is pushed onto /AcroForm /Fields (`push`),
/// and the form gets /SigFlags 3. Without an /AcroForm, a new indirect one
/// holds the widget. A form whose /Fields is missing or not an array gets a
/// new one (engine.ts fails there), and so does one whose /Fields array is
/// also a page's /Annots (the widget would show on that page too).
fn form(signing: &mut Signing, widget: ObjectId, push: bool) -> Result<(), AppError> {
    let catalog = Place::object(signing.doc.root());
    let entry = Object::Reference(widget);
    let acro = catalog.child(&signing.doc, b"AcroForm");
    if acro.read(&signing.doc).as_dict().is_none() {
        let mut form = Dictionary::new();
        form.insert(b"Fields".to_vec(), Object::array([entry]));
        form.insert(b"SigFlags".to_vec(), Object::Integer(3));
        let id = signing.allocate(Object::Dictionary(form)).map_err(failed)?;
        return put(signing, &catalog, b"AcroForm", Object::Reference(id));
    }
    if push {
        let fields = acro.child(&signing.doc, b"Fields");
        let items = fields.read(&signing.doc).as_array();
        match items {
            Some(_) if !annots_of_a_page(&signing.doc, &fields) => {
                fields
                    .update(signing, |value| {
                        if let Some(items) = value.as_array_mut() {
                            items.push(entry);
                        }
                    })
                    .map_err(failed)?;
            }
            _ => {
                let mut items = items.map(<[Object]>::to_vec).unwrap_or_default();
                items.push(entry);
                put(signing, &acro, b"Fields", Object::Array(items))?;
            }
        }
    }
    put(signing, &acro, b"SigFlags", Object::Integer(3))
}

/// Whether `place` is an array object some page's /Annots names.
fn annots_of_a_page(doc: &Document<'_>, place: &Place) -> bool {
    place.keys.is_empty()
        && info::pages(doc).iter().any(|page| {
            (page.dict.get(b"Annots".as_slice()))
                .and_then(|annots| doc.resolve_with_holder(annots).0)
                == Some(place.id)
        })
}

/// The catalog's /Perms becomes `{DocMDP: sig}` (MuPDF `root.put`).
fn certify(signing: &mut Signing, sig: ObjectId) -> Result<(), AppError> {
    let catalog = Place::object(signing.doc.root());
    let perms = Object::dictionary([("DocMDP", Object::Reference(sig))]);
    put(signing, &catalog, b"Perms", perms)
}

/// `dict.put(key, value)` on the dictionary at `place`.
fn put(signing: &mut Signing, place: &Place, key: &[u8], value: Object) -> Result<(), AppError> {
    place
        .update(signing, |target| {
            if let Some(dict) = target.as_dict_mut() {
                dict.insert(key.to_vec(), value);
            }
        })
        .map_err(failed)
}

/// sign-prepare.ts `appearance`: a light blue box with a dark blue border
/// and the lines "Digitally signed by <name>", the signer's local date and
/// time, and "Reason: <reason>" when there is one, in WinAnsi Helvetica
/// (a new font object each time), turned by the page's /Rotate.
fn appearance(
    signing: &mut Signing,
    options: &Options<'_>,
    rect: [f64; 4],
    turn: f64,
) -> Result<ObjectId, AppError> {
    let font = Object::dictionary([
        ("Type", name("Font")),
        ("Subtype", name("Type1")),
        ("BaseFont", name("Helvetica")),
        ("Encoding", name("WinAnsiEncoding")),
    ]);
    let font = signing.allocate(font).map_err(failed)?;
    let w = rect[2] - rect[0];
    let h = rect[3] - rect[1];
    let mut lines = vec![
        format!("Digitally signed by {}", options.name),
        local_stamp(options.when.saturating_add(options.local_offset)),
    ];
    if !options.reason.is_empty() {
        lines.push(format!("Reason: {}", options.reason));
    }
    let mut content = format!(
        "q 0.93 0.95 1 rg 0 0 {} {} re f 0.1 0.2 0.5 RG 1 w 0.5 0.5 {} {} re S BT /Helv 9 Tf 0 0 0.15 rg",
        js_number(w),
        js_number(h),
        js_number(w - 1.0),
        js_number(h - 1.0)
    )
    .into_bytes();
    // Td is relative: place the first line, then step down 11 pt per line.
    for (index, line) in lines.iter().enumerate() {
        let step = match index {
            0 => format!(" 8 {} Td (", js_number(h - 13.0)),
            _ => " 0 -11 Td (".into(),
        };
        content.extend_from_slice(step.as_bytes());
        win_ansi(line, &mut content);
        content.extend_from_slice(b") Tj");
    }
    content.extend_from_slice(b" ET Q\n");
    let matrix = if turn == 90.0 {
        [0.0, 1.0, -1.0, 0.0, 0.0, 0.0]
    } else if turn == 180.0 {
        [-1.0, 0.0, 0.0, -1.0, w, h]
    } else if turn == 270.0 {
        [0.0, -1.0, 1.0, 0.0, 0.0, w]
    } else {
        [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
    };
    let matrix: Result<Vec<Object>, AppError> = matrix.into_iter().map(number).collect();
    let mut dict = Dictionary::new();
    dict.insert(b"Type".to_vec(), name("XObject"));
    dict.insert(b"Subtype".to_vec(), name("Form"));
    let bbox: Result<Vec<Object>, AppError> = [0.0, 0.0, w, h].into_iter().map(number).collect();
    dict.insert(b"BBox".to_vec(), Object::Array(bbox?));
    dict.insert(b"Matrix".to_vec(), Object::Array(matrix?));
    let fonts = Object::dictionary([("Helv", Object::Reference(font))]);
    dict.insert(b"Resources".to_vec(), Object::dictionary([("Font", fonts)]));
    signing.stream(dict, content).map_err(failed)
}

/// sign-prepare.ts `signatureDict`: /Contents is 8192 zero bytes (written
/// as hex), /ByteRange the sentinel, /M the UTC time; certifying adds the
/// DocMDP reference with /P 2 (form filling and signing allowed).
fn signature(signing: &mut Signing, options: &Options<'_>) -> Result<ObjectId, AppError> {
    let mut dict = Dictionary::new();
    dict.insert(b"Type".to_vec(), name("Sig"));
    dict.insert(b"Filter".to_vec(), name("Adobe.PPKLite"));
    dict.insert(b"SubFilter".to_vec(), name("adbe.pkcs7.detached"));
    dict.insert(
        b"Contents".to_vec(),
        Object::String(vec![0; MAX_SIG_CONTENTS_BYTES]),
    );
    let range = [0, 1_000_000_000, 1_000_000_000, 1_000_000_000].map(Object::Integer);
    dict.insert(b"ByteRange".to_vec(), Object::array(range));
    dict.insert(b"Name".to_vec(), Object::text(options.name));
    dict.insert(b"M".to_vec(), Object::text(&pdf_date(options.when)));
    if !options.reason.is_empty() {
        dict.insert(b"Reason".to_vec(), Object::text(options.reason));
    }
    if options.certify {
        let params = Object::dictionary([
            ("Type", name("TransformParams")),
            ("P", Object::Integer(2)),
            ("V", name("1.2")),
        ]);
        let reference = Object::dictionary([
            ("Type", name("SigRef")),
            ("TransformMethod", name("DocMDP")),
            ("TransformParams", params),
        ]);
        dict.insert(b"Reference".to_vec(), Object::array([reference]));
    }
    signing.allocate(Object::Dictionary(dict)).map_err(failed)
}

/// engine-core.ts `WIN_ANSI_EXTRA`: the code points WinAnsi places in
/// 0x80..=0x9F.
const WIN_ANSI_EXTRA: [(u32, u8); 27] = [
    (0x20ac, 0x80),
    (0x201a, 0x82),
    (0x0192, 0x83),
    (0x201e, 0x84),
    (0x2026, 0x85),
    (0x2020, 0x86),
    (0x2021, 0x87),
    (0x02c6, 0x88),
    (0x2030, 0x89),
    (0x0160, 0x8a),
    (0x2039, 0x8b),
    (0x0152, 0x8c),
    (0x017d, 0x8e),
    (0x2018, 0x91),
    (0x2019, 0x92),
    (0x201c, 0x93),
    (0x201d, 0x94),
    (0x2022, 0x95),
    (0x2013, 0x96),
    (0x2014, 0x97),
    (0x02dc, 0x98),
    (0x2122, 0x99),
    (0x0161, 0x9a),
    (0x203a, 0x9b),
    (0x0153, 0x9c),
    (0x017e, 0x9e),
    (0x0178, 0x9f),
];

/// engine-core.ts `winAnsiBytes`: WinAnsi bytes, `?` for characters it
/// lacks, and `(`, `)` and `\` escaped for a literal string.
fn win_ansi(text: &str, out: &mut Vec<u8>) {
    for character in text.chars() {
        let code = u32::from(character);
        let byte = match code {
            0x20..=0x7e | 0xa0..=0xff => u8::try_from(code).unwrap_or(b'?'),
            _ => WIN_ANSI_EXTRA
                .iter()
                .find(|(point, _)| *point == code)
                .map_or(b'?', |(_, byte)| *byte),
        };
        if matches!(byte, b'(' | b')' | b'\\') {
            out.push(b'\\');
        }
        out.push(byte);
    }
}

/// A time value's calendar fields (proleptic Gregorian, as ECMAScript).
struct Civil {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
}

fn civil(ms: i64) -> Civil {
    let days = ms.div_euclid(86_400_000);
    let rest = ms.rem_euclid(86_400_000) / 1000;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    Civil {
        year,
        month,
        day,
        hour: rest / 3600,
        minute: rest / 60 % 60,
        second: rest % 60,
    }
}

/// sign-prepare.ts `localStamp`: `YYYY-MM-DD HH:MM` of `local` (the time
/// value shifted to the signer's zone).
fn local_stamp(local: i64) -> String {
    let t = civil(local);
    format!(
        "{}-{:02}-{:02} {:02}:{:02}",
        t.year, t.month, t.day, t.hour, t.minute
    )
}

/// sign-prepare.ts `pdfDate`: `D:YYYYMMDDHHmmSSZ` in UTC.
fn pdf_date(when: i64) -> String {
    let t = civil(when);
    format!(
        "D:{}{:02}{:02}{:02}{:02}{:02}Z",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    )
}

/// sign-prepare.ts `scanSignature`: each placeholder exactly once (past the
/// source in an incremental output), as `(contentsAt, byteRangeAt,
/// revisions)`; revisions count every /ByteRange of an incremental output.
pub(crate) fn scan(bytes: &[u8], source_length: usize) -> Option<(usize, usize, usize)> {
    let from = if source_length > 0 && bytes.starts_with(b"%PDF") {
        source_length
    } else {
        0
    };
    let tail = bytes.get(from..)?;
    let ranges: Vec<usize> = find_all(tail, SENTINEL).collect();
    let holes = contents_holes(tail);
    let ([range], [hole]) = (ranges.as_slice(), holes.as_slice()) else {
        return None;
    };
    let contents_at = from + hole + b"/Contents<".len();
    let revisions = match source_length {
        0 => 1,
        _ => find_all(bytes, b"/ByteRange").count(),
    };
    Some((contents_at, from + range, revisions))
}

fn find_all<'a>(haystack: &'a [u8], needle: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    haystack
        .windows(needle.len())
        .enumerate()
        .filter(move |(_, window)| *window == needle)
        .map(|(at, _)| at)
}

/// Offsets of `/Contents<` followed by exactly the hole's zeros and `>`.
/// A candidate stops at its first byte that is not `0`, so the scan is
/// linear.
fn contents_holes(bytes: &[u8]) -> Vec<usize> {
    let digits = MAX_SIG_CONTENTS_BYTES * 2;
    find_all(bytes, b"/Contents<")
        .filter(|&at| {
            let start = at + b"/Contents<".len();
            let zeros = bytes
                .get(start..)
                .unwrap_or_default()
                .iter()
                .take(digits)
                .take_while(|&&b| b == b'0')
                .count();
            zeros == digits && bytes.get(start + digits) == Some(&b'>')
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_follow_ecmascript() {
        assert_eq!(pdf_date(0), "D:19700101000000Z");
        // 2026-10-14T12:00:00Z, as the probe outputs show.
        let when = 1_791_979_200_000;
        assert_eq!(pdf_date(when), "D:20261014120000Z");
        assert_eq!(local_stamp(when - 5 * 3_600_000), "2026-10-14 07:00");
        assert_eq!(pdf_date(-1), "D:19691231235959Z");
        assert_eq!(local_stamp(951_782_400_000), "2000-02-29 00:00");
        assert_eq!(pdf_date(-8_640_000_000_000_000), "D:-2718210420000000Z");
        assert_eq!(pdf_date(8_640_000_000_000_000), "D:2757600913000000Z");
    }

    #[test]
    fn win_ansi_escapes_and_replaces() {
        let mut out = Vec::new();
        win_ansi("Zoë (QA) — € \\ 日", &mut out);
        assert_eq!(out, b"Zo\xEB \\(QA\\) \x97 \x80 \\\\ ?".to_vec());
    }

    #[test]
    fn the_scan_wants_each_placeholder_once() {
        let hole = format!("/Contents<{}>", "0".repeat(16384));
        let sentinel = String::from_utf8_lossy(SENTINEL).into_owned();
        let one = format!("%PDF-1.7 x{hole}/ByteRange{sentinel}");
        let at = one.find('<').map(|i| i + 1);
        let range = one.find('[');
        let found = scan(one.as_bytes(), 0);
        assert_eq!(
            found.map(|(c, r, n)| (Some(c), Some(r), n)),
            Some((at, range, 1))
        );
        let twice = format!("{one}{hole}{sentinel}");
        assert_eq!(scan(twice.as_bytes(), 0), None);
        // Past the source only the second pair counts.
        let past = scan(twice.as_bytes(), one.len());
        assert_eq!(past.map(|(c, _, n)| (c > one.len(), n)), Some((true, 1)));
        let later = format!("{one}/ByteRange[0 1 2 3]{hole}{sentinel}");
        assert_eq!(
            scan(later.as_bytes(), one.len()).map(|(_, _, n)| n),
            Some(2)
        );
        assert_eq!(scan(b"", 0), None);
        let short = format!("/Contents<{}>{sentinel}", "0".repeat(16383));
        assert_eq!(scan(short.as_bytes(), 0), None);
    }
}

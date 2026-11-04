// SPDX-License-Identifier: MIT
//! Stamps as things that can be found again: the record of each stamp's
//! settings in the catalog, the streams that draw it on each page, the
//! `stamps` listing and `stamp-remove`.
//!
//! The record is a page-piece dictionary (ISO 32000-2 14.5) on the catalog:
//! `/PieceInfo << /TandemPDF << /LastModified (D:…) /Private << /Stamps
//! [ << /Id 3 /Kind /Watermark /Text (DRAFT) … >> … ] >> >> >>`, one entry
//! per stamp, keyed by the `/TandemStamp` number of its marked content.
//!
//! A stamp's stream is one of the two shapes `stamp_pages` writes: on top,
//! `Q\n<marked content>\n` after a `q\n` stream put first; behind,
//! `q\n<marked content>\nQ\n` put first. A stream whose page has no record
//! (a file from another tool that kept the content but not the catalog
//! entry) is still found by its `/TandemStamp` number, and a stream from an
//! earlier version of the app (`/PDF4All <</Kind /Watermark>> BDC`, no
//! number) is listed as `legacy-watermark` (or `-header`, `-footer`).
use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use super::{
    Align, FONT_KEY, GS_PREFIX, StampAction, StampKind, kept_contents, resource_holder,
    win_ansi_char,
};
use crate::app::edit::{Place, get};
use crate::app::{AppError, AppSession, Transaction, info, reply};
use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId, decode_text_string};
use crate::stream::StreamLimits;

/// The catalog's page-piece key and the private key of the marked content.
const PIECE: &[u8] = b"TandemPDF";
/// A stamp stream is at most this long; longer streams are page content.
const MAX_STAMP_STREAM: usize = 16 << 10;
/// The most ids one `replace` or `stamp-remove` names.
const MAX_IDS: usize = 1000;

fn invalid(message: impl Into<String>) -> AppError {
    AppError::reason("invalid", message)
}

/// A stamp as requests name it: its recorded number, or the one legacy
/// stamp of a kind (every unnumbered stream of that kind).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StampId {
    Number(i64),
    Legacy(StampKind),
}

impl StampId {
    fn json(self) -> Value {
        match self {
            Self::Number(n) => n.into(),
            Self::Legacy(kind) => format!("legacy-{}", kind.name()).into(),
        }
    }

    fn parse(value: &Value) -> Option<Self> {
        if let Some(text) = value.as_str() {
            let kind = text.strip_prefix("legacy-")?;
            return StampKind::parse(kind).map(Self::Legacy);
        }
        crate::app::request::as_int(value)
            .filter(|n| *n >= 1)
            .map(Self::Number)
    }
}

/// What one stamp stream says.
#[derive(Clone, Debug, PartialEq)]
struct Drawn {
    id: StampId,
    kind: StampKind,
    behind: bool,
    size: Option<f64>,
    color: Option<[u8; 3]>,
    opacity: Option<u8>,
    /// The drawn text (placeholders filled in).
    text: String,
}

/// `needle`'s first position in `hay`.
fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// A number token as a stamp writes it.
fn number(token: &[u8]) -> Option<f64> {
    std::str::from_utf8(token)
        .ok()?
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
}

/// The stamp a content stream draws, when it is exactly one stamp stream.
fn parse(data: &[u8]) -> Option<Drawn> {
    // As written, `Q\n…EMC\n` or `q\n…EMC\nQ\n`; white space may differ in a
    // file another tool rewrote.
    let data = data.trim_ascii();
    let behind = match data.first()? {
        b'Q' => false,
        b'q' => true,
        _ => return None,
    };
    if !data.get(1)?.is_ascii_whitespace() {
        return None;
    }
    let rest = data[1..].trim_ascii_start();
    let body = if behind {
        let rest = rest.strip_suffix(b"Q")?;
        if !rest.last()?.is_ascii_whitespace() {
            return None;
        }
        rest.trim_ascii_end().strip_suffix(b"EMC")?
    } else {
        rest.strip_suffix(b"EMC")?
    };
    let (kind, id, draw) = if let Some(after) = body.strip_prefix(b"/Artifact <<") {
        let end = find(after, b">> BDC ")?;
        let tokens: Vec<&[u8]> = after[..end]
            .split(u8::is_ascii_whitespace)
            .filter(|t| !t.is_empty())
            .collect();
        let value = |key: &[u8]| tokens.windows(2).find(|w| w[0] == key).map(|w| w[1]);
        let kind = value(b"/Subtype")
            .and_then(|name| name.strip_prefix(b"/"))
            .and_then(StampKind::from_tag)?;
        let id = value(b"/TandemStamp")
            .and_then(number)
            .filter(|n| n.fract() == 0.0 && *n >= 1.0)?;
        (kind, StampId::Number(id as i64), &after[end + 7..])
    } else if let Some(after) = body.strip_prefix(b"/PDF4All <</Kind /") {
        let end = find(after, b">> BDC ")?;
        let kind = StampKind::from_tag(&after[..end])?;
        (kind, StampId::Legacy(kind), &after[end + 7..])
    } else {
        return None;
    };
    let mut drawn = Drawn {
        id,
        kind,
        behind,
        size: None,
        color: None,
        opacity: None,
        text: String::new(),
    };
    let mut draw = draw;
    if let Some(after) = draw.strip_prefix(format!("/{GS_PREFIX}").as_bytes()) {
        let end = find(after, b" gs ")?;
        drawn.opacity = std::str::from_utf8(&after[..end])
            .ok()
            .and_then(|p| p.parse().ok());
        draw = &after[end + 4..];
    }
    let bt = find(draw, b" BT ")?;
    let fill: Vec<f64> = draw[..bt]
        .split(|b| *b == b' ')
        .filter_map(number)
        .collect();
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    drawn.color = match fill.as_slice() {
        [g] => Some([byte(*g); 3]),
        [r, g, b] => Some([byte(*r), byte(*g), byte(*b)]),
        _ => None,
    };
    let after = draw[bt + 4..].strip_prefix(b"/PDF4AllF ")?;
    let tf = find(after, b" Tf ")?;
    drawn.size = number(&after[..tf]);
    let open = find(after, b" Tm (")?;
    let text = after[open + 5..]
        .trim_ascii_end()
        .strip_suffix(b") Tj ET")?;
    let mut escaped = false;
    for &byte in text {
        if byte == b'\\' && !escaped {
            escaped = true;
            continue;
        }
        escaped = false;
        drawn.text.push(win_ansi_char(byte));
    }
    Some(drawn)
}

/// A content stream's bytes when it is short enough to be a stamp's.
fn small_stream(doc: &Document<'_>, entry: &Object) -> Option<Vec<u8>> {
    let id = entry.as_reference()?;
    let limits = StreamLimits {
        max_encoded_bytes: MAX_STAMP_STREAM,
        max_decoded_bytes: MAX_STAMP_STREAM,
        max_filters: 4,
        max_total_decoded_bytes: MAX_STAMP_STREAM,
    };
    doc.decoded_stream(id, limits).ok()
}

/// One page's /Contents entries, read: a stamp stream, the `q\n` head of a
/// stamp on top, or anything else.
enum Entry {
    Stamp(Drawn),
    Head,
    Other,
}

fn entries(doc: &Document<'_>, kept: &[Object]) -> Vec<Entry> {
    kept.iter()
        .map(|entry| match small_stream(doc, entry) {
            Some(data) if data.trim_ascii() == b"q" => Entry::Head,
            Some(data) => parse(&data).map_or(Entry::Other, Entry::Stamp),
            None => Entry::Other,
        })
        .collect()
}

/// Every stamp stream of the document: (page index, stream).
fn scan(doc: &Document<'_>) -> Vec<(usize, Drawn)> {
    let mut found = Vec::new();
    for (index, page) in info::pages(doc).iter().enumerate() {
        if page.id.is_none() {
            continue;
        }
        let Some(kept) = kept_contents(doc, page.dict) else {
            continue;
        };
        for entry in entries(doc, kept) {
            if let Entry::Stamp(drawn) = entry {
                found.push((index, drawn));
            }
        }
    }
    found
}

/// The catalog's `/PieceInfo /TandemPDF /Private /Stamps` entries: (id,
/// record).
fn records<'d>(doc: &'d Document<'_>) -> Vec<(i64, &'d Dictionary)> {
    let catalog = doc.resolve_id(doc.root());
    let piece = get(doc, get(doc, catalog, b"PieceInfo"), PIECE);
    let list = get(doc, get(doc, piece, b"Private"), b"Stamps");
    (list.as_array().unwrap_or(&[]).iter())
        .filter_map(|record| {
            let record = doc.resolve(record).as_dict()?;
            let id = doc.dict_get(record, b"Id").as_i64().filter(|n| *n >= 1)?;
            Some((id, record))
        })
        .collect()
}

/// The next stamp number: past every recorded or drawn one.
pub(super) fn next_id(doc: &Document<'_>) -> i64 {
    let recorded = records(doc).iter().map(|(id, _)| *id).max().unwrap_or(0);
    let drawn = (scan(doc).iter())
        .filter_map(|(_, drawn)| match drawn.id {
            StampId::Number(n) => Some(n),
            StampId::Legacy(_) => None,
        })
        .max()
        .unwrap_or(0);
    recorded.max(drawn).saturating_add(1)
}

/// Now as a PDF date in UTC, for a request without `modified`.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let date = super::super::annots::civil_date(i64::try_from(secs / 86_400).unwrap_or(0));
    let rest = secs % 86_400;
    format!(
        "D:{}{}{}{:02}{:02}{:02}Z",
        &date[..4],
        &date[5..7],
        &date[8..10],
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

#[cfg(target_arch = "wasm32")]
pub(super) fn now() -> String {
    "D:19700101000000Z".into()
}

fn align_name(align: Align) -> &'static str {
    match align {
        Align::Left => "Left",
        Align::Center => "Center",
        Align::Right => "Right",
    }
}

/// The record of a stamp: what its request said, nothing more.
fn record_of(id: i64, action: &StampAction, pages: Option<&[usize]>) -> Object {
    let mut record = Dictionary::new();
    let mut put = |key: &str, value: Object| {
        record.insert(key.as_bytes().to_vec(), value);
    };
    put("Id", Object::Integer(id));
    put("Kind", Object::name(action.kind.tag()));
    put("Text", Object::text(&action.text));
    if let Some(align) = action.align {
        put("Align", Object::name(align_name(align)));
    }
    if let Some(size) = action.size {
        put("Size", Object::Integer(size));
    }
    if let Some([r, g, b]) = action.color {
        put("Color", Object::text(&format!("#{r:02x}{g:02x}{b:02x}")));
    }
    if let Some(opacity) = action.opacity {
        put("Opacity", Object::Integer(i64::from(opacity)));
    }
    if action.behind {
        put("Behind", Object::Boolean(true));
    }
    if let Some(pages) = pages {
        let pages = pages.iter().map(|&p| Object::Integer(p as i64));
        put("Pages", Object::array(pages));
    }
    if let Some(start) = action.bates_start {
        put("BatesStart", Object::Integer(start));
    }
    if let Some(pad) = action.bates_pad {
        put("BatesPad", Object::Integer(pad));
    }
    Object::Dictionary(record)
}

/// Makes `key` of the dictionary at `place` a dictionary when it is not one.
fn ensure_dict(tx: &mut Transaction<'_>, place: &Place, key: &[u8]) -> Result<(), AppError> {
    super::ensure_dict(tx, place, key)
}

/// Rewrites the record list with `edit`, stamping /LastModified; an empty
/// list takes the /TandemPDF entry (and an emptied /PieceInfo) away.
fn edit_records(
    tx: &mut Transaction<'_>,
    modified: &str,
    edit: impl FnOnce(&mut Vec<Object>),
) -> Result<(), AppError> {
    let catalog = Place::object(tx.document().root());
    ensure_dict(tx, &catalog, b"PieceInfo")?;
    let piece_info = catalog.child(tx.document(), b"PieceInfo");
    ensure_dict(tx, &piece_info, PIECE)?;
    let piece = piece_info.child(tx.document(), PIECE);
    ensure_dict(tx, &piece, b"Private")?;
    let private = piece.child(tx.document(), b"Private");
    let mut list = get(tx.document(), private.read(tx.document()), b"Stamps")
        .as_array()
        .map(<[Object]>::to_vec)
        .unwrap_or_default();
    edit(&mut list);
    if list.is_empty() {
        piece_info.update(tx, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.remove(PIECE);
            }
        })?;
        let empty =
            (piece_info.read(tx.document()).as_dict()).is_some_and(|d| d.keys().count() == 0);
        if empty {
            catalog.update(tx, |value| {
                if let Some(dict) = value.as_dict_mut() {
                    dict.remove(b"PieceInfo".as_slice());
                }
            })?;
        }
        return Ok(());
    }
    private.update(tx, |value| {
        if let Some(dict) = value.as_dict_mut() {
            dict.insert(b"Stamps".to_vec(), Object::Array(list));
        }
    })?;
    piece.update(tx, |value| {
        if let Some(dict) = value.as_dict_mut() {
            dict.insert(b"LastModified".to_vec(), Object::text(modified));
        }
    })?;
    Ok(())
}

/// Records a new stamp (replacing a record with its id).
pub(super) fn record(
    tx: &mut Transaction<'_>,
    id: i64,
    action: &StampAction,
    pages: Option<&[usize]>,
    modified: &str,
) -> Result<(), AppError> {
    let entry = record_of(id, action, pages);
    let doc = tx.document();
    let same = |item: &Object| {
        doc.resolve(item)
            .as_dict()
            .is_some_and(|d| doc.dict_get(d, b"Id").as_i64() == Some(id))
    };
    let keep: Vec<bool> = {
        let catalog = doc.resolve_id(doc.root());
        let piece = get(doc, get(doc, catalog, b"PieceInfo"), PIECE);
        let list = get(doc, get(doc, piece, b"Private"), b"Stamps");
        list.as_array()
            .unwrap_or(&[])
            .iter()
            .map(|item| !same(item))
            .collect()
    };
    edit_records(tx, modified, |list| {
        let mut keep = keep.into_iter();
        list.retain(|_| keep.next().unwrap_or(true));
        list.push(entry);
    })
}

/// The ids a request names: one id or a list of them.
pub(super) fn parse_ids(value: &Value, op: &str) -> Result<Vec<StampId>, AppError> {
    let bad = || {
        invalid(format!(
            "{op}: invalid-stamp: a stamp id is a number or legacy-watermark, legacy-header or legacy-footer"
        ))
    };
    let mut ids = match value {
        Value::Array(items) if (1..=MAX_IDS).contains(&items.len()) => items
            .iter()
            .map(|item| StampId::parse(item).ok_or_else(bad))
            .collect::<Result<Vec<_>, _>>()?,
        Value::Array(_) => return Err(bad()),
        value => vec![StampId::parse(value).ok_or_else(bad)?],
    };
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// `parse_ids`, each of which must name a stamp the document has.
pub(super) fn check_ids(
    doc: &Document<'_>,
    value: &Value,
    op: &str,
) -> Result<Vec<StampId>, AppError> {
    let ids = parse_ids(value, op)?;
    let known: BTreeSet<StampId> = listing(doc).into_iter().map(|s| s.id).collect();
    if let Some(id) = ids.iter().find(|id| !known.contains(id)) {
        return Err(invalid(format!(
            "{op}: unknown-stamp: the document has no stamp {}",
            id.json()
        )));
    }
    Ok(ids)
}

/// One stamp of the listing.
struct Listed<'d> {
    id: StampId,
    kind: StampKind,
    record: Option<&'d Dictionary>,
    /// The first stream found (for a stamp without a record).
    drawn: Option<Drawn>,
    pages: Vec<usize>,
}

/// Every stamp the document draws, by id; a record whose stamp is on no page
/// is left out.
fn listing<'d>(doc: &'d Document<'_>) -> Vec<Listed<'d>> {
    let recorded: BTreeMap<i64, &Dictionary> = records(doc).into_iter().collect();
    let mut stamps: BTreeMap<StampId, Listed<'d>> = BTreeMap::new();
    for (page, drawn) in scan(doc) {
        let record = match drawn.id {
            StampId::Number(n) => recorded.get(&n).copied(),
            StampId::Legacy(_) => None,
        };
        let listed = stamps.entry(drawn.id).or_insert_with(|| Listed {
            id: drawn.id,
            kind: record
                .and_then(|r| doc.dict_get(r, b"Kind").as_name())
                .and_then(StampKind::from_tag)
                .unwrap_or(drawn.kind),
            record,
            drawn: None,
            pages: Vec::new(),
        });
        if listed.pages.last() != Some(&page) {
            listed.pages.push(page);
        }
        if listed.drawn.is_none() {
            listed.drawn = Some(drawn);
        }
    }
    stamps.into_values().collect()
}

fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

/// A listed stamp as `stamps` replies it.
fn listed_json(doc: &Document<'_>, listed: &Listed<'_>) -> Value {
    let mut out = json!({
        "id": listed.id.json(),
        "kind": listed.kind.name(),
        "recorded": listed.record.is_some(),
        "found": listed.pages,
    });
    if let Some(record) = listed.record {
        let int = |key: &[u8]| doc.dict_get(record, key).as_i64();
        let text = (doc.dict_get(record, b"Text").as_string()).map(decode_text_string);
        out["text"] = text.unwrap_or_default().into();
        if let Some(align) = doc.dict_get(record, b"Align").as_name() {
            out["align"] = String::from_utf8_lossy(align).to_ascii_lowercase().into();
        }
        if let Some(size) = int(b"Size") {
            out["size"] = size.into();
        }
        if let Some(color) = doc.dict_get(record, b"Color").as_string() {
            out["color"] = decode_text_string(color).into();
        }
        if let Some(opacity) = int(b"Opacity") {
            out["opacity"] = json!(opacity as f64 / 100.0);
        }
        out["behind"] = (doc.dict_get(record, b"Behind").as_bool() == Some(true)).into();
        if let Some(pages) = doc.dict_get(record, b"Pages").as_array() {
            let pages: Vec<i64> = pages.iter().filter_map(Object::as_i64).collect();
            out["pages"] = pages.into();
        }
        if let Some(start) = int(b"BatesStart") {
            out["batesStart"] = start.into();
        }
        if let Some(pad) = int(b"BatesPad") {
            out["batesPad"] = pad.into();
        }
    } else if let Some(drawn) = &listed.drawn {
        // No record: what the first stream draws (its text as drawn there).
        out["text"] = drawn.text.clone().into();
        if let Some(size) = drawn.size {
            out["size"] = if size.fract() == 0.0 {
                json!(size as i64)
            } else {
                json!(size)
            };
        }
        if let Some(color) = drawn.color {
            out["color"] = hex(color).into();
        }
        if let Some(opacity) = drawn.opacity {
            out["opacity"] = json!(f64::from(opacity) / 100.0);
        }
        out["behind"] = drawn.behind.into();
    }
    out
}

/// `stamps`: every header, footer and watermark the document draws, by id
/// (`{items: [{id, kind, recorded, found, text, size?, color?, opacity?,
/// behind, align?, pages?, batesStart?, batesPad?}]}`): `found` is the
/// 0-based pages it is on; a recorded stamp says what its request said, an
/// unrecorded one what its first stream draws.
pub fn list(session: &AppSession, request: &Value) -> Result<Value, AppError> {
    let Some(doc) = &session.doc else {
        return Err(AppError::plain("stamps: no document open"));
    };
    if session.locked {
        return Err(AppError::plain(
            "stamps: document is encrypted, a password is required",
        ));
    }
    let doc = doc.document();
    let items: Vec<Value> = (listing(doc).iter())
        .map(|listed| listed_json(doc, listed))
        .collect();
    Ok(reply(request, json!({ "items": items })))
}

/// `stamp-remove {id}`: takes the stamp (or each of a list) off every page
/// it is on, drops its record and the stamp font and opacity resources no
/// content uses any more, in one undoable step (`remove watermark`, …, or
/// `remove stamps` for several kinds).
pub fn remove(session: &mut AppSession, request: &Value) -> Result<Value, AppError> {
    let doc = session.require_mutable("stamp-remove")?;
    let modified = super::check_modified(request, "stamp-remove")?.map_or_else(now, str::to_string);
    let value = request
        .get("id")
        .ok_or_else(|| invalid("stamp-remove: invalid-stamp: id is required"))?;
    let ids = check_ids(doc.document(), value, "stamp-remove")?;
    let kinds: BTreeSet<StampKind> = (listing(doc.document()).iter())
        .filter(|listed| ids.contains(&listed.id))
        .map(|listed| listed.kind)
        .collect();
    let step = match kinds.iter().next() {
        Some(kind) if kinds.len() == 1 => kind.remove_step(),
        _ => "remove stamps",
    };
    session.mutate(step, |session| {
        let mut tx = session.tx()?;
        let touched = take_off(&mut tx, &ids, &modified)?;
        drop_unused_resources(&mut tx, &touched)
    })?;
    let mut fields = session.edit_state();
    let removed: Vec<Value> = ids.iter().map(|id| id.json()).collect();
    fields.insert("removed".into(), removed.into());
    Ok(reply(request, Value::Object(fields)))
}

/// The resource names a stamp stream draws with.
fn names_of(drawn: &Drawn) -> Vec<Vec<u8>> {
    let mut names = vec![FONT_KEY.to_vec()];
    if let Some(opacity) = drawn.opacity {
        names.push(format!("{GS_PREFIX}{opacity}").into_bytes());
    }
    names
}

/// A page a removal changed and the resource names its removed streams drew
/// with.
pub(super) type Touched = Vec<(ObjectId, BTreeSet<Vec<u8>>)>;

/// Takes the streams of `ids` (and one `q` head per stamp on top) out of
/// every page's /Contents and their records out of the catalog. The
/// resources stay until `drop_unused_resources`, so a replacing stamp can
/// reuse them.
pub(super) fn take_off(
    tx: &mut Transaction<'_>,
    ids: &[StampId],
    modified: &str,
) -> Result<Touched, AppError> {
    let doc = tx.document();
    let mut changes = Vec::new();
    for page in info::pages(doc) {
        let Some(page_id) = page.id else {
            continue;
        };
        let Some(kept) = kept_contents(doc, page.dict) else {
            continue;
        };
        let read = entries(doc, kept);
        let mut drop = vec![false; kept.len()];
        let mut heads = 0usize;
        let mut names = BTreeSet::new();
        for (index, entry) in read.iter().enumerate() {
            if let Entry::Stamp(drawn) = entry
                && ids.contains(&drawn.id)
            {
                drop[index] = true;
                heads += usize::from(!drawn.behind);
                names.extend(names_of(drawn));
            }
        }
        if names.is_empty() {
            continue;
        }
        // The heads of stamps on top lead /Contents, among streams behind.
        for (index, entry) in read.iter().enumerate() {
            if heads == 0 {
                break;
            }
            match entry {
                Entry::Head => {
                    drop[index] = true;
                    heads -= 1;
                }
                Entry::Stamp(drawn) if drawn.behind => {}
                _ => break,
            }
        }
        let contents: Vec<Object> = (kept.iter().zip(&drop))
            .filter(|(_, dropped)| !**dropped)
            .map(|(entry, _)| entry.clone())
            .collect();
        changes.push((page_id, contents, names));
    }
    let mut touched = Vec::new();
    for (page_id, contents, names) in changes {
        tx.update(page_id, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.insert(b"Contents".to_vec(), Object::Array(contents));
            }
        })?;
        touched.push((page_id, names));
    }
    let numbers: BTreeSet<i64> = (ids.iter())
        .filter_map(|id| match id {
            StampId::Number(n) => Some(*n),
            StampId::Legacy(_) => None,
        })
        .collect();
    let doc = tx.document();
    let recorded: Vec<bool> = {
        let catalog = doc.resolve_id(doc.root());
        let piece = get(doc, get(doc, catalog, b"PieceInfo"), PIECE);
        let list = get(doc, get(doc, piece, b"Private"), b"Stamps");
        (list.as_array().unwrap_or(&[]).iter())
            .map(|item| {
                (doc.resolve(item).as_dict())
                    .and_then(|d| doc.dict_get(d, b"Id").as_i64())
                    .is_some_and(|id| numbers.contains(&id))
            })
            .collect()
    };
    if recorded.contains(&true) {
        edit_records(tx, modified, |list| {
            let mut gone = recorded.into_iter();
            list.retain(|_| !gone.next().unwrap_or(false));
        })?;
    }
    Ok(touched)
}

/// Whether `data` uses the name `/name` (followed by a delimiter, white
/// space or the end).
fn uses(data: &[u8], name: &[u8]) -> bool {
    let mut token = Vec::with_capacity(name.len() + 1);
    token.push(b'/');
    token.extend_from_slice(name);
    let mut at = 0;
    while let Some(found) = find(&data[at..], &token) {
        let end = at + found + token.len();
        let regular = data
            .get(end)
            .is_some_and(|b| !b.is_ascii_whitespace() && !b"()<>[]{}/%\0".contains(b));
        if !regular {
            return true;
        }
        at = end;
    }
    false
}

/// Whether any content stream of the page names `/name`.
fn page_uses(doc: &Document<'_>, page: &Dictionary, name: &[u8]) -> bool {
    let Some(kept) = kept_contents(doc, page) else {
        return true;
    };
    kept.iter().any(|entry| {
        let Some(id) = entry.as_reference() else {
            return false;
        };
        match doc.decoded_stream(id, StreamLimits::default()) {
            Ok(data) => uses(&data, name),
            // A stream that cannot be read might use it.
            Err(_) => true,
        }
    })
}

/// Where the page's `category` resources live, when it has them.
fn category_place(doc: &Document<'_>, page: ObjectId, category: &[u8]) -> Option<Place> {
    let holder = resource_holder(doc, page);
    let resources = holder.child(doc, b"Resources");
    resources.read(doc).as_dict()?;
    let entries = resources.child(doc, category);
    entries.read(doc).as_dict()?;
    Some(entries)
}

/// After a removal, takes each stamp resource name the removed streams drew
/// with out of the resources that hold it, unless a page drawing with those
/// resources still names it (another stamp on the page, or a stamp on a
/// page sharing them). An emptied /ExtGState or /Font written into the
/// /Resources goes too; the font and opacity objects are left for the
/// save's garbage collection.
pub(super) fn drop_unused_resources(
    tx: &mut Transaction<'_>,
    touched: &Touched,
) -> Result<(), AppError> {
    let mut wanted: Vec<(Place, Vec<u8>, Vec<u8>)> = Vec::new();
    let doc = tx.document();
    for (page, names) in touched {
        for name in names {
            let category: &[u8] = if name.as_slice() == FONT_KEY {
                b"Font"
            } else {
                b"ExtGState"
            };
            if let Some(place) = category_place(doc, *page, category)
                && place
                    .read(doc)
                    .as_dict()
                    .is_some_and(|d| d.get(name.as_slice()).is_some())
            {
                let item = (place, category.to_vec(), name.clone());
                if !wanted.contains(&item) {
                    wanted.push(item);
                }
            }
        }
    }
    if wanted.is_empty() {
        return Ok(());
    }
    let pages: Vec<(ObjectId, &Dictionary)> = (info::pages(doc).into_iter())
        .filter_map(|page| Some((page.id?, page.dict)))
        .collect();
    let mut unused = Vec::new();
    for (place, category, name) in wanted {
        let used = pages.iter().any(|(id, dict)| {
            category_place(doc, *id, &category).as_ref() == Some(&place)
                && page_uses(doc, dict, &name)
        });
        if !used {
            unused.push((place, name));
        }
    }
    for (place, name) in unused {
        place.update(tx, |value| {
            if let Some(dict) = value.as_dict_mut() {
                dict.remove(name.as_slice());
            }
        })?;
        // An emptied category written into its /Resources goes too.
        let empty = place
            .read(tx.document())
            .as_dict()
            .is_some_and(|d| d.keys().count() == 0);
        if empty && let Some((key, parent)) = place.keys.split_last() {
            let parent = Place {
                id: place.id,
                keys: parent.to_vec(),
            };
            parent.update(tx, |value| {
                if let Some(dict) = value.as_dict_mut() {
                    dict.remove(key.as_slice());
                }
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_streams_parse_back() {
        let top = b"Q\n/Artifact <</Type /Pagination /Subtype /Footer /TandemStamp 12>> BDC /PDF4AllG50 gs 1 0 0 rg BT /PDF4AllF 10 Tf 1 0 0 1 36 36 Tm (A \\(b\\) \x80) Tj ET EMC\n";
        let drawn = parse(top).unwrap();
        assert_eq!(drawn.id, StampId::Number(12));
        assert_eq!(drawn.kind, StampKind::Footer);
        assert!(!drawn.behind);
        assert_eq!(drawn.opacity, Some(50));
        assert_eq!(drawn.color, Some([255, 0, 0]));
        assert_eq!(drawn.size, Some(10.0));
        assert_eq!(drawn.text, "A (b) \u{20ac}");
        let behind = b"q\n/Artifact <</Type /Pagination /Subtype /Watermark /TandemStamp 1>> BDC 0.5 g BT /PDF4AllF 48 Tf 1 1 -1 1 9 9 Tm (DRAFT) Tj ET EMC\nQ\n";
        let drawn = parse(behind).unwrap();
        assert!(drawn.behind);
        assert_eq!(drawn.color, Some([128; 3]));
        let legacy = b"Q\n/PDF4All <</Kind /Header>> BDC 0 g BT /PDF4AllF 10 Tf 1 0 0 1 36 756 Tm (x) Tj ET EMC\n";
        assert_eq!(
            parse(legacy).unwrap().id,
            StampId::Legacy(StampKind::Header)
        );
        // Someone else's artifact, or no number: not a stamp.
        assert!(parse(b"Q\n/Artifact <</Type /Pagination /Subtype /Footer>> BDC 0 g BT /PDF4AllF 10 Tf 1 0 0 1 3 3 Tm (x) Tj ET EMC\n").is_none());
        assert!(parse(b"q\n").is_none());
    }

    #[test]
    fn names_are_found_as_whole_tokens() {
        assert!(uses(b"/PDF4AllG30 gs", b"PDF4AllG30"));
        assert!(!uses(b"/PDF4AllG300 gs", b"PDF4AllG30"));
        assert!(uses(b"x/PDF4AllF", b"PDF4AllF"));
        assert!(uses(b"/PDF4AllF/F1", b"PDF4AllF"));
        assert!(!uses(b"/PDF4AllFx", b"PDF4AllF"));
    }

    #[test]
    fn ids_parse() {
        let ids = parse_ids(&json!([3, "legacy-header", 3, 1]), "x").unwrap();
        assert_eq!(
            ids,
            [
                StampId::Number(1),
                StampId::Number(3),
                StampId::Legacy(StampKind::Header)
            ]
        );
        assert!(parse_ids(&json!(0), "x").is_err());
        assert!(parse_ids(&json!("legacy-x"), "x").is_err());
        assert!(parse_ids(&json!([]), "x").is_err());
    }
}

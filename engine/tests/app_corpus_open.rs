// SPDX-License-Identifier: AGPL-3.0-or-later
//! Opening, repair and saving of damaged inputs the external corpus found
//! (engine/design/app-corpus-open.md). Every file here is synthetic and
//! reproduces one structure: allocation past MuPDF's object-number limit,
//! encrypted object streams the repair reads, streams naming an external
//! file, malformed encryption dictionaries, lexical damage MuPDF reads
//! through, an update that frees an object, and saves of page trees MuPDF
//! would read differently from the app.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use md5::{Digest, Md5};
use serde_json::{Value, json};
use tandem_pdf_engine::app::document::{limits, open_options};
use tandem_pdf_engine::app::{AppDocument, AppEngine, AppSession, save_tree};
use tandem_pdf_engine::document::{Document, DocumentErrorKind, DocumentLimits};
use tandem_pdf_engine::object::{Object, ObjectId};
use tandem_pdf_engine::repair::{RepairLimits, rebuild, rebuild_with_passwords};

const MAX_OBJECT_NUMBER: u32 = 8_388_607;

fn id(n: u32) -> ObjectId {
    ObjectId::new(n, 0).unwrap()
}

/// A file assembled object by object, then closed by an xref.
struct Pdf {
    bytes: Vec<u8>,
    offsets: BTreeMap<u32, usize>,
}

impl Pdf {
    fn new() -> Self {
        Self {
            bytes: b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec(),
            offsets: BTreeMap::new(),
        }
    }
    fn object(&mut self, number: u32, body: &[u8]) -> &mut Self {
        self.offsets.insert(number, self.bytes.len());
        self.bytes
            .extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        self.bytes.extend_from_slice(body);
        self.bytes.extend_from_slice(b"\nendobj\n");
        self
    }
    /// `dictionary` is the stream dictionary without its closing `>>`.
    fn stream(&mut self, number: u32, dictionary: &str, data: &[u8]) -> &mut Self {
        let mut body = format!("{dictionary}/Length {}>>\nstream\n", data.len()).into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\nendstream");
        self.object(number, &body)
    }
    fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(bytes);
        self
    }
    /// A classic table, one subsection per run of numbers.
    fn classic(&self, trailer: &str) -> Vec<u8> {
        let mut bytes = self.bytes.clone();
        let xref = bytes.len();
        let size = self
            .offsets
            .keys()
            .next_back()
            .map_or(1, |n| u64::from(*n) + 1);
        bytes.extend_from_slice(b"xref\n0 1\n0000000000 65535 f \n");
        let numbers: Vec<u32> = self.offsets.keys().copied().collect();
        let mut start = 0;
        while start < numbers.len() {
            let mut end = start + 1;
            while end < numbers.len() && numbers[end] == numbers[end - 1] + 1 {
                end += 1;
            }
            bytes.extend_from_slice(format!("{} {}\n", numbers[start], end - start).as_bytes());
            for number in &numbers[start..end] {
                bytes.extend_from_slice(
                    format!("{:010} 00000 n \n", self.offsets[number]).as_bytes(),
                );
            }
            start = end;
        }
        bytes.extend_from_slice(
            format!("trailer\n<</Size {size} {trailer}>>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        bytes
    }
    /// An xref stream (object `number`, unfiltered) listing the objects
    /// written and `members` (number, container, index); `startxref` names
    /// `startxref` when given, else the stream.
    fn xref_stream(
        &mut self,
        number: u32,
        members: &[(u32, u32, u16)],
        trailer: &str,
        startxref: Option<usize>,
    ) -> Vec<u8> {
        let at = self.bytes.len();
        self.offsets.insert(number, at);
        let size = (self.offsets.keys().copied())
            .chain(members.iter().map(|m| m.0))
            .max()
            .unwrap_or(0)
            + 1;
        let mut rows = Vec::new();
        for n in 0..size {
            let (kind, field, index) =
                match (self.offsets.get(&n), members.iter().find(|m| m.0 == n)) {
                    (Some(&offset), _) => (1u8, offset as u32, 0u16),
                    (None, Some(&(_, container, index))) => (2, container, index),
                    (None, None) => (0, 0, if n == 0 { 65535 } else { 0 }),
                };
            rows.push(kind);
            rows.extend_from_slice(&field.to_be_bytes());
            rows.extend_from_slice(&index.to_be_bytes());
        }
        let dictionary = format!("<</Type/XRef/Size {size}/W[1 4 2]{trailer}");
        self.stream(number, &dictionary, &rows);
        let mut bytes = self.bytes.clone();
        bytes.extend_from_slice(
            format!("startxref\n{}\n%%EOF\n", startxref.unwrap_or(at)).as_bytes(),
        );
        bytes
    }
}

/// An object stream's data and /First for `members` (number, body).
fn object_stream(members: &[(u32, &[u8])]) -> (Vec<u8>, usize) {
    let mut header = String::new();
    let mut body = Vec::new();
    for (number, member) in members {
        header.push_str(&format!("{number} {} ", body.len()));
        body.extend_from_slice(member);
        body.push(b'\n');
    }
    let first = header.len();
    let mut data = header.into_bytes();
    data.extend_from_slice(&body);
    (data, first)
}

fn zlib(data: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec_zlib(data, 6)
}

// ------------------------------------------------- standard security handler

const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];
const FILE_ID: &[u8; 16] = b"0123456789abcdef";

fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: Vec<u8> = (0..=255).collect();
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, j as usize);
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|byte| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[i as usize]);
            s.swap(i as usize, j as usize);
            byte ^ s[s[i as usize].wrapping_add(s[j as usize]) as usize]
        })
        .collect()
}

fn padded(password: &[u8]) -> Vec<u8> {
    password.iter().chain(&PAD).take(32).copied().collect()
}

fn md5(parts: &[&[u8]]) -> Vec<u8> {
    let mut hash = Md5::new();
    for part in parts {
        hash.update(part);
    }
    hash.finalize().to_vec()
}

/// A revision 3 handler (RC4, 128-bit key): /O, /U and the file key.
struct Handler {
    o: Vec<u8>,
    u: Vec<u8>,
    key: Vec<u8>,
}

impl Handler {
    fn r3(user: &[u8], owner: &[u8], permissions: i32) -> Self {
        let rounds = |key: &[u8], data: &[u8]| {
            (0..20u8).fold(data.to_vec(), |data, i| {
                rc4(&key.iter().map(|b| b ^ i).collect::<Vec<_>>(), &data)
            })
        };
        let mut owner_key = md5(&[&padded(owner)]);
        for _ in 0..50 {
            owner_key = md5(&[&owner_key]);
        }
        let o = rounds(&owner_key, &padded(user));
        let mut key = md5(&[&padded(user), &o, &permissions.to_le_bytes(), FILE_ID]);
        for _ in 0..50 {
            key = md5(&[&key]);
        }
        let mut u = rounds(&key, &md5(&[&PAD, FILE_ID]));
        u.extend_from_slice(&[0x5A; 16]);
        Self { o, u, key }
    }
    fn encrypt(&self, number: u32, data: &[u8]) -> Vec<u8> {
        let object = md5(&[&self.key, &number.to_le_bytes()[..3], &[0, 0]]);
        rc4(&object, data)
    }
    fn dictionary(&self, extra: &str) -> Vec<u8> {
        let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02X}")).collect::<String>();
        format!(
            "<</Filter/Standard/V 2/R 3/Length 128/O<{}>/U<{}>{extra}>>",
            hex(&self.o),
            hex(&self.u)
        )
        .into_bytes()
    }
}

fn id_entry() -> String {
    format!(
        "/ID[<{0}><{0}>]",
        FILE_ID
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<String>()
    )
}

// ------------------------------------------------------------ app sessions

fn dispatch(engine: &mut AppEngine, session: &mut AppSession, request: Value) -> Value {
    dispatch_with(engine, session, request, Vec::new()).0
}

fn dispatch_with(
    engine: &mut AppEngine,
    session: &mut AppSession,
    request: Value,
    buffers: Vec<Vec<u8>>,
) -> (Value, Vec<Vec<u8>>) {
    engine.dispatch(session, request, buffers)
}

fn open(bytes: Vec<u8>) -> (AppEngine, AppSession, Value) {
    let mut engine = AppEngine::new();
    let mut session = AppSession::default();
    let (reply, _) = dispatch_with(
        &mut engine,
        &mut session,
        json!({"op": "open", "name": "a.pdf", "pdf": {"$buffer": 0}}),
        vec![bytes],
    );
    (engine, session, reply)
}

fn text_all(engine: &mut AppEngine, session: &mut AppSession) -> String {
    let reply = dispatch(engine, session, json!({"op": "text-all"}));
    assert_eq!(reply["ok"], true, "{reply}");
    reply["text"].as_str().unwrap().to_string()
}

fn save(engine: &mut AppEngine, session: &mut AppSession) -> Vec<u8> {
    let (reply, mut outputs) = dispatch_with(
        engine,
        session,
        json!({"op": "save", "entropy": {"$buffer": 0}}),
        vec![vec![7; 32]],
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let bytes = outputs.remove(0);
    let (check, _) = dispatch_with(
        engine,
        session,
        json!({"op": "validate", "pdf": {"$buffer": 0}}),
        vec![bytes.clone()],
    );
    assert_eq!(check["ok"], true, "{check}");
    bytes
}

fn numbers(session: &AppSession) -> BTreeSet<u32> {
    let doc = session.doc.as_ref().unwrap().document();
    doc.objects().map(|object| object.id.number()).collect()
}

fn page_text(content: &str) -> Vec<u8> {
    format!("BT /F1 12 Tf 20 100 Td ({content}) Tj ET").into_bytes()
}

const FONT: &[u8] = b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica/Encoding/WinAnsiEncoding>>";

// ---------------------------------------------------------- allocation

/// A file holding object 2147483647 makes the floor pass MuPDF's limit:
/// new objects take the lowest numbers nothing holds or references, never a
/// dangling reference's, a deleted object's or one handed out before.
#[test]
fn allocation_past_the_object_number_limit_takes_free_low_numbers_once() {
    let mut pdf = Pdf::new();
    pdf.object(1, b"<</Type/Catalog/Pages 2 0 R>>")
        .object(2, b"<</Type/Pages/Kids[3 0 R]/Count 1>>")
        .object(
            3,
            b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Annots[5 0 R]/Lost 4 0 R>>",
        )
        .object(5, b"<</Type/Annot/Subtype/Square/Rect[10 10 50 50]>>")
        .object(2_147_483_647, b"<</Root 1 0 R>>");
    let (mut engine, mut session, reply) = open(pdf.classic("/Root 1 0 R"));
    assert_eq!(reply["ok"], true, "{reply}");
    let original = numbers(&session);
    let mut handed = BTreeSet::new();
    let mut add = |engine: &mut AppEngine, session: &mut AppSession| {
        let before = numbers(session);
        let reply = dispatch(
            engine,
            session,
            json!({"op": "annot-add", "page": 0, "type": "Square", "rect": [20, 20, 60, 60], "contents": ""}),
        );
        assert_eq!(reply["ok"], true, "{reply}");
        let added: BTreeSet<u32> = numbers(session).difference(&before).copied().collect();
        assert!(!added.is_empty());
        for &number in &added {
            assert!(number <= MAX_OBJECT_NUMBER, "{number}");
            assert!(number != 4, "a dangling reference names 4");
            assert!(!original.contains(&number), "{number} was in the file");
            assert!(handed.insert(number), "{number} handed out twice");
        }
    };
    add(&mut engine, &mut session);
    let deleted = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "annot-delete", "page": 0, "annotId": 5}),
    );
    assert_eq!(deleted["ok"], true, "{deleted}");
    add(&mut engine, &mut session);
    for _ in 0..3 {
        assert_eq!(
            dispatch(&mut engine, &mut session, json!({"op": "undo"}))["ok"],
            true
        );
    }
    assert!(numbers(&session).contains(&5));
    // A new branch after undo still takes numbers never handed out.
    add(&mut engine, &mut session);
    add(&mut engine, &mut session);
    let saved = save(&mut engine, &mut session);
    let doc = Document::open(&saved, DocumentLimits::default()).unwrap();
    assert!(
        doc.objects()
            .all(|object| object.id.number() <= MAX_OBJECT_NUMBER)
    );
    let annots = doc.dict_get(doc.resolve_id(id(3)).as_dict().unwrap(), b"Annots");
    assert_eq!(annots.as_array().unwrap().len(), 3);
}

/// A document edited with no journal (`edit::Builder`) follows the same
/// rule: past MuPDF's limit, the lowest number nothing holds or references.
#[test]
fn a_builder_past_the_object_number_limit_takes_free_low_numbers() {
    use tandem_pdf_engine::app::edit::{Builder, Edit};
    let mut pdf = Pdf::new();
    pdf.object(1, b"<</Type/Catalog/Pages 2 0 R/Lost 3 0 R>>")
        .object(2, b"<</Type/Pages/Kids[]/Count 0>>")
        .object(2_147_483_647, b"<</Root 1 0 R>>");
    let doc = Document::open_owned(pdf.classic("/Root 1 0 R"), DocumentLimits::default()).unwrap();
    let mut builder = Builder::from_document(doc);
    let first = builder.allocate(Object::Integer(1)).unwrap();
    let second = builder.allocate(Object::Integer(2)).unwrap();
    assert_eq!((first.number(), second.number()), (4, 5));
}

// ---------------------------------------------- encrypted object streams

/// Catalog, page tree and Info in an encrypted object stream, the page and
/// its encrypted contents as top-level objects; `junk` between two objects
/// makes the strict reader refuse a file whose xref is intact, and
/// `startxref` past the xref makes the repair scan instead.
fn encrypted_object_streams(handler: &Handler, junk: bool, bad_startxref: bool) -> Vec<u8> {
    let (data, first) = object_stream(&[
        (1, b"<</Type/Catalog/Pages 2 0 R>>"),
        (2, b"<</Type/Pages/Kids[3 0 R]/Count 1>>"),
        (6, b"<</Title(Plain \\(title\\))>>"),
    ]);
    let mut pdf = Pdf::new();
    pdf.object(
        3,
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>",
    )
    .stream(4, "<<", &handler.encrypt(4, &page_text("Secret words")))
    .object(5, FONT);
    if junk {
        pdf.raw(b"junk\n");
    }
    pdf.stream(
        7,
        &format!("<</Type/ObjStm/N 3/First {first}"),
        &handler.encrypt(7, &data),
    )
    .object(8, &handler.dictionary("/P -4"));
    let trailer = format!("/Root 1 0 R/Info 6 0 R/Encrypt 8 0 R{}", id_entry());
    let bad = bad_startxref.then_some(5);
    pdf.xref_stream(9, &[(1, 7, 0), (2, 7, 1), (6, 7, 2)], &trailer, bad)
}

fn info_title(doc: &Document<'_>) -> Vec<u8> {
    let info = doc
        .resolve(doc.trailer_entry(b"Info").unwrap())
        .as_dict()
        .unwrap();
    doc.dict_get(info, b"Title").as_string().unwrap().to_vec()
}

#[test]
fn repair_reads_object_streams_of_an_encrypted_file_and_decrypts_once() {
    let handler = Handler::r3(b"", b"owner", -4);
    for (junk, bad_startxref) in [(true, false), (false, true)] {
        let bytes = encrypted_object_streams(&handler, junk, bad_startxref);
        assert!(Document::open(&bytes, DocumentLimits::default()).is_err());
        let doc = AppDocument::load(Arc::new(bytes.clone()), b"").unwrap();
        assert!(!doc.is_locked());
        assert_eq!(doc.page_count(), 1);
        // Member strings were in the decrypted container; they read once.
        assert_eq!(info_title(doc.document()), b"Plain (title)");
        let (mut engine, mut session, reply) = open(bytes);
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["info"]["encrypted"], true);
        assert_eq!(text_all(&mut engine, &mut session), "Secret words");
        // The save keeps the encryption; the members stay readable.
        let saved = save(&mut engine, &mut session);
        let reopened = Document::open(&saved, DocumentLimits::default()).unwrap();
        assert!(reopened.security().is_some());
        assert_eq!(info_title(&reopened), b"Plain (title)");
    }
}

#[test]
fn an_encrypted_file_whose_object_streams_need_a_password_opens_locked_then_unlocks() {
    let handler = Handler::r3(b"user", b"owner", -4);
    let bytes = encrypted_object_streams(&handler, true, false);
    let limits = RepairLimits::for_document(limits());
    let repaired = rebuild_with_passwords(&bytes, limits, &[b"wrong"]).unwrap();
    assert!(repaired.report.locked);
    assert!(
        !rebuild_with_passwords(&bytes, limits, &[b"user"])
            .unwrap()
            .report
            .locked
    );
    let (mut engine, mut session, reply) = open(bytes);
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["info"]["needsPassword"], true);
    let wrong = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "x"}),
    );
    assert_eq!(wrong["authenticated"], false, "{wrong}");
    let unlocked = dispatch(
        &mut engine,
        &mut session,
        json!({"op": "authenticate", "password": "user"}),
    );
    assert_eq!(unlocked["authenticated"], true, "{unlocked}");
    assert_eq!(unlocked["pageCount"], 1);
    assert_eq!(text_all(&mut engine, &mut session), "Secret words");
    assert_eq!(
        info_title(session.doc.as_ref().unwrap().document()),
        b"Plain (title)"
    );
}

// ---------------------------------------------------- external streams

#[test]
fn a_stream_naming_an_external_file_is_read_from_its_own_bytes() {
    let mut pdf = Pdf::new();
    pdf.object(1, b"<</Type/Catalog/Pages 2 0 R>>")
        .object(2, b"<</Type/Pages/Kids[3 0 R]/Count 1>>")
        .object(
            3,
            b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>",
        )
        .stream(4, "<</F(missing.bin)/FFilter/FlateDecode", &page_text("Inside"))
        .object(5, FONT);
    let bytes = pdf.classic("/Root 1 0 R");
    // The strict reader still refuses it; the app reads the embedded bytes.
    assert!(matches!(
        Document::open(&bytes, DocumentLimits::default())
            .unwrap_err()
            .kind,
        DocumentErrorKind::Unsupported("external-file streams")
    ));
    assert!(Document::open_with(&bytes, DocumentLimits::default(), b"", open_options()).is_ok());
    let (mut engine, mut session, reply) = open(bytes);
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(!reply["info"]["repaired"].as_bool().unwrap());
    assert_eq!(text_all(&mut engine, &mut session), "Inside");
    let saved = save(&mut engine, &mut session);
    let reopened =
        Document::open_with(&saved, DocumentLimits::default(), b"", open_options()).unwrap();
    let dictionary = reopened.stream_dict(id(4)).unwrap();
    assert!(
        dictionary.contains_key(b"F".as_slice()) && dictionary.contains_key(b"FFilter".as_slice())
    );
}

// ------------------------------------------------ encryption dictionaries

/// One page whose contents are encrypted with `handler`, its /Encrypt
/// written as `encrypt` (object 6), `extra` objects added.
fn encrypted_page(handler: &Handler, encrypt: &[u8], extra: &[(u32, &[u8])]) -> Vec<u8> {
    let mut pdf = Pdf::new();
    pdf.object(1, b"<</Type/Catalog/Pages 2 0 R>>")
        .object(2, b"<</Type/Pages/Kids[3 0 R]/Count 1>>")
        .object(
            3,
            b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>",
        )
        .stream(4, "<<", &handler.encrypt(4, &page_text("Hidden")))
        .object(5, FONT)
        .object(6, encrypt);
    for (number, body) in extra {
        pdf.object(*number, body);
    }
    pdf.classic(&format!("/Root 1 0 R/Encrypt 6 0 R{}", id_entry()))
}

fn opens_unlocked(bytes: Vec<u8>) -> (AppEngine, AppSession, Value) {
    let (mut engine, mut session, reply) = open(bytes);
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["info"]["needsPassword"], false, "{reply}");
    assert_eq!(text_all(&mut engine, &mut session), "Hidden");
    (engine, session, reply)
}

#[test]
fn a_short_u_entry_is_zero_padded_as_mupdf_reads_it() {
    let handler = Handler::r3(b"", b"owner", -4);
    let short = Handler {
        u: handler.u[..16].to_vec(),
        ..Handler::r3(b"", b"owner", -4)
    };
    opens_unlocked(encrypted_page(&short, &short.dictionary("/P -4"), &[]));
    // Eight bytes cannot hold the 16 that revision 3 compares.
    let shorter = Handler {
        u: handler.u[..8].to_vec(),
        ..Handler::r3(b"", b"owner", -4)
    };
    let (_, _, reply) = open(encrypted_page(&shorter, &shorter.dictionary("/P -4"), &[]));
    assert_eq!(reply["info"]["needsPassword"], true, "{reply}");
}

#[test]
fn a_missing_p_entry_reads_as_every_permission() {
    let handler = Handler::r3(b"", b"owner", -4);
    let (_, _, reply) = opens_unlocked(encrypted_page(&handler, &handler.dictionary(""), &[]));
    assert_eq!(
        reply["info"]["permissions"],
        json!({"print": true, "copy": true, "edit": true, "annotate": true, "form": true})
    );
    // A real /P is not an integer: it reads as missing, as in MuPDF (a key
    // hashed with -4 opens, one hashed with the real's value does not).
    opens_unlocked(encrypted_page(
        &handler,
        &handler.dictionary("/P 100.0"),
        &[],
    ));
    let hashed = Handler::r3(b"", b"owner", 100);
    let (_, _, reply) = open(encrypted_page(&hashed, &hashed.dictionary("/P 100.0"), &[]));
    assert_eq!(reply["info"]["needsPassword"], true, "{reply}");
}

#[test]
fn indirect_crypt_filters_resolve() {
    let handler = Handler::r3(b"", b"owner", -4);
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02X}")).collect::<String>();
    let encrypt = format!(
        "<</Filter/Standard/V 4/R 4/Length 128/CF 7 0 R/StmF/StdCF/StrF/StdCF/O<{}>/U<{}>/P -4>>",
        hex(&handler.o),
        hex(&handler.u)
    );
    // Revision 4 hashes as 3 does when metadata is encrypted.
    let (_, _, reply) = opens_unlocked(encrypted_page(
        &handler,
        encrypt.as_bytes(),
        &[
            (7, b"<</StdCF 8 0 R>>"),
            (8, b"<</CFM/V2/AuthEvent/DocOpen/Length 16>>"),
        ],
    ));
    assert_eq!(reply["info"]["algorithm"], "rc4-128");
}

#[test]
fn an_encrypt_entry_that_is_not_a_dictionary_names_no_handler() {
    let mut pdf = Pdf::new();
    pdf.object(1, b"<</Type/Catalog/Pages 2 0 R>>")
        .object(2, b"<</Type/Pages/Kids[3 0 R]/Count 1>>")
        .object(
            3,
            b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>",
        )
        .stream(4, "<<", &page_text("Plain"))
        .object(5, FONT);
    for encrypt in ["[]", "7"] {
        let bytes = pdf.classic(&format!("/Root 1 0 R/Encrypt {encrypt}{}", id_entry()));
        let (mut engine, mut session, reply) = open(bytes);
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["info"]["needsPassword"], false);
        assert_eq!(reply["info"]["encrypted"], true);
        assert_eq!(reply["info"]["algorithm"], "unknown");
        assert_eq!(text_all(&mut engine, &mut session), "Plain");
        let saved = save(&mut engine, &mut session);
        let reopened = Document::open(&saved, DocumentLimits::default()).unwrap();
        assert!(reopened.trailer_entry(b"Encrypt").is_none());
    }
}

// ------------------------------------------------- lexical damage in repair

fn page_tree_pdf(page: &[u8]) -> Pdf {
    let mut pdf = Pdf::new();
    pdf.object(1, b"<</Type/Catalog/Pages 2 0 R>>")
        .object(2, b"<</Type/Pages/Kids[3 0 R]/Count 1>>")
        .object(3, page)
        .stream(4, "<<", &page_text("Body"))
        .object(5, FONT);
    pdf
}

const PAGE: &[u8] =
    b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>";

#[test]
fn a_name_escape_without_two_hex_digits_stands_for_itself_in_repair() {
    let page = b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>/PieceInfo<</A#G1 1/B#4 2>>>>";
    let bytes = page_tree_pdf(page).classic("/Root 1 0 R");
    assert!(Document::open(&bytes, DocumentLimits::default()).is_err());
    let doc = AppDocument::load(Arc::new(bytes.clone()), b"").unwrap();
    let page = doc.document().resolve_id(id(3)).as_dict().unwrap();
    let piece = doc
        .document()
        .dict_get(page, b"PieceInfo")
        .as_dict()
        .unwrap();
    assert!(piece.contains_key(b"A#G1".as_slice()) && piece.contains_key(b"B#4".as_slice()));
    let (mut engine, mut session, reply) = open(bytes);
    assert_eq!(reply["pageCount"], 1, "{reply}");
    assert_eq!(text_all(&mut engine, &mut session), "Body");
    let saved = save(&mut engine, &mut session);
    assert!(Document::open(&saved, DocumentLimits::default()).is_ok());
}

/// The catalog and page tree inside a container the xref stream names,
/// built by `container` from the members' data and /First.
fn tree_in_container(container: impl Fn(&[u8], usize) -> (String, Vec<u8>)) -> Vec<u8> {
    let (data, first) = object_stream(&[
        (1, b"<</Type/Catalog/Pages 2 0 R>>"),
        (2, b"<</Type/Pages/Kids[3 0 R]/Count 1>>"),
    ]);
    let (dictionary, data) = container(&data, first);
    let mut pdf = Pdf::new();
    pdf.object(3, PAGE)
        .stream(4, "<<", &page_text("Body"))
        .object(5, FONT)
        .stream(6, &dictionary, &data);
    pdf.xref_stream(7, &[(1, 6, 0), (2, 6, 1)], "/Root 1 0 R", None)
}

#[test]
fn containers_mupdf_reads_are_read_in_repair() {
    let cases: [(&str, Vec<u8>); 3] = [
        (
            "any /Type",
            tree_in_container(|data, first| {
                (format!("<</Type/Other/N 2/First {first}"), data.to_vec())
            }),
        ),
        (
            "/Fl",
            tree_in_container(|data, first| {
                (
                    format!("<</Type/ObjStm/N 2/First {first}/Filter/Fl"),
                    zlib(data),
                )
            }),
        ),
        (
            "data after the zlib stream",
            tree_in_container(|data, first| {
                let mut data = zlib(data);
                data.extend_from_slice(b"\r\ntrailing junk");
                (
                    format!("<</Type/ObjStm/N 2/First {first}/Filter/FlateDecode"),
                    data,
                )
            }),
        ),
    ];
    for (label, bytes) in cases {
        assert!(
            Document::open(&bytes, DocumentLimits::default()).is_err(),
            "{label}"
        );
        let (mut engine, mut session, reply) = open(bytes);
        assert_eq!(reply["pageCount"], 1, "{label}: {reply}");
        assert_eq!(text_all(&mut engine, &mut session), "Body", "{label}");
        save(&mut engine, &mut session);
    }
}

#[test]
fn a_listed_page_whose_object_does_not_parse_is_a_blank_page() {
    let mut pdf = Pdf::new();
    pdf.object(1, b"<</Type/Catalog/Pages 2 0 R>>")
        .object(
            2,
            b"<</Type/Pages/Kids[3 0 R 6 0 R]/Count 2/MediaBox[0 0 300 300]>>",
        )
        .object(3, PAGE)
        .stream(4, "<<", &page_text("Body"))
        .object(5, FONT)
        // A key without a value: no reader parses it; MuPDF shows a blank page.
        .object(6, b"<</Type/Page/Parent 2 0 R/Odd>>");
    let (mut engine, mut session, reply) = open(pdf.classic("/Root 1 0 R"));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["pageCount"], 2, "{reply}");
    assert_eq!(reply["info"]["repaired"], true);
    // The blank page inherits from its node like any other.
    assert_eq!(reply["pages"][1], json!({"width": 300.0, "height": 300.0}));
    let text = text_all(&mut engine, &mut session);
    assert_eq!(text.trim_end_matches('\u{c}'), "Body");
    let saved = save(&mut engine, &mut session);
    let reopened = Document::open(&saved, DocumentLimits::default()).unwrap();
    assert_eq!(tandem_pdf_engine::app::info::pages(&reopened).len(), 2);
}

/// A stream whose `endstream` keyword is damaged ends at its /Length when
/// `endobj` follows closely; searching for the next `endstream` would
/// swallow the page after it.
#[test]
fn a_damaged_endstream_keyword_keeps_the_declared_length() {
    let mut pdf = Pdf::new();
    pdf.object(1, b"<</Type/Catalog/Pages 2 0 R>>")
        .object(2, b"<</Type/Pages/Kids[3 0 R 6 0 R]/Count 2>>")
        .object(3, PAGE);
    let body = page_text("Body");
    let mut damaged = format!("<</Length {}>>\nstream\n", body.len()).into_bytes();
    damaged.extend_from_slice(&body);
    damaged.extend_from_slice(b"\nendstre#m");
    pdf.object(4, &damaged)
        .object(
            6,
            b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents 7 0 R/Resources<</Font<</F1 5 0 R>>>>>>",
        )
        .stream(7, "<<", &page_text("Second"))
        .object(5, FONT);
    let bytes = pdf.classic("/Root 1 0 R");
    assert!(Document::open(&bytes, DocumentLimits::default()).is_err());
    let (mut engine, mut session, reply) = open(bytes);
    assert_eq!(reply["pageCount"], 2, "{reply}");
    assert_eq!(text_all(&mut engine, &mut session), "Body\u{c}Second");
}

// ----------------------------------------------------- freed by an update

#[test]
fn an_object_an_update_frees_stays_gone_in_the_view() {
    let base = page_tree_pdf(PAGE).classic("/Root 1 0 R");
    let prev = String::from_utf8_lossy(&base)
        .rsplit("startxref\n")
        .next()
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    let mut bytes = base.clone();
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!(
            "xref\n0 1\n0000000004 65535 f \n4 1\n0000000000 00001 f \ntrailer\n<</Size 6/Root 1 0 R/Prev {prev}>>\nstartxref\n{xref}\n%%EOF\n"
        )
        .as_bytes(),
    );
    let (mut engine, mut session, reply) = open(bytes);
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(!numbers(&session).contains(&4));
    assert_eq!(text_all(&mut engine, &mut session), "");
    let (mut engine, mut session, _) = open(base);
    assert_eq!(text_all(&mut engine, &mut session), "Body");
}

// ------------------------------------------------------- saved page trees

/// The page list `Document::open` gives `bytes`, as object numbers, with
/// the checks MuPDF's reading relies on: one indirect /Pages root with the
/// right /Count, every page's /Parent naming it.
fn saved_tree(bytes: &[u8]) -> Vec<u32> {
    let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
    let catalog = doc.resolve_id(doc.root()).as_dict().unwrap();
    let Some(Object::Reference(root)) = catalog.get(b"Pages".as_slice()) else {
        panic!("indirect /Pages");
    };
    let node = doc.resolve_id(*root).as_dict().unwrap();
    assert_eq!(
        doc.dict_get(node, b"Type").as_name(),
        Some(b"Pages".as_slice())
    );
    let kids = doc.dict_get(node, b"Kids").as_array().unwrap();
    assert_eq!(
        doc.dict_get(node, b"Count"),
        &Object::Integer(kids.len() as i64)
    );
    kids.iter()
        .map(|kid| {
            let Object::Reference(page) = kid else {
                panic!("indirect page");
            };
            let dict = doc.resolve_id(*page).as_dict().unwrap();
            assert_eq!(
                dict.get(b"Parent".as_slice()),
                Some(&Object::Reference(*root))
            );
            assert_eq!(
                doc.dict_get(dict, b"Type").as_name(),
                Some(b"Page".as_slice())
            );
            page.number()
        })
        .collect()
}

#[test]
fn a_page_list_mupdf_would_read_differently_is_saved_as_one_node() {
    // The catalog names the page itself; no tree.
    let mut pages_is_page = Pdf::new();
    pages_is_page
        .object(1, b"<</Type/Catalog/Pages 3 0 R>>")
        .object(3, PAGE)
        .stream(4, "<<", &page_text("Body"))
        .object(5, FONT);
    // A cycle back to its own node, a page listed twice, a wrong /Count.
    let mut cycle = Pdf::new();
    cycle
        .object(1, b"<</Type/Catalog/Pages 2 0 R>>")
        .object(2, b"<</Type/Pages/Kids[3 0 R 6 0 R 3 0 R]/Count 9>>")
        .object(3, PAGE)
        .stream(4, "<<", &page_text("Body"))
        .object(5, FONT)
        .object(6, b"<</Type/Pages/Parent 2 0 R/Kids[6 0 R]/Count 1>>");
    for (label, bytes, expected) in [
        (
            "pages is a page",
            pages_is_page.classic("/Root 1 0 R"),
            vec![3],
        ),
        ("cycle", cycle.classic("/Root 1 0 R"), vec![3]),
    ] {
        let (mut engine, mut session, reply) = open(bytes);
        assert_eq!(reply["pageCount"], 1, "{label}: {reply}");
        let before = session.doc.as_ref().unwrap().document().clone();
        let saved = save(&mut engine, &mut session);
        assert_eq!(saved_tree(&saved), expected, "{label}");
        // The session's document is not touched: nothing to undo, same tree.
        let after = session.doc.as_ref().unwrap();
        assert!(!after.can_undo(), "{label}");
        assert_eq!(
            after.document().resolve_id(id(1)),
            before.resolve_id(id(1)),
            "{label}"
        );
        let (mut engine, mut session, reply) = open(saved);
        assert_eq!(reply["pageCount"], 1, "{label}");
        assert_eq!(text_all(&mut engine, &mut session), "Body", "{label}");
    }
}

#[test]
fn a_rebuilt_tree_writes_what_each_page_inherited() {
    // Pages inherit a direct /Resources, a /MediaBox and a /Rotate from a
    // node whose /Count is wrong.
    let mut pdf = Pdf::new();
    pdf.object(1, b"<</Type/Catalog/Pages 2 0 R>>")
        .object(
            2,
            b"<</Type/Pages/Kids[3 0 R 6 0 R]/Count 5/MediaBox[0 0 300 400]/Rotate 90/Resources<</Font<</F1 5 0 R>>>>>>",
        )
        .object(3, b"<</Type/Page/Parent 2 0 R/Contents 4 0 R>>")
        .stream(4, "<<", &page_text("Body"))
        .object(5, FONT)
        .object(6, b"<</Type/Page/Parent 2 0 R/Contents 4 0 R/MediaBox[0 0 100 100]>>");
    let (mut engine, mut session, reply) = open(pdf.classic("/Root 1 0 R"));
    let sizes = reply["pages"].clone();
    let text = text_all(&mut engine, &mut session);
    let saved = save(&mut engine, &mut session);
    assert_eq!(saved_tree(&saved), vec![3, 6]);
    let doc = Document::open(&saved, DocumentLimits::default()).unwrap();
    let first = doc.resolve_id(id(3)).as_dict().unwrap();
    assert_eq!(doc.dict_get(first, b"Rotate"), &Object::Integer(90));
    // One shared resources object for both pages.
    let resources = |page: u32| {
        doc.resolve_id(id(page))
            .as_dict()
            .unwrap()
            .get(b"Resources".as_slice())
            .cloned()
    };
    assert!(matches!(resources(3), Some(Object::Reference(_))));
    assert_eq!(resources(3), resources(6));
    let (mut engine, mut session, reply) = open(saved);
    assert_eq!(reply["pages"], sizes);
    assert_eq!(text_all(&mut engine, &mut session), text);
}

#[test]
fn an_intact_tree_is_saved_as_it_is() {
    let mut pdf = Pdf::new();
    pdf.object(1, b"<</Type/Catalog/Pages 2 0 R>>")
        .object(
            2,
            b"<</Type/Pages/Kids[6 0 R]/Count 2/MediaBox[0 0 200 200]>>",
        )
        .object(3, b"<</Type/Page/Parent 6 0 R/Contents 4 0 R>>")
        .stream(4, "<<", &page_text("Body"))
        .object(5, FONT)
        .object(6, b"<</Type/Pages/Parent 2 0 R/Kids[3 0 R 7 0 R]/Count 2>>")
        .object(7, b"<</Parent 6 0 R/Contents 4 0 R>>");
    let doc = Document::open_owned(pdf.classic("/Root 1 0 R"), DocumentLimits::default()).unwrap();
    assert!(matches!(
        save_tree::for_save(&doc).unwrap(),
        std::borrow::Cow::Borrowed(_)
    ));
    // The same tree with a wrong /Count is rebuilt.
    let mut wrong = Pdf::new();
    wrong
        .object(1, b"<</Type/Catalog/Pages 2 0 R>>")
        .object(2, b"<</Type/Pages/Kids[3 0 R]/Count 2>>")
        .object(3, b"<</Type/Page/Parent 2 0 R>>");
    let doc =
        Document::open_owned(wrong.classic("/Root 1 0 R"), DocumentLimits::default()).unwrap();
    assert!(matches!(
        save_tree::for_save(&doc).unwrap(),
        std::borrow::Cow::Owned(_)
    ));
}

#[test]
fn open_tolerant_rebuilds_encrypted_object_streams_with_its_password() {
    let handler = Handler::r3(b"user", b"owner", -4);
    let bytes = encrypted_object_streams(&handler, true, false);
    assert!(
        rebuild(&bytes, RepairLimits::default())
            .unwrap()
            .report
            .locked
    );
    let limits = DocumentLimits::default();
    let tolerant = tandem_pdf_engine::repair::open_tolerant(bytes, limits, b"user").unwrap();
    assert!(!tolerant.needs_password && tolerant.repair.is_some());
    let doc = Document::open_with_password(&tolerant.bytes, limits, b"user").unwrap();
    assert_eq!(info_title(&doc), b"Plain (title)");
}

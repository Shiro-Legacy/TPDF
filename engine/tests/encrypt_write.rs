// SPDX-License-Identifier: AGPL-3.0-or-later
//! Encrypted output. `Encryption::Keep` re-encrypts with the document's own
//! handler; `Encryption::Aes256` writes a new V5 R6 handler. Outputs are read
//! back by the strict Document and by hayro (an independent decryptor), and
//! compared with a plaintext write of the same document. A reference reader
//! can also check the files `oracle_cases` writes when
//! TPDF_ENCRYPT_ORACLE_DIR is set.
use aes::cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
use aes::{Aes128, Aes256};
use md5::{Digest, Md5};
use sha2::Sha256;

use tandem_pdf_engine::document::{Document, DocumentErrorKind, DocumentLimits};
use tandem_pdf_engine::object::{Dictionary, Object, ObjectId};
use tandem_pdf_engine::security::{Authentication, EncryptionInfo, Method, standard_permissions};
use tandem_pdf_engine::serialize::{SerializeError, WriteLimits};
use tandem_pdf_engine::view::ViewDocument;
use tandem_pdf_engine::writer::{
    DocumentWriteError, DocumentWriteLimits, Encryption, Entropy, WriteOptions, write_document,
};

const SEED: Entropy = Entropy([7; 32]);
const NEW_USER: &[u8] = b"new user \xC3\xA9";
const NEW_OWNER: &[u8] = b"new owner";
/// Non-ASCII passwords of the -pdfdoc cases (UTF-8, as the app sends them)
/// and their PDFDocEncoding, which R2-R4 hash; 中 has no code.
const PDFDOC_USER: &[u8] = "pässwort€中".as_bytes();
const PDFDOC_OWNER: &[u8] = "öwner•".as_bytes();
const PDFDOC_USER_ENCODED: &[u8] = b"p\xE4sswort\xA0";
const PDFDOC_OWNER_ENCODED: &[u8] = b"\xF6wner\x80";

struct Case {
    name: &'static str,
    bytes: Vec<u8>,
    user: &'static [u8],
    owner: &'static [u8],
    /// V, R, key bits, method.
    expect: (i64, i64, i64, Method),
}

fn case(
    name: &'static str,
    path: String,
    passwords: (&'static [u8], &'static [u8]),
    expect: (i64, i64, i64, Method),
) -> Case {
    Case {
        name,
        bytes: std::fs::read(path).unwrap(),
        user: passwords.0,
        owner: passwords.1,
        expect,
    }
}

/// Every encrypted fixture, the cipher families a reference writer wrote (one with
/// non-ASCII passwords), and documents built here: R4 AES-128 with
/// /EncryptMetadata false and an indirect /Encrypt, R5 AES-256 (deprecated,
/// but readable), R3 without /ID with PDFDocEncoded passwords, and V4 with
/// RC4 (/CFM /V2) crypt filters.
fn cases() -> Vec<Case> {
    const APP: (&[u8], &[u8]) = (b"pdf4all-user", b"pdf4all-owner");
    const GEN: (&[u8], &[u8]) = (b"u", b"o");
    let rc4_128 = (2, 3, 128, Method::Rc4);
    let fixture = |name: &str| format!("../fixtures/{name}");
    let data = |name: &str| format!("tests/data/{name}");
    let mut cases = vec![
        case("encrypted", fixture("encrypted.pdf"), APP, rc4_128),
        case("noedit", fixture("encrypted-noedit.pdf"), APP, rc4_128),
        case("noprint", fixture("encrypted-noprint.pdf"), APP, rc4_128),
        case(
            "aes256",
            fixture("encrypted-aes256.pdf"),
            APP,
            (5, 6, 256, Method::AesV3),
        ),
        case(
            "edit-noannot",
            fixture("p3/encrypted-edit-noannot.pdf"),
            APP,
            rc4_128,
        ),
        case(
            "rc4-40",
            data("encrypted-rc4-40.pdf"),
            GEN,
            (1, 2, 40, Method::Rc4),
        ),
        case(
            "aes128",
            data("encrypted-aes128.pdf"),
            GEN,
            (4, 4, 128, Method::AesV2),
        ),
        case(
            "aes128-objstm",
            data("encrypted-aes128-objstm.pdf"),
            GEN,
            (4, 4, 128, Method::AesV2),
        ),
        case(
            "aes256-objstm",
            data("encrypted-aes256-objstm.pdf"),
            GEN,
            (5, 6, 256, Method::AesV3),
        ),
        case(
            "rc4-128-pdfdoc",
            data("encrypted-rc4-128-pdfdoc.pdf"),
            (PDFDOC_USER, PDFDOC_OWNER),
            rc4_128,
        ),
    ];
    cases.push(Case {
        name: "r4-plain-metadata",
        bytes: r4_document(),
        user: GEN.0,
        owner: GEN.1,
        expect: (4, 4, 128, Method::AesV2),
    });
    cases.push(Case {
        name: "r5",
        bytes: r5_document(),
        user: GEN.0,
        owner: GEN.1,
        expect: (5, 5, 256, Method::AesV3),
    });
    cases.push(Case {
        name: "r3-pdfdoc-no-id",
        bytes: legacy_document(Legacy {
            id: false,
            ..Legacy::new(2, 3, None, PDFDOC_USER_ENCODED, PDFDOC_OWNER_ENCODED)
        }),
        user: PDFDOC_USER,
        owner: PDFDOC_OWNER,
        expect: rc4_128,
    });
    cases.push(Case {
        name: "v4-rc4",
        bytes: legacy_document(Legacy::new(4, 4, Some("V2"), GEN.0, GEN.1)),
        user: GEN.0,
        owner: GEN.1,
        expect: (4, 4, 128, Method::Rc4),
    });
    cases
}

fn limits() -> DocumentLimits {
    DocumentLimits::default()
}
fn options(encryption: Encryption<'_>) -> WriteOptions<'_> {
    WriteOptions {
        encryption,
        entropy: Some(SEED),
        ..WriteOptions::APP
    }
}
fn write(doc: &Document<'_>, options: WriteOptions<'_>) -> Vec<u8> {
    write_document(doc, DocumentWriteLimits::default(), options).unwrap()
}
fn open<'a>(bytes: &'a [u8], password: &[u8]) -> Document<'a> {
    Document::open_with_password(bytes, limits(), password).unwrap()
}
fn needs_password(bytes: &[u8], password: &[u8]) -> bool {
    matches!(
        Document::open_with_password(bytes, limits(), password).map_err(|e| e.kind),
        Err(DocumentErrorKind::PasswordRequired)
    )
}
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
const AES256: Encryption<'static> = Encryption::Aes256 {
    user_password: NEW_USER,
    owner_password: NEW_OWNER,
    permissions: 4 | 32,
};

/// The same objects, values (strings decrypted; stream /Length aside) and
/// still-filtered stream bytes in both documents.
fn assert_same_objects(a: &Document<'_>, b: &Document<'_>, name: &str) {
    let ids = |d: &Document<'_>| d.objects().map(|o| o.id).collect::<Vec<_>>();
    assert_eq!(ids(a), ids(b), "{name}");
    let value = |d: &Document<'_>, id| {
        let object = d.object(id).unwrap();
        let mut value = object.value.clone();
        if let (Some(_), Object::Dictionary(entries)) = (&object.stream, &mut value) {
            entries.remove(b"Length".as_slice());
        }
        value
    };
    for object in a.objects() {
        let id = object.id;
        assert_eq!(value(a, id), value(b, id), "{name} {id:?}");
        assert_eq!(
            a.stream_data(id).unwrap(),
            b.stream_data(id).unwrap(),
            "{name} {id:?}"
        );
    }
}

/// Page text as hayro reads it with `password`.
fn texts(bytes: &[u8], password: &[u8]) -> Vec<String> {
    let view = ViewDocument::open(bytes.to_vec(), std::str::from_utf8(password).unwrap())
        .unwrap_or_else(|e| panic!("hayro: {e}"));
    (0..view.page_count())
        .map(|page| view.structured_text(page).unwrap().as_text())
        .collect()
}

/// `texts`, except that hayro hashes R2-R4 passwords as UTF-8, so a document
/// with a non-ASCII one is decrypted by a plaintext write instead.
fn case_texts(case: &Case, bytes: &[u8], password: &[u8]) -> Vec<String> {
    if password.is_ascii() || case.expect.1 >= 5 {
        return texts(bytes, password);
    }
    let plain = write(&open(bytes, password), options(Encryption::Plaintext));
    texts(&plain, b"")
}

fn file_id(doc: &Document<'_>) -> Option<Vec<Vec<u8>>> {
    let Some(Object::Array(items)) = doc.trailer_entry(b"ID") else {
        return None;
    };
    Some(
        items
            .iter()
            .map(|item| item.as_string().unwrap().to_vec())
            .collect(),
    )
}

#[test]
fn keep_reencrypts_with_the_documents_own_handler() {
    for case in cases() {
        let name = case.name;
        let info = Document::read_encryption(&case.bytes, limits())
            .unwrap()
            .unwrap();
        let (v, r, length, method) = case.expect;
        assert_eq!(
            (info.v, info.r, info.length, info.method),
            (v, r, length, Some(method)),
            "{name}"
        );
        assert!(needs_password(&case.bytes, b""), "{name}");
        for password in [case.user, case.owner] {
            let source = open(&case.bytes, password);
            assert_eq!(source.encryption().as_ref(), Some(&info), "{name}");
            let security = source.security().unwrap();
            let plain = write(&source, options(Encryption::Plaintext));
            let kept = write(&source, options(Encryption::Keep));
            assert_eq!(write(&source, options(Encryption::Keep)), kept, "{name}");
            let other_seed = WriteOptions {
                entropy: Some(Entropy([8; 32])),
                ..options(Encryption::Keep)
            };
            assert_ne!(write(&source, other_seed), kept, "{name}");
            for (password, auth) in [
                (case.user, Authentication::User),
                (case.owner, Authentication::Owner),
            ] {
                let reopened = open(&kept, password);
                assert_eq!(reopened.authentication(), Some(auth), "{name}");
                let again = reopened.security().unwrap();
                assert_eq!(again.dictionary(), security.dictionary(), "{name}");
                assert_eq!(again.permissions, security.permissions, "{name}");
                assert_same_objects(&open(&plain, b""), &reopened, name);
            }
            for wrong in [b"".as_slice(), b"wrong", NEW_USER] {
                assert!(needs_password(&kept, wrong), "{name}");
            }
            // /ID[0] is the one the key was derived with; /ID[1] is new.
            let ids = file_id(&open(&kept, case.user)).unwrap();
            assert_eq!(ids[0], security.first_id(), "{name}");
            assert_eq!(ids[1].len(), 16, "{name}");
            assert_ne!(
                file_id(&source).map(|ids| ids[1].clone()),
                Some(ids[1].clone())
            );
            assert_eq!(
                case_texts(&case, &kept, case.user),
                case_texts(&case, &case.bytes, case.user),
                "{name}"
            );
            for secret in [b"PDF4All".as_slice(), b"secret note", b"Hidden page"] {
                if contains(&plain, secret) {
                    assert!(!contains(&kept, secret), "{name}");
                }
            }
        }
    }
}

#[test]
fn aes256_writes_a_v5_r6_handler() {
    let hello = Case {
        name: "hello",
        bytes: std::fs::read("../fixtures/hello.pdf").unwrap(),
        user: b"",
        owner: b"",
        expect: (0, 0, 0, Method::Identity),
    };
    let p = standard_permissions(4 | 32);
    let expected = EncryptionInfo {
        filter: b"Standard".to_vec(),
        v: 5,
        r: 6,
        length: 256,
        method: Some(Method::AesV3),
        p,
        encrypt_metadata: true,
    };
    for case in cases().into_iter().chain([hello]) {
        let name = case.name;
        let source = open(&case.bytes, case.owner);
        let plain = write(&source, options(Encryption::Plaintext));
        let out = write(&source, options(AES256));
        assert_eq!(write(&source, options(AES256)), out, "{name}");
        assert_eq!(
            Document::read_encryption(&out, limits()).unwrap(),
            Some(expected.clone()),
            "{name}"
        );
        for wrong in [b"".as_slice(), b"new user", case.user, case.owner] {
            assert!(needs_password(&out, wrong), "{name}");
        }
        for (password, auth) in [
            (NEW_USER, Authentication::User),
            (NEW_OWNER, Authentication::Owner),
        ] {
            let reopened = open(&out, password);
            assert_eq!(reopened.authentication(), Some(auth), "{name}");
            assert_eq!(reopened.encryption(), Some(expected.clone()), "{name}");
            assert_same_objects(&open(&plain, b""), &reopened, name);
            assert_eq!(
                texts(&out, password),
                case_texts(&case, &case.bytes, case.user),
                "{name}"
            );
        }
        let dictionary = open(&out, NEW_USER)
            .security()
            .unwrap()
            .dictionary()
            .clone();
        let Some(Object::Dictionary(filters)) = dictionary.get(b"CF".as_slice()) else {
            panic!("{name}: /CF");
        };
        let Some(Object::Dictionary(filter)) = filters.get(b"StdCF".as_slice()) else {
            panic!("{name}: /StdCF");
        };
        assert_eq!(
            filter.get(b"CFM".as_slice()),
            Some(&Object::Name(b"AESV3".to_vec()))
        );
        // /ID[0] kept when the source has one; otherwise two equal new ones.
        let ids = file_id(&open(&out, NEW_USER)).unwrap();
        match file_id(&source) {
            Some(source_ids) => {
                assert_eq!(ids[0], source_ids[0], "{name}");
                assert_ne!(ids[1], source_ids[1], "{name}");
            }
            None => assert_eq!(ids[0], ids[1], "{name}"),
        }
        assert_eq!(ids[1].len(), 16);
    }
}

#[test]
fn permissions_match_reference_writers_with_bit_10_set() {
    // /P that a reference writer writes for encrypt=aes-256,permissions=N
    // (measured): (N & 0xF3C) | 0xFFFFF0C0. ISO 32000-2 Table 22 also
    // requires writers to set bit 10 (0x200).
    for (granted, written) in [
        (0, -3904),
        (4, -3900),
        (36, -3868),
        (316, -3588),
        (u32::MAX, -4),
        (512, -3392),
        (3900, -4),
        (4095, -4),
        (3, -3904),
        (4096, -3904),
        (192, -3904),
        (0x7FFF_FFFF, -4),
    ] {
        assert_eq!(standard_permissions(granted), written | 0x200, "{granted}");
    }
}

/// Signature /Contents that stay in plaintext: /Type /Sig, /DocTimeStamp
/// and untyped dictionaries with /ByteRange (one reference writer exempts only the first).
const SIGNATURE_CONTENTS: [&[u8]; 3] = [
    b"(SIGNATURE-PLACEHOLDER-0123)",
    b"(TIMESTAMP-PLACEHOLDER-0123)",
    b"(UNTYPED-PLACEHOLDER-0123)",
];
/// Strings encrypted with them: other signature entries and the /Contents
/// of a /Type /Sig without /ByteRange.
const SIGNATURE_SECRETS: [&[u8]; 3] = [b"Signer", b"Untyped reason", b"Unsigned contents"];

#[test]
fn exempt_strings_and_streams_stay_in_plaintext() {
    // Plaintext source: Aes256 encrypts everything except the signature
    // /Contents and the cross-reference-typed stream.
    let source_bytes = exemption_document(None);
    let source = open(&source_bytes, b"");
    let plain = write(&source, options(Encryption::Plaintext));
    let out = write(&source, options(AES256));
    for contents in SIGNATURE_CONTENTS {
        assert!(contains(&out, contents));
    }
    assert!(contains(&out, b"xref-stream-bytes"));
    for secret in [
        b"secret note".as_slice(),
        b"Exemption title",
        b"Hidden page",
        b"metadata-in-plain",
    ]
    .into_iter()
    .chain(SIGNATURE_SECRETS)
    {
        assert!(contains(&plain, secret));
        assert!(
            !contains(&out, secret),
            "{}",
            String::from_utf8_lossy(secret)
        );
    }
    assert_same_objects(&source, &open(&out, NEW_USER), "exemptions");
    // Keep with /EncryptMetadata false leaves the Metadata stream readable.
    let r4 = r4_document();
    let kept = write(&open(&r4, b"u"), options(Encryption::Keep));
    assert!(contains(&kept, b"metadata-in-plain"));
    assert!(contains(&kept, b"/EncryptMetadata false"));
    for contents in SIGNATURE_CONTENTS {
        assert!(contains(&kept, contents));
    }
    for secret in [b"Hidden page".as_slice()]
        .into_iter()
        .chain(SIGNATURE_SECRETS)
    {
        assert!(!contains(&kept, secret));
    }
    let plain = write(&open(&r4, b"u"), options(Encryption::Plaintext));
    assert_same_objects(&open(&plain, b""), &open(&kept, b"u"), "r4");
}

#[test]
fn a_source_encryption_dictionary_object_is_encrypted_like_any_other() {
    // The output's /Encrypt is direct, so readers decrypt every indirect
    // object. Written without garbage collection, the source's /Encrypt
    // object (14 in r4_document) must be encrypted to read back unchanged.
    let r4 = r4_document();
    let source = open(&r4, b"u");
    let encrypt = ObjectId::new(14, 0).unwrap();
    assert_eq!(
        source.trailer_entry(b"Encrypt"),
        Some(&Object::Reference(encrypt))
    );
    let all = |encryption| WriteOptions {
        garbage: false,
        ..options(encryption)
    };
    let plain = write(&source, all(Encryption::Plaintext));
    let plain = open(&plain, b"");
    for (encryption, password) in [(Encryption::Keep, b"u".as_slice()), (AES256, NEW_USER)] {
        let out = write(&source, all(encryption));
        let reopened = open(&out, password);
        assert_eq!(
            reopened.object(encrypt).unwrap().value,
            source.object(encrypt).unwrap().value
        );
        assert_same_objects(&plain, &reopened, "garbage=false");
    }
    // An object that takes over the number of the source's indirect
    // /Encrypt is encrypted too.
    let encrypted = std::fs::read("../fixtures/encrypted.pdf").unwrap();
    let mut doc = open(&encrypted, b"pdf4all-user");
    let Some(&Object::Reference(id)) = doc.trailer_entry(b"Encrypt") else {
        panic!("indirect /Encrypt");
    };
    doc.remove_object(id).unwrap();
    let mut info = Dictionary::new();
    info.insert(
        b"Title".to_vec(),
        Object::String(b"TOP-SECRET-TITLE".to_vec()),
    );
    assert_eq!(doc.allocate(Object::Dictionary(info), None).unwrap(), id);
    doc.set_trailer_entry(b"Info", Some(Object::Reference(id)))
        .unwrap();
    for (encryption, password) in [
        (Encryption::Keep, b"pdf4all-user".as_slice()),
        (AES256, NEW_USER),
    ] {
        let out = write(&doc, options(encryption));
        assert!(!contains(&out, b"TOP-SECRET-TITLE"));
        let reopened = open(&out, password);
        let title = reopened.object(id).unwrap().value.as_dict().unwrap();
        assert_eq!(
            title.get(b"Title".as_slice()),
            Some(&Object::String(b"TOP-SECRET-TITLE".to_vec()))
        );
    }
}

#[test]
fn r2_r4_passwords_are_pdfdoc_encoded() {
    // A reference writer wrote these with user "pässwort€中" and owner "öwner•":
    // PDFDocEncoding for R2-R4 (中 has no code and is dropped), UTF-8 for R6.
    // Reference readers authenticate the UTF-8 strings as levels 2 and 4.
    for (name, r) in [("rc4-40", 2), ("rc4-128", 3), ("aes128", 4), ("aes256", 6)] {
        let bytes = std::fs::read(format!("tests/data/encrypted-{name}-pdfdoc.pdf")).unwrap();
        let auth = |password: &[u8]| {
            Document::open_with_password(&bytes, limits(), password)
                .ok()
                .and_then(|doc| doc.authentication())
        };
        assert_eq!(auth(PDFDOC_USER), Some(Authentication::User), "{name}");
        assert_eq!(auth(PDFDOC_OWNER), Some(Authentication::Owner), "{name}");
        let legacy = (r <= 4).then_some(Authentication::User);
        assert_eq!(auth("pässwort€".as_bytes()), legacy, "{name}");
        assert_eq!(auth(PDFDOC_USER_ENCODED), legacy, "{name}");
        let legacy = (r <= 4).then_some(Authentication::Owner);
        assert_eq!(auth(PDFDOC_OWNER_ENCODED), legacy, "{name}");
        for wrong in [b"".as_slice(), b"wrong", "中".as_bytes(), b"p\xC3\xA4"] {
            assert_eq!(auth(wrong), None, "{name}");
        }
    }
    // A writer that hashed the UTF-8 bytes themselves: the bytes as given
    // are tried after their PDFDocEncoding.
    let utf8 = legacy_document(Legacy::new(2, 3, None, PDFDOC_USER, PDFDOC_OWNER));
    for (password, auth) in [
        (PDFDOC_USER, Authentication::User),
        (PDFDOC_OWNER, Authentication::Owner),
    ] {
        assert_eq!(open(&utf8, password).authentication(), Some(auth));
    }
    assert!(needs_password(&utf8, PDFDOC_USER_ENCODED));
}

#[test]
fn encrypted_output_needs_entropy_and_respects_limits() {
    let encrypted = std::fs::read("../fixtures/encrypted.pdf").unwrap();
    let hello = std::fs::read("../fixtures/hello.pdf").unwrap();
    let doc = open(&encrypted, b"pdf4all-user");
    let plain = open(&hello, b"");
    let no_entropy = |encryption| WriteOptions {
        encryption,
        ..WriteOptions::APP
    };
    let invalid = DocumentWriteError::Invalid("encrypted output needs entropy").to_string();
    for (doc, encryption) in [(&doc, Encryption::Keep), (&plain, AES256), (&doc, AES256)] {
        let error = write_document(doc, DocumentWriteLimits::default(), no_entropy(encryption));
        assert_eq!(error.unwrap_err().to_string(), invalid);
    }
    // Keep on a plaintext document is a plaintext write; without /ID none is
    // created, and without entropy /ID is copied.
    let app = write(&plain, WriteOptions::APP);
    assert_eq!(write(&plain, no_entropy(Encryption::Keep)), app);
    assert_eq!(write(&plain, options(Encryption::Keep)), app);
    let with_id = exemption_document(None);
    let with_id = open(&with_id, b"");
    let copied = write(&with_id, no_entropy(Encryption::Keep));
    assert_eq!(file_id(&open(&copied, b"")), file_id(&with_id));
    let fresh = file_id(&open(&write(&with_id, options(Encryption::Keep)), b"")).unwrap();
    assert_eq!(fresh[0], file_id(&with_id).unwrap()[0]);
    assert_eq!(fresh[1].len(), 16);
    // Output limits hold for the grown, encrypted output.
    let encrypted_length = write(&plain, options(AES256)).len();
    for max_output_bytes in [64, app.len(), encrypted_length - 1] {
        let limits = DocumentWriteLimits {
            syntax: WriteLimits {
                max_output_bytes,
                ..DocumentWriteLimits::default().syntax
            },
            ..DocumentWriteLimits::default()
        };
        assert!(matches!(
            write_document(&plain, limits, options(AES256)),
            Err(DocumentWriteError::Serialize(SerializeError::OutputLimit))
        ));
    }
    // A strict write refuses an encrypted source whatever the encryption.
    let strict = WriteOptions {
        strict: true,
        ..options(Encryption::Keep)
    };
    assert!(write_document(&doc, DocumentWriteLimits::default(), strict).is_err());
    // Passwords and entropy never reach Debug output.
    let debug = format!("{:?}", options(AES256));
    assert!(
        !debug.contains("new user") && !debug.contains("[7, 7"),
        "{debug}"
    );
}

#[test]
fn read_encryption_reports_locked_documents() {
    for case in cases() {
        let info = Document::read_encryption(&case.bytes, limits()).unwrap();
        assert_eq!(
            info,
            open(&case.bytes, case.user).encryption(),
            "{}",
            case.name
        );
    }
    let encrypted = std::fs::read("../fixtures/encrypted-noprint.pdf").unwrap();
    let info = Document::read_encryption(&encrypted, limits())
        .unwrap()
        .unwrap();
    assert_eq!(info.p & 4, 0, "print denied while locked");
    let hello = std::fs::read("../fixtures/hello.pdf").unwrap();
    assert_eq!(Document::read_encryption(&hello, limits()).unwrap(), None);
    assert!(Document::read_encryption(b"%PDF-1.7\nnot a pdf", limits()).is_err());
    assert_eq!(open(&hello, b"").authentication(), None);
}

/// Writes Keep and Aes256 outputs of every case, with a manifest, for a
/// reference reader to check.
#[test]
fn oracle_cases() {
    let Ok(dir) = std::env::var("TPDF_ENCRYPT_ORACLE_DIR") else {
        return;
    };
    let dir = std::path::Path::new(&dir);
    std::fs::create_dir_all(dir).unwrap();
    let text = |bytes: &[u8]| String::from_utf8(bytes.to_vec()).unwrap();
    let mut manifest = Vec::new();
    let mut save = |file: String, bytes: &[u8], entry: serde_json::Value| {
        std::fs::write(dir.join(&file), bytes).unwrap();
        let mut entry = entry;
        entry["file"] = file.into();
        manifest.push(entry);
    };
    for case in cases() {
        let source_file = format!("source-{}.pdf", case.name);
        std::fs::write(dir.join(&source_file), &case.bytes).unwrap();
        for (who, password) in [("user", case.user), ("owner", case.owner)] {
            let kept = write(&open(&case.bytes, password), options(Encryption::Keep));
            save(
                format!("keep-{}-{who}.pdf", case.name),
                &kept,
                serde_json::json!({"mode": "keep", "source": source_file,
                    "user": text(case.user), "owner": text(case.owner)}),
            );
        }
        let out = write(&open(&case.bytes, case.owner), options(AES256));
        // Reference readers decrypt the plaintext Metadata stream of an /EncryptMetadata
        // false source, so the AES-256 output is checked against the XML.
        let metadata = (case.name == "r4-plain-metadata").then_some(METADATA);
        save(
            format!("aes256-{}.pdf", case.name),
            &out,
            serde_json::json!({"mode": "aes-256", "source": source_file,
                "sourceUser": text(case.user), "user": text(NEW_USER),
                "owner": text(NEW_OWNER), "permissions": 36, "metadata": metadata}),
        );
    }
    let hello = std::fs::read("../fixtures/hello.pdf").unwrap();
    std::fs::write(dir.join("source-hello.pdf"), &hello).unwrap();
    let exemption = exemption_document(None);
    std::fs::write(dir.join("source-exemption.pdf"), &exemption).unwrap();
    for granted in [0u32, 4, 36, 316, 0xF3C, u32::MAX] {
        for (source, bytes) in [("hello", &hello), ("exemption", &exemption)] {
            let encryption = Encryption::Aes256 {
                user_password: b"u",
                owner_password: b"o",
                permissions: granted,
            };
            let out = write(&open(bytes, b""), options(encryption));
            save(
                format!("aes256-{source}-{granted}.pdf"),
                &out,
                serde_json::json!({"mode": "aes-256", "source": format!("source-{source}.pdf"),
                    "sourceUser": "", "user": "u", "owner": "o", "permissions": granted}),
            );
        }
    }
    let manifest = serde_json::to_string_pretty(&manifest).unwrap();
    std::fs::write(dir.join("manifest.json"), manifest).unwrap();
}

// Documents built here, encrypted with independent test code.

const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];
const ID0: &[u8] = b"0123456789abcdef";
const METADATA: &str = "<x:xmpmeta xmlns:x='adobe:ns:meta/'>metadata-in-plain</x:xmpmeta>";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

fn pdf(objects: &[Vec<u8>], trailer: &str) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    let count = objects.len() + 1;
    bytes.extend_from_slice(format!("xref\n0 {count}\n0000000000 65535 f \n").as_bytes());
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<</Size {count}/Root 1 0 R{trailer}>>\nstartxref\n{xref}\n%%EOF\n")
            .as_bytes(),
    );
    bytes
}

fn stream(dictionary: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<<{dictionary}/Length {}>>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// Object number and plaintext to ciphertext.
type Seal = Box<dyn Fn(u32, &[u8]) -> Vec<u8>>;

/// How a built document is encrypted: strings and streams, whether Metadata
/// is encrypted, the /Encrypt dictionary, and whether the trailer has /ID.
struct Crypt {
    encrypt: Seal,
    metadata: bool,
    dictionary: String,
    id: bool,
}

/// Catalog, page with text, Metadata stream, annotation and Info strings,
/// signature dictionaries (/Type /Sig, /DocTimeStamp and untyped, whose
/// /Contents are never encrypted, and a /Type /Sig without /ByteRange, whose
/// /Contents are), and a cross-reference-typed stream (never encrypted).
/// /ID unless `crypt` leaves it out; /Encrypt is object 14.
fn exemption_document(crypt: Option<Crypt>) -> Vec<u8> {
    let seal = |n: u32, data: &[u8]| match &crypt {
        Some(crypt) => (crypt.encrypt)(n, data),
        None => data.to_vec(),
    };
    let string = |n: u32, data: &[u8]| format!("<{}>", hex(&seal(n, data)));
    let xml = METADATA.as_bytes();
    let metadata = match &crypt {
        Some(crypt) if !crypt.metadata => xml.to_vec(),
        _ => seal(5, xml),
    };
    let mut objects = vec![
        b"<</Type/Catalog/Pages 2 0 R/Metadata 5 0 R/Sig 9 0 R/X 10 0 R\
          /TimeStamp 11 0 R/Untyped 12 0 R/Unsigned 13 0 R>>"
            .to_vec(),
        b"<</Type/Pages/Kids[3 0 R]/Count 1>>".to_vec(),
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents 4 0 R\
          /Resources<</Font<</F1 6 0 R>>>>/Annots[7 0 R]>>"
            .to_vec(),
        stream(
            "",
            &seal(4, b"BT /F1 12 Tf 20 100 Td (Hidden page text) Tj ET"),
        ),
        stream("/Type/Metadata/Subtype/XML", &metadata),
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".to_vec(),
        format!(
            "<</Type/Annot/Subtype/Text/Rect[10 10 30 30]/Contents {}/NM {}/M [{}]>>",
            string(7, b"secret note"),
            string(7, b""),
            string(7, b"0123456789abcdef0"),
        )
        .into_bytes(),
        format!("<</Title {}>>", string(8, b"Exemption title")).into_bytes(),
        format!(
            "<</Type/Sig/Filter/Adobe.PPKLite/ByteRange[0 0 0 0]\
             /Contents (SIGNATURE-PLACEHOLDER-0123)/Name {}>>",
            string(9, b"Signer")
        )
        .into_bytes(),
        stream("/Type/XRef/Size 1/W[1 1 1]", b"xref-stream-bytes"),
        b"<</Type/DocTimeStamp/Filter/Adobe.PPKLite/SubFilter/ETSI.RFC3161\
          /ByteRange[0 0 0 0]/Contents (TIMESTAMP-PLACEHOLDER-0123)>>"
            .to_vec(),
        format!(
            "<</Filter/Adobe.PPKLite/ByteRange[0 0 0 0]\
             /Contents (UNTYPED-PLACEHOLDER-0123)/Reason {}>>",
            string(12, b"Untyped reason")
        )
        .into_bytes(),
        format!(
            "<</Type/Sig/Filter/Adobe.PPKLite/Contents {}>>",
            string(13, b"Unsigned contents")
        )
        .into_bytes(),
    ];
    let mut trailer = "/Info 8 0 R".to_string();
    if crypt.as_ref().is_none_or(|crypt| crypt.id) {
        trailer.push_str(&format!("/ID[<{}><{}>]", hex(ID0), hex(b"second")));
    }
    if let Some(crypt) = &crypt {
        objects.push(crypt.dictionary.clone().into_bytes());
        trailer.push_str("/Encrypt 14 0 R");
    }
    pdf(&objects, &trailer)
}

fn padded(password: &[u8]) -> Vec<u8> {
    password.iter().chain(&PAD).take(32).copied().collect()
}

fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: Vec<u8> = (0..=255).collect();
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

/// RC4 with the key, then with the key XORed with 1..=19 (Algorithms 3, 5).
fn rc4_rounds(key: &[u8], data: &[u8]) -> Vec<u8> {
    (1..=19u8).fold(rc4(key, data), |data, i| {
        let round: Vec<u8> = key.iter().map(|b| b ^ i).collect();
        rc4(&round, &data)
    })
}

fn cbc(cipher: &impl BlockEncrypt, iv: &[u8], data: &[u8]) -> Vec<u8> {
    let mut previous = iv.to_vec();
    let mut out = Vec::new();
    for chunk in data.chunks(16) {
        let mut block = GenericArray::clone_from_slice(chunk);
        block.iter_mut().zip(&previous).for_each(|(a, b)| *a ^= b);
        cipher.encrypt_block(&mut block);
        previous = block.to_vec();
        out.extend_from_slice(&block);
    }
    out
}

/// IV, then CBC with PKCS#7 padding; the IV is derived from the data.
fn aes_padded(key: &[u8], data: &[u8]) -> Vec<u8> {
    let iv = Md5::digest(data).to_vec();
    let mut padded = data.to_vec();
    let pad = 16 - data.len() % 16;
    padded.resize(data.len() + pad, pad as u8);
    let body = if key.len() == 16 {
        cbc(&Aes128::new(GenericArray::from_slice(key)), &iv, &padded)
    } else {
        cbc(&Aes256::new(GenericArray::from_slice(key)), &iv, &padded)
    };
    [iv, body].concat()
}

/// A standard handler R2-R4 (Algorithms 2, 3, and 4 or 5) with /P -3904:
/// V1 and V2 use RC4 with 40- and 128-bit keys, V4 the `cfm` crypt filter
/// with a 128-bit key. Passwords are hashed as given.
struct Legacy {
    v: i64,
    r: i64,
    cfm: Option<&'static str>,
    user: &'static [u8],
    owner: &'static [u8],
    id: bool,
    metadata: bool,
}

impl Legacy {
    fn new(
        v: i64,
        r: i64,
        cfm: Option<&'static str>,
        user: &'static [u8],
        owner: &'static [u8],
    ) -> Self {
        Self {
            v,
            r,
            cfm,
            user,
            owner,
            id: true,
            metadata: true,
        }
    }
}

fn legacy_document(legacy: Legacy) -> Vec<u8> {
    let p: i32 = -3904;
    let bits = if legacy.v == 1 { 40 } else { 128 };
    let n = bits / 8;
    let id: &[u8] = if legacy.id { ID0 } else { b"" };
    let seal = |key: &[u8], data: &[u8]| match legacy.r {
        2 => rc4(key, data),
        _ => rc4_rounds(key, data),
    };
    let mut owner_key = Md5::digest(padded(legacy.owner)).to_vec();
    if legacy.r >= 3 {
        for _ in 0..50 {
            owner_key = Md5::digest(&owner_key).to_vec();
        }
    }
    let o = seal(&owner_key[..n], &padded(legacy.user));
    let mut md5 = Md5::new();
    md5.update(padded(legacy.user));
    md5.update(&o);
    md5.update(p.to_le_bytes());
    md5.update(id);
    if legacy.r >= 4 && !legacy.metadata {
        md5.update([0xFF; 4]);
    }
    let mut key = md5.finalize().to_vec();
    if legacy.r >= 3 {
        for _ in 0..50 {
            key = Md5::digest(&key[..n]).to_vec();
        }
    }
    key.truncate(n);
    let u = match legacy.r {
        2 => rc4(&key, &PAD),
        _ => {
            let mut u = rc4_rounds(&key, &Md5::digest([PAD.as_slice(), id].concat()));
            u.extend_from_slice(&[0; 16]);
            u
        }
    };
    let filters = match legacy.cfm {
        Some(cfm) => {
            format!("/CF<</StdCF<</CFM/{cfm}/AuthEvent/DocOpen/Length 16>>>>/StmF/StdCF/StrF/StdCF")
        }
        None => String::new(),
    };
    let metadata = if legacy.metadata {
        ""
    } else {
        "/EncryptMetadata false"
    };
    let dictionary = format!(
        "<</Filter/Standard/V {}/R {}/Length {bits}/P {p}/O <{}>/U <{}>{metadata}{filters}>>",
        legacy.v,
        legacy.r,
        hex(&o),
        hex(&u)
    );
    let aes = legacy.cfm == Some("AESV2");
    let encrypt = move |number: u32, data: &[u8]| {
        let mut md5 = Md5::new();
        md5.update(&key);
        md5.update(&number.to_le_bytes()[..3]);
        md5.update([0, 0]);
        if aes {
            md5.update(b"sAlT");
            aes_padded(&md5.finalize(), data)
        } else {
            rc4(&md5.finalize()[..(n + 5).min(16)], data)
        }
    };
    exemption_document(Some(Crypt {
        encrypt: Box::new(encrypt),
        metadata: legacy.metadata,
        dictionary,
        id: legacy.id,
    }))
}

/// V4 R4 AES-128 with /EncryptMetadata false.
fn r4_document() -> Vec<u8> {
    legacy_document(Legacy {
        metadata: false,
        ..Legacy::new(4, 4, Some("AESV2"), b"u", b"o")
    })
}

/// V5 R5 AES-256: SHA-256 password hashes (Adobe extension level 3).
fn r5_document() -> Vec<u8> {
    let key: Vec<u8> = (100..132).collect();
    let wrap = |hash: &[u8], data: &[u8]| {
        cbc(&Aes256::new(GenericArray::from_slice(hash)), &[0; 16], data)
    };
    let sha = |parts: &[&[u8]]| Sha256::digest(parts.concat()).to_vec();
    let (uvs, uks, ovs, oks) = (b"uvsuvsuv", b"uksuksuk", b"ovsovsov", b"oksoksok");
    let u = [sha(&[b"u", uvs]).as_slice(), uvs, uks].concat();
    let ue = wrap(&sha(&[b"u", uks]), &key);
    let o = [sha(&[b"o", ovs, &u]).as_slice(), ovs, oks].concat();
    let oe = wrap(&sha(&[b"o", oks, &u]), &key);
    let p: i32 = -44;
    let perms = wrap(
        &key,
        &[&p.to_le_bytes()[..], &[0xFF; 4], b"Tadb", &[0; 4]].concat(),
    );
    let dictionary = format!(
        "<</Filter/Standard/V 5/R 5/Length 256/P {p}/O <{}>/U <{}>/OE <{}>/UE <{}>\
         /Perms <{}>/CF<</StdCF<</CFM/AESV3/AuthEvent/DocOpen/Length 32>>>>\
         /StmF/StdCF/StrF/StdCF>>",
        hex(&o),
        hex(&u),
        hex(&oe),
        hex(&ue),
        hex(&perms)
    );
    exemption_document(Some(Crypt {
        encrypt: Box::new(move |_, data| aes_padded(&key, data)),
        metadata: true,
        dictionary,
        id: true,
    }))
}

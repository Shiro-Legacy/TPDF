// SPDX-License-Identifier: MIT
//! Standard-handler decryption on the application's encrypted fixtures:
//! RC4-128 (V2 R3) and AES-256 (V5 R6), user pdf4all-user, owner pdf4all-owner.
#[path = "support/pages.rs"]
mod pages;
use tandem_pdf_engine::{
    document::{Document, DocumentErrorKind, DocumentLimits},
    stream::StreamLimits,
    stream_object::decode_document_stream,
    writer::{DocumentWriteLimits, rewrite_document},
};

const FILES: [&str; 5] = [
    "encrypted.pdf",
    "encrypted-noedit.pdf",
    "encrypted-noprint.pdf",
    "encrypted-aes256.pdf",
    "p3/encrypted-edit-noannot.pdf",
];

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("../fixtures/{name}")).unwrap()
}

/// The first page's content streams, decoded and joined in order.
fn first_page_content(doc: &Document<'_>) -> Vec<u8> {
    let page = pages::page_ids(doc)[0];
    pages::contents(doc, page)
        .into_iter()
        .flat_map(|id| decode_document_stream(doc, id, StreamLimits::default()).unwrap())
        .collect()
}

#[test]
fn wrong_or_missing_password_is_refused() {
    for name in FILES {
        let bytes = fixture(name);
        for password in [b"".as_slice(), b"nope", b"PDF4ALL-USER"] {
            let error = Document::open_with_password(&bytes, DocumentLimits::default(), password)
                .unwrap_err();
            assert!(
                matches!(error.kind, DocumentErrorKind::PasswordRequired),
                "{name}"
            );
        }
    }
}

#[test]
fn user_and_owner_passwords_decrypt_content() {
    for name in FILES {
        let bytes = fixture(name);
        for (password, owner) in [
            (b"pdf4all-user".as_slice(), false),
            (b"pdf4all-owner", true),
        ] {
            let doc =
                Document::open_with_password(&bytes, DocumentLimits::default(), password).unwrap();
            assert_eq!(doc.security().unwrap().owner, owner, "{name}");
            let text = first_page_content(&doc);
            assert!(
                text.windows(2).any(|w| w == b"BT"),
                "{name}: plaintext content"
            );
            // Encrypted output would need the original key; saving is refused.
            assert!(rewrite_document(&doc, DocumentWriteLimits::default()).is_err());
        }
    }
}

/// Cipher families the application fixtures lack, written by a reference writer:
/// RC4-40 (R2), AES-128 (V4 R4), and AES object streams whose members are
/// decrypted with their container. User "u", owner "o".
#[test]
fn generated_cipher_families_decrypt() {
    for name in [
        "encrypted-rc4-40.pdf",
        "encrypted-aes128.pdf",
        "encrypted-aes128-objstm.pdf",
        "encrypted-aes256-objstm.pdf",
    ] {
        let bytes = std::fs::read(format!("tests/data/{name}")).unwrap();
        assert!(
            Document::open(&bytes, DocumentLimits::default()).is_err(),
            "{name}"
        );
        for (password, owner) in [(b"u".as_slice(), false), (b"o", true)] {
            let doc =
                Document::open_with_password(&bytes, DocumentLimits::default(), password).unwrap();
            assert_eq!(doc.security().unwrap().owner, owner, "{name}");
        }
    }
}

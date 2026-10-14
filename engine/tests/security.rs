// SPDX-License-Identifier: AGPL-3.0-or-later
//! Standard-handler decryption on the application's encrypted fixtures:
//! RC4-128 (V2 R3) and AES-256 (V5 R6), user pdf4all-user, owner pdf4all-owner.
use tandem_pdf_engine::{
    document::{Document, DocumentErrorKind, DocumentLimits},
    page_content::load_page_content,
    pages::collect_pages,
    session::{Session, SessionLimits},
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
        let Err(error) = Session::open_view(bytes, SessionLimits::default()) else {
            panic!("{name} opened without a password");
        };
        assert_eq!(error.reason, "needs-password", "{name}");
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
            let pages = collect_pages(&doc, Default::default()).unwrap();
            let content = load_page_content(&doc, &pages[0], Default::default()).unwrap();
            let text = &content.bytes;
            assert!(
                text.windows(2).any(|w| w == b"BT"),
                "{name}: plaintext content"
            );
            // Encrypted output would need the original key; saving is refused.
            assert!(rewrite_document(&doc, DocumentWriteLimits::default()).is_err());
        }
    }
}

#[test]
fn encrypted_session_is_view_only_and_renders() {
    for name in FILES {
        let mut session = Session::open_view_with_password(
            fixture(name),
            SessionLimits::default(),
            b"pdf4all-user",
        )
        .unwrap();
        let caps = session.capabilities();
        assert_eq!(caps.save_error.as_ref().unwrap().reason, "encrypted");
        assert!(caps.pages[0].render_error.is_none(), "{name}");
        session.render(0, 36.0).unwrap();
        assert!(session.save().is_err());
    }
}

/// MuPDF-written families the application fixtures lack (tools/generate-encrypted-cases.mjs):
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
        let mut session =
            Session::open_view_with_password(bytes, SessionLimits::default(), b"u").unwrap();
        assert!(
            session.capabilities().pages[0].render_error.is_none(),
            "{name}"
        );
        session.render(0, 36.0).unwrap();
    }
}

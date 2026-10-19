// SPDX-License-Identifier: MIT
//! The repository's signing fixtures (`fixtures/p0b`, `fixtures/p5`) through
//! the Rust code, with the answers the TypeScript verifier gave
//! (tests/p5-verify-crypto.test.mjs). The key material under
//! `fixtures/p0b/.ca` and `fixtures/p5/*.p12|*.pem` is not in git
//! (`bash fixtures/p0b/make-signed.sh`, `bash fixtures/p5/make-p12.sh`); a test
//! whose file is missing says so and passes, as the gates machine has them.

use serde_json::{Value, json};
use std::path::PathBuf;
use tandem_sign::cert::{Cert, certs_from_pem};
use tandem_sign::cms::{self, Offsets};
use tandem_sign::drbg::HmacDrbg;
use tandem_sign::identity::{Identity, SignError};
use tandem_sign::verify::{Coverage, Status, Trust, Verdict, verify_signature};

const NOW: i64 = 1_792_000_000_000; // 2026-10-15

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> Option<Vec<u8>> {
    let path = root().join(rel);
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!("skipped: {} is missing", path.display());
            None
        }
    }
}

fn pem_cert(rel: &str) -> Option<Cert> {
    let der = certs_from_pem(&read(rel)?).into_iter().next()?;
    Cert::parse(&der)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn numbers_after(hay: &[u8], at: usize, n: usize) -> Vec<u64> {
    let text = String::from_utf8_lossy(&hay[at..(at + 200).min(hay.len())]).into_owned();
    text.split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .take(n)
        .map(|s| s.parse().unwrap())
        .collect()
}

/// The engine's sign-list entry for the first signature, as the JS tests build it.
fn entry_from(pdf: &[u8]) -> Value {
    let br = find(pdf, b"/ByteRange").unwrap();
    let r = numbers_after(pdf, br + 10, 4);
    let hex = String::from_utf8_lossy(&pdf[r[1] as usize + 1..r[2] as usize - 1]).into_owned();
    let sub = find(pdf, b"/SubFilter").map(|at| {
        let rest = String::from_utf8_lossy(&pdf[at + 10..at + 60]).into_owned();
        rest.trim_start()
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || "/<>[".contains(c))
            .next()
            .unwrap()
            .to_string()
    });
    let docmdp = find(pdf, b"/TransformMethod /DocMDP")
        .or_else(|| find(pdf, b"/TransformMethod/DocMDP"))
        .and_then(|at| {
            let window = &pdf[at..(at + 260).min(pdf.len())];
            find(window, b"/P ").map(|p| numbers_after(window, p + 3, 1)[0])
        });
    json!({
        "field": "Sig1", "page": 1, "rect": null, "empty": false,
        "subFilter": sub.unwrap_or_default(),
        "byteRange": r,
        "contents": hex,
        "docmdp": docmdp,
    })
}

fn verify(pdf: &[u8], entry: &Value, trusted: &[&Cert]) -> Verdict {
    let anchors: Vec<Cert> = trusted
        .iter()
        .map(|c| Cert::parse(&c.der).unwrap())
        .collect();
    let src: &[u8] = pdf;
    verify_signature(&src, entry, &anchors, NOW)
}

#[test]
fn p0b_fixtures_match_the_contract_table() {
    let Some(ca) = pem_cert("fixtures/p0b/.ca/ca.pem") else {
        return;
    };
    let rows = [
        (
            "fixtures/p0b/signed-intact.pdf",
            Status::Valid,
            Coverage::EntireFile,
            Trust::Trusted,
        ),
        (
            "fixtures/p0b/signed-tampered.pdf",
            Status::Invalid,
            Coverage::EntireFile,
            Trust::None,
        ),
        (
            "fixtures/p0b/signed-untrusted.pdf",
            Status::Valid,
            Coverage::EntireFile,
            Trust::Untrusted,
        ),
        (
            "fixtures/p0b/certified-p2.pdf",
            Status::Valid,
            Coverage::EntireFile,
            Trust::Trusted,
        ),
        (
            "fixtures/p0b/certified-p2-filled.pdf",
            Status::Valid,
            Coverage::Partial,
            Trust::Trusted,
        ),
        (
            "fixtures/p0b/certified-p2-annot.pdf",
            Status::Valid,
            Coverage::Partial,
            Trust::Trusted,
        ),
    ];
    for (rel, status, coverage, trust) in rows {
        let pdf = read(rel).unwrap();
        let v = verify(&pdf, &entry_from(&pdf), &[&ca]);
        assert_eq!(
            (v.status, v.coverage, v.trust),
            (status, coverage, trust),
            "{rel}: {v:?}"
        );
        if status == Status::Invalid {
            assert_eq!(v.reason.as_deref(), Some("digest"));
        }
        if trust == Trust::Trusted {
            assert_eq!(v.issuer_cn.as_deref(), Some("P0B Test CA"));
        }
        if rel.contains("certified-p2") {
            assert_eq!(v.docmdp, Some(json!(2)));
        }
    }
}

#[test]
fn ec_sha1_and_expired_fixtures() {
    let Some(ca) = pem_cert("fixtures/p0b/.ca/ca.pem") else {
        return;
    };
    let ec = read("fixtures/p5/signed-ec.pdf").unwrap();
    let v = verify(&ec, &entry_from(&ec), &[&ca]);
    assert_eq!(
        (v.status, v.trust, v.digest),
        (Status::Valid, Trust::Trusted, Some("sha256"))
    );
    assert_eq!(v.signer_cn.as_deref(), Some("P0B EC"));

    let sha1 = read("fixtures/p5/signed-sha1.pdf").unwrap();
    let v = verify(&sha1, &entry_from(&sha1), &[&ca]);
    assert_eq!(
        (v.status, v.trust, v.digest),
        (Status::Valid, Trust::Trusted, Some("sha1"))
    );
    assert_eq!(v.reason.as_deref(), Some("weak-digest"));

    let notime = read("fixtures/p5/signed-sha1-notime.pdf").unwrap();
    let v = verify(&notime, &entry_from(&notime), &[&ca]);
    assert_eq!((v.status, v.trust), (Status::Valid, Trust::Trusted));
    assert_eq!(v.reason.as_deref(), Some("weak-digest no-signing-time"));

    let expired = read("fixtures/p5/signed-expired.pdf").unwrap();
    let v = verify(&expired, &entry_from(&expired), &[&ca]);
    assert_eq!((v.status, v.trust), (Status::Valid, Trust::Expired));
    assert_eq!(v.issuer_cn, None);
}

#[test]
fn a_ca_false_issuer_does_not_extend_trust() {
    let Some(ca) = pem_cert("fixtures/p0b/.ca/ca.pem") else {
        return;
    };
    let Some(alice) = pem_cert("fixtures/p5/false-ca/alice.pem") else {
        return;
    };
    assert!(!alice.is_ca);
    let pdf = read("fixtures/p5/signed-false-ca.pdf").unwrap();
    let entry = entry_from(&pdf);
    let v = verify(&pdf, &entry, &[&ca]);
    assert_eq!((v.status, v.trust), (Status::Valid, Trust::Untrusted));
    assert_eq!(v.signer_cn.as_deref(), Some("Forged Bob"));
    assert_eq!(verify(&pdf, &entry, &[&alice]).trust, Trust::Untrusted);
    assert_eq!(verify(&pdf, &entry, &[&alice, &ca]).trust, Trust::Untrusted);
}

#[test]
fn byte_range_digest_coverage_and_subfilter_failures() {
    let Some(ca) = pem_cert("fixtures/p0b/.ca/ca.pem") else {
        return;
    };
    let pdf = read("fixtures/p0b/signed-intact.pdf").unwrap();
    let entry = entry_from(&pdf);
    let r: Vec<u64> = entry["byteRange"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_u64().unwrap())
        .collect();

    let mut flipped = pdf.clone();
    flipped[80] ^= 1;
    let v = verify(&flipped, &entry, &[&ca]);
    assert_eq!(
        (v.status, v.reason.as_deref()),
        (Status::Invalid, Some("digest"))
    );

    let mut widened = entry.clone();
    widened["byteRange"] = json!([0, r[1], r[2] + 2, r[3] - 2]);
    assert_eq!(
        verify(&pdf, &widened, &[&ca]).reason.as_deref(),
        Some("byte-range")
    );

    let mut spaced = pdf.clone();
    spaced[r[1] as usize + 1] = b' ';
    assert_eq!(
        verify(&spaced, &entry, &[&ca]).reason.as_deref(),
        Some("byte-range")
    );

    let mut appended = pdf.clone();
    appended.extend_from_slice(b"\ntrailer");
    let v = verify(&appended, &entry, &[&ca]);
    assert_eq!((v.status, v.coverage), (Status::Valid, Coverage::Partial));
    assert_eq!(
        (v.covered, v.total),
        (Some(pdf.len() as u64), Some(appended.len() as u64))
    );

    let mut cut = entry.clone();
    cut["byteRange"] = json!([0, r[1], r[2], r[3] - 10]);
    assert_eq!(
        verify(&pdf, &cut, &[&ca]).reason.as_deref(),
        Some("byte-range")
    );

    let mut sub = entry.clone();
    sub["subFilter"] = json!("adbe.x509.rsa_sha1");
    assert_eq!(verify(&pdf, &sub, &[&ca]).status, Status::Unsupported);
    sub["subFilter"] = json!("no.such.filter");
    assert_eq!(
        verify(&pdf, &sub, &[&ca]).reason.as_deref(),
        Some("no.such.filter")
    );
    assert_eq!(
        verify(
            &pdf,
            &json!({"field": "Blank", "page": 1, "rect": null, "empty": true}),
            &[]
        )
        .status,
        Status::Empty
    );
    let mut frac = entry.clone();
    frac["byteRange"] = json!([0, 1.5, 2, 3]);
    assert_eq!(
        verify(&pdf, &frac, &[&ca]).reason.as_deref(),
        Some("malformed")
    );
}

#[test]
fn p1_plus_a_later_revision_is_changes_not_permitted() {
    let Some(ca) = pem_cert("fixtures/p0b/.ca/ca.pem") else {
        return;
    };
    let pdf = read("fixtures/p5/certified-p1.pdf").unwrap();
    let mut entry = entry_from(&pdf);
    entry["docmdp"] = json!(1);
    let mut later = pdf.clone();
    later.extend_from_slice(b"%%later\n");
    let v = verify(&later, &entry, &[&ca]);
    assert_eq!(
        (v.status, v.trust, v.coverage),
        (Status::Valid, Trust::Trusted, Coverage::Partial)
    );
    assert_eq!(v.reason.as_deref(), Some("changes-not-permitted"));
}

#[test]
fn trust_anchors_empty_other_ca_the_ca_and_the_leaf() {
    let (Some(_ca_present), Some(other), Some(leaf)) = (
        pem_cert("fixtures/p0b/.ca/ca.pem"),
        pem_cert("fixtures/p0b/.ca/other-ca.pem"),
        pem_cert("fixtures/p0b/.ca/leaf.pem"),
    ) else {
        return;
    };
    let pdf = read("fixtures/p0b/signed-intact.pdf").unwrap();
    let entry = entry_from(&pdf);
    assert_eq!(verify(&pdf, &entry, &[]).trust, Trust::Untrusted);
    assert_eq!(verify(&pdf, &entry, &[&other]).trust, Trust::Untrusted);
    let v = verify(&pdf, &entry, &[&leaf]);
    assert_eq!(
        (v.trust, v.issuer_cn.as_deref()),
        (Trust::Trusted, Some("P0B Signer"))
    );
    assert!(!leaf.is_ca);
    let untrusted = read("fixtures/p0b/signed-untrusted.pdf").unwrap();
    assert_eq!(
        verify(&untrusted, &entry_from(&untrusted), &[&leaf]).trust,
        Trust::Untrusted
    );
}

#[test]
fn pkcs12_and_pem_identities() {
    let Some(p12) = read("fixtures/p5/leaf.p12") else {
        return;
    };
    let id = Identity::from_p12(&p12, "pdf4all").unwrap();
    assert_eq!(
        (id.key.kind(), id.key.bits(), id.certs.len()),
        ("rsa", 2048, 2)
    );
    assert_eq!(
        Cert::parse(&id.certs[0]).unwrap().subject_cn(),
        "P0B Signer"
    );
    assert_eq!(id.check_usable(NOW).unwrap().cn, "P0B Signer");
    assert_eq!(
        Identity::from_p12(&p12, "nope").unwrap_err(),
        SignError::BadPassword
    );

    let legacy =
        Identity::from_p12(&read("fixtures/p5/leaf-legacy.p12").unwrap(), "pdf4all").unwrap();
    assert_eq!(
        Cert::parse(&legacy.certs[0]).unwrap().subject_cn(),
        "P0B Signer"
    );
    assert_eq!(legacy.certs.len(), 2);

    let ec = Identity::from_p12(&read("fixtures/p5/leaf-ec.p12").unwrap(), "pdf4all").unwrap();
    assert_eq!((ec.key.kind(), ec.key.curve()), ("ec", Some("P-256")));
    assert_eq!(Cert::parse(&ec.certs[0]).unwrap().subject_cn(), "P0B EC");

    let plain = Identity::from_pem(&read("fixtures/p5/leaf.pem").unwrap(), "").unwrap();
    assert_eq!(plain.certs.len(), 2);
    assert_eq!(
        Cert::parse(&plain.certs[0]).unwrap().subject_cn(),
        "P0B Signer"
    );
    let enc = read("fixtures/p5/leaf-enc.pem").unwrap();
    assert_eq!(Identity::from_pem(&enc, "pdf4all").unwrap().certs.len(), 2);
    assert_eq!(
        Identity::from_pem(&enc, "nope").unwrap_err(),
        SignError::BadPassword
    );
}

#[test]
fn expired_and_not_yet_valid_leaves() {
    let Some(p12) = read("fixtures/p5/leaf.p12") else {
        return;
    };
    let id = Identity::from_p12(&p12, "pdf4all").unwrap();
    let leaf = Cert::parse(&id.certs[0]).unwrap();
    let after = id.check_usable(leaf.not_after_ms + 86_400_000).unwrap_err();
    assert_eq!(after.reason(), "certificate-expired");
    assert!(
        after
            .status()
            .unwrap()
            .starts_with("this certificate expired on ")
    );
    let early = id.check_usable(0).unwrap_err();
    assert!(
        early
            .status()
            .unwrap()
            .starts_with("this certificate is not valid until ")
    );
}

/// A minimal prepared PDF: a hole and the sentinel, as `sign-prepare` leaves them.
fn prepared() -> (Vec<u8>, Offsets) {
    let contents_len = cms::MAX_SIG_CONTENTS_BYTES * 2;
    let head = b"%PDF-1.7\n1 0 obj\n<</Type/Sig/Filter/Adobe.PPKLite/SubFilter/adbe.pkcs7.detached/Contents <";
    let contents_at = head.len() as u64;
    let mut pdf = head.to_vec();
    pdf.extend(std::iter::repeat_n(b'0', contents_len as usize));
    pdf.extend_from_slice(b">/ByteRange");
    let byte_range_at = pdf.len() as u64;
    pdf.extend_from_slice(cms::BYTE_RANGE_SENTINEL);
    pdf.extend_from_slice(b">>\nendobj\ntrailer\n<<>>\n%%EOF\n");
    (
        pdf,
        Offsets {
            contents_at,
            contents_len,
            byte_range_at,
        },
    )
}

#[test]
fn signs_a_prepared_pdf_that_then_verifies_and_tampering_fails() {
    let Some(ca) = pem_cert("fixtures/p0b/.ca/ca.pem") else {
        return;
    };
    for (file, password) in [
        ("fixtures/p5/leaf.p12", "pdf4all"),
        ("fixtures/p5/leaf-ec.p12", "pdf4all"),
    ] {
        let Some(bytes) = read(file) else { return };
        let id = Identity::from_p12(&bytes, password).unwrap();
        let (mut pdf, offsets) = prepared();
        let mut rng = HmacDrbg::new(&[9u8; 48]).unwrap();
        cms::sign_pdf(&mut pdf, offsets, &id.key, &id.certs, NOW, &mut rng).unwrap();
        let entry = entry_from(&pdf);
        let v = verify(&pdf, &entry, &[&ca]);
        assert_eq!(
            (v.status, v.trust, v.coverage),
            (Status::Valid, Trust::Trusted, Coverage::EntireFile),
            "{file}"
        );
        assert_eq!(v.signing_time_ms, Some(NOW / 1000 * 1000));
        let mut tampered = pdf.clone();
        let last = tampered.len() - 3;
        tampered[last] ^= 1;
        assert_eq!(
            verify(&tampered, &entry, &[&ca]).reason.as_deref(),
            Some("digest")
        );
        // The hole is checked: a prepared file that changed is refused.
        let (mut changed, offsets) = prepared();
        changed[offsets.contents_at as usize] = b'1';
        let err =
            cms::sign_pdf(&mut changed, offsets, &id.key, &id.certs, NOW, &mut rng).unwrap_err();
        assert_eq!(err, SignError::Unreadable);
    }
}

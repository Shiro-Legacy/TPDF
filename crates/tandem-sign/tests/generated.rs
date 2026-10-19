// SPDX-License-Identifier: MIT
//! Signing and verification on certificates made here (no fixture files), so
//! a clean checkout exercises the chain rules, the ESS binding and the report:
//! a root CA, an intermediate, leaves with RSA, P-256 and P-384 keys, a CA:FALSE
//! "issuer", a CA without keyCertSign, an expired leaf, and a signature whose
//! certificate was swapped for another one with the same key.

use rsa::traits::PublicKeyParts;
use serde_json::{Value, json};
use tandem_sign::asn1::{algorithm, oid, seq, tlv, uint};
use tandem_sign::cert::Cert;
use tandem_sign::cms::{self, Offsets};
use tandem_sign::drbg::HmacDrbg;
use tandem_sign::keys::SigningKey;
use tandem_sign::oids;
use tandem_sign::report::plain_sentence;
use tandem_sign::time::utc_ms;
use tandem_sign::verify::{Coverage, Status, Trust, verify_signature};

const NOW: i64 = 1_792_000_000_000;
const DAY: i64 = 86_400_000;

fn rng(seed: u8) -> HmacDrbg {
    HmacDrbg::new(&[seed; 48]).unwrap()
}

fn rsa_key(seed: u8) -> SigningKey {
    SigningKey::Rsa(Box::new(
        rsa::RsaPrivateKey::new(&mut rng(seed), 2048).unwrap(),
    ))
}

fn ec_key(seed: u8, p384: bool) -> SigningKey {
    let mut bytes = vec![seed; if p384 { 48 } else { 32 }];
    bytes[0] = 0x11;
    if p384 {
        SigningKey::P384(p384::SecretKey::from_slice(&bytes).unwrap())
    } else {
        SigningKey::P256(p256::SecretKey::from_slice(&bytes).unwrap())
    }
}

fn big_int(bytes: &[u8]) -> Vec<u8> {
    let mut b = bytes.to_vec();
    while b.len() > 1 && b[0] == 0 {
        b.remove(0);
    }
    if b[0] & 0x80 != 0 {
        b.insert(0, 0);
    }
    tlv(0x02, &b)
}

fn spki(key: &SigningKey) -> Vec<u8> {
    match key {
        SigningKey::Rsa(k) => {
            let public = seq(&[
                &big_int(&k.n().to_bytes_be()),
                &big_int(&k.e().to_bytes_be()),
            ]);
            seq(&[
                &algorithm(&oids::RSA, true),
                &tlv(0x03, &[&[0u8][..], &public].concat()),
            ])
        }
        SigningKey::P256(k) => {
            let point = p256::elliptic_curve::sec1::ToEncodedPoint::to_encoded_point(
                &k.public_key(),
                false,
            );
            seq(&[
                &seq(&[&oid(&oids::EC_PUBLIC_KEY), &oid(&oids::P256)]),
                &tlv(0x03, &[&[0u8][..], point.as_bytes()].concat()),
            ])
        }
        SigningKey::P384(k) => {
            let point = p384::elliptic_curve::sec1::ToEncodedPoint::to_encoded_point(
                &k.public_key(),
                false,
            );
            seq(&[
                &seq(&[&oid(&oids::EC_PUBLIC_KEY), &oid(&oids::P384)]),
                &tlv(0x03, &[&[0u8][..], point.as_bytes()].concat()),
            ])
        }
    }
}

fn name(cn: &str) -> Vec<u8> {
    seq(&[&tlv(
        0x31,
        &seq(&[&oid(&oids::COMMON_NAME), &tlv(0x0c, cn.as_bytes())]),
    )])
}

fn utc(ms: i64) -> Vec<u8> {
    tandem_sign::time::der_time(ms)
}

fn ext(id: &const_oid::ObjectIdentifier, critical: bool, value: Vec<u8>) -> Vec<u8> {
    if critical {
        seq(&[&oid(id), &[0x01, 0x01, 0xff], &tlv(0x04, &value)])
    } else {
        seq(&[&oid(id), &tlv(0x04, &value)])
    }
}

struct Spec<'a> {
    cn: &'a str,
    serial: u64,
    key: &'a SigningKey,
    issuer: Option<(&'a Cert, &'a SigningKey)>,
    ca: bool,
    /// keyUsage bits byte for a CA (0x06 = keyCertSign|cRLSign), or a leaf (0xc0 = digitalSignature|nonRepudiation).
    usage: u8,
    from: i64,
    to: i64,
    skid: u8,
}

fn make(s: Spec<'_>) -> Cert {
    let sig_alg = |k: &SigningKey| match k {
        SigningKey::Rsa(_) => algorithm(&oids::SHA256_RSA, true),
        SigningKey::P256(_) => algorithm(&oids::ECDSA_SHA256, false),
        SigningKey::P384(_) => algorithm(&oids::ECDSA_SHA384, false),
    };
    let signer = s.issuer.map_or(s.key, |(_, k)| k);
    let issuer_name = s
        .issuer
        .map_or_else(|| name(s.cn), |(c, _)| c.subject_bytes().to_vec());
    let mut exts = vec![
        ext(
            &oids::BASIC_CONSTRAINTS,
            true,
            if s.ca {
                seq(&[&[0x01, 0x01, 0xff]])
            } else {
                seq(&[])
            },
        ),
        ext(
            &oids::KEY_USAGE,
            true,
            tlv(0x03, &[if s.usage & 0x02 != 0 { 1 } else { 0 }, s.usage]),
        ),
        ext(&oids::SUBJECT_KEY_ID, false, tlv(0x04, &[s.skid; 20])),
    ];
    if let Some((issuer, _)) = s.issuer {
        let akid = issuer.subject_key_id.clone().unwrap();
        exts.push(ext(
            &oids::AUTHORITY_KEY_ID,
            false,
            seq(&[&tlv(0x80, &akid)]),
        ));
    }
    let tbs = seq(&[
        &tlv(0xa0, &uint(2)),
        &uint(s.serial),
        &sig_alg(signer),
        &issuer_name,
        &seq(&[&utc(s.from), &utc(s.to)]),
        &name(s.cn),
        &spki(s.key),
        &tlv(
            0xa3,
            &seq(&exts.iter().map(Vec::as_slice).collect::<Vec<_>>()),
        ),
    ]);
    let hash = signer.digest().hash(&tbs);
    let sig = signer.sign_hash(&hash, &mut rng(s.serial as u8)).unwrap();
    let der = seq(&[
        &tbs,
        &sig_alg(signer),
        &tlv(0x03, &[&[0u8][..], &sig].concat()),
    ]);
    Cert::parse(&der).expect("generated certificate parses")
}

fn prepared() -> (Vec<u8>, Offsets) {
    let contents_len = cms::MAX_SIG_CONTENTS_BYTES * 2;
    let head = b"%PDF-1.7\n1 0 obj\n<</Type/Sig/SubFilter/adbe.pkcs7.detached/Contents <";
    let contents_at = head.len() as u64;
    let mut pdf = head.to_vec();
    pdf.extend(std::iter::repeat_n(b'0', contents_len as usize));
    pdf.extend_from_slice(b">/ByteRange");
    let byte_range_at = pdf.len() as u64;
    pdf.extend_from_slice(cms::BYTE_RANGE_SENTINEL);
    pdf.extend_from_slice(b">>\nendobj\n%%EOF\n");
    (
        pdf,
        Offsets {
            contents_at,
            contents_len,
            byte_range_at,
        },
    )
}

fn sign(key: &SigningKey, certs: &[&Cert], when: i64) -> (Vec<u8>, Value) {
    let (mut pdf, o) = prepared();
    let ders: Vec<Vec<u8>> = certs.iter().map(|c| c.der.clone()).collect();
    cms::sign_pdf(&mut pdf, o, key, &ders, when, &mut rng(42)).unwrap();
    let b = o.contents_at - 1;
    let c = o.contents_at + o.contents_len + 1;
    let hex = String::from_utf8(pdf[b as usize + 1..c as usize - 1].to_vec()).unwrap();
    let entry = json!({
        "field": "Sig1", "page": 0, "empty": false, "subFilter": "adbe.pkcs7.detached",
        "byteRange": [0, b, c, pdf.len() as u64 - c], "contents": hex, "docmdp": null,
    });
    (pdf, entry)
}

fn check(pdf: &[u8], entry: &Value, trusted: &[&Cert]) -> tandem_sign::verify::Verdict {
    let anchors: Vec<Cert> = trusted
        .iter()
        .map(|c| Cert::parse(&c.der).unwrap())
        .collect();
    let src: &[u8] = pdf;
    verify_signature(&src, entry, &anchors, NOW)
}

struct World {
    root_key: SigningKey,
    root: Cert,
    mid_key: SigningKey,
    mid: Cert,
}

fn world() -> World {
    let root_key = ec_key(1, true);
    let root = make(Spec {
        cn: "Test Root",
        serial: 1,
        key: &root_key,
        issuer: None,
        ca: true,
        usage: 0x06,
        from: NOW - 365 * DAY,
        to: NOW + 3650 * DAY,
        skid: 1,
    });
    let mid_key = ec_key(2, false);
    let mid = make(Spec {
        cn: "Test Intermediate",
        serial: 2,
        key: &mid_key,
        issuer: Some((&root, &root_key)),
        ca: true,
        usage: 0x06,
        from: NOW - 365 * DAY,
        to: NOW + 3650 * DAY,
        skid: 2,
    });
    World {
        root_key,
        root,
        mid_key,
        mid,
    }
}

#[test]
fn rsa_p256_and_p384_leaves_sign_through_an_intermediate_to_a_trusted_root() {
    let w = world();
    for (i, key) in [rsa_key(10), ec_key(11, false), ec_key(12, true)]
        .iter()
        .enumerate()
    {
        let leaf = make(Spec {
            cn: "Jane Doe",
            serial: 100 + i as u64,
            key,
            issuer: Some((&w.mid, &w.mid_key)),
            ca: false,
            usage: 0xc0,
            from: NOW - DAY,
            to: NOW + 30 * DAY,
            skid: 50 + i as u8,
        });
        let when = utc_ms(2026, 10, 18, 9, 30, 0);
        let (pdf, entry) = sign(key, &[&leaf, &w.mid], when);
        let v = check(&pdf, &entry, &[&w.root]);
        assert_eq!(
            (v.status, v.trust, v.coverage),
            (Status::Valid, Trust::Trusted, Coverage::EntireFile),
            "{} {v:?}",
            key.kind()
        );
        assert_eq!(v.issuer_cn.as_deref(), Some("Test Root"));
        assert_eq!(
            v.digest,
            Some(if matches!(key, SigningKey::P384(_)) {
                "sha384"
            } else {
                "sha256"
            })
        );
        assert_eq!(
            plain_sentence(&v),
            "Signed by Jane Doe on 18 Oct 2026. The document has not changed since it was signed. The signer's certificate is trusted: it was issued through Test Root, which is in your trusted list."
        );
        // Without the intermediate in the signature the chain stops; trusting it directly works.
        let (pdf2, entry2) = sign(key, &[&leaf], when);
        assert_eq!(check(&pdf2, &entry2, &[&w.root]).trust, Trust::Untrusted);
        assert_eq!(check(&pdf2, &entry2, &[&w.mid]).trust, Trust::Trusted);
        assert_eq!(
            plain_sentence(&check(&pdf2, &entry2, &[])),
            "Signed by Jane Doe on 18 Oct 2026. The document has not changed since it was signed. The signer's certificate is not in your trusted list."
        );
    }
}

#[test]
fn issuers_must_be_cas_that_may_sign_certificates() {
    let w = world();
    let not_ca_key = ec_key(20, false);
    let not_ca = make(Spec {
        cn: "Honest Alice",
        serial: 20,
        key: &not_ca_key,
        issuer: Some((&w.root, &w.root_key)),
        ca: false,
        usage: 0xc0,
        from: NOW - DAY,
        to: NOW + 30 * DAY,
        skid: 20,
    });
    let no_cert_sign_key = ec_key(21, false);
    let no_cert_sign = make(Spec {
        cn: "No CertSign CA",
        serial: 21,
        key: &no_cert_sign_key,
        issuer: Some((&w.root, &w.root_key)),
        ca: true,
        usage: 0x80,
        from: NOW - DAY,
        to: NOW + 30 * DAY,
        skid: 21,
    });
    for (issuer, issuer_key) in [(&not_ca, &not_ca_key), (&no_cert_sign, &no_cert_sign_key)] {
        let key = ec_key(22, false);
        let leaf = make(Spec {
            cn: "Forged Bob",
            serial: 22,
            key: &key,
            issuer: Some((issuer, issuer_key)),
            ca: false,
            usage: 0xc0,
            from: NOW - DAY,
            to: NOW + 30 * DAY,
            skid: 22,
        });
        let (pdf, entry) = sign(&key, &[&leaf, issuer], NOW);
        let v = check(&pdf, &entry, &[&w.root]);
        assert_eq!(
            (v.status, v.trust),
            (Status::Valid, Trust::Untrusted),
            "{}",
            issuer.subject_cn()
        );
        assert_eq!(check(&pdf, &entry, &[issuer]).trust, Trust::Untrusted);
    }
}

#[test]
fn an_expired_link_is_expired_and_signing_refuses_an_expired_leaf() {
    let w = world();
    let key = ec_key(30, false);
    let leaf = make(Spec {
        cn: "Old Key",
        serial: 30,
        key: &key,
        issuer: Some((&w.mid, &w.mid_key)),
        ca: false,
        usage: 0xc0,
        from: NOW - 30 * DAY,
        to: NOW - DAY,
        skid: 30,
    });
    let (pdf, entry) = sign(&key, &[&leaf, &w.mid], NOW);
    let v = check(&pdf, &entry, &[&w.root]);
    assert_eq!((v.status, v.trust), (Status::Valid, Trust::Expired));
    assert!(
        plain_sentence(&v)
            .ends_with("The signer's certificate was not valid on the date it was signed.")
    );
    let err = tandem_sign::identity::check_usable(std::slice::from_ref(&leaf.der), &key, NOW)
        .unwrap_err();
    assert_eq!(err.reason(), "certificate-expired");
}

#[test]
fn a_swapped_certificate_of_equal_size_is_caught() {
    // Names of equal length so the swapped certificate has the same size as the real one.
    let w = world();
    let key = ec_key(50, false);
    let real = make(Spec {
        cn: "Jane Doe A",
        serial: 50,
        key: &key,
        issuer: Some((&w.mid, &w.mid_key)),
        ca: false,
        usage: 0xc0,
        from: NOW - DAY,
        to: NOW + 30 * DAY,
        skid: 50,
    });
    // ECDSA signatures vary in DER length by a byte or two: pick a serial whose certificate has the same size.
    let other = (51..120)
        .map(|serial| {
            make(Spec {
                cn: "Jane Doe B",
                serial,
                key: &key,
                issuer: Some((&w.mid, &w.mid_key)),
                ca: false,
                usage: 0xc0,
                from: NOW - DAY,
                to: NOW + 30 * DAY,
                skid: 50,
            })
        })
        .find(|c| c.der.len() == real.der.len())
        .unwrap();
    let (pdf, entry) = sign(&key, &[&real, &w.mid], NOW);
    let hex = entry["contents"].as_str().unwrap();
    let to_hex = |b: &[u8]| b.iter().map(|x| format!("{x:02X}")).collect::<String>();
    // The SignerInfo's serial comes after the certificates; replace the certificate first, then the sid serial.
    let swapped = hex.replacen(&to_hex(&real.der), &to_hex(&other.der), 1);
    let sid_at = swapped.rfind(&to_hex(real.serial_bytes())).unwrap();
    let mut swapped = swapped;
    swapped.replace_range(
        sid_at..sid_at + 2 * real.serial_bytes().len(),
        &to_hex(other.serial_bytes()),
    );
    let mut bad = pdf.clone();
    let b = entry["byteRange"][1].as_u64().unwrap() as usize;
    bad[b + 1..b + 1 + swapped.len()].copy_from_slice(swapped.as_bytes());
    let mut e = entry.clone();
    e["contents"] = json!(swapped);
    let v = check(&bad, &e, &[&w.root]);
    assert_eq!(
        (v.status, v.reason.as_deref()),
        (Status::Invalid, Some("signer-cert")),
        "{v:?}"
    );
    assert!(plain_sentence(&v).contains("does not belong to the signer"));
}

#[test]
fn a_tampered_byte_and_a_later_revision() {
    let w = world();
    let key = rsa_key(60);
    let leaf = make(Spec {
        cn: "Jane Doe",
        serial: 60,
        key: &key,
        issuer: Some((&w.mid, &w.mid_key)),
        ca: false,
        usage: 0xc0,
        from: NOW - DAY,
        to: NOW + 30 * DAY,
        skid: 60,
    });
    let (pdf, entry) = sign(&key, &[&leaf, &w.mid], NOW);
    let mut tampered = pdf.clone();
    tampered[3] ^= 1;
    let v = check(&tampered, &entry, &[&w.root]);
    assert_eq!(
        (v.status, v.reason.as_deref()),
        (Status::Invalid, Some("digest"))
    );
    let mut later = pdf.clone();
    later.extend_from_slice(b"1 0 obj\n<<>>\nendobj\n%%EOF\n");
    let v = check(&later, &entry, &[&w.root]);
    assert_eq!((v.status, v.coverage), (Status::Valid, Coverage::Partial));
    assert!(plain_sentence(&v).contains("Later changes were added after signing"));
    let mut certified = entry.clone();
    certified["docmdp"] = json!(1);
    let v = check(&later, &certified, &[&w.root]);
    assert_eq!(v.reason.as_deref(), Some("changes-not-permitted"));
    assert!(plain_sentence(&v).contains("although the signer allowed no changes"));
}

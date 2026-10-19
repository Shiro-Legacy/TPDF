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

/// What else a certificate made here may have.
#[derive(Default)]
struct Opts {
    /// Signed with SHA-1 (RSA issuers only).
    sha1: bool,
    /// Signed with RSA-PSS over SHA-1 (RSA issuers only; RFC 4055 default parameters).
    pss_sha1: bool,
    /// basicConstraints pathLenConstraint.
    path_len: Option<u64>,
    /// Extra extensions, DER.
    extra: Vec<Vec<u8>>,
    /// keyUsage replaced by these two bytes (bits, as `Cert::key_usage`).
    usage16: Option<u16>,
}

fn make(s: Spec<'_>) -> Cert {
    make_with(s, Opts::default())
}

fn make_with(s: Spec<'_>, o: Opts) -> Cert {
    let sig_alg = |k: &SigningKey| match k {
        SigningKey::Rsa(_) if o.sha1 => algorithm(&oids::SHA1_RSA, true),
        SigningKey::Rsa(_) if o.pss_sha1 => seq(&[&oid(&oids::RSA_PSS), &seq(&[])]),
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
            match (s.ca, o.path_len) {
                (true, Some(n)) => seq(&[&[0x01, 0x01, 0xff], &uint(n)]),
                (true, None) => seq(&[&[0x01, 0x01, 0xff]]),
                _ => seq(&[]),
            },
        ),
        ext(
            &oids::KEY_USAGE,
            true,
            match o.usage16 {
                Some(u) => tlv(0x03, &[0, (u >> 8) as u8, u as u8]),
                None => tlv(0x03, &[if s.usage & 0x02 != 0 { 1 } else { 0 }, s.usage]),
            },
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
    exts.extend(o.extra.iter().cloned());
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
    let sig = match signer {
        SigningKey::Rsa(k) if o.pss_sha1 => k
            .sign_with_rng(
                &mut rng(s.serial as u8),
                rsa::Pss::new_with_salt::<sha1::Sha1>(20),
                &tandem_sign::digest::DigestAlg::Sha1.hash(&tbs),
            )
            .unwrap(),
        SigningKey::Rsa(k) if o.sha1 => k
            .sign_with_rng(
                &mut rng(s.serial as u8),
                rsa::Pkcs1v15Sign::new::<sha1::Sha1>(),
                &tandem_sign::digest::DigestAlg::Sha1.hash(&tbs),
            )
            .unwrap(),
        _ => signer
            .sign_hash(&signer.digest().hash(&tbs), &mut rng(s.serial as u8))
            .unwrap(),
    };
    let der = seq(&[
        &tbs,
        &sig_alg(signer),
        &tlv(0x03, &[&[0u8][..], &sig].concat()),
    ]);
    Cert::parse(&der).expect("generated certificate parses")
}

fn prepared(when: i64) -> (Vec<u8>, Offsets) {
    let contents_len = cms::MAX_SIG_CONTENTS_BYTES * 2;
    let head = format!(
        "%PDF-1.7\n1 0 obj\n<</Type/Sig/SubFilter/ETSI.CAdES.detached/M({})/Contents <",
        tandem_sign::time::pdf_date(when)
    );
    let contents_at = head.len() as u64;
    let mut pdf = head.into_bytes();
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
    let (mut pdf, o) = prepared(when);
    let ders: Vec<Vec<u8>> = certs.iter().map(|c| c.der.clone()).collect();
    cms::sign_pdf(&mut pdf, o, key, &ders, &mut rng(42)).unwrap();
    let b = o.contents_at - 1;
    let c = o.contents_at + o.contents_len + 1;
    let hex = String::from_utf8(pdf[b as usize + 1..c as usize - 1].to_vec()).unwrap();
    let entry = json!({
        "field": "Sig1", "page": 0, "empty": false, "subFilter": "ETSI.CAdES.detached",
        "m": tandem_sign::time::pdf_date(when),
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
            "Signed by “Jane Doe”, dated 18 Oct 2026. The document has not changed since it was signed. The signer's certificate is trusted: it was issued through “Test Root”, which is in your trusted list."
        );
        // Without the intermediate in the signature the chain stops; trusting it directly works.
        let (pdf2, entry2) = sign(key, &[&leaf], when);
        assert_eq!(check(&pdf2, &entry2, &[&w.root]).trust, Trust::Untrusted);
        assert_eq!(check(&pdf2, &entry2, &[&w.mid]).trust, Trust::Trusted);
        assert_eq!(
            plain_sentence(&check(&pdf2, &entry2, &[])),
            "The signer's certificate is not in your trusted list, so the name is not confirmed. Signed by “Jane Doe”, dated 18 Oct 2026. The document has not changed since it was signed."
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
            .starts_with("The signer's certificate was not valid on the date it was signed")
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

fn root_rsa(seed: u8, bits: usize, serial: u64, skid: u8) -> (SigningKey, Cert) {
    let key = SigningKey::Rsa(Box::new(
        rsa::RsaPrivateKey::new(&mut rng(seed), bits).unwrap(),
    ));
    let cert = make(Spec {
        cn: "RSA Root",
        serial,
        key: &key,
        issuer: None,
        ca: true,
        usage: 0x06,
        from: NOW - 365 * DAY,
        to: NOW + 3650 * DAY,
        skid,
    });
    (key, cert)
}

fn leaf_under(
    issuer: &Cert,
    issuer_key: &SigningKey,
    key: &SigningKey,
    serial: u64,
    o: Opts,
) -> Cert {
    make_with(
        Spec {
            cn: "Jane Doe",
            serial,
            key,
            issuer: Some((issuer, issuer_key)),
            ca: false,
            usage: 0xc0,
            from: NOW - DAY,
            to: NOW + 30 * DAY,
            skid: serial as u8,
        },
        o,
    )
}

fn has_note(v: &tandem_sign::verify::Verdict, note: &str) -> bool {
    v.reason
        .as_deref()
        .is_some_and(|r| r.split(' ').any(|n| n == note))
}

#[test]
fn links_with_outdated_security_are_not_trusted() {
    let (root_key, root) = root_rsa(70, 2048, 70, 70);
    // An intermediate signed with SHA-1.
    let mid_key = ec_key(71, false);
    let mid = make_with(
        Spec {
            cn: "SHA-1 Intermediate",
            serial: 71,
            key: &mid_key,
            issuer: Some((&root, &root_key)),
            ca: true,
            usage: 0x06,
            from: NOW - DAY,
            to: NOW + 30 * DAY,
            skid: 71,
        },
        Opts {
            sha1: true,
            ..Opts::default()
        },
    );
    let key = ec_key(72, false);
    let leaf = leaf_under(&mid, &mid_key, &key, 72, Opts::default());
    let (pdf, entry) = sign(&key, &[&leaf, &mid], NOW);
    let v = check(&pdf, &entry, &[&root]);
    assert_eq!(
        (v.status, v.trust),
        (Status::Valid, Trust::Untrusted),
        "{v:?}"
    );
    assert!(has_note(&v, "outdated-security"));
    assert!(plain_sentence(&v).starts_with("The signer's certificate uses outdated security"));
    // Trusting the intermediate itself avoids the SHA-1 link.
    assert_eq!(check(&pdf, &entry, &[&mid]).trust, Trust::Trusted);

    // A root with a 1024-bit RSA key.
    let (small_key, small_root) = root_rsa(73, 1024, 73, 73);
    let key = ec_key(74, false);
    let leaf = leaf_under(&small_root, &small_key, &key, 74, Opts::default());
    let (pdf, entry) = sign(&key, &[&leaf], NOW);
    let v = check(&pdf, &entry, &[&small_root]);
    assert_eq!(v.trust, Trust::Untrusted);
    assert!(has_note(&v, "outdated-security"));

    // A document signed with a 1024-bit key, trusted directly.
    let weak = SigningKey::Rsa(Box::new(
        rsa::RsaPrivateKey::new(&mut rng(75), 1024).unwrap(),
    ));
    let leaf = leaf_under(&root, &root_key, &weak, 75, Opts::default());
    let (pdf, entry) = sign(&weak, &[&leaf], NOW);
    let v = check(&pdf, &entry, &[&leaf]);
    assert_eq!((v.status, v.trust), (Status::Valid, Trust::Untrusted));
    assert!(has_note(&v, "outdated-security"));
}

#[test]
fn path_length_and_unknown_critical_extensions_stop_a_chain() {
    let root_key = ec_key(80, true);
    for (path_len, trusted) in [
        (Some(0), Trust::Untrusted),
        (Some(1), Trust::Trusted),
        (None, Trust::Trusted),
    ] {
        let root = make_with(
            Spec {
                cn: "Path Root",
                serial: 80,
                key: &root_key,
                issuer: None,
                ca: true,
                usage: 0x06,
                from: NOW - DAY,
                to: NOW + 30 * DAY,
                skid: 80,
            },
            Opts {
                path_len,
                ..Opts::default()
            },
        );
        let mid_key = ec_key(81, false);
        let mid = make(Spec {
            cn: "Path Intermediate",
            serial: 81,
            key: &mid_key,
            issuer: Some((&root, &root_key)),
            ca: true,
            usage: 0x06,
            from: NOW - DAY,
            to: NOW + 30 * DAY,
            skid: 81,
        });
        let key = ec_key(82, false);
        let leaf = leaf_under(&mid, &mid_key, &key, 82, Opts::default());
        let (pdf, entry) = sign(&key, &[&leaf, &mid], NOW);
        assert_eq!(
            check(&pdf, &entry, &[&root]).trust,
            trusted,
            "pathLen {path_len:?}"
        );
    }
    let w = world();
    let odd = ext(
        &const_oid::ObjectIdentifier::new_unwrap("1.2.3.4.5"),
        true,
        tlv(0x05, &[]),
    );
    let mid_key = ec_key(83, false);
    let mid = make_with(
        Spec {
            cn: "Odd Intermediate",
            serial: 83,
            key: &mid_key,
            issuer: Some((&w.root, &w.root_key)),
            ca: true,
            usage: 0x06,
            from: NOW - DAY,
            to: NOW + 30 * DAY,
            skid: 83,
        },
        Opts {
            extra: vec![odd.clone()],
            ..Opts::default()
        },
    );
    let key = ec_key(84, false);
    let leaf = leaf_under(&mid, &mid_key, &key, 84, Opts::default());
    let (pdf, entry) = sign(&key, &[&leaf, &mid], NOW);
    assert_eq!(check(&pdf, &entry, &[&w.root]).trust, Trust::Untrusted);
    // The same extension, not critical, is ignored.
    let plain_ext = ext(
        &const_oid::ObjectIdentifier::new_unwrap("1.2.3.4.5"),
        false,
        tlv(0x05, &[]),
    );
    let mid = make_with(
        Spec {
            cn: "Odd Intermediate",
            serial: 85,
            key: &mid_key,
            issuer: Some((&w.root, &w.root_key)),
            ca: true,
            usage: 0x06,
            from: NOW - DAY,
            to: NOW + 30 * DAY,
            skid: 83,
        },
        Opts {
            extra: vec![plain_ext],
            ..Opts::default()
        },
    );
    let leaf = leaf_under(&mid, &mid_key, &key, 86, Opts::default());
    let (pdf, entry) = sign(&key, &[&leaf, &mid], NOW);
    assert_eq!(check(&pdf, &entry, &[&w.root]).trust, Trust::Trusted);
}

#[test]
fn the_signers_certificate_must_be_meant_for_signing() {
    let w = world();
    let eku = |id: &str| {
        ext(
            &oids::EXT_KEY_USAGE,
            false,
            seq(&[&oid(&const_oid::ObjectIdentifier::new_unwrap(id))]),
        )
    };
    let odd = ext(
        &const_oid::ObjectIdentifier::new_unwrap("1.2.3.4.5"),
        true,
        tlv(0x05, &[]),
    );
    let cases = [
        (
            "keyCertSign only",
            Opts {
                usage16: Some(0x0400),
                ..Opts::default()
            },
            Trust::Untrusted,
        ),
        (
            "server authentication only",
            Opts {
                extra: vec![eku("1.3.6.1.5.5.7.3.1")],
                ..Opts::default()
            },
            Trust::Untrusted,
        ),
        (
            "an unknown critical extension",
            Opts {
                extra: vec![odd],
                ..Opts::default()
            },
            Trust::Untrusted,
        ),
        (
            "email protection",
            Opts {
                extra: vec![eku("1.3.6.1.5.5.7.3.4")],
                ..Opts::default()
            },
            Trust::Trusted,
        ),
        (
            "document signing",
            Opts {
                extra: vec![eku("1.3.6.1.5.5.7.3.36")],
                ..Opts::default()
            },
            Trust::Trusted,
        ),
        (
            "nonRepudiation only",
            Opts {
                usage16: Some(0x4000),
                ..Opts::default()
            },
            Trust::Trusted,
        ),
    ];
    for (i, (label, opts, want)) in cases.into_iter().enumerate() {
        let key = ec_key(90 + i as u8, false);
        let leaf = leaf_under(&w.mid, &w.mid_key, &key, 90 + i as u64, opts);
        let (pdf, entry) = sign(&key, &[&leaf, &w.mid], NOW);
        let v = check(&pdf, &entry, &[&w.root]);
        assert_eq!((v.status, v.trust), (Status::Valid, want), "{label}: {v:?}");
        if want == Trust::Untrusted {
            assert!(has_note(&v, "not-for-signing"), "{label}");
            assert!(
                plain_sentence(&v)
                    .starts_with("The signer's certificate is not meant for signing documents")
            );
        }
    }
}

#[test]
fn the_content_type_must_be_data() {
    use tandem_sign::asn1::set_of;
    let w = world();
    let key = ec_key(100, false);
    let leaf = leaf_under(&w.mid, &w.mid_key, &key, 100, Opts::default());
    let (good_pdf, entry) = sign(&key, &[&leaf, &w.mid], NOW);
    assert_eq!(check(&good_pdf, &entry, &[&w.root]).status, Status::Valid);
    // The same content signed with a content-type attribute of id-signedData.
    let b = entry["byteRange"][1].as_u64().unwrap() as usize;
    let c = entry["byteRange"][2].as_u64().unwrap() as usize;
    let content = [&good_pdf[..b], &good_pdf[c..]].concat();
    let attr = |id: &const_oid::ObjectIdentifier, v: Vec<u8>| seq(&[&oid(id), &set_of(&[v])]);
    let attrs = set_of(&[
        attr(&oids::CONTENT_TYPE, oid(&oids::SIGNED_DATA)),
        attr(
            &oids::MESSAGE_DIGEST,
            tlv(0x04, &key.digest().hash(&content)),
        ),
    ]);
    let sig = key
        .sign_hash(&key.digest().hash(&attrs), &mut rng(1))
        .unwrap();
    let mut implicit = attrs.clone();
    implicit[0] = 0xa0;
    let signer_info = seq(&[
        &uint(1),
        &seq(&[leaf.issuer_bytes(), leaf.serial_bytes()]),
        &algorithm(&oids::SHA256, false),
        &implicit,
        &key.signature_algorithm(),
        &tlv(0x04, &sig),
    ]);
    let signed_data = seq(&[
        &uint(1),
        &set_of(&[algorithm(&oids::SHA256, false)]),
        &seq(&[&oid(&oids::DATA)]),
        &tlv(0xa0, &[leaf.der.clone(), w.mid.der.clone()].concat()),
        &set_of(&[signer_info]),
    ]);
    let cms = seq(&[&oid(&oids::SIGNED_DATA), &tlv(0xa0, &signed_data)]);
    let hex: String = cms.iter().map(|x| format!("{x:02X}")).collect();
    let mut bad = good_pdf.clone();
    let hole = c - b - 2;
    let padded = format!("{hex}{}", "0".repeat(hole - hex.len()));
    bad[b + 1..c - 1].copy_from_slice(padded.as_bytes());
    let mut e = entry.clone();
    e["contents"] = json!(padded);
    let v = check(&bad, &e, &[&w.root]);
    assert_eq!(
        (v.status, v.reason.as_deref()),
        (Status::Invalid, Some("content-type")),
        "{v:?}"
    );
}

#[test]
fn a_sha1_rsa_pss_link_is_outdated_too() {
    let (root_key, root) = root_rsa(110, 2048, 110, 110);
    let mid_key = ec_key(111, false);
    let mid = make_with(
        Spec {
            cn: "PSS Intermediate",
            serial: 111,
            key: &mid_key,
            issuer: Some((&root, &root_key)),
            ca: true,
            usage: 0x06,
            from: NOW - DAY,
            to: NOW + 30 * DAY,
            skid: 111,
        },
        Opts {
            pss_sha1: true,
            ..Opts::default()
        },
    );
    assert!(mid.signed_with_sha1());
    assert!(
        mid.signature_verifies_under(&root.public_key().unwrap()),
        "the PSS link itself verifies"
    );
    let key = ec_key(112, false);
    let leaf = leaf_under(&mid, &mid_key, &key, 112, Opts::default());
    let (pdf, entry) = sign(&key, &[&leaf, &mid], NOW);
    let v = check(&pdf, &entry, &[&root]);
    assert_eq!((v.status, v.trust), (Status::Valid, Trust::Untrusted));
    assert!(has_note(&v, "outdated-security"));
}

#[test]
fn trusted_certificates_are_taken_as_they_are_but_ca_rules_under_them_are_not_checked() {
    let odd = ext(
        &const_oid::ObjectIdentifier::new_unwrap("1.2.3.4.5"),
        true,
        tlv(0x05, &[]),
    );
    let name_constraints = ext(
        &const_oid::ObjectIdentifier::new_unwrap("2.5.29.30"),
        true,
        seq(&[]),
    );
    // A trusted root with an unknown critical extension, and one with name constraints: both still anchor.
    for (i, extra) in [odd.clone(), name_constraints.clone()]
        .into_iter()
        .enumerate()
    {
        let root_key = ec_key(120 + i as u8, true);
        let root = make_with(
            Spec {
                cn: "Odd Root",
                serial: 120 + i as u64,
                key: &root_key,
                issuer: None,
                ca: true,
                usage: 0x06,
                from: NOW - DAY,
                to: NOW + 30 * DAY,
                skid: 120 + i as u8,
            },
            Opts {
                extra: vec![extra],
                ..Opts::default()
            },
        );
        let key = ec_key(125 + i as u8, false);
        let leaf = leaf_under(&root, &root_key, &key, 125 + i as u64, Opts::default());
        let (pdf, entry) = sign(&key, &[&leaf], NOW);
        assert_eq!(
            check(&pdf, &entry, &[&root]).trust,
            Trust::Trusted,
            "root {i}"
        );
    }
    // An intermediate with name constraints: a rule Tandem cannot check yet.
    let w = world();
    let mid_key = ec_key(130, false);
    let mid = make_with(
        Spec {
            cn: "Constrained CA",
            serial: 130,
            key: &mid_key,
            issuer: Some((&w.root, &w.root_key)),
            ca: true,
            usage: 0x06,
            from: NOW - DAY,
            to: NOW + 30 * DAY,
            skid: 130,
        },
        Opts {
            extra: vec![name_constraints],
            ..Opts::default()
        },
    );
    let key = ec_key(131, false);
    let leaf = leaf_under(&mid, &mid_key, &key, 131, Opts::default());
    let (pdf, entry) = sign(&key, &[&leaf, &mid], NOW);
    let v = check(&pdf, &entry, &[&w.root]);
    assert_eq!((v.status, v.trust), (Status::Valid, Trust::Untrusted));
    assert!(has_note(&v, "unsupported-ca-rule"));
    assert!(
        plain_sentence(&v).starts_with(
            "The issuer of the signer's certificate uses rules Tandem can't check yet"
        )
    );
}

#[test]
fn a_later_update_cannot_change_the_signing_date() {
    // Review p2b M1: the verifier reads /M from the signed dictionary, never from a later version.
    let w = world();
    let key = ec_key(140, false);
    let leaf = make(Spec {
        cn: "Jane Doe",
        serial: 140,
        key: &key,
        issuer: Some((&w.mid, &w.mid_key)),
        ca: false,
        usage: 0xc0,
        from: NOW - 2 * DAY,
        to: NOW + 2 * DAY,
        skid: 140,
    });
    let signed_at = NOW;
    let (pdf, entry) = sign(&key, &[&leaf, &w.mid], signed_at);
    let mut later = pdf.clone();
    later.extend_from_slice(b"1 0 obj\n<</Type/Sig/SubFilter/ETSI.CAdES.detached/M(D:20000101000000Z)/Contents <00>>>\nendobj\n%%EOF\n");
    for forged in ["D:20000101000000Z", "D:20990101000000Z"] {
        let mut e = entry.clone();
        e["m"] = json!(forged);
        let v = check(&later, &e, &[&w.root]);
        assert_eq!(
            (v.status, v.trust, v.coverage),
            (Status::Valid, Trust::Trusted, Coverage::Partial),
            "{forged}"
        );
        assert_eq!(v.signing_time_ms, Some(signed_at / 1000 * 1000), "{forged}");
        assert!(has_note(&v, "time-changed"));
        assert!(plain_sentence(&v).contains("A later change edited the signature's date"));
    }
    // Without the signed /M the time is missing, whatever the entry says.
    let (mut bare, offsets) = {
        let contents_len = cms::MAX_SIG_CONTENTS_BYTES * 2;
        let head =
            b"%PDF-1.7\n1 0 obj\n<</Type/Sig/SubFilter/ETSI.CAdES.detached/Contents <".to_vec();
        let contents_at = head.len() as u64;
        let mut pdf = head;
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
    };
    cms::sign_pdf(
        &mut bare,
        offsets,
        &key,
        &[leaf.der.clone(), w.mid.der.clone()],
        &mut rng(9),
    )
    .unwrap();
    let b = offsets.contents_at - 1;
    let c = offsets.contents_at + offsets.contents_len + 1;
    let hex = String::from_utf8(bare[b as usize + 1..c as usize - 1].to_vec()).unwrap();
    let e = json!({"field": "Sig1", "page": 0, "empty": false, "subFilter": "ETSI.CAdES.detached", "m": "D:20261018093000Z",
        "byteRange": [0, b, c, bare.len() as u64 - c], "contents": hex});
    let v = check(&bare, &e, &[&w.root]);
    assert_eq!(v.status, Status::Valid);
    assert!(has_note(&v, "no-signing-time"));
    assert!(has_note(&v, "time-changed"));
}

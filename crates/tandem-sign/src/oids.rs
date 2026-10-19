// SPDX-License-Identifier: MIT
//! The object identifiers this crate reads and writes.

use const_oid::ObjectIdentifier as Oid;

pub const DATA: Oid = Oid::new_unwrap("1.2.840.113549.1.7.1");
pub const SIGNED_DATA: Oid = Oid::new_unwrap("1.2.840.113549.1.7.2");
pub const ENCRYPTED_DATA: Oid = Oid::new_unwrap("1.2.840.113549.1.7.6");
pub const CONTENT_TYPE: Oid = Oid::new_unwrap("1.2.840.113549.1.9.3");
pub const MESSAGE_DIGEST: Oid = Oid::new_unwrap("1.2.840.113549.1.9.4");
pub const SIGNING_TIME: Oid = Oid::new_unwrap("1.2.840.113549.1.9.5");
/// ESS signing-certificate (RFC 2634) and signing-certificate-v2 (RFC 5035).
pub const SIGNING_CERTIFICATE: Oid = Oid::new_unwrap("1.2.840.113549.1.9.16.2.12");
pub const SIGNING_CERTIFICATE_V2: Oid = Oid::new_unwrap("1.2.840.113549.1.9.16.2.47");

pub const SHA1: Oid = Oid::new_unwrap("1.3.14.3.2.26");
pub const SHA256: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.1");
pub const SHA384: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.2");
pub const SHA512: Oid = Oid::new_unwrap("2.16.840.1.101.3.4.2.3");

pub const RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.1");
pub const RSA_PSS: Oid = Oid::new_unwrap("1.2.840.113549.1.1.10");
pub const MGF1: Oid = Oid::new_unwrap("1.2.840.113549.1.1.8");
pub const SHA1_RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.5");
pub const SHA256_RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.11");
pub const SHA384_RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.12");
pub const SHA512_RSA: Oid = Oid::new_unwrap("1.2.840.113549.1.1.13");
pub const EC_PUBLIC_KEY: Oid = Oid::new_unwrap("1.2.840.10045.2.1");
pub const ECDSA_SHA1: Oid = Oid::new_unwrap("1.2.840.10045.4.1");
pub const ECDSA_SHA256: Oid = Oid::new_unwrap("1.2.840.10045.4.3.2");
pub const ECDSA_SHA384: Oid = Oid::new_unwrap("1.2.840.10045.4.3.3");
pub const ECDSA_SHA512: Oid = Oid::new_unwrap("1.2.840.10045.4.3.4");
pub const P256: Oid = Oid::new_unwrap("1.2.840.10045.3.1.7");
pub const P384: Oid = Oid::new_unwrap("1.3.132.0.34");
pub const P521: Oid = Oid::new_unwrap("1.3.132.0.35");

pub const PBES2: Oid = Oid::new_unwrap("1.2.840.113549.1.5.13");
pub const PBE_SHA_3DES: Oid = Oid::new_unwrap("1.2.840.113549.1.12.1.3");
pub const PBE_SHA_RC2_40: Oid = Oid::new_unwrap("1.2.840.113549.1.12.1.6");
pub const PBE_SHA_RC2_128: Oid = Oid::new_unwrap("1.2.840.113549.1.12.1.5");
pub const KEY_BAG: Oid = Oid::new_unwrap("1.2.840.113549.1.12.10.1.1");
pub const SHROUDED_KEY_BAG: Oid = Oid::new_unwrap("1.2.840.113549.1.12.10.1.2");
pub const CERT_BAG: Oid = Oid::new_unwrap("1.2.840.113549.1.12.10.1.3");
pub const X509_CERT: Oid = Oid::new_unwrap("1.2.840.113549.1.9.22.1");

pub const COMMON_NAME: Oid = Oid::new_unwrap("2.5.4.3");
pub const BASIC_CONSTRAINTS: Oid = Oid::new_unwrap("2.5.29.19");
pub const KEY_USAGE: Oid = Oid::new_unwrap("2.5.29.15");
pub const SUBJECT_KEY_ID: Oid = Oid::new_unwrap("2.5.29.14");
pub const AUTHORITY_KEY_ID: Oid = Oid::new_unwrap("2.5.29.35");
pub const EXT_KEY_USAGE: Oid = Oid::new_unwrap("2.5.29.37");
pub const SUBJECT_ALT_NAME: Oid = Oid::new_unwrap("2.5.29.17");
pub const ISSUER_ALT_NAME: Oid = Oid::new_unwrap("2.5.29.18");
pub const CERTIFICATE_POLICIES: Oid = Oid::new_unwrap("2.5.29.32");

/// Extended key usages a document-signing leaf may carry (any one is enough).
pub const EKU_ANY: Oid = Oid::new_unwrap("2.5.29.37.0");
pub const EKU_DOCUMENT_SIGNING: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.3.36");
pub const EKU_EMAIL_PROTECTION: Oid = Oid::new_unwrap("1.3.6.1.5.5.7.3.4");
pub const EKU_ADOBE_AUTHENTIC_DOCUMENTS: Oid = Oid::new_unwrap("1.2.840.113583.1.1.5");
pub const EKU_MS_DOCUMENT_SIGNING: Oid = Oid::new_unwrap("1.3.6.1.4.1.311.10.3.12");

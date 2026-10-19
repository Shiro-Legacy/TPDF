// SPDX-License-Identifier: MIT
//! Digital signing and signature verification for Tandem PDF.
//!
//! - [`identity`]: read a signing identity (private key and certificates) from
//!   a PKCS#12 file or PEM, and check that it can sign now.
//! - [`cms`]: build the detached CMS SignedData a PDF signature holds, and
//!   patch it into a prepared PDF.
//! - [`verify`]: check each signature of a PDF and report what it proves.
//! - [`report`]: say that report in plain English.
//!
//! The crate does no I/O and reads no clock or entropy source of its own: the
//! host passes the time and random bytes in, so the same code runs natively
//! and as WASM in Electron's main process.
#![forbid(unsafe_code)]

pub mod asn1;
pub mod cert;
pub mod cms;
pub mod digest;
pub mod drbg;
pub mod identity;
pub mod keys;
pub mod oids;
pub mod pem;
pub mod pkcs12;
pub mod report;
pub mod sigdict;
pub mod source;
pub mod time;
pub mod trust;
pub mod verify;
#[cfg(feature = "wasm")]
pub mod wasm;

pub use identity::{Identity, SignError};
pub use keys::SigningKey;

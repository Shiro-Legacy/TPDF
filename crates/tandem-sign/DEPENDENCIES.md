# tandem-sign dependencies

`tandem-sign` is the Rust port of `src/sign/*` (docs/rust-and-ux-plan.md §1.5,
Phase 2). It is MIT, forbids unsafe code in its own source, builds on Rust 1.92,
and does no I/O: the host passes the time and 48 bytes of entropy per
signature. All cryptography comes from RustCrypto. Every dependency is pinned
with `=` in `Cargo.toml`; `Cargo.lock` pins the rest with checksums. Licences
were read from each package's own `Cargo.toml` (`cargo tree --format '{l}'`)
and its licence files, which `licenses.sh` copies to `licenses/`. `NOTICE`
lists every package compiled into the WASM module.

## Direct dependencies (October 18, 2026)

The 0.7/0.10/0.13 RustCrypto generation is used throughout: it is the newest
one in which every piece (above all `rsa` 0.9 and `cms` 0.2) has a stable
release. The 0.8/0.11/0.14 generation still has `rsa` 0.10 and `pkcs12` 0.2
only as release candidates. The engine already pins `aes 0.8.4`, `md-5 0.10.6`
and `sha2 0.10.9` from the same generation.

| Crate | Version | Licence | Used for |
| --- | --- | --- | --- |
| aes | 0.8.4 | MIT OR Apache-2.0 | legacy encrypted PEM keys (`DEK-Info: AES-*-CBC`) |
| base64ct | 1.8.3 | Apache-2.0 OR MIT | PEM bodies (constant-time decoding) |
| cbc | 0.1.2 | MIT OR Apache-2.0 | CBC mode for 3DES, RC2, DES and AES decryption |
| const-oid | 0.9.6 | Apache-2.0 OR MIT | object identifiers |
| der | 0.7.10 | Apache-2.0 OR MIT | typed decoding of PKCS#1, PKCS#8, SEC1 and SPKI (through the crates below) |
| des | 0.8.1 | MIT OR Apache-2.0 | 3DES (PKCS#12 PBE, PBES2, legacy PEM) and DES (legacy PEM) |
| ecdsa | 0.16.9 | Apache-2.0 OR MIT | ECDSA signing (RFC 6979 nonces) and verification, DER signatures |
| hmac | 0.12.1 | MIT OR Apache-2.0 | PKCS#12 MAC, HMAC_DRBG |
| md-5 | 0.10.6 | MIT OR Apache-2.0 | OpenSSL's EVP_BytesToKey for legacy encrypted PEM keys only |
| p256, p384, p521 | 0.13.2, 0.13.1, 0.13.3 | Apache-2.0 OR MIT | the NIST curves (P-256 and P-384 sign; all three verify) |
| pkcs1 | 0.7.5 | Apache-2.0 OR MIT | RSA key structures |
| pkcs12 | 0.1.0 | Apache-2.0 OR MIT | the RFC 7292 key derivation (feature `kdf`) |
| pkcs5 | 0.7.1 | Apache-2.0 OR MIT | PBES2 with PBKDF2 (HMAC-SHA-1/-2) and AES or 3DES (features `pbes2`, `3des`, `sha1-insecure`) |
| pkcs8 | 0.10.2 | Apache-2.0 OR MIT | PrivateKeyInfo |
| rand_core | 0.6.4 | MIT OR Apache-2.0 | the RNG traits the DRBG implements (no `getrandom`) |
| rc2 | 0.8.1 | MIT OR Apache-2.0 | RC2-40 and RC2-128 (`openssl pkcs12 -legacy`, old Windows exports) |
| rsa | 0.9.10 | MIT OR Apache-2.0 | RSA PKCS#1 v1.5 signing (blinded) and v1.5/PSS verification |
| sec1 | 0.7.3 | Apache-2.0 OR MIT | EC private keys |
| serde_json | 1.0.151 | MIT OR Apache-2.0 | the JSON the WASM interface exchanges (same version as the engine) |
| sha1, sha2 | 0.10.6, 0.10.9 | MIT OR Apache-2.0 | SHA-1 (reading old files and signatures) and SHA-256/384/512 |
| signature | 2.2.0 | Apache-2.0 OR MIT | prehash signer and verifier traits |
| spki | 0.7.3 | Apache-2.0 OR MIT | SubjectPublicKeyInfo |
| wasm-bindgen | 0.2.129 | MIT OR Apache-2.0 | the WASM interface, feature `wasm` only (same version as the engine bridge and the CLI at `$HOME/.local/tpdf-wasm-tools`) |
| zeroize | 1.9.0 | Apache-2.0 OR MIT | the DRBG state is zeroed on drop |

Transitive packages compiled into the module: base16ct, block-buffer,
block-padding, cipher, cms, crypto-bigint, crypto-common, digest,
elliptic-curve, ff, flagset (Apache-2.0), generic-array (MIT), group, inout,
itoa, lazy_static, libm (MIT), memchr (Unlicense OR MIT), num-bigint-dig,
num-integer, num-iter, num-traits, once_cell, pbkdf2, pem-rfc7468,
ppv-lite86, primeorder, rand, rand_chacha, rfc6979, salsa20, scrypt,
serde_core, smallvec, spin (MIT), subtle (BSD-3-Clause), typenum,
unicode-ident ((MIT OR Apache-2.0) AND Unicode-3.0, as in the engine bridge),
wasm-bindgen-shared, x509-cert, zerocopy (BSD-2-Clause OR Apache-2.0 OR MIT)
and zmij (MIT); the rest are MIT OR Apache-2.0. `cms` and `x509-cert` come in
with `pkcs12` (its PFX types); `scrypt` and `salsa20` with `pkcs5`'s `pbes2`
feature; `rand` and `rand_chacha` with `num-bigint-dig`. None of them pulls
`getrandom`, so the wasm32 build needs no JavaScript entropy shim.

Unsafe code: as with the engine, the crate's own `unsafe_code = "forbid"`
does not reach its dependencies. These contain internal unsafe code (an
`unsafe` keyword in the packaged source): aes, base16ct, base64ct,
block-buffer, block-padding, cipher, crypto-bigint, flagset, generic-array,
inout, itoa, lazy_static, libm, md-5, memchr, num-bigint-dig, num-traits,
once_cell, pem-rfc7468, ppv-lite86, rand, rand_chacha, rand_core, serde_core,
serde_json, sha1, sha2, smallvec, spin, subtle, unicode-ident, wasm-bindgen,
zerocopy, zeroize and zmij. The RustCrypto format, key and signature crates
(der, spki, pkcs1, pkcs5, pkcs8, pkcs12, sec1, x509-cert, cms, ecdsa,
elliptic-curve, p256, p384, p521, primeorder, signature, hmac, cbc,
const-oid, digest) forbid unsafe code; rsa, des, rc2, pbkdf2, rfc6979 and
scrypt contain none.

## Accepted risk: RUSTSEC-2023-0071 (owner decision D3)

`rsa` 0.9 carries advisory RUSTSEC-2023-0071, the Marvin timing side channel
in RSA private-key operations. There is no fixed release of the 0.9 line.
Exploiting it needs an attacker who can time many private-key operations;
Tandem signs locally, with the user's own key, a few times per session, and
never decrypts with RSA. The owner accepted this as a documented risk (D3) in
preference to `aws-lc-rs`, which is constant-time but not pure Rust. The crate
still signs through `sign_with_rng`, so every signature is blinded with fresh
host entropy, which is the advisory's recommended mitigation for signing.
Revisit when `rsa` 0.10 (constant-time `crypto-bigint` backend) is stable.

## Choices made while porting

- **No `cms` builder and no `x509-cert` decoding of our own.** The CMS is
  written with a 60-line DER writer (`src/asn1.rs`), as `src/sign/cms.ts` was:
  certificates are embedded byte for byte (a re-encoding of a certificate that
  is not perfectly DER would break its issuer's signature), and the signed
  attributes are exactly the bytes signed. Certificates and CMS structures are
  read with a lenient TLV walker that never panics, so a certificate OpenSSL
  accepts is not refused over an encoding nit; typed decoding of keys goes
  through `pkcs1`, `pkcs8`, `sec1` and `spki`.
- **Chain building is our own** (`src/trust.rs`), as in `verify.ts`:
  `rustls-webpki` is for TLS and its EKU rules do not fit document signing.
- **Entropy and time from the host.** The crate has an HMAC_DRBG
  (SP 800-90A, SHA-256) seeded with 48 bytes per signature; Electron passes
  `crypto.getRandomValues` (the platform CSPRNG). ECDSA nonces are
  deterministic (RFC 6979) and need no randomness.
- **Bounds against hostile files:** a key derivation asking for more than
  2,000,000 iterations, or a certificate file asking for more than 10,000,000
  in all (MAC and every bag), is refused as `too-costly` before the work; RSA
  moduli above 16,384 bits are refused; PKCS#12 nesting stops at 16 levels;
  the PEM scanner is one pass. In Electron the file is read in a worker thread
  (`src/sign/sign-worker.ts`), which also keeps the private key, so main never
  waits on it.
- **PAdES B-B:** the engine writes SubFilter `ETSI.CAdES.detached` and the
  claimed time as `/M`; the CMS has no signing-time attribute. Verification
  still accepts `adbe.pkcs7.detached`.
- **cms 0.2.3's licence files** are not in its package; `licenses-upstream/`
  keeps the copies from the RustCrypto repository (`SOURCE.txt` there).

## Test oracles (not dependencies)

OpenSSL (`node:crypto`, the `openssl` command) and pyHanko
(`scripts/verify-sign.py`) check this crate's output and supply independent
inputs in `tests/sign-differential.test.mjs`, `tests/p5-sign-*.test.mjs` and
`tests/p5-verify-*.test.mjs`. They are test-only, like MuPDF for the engine.

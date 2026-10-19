//! The user's trusted certificates (port of `src/sign/trust.ts`): a PEM file
//! in the user-data folder, or memory only in a smoke or drive run. Writes go
//! through a temp file and a rename.

use super::util::{base64_decode_lenient, base64_encode, write_store};
use std::path::PathBuf;
use tandem_sign::cert::Cert;

/// A certificate in the trusted list; `raw` is its DER.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedCert {
    pub raw: Vec<u8>,
    pub cn: String,
    pub fingerprint256: String,
    /// ISO time (`2027-01-01T00:00:00.000Z`).
    pub not_after: String,
}

const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
const END: &str = "-----END CERTIFICATE-----";

/// Certificates from PEM text; blocks that are not a readable certificate are skipped.
pub fn parse_blocks(text: &str) -> Vec<TrustedCert> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(BEGIN) {
        let body = &rest[start + BEGIN.len()..];
        let Some(end) = body.find(END) else { break };
        let raw = base64_decode_lenient(&body[..end]);
        rest = &body[end + END.len()..];
        let Some(cert) = (!raw.is_empty()).then(|| Cert::parse(&raw)).flatten() else {
            continue;
        };
        out.push(TrustedCert {
            cn: cert.subject_cn(),
            fingerprint256: cert.fingerprint256(),
            not_after: tandem_sign::time::iso(cert.not_after_ms),
            raw,
        });
    }
    out
}

fn pem(cert: &TrustedCert) -> String {
    let b64 = base64_encode(&cert.raw);
    let mut lines = vec![BEGIN.to_string()];
    lines.extend(
        b64.as_bytes()
            .chunks(64)
            .map(|c| String::from_utf8_lossy(c).into_owned()),
    );
    lines.push(END.to_string());
    lines.join("\n")
}

#[derive(Debug, Default)]
pub struct TrustStore {
    path: Option<PathBuf>,
    journal: Option<PathBuf>,
    memory: Vec<TrustedCert>,
}

impl TrustStore {
    /// `path: None` keeps the store in memory (smoke/drive).
    pub fn new(path: Option<PathBuf>, journal: Option<PathBuf>) -> Self {
        TrustStore {
            path,
            journal,
            memory: Vec::new(),
        }
    }

    /// Every trusted certificate (read from the file each time, as trust.ts did).
    pub fn certs(&self) -> Vec<TrustedCert> {
        match &self.path {
            None => self.memory.clone(),
            Some(path) => std::fs::read(path)
                .map(|b| parse_blocks(&String::from_utf8_lossy(&b)))
                .unwrap_or_default(),
        }
    }

    fn write(&mut self, certs: Vec<TrustedCert>) -> Result<(), crate::safe_write::SaveFail> {
        let Some(path) = &self.path else {
            self.memory = certs;
            return Ok(());
        };
        let mut body = certs.iter().map(pem).collect::<Vec<_>>().join("\n");
        if !certs.is_empty() {
            body.push('\n');
        }
        write_store(path, body.as_bytes(), false, self.journal.as_deref())
    }

    /// Adds the certificates in `pem_text` that are not listed yet: `(added, total)`.
    pub fn add(&mut self, pem_text: &[u8]) -> Result<(usize, usize), crate::safe_write::SaveFail> {
        let mut current = self.certs();
        let mut added = 0;
        for cert in parse_blocks(&String::from_utf8_lossy(pem_text)) {
            if current
                .iter()
                .any(|c| c.fingerprint256 == cert.fingerprint256)
            {
                continue;
            }
            current.push(cert);
            added += 1;
        }
        let total = current.len();
        self.write(current)?;
        Ok((added, total))
    }

    pub fn clear(&mut self) -> Result<(), crate::safe_write::SaveFail> {
        self.write(Vec::new())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// `fixtures/p0b/.ca/<name>` from the repository.
    pub(crate) fn fixture_pem(name: &str) -> String {
        let path = format!(
            "{}/../../fixtures/p0b/.ca/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    #[test]
    fn round_trip_through_a_file() {
        let text = format!("{}\n{}", fixture_pem("ca.pem"), fixture_pem("other-ca.pem"));
        let dir =
            super::super::util::mkdtemp(&super::super::util::temp_dir().join("tandem-trust-"))
                .unwrap();
        let file = dir.join("trusted-certs.pem");
        let mut store = TrustStore::new(Some(file.clone()), None);
        assert_eq!(store.add(text.as_bytes()).unwrap(), (2, 2));
        // The same certificates again add nothing; one more adds one.
        assert_eq!(store.add(fixture_pem("ca.pem").as_bytes()).unwrap(), (0, 2));
        assert_eq!(
            store.add(fixture_pem("leaf.pem").as_bytes()).unwrap(),
            (1, 3)
        );
        let again = TrustStore::new(Some(file.clone()), None);
        let certs = again.certs();
        assert_eq!(certs.len(), 3);
        assert!(certs[0].not_after.ends_with('Z'));
        assert!(!certs[0].cn.is_empty());
        // Colon-separated hex, as Node's `X509Certificate#fingerprint256`.
        assert_eq!(certs[0].fingerprint256.len(), 95);
        let written = std::fs::read_to_string(&file).unwrap();
        assert!(written.starts_with(BEGIN) && written.ends_with("-----\n"));
        assert!(
            written
                .lines()
                .all(|l| l.len() <= 64 || l.starts_with("-----"))
        );
        assert_eq!(parse_blocks(&written), certs);
        store.clear().unwrap();
        assert!(again.certs().is_empty());
        assert_eq!(std::fs::read(&file).unwrap(), b"");
        // Memory only.
        let mut mem = TrustStore::new(None, None);
        assert_eq!(mem.add(text.as_bytes()).unwrap(), (2, 2));
        assert_eq!(mem.certs().len(), 2);
        mem.clear().unwrap();
        assert!(mem.certs().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn junk_is_skipped() {
        assert!(parse_blocks("nothing here").is_empty());
        assert!(parse_blocks(&format!("{BEGIN}\nAAAA\n{END}\n")).is_empty());
        assert!(parse_blocks(&format!("{BEGIN}\nAAAA\n")).is_empty());
    }
}

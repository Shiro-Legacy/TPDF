// SPDX-License-Identifier: MIT
//! Random access to the PDF bytes being signed or verified.
//!
//! Natively the bytes are a slice. In Electron the file stays in the host's
//! memory and the WASM module reads the windows and hash chunks it needs, so
//! a 120 MB PDF never has to be copied into the module whole.

/// Bytes the signer and verifier read by offset.
pub trait Source {
    /// The total length.
    fn len(&self) -> u64;
    /// Whether the source is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// `len` bytes at `offset`, or `None` when the range is out of bounds.
    fn read_at(&self, offset: u64, len: usize) -> Option<Vec<u8>>;
}

impl Source for &[u8] {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }
    fn read_at(&self, offset: u64, len: usize) -> Option<Vec<u8>> {
        let start = usize::try_from(offset).ok()?;
        self.get(start..start.checked_add(len)?).map(<[u8]>::to_vec)
    }
}

impl Source for Vec<u8> {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }
    fn read_at(&self, offset: u64, len: usize) -> Option<Vec<u8>> {
        self.as_slice().read_at(offset, len)
    }
}

/// Hashing reads this much at a time.
pub const CHUNK: u64 = 1 << 20;

/// Hashes `ranges` (half-open, in order) of `src`, with `overlay` (offset,
/// bytes) written over whatever it covers, as if the file had been patched.
pub fn hash_ranges(
    src: &dyn Source,
    alg: crate::digest::DigestAlg,
    ranges: &[(u64, u64)],
    overlay: Option<(u64, &[u8])>,
) -> Option<Vec<u8>> {
    let mut hasher = crate::digest::Hasher::new(alg);
    for &(start, end) in ranges {
        if end < start || end > src.len() {
            return None;
        }
        let mut at = start;
        while at < end {
            let n = (end - at).min(CHUNK);
            let mut chunk = src.read_at(at, usize::try_from(n).ok()?)?;
            if let Some((o_at, bytes)) = overlay {
                let o_end = o_at + bytes.len() as u64;
                let lo = o_at.max(at);
                let hi = o_end.min(at + n);
                if lo < hi {
                    let dst = usize::try_from(lo - at).ok()?;
                    let srci = usize::try_from(lo - o_at).ok()?;
                    let count = usize::try_from(hi - lo).ok()?;
                    chunk[dst..dst + count].copy_from_slice(&bytes[srci..srci + count]);
                }
            }
            hasher.update(&chunk);
            at += n;
        }
    }
    Some(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::DigestAlg;

    #[test]
    fn overlay_matches_a_patched_copy() {
        let src: Vec<u8> = (0..=255u8).cycle().take(3000).collect();
        let mut patched = src.clone();
        patched[100..110].copy_from_slice(b"0123456789");
        let want = DigestAlg::Sha256.hash(&[&patched[..500], &patched[900..]].concat());
        let got = hash_ranges(
            &src,
            DigestAlg::Sha256,
            &[(0, 500), (900, 3000)],
            Some((100, b"0123456789")),
        );
        assert_eq!(got, Some(want));
        assert_eq!(
            hash_ranges(&src, DigestAlg::Sha256, &[(0, 3001)], None),
            None
        );
    }
}

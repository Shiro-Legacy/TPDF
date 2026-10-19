// SPDX-License-Identifier: MIT
//! HMAC_DRBG with SHA-256 (NIST SP 800-90A Rev. 1, 10.1.2), seeded from
//! entropy the host passes in, so the crate needs no OS randomness of its own
//! (in Electron the seed comes from the platform's Web Crypto). It feeds RSA
//! blinding; ECDSA nonces are deterministic (RFC 6979).

use hmac::{Hmac, Mac};
use rand_core::{CryptoRng, RngCore};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// The least seed accepted: 256 bits of entropy plus a 128-bit nonce.
pub const MIN_SEED: usize = 48;

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct HmacDrbg {
    k: [u8; 32],
    v: [u8; 32],
}

fn hmac(key: &[u8; 32], parts: &[&[u8]]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC takes any key length");
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

impl HmacDrbg {
    /// `None` when the seed is shorter than [`MIN_SEED`].
    pub fn new(seed: &[u8]) -> Option<Self> {
        if seed.len() < MIN_SEED {
            return None;
        }
        let mut drbg = HmacDrbg {
            k: [0; 32],
            v: [1; 32],
        };
        drbg.update(Some(seed));
        Some(drbg)
    }

    fn update(&mut self, data: Option<&[u8]>) {
        let data = data.unwrap_or(&[]);
        self.k = hmac(&self.k, &[&self.v, &[0x00], data]);
        self.v = hmac(&self.k, &[&self.v]);
        if !data.is_empty() {
            self.k = hmac(&self.k, &[&self.v, &[0x01], data]);
            self.v = hmac(&self.k, &[&self.v]);
        }
    }
}

impl RngCore for HmacDrbg {
    fn next_u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        self.fill_bytes(&mut b);
        u32::from_le_bytes(b)
    }
    fn next_u64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        self.fill_bytes(&mut b);
        u64::from_le_bytes(b)
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        for chunk in dest.chunks_mut(32) {
            self.v = hmac(&self.k, &[&self.v]);
            chunk.copy_from_slice(&self.v[..chunk.len()]);
        }
        self.update(None);
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

impl CryptoRng for HmacDrbg {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_per_seed_and_refuses_short_seeds() {
        assert!(HmacDrbg::new(&[0u8; 47]).is_none());
        let mut a = HmacDrbg::new(&[7u8; 48]).unwrap();
        let mut b = HmacDrbg::new(&[7u8; 48]).unwrap();
        let mut c = HmacDrbg::new(&[8u8; 48]).unwrap();
        let (mut x, mut y, mut z) = ([0u8; 70], [0u8; 70], [0u8; 70]);
        a.fill_bytes(&mut x);
        b.fill_bytes(&mut y);
        c.fill_bytes(&mut z);
        assert_eq!(x, y);
        assert_ne!(x, z);
        let mut again = [0u8; 70];
        a.fill_bytes(&mut again);
        assert_ne!(x, again);
    }
}

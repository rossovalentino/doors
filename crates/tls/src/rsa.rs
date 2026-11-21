//! RSA signature verification (RFC 8017): PKCS #1 v1.5 and PSS (MGF1,
//! salt length equal to the hash), over a small Montgomery bignum. Only
//! public operations exist here: nothing secret is computed.
use crate::der::{self, Reader};
use alloc::vec::Vec;
use sha2::Digest;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hash {
    /// Authenticode signatures from before SHA-2 (never TLS: the
    /// handshake's signature schemes have no SHA-1 entry here).
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}
impl Hash {
    pub fn size(self) -> usize {
        match self {
            Self::Sha1 => 20,
            Self::Sha256 => 32,
            Self::Sha384 => 48,
            Self::Sha512 => 64,
        }
    }
    pub fn digest(self, parts: &[&[u8]]) -> Vec<u8> {
        fn run<D: Digest>(parts: &[&[u8]]) -> Vec<u8> {
            let mut d = D::new();
            for p in parts {
                d.update(p);
            }
            d.finalize().to_vec()
        }
        match self {
            Self::Sha1 => run::<sha1::Sha1>(parts),
            Self::Sha256 => run::<sha2::Sha256>(parts),
            Self::Sha384 => run::<sha2::Sha384>(parts),
            Self::Sha512 => run::<sha2::Sha512>(parts),
        }
    }
    fn digest_info(self) -> &'static [u8] {
        match self {
            Self::Sha1 => &[
                0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2B, 0x0E, 0x03, 0x02, 0x1A, 0x05, 0x00, 0x04,
                0x14,
            ],
            Self::Sha256 => &[
                0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02,
                0x01, 0x05, 0x00, 0x04, 0x20,
            ],
            Self::Sha384 => &[
                0x30, 0x41, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02,
                0x02, 0x05, 0x00, 0x04, 0x30,
            ],
            Self::Sha512 => &[
                0x30, 0x51, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02,
                0x03, 0x05, 0x00, 0x04, 0x40,
            ],
        }
    }
}

/// Little-endian 32-bit limbs.
type Limbs = Vec<u32>;
fn from_be(b: &[u8], limbs: usize) -> Limbs {
    let mut v = alloc::vec![0u32; limbs];
    for (i, byte) in b.iter().rev().enumerate() {
        if i / 4 < limbs {
            v[i / 4] |= u32::from(*byte) << (8 * (i % 4));
        }
    }
    v
}
fn to_be(v: &[u32], len: usize) -> Vec<u8> {
    let mut out = alloc::vec![0u8; len];
    for i in 0..len {
        out[len - 1 - i] = (v.get(i / 4).copied().unwrap_or(0) >> (8 * (i % 4))) as u8;
    }
    out
}
fn geq(a: &[u32], b: &[u32]) -> bool {
    for i in (0..a.len()).rev() {
        if a[i] != b[i] {
            return a[i] > b[i];
        }
    }
    true
}
fn sub_in_place(a: &mut [u32], b: &[u32]) {
    let mut borrow = 0i64;
    for i in 0..a.len() {
        let d = i64::from(a[i]) - i64::from(b[i]) - borrow;
        a[i] = d as u32;
        borrow = i64::from(d < 0);
    }
}
struct Mont {
    n: Limbs,
    n0: u32,
    r2: Limbs,
}
impl Mont {
    fn new(n: Limbs) -> Self {
        let k = n.len();
        // -n^-1 mod 2^32 by Newton iteration.
        let mut inv: u32 = 1;
        for _ in 0..5 {
            inv = inv.wrapping_mul(2u32.wrapping_sub(n[0].wrapping_mul(inv)));
        }
        let n0 = inv.wrapping_neg();
        // R^2 mod n by doubling 1, 2 * 32 * k times.
        let mut r2 = alloc::vec![0u32; k + 1];
        r2[0] = 1;
        let mut nn = n.clone();
        nn.push(0);
        for _ in 0..64 * k {
            let mut carry = 0;
            for limb in r2.iter_mut() {
                let v = (*limb >> 31) & 1;
                *limb = (*limb << 1) | carry;
                carry = v;
            }
            if geq(&r2, &nn) {
                sub_in_place(&mut r2, &nn);
            }
        }
        r2.truncate(k);
        Self { n, n0, r2 }
    }
    fn mul(&self, a: &[u32], b: &[u32]) -> Limbs {
        let k = self.n.len();
        let mut t = alloc::vec![0u32; k + 2];
        for &bi in b.iter().take(k) {
            let mut carry = 0u64;
            for j in 0..k {
                let v = u64::from(t[j]) + u64::from(a[j]) * u64::from(bi) + carry;
                t[j] = v as u32;
                carry = v >> 32;
            }
            let v = u64::from(t[k]) + carry;
            t[k] = v as u32;
            t[k + 1] = (v >> 32) as u32;
            let m = t[0].wrapping_mul(self.n0);
            let mut carry = (u64::from(t[0]) + u64::from(m) * u64::from(self.n[0])) >> 32;
            for j in 1..k {
                let v = u64::from(t[j]) + u64::from(m) * u64::from(self.n[j]) + carry;
                t[j - 1] = v as u32;
                carry = v >> 32;
            }
            let v = u64::from(t[k]) + carry;
            t[k - 1] = v as u32;
            t[k] = t[k + 1] + (v >> 32) as u32;
        }
        let mut r: Limbs = t[..k + 1].to_vec();
        let mut nn = self.n.clone();
        nn.push(0);
        if geq(&r, &nn) {
            sub_in_place(&mut r, &nn);
        }
        r.truncate(k);
        r
    }
    fn pow(&self, base: &[u32], e: &[u8]) -> Limbs {
        let k = self.n.len();
        let mut one = alloc::vec![0u32; k];
        one[0] = 1;
        let x = self.mul(base, &self.r2);
        let mut acc = self.mul(&one, &self.r2);
        for byte in e {
            for bit in (0..8).rev() {
                acc = self.mul(&acc, &acc);
                if (byte >> bit) & 1 == 1 {
                    acc = self.mul(&acc, &x);
                }
            }
        }
        self.mul(&acc, &one)
    }
}
/// `sig^e mod n` as a big-endian byte string of the modulus length, from an
/// RSAPublicKey (SEQUENCE { n, e }).
fn public_op(key: &[u8], sig: &[u8]) -> Option<(Vec<u8>, usize)> {
    let mut r = Reader::new(Reader::new(key).expect(der::SEQUENCE)?.value);
    let n = r.expect(der::INTEGER)?.value;
    let e = r.expect(der::INTEGER)?.value;
    let n = match n.first() {
        Some(0) => &n[1..],
        _ => n,
    };
    // A zero or empty modulus, or one with a redundant leading zero, is
    // malformed (and would underflow the bit count).
    if n.first().is_none_or(|b| *b == 0) {
        return None;
    }
    // The public exponent as webpki accepts it: at most 33 bits, odd, at
    // least 3. A long exponent would make one verification take minutes.
    let e = match e.first() {
        Some(0) => &e[1..],
        _ => e,
    };
    if e.is_empty()
        || e.len() > 5
        || (e.len() == 5 && e[0] > 1)
        || e.last().is_none_or(|b| b & 1 == 0)
        || (e.len() == 1 && e[0] < 3)
    {
        return None;
    }
    let bits = n.len() * 8 - n[0].leading_zeros() as usize;
    if !(1024..=8192).contains(&bits) || n.last().is_none_or(|b| b & 1 == 0) || sig.len() != n.len()
    {
        return None;
    }
    let limbs = n.len().div_ceil(4);
    let m = Mont::new(from_be(n, limbs));
    let s = from_be(sig, limbs);
    if geq(&s, &m.n) {
        return None;
    }
    Some((to_be(&m.pow(&s, e), n.len()), bits))
}
pub fn verify_pkcs1(key: &[u8], hash: Hash, message: &[u8], sig: &[u8]) -> bool {
    let Some((em, _)) = public_op(key, sig) else {
        return false;
    };
    let info = hash.digest_info();
    let t_len = info.len() + hash.size();
    if em.len() < t_len + 11 {
        return false;
    }
    let ps = em.len() - t_len - 3;
    let mut expected = alloc::vec![0u8, 1];
    expected.extend(core::iter::repeat_n(0xFF, ps));
    expected.push(0);
    expected.extend_from_slice(info);
    expected.extend_from_slice(&hash.digest(&[message]));
    em == expected
}
fn mgf1(hash: Hash, seed: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + hash.size());
    let mut counter = 0u32;
    while out.len() < len {
        out.extend_from_slice(&hash.digest(&[seed, &counter.to_be_bytes()]));
        counter += 1;
    }
    out.truncate(len);
    out
}
pub fn verify_pss(key: &[u8], hash: Hash, message: &[u8], sig: &[u8]) -> bool {
    let Some((em_full, bits)) = public_op(key, sig) else {
        return false;
    };
    let em_bits = bits - 1;
    let em_len = em_bits.div_ceil(8);
    let em = &em_full[em_full.len() - em_len..];
    if em_full.len() > em_len && em_full[0] != 0 {
        return false;
    }
    let h_len = hash.size();
    if em_len < 2 * h_len + 2 || em[em_len - 1] != 0xBC {
        return false;
    }
    let (masked, rest) = em.split_at(em_len - h_len - 1);
    let h = &rest[..h_len];
    let top = 8 * em_len - em_bits;
    if top > 0 && masked[0] >> (8 - top) != 0 {
        return false;
    }
    let mask = mgf1(hash, h, masked.len());
    let mut db: Vec<u8> = masked.iter().zip(&mask).map(|(a, b)| a ^ b).collect();
    if top > 0 {
        db[0] &= 0xFF >> top;
    }
    let ps = db.len() - h_len - 1;
    if db[..ps].iter().any(|b| *b != 0) || db[ps] != 1 {
        return false;
    }
    let salt = &db[ps + 1..];
    // rsa_pss_rsae_*: the salt is as long as the hash (RFC 8446 4.2.3).
    if salt.len() != h_len {
        return false;
    }
    let m_hash = hash.digest(&[message]);
    hash.digest(&[&[0u8; 8], &m_hash, salt]) == h
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(n: &[u8], e: &[u8]) -> Vec<u8> {
        let int = |v: &[u8]| {
            let mut out = alloc::vec![0x02u8];
            if v.len() < 128 {
                out.push(v.len() as u8);
            } else {
                out.extend([0x82, (v.len() >> 8) as u8, v.len() as u8]);
            }
            out.extend_from_slice(v);
            out
        };
        let body = [int(n), int(e)].concat();
        let mut k = alloc::vec![0x30u8, 0x82, (body.len() >> 8) as u8, body.len() as u8];
        k.extend(body);
        k
    }
    #[test]
    fn hostile_keys_are_refused_without_panicking() {
        // Zero and empty moduli (a server-chosen "intermediate").
        assert!(public_op(&key(&[0], &[1, 0, 1]), &[]).is_none());
        assert!(public_op(&key(&[], &[1, 0, 1]), &[]).is_none());
        // A huge exponent would take minutes per verification.
        let mut n = alloc::vec![0xC3u8; 256];
        n[255] |= 1;
        assert!(public_op(&key(&n, &alloc::vec![0xFF; 1024]), &[1; 256]).is_none());
        // Even and tiny exponents.
        assert!(public_op(&key(&n, &[2]), &[1; 256]).is_none());
        assert!(public_op(&key(&n, &[1]), &[1; 256]).is_none());
        assert!(public_op(&key(&n, &[1, 0, 1]), &[1; 256]).is_some());
    }
    #[test]
    fn montgomery_matches_small_numbers() {
        // 4^13 mod 497 = 445 (n odd, single limb).
        let m = Mont::new(alloc::vec![497]);
        assert_eq!(m.pow(&[4], &[13]), alloc::vec![445]);
        // Two limbs: 3^65537 mod (2^61 - 1).
        let n = (1u64 << 61) - 1;
        let m = Mont::new(alloc::vec![n as u32, (n >> 32) as u32]);
        let got = m.pow(&[3, 0], &[1, 0, 1]);
        let mut want: u128 = 1;
        let mut base: u128 = 3;
        let mut e = 65537u32;
        while e > 0 {
            if e & 1 == 1 {
                want = want * base % u128::from(n);
            }
            base = base * base % u128::from(n);
            e >>= 1;
        }
        assert_eq!(u64::from(got[0]) | (u64::from(got[1]) << 32), want as u64);
    }
}

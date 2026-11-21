//! RSA keys for CryptoAPI's RSA_KEYX and RSA_SIGN: generation, the
//! PRIVATEKEYBLOB/PUBLICKEYBLOB layouts, and PKCS #1 v1.5 (and OAEP with
//! SHA-1) encryption and signatures, with CryptoAPI's little-endian byte
//! order. Arithmetic is crypto-bigint's constant-time Montgomery form; the
//! private operation uses the Chinese remainder theorem.
use alloc::vec::Vec;
use crypto_bigint::{
    Encoding, NonZero, U4096,
    modular::runtime_mod::{DynResidue, DynResidueParams},
};

/// The largest key Doors' providers make or import.
pub const MAX_BITS: usize = 4096;
type Big = U4096;
pub const PUBLIC_EXPONENT: u32 = 65537;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicKey {
    pub bits: usize,
    pub e: u32,
    pub n: Big,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivateKey {
    pub public: PublicKey,
    pub p: Big,
    pub q: Big,
    pub dp: Big,
    pub dq: Big,
    pub qinv: Big,
    pub d: Big,
}

fn modpow(base: &Big, exponent: &Big, exponent_bits: usize, modulus: &Big) -> Big {
    let params = DynResidueParams::new(modulus);
    DynResidue::new(base, params)
        .pow_bounded_exp(exponent, exponent_bits)
        .retrieve()
}
fn nonzero(v: &Big) -> Option<NonZero<Big>> {
    Option::from(NonZero::new(*v))
}
/// `bytes` (little-endian, at most 512) as a number.
fn from_le(bytes: &[u8]) -> Option<Big> {
    if bytes.len() > MAX_BITS / 8 {
        return None;
    }
    let mut b = [0u8; MAX_BITS / 8];
    b[..bytes.len()].copy_from_slice(bytes);
    Some(Big::from_le_slice(&b))
}
/// The low `len` bytes of `v`, little-endian.
fn to_le(v: &Big, len: usize) -> Vec<u8> {
    v.to_le_bytes()[..len].to_vec()
}

/// The odd primes below 2^12 (trial division before Miller-Rabin).
fn small_primes() -> Vec<u32> {
    let mut sieve = [true; 4096];
    let mut out = Vec::new();
    for i in 3..4096usize {
        if sieve[i] {
            if i % 2 == 1 {
                out.push(i as u32);
            }
            let mut j = i * i;
            while j < 4096 {
                sieve[j] = false;
                j += i;
            }
        }
    }
    out
}
/// Miller-Rabin with `rounds` random bases (`random` fills a buffer).
fn probably_prime(n: &Big, bits: usize, rounds: usize, random: &mut dyn FnMut(&mut [u8])) -> bool {
    let one = Big::ONE;
    let n1 = n.wrapping_sub(&one);
    let mut s = 0;
    while !n1.bit_vartime(s) {
        s += 1;
    }
    let d = n1.shr_vartime(s);
    let params = DynResidueParams::new(n);
    let one_m = DynResidue::one(params);
    let minus_one = DynResidue::new(&n1, params);
    for _ in 0..rounds {
        // A base in [2, n - 2].
        let mut buffer = [0u8; MAX_BITS / 8];
        random(&mut buffer[..bits / 8]);
        let a =
            Big::from_le_slice(&buffer).rem(&nonzero(&n.wrapping_sub(&Big::from_u32(3))).unwrap());
        let a = a.wrapping_add(&Big::from_u32(2));
        let mut x = DynResidue::new(&a, params).pow_bounded_exp(&d, bits);
        if x == one_m || x == minus_one {
            continue;
        }
        let mut composite = true;
        for _ in 1..s {
            x = x.square();
            if x == minus_one {
                composite = false;
                break;
            }
        }
        if composite {
            return false;
        }
    }
    true
}
/// A random prime of exactly `bits` bits (top two bits set) with
/// p mod e != 1.
fn prime(bits: usize, e: u32, random: &mut dyn FnMut(&mut [u8])) -> Big {
    let primes = small_primes();
    loop {
        let mut buffer = [0u8; MAX_BITS / 8];
        random(&mut buffer[..bits / 8]);
        buffer[0] |= 1;
        buffer[bits / 8 - 1] |= 0xC0;
        let candidate = Big::from_le_slice(&buffer);
        let divisible = primes.iter().any(|&p| {
            let (_, r) =
                candidate.div_rem_limb(NonZero::new(crypto_bigint::Limb::from(p)).unwrap());
            r.0 == 0
        });
        if divisible {
            continue;
        }
        let (_, r) = candidate.div_rem_limb(NonZero::new(crypto_bigint::Limb::from(e)).unwrap());
        if r.0 == 1 {
            continue;
        }
        let rounds = if bits >= 1024 {
            5
        } else if bits >= 512 {
            7
        } else {
            12
        };
        if probably_prime(&candidate, bits, rounds, random) {
            return candidate;
        }
    }
}
impl PrivateKey {
    /// A new key of `bits` (a multiple of 16, 384 to `MAX_BITS`).
    pub fn generate(bits: usize, random: &mut dyn FnMut(&mut [u8])) -> Option<Self> {
        if !(384..=MAX_BITS).contains(&bits) || !bits.is_multiple_of(16) {
            return None;
        }
        let e = PUBLIC_EXPONENT;
        loop {
            let p = prime(bits / 2, e, random);
            let q = prime(bits / 2, e, random);
            if p == q {
                continue;
            }
            // prime1 the larger, as CryptoAPI's blobs keep them.
            let (p, q) = if p > q { (p, q) } else { (q, p) };
            let n = p.wrapping_mul(&q);
            if n.bits_vartime() != bits {
                continue;
            }
            if let Some(key) = Self::from_primes(bits, e, p, q) {
                return Some(key);
            }
        }
    }
    /// The key of primes `p` and `q`.
    pub fn from_primes(bits: usize, e: u32, p: Big, q: Big) -> Option<Self> {
        let one = Big::ONE;
        let (p1, q1) = (p.wrapping_sub(&one), q.wrapping_sub(&one));
        let phi = p1.wrapping_mul(&q1);
        let (d, ok) = Big::from_u32(e).inv_mod(&phi);
        if !bool::from(ok) {
            return None;
        }
        let dp = d.rem(&nonzero(&p1)?);
        let dq = d.rem(&nonzero(&q1)?);
        let (qinv, ok) = q.rem(&nonzero(&p)?).inv_odd_mod(&p);
        if !bool::from(ok) {
            return None;
        }
        Some(Self {
            public: PublicKey {
                bits,
                e,
                n: p.wrapping_mul(&q),
            },
            p,
            q,
            dp,
            dq,
            qinv,
            d,
        })
    }
    /// c^d mod n through the primes.
    pub fn raw(&self, c: &Big) -> Big {
        let half = self.public.bits / 2;
        let (p, q) = (nonzero(&self.p).unwrap(), nonzero(&self.q).unwrap());
        let m1 = modpow(&c.rem(&p), &self.dp, half, &self.p);
        let m2 = modpow(&c.rem(&q), &self.dq, half, &self.q);
        // h = qinv (m1 - m2) mod p.
        let m2p = m2.rem(&p);
        let diff = m1.add_mod(&self.p.wrapping_sub(&m2p), &self.p);
        let params = DynResidueParams::new(&self.p);
        let h = (DynResidue::new(&diff, params) * DynResidue::new(&self.qinv, params)).retrieve();
        m2.wrapping_add(&h.wrapping_mul(&self.q))
    }
    /// PRIVATEKEYBLOB after its BLOBHEADER: RSAPUBKEY ("RSA2", bit length,
    /// public exponent), modulus, prime1, prime2, exponent1, exponent2,
    /// coefficient, private exponent, little-endian.
    pub fn blob(&self) -> Vec<u8> {
        let (n, h) = (self.public.bits / 8, self.public.bits / 16);
        let mut b = Vec::new();
        b.extend_from_slice(b"RSA2");
        b.extend_from_slice(&(self.public.bits as u32).to_le_bytes());
        b.extend_from_slice(&self.public.e.to_le_bytes());
        b.extend(to_le(&self.public.n, n));
        for v in [&self.p, &self.q, &self.dp, &self.dq, &self.qinv] {
            b.extend(to_le(v, h));
        }
        b.extend(to_le(&self.d, n));
        b
    }
    /// The key in a PRIVATEKEYBLOB's body (`blob`'s layout).
    pub fn from_blob(b: &[u8]) -> Option<Self> {
        if b.get(..4)? != b"RSA2" {
            return None;
        }
        let bits = u32::from_le_bytes(b.get(4..8)?.try_into().ok()?) as usize;
        let e = u32::from_le_bytes(b.get(8..12)?.try_into().ok()?);
        if !(384..=MAX_BITS).contains(&bits) || !bits.is_multiple_of(16) {
            return None;
        }
        let (n, h) = (bits / 8, bits / 16);
        let mut at = 12;
        let mut take = |len: usize| -> Option<Big> {
            let v = from_le(b.get(at..at + len)?)?;
            at += len;
            Some(v)
        };
        let modulus = take(n)?;
        let (p, q, dp, dq, qinv) = (take(h)?, take(h)?, take(h)?, take(h)?, take(h)?);
        let d = take(n)?;
        if p.bit_vartime(0) && q.bit_vartime(0) && p.wrapping_mul(&q) == modulus {
            Some(Self {
                public: PublicKey {
                    bits,
                    e,
                    n: modulus,
                },
                p,
                q,
                dp,
                dq,
                qinv,
                d,
            })
        } else {
            None
        }
    }
}
impl PublicKey {
    pub fn bytes(&self) -> usize {
        self.bits / 8
    }
    /// m^e mod n.
    pub fn raw(&self, m: &Big) -> Big {
        modpow(m, &Big::from_u32(self.e), 32, &self.n)
    }
    /// PUBLICKEYBLOB after its BLOBHEADER: "RSA1", bit length, exponent,
    /// modulus.
    pub fn blob(&self) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"RSA1");
        b.extend_from_slice(&(self.bits as u32).to_le_bytes());
        b.extend_from_slice(&self.e.to_le_bytes());
        b.extend(to_le(&self.n, self.bytes()));
        b
    }
    pub fn from_blob(b: &[u8]) -> Option<Self> {
        if b.get(..4)? != b"RSA1" && b.get(..4)? != b"RSA2" {
            return None;
        }
        let bits = u32::from_le_bytes(b.get(4..8)?.try_into().ok()?) as usize;
        let e = u32::from_le_bytes(b.get(8..12)?.try_into().ok()?);
        if !(384..=MAX_BITS).contains(&bits) || !bits.is_multiple_of(8) || e == 0 {
            return None;
        }
        let n = from_le(b.get(12..12 + bits / 8)?)?;
        n.bit_vartime(0).then_some(Self { bits, e, n })
    }
}

/// The DER DigestInfo prefix PKCS #1 v1.5 signatures put before a hash.
pub fn digest_info(algorithm: u32) -> Option<&'static [u8]> {
    Some(match algorithm {
        0x8001 => &[
            0x30, 0x20, 0x30, 0x0c, 0x06, 0x08, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x02, 0x02,
            0x05, 0x00, 0x04, 0x10,
        ],
        0x8003 => &[
            0x30, 0x20, 0x30, 0x0c, 0x06, 0x08, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x02, 0x05,
            0x05, 0x00, 0x04, 0x10,
        ],
        0x8004 => &[
            0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a, 0x05, 0x00, 0x04,
            0x14,
        ],
        0x800C => &[
            0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02,
            0x01, 0x05, 0x00, 0x04, 0x20,
        ],
        0x800D => &[
            0x30, 0x41, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02,
            0x02, 0x05, 0x00, 0x04, 0x30,
        ],
        0x800E => &[
            0x30, 0x51, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02,
            0x03, 0x05, 0x00, 0x04, 0x40,
        ],
        _ => return None,
    })
}
/// The big-endian encoded message `em` as a number.
fn from_be(em: &[u8]) -> Big {
    let mut b = [0u8; MAX_BITS / 8];
    b[MAX_BITS / 8 - em.len()..].copy_from_slice(em);
    Big::from_be_slice(&b)
}
fn to_be(v: &Big, len: usize) -> Vec<u8> {
    v.to_be_bytes()[MAX_BITS / 8 - len..].to_vec()
}
/// EMSA-PKCS1-v1_5 for `hash` (with its DigestInfo unless `no_oid`).
fn signature_block(bytes: usize, algorithm: u32, hash: &[u8], no_oid: bool) -> Option<Vec<u8>> {
    let prefix: &[u8] = if no_oid { &[] } else { digest_info(algorithm)? };
    let t = prefix.len() + hash.len();
    if bytes < t + 11 {
        return None;
    }
    let mut em = alloc::vec![0xffu8; bytes];
    em[0] = 0;
    em[1] = 1;
    em[bytes - t - 1] = 0;
    em[bytes - t..bytes - hash.len()].copy_from_slice(prefix);
    em[bytes - hash.len()..].copy_from_slice(hash);
    Some(em)
}
/// CryptSignHash: the signature, little-endian.
pub fn sign(key: &PrivateKey, algorithm: u32, hash: &[u8], no_oid: bool) -> Option<Vec<u8>> {
    let bytes = key.public.bytes();
    let em = signature_block(bytes, algorithm, hash, no_oid)?;
    let s = key.raw(&from_be(&em));
    let mut out = to_be(&s, bytes);
    out.reverse();
    Some(out)
}
/// CryptVerifySignature with a little-endian signature.
pub fn verify(
    key: &PublicKey,
    algorithm: u32,
    hash: &[u8],
    signature: &[u8],
    no_oid: bool,
) -> bool {
    let bytes = key.bytes();
    if signature.len() != bytes {
        return false;
    }
    let mut be = signature.to_vec();
    be.reverse();
    let s = from_be(&be);
    if s >= key.n {
        return false;
    }
    let Some(expected) = signature_block(bytes, algorithm, hash, no_oid) else {
        return false;
    };
    to_be(&key.raw(&s), bytes) == expected
}
/// RSAES-PKCS1-v1_5 (or OAEP with SHA-1 and MGF1) encryption of
/// `message`: the ciphertext, little-endian.
pub fn encrypt(
    key: &PublicKey,
    message: &[u8],
    oaep: bool,
    random: &mut dyn FnMut(&mut [u8]),
) -> Option<Vec<u8>> {
    let k = key.bytes();
    let mut em = alloc::vec![0u8; k];
    if oaep {
        let h = 20;
        if message.len() + 2 * h + 2 > k {
            return None;
        }
        let mut db = alloc::vec![0u8; k - h - 1];
        db[..h].copy_from_slice(&sha1_of(&[b""]));
        let dlen = db.len();
        db[dlen - message.len() - 1] = 1;
        db[dlen - message.len()..].copy_from_slice(message);
        let mut seed = [0u8; 20];
        random(&mut seed);
        let mask = mgf1(&seed, db.len());
        for (b, m) in db.iter_mut().zip(mask) {
            *b ^= m;
        }
        let seed_mask = mgf1(&db, h);
        for (s, m) in seed.iter_mut().zip(seed_mask) {
            *s ^= m;
        }
        em[1..1 + h].copy_from_slice(&seed);
        em[1 + h..].copy_from_slice(&db);
    } else {
        if message.len() + 11 > k {
            return None;
        }
        em[1] = 2;
        let ps = k - message.len() - 3;
        random(&mut em[2..2 + ps]);
        for b in &mut em[2..2 + ps] {
            while *b == 0 {
                let mut one = [0u8; 1];
                random(&mut one);
                *b = one[0];
            }
        }
        em[k - message.len()..].copy_from_slice(message);
    }
    let c = key.raw(&from_be(&em));
    let mut out = to_be(&c, k);
    out.reverse();
    Some(out)
}
/// The decryption of a little-endian ciphertext (None: not a valid
/// encryption block).
pub fn decrypt(key: &PrivateKey, ciphertext: &[u8], oaep: bool) -> Option<Vec<u8>> {
    let k = key.public.bytes();
    if ciphertext.len() != k {
        return None;
    }
    let mut be = ciphertext.to_vec();
    be.reverse();
    let c = from_be(&be);
    if c >= key.public.n {
        return None;
    }
    let em = to_be(&key.raw(&c), k);
    if oaep {
        let h = 20;
        if k < 2 * h + 2 || em[0] != 0 {
            return None;
        }
        let (masked_seed, masked_db) = em[1..].split_at(h);
        let mut seed = masked_seed.to_vec();
        for (s, m) in seed.iter_mut().zip(mgf1(masked_db, h)) {
            *s ^= m;
        }
        let mut db = masked_db.to_vec();
        for (b, m) in db.iter_mut().zip(mgf1(&seed, masked_db.len())) {
            *b ^= m;
        }
        if db[..h] != sha1_of(&[b""])[..] {
            return None;
        }
        let rest = &db[h..];
        let one = rest.iter().position(|&b| b != 0)?;
        (rest[one] == 1).then(|| rest[one + 1..].to_vec())
    } else {
        if em[0] != 0 || em[1] != 2 {
            return None;
        }
        let zero = em[2..].iter().position(|&b| b == 0)? + 2;
        (zero >= 10).then(|| em[zero + 1..].to_vec())
    }
}
fn sha1_of(parts: &[&[u8]]) -> [u8; 20] {
    use sha1::Digest;
    let mut h = sha1::Sha1::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}
fn mgf1(seed: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut counter = 0u32;
    while out.len() < len {
        out.extend_from_slice(&sha1_of(&[seed, &counter.to_be_bytes()]));
        counter += 1;
    }
    out.truncate(len);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A deterministic byte stream (xorshift) for repeatable keys.
    fn stream(seed: u64) -> impl FnMut(&mut [u8]) {
        let mut s = seed;
        move |buf: &mut [u8]| {
            for b in buf {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                *b = s as u8;
            }
        }
    }
    #[test]
    fn keys_sign_verify_encrypt_and_round_trip_their_blobs() {
        let mut random = stream(0x2026_1109);
        let key = PrivateKey::generate(1024, &mut random).unwrap();
        assert_eq!(key.public.n.bits_vartime(), 1024);
        let blob = key.blob();
        assert_eq!(blob.len(), 12 + 128 + 5 * 64 + 128);
        assert_eq!(PrivateKey::from_blob(&blob), Some(key.clone()));
        assert_eq!(
            PublicKey::from_blob(&key.public.blob()),
            Some(key.public.clone())
        );
        let hash = sha1_of(&[b"Doors"]);
        let sig = sign(&key, 0x8004, &hash, false).unwrap();
        assert!(verify(&key.public, 0x8004, &hash, &sig, false));
        assert!(!verify(&key.public, 0x8003, &hash, &sig, false));
        let mut bad = sig.clone();
        bad[0] ^= 1;
        assert!(!verify(&key.public, 0x8004, &hash, &bad, false));
        for oaep in [false, true] {
            let c = encrypt(&key.public, b"session key", oaep, &mut random).unwrap();
            assert_eq!(
                decrypt(&key, &c, oaep).as_deref(),
                Some(&b"session key"[..])
            );
        }
        // d computed from phi gives the same private operation.
        let m = Big::from_u32(0x1234_5678);
        let c = key.public.raw(&m);
        assert_eq!(key.raw(&c), m);
        assert_eq!(modpow(&c, &key.d, 1024, &key.public.n), m);
    }
}

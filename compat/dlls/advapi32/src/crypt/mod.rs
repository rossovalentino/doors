//! CryptoAPI's hashing and providers (host-testable): the Microsoft
//! providers' names and types, the hash algorithms each type offers, and
//! streaming MD2, MD4, MD5, SHA-1 and SHA-2 as CryptHashData feeds them.
//!
//! Measured on Windows 11 26H2: PROV_RSA_AES (24) hashes MD2..SHA-512,
//! PROV_RSA_FULL (1) has no SHA-2 (NTE_BAD_ALGID), CALG_HMAC needs a key
//! (NTE_BAD_KEY), hashing after the value was read is NTE_BAD_HASH_STATE.
use alloc::vec::Vec;
use sha1::Digest;
pub mod ciphers;
pub mod keys;
pub mod rsa;

/// HRESULT-style NTE_* codes CryptoAPI sets as the last error.
pub mod nte {
    pub const BAD_HASH: u32 = 0x8009_0002;
    pub const BAD_KEY: u32 = 0x8009_0003;
    pub const BAD_LEN: u32 = 0x8009_0004;
    pub const BAD_DATA: u32 = 0x8009_0005;
    pub const BAD_SIGNATURE: u32 = 0x8009_0006;
    pub const BAD_VER: u32 = 0x8009_0007;
    pub const BAD_TYPE: u32 = 0x8009_000A;
    pub const BAD_KEY_STATE: u32 = 0x8009_000B;
    pub const NO_KEY: u32 = 0x8009_000D;
    pub const BAD_ALGID: u32 = 0x8009_0008;
    pub const BAD_FLAGS: u32 = 0x8009_0009;
    pub const BAD_HASH_STATE: u32 = 0x8009_000C;
    pub const BAD_KEYSET: u32 = 0x8009_0016;
    pub const BAD_PROV_TYPE: u32 = 0x8009_0014;
    pub const PROV_TYPE_NOT_DEF: u32 = 0x8009_0017;
    pub const PROV_TYPE_NO_MATCH: u32 = 0x8009_001B;
    pub const KEYSET_NOT_DEF: u32 = 0x8009_0019;
    pub const EXISTS: u32 = 0x8009_000F;
    pub const NOT_SUPPORTED: u32 = 0x8009_0029;
}
/// CryptAcquireContext flags.
pub mod flags {
    pub const VERIFYCONTEXT: u32 = 0xF000_0000;
    pub const NEWKEYSET: u32 = 0x8;
    pub const DELETEKEYSET: u32 = 0x10;
    pub const MACHINE_KEYSET: u32 = 0x20;
    pub const SILENT: u32 = 0x40;
}
pub const PROV_RSA_FULL: u32 = 1;
pub const PROV_RSA_AES: u32 = 24;
/// The Base provider's name (shorter keys, no triple DES).
pub const BASE_PROVIDER: &str = "Microsoft Base Cryptographic Provider v1.0";
/// The providers by name (identifiers programs pass, as Windows spells
/// them) and type.
const PROVIDERS: &[(&str, u32)] = &[
    (BASE_PROVIDER, PROV_RSA_FULL),
    (
        "Microsoft Enhanced Cryptographic Provider v1.0",
        PROV_RSA_FULL,
    ),
    ("Microsoft Strong Cryptographic Provider", PROV_RSA_FULL),
    (
        "Microsoft Enhanced RSA and AES Cryptographic Provider",
        PROV_RSA_AES,
    ),
    (
        "Microsoft Enhanced RSA and AES Cryptographic Provider (Prototype)",
        PROV_RSA_AES,
    ),
];
/// The provider CryptAcquireContext opens: the named one (its type must
/// match) or the type's default. Err: the NTE code.
pub fn provider(name: Option<&str>, kind: u32) -> Result<&'static str, u32> {
    if kind != PROV_RSA_FULL && kind != PROV_RSA_AES {
        return Err(nte::PROV_TYPE_NOT_DEF);
    }
    match name {
        None => Ok(if kind == PROV_RSA_AES {
            PROVIDERS[3].0
        } else {
            PROVIDERS[2].0
        }),
        Some(n) => match PROVIDERS.iter().find(|(p, _)| p.eq_ignore_ascii_case(n)) {
            Some((p, t)) if *t == kind => Ok(p),
            Some(_) => Err(nte::PROV_TYPE_NO_MATCH),
            None => Err(nte::KEYSET_NOT_DEF),
        },
    }
}
/// ALG_ID values.
pub mod alg {
    pub const MD2: u32 = 0x8001;
    pub const MD4: u32 = 0x8002;
    pub const MD5: u32 = 0x8003;
    pub const SHA1: u32 = 0x8004;
    pub const MAC: u32 = 0x8005;
    pub const SSL3_SHAMD5: u32 = 0x8008;
    pub const HMAC: u32 = 0x8009;
    pub const SHA_256: u32 = 0x800C;
    pub const SHA_384: u32 = 0x800D;
    pub const SHA_512: u32 = 0x800E;
}
#[derive(Clone)]
enum State {
    Md2(Md2),
    Md4(Md45),
    Md5(Md45),
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Sha384(sha2::Sha384),
    Sha512(sha2::Sha512),
    /// HMAC: the key until HP_HMAC_INFO names the hash, then the inner
    /// hash and the outer pad.
    Hmac {
        key: Vec<u8>,
        inner: Option<(alloc::boxed::Box<Hash>, Vec<u8>)>,
    },
    /// CALG_MAC: the block cipher key and the data so far (its CBC-MAC is
    /// the last block of the padded CBC encryption).
    Mac(alloc::boxed::Box<keys::Symmetric>, Vec<u8>),
    /// SSL3 SHAMD5: no data, only a value set with HP_HASHVAL.
    Ssl3,
}
/// A CryptCreateHash object.
#[derive(Clone)]
pub struct Hash {
    pub algorithm: u32,
    state: State,
    value: Option<Vec<u8>>,
}
/// Block sizes of the hashes HMAC can use.
fn block_of(algorithm: u32) -> Option<usize> {
    match algorithm {
        alg::MD2 => Some(16),
        alg::MD4 | alg::MD5 | alg::SHA1 | alg::SHA_256 => Some(64),
        alg::SHA_384 | alg::SHA_512 => Some(128),
        _ => None,
    }
}
impl Hash {
    /// CryptCreateHash for a provider of `kind`.
    pub fn new(kind: u32, algorithm: u32) -> Result<Self, u32> {
        let state = match algorithm {
            alg::MD2 => State::Md2(Md2::default()),
            alg::MD4 => State::Md4(Md45::new(false)),
            alg::MD5 => State::Md5(Md45::new(true)),
            alg::SHA1 => State::Sha1(sha1::Sha1::new()),
            alg::SHA_256 if kind == PROV_RSA_AES => State::Sha256(sha2::Sha256::new()),
            alg::SHA_384 if kind == PROV_RSA_AES => State::Sha384(sha2::Sha384::new()),
            alg::SHA_512 if kind == PROV_RSA_AES => State::Sha512(sha2::Sha512::new()),
            alg::SSL3_SHAMD5 => State::Ssl3,
            // The keyed hashes need a key.
            alg::HMAC | alg::MAC => return Err(nte::BAD_KEY),
            _ => return Err(nte::BAD_ALGID),
        };
        Ok(Self {
            algorithm,
            state,
            value: None,
        })
    }
    /// CALG_HMAC with `key`'s bytes (the hash comes with HP_HMAC_INFO).
    pub fn hmac(key: &[u8]) -> Self {
        Self {
            algorithm: alg::HMAC,
            state: State::Hmac {
                key: key.to_vec(),
                inner: None,
            },
            value: None,
        }
    }
    /// CALG_MAC with a block cipher's key (RC4: NTE_BAD_KEY).
    pub fn mac(key: &keys::Symmetric) -> Result<Self, u32> {
        if key.block() == 0 || key.hmac_only {
            return Err(nte::BAD_KEY);
        }
        let mut key = key.clone();
        key.restart();
        Ok(Self {
            algorithm: alg::MAC,
            state: State::Mac(alloc::boxed::Box::new(key), Vec::new()),
            value: None,
        })
    }
    /// HP_HMAC_INFO: the hash HMAC uses (Windows' default inner and outer
    /// strings: 0x36 and 0x5C over the hash's block).
    pub fn set_hmac_info(&mut self, kind: u32, algorithm: u32) -> Result<(), u32> {
        let State::Hmac { key, inner } = &mut self.state else {
            return Err(nte::BAD_TYPE);
        };
        if inner.is_some() || self.value.is_some() {
            return Err(nte::BAD_HASH_STATE);
        }
        let block = block_of(algorithm).ok_or(nte::BAD_ALGID)?;
        let mut k = key.clone();
        if k.len() > block {
            let mut h = Hash::new(kind, algorithm)?;
            h.update(&k)?;
            k = h.value();
        }
        k.resize(block, 0);
        let mut h = Hash::new(kind, algorithm)?;
        let ipad: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
        h.update(&ipad)?;
        let opad: Vec<u8> = k.iter().map(|b| b ^ 0x5C).collect();
        *inner = Some((alloc::boxed::Box::new(h), opad));
        Ok(())
    }
    pub fn size(&self) -> usize {
        match &self.state {
            State::Hmac {
                inner: Some((h, _)),
                ..
            } => h.size(),
            State::Hmac { .. } => 0,
            State::Mac(k, _) => k.block(),
            State::Ssl3 => 36,
            _ => match self.algorithm {
                alg::SHA1 => 20,
                alg::SHA_256 => 32,
                alg::SHA_384 => 48,
                alg::SHA_512 => 64,
                _ => 16,
            },
        }
    }
    /// CryptHashData: refused once the value was read.
    pub fn update(&mut self, data: &[u8]) -> Result<(), u32> {
        if self.value.is_some() {
            return Err(nte::BAD_HASH_STATE);
        }
        match &mut self.state {
            State::Md2(s) => s.update(data),
            State::Md4(s) | State::Md5(s) => s.update(data),
            State::Sha1(s) => s.update(data),
            State::Sha256(s) => s.update(data),
            State::Sha384(s) => s.update(data),
            State::Sha512(s) => s.update(data),
            State::Hmac {
                inner: Some((h, _)),
                ..
            } => h.update(data)?,
            State::Hmac { .. } => return Err(nte::BAD_HASH_STATE),
            State::Mac(_, buffer) => buffer.extend_from_slice(data),
            // As Windows: SSL3 SHAMD5 takes no data (ERROR_INVALID_HANDLE).
            State::Ssl3 => return Err(6),
        }
        Ok(())
    }
    /// HP_HASHVAL set: a value computed elsewhere (to sign it).
    pub fn set_value(&mut self, value: &[u8]) -> Result<(), u32> {
        if matches!(self.state, State::Hmac { .. } | State::Mac(..)) {
            return Err(nte::BAD_TYPE);
        }
        if value.len() < self.size() {
            return Err(nte::BAD_LEN);
        }
        self.value = Some(value[..self.size()].to_vec());
        Ok(())
    }
    /// HP_HASHVAL: finishes the hash; later reads repeat the value.
    pub fn try_value(&mut self) -> Result<Vec<u8>, u32> {
        if let Some(v) = &self.value {
            return Ok(v.clone());
        }
        let v = match self.state.clone() {
            State::Md2(s) => s.finish().to_vec(),
            State::Md4(s) | State::Md5(s) => s.finish().to_vec(),
            State::Sha1(s) => s.finalize().to_vec(),
            State::Sha256(s) => s.finalize().to_vec(),
            State::Sha384(s) => s.finalize().to_vec(),
            State::Sha512(s) => s.finalize().to_vec(),
            State::Hmac {
                inner: Some((mut h, opad)),
                key,
            } => {
                let inner_value = h.value();
                let mut outer = Hash::new(PROV_RSA_AES, h.algorithm)?;
                outer.update(&opad)?;
                outer.update(&inner_value)?;
                let _ = key;
                outer.value()
            }
            State::Hmac { .. } | State::Ssl3 => return Err(nte::BAD_HASH_STATE),
            State::Mac(mut key, mut data) => {
                key.encrypt(&mut data, true)?;
                let block = key.block();
                data[data.len() - block..].to_vec()
            }
        };
        self.value = Some(v.clone());
        Ok(v)
    }
    /// `try_value` of a hash that always has one.
    pub fn value(&mut self) -> Vec<u8> {
        self.try_value().unwrap_or_default()
    }
}

/// MD4 (RFC 1320) and MD5 (RFC 1321), streaming.
#[derive(Clone)]
struct Md45 {
    md5: bool,
    state: [u32; 4],
    block: [u8; 64],
    filled: usize,
    length: u64,
}
const K5: [u32; 64] = {
    let mut k = [0u32; 64];
    // floor(abs(sin(i + 1)) * 2^32), as RFC 1321 tabulates it.
    let table: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];
    let mut i = 0;
    while i < 64 {
        k[i] = table[i];
        i += 1;
    }
    k
};
const R5: [u32; 16] = [7, 12, 17, 22, 5, 9, 14, 20, 4, 11, 16, 23, 6, 10, 15, 21];
impl Md45 {
    fn new(md5: bool) -> Self {
        Self {
            md5,
            state: [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476],
            block: [0; 64],
            filled: 0,
            length: 0,
        }
    }
    fn update(&mut self, mut data: &[u8]) {
        self.length = self.length.wrapping_add(data.len() as u64);
        while !data.is_empty() {
            let n = (64 - self.filled).min(data.len());
            self.block[self.filled..self.filled + n].copy_from_slice(&data[..n]);
            self.filled += n;
            data = &data[n..];
            if self.filled == 64 {
                let block = self.block;
                self.compress(&block);
                self.filled = 0;
            }
        }
    }
    fn finish(mut self) -> [u8; 16] {
        let bits = self.length.wrapping_mul(8);
        self.update(&[0x80]);
        while self.filled != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_le_bytes());
        let mut out = [0u8; 16];
        for (i, s) in self.state.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&s.to_le_bytes());
        }
        out
    }
    fn compress(&mut self, block: &[u8; 64]) {
        let mut x = [0u32; 16];
        for (i, w) in x.iter_mut().enumerate() {
            *w = u32::from_le_bytes(block[4 * i..4 * i + 4].try_into().unwrap());
        }
        let [mut a, mut b, mut c, mut d] = self.state;
        if self.md5 {
            for i in 0..64 {
                let (f, k) = match i / 16 {
                    0 => ((b & c) | (!b & d), i),
                    1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                    2 => (b ^ c ^ d, (3 * i + 5) % 16),
                    _ => (c ^ (b | !d), (7 * i) % 16),
                };
                let t = a
                    .wrapping_add(f)
                    .wrapping_add(K5[i])
                    .wrapping_add(x[k])
                    .rotate_left(R5[(i / 16) * 4 + i % 4]);
                (a, b, c, d) = (d, b.wrapping_add(t), b, c);
            }
        } else {
            for i in 0..16 {
                let f = (b & c) | (!b & d);
                let t = a
                    .wrapping_add(f)
                    .wrapping_add(x[i])
                    .rotate_left([3, 7, 11, 19][i % 4]);
                (a, b, c, d) = (d, t, b, c);
            }
            for i in 0..16 {
                let g = (b & c) | (b & d) | (c & d);
                let t = a
                    .wrapping_add(g)
                    .wrapping_add(x[(i % 4) * 4 + i / 4])
                    .wrapping_add(0x5a82_7999)
                    .rotate_left([3, 5, 9, 13][i % 4]);
                (a, b, c, d) = (d, t, b, c);
            }
            const ORDER: [usize; 16] = [0, 8, 4, 12, 2, 10, 6, 14, 1, 9, 5, 13, 3, 11, 7, 15];
            for (i, &k) in ORDER.iter().enumerate() {
                let t = a
                    .wrapping_add(b ^ c ^ d)
                    .wrapping_add(x[k])
                    .wrapping_add(0x6ed9_eba1)
                    .rotate_left([3, 9, 11, 15][i % 4]);
                (a, b, c, d) = (d, t, b, c);
            }
        }
        for (s, v) in self.state.iter_mut().zip([a, b, c, d]) {
            *s = s.wrapping_add(v);
        }
    }
}

/// MD2 (RFC 1319), streaming.
#[derive(Clone)]
struct Md2 {
    state: [u8; 48],
    checksum: [u8; 16],
    block: [u8; 16],
    filled: usize,
}
/// RFC 1319's permutation of 0..255 (from the digits of pi).
const PI_SUBST: [u8; 256] = [
    41, 46, 67, 201, 162, 216, 124, 1, 61, 54, 84, 161, 236, 240, 6, 19, 98, 167, 5, 243, 192, 199,
    115, 140, 152, 147, 43, 217, 188, 76, 130, 202, 30, 155, 87, 60, 253, 212, 224, 22, 103, 66,
    111, 24, 138, 23, 229, 18, 190, 78, 196, 214, 218, 158, 222, 73, 160, 251, 245, 142, 187, 47,
    238, 122, 169, 104, 121, 145, 21, 178, 7, 63, 148, 194, 16, 137, 11, 34, 95, 33, 128, 127, 93,
    154, 90, 144, 50, 39, 53, 62, 204, 231, 191, 247, 151, 3, 255, 25, 48, 179, 72, 165, 181, 209,
    215, 94, 146, 42, 172, 86, 170, 198, 79, 184, 56, 210, 150, 164, 125, 182, 118, 252, 107, 226,
    156, 116, 4, 241, 69, 157, 112, 89, 100, 113, 135, 32, 134, 91, 207, 101, 230, 45, 168, 2, 27,
    96, 37, 173, 174, 176, 185, 246, 28, 70, 97, 105, 52, 64, 126, 15, 85, 71, 163, 35, 221, 81,
    175, 58, 195, 92, 249, 206, 186, 197, 234, 38, 44, 83, 13, 110, 133, 40, 132, 9, 211, 223, 205,
    244, 65, 129, 77, 82, 106, 220, 55, 200, 108, 193, 171, 250, 36, 225, 123, 8, 12, 189, 177, 74,
    120, 136, 149, 139, 227, 99, 232, 109, 233, 203, 213, 254, 59, 0, 29, 57, 242, 239, 183, 14,
    102, 88, 208, 228, 166, 119, 114, 248, 235, 117, 75, 10, 49, 68, 80, 180, 143, 237, 31, 26,
    219, 153, 141, 51, 159, 17, 131, 20,
];
impl Default for Md2 {
    fn default() -> Self {
        Self {
            state: [0; 48],
            checksum: [0; 16],
            block: [0; 16],
            filled: 0,
        }
    }
}
impl Md2 {
    fn update(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            let n = (16 - self.filled).min(data.len());
            self.block[self.filled..self.filled + n].copy_from_slice(&data[..n]);
            self.filled += n;
            data = &data[n..];
            if self.filled == 16 {
                let block = self.block;
                self.compress(&block);
                self.filled = 0;
            }
        }
    }
    fn compress(&mut self, block: &[u8; 16]) {
        for (i, b) in block.iter().enumerate() {
            self.state[16 + i] = *b;
            self.state[32 + i] = *b ^ self.state[i];
        }
        let mut t = 0u8;
        for round in 0..18u8 {
            for v in self.state.iter_mut() {
                *v ^= PI_SUBST[usize::from(t)];
                t = *v;
            }
            t = t.wrapping_add(round);
        }
        let mut l = self.checksum[15];
        for i in 0..16 {
            self.checksum[i] ^= PI_SUBST[usize::from(block[i] ^ l)];
            l = self.checksum[i];
        }
    }
    fn finish(mut self) -> [u8; 16] {
        let pad = (16 - self.filled) as u8;
        let padding = [pad; 16];
        self.update(&padding[..usize::from(pad)]);
        let checksum = self.checksum;
        self.compress(&checksum);
        self.state[..16].try_into().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hex(b: &[u8]) -> alloc::string::String {
        b.iter().map(|v| alloc::format!("{v:02x}")).collect()
    }
    #[test]
    fn digests_match_the_windows_providers() {
        // CryptHashData("abc") on Windows 11 (secprobe more).
        for (kind, algorithm, digest) in [
            (PROV_RSA_AES, alg::MD5, "900150983cd24fb0d6963f7d28e17f72"),
            (
                PROV_RSA_AES,
                alg::SHA1,
                "a9993e364706816aba3e25717850c26c9cd0d89d",
            ),
            (
                PROV_RSA_AES,
                alg::SHA_256,
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                PROV_RSA_AES,
                alg::SHA_384,
                "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7",
            ),
            (
                PROV_RSA_AES,
                alg::SHA_512,
                "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
            ),
            (PROV_RSA_FULL, alg::MD4, "a448017aaf21d8525fc10ae87aa6729d"),
            (PROV_RSA_FULL, alg::MD2, "da853b0d3f88d99b30283a69e6ded6bb"),
        ] {
            let mut h = Hash::new(kind, algorithm).unwrap();
            // Fed in pieces.
            h.update(b"a").unwrap();
            h.update(b"bc").unwrap();
            assert_eq!(hex(&h.value()), digest, "{algorithm:x}");
            assert_eq!(h.size(), digest.len() / 2);
            assert_eq!(h.update(b"x"), Err(nte::BAD_HASH_STATE));
            assert_eq!(hex(&h.value()), digest);
        }
        // Longer than a block, across block boundaries.
        let long = [0x61u8; 1000];
        let mut h = Hash::new(PROV_RSA_FULL, alg::MD5).unwrap();
        for chunk in long.chunks(37) {
            h.update(chunk).unwrap();
        }
        assert_eq!(hex(&h.value()), "cabe45dcc9ae5b66ba86600cca6b8ba8");
        let mut h = Hash::new(PROV_RSA_FULL, alg::MD2).unwrap();
        assert_eq!(hex(&h.value()), "8350e5a3e24c153df2275c9f80692773");
    }
    #[test]
    fn providers_and_algorithms_per_type() {
        assert_eq!(
            Hash::new(PROV_RSA_FULL, alg::SHA_256).err(),
            Some(nte::BAD_ALGID)
        );
        assert_eq!(
            Hash::new(PROV_RSA_FULL, alg::HMAC).err(),
            Some(nte::BAD_KEY)
        );
        assert_eq!(
            provider(None, PROV_RSA_AES),
            Ok("Microsoft Enhanced RSA and AES Cryptographic Provider")
        );
        assert_eq!(provider(None, 999), Err(nte::PROV_TYPE_NOT_DEF));
        assert_eq!(
            provider(
                Some("Microsoft Base Cryptographic Provider v1.0"),
                PROV_RSA_AES
            ),
            Err(nte::PROV_TYPE_NO_MATCH)
        );
        assert_eq!(
            provider(Some("Nobody"), PROV_RSA_FULL),
            Err(nte::KEYSET_NOT_DEF)
        );
    }
}

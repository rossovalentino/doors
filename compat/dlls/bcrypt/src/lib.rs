#![no_std]
//! Original bcrypt.dll (Cryptography API: Next Generation, the primitive
//! provider): the system random number generator and the hash algorithms
//! (MD2, MD4, MD5, SHA-1, SHA-256, SHA-384, SHA-512, each also as HMAC),
//! through algorithm handles (opened, or Windows 10's pseudo-handles such
//! as BCRYPT_SHA256_ALG_HANDLE), reusable hash objects and the one-shot
//! BCryptHash. The digests are advapi32's CryptoAPI engine
//! (`advapi32::crypt`), so both APIs hash identically.
//!
//! This module is the host-testable part: algorithm names, pseudo-handles,
//! properties and hash objects. The exports are in `guest`.
//!
//! The same sources build the x64 and the x86 DLL: exports take
//! pointer-width parameters, widened at entry.
#![allow(clippy::redundant_closure_call)]
extern crate alloc;

use advapi32::crypt::{self, Hash, alg};
use alloc::vec::Vec;

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

/// NTSTATUS values CNG returns (ntstatus.h).
pub mod status {
    pub const SUCCESS: u32 = 0;
    pub const INVALID_HANDLE: u32 = 0xC000_0008;
    pub const INVALID_PARAMETER: u32 = 0xC000_000D;
    pub const NO_MEMORY: u32 = 0xC000_0017;
    pub const BUFFER_TOO_SMALL: u32 = 0xC000_0023;
    pub const NOT_SUPPORTED: u32 = 0xC000_00BB;
    pub const NOT_FOUND: u32 = 0xC000_0225;
    pub const INVALID_BUFFER_SIZE: u32 = 0xC000_0206;
}
/// Flags of BCryptOpenAlgorithmProvider and BCryptCreateHash.
pub mod flags {
    pub const PROV_DISPATCH: u32 = 0x1;
    pub const ALG_HANDLE_HMAC: u32 = 0x8;
    pub const HASH_REUSABLE: u32 = 0x20;
    /// BCryptGenRandom: the system RNG, with no algorithm handle.
    pub const USE_SYSTEM_PREFERRED_RNG: u32 = 0x2;
}

/// The algorithms the primitive provider offers here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Algorithm {
    Rng,
    /// A hash: its CryptoAPI ALG_ID.
    Hash(u32),
}
impl Algorithm {
    /// BCRYPT_*_ALGORITHM identifiers, compared case-insensitively.
    pub fn by_name(name: &[u16]) -> Option<Self> {
        const NAMES: [(&str, Algorithm); 8] = [
            ("RNG", Algorithm::Rng),
            ("MD2", Algorithm::Hash(alg::MD2)),
            ("MD4", Algorithm::Hash(alg::MD4)),
            ("MD5", Algorithm::Hash(alg::MD5)),
            ("SHA1", Algorithm::Hash(alg::SHA1)),
            ("SHA256", Algorithm::Hash(alg::SHA_256)),
            ("SHA384", Algorithm::Hash(alg::SHA_384)),
            ("SHA512", Algorithm::Hash(alg::SHA_512)),
        ];
        NAMES.iter().find_map(|(n, a)| {
            (n.len() == name.len() && n.bytes().zip(name).all(|(b, u)| u16::from(b) == fold(*u)))
                .then_some(*a)
        })
    }
    /// The identifier BCryptGetProperty(BCRYPT_ALGORITHM_NAME) reports.
    pub fn name(self) -> &'static str {
        match self {
            Self::Rng => "RNG",
            Self::Hash(alg::MD2) => "MD2",
            Self::Hash(alg::MD4) => "MD4",
            Self::Hash(alg::MD5) => "MD5",
            Self::Hash(alg::SHA1) => "SHA1",
            Self::Hash(alg::SHA_256) => "SHA256",
            Self::Hash(alg::SHA_384) => "SHA384",
            Self::Hash(_) => "SHA512",
        }
    }
    pub fn digest_bytes(self) -> Option<u32> {
        match self {
            Self::Rng => None,
            Self::Hash(alg::SHA1) => Some(20),
            Self::Hash(alg::SHA_256) => Some(32),
            Self::Hash(alg::SHA_384) => Some(48),
            Self::Hash(alg::SHA_512) => Some(64),
            Self::Hash(_) => Some(16),
        }
    }
    pub fn block_bytes(self) -> Option<u32> {
        match self {
            Self::Rng => None,
            Self::Hash(alg::MD2) => Some(16),
            Self::Hash(alg::SHA_384 | alg::SHA_512) => Some(128),
            Self::Hash(_) => Some(64),
        }
    }
}
fn fold(unit: u16) -> u16 {
    if (u16::from(b'a')..=u16::from(b'z')).contains(&unit) {
        unit - 0x20
    } else {
        unit
    }
}

/// An algorithm handle's meaning: the algorithm, whether it makes HMACs
/// and whether its hash objects are reusable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Provider {
    pub algorithm: Algorithm,
    pub hmac: bool,
    pub reusable: bool,
}
impl Provider {
    /// BCryptOpenAlgorithmProvider: HMAC only for hashes; any
    /// implementation name but the primitive provider's is not found.
    pub fn open(name: &[u16], flags: u32) -> Result<Self, u32> {
        let known = flags::PROV_DISPATCH | flags::ALG_HANDLE_HMAC | flags::HASH_REUSABLE;
        if flags & !known != 0 {
            return Err(status::INVALID_PARAMETER);
        }
        let algorithm = Algorithm::by_name(name).ok_or(status::NOT_FOUND)?;
        let hmac = flags & flags::ALG_HANDLE_HMAC != 0;
        if hmac && algorithm == Algorithm::Rng {
            return Err(status::NOT_SUPPORTED);
        }
        Ok(Self {
            algorithm,
            hmac,
            reusable: flags & flags::HASH_REUSABLE != 0,
        })
    }
    /// Windows 10's pseudo-handles (bcrypt.h BCRYPT_*_ALG_HANDLE): usable
    /// wherever an algorithm handle is, never opened or closed.
    pub fn pseudo(handle: u64) -> Option<Self> {
        let (algorithm, hmac) = match handle {
            0x01 => (Algorithm::Hash(alg::MD2), false),
            0x11 => (Algorithm::Hash(alg::MD4), false),
            0x21 => (Algorithm::Hash(alg::MD5), false),
            0x31 => (Algorithm::Hash(alg::SHA1), false),
            0x41 => (Algorithm::Hash(alg::SHA_256), false),
            0x51 => (Algorithm::Hash(alg::SHA_384), false),
            0x61 => (Algorithm::Hash(alg::SHA_512), false),
            0x81 => (Algorithm::Rng, false),
            0x91 => (Algorithm::Hash(alg::MD5), true),
            0xa1 => (Algorithm::Hash(alg::SHA1), true),
            0xb1 => (Algorithm::Hash(alg::SHA_256), true),
            0xc1 => (Algorithm::Hash(alg::SHA_384), true),
            0xd1 => (Algorithm::Hash(alg::SHA_512), true),
            _ => return None,
        };
        Some(Self {
            algorithm,
            hmac,
            // Pseudo-handles' hashes are reusable (as Windows makes them).
            reusable: true,
        })
    }
}

/// What BCryptGetProperty answers on an algorithm or hash handle: a
/// DWORD or a NUL-terminated UTF-16 string.
pub fn property(provider: &Provider, name: &[u16]) -> Result<Vec<u8>, u32> {
    let is = |s: &str| s.len() == name.len() && s.encode_utf16().zip(name).all(|(a, b)| a == *b);
    let dword = |v: Option<u32>| {
        v.map(|v| v.to_le_bytes().to_vec())
            .ok_or(status::NOT_SUPPORTED)
    };
    if is("AlgorithmName") {
        return Ok(provider
            .algorithm
            .name()
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect());
    }
    if is("HashDigestLength") {
        return dword(provider.algorithm.digest_bytes());
    }
    if is("HashBlockLength") {
        return dword(provider.algorithm.block_bytes());
    }
    if is("ObjectLength") {
        // The caller's hash object buffer: unused here (state lives in the
        // DLL), but sized as a real one so callers allocate it.
        return dword(provider.algorithm.digest_bytes().map(|_| OBJECT_BYTES));
    }
    Err(status::NOT_SUPPORTED)
}
/// The hash object size reported (Windows' primitive provider objects are
/// a few hundred bytes).
pub const OBJECT_BYTES: u32 = 0x200;

/// A BCryptCreateHash object: the running hash and, for a reusable one,
/// the state it restarts from after BCryptFinishHash.
#[derive(Clone)]
pub struct HashObject {
    pub provider: Provider,
    running: Hash,
    fresh: Option<Hash>,
}
impl HashObject {
    /// A hash (an HMAC keyed with `secret`, which may be empty).
    pub fn new(provider: Provider, secret: &[u8], reusable: bool) -> Result<Self, u32> {
        let Algorithm::Hash(id) = provider.algorithm else {
            return Err(status::INVALID_HANDLE);
        };
        let hash = if provider.hmac {
            let mut h = Hash::hmac(secret);
            h.set_hmac_info(crypt::PROV_RSA_AES, id)
                .map_err(|_| status::NOT_SUPPORTED)?;
            h
        } else {
            Hash::new(crypt::PROV_RSA_AES, id).map_err(|_| status::NOT_SUPPORTED)?
        };
        let reusable = reusable || provider.reusable;
        Ok(Self {
            provider,
            fresh: reusable.then(|| hash.clone()),
            running: hash,
        })
    }
    pub fn update(&mut self, data: &[u8]) -> Result<(), u32> {
        self.running
            .update(data)
            .map_err(|_| status::INVALID_PARAMETER)
    }
    /// BCryptFinishHash: `out` must be the digest's exact size. A reusable
    /// object starts over; another is finished (further use is invalid).
    pub fn finish(&mut self, out: &mut [u8]) -> Result<(), u32> {
        let size = self.provider.algorithm.digest_bytes().unwrap_or(0) as usize;
        if out.len() != size {
            return Err(status::INVALID_PARAMETER);
        }
        let value = self
            .running
            .try_value()
            .map_err(|_| status::INVALID_HANDLE)?;
        out.copy_from_slice(&value[..size]);
        if let Some(fresh) = &self.fresh {
            self.running = fresh.clone();
        }
        Ok(())
    }
}

/// BCryptHash: one call over `data`.
pub fn hash_once(
    provider: Provider,
    secret: &[u8],
    data: &[u8],
    out: &mut [u8],
) -> Result<(), u32> {
    let mut object = HashObject::new(provider, secret, false)?;
    object.update(data)?;
    object.finish(out)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    fn hex(bytes: &[u8]) -> std::string::String {
        bytes.iter().map(|b| std::format!("{b:02x}")).collect()
    }

    #[test]
    fn algorithms_open_by_name_case_insensitively() {
        let p = Provider::open(&wide("sha256"), 0).unwrap();
        assert_eq!(p.algorithm, Algorithm::Hash(alg::SHA_256));
        assert!(!p.hmac && !p.reusable);
        let p =
            Provider::open(&wide("SHA1"), flags::ALG_HANDLE_HMAC | flags::HASH_REUSABLE).unwrap();
        assert!(p.hmac && p.reusable);
        assert_eq!(Provider::open(&wide("AES"), 0), Err(status::NOT_FOUND));
        assert_eq!(
            Provider::open(&wide("RNG"), flags::ALG_HANDLE_HMAC),
            Err(status::NOT_SUPPORTED)
        );
        assert_eq!(
            Provider::open(&wide("SHA256"), 0x100),
            Err(status::INVALID_PARAMETER)
        );
    }

    #[test]
    fn properties_answer_as_dwords_and_names() {
        let p = Provider::open(&wide("SHA384"), 0).unwrap();
        assert_eq!(
            property(&p, &wide("HashDigestLength")).unwrap(),
            48u32.to_le_bytes()
        );
        assert_eq!(
            property(&p, &wide("HashBlockLength")).unwrap(),
            128u32.to_le_bytes()
        );
        assert_eq!(
            property(&p, &wide("ObjectLength")).unwrap(),
            OBJECT_BYTES.to_le_bytes()
        );
        let name = property(&p, &wide("AlgorithmName")).unwrap();
        assert_eq!(
            name,
            wide("SHA384\0")
                .iter()
                .flat_map(|u| u.to_le_bytes())
                .collect::<Vec<_>>()
        );
        assert_eq!(property(&p, &wide("KeyLength")), Err(status::NOT_SUPPORTED));
    }

    #[test]
    fn digests_match_the_standard_vectors() {
        let mut out = [0u8; 32];
        hash_once(Provider::pseudo(0x41).unwrap(), &[], b"abc", &mut out).unwrap();
        assert_eq!(
            hex(&out),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let mut md5 = [0u8; 16];
        hash_once(Provider::pseudo(0x21).unwrap(), &[], b"abc", &mut md5).unwrap();
        assert_eq!(hex(&md5), "900150983cd24fb0d6963f7d28e17f72");
        // RFC 4231 test case 2: HMAC-SHA-256, key "Jefe".
        hash_once(
            Provider::pseudo(0xb1).unwrap(),
            b"Jefe",
            b"what do ya want for nothing?",
            &mut out,
        )
        .unwrap();
        assert_eq!(
            hex(&out),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // A wrong output size is refused.
        assert_eq!(
            hash_once(Provider::pseudo(0x41).unwrap(), &[], b"abc", &mut md5),
            Err(status::INVALID_PARAMETER)
        );
    }

    #[test]
    fn reusable_objects_start_over_after_finishing() {
        let p = Provider::open(&wide("SHA1"), flags::HASH_REUSABLE).unwrap();
        let mut h = HashObject::new(p, &[], false).unwrap();
        let mut first = [0u8; 20];
        h.update(b"ab").unwrap();
        h.update(b"c").unwrap();
        h.finish(&mut first).unwrap();
        assert_eq!(hex(&first), "a9993e364706816aba3e25717850c26c9cd0d89d");
        let mut second = [0u8; 20];
        h.update(b"abc").unwrap();
        h.finish(&mut second).unwrap();
        assert_eq!(first, second);
        // Not reusable: a second finish is refused.
        let p = Provider::open(&wide("SHA1"), 0).unwrap();
        let mut once = HashObject::new(p, &[], false).unwrap();
        once.finish(&mut first).unwrap();
        assert!(once.update(b"x").is_err());
    }
}

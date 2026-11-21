//! CryptoAPI keys as Microsoft's RSA providers keep them: symmetric session
//! keys (RC2, RC4, DES, two- and three-key triple DES, AES) with their
//! salt, mode, padding and chaining state, and RSA key pairs; import and
//! export as key blobs, CryptDeriveKey, CryptGenKey's sizes, CryptEncrypt and
//! CryptDecrypt, HMAC keys. Behaviour, defaults and error codes follow what
//! Windows' Base, Enhanced, Strong and Enhanced RSA and AES providers report
//! (`tests/reference/cryptprobe-raw.txt`).
use super::ciphers::{Des, Rc2, Rc4, TripleDes};
use super::rsa::{self, PrivateKey, PublicKey};
use alloc::vec::Vec;

/// ALG_IDs of keys.
pub mod calg {
    pub const RC2: u32 = 0x6602;
    pub const RC4: u32 = 0x6801;
    pub const DES: u32 = 0x6601;
    pub const DES3_112: u32 = 0x6609;
    pub const DES3: u32 = 0x6603;
    pub const AES_128: u32 = 0x660E;
    pub const AES_192: u32 = 0x660F;
    pub const AES_256: u32 = 0x6610;
    pub const RSA_SIGN: u32 = 0x2400;
    pub const RSA_KEYX: u32 = 0xA400;
    pub const AT_KEYEXCHANGE: u32 = 1;
    pub const AT_SIGNATURE: u32 = 2;
}
/// KP_* key parameters.
pub mod kp {
    pub const IV: u32 = 1;
    pub const SALT: u32 = 2;
    pub const PADDING: u32 = 3;
    pub const MODE: u32 = 4;
    pub const MODE_BITS: u32 = 5;
    pub const PERMISSIONS: u32 = 6;
    pub const ALGID: u32 = 7;
    pub const BLOCKLEN: u32 = 8;
    pub const KEYLEN: u32 = 9;
    pub const EFFECTIVE_KEYLEN: u32 = 19;
}
/// CRYPT_MODE_*.
pub mod mode {
    pub const CBC: u32 = 1;
    pub const ECB: u32 = 2;
    pub const OFB: u32 = 3;
    pub const CFB: u32 = 4;
    pub const CTS: u32 = 5;
}
/// Key blob types (BLOBHEADER.bType).
pub mod blob {
    pub const SIMPLE: u8 = 1;
    pub const PUBLIC: u8 = 6;
    pub const PRIVATE: u8 = 7;
    pub const PLAINTEXT: u8 = 8;
}
/// CryptGenKey/CryptDeriveKey/CryptImportKey flags.
pub mod flag {
    pub const EXPORTABLE: u32 = 0x1;
    pub const CREATE_SALT: u32 = 0x4;
    pub const NO_SALT: u32 = 0x10;
    pub const IPSEC_HMAC_KEY: u32 = 0x100;
    pub const OAEP: u32 = 0x40;
    pub const NOHASHOID: u32 = 0x1;
}
/// CRYPT_ENCRYPT | DECRYPT | READ | WRITE | MAC, plus EXPORT when
/// exportable.
const PERMISSIONS: u32 = 0x3B;
const PERMISSION_EXPORT: u32 = 0x4;

use super::nte;
const ERROR_MORE_DATA: u32 = 234;
const ERROR_INTERNAL_ERROR: u32 = 1359;
const ERROR_GEN_FAILURE: u32 = 31;

/// One algorithm of a provider, as PP_ENUMALGS_EX describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Algorithm {
    pub id: u32,
    pub default: u32,
    pub min: u32,
    pub max: u32,
    pub protocols: u32,
    pub name: &'static str,
    pub long_name: &'static str,
}
const fn alg(
    id: u32,
    default: u32,
    min: u32,
    max: u32,
    protocols: u32,
    name: &'static str,
    long_name: &'static str,
) -> Algorithm {
    Algorithm {
        id,
        default,
        min,
        max,
        protocols,
        name,
        long_name,
    }
}
/// The Base provider's algorithms, in Windows' order.
const BASE: [Algorithm; 12] = [
    alg(0x6602, 40, 40, 56, 0x00, "RC2", "RSA Data Security's RC2"),
    alg(0x6801, 40, 40, 56, 0x00, "RC4", "RSA Data Security's RC4"),
    alg(
        0x6601,
        56,
        56,
        56,
        0x00,
        "DES",
        "Data Encryption Standard (DES)",
    ),
    alg(
        0x8004,
        160,
        160,
        160,
        0x20,
        "SHA-1",
        "Secure Hash Algorithm (SHA-1)",
    ),
    alg(0x8001, 128, 128, 128, 0x20, "MD2", "Message Digest 2 (MD2)"),
    alg(0x8002, 128, 128, 128, 0x20, "MD4", "Message Digest 4 (MD4)"),
    alg(0x8003, 128, 128, 128, 0x20, "MD5", "Message Digest 5 (MD5)"),
    alg(0x8008, 288, 288, 288, 0x00, "SSL3 SHAMD5", "SSL3 SHAMD5"),
    alg(0x8005, 0, 0, 0, 0x00, "MAC", "Message Authentication Code"),
    alg(0x2400, 512, 384, 16384, 0x30, "RSA_SIGN", "RSA Signature"),
    alg(0xa400, 512, 384, 1024, 0x30, "RSA_KEYX", "RSA Key Exchange"),
    alg(0x8009, 0, 0, 0, 0x00, "HMAC", "Hugo's MAC (HMAC)"),
];
/// The Enhanced and Strong providers' (identical).
const ENHANCED: [Algorithm; 14] = [
    alg(0x6602, 128, 40, 128, 0x00, "RC2", "RSA Data Security's RC2"),
    alg(0x6801, 128, 40, 128, 0x00, "RC4", "RSA Data Security's RC4"),
    alg(
        0x6601,
        56,
        56,
        56,
        0x00,
        "DES",
        "Data Encryption Standard (DES)",
    ),
    alg(
        0x6609,
        112,
        112,
        112,
        0x00,
        "3DES TWO KEY",
        "Two Key Triple DES",
    ),
    alg(0x6603, 168, 168, 168, 0x00, "3DES", "Three Key Triple DES"),
    alg(
        0x8004,
        160,
        160,
        160,
        0x20,
        "SHA-1",
        "Secure Hash Algorithm (SHA-1)",
    ),
    alg(0x8001, 128, 128, 128, 0x20, "MD2", "Message Digest 2 (MD2)"),
    alg(0x8002, 128, 128, 128, 0x20, "MD4", "Message Digest 4 (MD4)"),
    alg(0x8003, 128, 128, 128, 0x20, "MD5", "Message Digest 5 (MD5)"),
    alg(0x8008, 288, 288, 288, 0x00, "SSL3 SHAMD5", "SSL3 SHAMD5"),
    alg(0x8005, 0, 0, 0, 0x00, "MAC", "Message Authentication Code"),
    alg(0x2400, 1024, 384, 16384, 0x30, "RSA_SIGN", "RSA Signature"),
    alg(
        0xa400,
        1024,
        384,
        16384,
        0x30,
        "RSA_KEYX",
        "RSA Key Exchange",
    ),
    alg(0x8009, 0, 0, 0, 0x00, "HMAC", "Hugo's MAC (HMAC)"),
];
/// The Enhanced RSA and AES provider's.
const AES: [Algorithm; 20] = [
    alg(0x6602, 128, 40, 128, 0x00, "RC2", "RSA Data Security's RC2"),
    alg(0x6801, 128, 40, 128, 0x00, "RC4", "RSA Data Security's RC4"),
    alg(
        0x6601,
        56,
        56,
        56,
        0x00,
        "DES",
        "Data Encryption Standard (DES)",
    ),
    alg(
        0x6609,
        112,
        112,
        112,
        0x00,
        "3DES TWO KEY",
        "Two Key Triple DES",
    ),
    alg(0x6603, 168, 168, 168, 0x00, "3DES", "Three Key Triple DES"),
    alg(
        0x8004,
        160,
        160,
        160,
        0x20,
        "SHA-1",
        "Secure Hash Algorithm (SHA-1)",
    ),
    alg(
        0x800c,
        256,
        256,
        256,
        0x20,
        "SHA-256",
        "Secure Hash Algorithm 256 (SHA-256)",
    ),
    alg(
        0x800d,
        384,
        384,
        384,
        0x20,
        "SHA-384",
        "Secure Hash Algorithm 384 (SHA-384)",
    ),
    alg(
        0x800e,
        512,
        512,
        512,
        0x20,
        "SHA-512",
        "Secure Hash Algorithm 512 (SHA-512)",
    ),
    alg(0x8001, 128, 128, 128, 0x20, "MD2", "Message Digest 2 (MD2)"),
    alg(0x8002, 128, 128, 128, 0x20, "MD4", "Message Digest 4 (MD4)"),
    alg(0x8003, 128, 128, 128, 0x20, "MD5", "Message Digest 5 (MD5)"),
    alg(0x8008, 288, 288, 288, 0x00, "SSL3 SHAMD5", "SSL3 SHAMD5"),
    alg(0x8005, 0, 0, 0, 0x00, "MAC", "Message Authentication Code"),
    alg(0x2400, 1024, 384, 16384, 0x30, "RSA_SIGN", "RSA Signature"),
    alg(
        0xa400,
        1024,
        384,
        16384,
        0x30,
        "RSA_KEYX",
        "RSA Key Exchange",
    ),
    alg(0x8009, 0, 0, 0, 0x00, "HMAC", "Hugo's MAC (HMAC)"),
    alg(
        0x660e,
        128,
        128,
        128,
        0x00,
        "AES 128",
        "Advanced Encryption Standard 128-bit",
    ),
    alg(
        0x660f,
        192,
        192,
        192,
        0x00,
        "AES 192",
        "Advanced Encryption Standard 192-bit",
    ),
    alg(
        0x6610,
        256,
        256,
        256,
        0x00,
        "AES 256",
        "Advanced Encryption Standard 256-bit",
    ),
];
/// The algorithms `provider` offers (`tests/reference/cryptprobe-raw.txt`).
pub fn algorithms(provider: Provider) -> &'static [Algorithm] {
    if provider.aes {
        &AES
    } else if provider.base {
        &BASE
    } else {
        &ENHANCED
    }
}
/// A provider's key lengths in bits for a key algorithm `alg`: (default,
/// min, max), None when it does not offer it.
pub fn lengths(provider: Provider, alg: u32) -> Option<(u32, u32, u32)> {
    algorithms(provider)
        .iter()
        .find(|a| a.id == alg && a.max > 0 && alg & 0xE000 != 0x8000)
        .map(|a| (a.default, a.min, a.max))
}
/// PP_ENUMALGS_EX's record: ALG_ID, default, min and max lengths,
/// protocols, then the name (20 bytes) and long name (40), each with its
/// length.
pub fn enumalgs_ex(a: &Algorithm) -> [u8; 88] {
    let mut b = [0u8; 88];
    for (i, v) in [a.id, a.default, a.min, a.max, a.protocols]
        .into_iter()
        .enumerate()
    {
        b[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
    b[20..24].copy_from_slice(&(a.name.len() as u32 + 1).to_le_bytes());
    b[24..24 + a.name.len()].copy_from_slice(a.name.as_bytes());
    b[44..48].copy_from_slice(&(a.long_name.len() as u32 + 1).to_le_bytes());
    b[48..48 + a.long_name.len()].copy_from_slice(a.long_name.as_bytes());
    b
}
/// PP_ENUMALGS's: ALG_ID, the default length, the name (20 bytes) and its
/// length.
pub fn enumalgs(a: &Algorithm) -> [u8; 32] {
    let mut b = [0u8; 32];
    b[..4].copy_from_slice(&a.id.to_le_bytes());
    b[4..8].copy_from_slice(&a.default.to_le_bytes());
    b[8..12].copy_from_slice(&(a.name.len() as u32 + 1).to_le_bytes());
    b[12..12 + a.name.len()].copy_from_slice(a.name.as_bytes());
    b
}
/// A provider: what CryptAcquireContext opened (`kind` 24 is PROV_RSA_AES).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Provider {
    pub aes: bool,
    pub base: bool,
}

fn block_len(alg: u32) -> usize {
    match alg {
        calg::RC4 => 0,
        calg::AES_128 | calg::AES_192 | calg::AES_256 => 16,
        _ => 8,
    }
}
/// Odd parity in each byte (DES keys).
fn parity(key: &mut [u8]) {
    for b in key {
        let ones = (*b & 0xFE).count_ones();
        *b = (*b & 0xFE) | u8::from(ones % 2 == 0);
    }
}

#[derive(Clone)]
enum Engine {
    Rc2(Rc2),
    Rc4(Rc4),
    Des(Des),
    Des3(TripleDes),
    Aes128(aes::Aes128),
    Aes192(aes::Aes192),
    Aes256(aes::Aes256),
}
impl Engine {
    fn block(&self, data: &mut [u8], decrypt: bool) {
        use aes::cipher::{BlockDecrypt, BlockEncrypt, generic_array::GenericArray};
        match self {
            Self::Rc2(c) => {
                let b: [u8; 8] = data.try_into().unwrap();
                data.copy_from_slice(&if decrypt { c.decrypt(b) } else { c.encrypt(b) });
            }
            Self::Des(c) => {
                let b: [u8; 8] = data.try_into().unwrap();
                data.copy_from_slice(&if decrypt { c.decrypt(b) } else { c.encrypt(b) });
            }
            Self::Des3(c) => {
                let b: [u8; 8] = data.try_into().unwrap();
                data.copy_from_slice(&if decrypt { c.decrypt(b) } else { c.encrypt(b) });
            }
            Self::Aes128(c) => {
                let b = GenericArray::from_mut_slice(data);
                if decrypt {
                    c.decrypt_block(b)
                } else {
                    c.encrypt_block(b)
                }
            }
            Self::Aes192(c) => {
                let b = GenericArray::from_mut_slice(data);
                if decrypt {
                    c.decrypt_block(b)
                } else {
                    c.encrypt_block(b)
                }
            }
            Self::Aes256(c) => {
                let b = GenericArray::from_mut_slice(data);
                if decrypt {
                    c.decrypt_block(b)
                } else {
                    c.encrypt_block(b)
                }
            }
            Self::Rc4(_) => {}
        }
    }
}

/// A session key.
#[derive(Clone)]
pub struct Symmetric {
    pub alg: u32,
    /// The key's bytes (DES keys with their parity bits).
    pub key: Vec<u8>,
    pub salt: Vec<u8>,
    pub effective: u32,
    pub mode: u32,
    pub mode_bits: u32,
    pub padding: u32,
    pub iv: Vec<u8>,
    pub exportable: bool,
    /// An HMAC key (CRYPT_IPSEC_HMAC_KEY): any length, not for encryption.
    pub hmac_only: bool,
    /// The cipher (boxed: AES key schedules and RC4 state are large).
    engine: Option<alloc::boxed::Box<Engine>>,
    /// The chaining value (CBC, CFB) of a message in progress.
    chain: Option<Vec<u8>>,
    /// Encrypted bytes a non-final decryption held back (the last block,
    /// which may carry padding).
    held: Vec<u8>,
}
impl Symmetric {
    /// A key of `alg` with `key` bytes; 40-bit RC2 and RC4 keys get 11 zero
    /// bytes of salt unless `no_salt`.
    pub fn new(provider: Provider, alg: u32, key: &[u8], no_salt: bool, exportable: bool) -> Self {
        let mut key = key.to_vec();
        if matches!(alg, calg::DES | calg::DES3_112 | calg::DES3) {
            parity(&mut key);
        }
        let salt = if matches!(alg, calg::RC2 | calg::RC4) && key.len() == 5 && !no_salt {
            alloc::vec![0u8; 11]
        } else {
            Vec::new()
        };
        let block = block_len(alg);
        let mut k = Self {
            alg,
            // The Base provider keeps RC2's effective length at 40 bits.
            effective: if provider.base {
                40
            } else {
                (key.len() * 8) as u32
            },
            key,
            salt,
            mode: mode::CBC,
            mode_bits: 8,
            padding: 1,
            iv: alloc::vec![0u8; block],
            exportable,
            hmac_only: false,
            engine: None,
            chain: None,
            held: Vec::new(),
        };
        k.rekey();
        k
    }
    /// KP_KEYLEN in bits (DES keys count their parity bits).
    pub fn bits(&self) -> u32 {
        (self.key.len() * 8) as u32
    }
    pub fn block(&self) -> usize {
        block_len(self.alg)
    }
    pub fn permissions(&self) -> u32 {
        PERMISSIONS
            | if self.exportable {
                PERMISSION_EXPORT
            } else {
                0
            }
    }
    /// A fresh message: no chaining value, RC4 from its start.
    pub fn restart(&mut self) {
        self.rekey();
    }
    /// The cipher's key: the key's bytes and its salt.
    fn rekey(&mut self) {
        use aes::cipher::KeyInit;
        let mut full = self.key.clone();
        full.extend_from_slice(&self.salt);
        let engine = match self.alg {
            calg::RC2 if !self.hmac_only => {
                Rc2::new(&full, self.effective as usize).map(Engine::Rc2)
            }
            calg::RC4 if !self.hmac_only => Some(Engine::Rc4(Rc4::new(&full))),
            calg::DES => Some(Engine::Des(Des::new(full[..8].try_into().unwrap()))),
            calg::DES3_112 | calg::DES3 => TripleDes::new(&full).map(Engine::Des3),
            calg::AES_128 => aes::Aes128::new_from_slice(&full).ok().map(Engine::Aes128),
            calg::AES_192 => aes::Aes192::new_from_slice(&full).ok().map(Engine::Aes192),
            calg::AES_256 => aes::Aes256::new_from_slice(&full).ok().map(Engine::Aes256),
            _ => None,
        };
        self.engine = engine.map(alloc::boxed::Box::new);
        self.chain = None;
        self.held.clear();
    }
    /// KP_MODE.
    pub fn set_mode(&mut self, m: u32) -> Result<(), u32> {
        match m {
            _ if self.alg == calg::RC4 => Err(nte::BAD_TYPE),
            mode::CBC | mode::ECB | mode::CFB => {
                self.mode = m;
                self.chain = None;
                Ok(())
            }
            mode::OFB => Err(ERROR_INTERNAL_ERROR),
            mode::CTS => Err(nte::BAD_DATA),
            _ => Err(nte::BAD_DATA),
        }
    }
    /// KP_IV (a block's worth of bytes).
    pub fn set_iv(&mut self, iv: &[u8]) -> Result<(), u32> {
        let block = self.block();
        if block == 0 || iv.len() < block {
            return Err(nte::BAD_TYPE);
        }
        self.iv = iv[..block].to_vec();
        self.chain = None;
        Ok(())
    }
    /// KP_SALT (the salt's length is the key's).
    /// KP_SALT: measured on Windows 11, setting it leaves the key without
    /// salt (a 40-bit key then encrypts with its five bytes alone, and
    /// KP_SALT reads back empty); a key without salt is unchanged.
    pub fn set_salt(&mut self, _salt: &[u8]) -> Result<(), u32> {
        if !self.salt.is_empty() {
            self.salt.clear();
            self.rekey();
        }
        Ok(())
    }
    /// KP_EFFECTIVE_KEYLEN (RC2: 1 to 1024 bits).
    pub fn set_effective(&mut self, bits: u32) -> Result<(), u32> {
        if self.alg != calg::RC2 || !(1..=1024).contains(&bits) {
            return Err(nte::BAD_DATA);
        }
        self.effective = bits;
        self.rekey();
        Ok(())
    }
    fn cfb(&mut self, data: &mut [u8], decrypt: bool) {
        let block = self.block();
        let engine = self.engine.clone().unwrap();
        let mut register = self.chain.take().unwrap_or_else(|| self.iv.clone());
        for b in data.iter_mut() {
            let mut out = register.clone();
            engine.block(&mut out, false);
            let c = if decrypt { *b } else { *b ^ out[0] };
            *b ^= out[0];
            register.copy_within(1..block, 0);
            register[block - 1] = c;
        }
        self.chain = Some(register);
    }
    fn blocks(&mut self, data: &mut [u8], decrypt: bool) {
        let block = self.block();
        let engine = self.engine.clone().unwrap();
        let mut chain = self.chain.take().unwrap_or_else(|| self.iv.clone());
        for piece in data.chunks_mut(block) {
            match self.mode {
                mode::ECB => engine.block(piece, decrypt),
                _ if decrypt => {
                    let saved = piece.to_vec();
                    engine.block(piece, true);
                    for (p, c) in piece.iter_mut().zip(&chain) {
                        *p ^= c;
                    }
                    chain = saved;
                }
                _ => {
                    for (p, c) in piece.iter_mut().zip(&chain) {
                        *p ^= c;
                    }
                    engine.block(piece, false);
                    chain = piece.to_vec();
                }
            }
        }
        self.chain = Some(chain);
    }
    /// The length CryptEncrypt gives `len` bytes.
    pub fn encrypted_len(&self, len: usize, last: bool) -> usize {
        let block = self.block();
        if block == 0 || !last {
            len
        } else {
            (len / block + 1) * block
        }
    }
    /// CryptEncrypt: `data` in place (grown by padding when `last`).
    pub fn encrypt(&mut self, data: &mut Vec<u8>, last: bool) -> Result<(), u32> {
        if self.engine.is_none() {
            return Err(nte::BAD_KEY);
        }
        let block = self.block();
        if block == 0 {
            if let Some(Engine::Rc4(r)) = self.engine.as_deref_mut() {
                r.apply(data);
            }
            if last {
                self.rekey();
            }
            return Ok(());
        }
        if last {
            let pad = block - data.len() % block;
            data.extend(core::iter::repeat_n(pad as u8, pad));
        } else if !data.len().is_multiple_of(block) {
            return Err(nte::BAD_DATA);
        }
        if self.mode == mode::CFB {
            self.cfb(data, false);
        } else {
            self.blocks(data, false);
        }
        if last {
            self.chain = None;
        }
        Ok(())
    }
    /// CryptDecrypt: `data` in place (its padding removed when `last`).
    pub fn decrypt(&mut self, data: &mut Vec<u8>, last: bool) -> Result<(), u32> {
        if self.engine.is_none() {
            return Err(nte::BAD_KEY);
        }
        let block = self.block();
        if block == 0 {
            if let Some(Engine::Rc4(r)) = self.engine.as_deref_mut() {
                r.apply(data);
            }
            if last {
                self.rekey();
            }
            return Ok(());
        }
        if !data.len().is_multiple_of(block) {
            self.chain = None;
            return Err(nte::BAD_DATA);
        }
        if self.mode == mode::CFB {
            self.cfb(data, true);
        } else {
            self.blocks(data, true);
        }
        if last {
            self.chain = None;
            let pad = usize::from(*data.last().ok_or(nte::BAD_DATA)?);
            if pad == 0
                || pad > block
                || pad > data.len()
                || data[data.len() - pad..]
                    .iter()
                    .any(|&b| usize::from(b) != pad)
            {
                return Err(nte::BAD_DATA);
            }
            data.truncate(data.len() - pad);
        }
        Ok(())
    }
    /// The PLAINTEXTKEYBLOB body: the key's length and bytes.
    pub fn plaintext(&self) -> Vec<u8> {
        let mut b = (self.key.len() as u32).to_le_bytes().to_vec();
        b.extend_from_slice(&self.key);
        b
    }
}

/// An RSA key: a pair, or the public half only.
#[derive(Clone)]
pub struct Asymmetric {
    pub alg: u32,
    pub public: PublicKey,
    /// The private half (boxed: its numbers are 3.5 KB).
    pub private: Option<alloc::boxed::Box<PrivateKey>>,
    pub exportable: bool,
}
impl Asymmetric {
    pub fn permissions(&self) -> u32 {
        PERMISSIONS
            | if self.exportable {
                PERMISSION_EXPORT
            } else {
                0
            }
    }
}

#[derive(Clone)]
pub enum Key {
    Symmetric(Symmetric),
    /// Boxed: the modulus alone is 512 bytes.
    Asymmetric(alloc::boxed::Box<Asymmetric>),
}
impl Key {
    pub fn alg(&self) -> u32 {
        match self {
            Self::Symmetric(k) => k.alg,
            Self::Asymmetric(k) => k.alg,
        }
    }
    /// CryptGetKeyParam's DWORD parameters.
    pub fn dword(&self, param: u32) -> Result<u32, u32> {
        match (self, param) {
            (_, kp::ALGID) => Ok(self.alg()),
            (Self::Symmetric(k), kp::KEYLEN) => Ok(k.bits()),
            (Self::Symmetric(k), kp::BLOCKLEN) => Ok((k.block() * 8) as u32),
            (Self::Symmetric(k), kp::PERMISSIONS) => Ok(k.permissions()),
            (Self::Symmetric(k), kp::EFFECTIVE_KEYLEN) if k.alg == calg::RC2 => Ok(k.effective),
            (Self::Symmetric(k), kp::MODE) if k.block() > 0 => Ok(k.mode),
            (Self::Symmetric(k), kp::MODE_BITS) if k.block() > 0 => Ok(k.mode_bits),
            (Self::Symmetric(k), kp::PADDING) if k.block() > 0 => Ok(k.padding),
            (Self::Asymmetric(k), kp::KEYLEN | kp::BLOCKLEN) => Ok(k.public.bits as u32),
            (Self::Asymmetric(k), kp::PERMISSIONS) => Ok(k.permissions()),
            _ => Err(nte::BAD_TYPE),
        }
    }
    /// CryptGetKeyParam's byte parameters.
    pub fn bytes(&self, param: u32) -> Result<Vec<u8>, u32> {
        match (self, param) {
            (Self::Symmetric(k), kp::IV) if k.block() > 0 => Ok(k.iv.clone()),
            (Self::Symmetric(k), kp::SALT) => Ok(k.salt.clone()),
            _ => self.dword(param).map(|v| v.to_le_bytes().to_vec()),
        }
    }
}

/// The key's bytes CryptDeriveKey takes from a finished hash `value` of
/// `hash` for `alg` and `bytes`: the hash's first bytes, or for three-key
/// triple DES and AES from a hash older than SHA-2 the expansion Windows
/// documents (the hash XORed into 0x36 and 0x5C pads, each hashed again).
pub fn derived_bytes(hash: u32, value: &[u8], alg: u32, bytes: usize) -> Vec<u8> {
    let sha2 = matches!(hash, 0x800C..=0x800E);
    let expand = !sha2
        && matches!(
            alg,
            calg::DES3 | calg::AES_128 | calg::AES_192 | calg::AES_256
        );
    if !expand {
        return value[..bytes.min(value.len())].to_vec();
    }
    let digest = |data: &[u8]| -> Vec<u8> {
        let mut h = super::Hash::new(super::PROV_RSA_AES, hash).unwrap();
        let _ = h.update(data);
        h.value()
    };
    let mut out = Vec::new();
    for pad in [0x36u8, 0x5C] {
        let mut buffer = [pad; 64];
        for (b, v) in buffer.iter_mut().zip(value) {
            *b ^= v;
        }
        out.extend(digest(&buffer));
    }
    out.truncate(bytes);
    out
}

/// The key length (bits) a CryptGenKey or CryptDeriveKey of `alg` with
/// `flags` makes on `provider`: the flags' upper word, else the default.
pub fn key_bits(provider: Provider, alg: u32, flags: u32) -> Result<u32, u32> {
    let (default, min, max) = lengths(provider, alg).ok_or(nte::BAD_ALGID)?;
    let asked = flags >> 16;
    if asked == 0 {
        return Ok(default);
    }
    if asked < min || asked > max {
        return Err(nte::BAD_FLAGS);
    }
    // Fixed-size ciphers ignore what is asked.
    Ok(if min == max { default } else { asked })
}
/// Bytes of a key of `alg` and `bits` (DES-family sizes include parity).
pub fn key_bytes(alg: u32, bits: u32) -> usize {
    match alg {
        calg::DES => 8,
        calg::DES3_112 => 16,
        calg::DES3 => 24,
        _ => (bits / 8) as usize,
    }
}

/// CryptImportKey of a PLAINTEXTKEYBLOB body (`[len u32][bytes]`).
pub fn import_plaintext(
    provider: Provider,
    alg: u32,
    body: &[u8],
    flags: u32,
) -> Result<Symmetric, u32> {
    let len = body
        .get(..4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or(nte::BAD_DATA)? as usize;
    let key = body.get(4..4 + len).ok_or(nte::BAD_DATA)?;
    let hmac = flags & flag::IPSEC_HMAC_KEY != 0;
    let (_, min, max) = lengths(provider, alg).ok_or(nte::BAD_ALGID)?;
    if !hmac {
        let bits = match alg {
            calg::DES => 56,
            calg::DES3_112 => 112,
            calg::DES3 => 168,
            _ => (len * 8) as u32,
        };
        if bits < min || bits > max || len != key_bytes(alg, bits) {
            return Err(nte::BAD_LEN);
        }
    }
    let mut k = Symmetric::new(
        provider,
        alg,
        key,
        flags & flag::NO_SALT != 0 || hmac,
        flags & flag::EXPORTABLE != 0,
    );
    if hmac {
        k.hmac_only = true;
        k.engine = None;
    }
    Ok(k)
}
/// A SIMPLEBLOB body (`[ALG_ID of the exchange key][encrypted key]`)
/// decrypted with `exchange`.
pub fn import_simple(
    provider: Provider,
    alg: u32,
    body: &[u8],
    exchange: &Asymmetric,
    flags: u32,
) -> Result<Symmetric, u32> {
    let private = exchange.private.as_ref().ok_or(nte::NO_KEY)?;
    let encrypted = body.get(4..).ok_or(nte::BAD_DATA)?;
    let key = rsa::decrypt(private, encrypted, false).ok_or(nte::BAD_DATA)?;
    let mut plain = (key.len() as u32).to_le_bytes().to_vec();
    plain.extend_from_slice(&key);
    import_plaintext(provider, alg, &plain, flags & !flag::IPSEC_HMAC_KEY)
}

/// CryptEncrypt with an RSA key: `data` replaced by its encryption.
pub fn rsa_encrypt(
    key: &Asymmetric,
    data: &mut Vec<u8>,
    oaep: bool,
    random: &mut dyn FnMut(&mut [u8]),
) -> Result<(), u32> {
    let limit = key.public.bytes() - if oaep { 42 } else { 11 };
    if data.len() > limit {
        return Err(nte::BAD_LEN);
    }
    *data = rsa::encrypt(&key.public, data, oaep, random).ok_or(nte::BAD_LEN)?;
    Ok(())
}
/// CryptDecrypt with an RSA key.
pub fn rsa_decrypt(key: &Asymmetric, data: &mut Vec<u8>, oaep: bool) -> Result<(), u32> {
    let private = key.private.as_ref().ok_or(nte::NO_KEY)?;
    if data.len() != key.public.bytes() {
        return Err(nte::BAD_DATA);
    }
    *data = rsa::decrypt(private, data, oaep).ok_or(ERROR_GEN_FAILURE)?;
    Ok(())
}
/// ERROR_MORE_DATA, for the guest's buffer checks.
pub const MORE_DATA: u32 = ERROR_MORE_DATA;

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::{String, ToString};
    use alloc::vec::Vec;
    const RAW: &str = include_str!("../../tests/reference/cryptprobe-raw.txt");
    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
    fn tohex(b: &[u8]) -> String {
        b.iter().map(|x| alloc::format!("{x:02x}")).collect()
    }
    fn field<'a>(line: &'a str, name: &str) -> Option<&'a str> {
        line.split(' ')
            .find_map(|w| w.strip_prefix(name).and_then(|r| r.strip_prefix('=')))
    }
    const KEY: [u8; 32] = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32,
        0x10, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
        0xee, 0xff,
    ];
    const TEXT: &[u8] = b"Doors CryptoAPI test";
    fn sym(name: &str) -> (u32, usize) {
        match name {
            "rc2-40" => (calg::RC2, 5),
            "rc2-128" => (calg::RC2, 16),
            "rc4-40" => (calg::RC4, 5),
            "rc4-128" => (calg::RC4, 16),
            "des" => (calg::DES, 8),
            "3des-112" => (calg::DES3_112, 16),
            "3des" => (calg::DES3, 24),
            "aes-128" => (calg::AES_128, 16),
            "aes-192" => (calg::AES_192, 24),
            "aes-256" => (calg::AES_256, 32),
            _ => panic!("{name}"),
        }
    }
    fn provider(label: &str) -> Provider {
        match label {
            "base" => Provider {
                aes: false,
                base: true,
            },
            "aes" => Provider {
                aes: true,
                base: false,
            },
            _ => Provider {
                aes: false,
                base: false,
            },
        }
    }
    fn section(name: &str) -> impl Iterator<Item = &'static str> {
        let start = RAW.find(&alloc::format!("== {name}\r\n")).unwrap();
        RAW[start..]
            .lines()
            .skip(1)
            .take_while(|l| !l.starts_with("== "))
    }
    fn code(e: &str) -> u32 {
        u32::from_str_radix(&e[2..], 16).unwrap()
    }
    #[test]
    fn imported_keys_encrypt_as_windows_does() {
        let mut seen = 0;
        for line in section("import") {
            let words: Vec<&str> = line.split(' ').collect();
            let (alg, len) = sym(words[1]);
            let mut body = (len as u32).to_le_bytes().to_vec();
            body.extend_from_slice(&KEY[..len]);
            let k = import_plaintext(provider(words[0]), alg, &body, 0);
            match field(line, "error") {
                Some(e) => assert_eq!(k.err(), Some(code(e)), "{line}"),
                None => {
                    let mut k = k.unwrap();
                    let key = Key::Symmetric(k.clone());
                    assert_eq!(
                        key.dword(kp::KEYLEN).unwrap().to_string(),
                        field(line, "len").unwrap(),
                        "{line}"
                    );
                    assert_eq!(key.dword(kp::PERMISSIONS).unwrap(), 59);
                    let salt = field(line, "salt").unwrap();
                    assert_eq!(
                        alloc::format!("{}:{}", k.salt.len(), tohex(&k.salt)),
                        salt,
                        "{line}"
                    );
                    let mut data = TEXT.to_vec();
                    k.encrypt(&mut data, true).unwrap();
                    assert_eq!(tohex(&data), field(line, "enc").unwrap(), "{line}");
                    k.decrypt(&mut data, true).unwrap();
                    assert_eq!(data, TEXT);
                    seen += 1;
                }
            }
        }
        assert_eq!(seen, 20);
    }
    #[test]
    fn modes_match_windows() {
        let iv: Vec<u8> = (0xa0..=0xafu8).collect();
        for line in section("modes").filter(|l| l.contains(" set ")) {
            let words: Vec<&str> = line.split(' ').collect();
            let (alg, len) = sym(words[0]);
            let m = match words[1] {
                "ecb" => mode::ECB,
                "cbc" => mode::CBC,
                "cfb" => mode::CFB,
                "ofb" => mode::OFB,
                _ => mode::CTS,
            };
            let mut body = (len as u32).to_le_bytes().to_vec();
            body.extend_from_slice(&KEY[..len]);
            let mut k = import_plaintext(provider("aes"), alg, &body, 0).unwrap();
            match k.set_mode(m) {
                Err(e) => assert_eq!(Some(e), field(line, "error").map(code), "{line}"),
                Ok(()) => {
                    k.set_iv(&iv).unwrap();
                    let mut data = TEXT.to_vec();
                    k.encrypt(&mut data, true).unwrap();
                    assert_eq!(tohex(&data), field(line, "enc").unwrap(), "{line}");
                    k.decrypt(&mut data, true).unwrap();
                    assert_eq!(data, TEXT);
                }
            }
        }
    }
    #[test]
    fn derived_keys_match_windows() {
        let mut seen = 0;
        for line in section("derive").filter(|l| !l.starts_with("after")) {
            let words: Vec<&str> = line.split(' ').collect();
            let p = provider(words[0]);
            let hash = match words[1] {
                "md5" => 0x8003,
                "sha1" => 0x8004,
                _ => 0x800C,
            };
            let (name, length) = words[2].split_once('/').unwrap();
            let alg = match name {
                "rc4" => calg::RC4,
                "rc2" => calg::RC2,
                "des" => calg::DES,
                "3des-112" => calg::DES3_112,
                "3des" => calg::DES3,
                "aes-128" => calg::AES_128,
                _ => calg::AES_256,
            };
            let bits = key_bits(p, alg, length.parse::<u32>().unwrap() << 16);
            if let Some(e) = field(line, "error") {
                assert_eq!(bits.err(), Some(code(e)), "{line}");
                continue;
            }
            let bits = bits.unwrap();
            let mut h = super::super::Hash::new(super::super::PROV_RSA_AES, hash).unwrap();
            h.update(b"password").unwrap();
            let value = h.value();
            let key = derived_bytes(hash, &value, alg, key_bytes(alg, bits));
            let mut k = Symmetric::new(p, alg, &key, false, true);
            assert_eq!(tohex(&k.key), field(line, "key").unwrap(), "{line}");
            let mut data = TEXT.to_vec();
            k.encrypt(&mut data, true).unwrap();
            assert_eq!(tohex(&data), field(line, "enc").unwrap(), "{line}");
            seen += 1;
        }
        assert!(seen > 60, "{seen}");
    }
    #[test]
    fn rsa_signatures_match_windows() {
        let fixture =
            include_str!("../../../../../tests/fixtures/windows-security/cryptprobe.fixture.h");
        let hexes: String = fixture
            .split("private_hex[] =")
            .nth(1)
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .chars()
            .filter(char::is_ascii_hexdigit)
            .collect();
        let key = super::super::rsa::PrivateKey::from_blob(&hex(&hexes)).unwrap();
        for line in section("rsa").filter(|l| l.starts_with("sign ")) {
            let alg = match line.split(' ').nth(1).unwrap() {
                "md5" => 0x8003,
                "sha1" => 0x8004,
                "sha256" => 0x800C,
                _ => 0x800E,
            };
            let mut h = super::super::Hash::new(super::super::PROV_RSA_AES, alg).unwrap();
            h.update(b"Doors").unwrap();
            let value = h.value();
            let sig = super::super::rsa::sign(&key, alg, &value, false).unwrap();
            assert_eq!(tohex(&sig), field(line, "sig").unwrap(), "{line}");
            if let Some(nooid) = field(line, "nooid") {
                assert_eq!(
                    tohex(&super::super::rsa::sign(&key, alg, &value, true).unwrap()),
                    nooid
                );
            }
        }
        let public = section("rsa").find(|l| l.starts_with("public n=")).unwrap();
        let blob = field(public, "blob").unwrap();
        assert_eq!(tohex(&key.public.blob()), blob[16..]);
    }
}

#[cfg(test)]
mod more_tests {
    use super::super::{Hash, PROV_RSA_AES, alg};
    use super::*;
    use alloc::string::String;
    use alloc::vec::Vec;
    const RAW: &str = include_str!("../../tests/reference/cryptprobe-raw.txt");
    fn tohex(b: &[u8]) -> String {
        b.iter().map(|x| alloc::format!("{x:02x}")).collect()
    }
    fn line(prefix: &str) -> &'static str {
        RAW.lines().find(|l| l.starts_with(prefix)).unwrap()
    }
    fn field<'a>(line: &'a str, name: &str) -> &'a str {
        line.split(' ')
            .find_map(|w| w.strip_prefix(name).and_then(|r| r.strip_prefix('=')))
            .unwrap()
    }
    const KEY: [u8; 24] = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32,
        0x10, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
    ];
    const AES: Provider = Provider {
        aes: true,
        base: false,
    };
    const ENHANCED: Provider = Provider {
        aes: false,
        base: false,
    };
    fn plain(p: Provider, alg: u32, len: usize, flags: u32) -> Symmetric {
        let mut body = (len as u32).to_le_bytes().to_vec();
        body.extend_from_slice(&KEY[..len]);
        import_plaintext(p, alg, &body, flags).unwrap()
    }
    #[test]
    fn hmac_matches_windows_and_rfc_4231() {
        for (name, a) in [
            ("md5", alg::MD5),
            ("sha1", alg::SHA1),
            ("sha256", alg::SHA_256),
            ("sha384", alg::SHA_384),
            ("sha512", alg::SHA_512),
        ] {
            let mut body = 4u32.to_le_bytes().to_vec();
            body.extend_from_slice(b"Jefe");
            let k = import_plaintext(AES, calg::RC2, &body, flag::IPSEC_HMAC_KEY).unwrap();
            let mut h = Hash::hmac(&k.key);
            h.set_hmac_info(PROV_RSA_AES, a).unwrap();
            h.update(b"what do ya want for nothing?").unwrap();
            let l = line(&alloc::format!("hmac {name} "));
            assert_eq!(tohex(&h.value()), field(l, "value"), "{name}");
        }
        // Four bytes are too short without CRYPT_IPSEC_HMAC_KEY.
        let mut body = 4u32.to_le_bytes().to_vec();
        body.extend_from_slice(b"Jefe");
        assert_eq!(
            import_plaintext(AES, calg::RC2, &body, 0).err(),
            Some(nte::BAD_LEN)
        );
    }
    #[test]
    fn mac_and_ssl3_match_windows() {
        for (name, a, len) in [
            ("des", calg::DES, 8),
            ("3des", calg::DES3, 24),
            ("aes-128", calg::AES_128, 16),
        ] {
            let k = plain(AES, a, len, 0);
            let mut h = Hash::mac(&k).unwrap();
            h.update(b"Doors CryptoAPI test").unwrap();
            assert_eq!(
                tohex(&h.value()),
                field(line(&alloc::format!("mac {name} ")), "value")
            );
        }
        let rc4 = {
            let mut body = 16u32.to_le_bytes().to_vec();
            body.extend_from_slice(&KEY[..16]);
            import_plaintext(AES, calg::RC4, &body, 0).unwrap()
        };
        assert_eq!(Hash::mac(&rc4).err(), Some(nte::BAD_KEY));
        let mut s = Hash::new(PROV_RSA_AES, alg::SSL3_SHAMD5).unwrap();
        assert_eq!(s.size(), 36);
        assert_eq!(s.update(b"x"), Err(6));
        assert_eq!(s.try_value(), Err(nte::BAD_HASH_STATE));
    }
    #[test]
    fn extras_match_windows() {
        let l = line("session-hash ");
        let k = plain(AES, calg::AES_128, 16, 0);
        let mut forward = k.key.clone();
        let mut h = Hash::new(PROV_RSA_AES, alg::SHA1).unwrap();
        forward.reverse();
        h.update(&forward).unwrap();
        let values: Vec<&str> = l
            .split(' ')
            .filter_map(|w| w.strip_prefix("value="))
            .collect();
        assert_eq!(tohex(&h.value()), values[0]);
        let mut h = Hash::new(PROV_RSA_AES, alg::SHA1).unwrap();
        h.update(&k.key).unwrap();
        assert_eq!(tohex(&h.value()), values[1]);
        // A salt set on a 40-bit RC4 key.
        let salt: Vec<u8> = (1..=11).collect();
        let mut k = plain(ENHANCED, calg::RC4, 5, 0);
        k.set_salt(&salt).unwrap();
        let mut data = b"Doors CryptoAPI test".to_vec();
        k.encrypt(&mut data, true).unwrap();
        assert_eq!(tohex(&data), field(line("salt set "), "enc"));
        assert_eq!(Key::Symmetric(k).bytes(kp::SALT).unwrap(), Vec::<u8>::new());
        // RC2's effective length.
        let mut k = plain(ENHANCED, calg::RC2, 16, 0);
        k.set_effective(64).unwrap();
        let mut data = b"Doors CryptoAPI test".to_vec();
        k.encrypt(&mut data, true).unwrap();
        assert_eq!(tohex(&data), field(line("effective set "), "enc"));
        assert_eq!(k.set_effective(2000), Err(nte::BAD_DATA));
    }
}

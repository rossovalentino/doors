//! Archive codecs (P15): deflate and inflate (miniz_oxide), ZIP entries
//! with WinZip AES-256 encryption (AE-2: PBKDF2-HMAC-SHA1 keys, AES-256 in
//! the little-endian counter mode WinZip defines, HMAC-SHA1 authentication),
//! TAR headers and gzip members. Pure functions over byte slices: File
//! Explorer's archive jobs and Preview's converter do the storage I/O.
extern crate alloc;
use aes::cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
use alloc::{string::String, vec::Vec};
use hmac::{Hmac, Mac};
use sha1::Sha1;

/// ZIP compression methods Doors writes and reads.
pub const STORED: u16 = 0;
pub const DEFLATE: u16 = 8;
/// Method 99: WinZip AES (the real method is in the 0x9901 extra field).
pub const AES: u16 = 99;
/// Item method for 7-Zip entries (`offset` holds the entry index).
const SEVEN_ZIP: u16 = 0xFF07;
/// Item method for RAR 5 entries (`offset` holds the entry index).
const RAR: u16 = 0xFF05;
/// PBKDF2 iterations WinZip AES uses.
const ITERATIONS: u32 = 1000;
/// Bytes of the HMAC-SHA1 authentication code stored after the data.
pub const AUTH_BYTES: usize = 10;
pub const SALT_BYTES: usize = 16;

/// Deflate `data` at `level` (0..=10; 6 is ZIP's "normal").
pub fn deflate(data: &[u8], level: u8) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec(data, level.min(10))
}
/// Inflate raw deflate `data`, refusing output past `limit` bytes.
pub fn inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, &'static str> {
    miniz_oxide::inflate::decompress_to_vec_with_limit(data, limit).map_err(|e| match e.status {
        miniz_oxide::inflate::TINFLStatus::HasMoreOutput => "The entry is larger than Doors allows",
        _ => "The compressed data is damaged",
    })
}

/// WinZip AES-256 keys from `password` and `salt`: (encryption key,
/// authentication key, password verifier).
fn keys(password: &str, salt: &[u8; SALT_BYTES]) -> ([u8; 32], [u8; 32], [u8; 2]) {
    let mut derived = [0u8; 66];
    pbkdf2::pbkdf2_hmac::<Sha1>(password.as_bytes(), salt, ITERATIONS, &mut derived);
    let mut enc = [0u8; 32];
    let mut auth = [0u8; 32];
    enc.copy_from_slice(&derived[..32]);
    auth.copy_from_slice(&derived[32..64]);
    (enc, auth, [derived[64], derived[65]])
}
/// AES-256 in WinZip's counter mode: a little-endian 128-bit counter from 1,
/// XORed over the data (the same operation encrypts and decrypts).
fn ctr(key: &[u8; 32], data: &mut [u8]) {
    let cipher = aes::Aes256::new(GenericArray::from_slice(key));
    let mut counter: u128 = 1;
    for chunk in data.chunks_mut(16) {
        let mut block = GenericArray::clone_from_slice(&counter.to_le_bytes());
        cipher.encrypt_block(&mut block);
        for (b, k) in chunk.iter_mut().zip(block.iter()) {
            *b ^= k;
        }
        counter = counter.wrapping_add(1);
    }
}
fn auth_code(key: &[u8; 32], data: &[u8]) -> [u8; AUTH_BYTES] {
    let mut mac = <Hmac<Sha1> as Mac>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(data);
    let full = mac.finalize().into_bytes();
    let mut out = [0u8; AUTH_BYTES];
    out.copy_from_slice(&full[..AUTH_BYTES]);
    out
}
/// An entry's stored bytes under WinZip AES-256: salt, verifier, the
/// encrypted (already compressed) data, then the authentication code.
pub fn encrypt(password: &str, salt: [u8; SALT_BYTES], compressed: &[u8]) -> Vec<u8> {
    let (enc, auth, verifier) = keys(password, &salt);
    let mut data = Vec::with_capacity(SALT_BYTES + 2 + compressed.len() + AUTH_BYTES);
    data.extend_from_slice(&salt);
    data.extend_from_slice(&verifier);
    let start = data.len();
    data.extend_from_slice(compressed);
    ctr(&enc, &mut data[start..]);
    let code = auth_code(&auth, &data[start..]);
    data.extend_from_slice(&code);
    data
}
/// Why decrypting an entry failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The password is wrong (verifier mismatch).
    Password,
    /// The data was changed or damaged (authentication code mismatch).
    Damaged,
}
/// The compressed bytes of a WinZip AES-256 entry (`stored` as `encrypt`
/// wrote them).
pub fn decrypt(password: &str, stored: &[u8]) -> Result<Vec<u8>, Refused> {
    if stored.len() < SALT_BYTES + 2 + AUTH_BYTES {
        return Err(Refused::Damaged);
    }
    let mut salt = [0u8; SALT_BYTES];
    salt.copy_from_slice(&stored[..SALT_BYTES]);
    let (enc, auth, verifier) = keys(password, &salt);
    if stored[SALT_BYTES..SALT_BYTES + 2] != verifier {
        return Err(Refused::Password);
    }
    let body = &stored[SALT_BYTES + 2..stored.len() - AUTH_BYTES];
    if auth_code(&auth, body) != stored[stored.len() - AUTH_BYTES..] {
        return Err(Refused::Damaged);
    }
    let mut out = body.to_vec();
    ctr(&enc, &mut out);
    Ok(out)
}
/// The 0x9901 extra field of a WinZip AES-256 (AE-2) entry whose real
/// method is `method`.
pub fn aes_extra(method: u16) -> [u8; 11] {
    let mut e = [0u8; 11];
    e[..2].copy_from_slice(&0x9901u16.to_le_bytes());
    e[2..4].copy_from_slice(&7u16.to_le_bytes());
    e[4..6].copy_from_slice(&2u16.to_le_bytes());
    e[6..8].copy_from_slice(b"AE");
    e[8] = 3;
    e[9..11].copy_from_slice(&method.to_le_bytes());
    e
}
/// The real method and strength of an entry's 0x9901 field in `extra`.
pub fn aes_info(extra: &[u8]) -> Option<(u16, u8)> {
    let mut at = 0;
    while at + 4 <= extra.len() {
        let id = u16::from_le_bytes([extra[at], extra[at + 1]]);
        let len = usize::from(u16::from_le_bytes([extra[at + 2], extra[at + 3]]));
        if id == 0x9901 && len >= 7 && at + 4 + 7 <= extra.len() {
            let f = &extra[at + 4..];
            return (&f[2..4] == b"AE").then(|| (u16::from_le_bytes([f[5], f[6]]), f[4]));
        }
        at += 4 + len;
    }
    None
}

/// A gzip member around raw deflate data (no name; mtime 0).
pub fn gzip(name: Option<&str>, data: &[u8], level: u8) -> Vec<u8> {
    let mut out = alloc::vec![
        0x1F,
        0x8B,
        8,
        if name.is_some() { 8 } else { 0 },
        0,
        0,
        0,
        0,
        0,
        3
    ];
    if let Some(name) = name {
        out.extend(name.bytes().filter(|b| *b != 0));
        out.push(0);
    }
    out.extend(deflate(data, level));
    out.extend(crc32(0, data).to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out
}
/// The data of a gzip member (header flags honoured, CRC checked).
pub fn gunzip(member: &[u8], limit: usize) -> Result<(Option<String>, Vec<u8>), &'static str> {
    if member.len() < 18 || member[..3] != [0x1F, 0x8B, 8] {
        return Err("This isn't a gzip file");
    }
    let flags = member[3];
    let mut at = 10;
    if flags & 4 != 0 {
        let x = usize::from(u16::from_le_bytes([member[10], member[11]]));
        at += 2 + x;
    }
    let mut name = None;
    for flag in [8u8, 16] {
        if flags & flag != 0 {
            let end = member
                .get(at..)
                .and_then(|r| r.iter().position(|c| *c == 0))
                .ok_or("The gzip header is damaged")?;
            if flag == 8 {
                name = Some(
                    member[at..at + end]
                        .iter()
                        .map(|c| char::from(*c))
                        .collect(),
                );
            }
            at += end + 1;
        }
    }
    if flags & 2 != 0 {
        at += 2;
    }
    let body = member
        .get(at..member.len() - 8)
        .ok_or("The gzip file is damaged")?;
    let data = inflate(body, limit)?;
    let tail = &member[member.len() - 8..];
    if u32::from_le_bytes([tail[0], tail[1], tail[2], tail[3]]) != crc32(0, &data) {
        return Err("The gzip file is damaged (checksum)");
    }
    Ok((name, data))
}

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}
static CRC: [u32; 256] = crc_table();
/// Running CRC-32 (IEEE, as ZIP, gzip and PNG use it).
pub fn crc32(mut crc: u32, bytes: &[u8]) -> u32 {
    crc = !crc;
    for b in bytes {
        crc = CRC[((crc ^ u32::from(*b)) & 0xff) as usize] ^ (crc >> 8);
    }
    !crc
}

/// A 512-byte POSIX (ustar) header for `name` (a folder ends in '/').
pub fn tar_header(name: &str, size: u64, mtime: u64, folder: bool) -> Option<[u8; 512]> {
    let mut h = [0u8; 512];
    let (prefix, base) = if name.len() <= 100 {
        ("", name)
    } else {
        let split = name[..name.len().min(155)].rfind('/')?;
        (&name[..split], &name[split + 1..])
    };
    if base.len() > 100 || prefix.len() > 155 {
        return None;
    }
    h[..base.len()].copy_from_slice(base.as_bytes());
    let octal = |field: &mut [u8], value: u64| {
        let digits = alloc::format!("{:0width$o}", value, width = field.len() - 1);
        let n = field.len() - 1;
        field[..n].copy_from_slice(&digits.as_bytes()[digits.len() - n..]);
    };
    octal(&mut h[100..108], if folder { 0o755 } else { 0o644 });
    octal(&mut h[108..116], 0);
    octal(&mut h[116..124], 0);
    octal(&mut h[124..136], if folder { 0 } else { size });
    octal(&mut h[136..148], mtime);
    h[156] = if folder { b'5' } else { b'0' };
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|b| u32::from(*b)).sum();
    let s = alloc::format!("{sum:06o}\0 ");
    h[148..156].copy_from_slice(s.as_bytes());
    Some(h)
}

/// One entry of an opened archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// A safe relative path ('/'-separated, no `..`, no drive or root).
    pub path: String,
    pub folder: bool,
    pub size: u64,
    pub encrypted: bool,
    method: u16,
    /// Where its stored bytes start in `Archive::data`, and how many.
    offset: usize,
    packed: usize,
    crc: u32,
}
/// An archive read whole: ZIP (its own bytes), TAR (its bytes, or a .tar.gz
/// inflated), or a single gzip member's content.
pub struct Archive {
    data: Vec<u8>,
    pub items: Vec<Item>,
    /// Entries left out (unsafe paths, links, devices).
    pub skipped: usize,
    /// 7-Zip: the password the archive was opened with (encrypted
    /// headers) and the last decoded solid folder (index, bytes).
    seven_password: Option<String>,
    folder: core::cell::RefCell<Option<(usize, Vec<u8>)>>,
    /// RAR: every entry decoded once (solid runs decode in order).
    rar: core::cell::RefCell<Option<Vec<Option<Vec<u8>>>>>,
}
/// Why an entry could not be extracted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Encrypted, and no password or the wrong one.
    Password,
    Damaged,
    /// A compression method Doors doesn't read (LZMA, BZIP2, ...).
    Method,
    TooLarge,
}
/// `raw` as a safe relative path, or None (absolute, drive, `..`).
pub fn safe_path(raw: &str) -> Option<String> {
    let mut parts = Vec::new();
    for part in raw.split(['/', '\\']) {
        match part {
            "" | "." => (),
            ".." => return None,
            p if p.contains(':') || p.bytes().any(|b| b < 0x20) => return None,
            // Windows reserved device names and names ending in a dot or
            // space cannot be files on C:.
            p if reserved(p) || p.ends_with(['.', ' ']) => return None,
            p => parts.push(p),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}
/// CON, PRN, AUX, NUL, COM1-9, LPT1-9 (with any extension).
fn reserved(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or("").trim_end();
    ["con", "prn", "aux", "nul"]
        .iter()
        .any(|d| stem.eq_ignore_ascii_case(d))
        || stem.len() == 4
            && (stem[..3].eq_ignore_ascii_case("com") || stem[..3].eq_ignore_ascii_case("lpt"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9')
}
fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
impl Archive {
    /// Open `bytes` of the file `name`; `limit` bounds what a .tar.gz or
    /// .gz may inflate to.
    pub fn open(name: &str, bytes: Vec<u8>, limit: usize) -> Result<Self, &'static str> {
        Self::open_with(name, bytes, limit, None)
    }
    /// `open`, with the password a 7-Zip archive with encrypted file names
    /// needs to be listed.
    pub fn open_with(
        name: &str,
        bytes: Vec<u8>,
        limit: usize,
        password: Option<&str>,
    ) -> Result<Self, &'static str> {
        if bytes.starts_with(&crate::sevenzip::SIGNATURE) {
            return Self::seven(bytes, limit, password);
        }
        if bytes.starts_with(&crate::rar5::SIGNATURE)
            || bytes.starts_with(&crate::rar5::SIGNATURE_RAR4)
        {
            return Self::rar(bytes);
        }
        let lower = name.to_ascii_lowercase();
        if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") {
            return Self::zip(bytes);
        }
        if bytes.starts_with(&[0x1F, 0x8B]) {
            let (inner, data) = gunzip(&bytes, limit)?;
            let tarred = lower.ends_with(".tgz")
                || lower.ends_with(".tar.gz")
                || inner
                    .as_deref()
                    .is_some_and(|n| n.to_ascii_lowercase().ends_with(".tar"));
            if tarred {
                return Self::tar(data);
            }
            let file = lower.rsplit(['/', '\\']).next().unwrap_or("file");
            let path = inner
                .as_deref()
                .and_then(safe_path)
                .unwrap_or_else(|| String::from(file.strip_suffix(".gz").unwrap_or(file)));
            let size = data.len();
            return Ok(Self {
                items: alloc::vec![Item {
                    path,
                    folder: false,
                    size: size as u64,
                    encrypted: false,
                    method: STORED,
                    offset: 0,
                    packed: size,
                    crc: crc32(0, &data),
                }],
                data,
                skipped: 0,
                seven_password: None,
                folder: core::cell::RefCell::new(None),
                rar: core::cell::RefCell::new(None),
            });
        }
        if bytes.len() >= 512 && (&bytes[257..262] == b"ustar" || lower.ends_with(".tar")) {
            return Self::tar(bytes);
        }
        Err("This file isn't a ZIP, 7Z, TAR or gzip archive")
    }
    fn rar(data: Vec<u8>) -> Result<Self, &'static str> {
        use crate::rar5::{Archive as Rar, Fault as F};
        let archive = Rar::open(&data).map_err(|f| match f {
            F::Password => "This archive's file names are encrypted, which Doors can't list yet",
            F::Unsupported => "RAR 4 and multi-volume RAR archives can't be read yet",
            F::TooLarge => "This archive is larger than Doors allows",
            F::Malformed => "The archive is damaged",
        })?;
        let mut items = Vec::new();
        let mut skipped = 0;
        for (i, e) in archive.entries.iter().enumerate() {
            let Some(path) = safe_path(&e.path) else {
                skipped += 1;
                continue;
            };
            items.push(Item {
                path,
                folder: e.folder,
                size: e.size,
                encrypted: e.encrypted,
                method: RAR,
                offset: i,
                packed: 0,
                crc: 0,
            });
        }
        Ok(Self {
            data,
            items,
            skipped,
            seven_password: None,
            folder: core::cell::RefCell::new(None),
            rar: core::cell::RefCell::new(None),
        })
    }
    fn seven(data: Vec<u8>, limit: usize, password: Option<&str>) -> Result<Self, &'static str> {
        use crate::sevenzip::{Archive as Seven, Fault as F};
        let archive = Seven::open(&data, password, limit).map_err(|f| match f {
            F::Password => "This archive's file names are encrypted. Enter its password.",
            F::Unsupported => "This 7-Zip archive uses a compression method Doors doesn't read",
            F::TooLarge => "This archive is larger than Doors allows",
            F::Malformed => "The archive is damaged",
        })?;
        let mut items = Vec::new();
        let mut skipped = 0;
        for (i, e) in archive.entries.iter().enumerate() {
            let Some(path) = safe_path(&e.path) else {
                skipped += 1;
                continue;
            };
            items.push(Item {
                path,
                folder: e.folder,
                size: e.size,
                encrypted: e.encrypted,
                method: SEVEN_ZIP,
                offset: i,
                packed: 0,
                crc: 0,
            });
        }
        Ok(Self {
            data,
            items,
            skipped,
            seven_password: password.map(String::from),
            folder: core::cell::RefCell::new(None),
            rar: core::cell::RefCell::new(None),
        })
    }
    fn zip(data: Vec<u8>) -> Result<Self, &'static str> {
        const DAMAGED: &str = "The archive is damaged";
        let tail = data.len().saturating_sub(65_557);
        let eocd = (tail..data.len().saturating_sub(21))
            .rev()
            .find(|i| data[*i..].starts_with(b"PK\x05\x06"))
            .ok_or(DAMAGED)?;
        let count = usize::from(le16(&data, eocd + 10).ok_or(DAMAGED)?);
        let mut at = le32(&data, eocd + 16).ok_or(DAMAGED)? as usize;
        let mut items = Vec::new();
        let mut skipped = 0;
        for _ in 0..count {
            let h = data
                .get(at..at + 46)
                .filter(|h| h.starts_with(b"PK\x01\x02"))
                .ok_or(DAMAGED)?;
            let flags = le16(h, 8).unwrap_or(0);
            let mut method = le16(h, 10).unwrap_or(0);
            let crc = le32(h, 16).unwrap_or(0);
            let packed = le32(h, 20).unwrap_or(0) as usize;
            let size = u64::from(le32(h, 24).unwrap_or(0));
            let (n, x, c) = (
                usize::from(le16(h, 28).unwrap_or(0)),
                usize::from(le16(h, 30).unwrap_or(0)),
                usize::from(le16(h, 32).unwrap_or(0)),
            );
            let local = le32(h, 42).unwrap_or(0) as usize;
            let raw = data.get(at + 46..at + 46 + n).ok_or(DAMAGED)?;
            let extra = data.get(at + 46 + n..at + 46 + n + x).unwrap_or(&[]);
            at += 46 + n + x + c;
            let name = if flags & 0x800 != 0 {
                String::from_utf8_lossy(raw).into_owned()
            } else {
                raw.iter().map(|b| char::from(*b)).collect()
            };
            let Some(path) = safe_path(&name) else {
                skipped += 1;
                continue;
            };
            let encrypted = flags & 1 != 0;
            if method == AES {
                method = aes_info(extra).map_or(u16::MAX, |(m, _)| m);
            }
            let lh = data
                .get(local..local + 30)
                .filter(|h| h.starts_with(b"PK\x03\x04"))
                .ok_or(DAMAGED)?;
            let offset = local
                + 30
                + usize::from(le16(lh, 26).unwrap_or(0))
                + usize::from(le16(lh, 28).unwrap_or(0));
            if offset.checked_add(packed).is_none_or(|e| e > data.len()) {
                return Err(DAMAGED);
            }
            items.push(Item {
                folder: name.ends_with('/') || name.ends_with('\\'),
                path,
                size,
                encrypted,
                method,
                offset,
                packed,
                crc,
            });
        }
        Ok(Self {
            data,
            items,
            skipped,
            seven_password: None,
            folder: core::cell::RefCell::new(None),
            rar: core::cell::RefCell::new(None),
        })
    }
    fn tar(data: Vec<u8>) -> Result<Self, &'static str> {
        let octal = |b: &[u8]| -> Option<u64> {
            let s = core::str::from_utf8(b)
                .ok()?
                .trim_matches(|c: char| c == '\0' || c == ' ');
            if s.is_empty() {
                Some(0)
            } else {
                u64::from_str_radix(s, 8).ok()
            }
        };
        let (mut items, mut skipped, mut at) = (Vec::new(), 0, 0usize);
        let mut long: Option<String> = None;
        while at + 512 <= data.len() {
            let h = &data[at..at + 512];
            if h.iter().all(|b| *b == 0) {
                break;
            }
            let len = octal(&h[124..136]).ok_or("The TAR archive is damaged")? as usize;
            let body = at + 512;
            if body.checked_add(len).is_none_or(|e| e > data.len()) {
                return Err("The TAR archive is damaged");
            }
            let field = |b: &[u8]| -> String {
                let end = b.iter().position(|c| *c == 0).unwrap_or(b.len());
                String::from_utf8_lossy(&b[..end]).into_owned()
            };
            match h[156] {
                b'L' => long = Some(field(&data[body..body + len])),
                b'x' | b'g' => (),
                kind @ (b'0' | b'\0' | b'5' | b'7') => {
                    let name = long.take().unwrap_or_else(|| {
                        let (prefix, base) = (field(&h[345..500]), field(&h[..100]));
                        if &h[257..262] == b"ustar" && !prefix.is_empty() {
                            alloc::format!("{prefix}/{base}")
                        } else {
                            base
                        }
                    });
                    match safe_path(&name) {
                        Some(path) => items.push(Item {
                            path,
                            folder: kind == b'5',
                            size: len as u64,
                            encrypted: false,
                            method: STORED,
                            offset: body,
                            packed: len,
                            crc: 0,
                        }),
                        None => skipped += 1,
                    }
                }
                // Links, devices and FIFOs are left out.
                _ => skipped += 1,
            }
            at = body + len.div_ceil(512) * 512;
        }
        Ok(Self {
            data,
            items,
            skipped,
            seven_password: None,
            folder: core::cell::RefCell::new(None),
            rar: core::cell::RefCell::new(None),
        })
    }
    /// Entry `i`'s content (ZIP entries checked against their CRC; AES
    /// entries authenticated with `password`).
    pub fn read(&self, i: usize, password: Option<&str>, limit: usize) -> Result<Vec<u8>, Fault> {
        let item = self.items.get(i).ok_or(Fault::Damaged)?;
        if item.folder {
            return Ok(Vec::new());
        }
        if item.size as usize > limit {
            return Err(Fault::TooLarge);
        }
        if item.method == SEVEN_ZIP {
            return self.seven_read(item, password, limit);
        }
        if item.method == RAR {
            return self.rar_read(item, password, limit);
        }
        let stored = &self.data[item.offset..item.offset + item.packed];
        let decrypted;
        let body = if item.encrypted {
            let password = password.ok_or(Fault::Password)?;
            decrypted = decrypt(password, stored).map_err(|e| match e {
                Refused::Password => Fault::Password,
                Refused::Damaged => Fault::Damaged,
            })?;
            &decrypted[..]
        } else {
            stored
        };
        let out = match item.method {
            STORED => body.to_vec(),
            DEFLATE => inflate(body, limit).map_err(|_| Fault::Damaged)?,
            _ => return Err(Fault::Method),
        };
        // AE-2 entries carry no CRC (the authentication code replaces it).
        if out.len() as u64 != item.size || (item.crc != 0 && crc32(0, &out) != item.crc) {
            return Err(Fault::Damaged);
        }
        Ok(out)
    }
}

impl Archive {
    fn rar_read(
        &self,
        item: &Item,
        password: Option<&str>,
        limit: usize,
    ) -> Result<Vec<u8>, Fault> {
        use crate::rar5::{Archive as Rar, Fault as F};
        let map = |f: F| match f {
            F::Password => Fault::Password,
            F::Unsupported => Fault::Method,
            F::TooLarge => Fault::TooLarge,
            F::Malformed => Fault::Damaged,
        };
        let mut cache = self.rar.borrow_mut();
        if cache.is_none() {
            let archive = Rar::open(&self.data).map_err(map)?;
            *cache = Some(archive.read_all(password, limit).map_err(map)?);
        }
        Ok(cache
            .as_ref()
            .unwrap()
            .get(item.offset)
            .cloned()
            .flatten()
            .unwrap_or_default())
    }
    fn seven_read(
        &self,
        item: &Item,
        password: Option<&str>,
        limit: usize,
    ) -> Result<Vec<u8>, Fault> {
        use crate::sevenzip::{Archive as Seven, Fault as F};
        let map = |f: F| match f {
            F::Password => Fault::Password,
            F::Unsupported => Fault::Method,
            F::TooLarge => Fault::TooLarge,
            F::Malformed => Fault::Damaged,
        };
        let header_password = self.seven_password.as_deref().or(password);
        let archive = Seven::open(&self.data, header_password, limit).map_err(map)?;
        let Some(folder) = archive.folder_of(item.offset) else {
            return Ok(Vec::new());
        };
        let mut cache = self.folder.borrow_mut();
        if cache.as_ref().is_none_or(|(f, _)| *f != folder) {
            *cache = Some((folder, archive.unpack(folder, password).map_err(map)?));
        }
        let bytes = &cache.as_ref().unwrap().1;
        archive.slice(item.offset, bytes, password).map_err(map)
    }
}

/// A ZIP writer in memory: deflated entries (stored when deflate doesn't
/// help), optionally WinZip AES-256 encrypted with `password` and
/// per-entry salts from `salt`.
pub struct ZipWriter<'p> {
    out: Vec<u8>,
    central: Vec<u8>,
    entries: u16,
    password: Option<&'p str>,
}
impl<'p> ZipWriter<'p> {
    pub fn new(password: Option<&'p str>) -> Self {
        Self {
            out: Vec::new(),
            central: Vec::new(),
            entries: 0,
            password,
        }
    }
    /// Add `path` (a folder when `data` is None).
    pub fn add(
        &mut self,
        path: &str,
        data: Option<&[u8]>,
        level: u8,
        salt: impl FnOnce() -> [u8; SALT_BYTES],
    ) {
        let folder = data.is_none();
        let name = if folder && !path.ends_with('/') {
            alloc::format!("{path}/")
        } else {
            String::from(path)
        };
        let raw = data.unwrap_or(&[]);
        let crc = crc32(0, raw);
        let (mut method, mut body) = (STORED, raw.to_vec());
        if !folder && level > 0 {
            let deflated = deflate(raw, level);
            if deflated.len() < raw.len() {
                (method, body) = (DEFLATE, deflated);
            }
        }
        let mut extra = Vec::new();
        let (mut flags, mut header_method, mut header_crc) = (0u16, method, crc);
        if let (Some(password), false) = (self.password, folder) {
            body = encrypt(password, salt(), &body);
            extra.extend_from_slice(&aes_extra(method));
            flags |= 1;
            header_method = AES;
            header_crc = 0;
        }
        if !name.is_ascii() {
            flags |= 1 << 11;
        }
        let offset = self.out.len() as u32;
        let version: u16 = if header_method == AES { 51 } else { 20 };
        let record = |sig: u32, central: bool| {
            let mut b = Vec::new();
            b.extend(sig.to_le_bytes());
            if central {
                b.extend(version.to_le_bytes());
            }
            b.extend(version.to_le_bytes());
            b.extend(flags.to_le_bytes());
            b.extend(header_method.to_le_bytes());
            b.extend([0u8, 0, 0x21, 0]); // time 00:00, date 1980-01-01
            b.extend(header_crc.to_le_bytes());
            b.extend((body.len() as u32).to_le_bytes());
            b.extend((raw.len() as u32).to_le_bytes());
            b.extend((name.len() as u16).to_le_bytes());
            b.extend((extra.len() as u16).to_le_bytes());
            if central {
                b.extend([0u8; 6]); // comment, disk, internal attributes
                b.extend((if folder { 0x10u32 } else { 0x20 }).to_le_bytes());
                b.extend(offset.to_le_bytes());
            }
            b.extend(name.as_bytes());
            b.extend(&extra);
            b
        };
        let local = record(0x0403_4b50, false);
        let central = record(0x0201_4b50, true);
        self.out.extend(local);
        self.out.extend(&body);
        self.central.extend(central);
        self.entries += 1;
    }
    pub fn finish(mut self) -> Vec<u8> {
        let offset = self.out.len() as u32;
        let size = self.central.len() as u32;
        self.out.extend(core::mem::take(&mut self.central));
        self.out.extend(0x0605_4b50u32.to_le_bytes());
        self.out.extend([0u8; 4]);
        self.out.extend(self.entries.to_le_bytes());
        self.out.extend(self.entries.to_le_bytes());
        self.out.extend(size.to_le_bytes());
        self.out.extend(offset.to_le_bytes());
        self.out.extend([0u8; 2]);
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zip_writer_round_trips_plain_and_aes_entries_through_archive() {
        let text = b"hello archive ".repeat(50);
        for password in [None, Some("s3cret!")] {
            let mut w = ZipWriter::new(password);
            w.add("docs", None, 6, || [1; SALT_BYTES]);
            w.add("docs/a.txt", Some(&text), 6, || [2; SALT_BYTES]);
            w.add("b.bin", Some(&[0, 1, 2, 3]), 6, || [3; SALT_BYTES]);
            let zip = w.finish();
            let a = Archive::open("x.zip", zip, 1 << 20).unwrap();
            let paths: Vec<_> = a
                .items
                .iter()
                .map(|i| (i.path.as_str(), i.folder, i.encrypted))
                .collect();
            let enc = password.is_some();
            assert_eq!(
                paths,
                [
                    ("docs", true, false),
                    ("docs/a.txt", false, enc),
                    ("b.bin", false, enc)
                ]
            );
            assert_eq!(a.read(1, password, 1 << 20).unwrap(), text);
            assert_eq!(a.read(2, password, 1 << 20).unwrap(), [0, 1, 2, 3]);
            if enc {
                assert_eq!(a.read(1, None, 1 << 20), Err(Fault::Password));
                assert_eq!(a.read(1, Some("nope"), 1 << 20), Err(Fault::Password));
            }
            assert_eq!(a.read(1, password, 10), Err(Fault::TooLarge));
        }
    }
    #[test]
    fn tar_gz_and_gz_open_and_unsafe_paths_are_skipped() {
        let mut tar = Vec::new();
        for (name, data, folder) in [
            ("dir/", &b""[..], true),
            ("dir/f.txt", b"abc", false),
            ("../evil", b"x", false),
        ] {
            tar.extend(tar_header(name, data.len() as u64, 0, folder).unwrap());
            tar.extend(data);
            tar.resize(tar.len().next_multiple_of(512), 0);
        }
        tar.resize(tar.len() + 1024, 0);
        let a = Archive::open("x.tar.gz", gzip(Some("x.tar"), &tar, 6), 1 << 20).unwrap();
        assert_eq!(a.items.len(), 2);
        assert_eq!(a.skipped, 1, "../evil is never written");
        assert_eq!(a.read(1, None, 64).unwrap(), b"abc");
        let g = Archive::open("notes.txt.gz", gzip(None, b"plain", 6), 64).unwrap();
        assert_eq!(
            (g.items[0].path.as_str(), g.read(0, None, 64).unwrap()),
            ("notes.txt", b"plain".to_vec())
        );
        assert_eq!(safe_path("a\\b/../c"), None);
        assert_eq!(safe_path("/abs/./x"), Some("abs/x".into()));
        assert_eq!(safe_path("C:/win"), None);
    }
    #[test]
    fn deflate_round_trips_and_inflate_honours_its_limit() {
        let data: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        let packed = deflate(&data, 6);
        assert!(packed.len() < data.len() / 4);
        assert_eq!(inflate(&packed, data.len()).unwrap(), data);
        assert!(inflate(&packed, 100).is_err());
    }
    #[test]
    fn winzip_aes_round_trips_and_refuses_a_wrong_password_or_tampering() {
        let salt = [7u8; SALT_BYTES];
        let plain = b"the quick brown fox jumps over the lazy dog".repeat(9);
        let stored = encrypt("correct horse", salt, &plain);
        assert_eq!(stored.len(), SALT_BYTES + 2 + plain.len() + AUTH_BYTES);
        assert_ne!(
            &stored[SALT_BYTES + 2..SALT_BYTES + 2 + plain.len()],
            &plain[..]
        );
        assert_eq!(decrypt("correct horse", &stored).unwrap(), plain);
        assert_eq!(decrypt("wrong", &stored), Err(Refused::Password));
        let mut tampered = stored.clone();
        tampered[SALT_BYTES + 5] ^= 1;
        assert_eq!(decrypt("correct horse", &tampered), Err(Refused::Damaged));
        assert_eq!(aes_info(&aes_extra(DEFLATE)), Some((DEFLATE, 3)));
    }
    #[test]
    fn pbkdf2_hmac_sha1_matches_rfc_6070() {
        // RFC 6070 test vector 2: "password"/"salt", 2 iterations, 20 bytes.
        let mut out = [0u8; 20];
        pbkdf2::pbkdf2_hmac::<Sha1>(b"password", b"salt", 2, &mut out);
        assert_eq!(
            out,
            [
                0xea, 0x6c, 0x01, 0x4d, 0xc7, 0x2d, 0x6f, 0x8c, 0xcd, 0x1e, 0xd9, 0x2a, 0xce, 0x1d,
                0x41, 0xf0, 0xd8, 0xde, 0x89, 0x57
            ]
        );
    }
    #[test]
    fn aes_256_matches_fips_197() {
        // FIPS-197 C.3: AES-256 of 00112233...ff with key 000102...1f.
        let key: [u8; 32] = core::array::from_fn(|i| i as u8);
        let cipher = aes::Aes256::new(GenericArray::from_slice(&key));
        let mut block = GenericArray::clone_from_slice(&core::array::from_fn::<u8, 16, _>(|i| {
            (i as u8) * 0x11
        }));
        cipher.encrypt_block(&mut block);
        assert_eq!(
            block.as_slice(),
            [
                0x8e, 0xa2, 0xb7, 0xca, 0x51, 0x67, 0x45, 0xbf, 0xea, 0xfc, 0x49, 0x90, 0x4b, 0x49,
                0x60, 0x89
            ]
        );
    }
    #[test]
    fn gzip_round_trips_with_its_name_and_checks_the_crc() {
        let data = b"hello hello hello hello".to_vec();
        let g = gzip(Some("notes.txt"), &data, 9);
        assert_eq!(gunzip(&g, 1024).unwrap(), (Some("notes.txt".into()), data));
        let mut bad = g.clone();
        let n = bad.len();
        bad[n - 6] ^= 1;
        assert!(gunzip(&bad, 1024).is_err());
        assert_eq!(crc32(0, b"123456789"), 0xCBF4_3926);
    }
    #[test]
    fn tar_headers_are_valid_ustar_with_long_names_split() {
        let h = tar_header("docs/readme.txt", 5, 0, false).unwrap();
        assert_eq!(&h[257..262], b"ustar");
        assert_eq!(&h[124..135], b"00000000005");
        let long = alloc::format!("{}/{}", "a".repeat(120), "file.bin");
        let h = tar_header(&long, 1, 0, false).unwrap();
        assert_eq!(&h[..8], b"file.bin");
        assert_eq!(h[345], b'a');
        assert!(tar_header(&"x".repeat(300), 1, 0, false).is_none());
    }
}

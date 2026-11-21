//! RAR 5 archives read from memory (extraction only; Doors never writes
//! RAR): the block structure from RARLAB's published "RAR 5.0 archive
//! format" technote, stored and compressed entries (the RAR 5 LZ/Huffman
//! coder with its delta, x86 and ARM filters, solid streams across files)
//! and AES-256 encrypted entries (PBKDF2-HMAC-SHA256 keys). RAR 4 archives,
//! multi-volume sets and encrypted headers are reported, never guessed.
extern crate alloc;
use alloc::{string::String, vec::Vec};

pub const SIGNATURE: [u8; 8] = [0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x01, 0x00];
pub const SIGNATURE_RAR4: [u8; 7] = [0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x00];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    Malformed,
    Unsupported,
    Password,
    TooLarge,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub folder: bool,
    pub size: u64,
    pub encrypted: bool,
    /// Data area within the archive.
    data: (usize, usize),
    method: u32,
    dictionary: usize,
    solid: bool,
    crc: Option<u32>,
    crypt: Option<Crypt>,
    link: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Crypt {
    kdf: u8,
    salt: [u8; 16],
    iv: [u8; 16],
    check: Option<[u8; 8]>,
    /// Checksums are HMAC-converted (the "use MAC" flag).
    mac: bool,
}

fn vint(b: &[u8], at: &mut usize) -> Result<u64, Fault> {
    let mut v = 0u64;
    for i in 0..10 {
        let byte = *b.get(*at).ok_or(Fault::Malformed)?;
        *at += 1;
        v |= u64::from(byte & 0x7F) << (7 * i);
        if byte & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err(Fault::Malformed)
}
fn crc32(data: &[u8]) -> u32 {
    crate::archive_codec::crc32(0, data)
}

pub struct Archive<'a> {
    data: &'a [u8],
    pub entries: Vec<Entry>,
    pub solid: bool,
}
impl<'a> Archive<'a> {
    pub fn open(data: &'a [u8]) -> Result<Self, Fault> {
        if data.starts_with(&SIGNATURE_RAR4) {
            return Err(Fault::Unsupported);
        }
        if !data.starts_with(&SIGNATURE) {
            return Err(Fault::Malformed);
        }
        let mut at = SIGNATURE.len();
        let mut entries = Vec::new();
        let mut solid = false;
        while at < data.len() {
            let crc = u32::from_le_bytes(
                data.get(at..at + 4)
                    .ok_or(Fault::Malformed)?
                    .try_into()
                    .unwrap(),
            );
            let mut p = at + 4;
            let size = vint(data, &mut p)? as usize;
            let header_start = p;
            let header_end = header_start.checked_add(size).ok_or(Fault::Malformed)?;
            let header = data.get(at + 4..header_end).ok_or(Fault::Malformed)?;
            if crc32(header) != crc {
                return Err(Fault::Malformed);
            }
            let kind = vint(data, &mut p)?;
            let flags = vint(data, &mut p)?;
            let extra = if flags & 1 != 0 {
                vint(data, &mut p)? as usize
            } else {
                0
            };
            let data_size = if flags & 2 != 0 {
                vint(data, &mut p)? as usize
            } else {
                0
            };
            let data_start = header_end;
            let data_end = data_start.checked_add(data_size).ok_or(Fault::Malformed)?;
            if data_end > data.len() {
                return Err(Fault::Malformed);
            }
            let extra_start = header_end.checked_sub(extra).ok_or(Fault::Malformed)?;
            match kind {
                1 => {
                    let archive_flags = vint(data, &mut p)?;
                    if archive_flags & 1 != 0 {
                        return Err(Fault::Unsupported); // multi-volume
                    }
                    solid = archive_flags & 4 != 0;
                }
                2 => {
                    let file_flags = vint(data, &mut p)?;
                    let unpacked = vint(data, &mut p)?;
                    let _attributes = vint(data, &mut p)?;
                    if file_flags & 2 != 0 {
                        p += 4;
                    }
                    let crc = if file_flags & 4 != 0 {
                        let c = u32::from_le_bytes(
                            data.get(p..p + 4)
                                .ok_or(Fault::Malformed)?
                                .try_into()
                                .unwrap(),
                        );
                        p += 4;
                        Some(c)
                    } else {
                        None
                    };
                    let info = vint(data, &mut p)?;
                    let _host = vint(data, &mut p)?;
                    let name_len =
                        usize::try_from(vint(data, &mut p)?).map_err(|_| Fault::Malformed)?;
                    let name = data
                        .get(p..p.checked_add(name_len).ok_or(Fault::Malformed)?)
                        .ok_or(Fault::Malformed)?;
                    let path = String::from_utf8_lossy(name).replace('\\', "/");
                    let (crypt, link) =
                        extras(data.get(extra_start..header_end).ok_or(Fault::Malformed)?)?;
                    if info & 0x3F != 0 {
                        return Err(Fault::Unsupported);
                    }
                    entries.push(Entry {
                        path,
                        folder: file_flags & 1 != 0,
                        size: if file_flags & 8 != 0 {
                            u64::MAX
                        } else {
                            unpacked
                        },
                        encrypted: crypt.is_some(),
                        data: (data_start, data_end),
                        method: ((info >> 7) & 7) as u32,
                        dictionary: 0x20000usize << ((info >> 10) & 0x1F),
                        solid: info & 0x40 != 0,
                        crc,
                        crypt,
                        link,
                    });
                }
                3 => (), // service headers (comments, ACLs, streams)
                4 => return Err(Fault::Password), // encrypted headers
                5 => break,
                _ => (),
            }
            at = data_end;
        }
        Ok(Self {
            data,
            entries,
            solid,
        })
    }
    /// Entry `index`'s bytes. Solid entries decode every earlier solid
    /// entry first (the stream is continuous).
    pub fn read(
        &self,
        index: usize,
        password: Option<&str>,
        limit: usize,
    ) -> Result<Vec<u8>, Fault> {
        let e = self.entries.get(index).ok_or(Fault::Malformed)?;
        if e.folder || e.link {
            return Ok(Vec::new());
        }
        if e.size > limit as u64 {
            return Err(Fault::TooLarge);
        }
        // The solid run this entry belongs to.
        // Folders and links inside a solid run carry no data: skip them.
        let mut first = index;
        if e.solid {
            let mut i = index;
            while i > 0 {
                i -= 1;
                let f = &self.entries[i];
                if f.folder || f.link {
                    continue;
                }
                first = i;
                if !f.solid {
                    break;
                }
            }
        }
        let mut window: Vec<u8> = Vec::new();
        let mut state = Unpack::new();
        let mut result = Vec::new();
        let mut hash_key = None;
        for i in first..=index {
            let f = &self.entries[i];
            if f.folder || f.link {
                continue;
            }
            let packed = &self.data[f.data.0..f.data.1];
            let decrypted;
            let body = match &f.crypt {
                Some(c) => {
                    let (d, k) = decrypt(c, packed, password)?;
                    decrypted = d;
                    if i == index {
                        hash_key = Some(k);
                    }
                    &decrypted[..]
                }
                None => packed,
            };
            let start = window.len();
            if f.method == 0 {
                window.extend_from_slice(body.get(..f.size as usize).ok_or(Fault::Malformed)?);
            } else {
                if !f.solid || i == first {
                    state = Unpack::new();
                    window.clear();
                }
                let out = state.decode(
                    body,
                    &mut window,
                    f.size as usize,
                    f.dictionary.min(limit.max(1 << 22)),
                )?;
                if out.len() != f.size as usize {
                    return Err(Fault::Malformed);
                }
                if i == index {
                    result = out;
                }
                continue;
            }
            if i == index {
                result = window[start..].to_vec();
            }
        }
        verify(e, &result, hash_key.as_ref())?;
        Ok(result)
    }
}
/// An entry's checksum: plain CRC32, or (encrypted with the MAC flag) the
/// HMAC-converted one. An encrypted entry with neither a password check nor
/// a checksum could not tell a wrong password from data, and is refused.
fn verify(e: &Entry, data: &[u8], hash_key: Option<&[u8; 32]>) -> Result<(), Fault> {
    let wrong = if e.encrypted {
        Fault::Password
    } else {
        Fault::Malformed
    };
    match (e.crc, &e.crypt) {
        (Some(crc), Some(c)) if c.mac => {
            let key = hash_key.ok_or(Fault::Password)?;
            if mac_crc(key, crc32(data)) != crc {
                return Err(wrong);
            }
        }
        (Some(crc), _) => {
            if crc32(data) != crc {
                return Err(wrong);
            }
        }
        (None, Some(c)) if c.check.is_none() => return Err(Fault::Unsupported),
        (None, _) => (),
    }
    Ok(())
}
fn extras(b: &[u8]) -> Result<(Option<Crypt>, bool), Fault> {
    let mut at = 0;
    let mut crypt = None;
    let mut link = false;
    while at < b.len() {
        let size = vint(b, &mut at)? as usize;
        let end = at.checked_add(size).ok_or(Fault::Malformed)?;
        if end > b.len() {
            return Err(Fault::Malformed);
        }
        let mut p = at;
        let kind = vint(b, &mut p)?;
        match kind {
            1 => {
                let version = vint(b, &mut p)?;
                let flags = vint(b, &mut p)?;
                if version != 0 {
                    return Err(Fault::Unsupported);
                }
                let kdf = *b.get(p).ok_or(Fault::Malformed)?;
                let salt: [u8; 16] = b
                    .get(p + 1..p + 17)
                    .ok_or(Fault::Malformed)?
                    .try_into()
                    .unwrap();
                let iv: [u8; 16] = b
                    .get(p + 17..p + 33)
                    .ok_or(Fault::Malformed)?
                    .try_into()
                    .unwrap();
                let check = if flags & 1 != 0 {
                    Some(
                        b.get(p + 33..p + 41)
                            .ok_or(Fault::Malformed)?
                            .try_into()
                            .unwrap(),
                    )
                } else {
                    None
                };
                crypt = Some(Crypt {
                    kdf,
                    salt,
                    iv,
                    check,
                    mac: flags & 2 != 0,
                });
            }
            5 => link = true,
            _ => (),
        }
        at = end;
    }
    Ok((crypt, link))
}
/// PBKDF2-HMAC-SHA256 as RAR 5 runs it: one iteration chain gives the AES
/// key (after `2^kdf` rounds), the checksum MAC key (+16) and the password
/// check value (+32).
fn derive(password: &str, salt: &[u8; 16], kdf: u8) -> ([u8; 32], [u8; 32], [u8; 32]) {
    use hmac::{Hmac, Mac};
    let rounds = 1usize << kdf;
    let mac =
        <Hmac<sha2::Sha256> as Mac>::new_from_slice(password.as_bytes()).expect("any key length");
    let mut m = mac.clone();
    m.update(salt);
    m.update(&1u32.to_be_bytes());
    let mut u: [u8; 32] = m.finalize().into_bytes().into();
    let mut f = u;
    let (mut key, mut hash_key) = ([0u8; 32], [0u8; 32]);
    for i in 1..rounds + 32 {
        if i == rounds {
            key = f;
        }
        if i == rounds + 16 {
            hash_key = f;
        }
        let mut m = mac.clone();
        m.update(&u);
        u = m.finalize().into_bytes().into();
        for (a, b) in f.iter_mut().zip(u) {
            *a ^= b;
        }
    }
    (key, hash_key, f)
}
/// The CRC32 an encrypted entry stores: HMAC-SHA256 of the CRC under the
/// hash key, folded to 32 bits (unrar's ConvertHashToMAC).
fn mac_crc(hash_key: &[u8; 32], crc: u32) -> u32 {
    use hmac::{Hmac, Mac};
    let mut m = <Hmac<sha2::Sha256> as Mac>::new_from_slice(hash_key).expect("32-byte key");
    m.update(&crc.to_le_bytes());
    let digest = m.finalize().into_bytes();
    digest
        .iter()
        .enumerate()
        .fold(0u32, |v, (i, b)| v ^ (u32::from(*b) << ((i & 3) * 8)))
}
/// Decrypt an entry's data; also returns the checksum MAC key.
fn decrypt(c: &Crypt, data: &[u8], password: Option<&str>) -> Result<(Vec<u8>, [u8; 32]), Fault> {
    use aes::cipher::{BlockDecrypt, KeyInit, generic_array::GenericArray};
    let password = password.ok_or(Fault::Password)?;
    if c.kdf > 24 || !data.len().is_multiple_of(16) {
        return Err(Fault::Unsupported);
    }
    let (key, hash_key, v) = derive(password, &c.salt, c.kdf);
    if let Some(check) = c.check {
        let mut folded = [0u8; 8];
        for (i, b) in v.iter().enumerate() {
            folded[i % 8] ^= b;
        }
        if folded != check {
            return Err(Fault::Password);
        }
    }
    let cipher = aes::Aes256::new(GenericArray::from_slice(&key));
    let mut out = data.to_vec();
    let mut prev = c.iv;
    for block in out.chunks_exact_mut(16) {
        let saved: [u8; 16] = block.try_into().unwrap();
        cipher.decrypt_block(GenericArray::from_mut_slice(block));
        for (b, p) in block.iter_mut().zip(prev) {
            *b ^= p;
        }
        prev = saved;
    }
    Ok((out, hash_key))
}

// ---- The RAR 5 LZ/Huffman decoder ----

const NC: usize = 306;
const DC: usize = 64;
const LDC: usize = 16;
const RC: usize = 44;
const BC: usize = 20;

/// A canonical Huffman table (lengths up to 15 bits).
#[derive(Clone)]
struct Huffman {
    limit: [u32; 16],
    pos: [u32; 16],
    symbols: Vec<u16>,
}
impl Huffman {
    fn new(lengths: &[u8]) -> Self {
        let mut count = [0u32; 16];
        for &l in lengths {
            count[usize::from(l & 15)] += 1;
        }
        count[0] = 0;
        let mut limit = [0u32; 16];
        let mut pos = [0u32; 16];
        let mut upper = 0u32;
        for i in 1..16 {
            upper += count[i];
            limit[i] = upper << (16 - i);
            upper *= 2;
            pos[i] = pos[i - 1] + count[i - 1];
        }
        let mut next = pos;
        let mut symbols = alloc::vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            let l = usize::from(l & 15);
            if l != 0 {
                symbols[next[l] as usize] = s as u16;
                next[l] += 1;
            }
        }
        Self {
            limit,
            pos,
            symbols,
        }
    }
    fn decode(&self, r: &mut BitReader<'_>) -> Result<usize, Fault> {
        let field = r.peek16() & 0xFFFE;
        let mut bits = 15;
        for i in 1..15 {
            if field < self.limit[i] {
                bits = i;
                break;
            }
        }
        r.skip(bits as u32);
        let dist = (field - self.limit[bits - 1]) >> (16 - bits);
        let at = (self.pos[bits] + dist) as usize;
        Ok(usize::from(*self.symbols.get(at).unwrap_or(&0)))
    }
}

struct BitReader<'a> {
    b: &'a [u8],
    bit: usize,
}
impl<'a> BitReader<'a> {
    fn peek32(&self) -> u32 {
        let byte = self.bit / 8;
        let mut v = 0u64;
        for i in 0..5 {
            v = (v << 8) | u64::from(self.b.get(byte + i).copied().unwrap_or(0));
        }
        ((v << (self.bit % 8)) >> 8) as u32
    }
    fn peek16(&self) -> u32 {
        self.peek32() >> 16
    }
    fn skip(&mut self, n: u32) {
        self.bit += n as usize;
    }
    fn bits(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let v = self.peek32() >> (32 - n);
        self.skip(n);
        v
    }
    fn exhausted(&self) -> bool {
        self.bit > self.b.len() * 8
    }
}

#[derive(Clone, Copy)]
struct Filter {
    start: usize,
    length: usize,
    kind: u32,
    channels: usize,
}

struct Unpack {
    tables: Option<(Huffman, Huffman, Huffman, Huffman)>,
    old: [usize; 4],
    last_length: usize,
    filters: Vec<Filter>,
}
impl Unpack {
    fn new() -> Self {
        Self {
            tables: None,
            old: [0; 4],
            last_length: 0,
            filters: Vec::new(),
        }
    }
    fn read_tables(r: &mut BitReader<'_>) -> Result<(Huffman, Huffman, Huffman, Huffman), Fault> {
        let mut bc = [0u8; BC];
        let mut i = 0;
        while i < BC {
            let len = r.bits(4) as u8;
            if len == 15 {
                let zeros = r.bits(4) as usize;
                if zeros == 0 {
                    bc[i] = 15;
                    i += 1;
                } else {
                    for _ in 0..zeros + 2 {
                        if i < BC {
                            bc[i] = 0;
                            i += 1;
                        }
                    }
                }
            } else {
                bc[i] = len;
                i += 1;
            }
        }
        let bct = Huffman::new(&bc);
        let total = NC + DC + LDC + RC;
        let mut lengths = alloc::vec![0u8; total];
        let mut i = 0;
        while i < total {
            let n = bct.decode(r)?;
            if n < 16 {
                lengths[i] = n as u8;
                i += 1;
            } else if n < 18 {
                let count = if n == 16 {
                    r.bits(3) + 3
                } else {
                    r.bits(7) + 11
                } as usize;
                if i == 0 {
                    return Err(Fault::Malformed);
                }
                let prev = lengths[i - 1];
                for _ in 0..count {
                    if i < total {
                        lengths[i] = prev;
                        i += 1;
                    }
                }
            } else {
                let count = if n == 18 {
                    r.bits(3) + 3
                } else {
                    r.bits(7) + 11
                } as usize;
                for _ in 0..count {
                    if i < total {
                        lengths[i] = 0;
                        i += 1;
                    }
                }
            }
            if r.exhausted() {
                return Err(Fault::Malformed);
            }
        }
        Ok((
            Huffman::new(&lengths[..NC]),
            Huffman::new(&lengths[NC..NC + DC]),
            Huffman::new(&lengths[NC + DC..NC + DC + LDC]),
            Huffman::new(&lengths[NC + DC + LDC..]),
        ))
    }
    fn slot_length(r: &mut BitReader<'_>, slot: usize) -> usize {
        if slot < 8 {
            2 + slot
        } else {
            let bits = slot / 4 - 1;
            (2 + ((4 | (slot & 3)) << bits)) + r.bits(bits as u32) as usize
        }
    }
    fn filter_data(r: &mut BitReader<'_>) -> usize {
        let count = r.bits(2) as usize + 1;
        let mut v = 0usize;
        for i in 0..count {
            v |= (r.bits(8) as usize) << (8 * i);
        }
        v
    }
    /// Decode one entry's compressed data, appending `size` bytes to `out`
    /// (which already holds the solid stream so far).
    fn decode(
        &mut self,
        data: &[u8],
        out: &mut Vec<u8>,
        size: usize,
        dictionary: usize,
    ) -> Result<Vec<u8>, Fault> {
        // Unknown sizes (u64::MAX) decode to the end of the data, still
        // within the caller's limit.
        let end = out.len().saturating_add(size);
        let base = out.len();
        let mut at = 0usize;
        let mut last = false;
        // Every block is read, even past the entry's size: an empty entry
        // in a solid stream may still carry the tables the next one uses.
        while !last && at < data.len() {
            // Block header.
            let flags = *data.get(at).ok_or(Fault::Malformed)?;
            let check = *data.get(at + 1).ok_or(Fault::Malformed)?;
            let count = usize::from((flags >> 3) & 3) + 1;
            if count == 4 {
                return Err(Fault::Malformed);
            }
            let mut block = 0usize;
            let mut sum = 0x5Au8 ^ flags;
            for i in 0..count {
                let b = *data.get(at + 2 + i).ok_or(Fault::Malformed)?;
                block |= usize::from(b) << (8 * i);
                sum ^= b;
            }
            if sum != check {
                return Err(Fault::Malformed);
            }
            let body_start = at + 2 + count;
            let body_end = body_start.checked_add(block).ok_or(Fault::Malformed)?;
            let body = data.get(body_start..body_end).ok_or(Fault::Malformed)?;
            let bit_size = usize::from(flags & 7) + 1;
            let total_bits = if block == 0 {
                0
            } else {
                (block - 1) * 8 + bit_size
            };
            last = flags & 0x40 != 0;
            let mut r = BitReader { b: body, bit: 0 };
            if flags & 0x80 != 0 {
                self.tables = Some(Self::read_tables(&mut r)?);
            }
            let Some((ld, dd, ldd, rd)) = self.tables.clone() else {
                return Err(Fault::Malformed);
            };
            while r.bit < total_bits && out.len() < end {
                let main = ld.decode(&mut r)?;
                if main < 256 {
                    out.push(main as u8);
                } else if main >= 262 {
                    let mut length = Self::slot_length(&mut r, main - 262);
                    let slot = dd.decode(&mut r)?;
                    let mut distance = 1usize;
                    let dbits;
                    if slot < 4 {
                        dbits = 0;
                        distance += slot;
                    } else {
                        dbits = slot / 2 - 1;
                        distance += (2 | (slot & 1)) << dbits;
                    }
                    if dbits > 0 {
                        if dbits >= 4 {
                            if dbits > 4 {
                                distance += (r.bits(dbits as u32 - 4) as usize) << 4;
                            }
                            distance += ldd.decode(&mut r)?;
                        } else {
                            distance += r.bits(dbits as u32) as usize;
                        }
                    }
                    if distance > 0x100 {
                        length += 1;
                        if distance > 0x2000 {
                            length += 1;
                            if distance > 0x40000 {
                                length += 1;
                            }
                        }
                    }
                    self.old = [distance, self.old[0], self.old[1], self.old[2]];
                    self.last_length = length;
                    copy(out, distance, length, end, dictionary)?;
                } else if main == 256 {
                    let start = Self::filter_data(&mut r);
                    let length = Self::filter_data(&mut r);
                    let kind = r.bits(3);
                    let channels = if kind == 0 { r.bits(5) as usize + 1 } else { 0 };
                    if length <= 0x40_0000 && self.filters.len() < 8192 {
                        self.filters.push(Filter {
                            start: out.len() + start,
                            length,
                            kind,
                            channels,
                        });
                    }
                } else if main == 257 {
                    if self.last_length != 0 {
                        copy(out, self.old[0], self.last_length, end, dictionary)?;
                    }
                } else {
                    let n = main - 258;
                    let distance = self.old[n];
                    for k in (1..=n).rev() {
                        self.old[k] = self.old[k - 1];
                    }
                    self.old[0] = distance;
                    let slot = rd.decode(&mut r)?;
                    let length = Self::slot_length(&mut r, slot);
                    self.last_length = length;
                    copy(out, distance, length, end, dictionary)?;
                }
            }
            at = body_end;
        }
        if out.len() != end {
            return Err(Fault::Malformed);
        }
        // Filters whose blocks lie in this entry's output apply to a copy:
        // the window keeps the raw LZ output later matches read from.
        let mut file = out[base..].to_vec();
        let filters = core::mem::take(&mut self.filters);
        for f in filters {
            if f.start < base || f.start.checked_add(f.length).is_none_or(|e| e > out.len()) {
                if f.start >= out.len() {
                    self.filters.push(f);
                }
                continue;
            }
            let at = f.start - base;
            apply(&mut file[at..at + f.length], f, at)?;
        }
        Ok(file)
    }
}
fn copy(
    out: &mut Vec<u8>,
    distance: usize,
    length: usize,
    end: usize,
    dictionary: usize,
) -> Result<(), Fault> {
    if distance == 0 || distance > out.len() || distance > dictionary {
        return Err(Fault::Malformed);
    }
    let n = length.min(end - out.len());
    for _ in 0..n {
        out.push(out[out.len() - distance]);
    }
    Ok(())
}
fn apply(block: &mut [u8], f: Filter, offset: usize) -> Result<(), Fault> {
    match f.kind {
        0 => {
            let src = block.to_vec();
            let mut s = 0;
            for ch in 0..f.channels {
                let mut prev = 0u8;
                let mut d = ch;
                while d < block.len() {
                    prev = prev.wrapping_sub(*src.get(s).ok_or(Fault::Malformed)?);
                    block[d] = prev;
                    s += 1;
                    d += f.channels;
                }
            }
        }
        1 | 2 => {
            const FILE_SIZE: u32 = 0x100_0000;
            let e9 = f.kind == 2;
            let mut pos = 0usize;
            while pos + 4 < block.len() {
                let b = block[pos];
                pos += 1;
                if b == 0xE8 || (e9 && b == 0xE9) {
                    let off = ((pos + offset) as u32) % FILE_SIZE;
                    let addr = u32::from_le_bytes(block[pos..pos + 4].try_into().unwrap());
                    if addr & 0x8000_0000 != 0 {
                        if addr.wrapping_add(off) & 0x8000_0000 == 0 {
                            block[pos..pos + 4]
                                .copy_from_slice(&addr.wrapping_add(FILE_SIZE).to_le_bytes());
                        }
                    } else if addr.wrapping_sub(FILE_SIZE) & 0x8000_0000 != 0 {
                        block[pos..pos + 4].copy_from_slice(&addr.wrapping_sub(off).to_le_bytes());
                    }
                    pos += 4;
                }
            }
        }
        3 => {
            let mut pos = 0usize;
            while pos + 3 < block.len() {
                if block[pos + 3] == 0xEB {
                    let mut v = u32::from(block[pos])
                        | (u32::from(block[pos + 1]) << 8)
                        | (u32::from(block[pos + 2]) << 16);
                    v = v.wrapping_sub(((offset + pos) / 4) as u32);
                    block[pos] = v as u8;
                    block[pos + 1] = (v >> 8) as u8;
                    block[pos + 2] = (v >> 16) as u8;
                }
                pos += 4;
            }
        }
        _ => return Err(Fault::Unsupported),
    }
    Ok(())
}

impl Archive<'_> {
    /// Every entry's bytes in archive order, decoding each solid run once
    /// (folders, links and refused entries are None).
    pub fn read_all(
        &self,
        password: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Option<Vec<u8>>>, Fault> {
        let mut out: Vec<Option<Vec<u8>>> = alloc::vec![None; self.entries.len()];
        let mut window: Vec<u8> = Vec::new();
        let mut state = Unpack::new();
        let mut total = 0usize;
        for (i, f) in self.entries.iter().enumerate() {
            if f.folder || f.link {
                continue;
            }
            if f.size > limit as u64 || total + f.size as usize > limit {
                return Err(Fault::TooLarge);
            }
            let packed = &self.data[f.data.0..f.data.1];
            let decrypted;
            let mut hash_key = None;
            let body = match &f.crypt {
                Some(c) => {
                    let (d, k) = decrypt(c, packed, password)?;
                    decrypted = d;
                    hash_key = Some(k);
                    &decrypted[..]
                }
                None => packed,
            };
            let bytes = if f.method == 0 {
                body.get(..f.size as usize)
                    .ok_or(Fault::Malformed)?
                    .to_vec()
            } else {
                if !f.solid {
                    state = Unpack::new();
                    window.clear();
                }
                state.decode(
                    body,
                    &mut window,
                    f.size as usize,
                    f.dictionary.min(limit.max(1 << 22)),
                )?
            };
            verify(f, &bytes, hash_key.as_ref())?;
            total += bytes.len();
            out[i] = Some(bytes);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    /// A RAR 5 file whose one file header carries `name_len`.
    fn with_name_length(name_len: u64) -> Vec<u8> {
        let vint = |mut v: u64, out: &mut Vec<u8>| loop {
            let b = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                break;
            }
            out.push(b | 0x80);
        };
        let mut header = Vec::new();
        for v in [2u64, 0, 0, 0, 0, 0, 0] {
            vint(v, &mut header); // type, flags, file flags, size, attrs, info, host
        }
        vint(name_len, &mut header);
        let mut block = Vec::new();
        vint(header.len() as u64, &mut block);
        block.extend_from_slice(&header);
        let mut out = SIGNATURE.to_vec();
        out.extend_from_slice(&crc32(&block).to_le_bytes());
        out.extend_from_slice(&block);
        out
    }
    #[test]
    fn hostile_headers_are_refused_without_panicking() {
        assert_eq!(
            Archive::open(&with_name_length(u64::MAX)).err(),
            Some(Fault::Malformed)
        );
        assert_eq!(
            Archive::open(&with_name_length(1 << 40)).err(),
            Some(Fault::Malformed)
        );
    }
    #[test]
    fn refuses_rar4_and_damaged_headers() {
        assert_eq!(
            Archive::open(b"Rar!\x1a\x07\x00rest").err(),
            Some(Fault::Unsupported)
        );
        assert_eq!(Archive::open(b"nope").err(), Some(Fault::Malformed));
        let mut bad = SIGNATURE.to_vec();
        bad.extend_from_slice(&[1, 2, 3, 4, 3, 1, 0, 0]);
        assert_eq!(Archive::open(&bad).err(), Some(Fault::Malformed));
    }
    #[test]
    fn huffman_tables_decode_canonical_codes() {
        // Lengths 1, 2, 3, 3: codes 0, 10, 110, 111.
        let h = Huffman::new(&[1, 2, 3, 3]);
        let bytes = [0b0101_1011, 0b1000_0000];
        let mut r = BitReader { b: &bytes, bit: 0 };
        let got: Vec<usize> = (0..4).map(|_| h.decode(&mut r).unwrap()).collect();
        assert_eq!(got, [0, 1, 2, 3]);
    }
    #[test]
    fn filters_undo_delta_and_x86() {
        // Delta, 2 channels: stored as channel runs of differences.
        let mut block = [0xFF, 0xFF, 0xFE, 0xFE];
        apply(
            &mut block,
            Filter {
                start: 0,
                length: 4,
                kind: 0,
                channels: 2,
            },
            0,
        )
        .unwrap();
        assert_eq!(block, [1, 2, 2, 4]);
        // E8 with a relative address converted back from absolute.
        let mut code = [0xE8, 0x10, 0, 0, 0, 0x90, 0x90, 0x90];
        apply(
            &mut code,
            Filter {
                start: 0,
                length: 8,
                kind: 1,
                channels: 0,
            },
            0,
        )
        .unwrap();
        assert_eq!(&code[1..5], &0x0Fu32.to_le_bytes());
    }
    /// Archives from WinRAR (`DOOR_RAR_REF=<dir with ref-*.rar and src/>`).
    #[test]
    #[ignore = "needs DOOR_RAR_REF reference archives"]
    fn reads_winrar_archives() {
        let dir = std::env::var("DOOR_RAR_REF").expect("DOOR_RAR_REF");
        for (name, password) in [
            ("store", None),
            ("normal", None),
            ("solid", None),
            ("pass", Some("doors123")),
        ] {
            let bytes = std::fs::read(std::format!("{dir}/ref-{name}.rar")).unwrap();
            let a = Archive::open(&bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert!(!a.entries.is_empty(), "{name}");
            for (i, e) in a.entries.iter().enumerate() {
                if e.folder {
                    continue;
                }
                let want = std::fs::read(std::format!("{dir}/src/{}", e.path)).unwrap();
                let got = a
                    .read(i, password, 1 << 26)
                    .unwrap_or_else(|f| panic!("{name} {}: {f:?}", e.path));
                assert_eq!(got.len(), want.len(), "{name} {}", e.path);
                assert!(got == want, "{name} {} differs", e.path);
            }
            if password.is_some() {
                // WinRAR stores HMAC-converted checksums: the MAC path ran.
                assert!(
                    a.entries
                        .iter()
                        .any(|e| e.crypt.as_ref().is_some_and(|c| c.mac && e.crc.is_some())),
                    "{name}"
                );
                let i = a
                    .entries
                    .iter()
                    .position(|e| !e.folder && e.size > 0)
                    .unwrap();
                assert_eq!(
                    a.read(i, Some("wrong"), 1 << 26).err(),
                    Some(Fault::Password)
                );
                assert_eq!(a.read(i, None, 1 << 26).err(), Some(Fault::Password));
            }
        }
    }
}

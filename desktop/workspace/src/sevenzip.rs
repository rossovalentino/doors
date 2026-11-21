//! 7-Zip archives (`.7z`, format 0.4) read from memory: the signature
//! header, packed and encoded headers, folders with Copy, LZMA, LZMA2,
//! Deflate and BCJ (x86) coders, 7zAES (AES-256-CBC with the SHA-256 key
//! derivation) for passwords, substreams and file properties. Written from
//! the public `7zFormat.txt` description (7-Zip, public domain). BCJ2,
//! PPMd and BZip2 folders are reported as unsupported, never guessed.
extern crate alloc;
use crate::lzma;
use alloc::{string::String, vec::Vec};

pub const SIGNATURE: [u8; 6] = [b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    Malformed,
    /// A coder or feature this reader does not implement.
    Unsupported,
    /// Encrypted and no (or a wrong) password given.
    Password,
    TooLarge,
}

/// One entry: a file, a folder or an empty file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub folder: bool,
    pub size: u64,
    pub encrypted: bool,
    /// Folder index and substream within it (files with data).
    stream: Option<(usize, usize)>,
    crc: Option<u32>,
}

#[derive(Clone, Debug, Default)]
struct Coder {
    id: Vec<u8>,
    props: Vec<u8>,
    inputs: usize,
    outputs: usize,
}
#[derive(Clone, Debug, Default)]
struct Folder {
    coders: Vec<Coder>,
    /// (in index, out index) pairs.
    bonds: Vec<(usize, usize)>,
    packed: Vec<usize>,
    unpack_sizes: Vec<u64>,
    crc: Option<u32>,
    /// Pack stream index of this folder's first packed stream.
    first_pack: usize,
    /// Substream sizes and CRCs.
    sizes: Vec<u64>,
    crcs: Vec<Option<u32>>,
}
impl Folder {
    fn out_total(&self) -> usize {
        self.coders.iter().map(|c| c.outputs).sum()
    }
    /// The folder's final output: the out stream no bond consumes.
    fn main_out(&self) -> Option<usize> {
        (0..self.out_total()).find(|o| !self.bonds.iter().any(|b| b.1 == *o))
    }
    fn unpack_size(&self) -> u64 {
        self.main_out()
            .and_then(|o| self.unpack_sizes.get(o).copied())
            .unwrap_or(0)
    }
    fn encrypted(&self) -> bool {
        self.coders.iter().any(|c| c.id == AES)
    }
}

const COPY: &[u8] = &[0x00];
const LZMA: &[u8] = &[0x03, 0x01, 0x01];
const LZMA2: &[u8] = &[0x21];
const DEFLATE: &[u8] = &[0x04, 0x01, 0x08];
const BCJ: &[u8] = &[0x03, 0x03, 0x01, 0x03];
const AES: &[u8] = &[0x06, 0xF1, 0x07, 0x01];

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn byte(&mut self) -> Result<u8, Fault> {
        let v = *self.b.get(self.at).ok_or(Fault::Malformed)?;
        self.at += 1;
        Ok(v)
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], Fault> {
        let s = self
            .b
            .get(self.at..self.at.checked_add(n).ok_or(Fault::Malformed)?)
            .ok_or(Fault::Malformed)?;
        self.at += n;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, Fault> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, Fault> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    /// The 7z variable-length number.
    fn number(&mut self) -> Result<u64, Fault> {
        let first = self.byte()?;
        let mut mask = 0x80u8;
        let mut value = 0u64;
        for i in 0..8 {
            if first & mask == 0 {
                let high = u64::from(first & mask.wrapping_sub(1));
                return Ok(value | (high << (8 * i)));
            }
            value |= u64::from(self.byte()?) << (8 * i);
            mask >>= 1;
        }
        Ok(value)
    }
    fn count(&mut self, max: usize) -> Result<usize, Fault> {
        let n = self.number()?;
        if n > max as u64 {
            return Err(Fault::Malformed);
        }
        Ok(n as usize)
    }
    fn bits(&mut self, n: usize) -> Result<Vec<bool>, Fault> {
        let mut out = Vec::with_capacity(n);
        let mut byte = 0;
        for i in 0..n {
            if i % 8 == 0 {
                byte = self.byte()?;
            }
            out.push(byte & (0x80 >> (i % 8)) != 0);
        }
        Ok(out)
    }
    /// An "all defined" byte, then the vector.
    fn defined(&mut self, n: usize) -> Result<Vec<bool>, Fault> {
        if self.byte()? != 0 {
            Ok(alloc::vec![true; n])
        } else {
            self.bits(n)
        }
    }
    fn digests(&mut self, n: usize) -> Result<Vec<Option<u32>>, Fault> {
        let defined = self.defined(n)?;
        defined
            .into_iter()
            .map(|d| if d { self.u32().map(Some) } else { Ok(None) })
            .collect()
    }
}

const MAX_ITEMS: usize = 1 << 20;

#[derive(Default)]
struct Streams {
    pack_pos: u64,
    pack_sizes: Vec<u64>,
    folders: Vec<Folder>,
}
fn read_streams(r: &mut Reader<'_>) -> Result<Streams, Fault> {
    let mut s = Streams::default();
    let mut id = r.byte()?;
    if id == 0x06 {
        s.pack_pos = r.number()?;
        let n = r.count(MAX_ITEMS)?;
        loop {
            match r.byte()? {
                0x00 => break,
                0x09 => {
                    for _ in 0..n {
                        s.pack_sizes.push(r.number()?);
                    }
                }
                0x0A => {
                    r.digests(n)?;
                }
                _ => return Err(Fault::Malformed),
            }
        }
        id = r.byte()?;
    }
    if id == 0x07 {
        if r.byte()? != 0x0B {
            return Err(Fault::Malformed);
        }
        let n = r.count(MAX_ITEMS)?;
        if r.byte()? != 0 {
            return Err(Fault::Unsupported);
        }
        let mut first_pack = 0;
        for _ in 0..n {
            let mut f = Folder::default();
            let coders = r.count(64)?;
            for _ in 0..coders {
                let flags = r.byte()?;
                let mut c = Coder {
                    id: r.bytes(usize::from(flags & 0x0F))?.to_vec(),
                    inputs: 1,
                    outputs: 1,
                    ..Coder::default()
                };
                if flags & 0x10 != 0 {
                    c.inputs = r.count(64)?;
                    c.outputs = r.count(64)?;
                }
                if flags & 0x20 != 0 {
                    let len = r.count(1 << 16)?;
                    c.props = r.bytes(len)?.to_vec();
                }
                if flags & 0x80 != 0 {
                    return Err(Fault::Unsupported);
                }
                f.coders.push(c);
            }
            let outs = f.out_total();
            let ins: usize = f.coders.iter().map(|c| c.inputs).sum();
            for _ in 0..outs.saturating_sub(1) {
                let i = r.count(ins)?;
                let o = r.count(outs)?;
                f.bonds.push((i, o));
            }
            let packed = ins
                .checked_sub(outs.saturating_sub(1))
                .ok_or(Fault::Malformed)?;
            if packed == 1 {
                f.packed.push(
                    (0..ins)
                        .find(|i| !f.bonds.iter().any(|b| b.0 == *i))
                        .ok_or(Fault::Malformed)?,
                );
            } else {
                for _ in 0..packed {
                    f.packed.push(r.count(ins)?);
                }
            }
            f.first_pack = first_pack;
            first_pack += packed;
            s.folders.push(f);
        }
        if r.byte()? != 0x0C {
            return Err(Fault::Malformed);
        }
        for f in &mut s.folders {
            for _ in 0..f.out_total() {
                f.unpack_sizes.push(r.number()?);
            }
        }
        loop {
            match r.byte()? {
                0x00 => break,
                0x0A => {
                    let d = r.digests(s.folders.len())?;
                    for (f, c) in s.folders.iter_mut().zip(d) {
                        f.crc = c;
                    }
                }
                _ => return Err(Fault::Malformed),
            }
        }
        id = r.byte()?;
    }
    // One substream per folder unless SubStreamsInfo says otherwise.
    for f in &mut s.folders {
        f.sizes = alloc::vec![f.unpack_size()];
        f.crcs = alloc::vec![f.crc];
    }
    if id == 0x08 {
        let mut counts: Vec<usize> = alloc::vec![1; s.folders.len()];
        let mut prop = r.byte()?;
        if prop == 0x0D {
            for c in &mut counts {
                *c = r.count(MAX_ITEMS)?;
            }
            prop = r.byte()?;
        }
        for (f, &n) in s.folders.iter_mut().zip(&counts) {
            f.sizes.clear();
            if n == 0 {
                continue;
            }
            let total = f.unpack_size();
            let mut sum = 0u64;
            if prop == 0x09 {
                for _ in 0..n - 1 {
                    let size = r.number()?;
                    sum = sum.checked_add(size).ok_or(Fault::Malformed)?;
                    f.sizes.push(size);
                }
            }
            f.sizes
                .push(total.checked_sub(sum).ok_or(Fault::Malformed)?);
            if prop != 0x09 && n > 1 {
                return Err(Fault::Malformed);
            }
        }
        if prop == 0x09 {
            prop = r.byte()?;
        }
        // CRCs: for substreams whose folder CRC does not already cover
        // them (folders with one substream and a folder CRC).
        let need: usize = s
            .folders
            .iter()
            .zip(&counts)
            .map(|(f, &n)| if n == 1 && f.crc.is_some() { 0 } else { n })
            .try_fold(0usize, |a, n| a.checked_add(n).filter(|t| *t <= MAX_ITEMS))
            .ok_or(Fault::Malformed)?;
        let mut digests: Vec<Option<u32>> = alloc::vec![None; need];
        while prop != 0x00 {
            match prop {
                0x0A => digests = r.digests(need)?,
                _ => return Err(Fault::Malformed),
            }
            prop = r.byte()?;
        }
        let mut d = digests.into_iter();
        for (f, &n) in s.folders.iter_mut().zip(&counts) {
            if n == 1 && f.crc.is_some() {
                f.crcs = alloc::vec![f.crc];
            } else {
                f.crcs = (0..n).map(|_| d.next().flatten()).collect();
            }
        }
        id = r.byte()?;
    }
    if id != 0x00 {
        return Err(Fault::Malformed);
    }
    Ok(s)
}

/// SHA-256 based 7zAES key (2^cycles rounds over salt, the UTF-16LE
/// password and a 64-bit counter).
fn aes_key(cycles: u32, salt: &[u8], password: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let pass: Vec<u8> = password.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut key = [0u8; 32];
    if cycles == 0x3F {
        let mut all = Vec::new();
        all.extend_from_slice(salt);
        all.extend_from_slice(&pass);
        let n = all.len().min(32);
        key[..n].copy_from_slice(&all[..n]);
        return key;
    }
    let mut h = Sha256::new();
    for i in 0..(1u64 << cycles) {
        h.update(salt);
        h.update(&pass);
        h.update(i.to_le_bytes());
    }
    key.copy_from_slice(&h.finalize());
    key
}
fn aes_decode(props: &[u8], data: &[u8], password: Option<&str>) -> Result<Vec<u8>, Fault> {
    use aes::cipher::{BlockDecrypt, KeyInit, generic_array::GenericArray};
    let password = password.ok_or(Fault::Password)?;
    let b0 = *props.first().ok_or(Fault::Malformed)?;
    let cycles = u32::from(b0 & 0x3F);
    if cycles > 24 && cycles != 0x3F {
        return Err(Fault::Unsupported);
    }
    let (mut salt_len, mut iv_len) = (0usize, 0usize);
    let mut at = 1;
    if b0 & 0xC0 != 0 {
        let b1 = *props.get(1).ok_or(Fault::Malformed)?;
        salt_len = usize::from((b0 >> 7) & 1) + usize::from(b1 >> 4);
        iv_len = usize::from((b0 >> 6) & 1) + usize::from(b1 & 0x0F);
        at = 2;
    }
    let salt = props.get(at..at + salt_len).ok_or(Fault::Malformed)?;
    let mut iv = [0u8; 16];
    iv[..iv_len.min(16)].copy_from_slice(
        props
            .get(at + salt_len..at + salt_len + iv_len.min(16))
            .ok_or(Fault::Malformed)?,
    );
    if !data.len().is_multiple_of(16) {
        return Err(Fault::Malformed);
    }
    let key = aes_key(cycles, salt, password);
    let cipher = aes::Aes256::new(GenericArray::from_slice(&key));
    let mut out = data.to_vec();
    let mut prev = iv;
    for block in out.chunks_exact_mut(16) {
        let saved: [u8; 16] = block.try_into().unwrap();
        cipher.decrypt_block(GenericArray::from_mut_slice(block));
        for (b, p) in block.iter_mut().zip(prev) {
            *b ^= p;
        }
        prev = saved;
    }
    Ok(out)
}
fn crc32(data: &[u8]) -> u32 {
    crate::archive_codec::crc32(0, data)
}

/// An opened archive.
pub struct Archive<'a> {
    data: &'a [u8],
    streams: Streams,
    pub entries: Vec<Entry>,
    /// The header itself was encrypted (names hidden without the password).
    pub header_encrypted: bool,
    limit: usize,
}
impl<'a> Archive<'a> {
    /// Open from bytes; `password` is needed only when the header is
    /// encrypted. `limit` bounds every decoded buffer.
    pub fn open(data: &'a [u8], password: Option<&str>, limit: usize) -> Result<Self, Fault> {
        if data.len() < 32 || data[..6] != SIGNATURE || data[6] != 0 {
            return Err(Fault::Malformed);
        }
        let mut r = Reader { b: data, at: 8 };
        let start_crc = r.u32()?;
        if crc32(&data[12..32]) != start_crc {
            return Err(Fault::Malformed);
        }
        let offset = r.u64()?;
        let size = r.u64()?;
        let crc = r.u32()?;
        let begin = 32usize
            .checked_add(usize::try_from(offset).map_err(|_| Fault::Malformed)?)
            .ok_or(Fault::Malformed)?;
        let end = begin
            .checked_add(usize::try_from(size).map_err(|_| Fault::Malformed)?)
            .ok_or(Fault::Malformed)?;
        let mut header: Vec<u8> = data.get(begin..end).ok_or(Fault::Malformed)?.to_vec();
        if size == 0 {
            return Ok(Self {
                data,
                streams: Streams::default(),
                entries: Vec::new(),
                header_encrypted: false,
                limit,
            });
        }
        if crc32(&header) != crc {
            return Err(Fault::Malformed);
        }
        let mut header_encrypted = false;
        // Encoded (compressed, maybe encrypted) headers, possibly nested.
        for _ in 0..4 {
            if header.first() != Some(&0x17) {
                break;
            }
            let mut hr = Reader { b: &header, at: 1 };
            let streams = read_streams(&mut hr)?;
            let folder = streams.folders.first().ok_or(Fault::Malformed)?;
            header_encrypted |= folder.encrypted();
            let probe = Self {
                data,
                streams,
                entries: Vec::new(),
                header_encrypted,
                limit,
            };
            header = probe.unpack_folder(0, password)?;
        }
        let mut r = Reader { b: &header, at: 0 };
        if r.byte()? != 0x01 {
            return Err(Fault::Malformed);
        }
        let mut id = r.byte()?;
        if id == 0x02 {
            // Archive properties: skip.
            loop {
                let t = r.byte()?;
                if t == 0 {
                    break;
                }
                let n = r.count(1 << 20)?;
                r.bytes(n)?;
            }
            id = r.byte()?;
        }
        if id == 0x03 {
            read_streams(&mut r)?;
            id = r.byte()?;
        }
        let mut streams = Streams::default();
        if id == 0x04 {
            streams = read_streams(&mut r)?;
            id = r.byte()?;
        }
        let mut entries = Vec::new();
        if id == 0x05 {
            entries = read_files(&mut r, &streams)?;
            id = r.byte()?;
        }
        if id != 0x00 {
            return Err(Fault::Malformed);
        }
        Ok(Self {
            data,
            streams,
            entries,
            header_encrypted,
            limit,
        })
    }
    /// The decoded main output of folder `index`.
    fn unpack_folder(&self, index: usize, password: Option<&str>) -> Result<Vec<u8>, Fault> {
        let f = self.streams.folders.get(index).ok_or(Fault::Malformed)?;
        let mut pack_at = 32u64
            .checked_add(self.streams.pack_pos)
            .ok_or(Fault::Malformed)?;
        for size in &self.streams.pack_sizes[..f.first_pack.min(self.streams.pack_sizes.len())] {
            pack_at = pack_at.checked_add(*size).ok_or(Fault::Malformed)?;
        }
        // Packed input streams of this folder, by in-stream index.
        let mut packed: Vec<(usize, &[u8])> = Vec::new();
        let mut at = usize::try_from(pack_at).map_err(|_| Fault::Malformed)?;
        for (k, &input) in f.packed.iter().enumerate() {
            let size = *self
                .streams
                .pack_sizes
                .get(f.first_pack + k)
                .ok_or(Fault::Malformed)? as usize;
            packed.push((
                input,
                self.data
                    .get(at..at.checked_add(size).ok_or(Fault::Malformed)?)
                    .ok_or(Fault::Malformed)?,
            ));
            at += size;
        }
        let out = f.main_out().ok_or(Fault::Malformed)?;
        let result = self.decode_out(f, out, &packed, password, 0)?;
        if let Some(crc) = f.crc
            && crc32(&result) != crc
        {
            return Err(if f.encrypted() {
                Fault::Password
            } else {
                Fault::Malformed
            });
        }
        Ok(result)
    }
    /// Decode out stream `out` of folder `f` (recursively through bonds).
    fn decode_out(
        &self,
        f: &Folder,
        out: usize,
        packed: &[(usize, &[u8])],
        password: Option<&str>,
        depth: u32,
    ) -> Result<Vec<u8>, Fault> {
        if depth > 8 {
            return Err(Fault::Malformed);
        }
        // Which coder produces `out`, and its first input index.
        let (mut ci, mut out_base, mut in_base) = (0, 0, 0);
        loop {
            let c = f.coders.get(ci).ok_or(Fault::Malformed)?;
            if out < out_base + c.outputs {
                break;
            }
            out_base += c.outputs;
            in_base += c.inputs;
            ci += 1;
        }
        let c = &f.coders[ci];
        if c.inputs != 1 || c.outputs != 1 {
            return Err(Fault::Unsupported);
        }
        let size = usize::try_from(*f.unpack_sizes.get(out).ok_or(Fault::Malformed)?)
            .map_err(|_| Fault::TooLarge)?;
        if size > self.limit {
            return Err(Fault::TooLarge);
        }
        let input: Vec<u8> = match packed.iter().find(|(i, _)| *i == in_base) {
            Some((_, bytes)) => bytes.to_vec(),
            None => {
                let bond = f
                    .bonds
                    .iter()
                    .find(|b| b.0 == in_base)
                    .ok_or(Fault::Malformed)?;
                self.decode_out(f, bond.1, packed, password, depth + 1)?
            }
        };
        let map = |e: lzma::Error| match e {
            lzma::Error::Corrupt => Fault::Malformed,
            lzma::Error::TooLarge => Fault::TooLarge,
        };
        let decoded = match c.id.as_slice() {
            COPY => input,
            LZMA => lzma::decode_lzma(&c.props, &input, size, self.limit).map_err(map)?,
            LZMA2 => lzma::decode_lzma2(&input, size, self.limit).map_err(map)?,
            DEFLATE => crate::archive_codec::inflate(&input, size).map_err(|_| Fault::Malformed)?,
            BCJ => {
                let mut v = input;
                lzma::bcj_x86_decode(&mut v);
                v
            }
            AES => {
                let mut v = aes_decode(&c.props, &input, password)?;
                if v.len() < size {
                    return Err(Fault::Malformed);
                }
                v.truncate(size);
                v
            }
            _ => return Err(Fault::Unsupported),
        };
        if decoded.len() != size {
            // A wrong password usually ends here (garbage LZMA input).
            return Err(if f.encrypted() {
                Fault::Password
            } else {
                Fault::Malformed
            });
        }
        Ok(decoded)
    }
    /// The folder holding entry `index`'s data (None: empty or a folder).
    pub fn folder_of(&self, index: usize) -> Option<usize> {
        self.entries
            .get(index)
            .and_then(|e| e.stream)
            .map(|(f, _)| f)
    }
    /// The decoded output of folder `index`.
    pub fn unpack(&self, index: usize, password: Option<&str>) -> Result<Vec<u8>, Fault> {
        let encrypted = self
            .streams
            .folders
            .get(index)
            .is_some_and(Folder::encrypted);
        self.unpack_folder(index, password).map_err(|f| match f {
            Fault::Malformed if encrypted => Fault::Password,
            other => other,
        })
    }
    /// Entry `index`'s bytes within its decoded folder, CRC checked.
    pub fn slice(
        &self,
        index: usize,
        folder: &[u8],
        _password: Option<&str>,
    ) -> Result<Vec<u8>, Fault> {
        let e = self.entries.get(index).ok_or(Fault::Malformed)?;
        let Some((f, sub)) = e.stream else {
            return Ok(Vec::new());
        };
        let sizes = &self.streams.folders[f].sizes;
        let start = sizes[..sub].iter().sum::<u64>() as usize;
        let end = start + sizes[sub] as usize;
        let bytes = folder.get(start..end).ok_or(Fault::Malformed)?.to_vec();
        if let Some(crc) = e.crc
            && crc32(&bytes) != crc
        {
            return Err(if e.encrypted {
                Fault::Password
            } else {
                Fault::Malformed
            });
        }
        Ok(bytes)
    }
    /// The bytes of entry `index` (checked against its CRC).
    pub fn read(&self, index: usize, password: Option<&str>) -> Result<Vec<u8>, Fault> {
        let e = self.entries.get(index).ok_or(Fault::Malformed)?;
        let Some((folder, sub)) = e.stream else {
            return Ok(Vec::new());
        };
        let data = self.unpack_folder(folder, password).map_err(|f| match f {
            Fault::Malformed if e.encrypted => Fault::Password,
            other => other,
        })?;
        let f = &self.streams.folders[folder];
        let start: u64 = f.sizes[..sub].iter().sum();
        let start = start as usize;
        let end = start + f.sizes[sub] as usize;
        let bytes = data.get(start..end).ok_or(Fault::Malformed)?.to_vec();
        if let Some(crc) = e.crc
            && crc32(&bytes) != crc
        {
            return Err(if e.encrypted {
                Fault::Password
            } else {
                Fault::Malformed
            });
        }
        Ok(bytes)
    }
}
fn read_files(r: &mut Reader<'_>, streams: &Streams) -> Result<Vec<Entry>, Fault> {
    let n = r.count(MAX_ITEMS)?;
    // Every file needs at least a two-byte name: a count beyond the header
    // is malformed (and would allocate for nothing).
    if n > r.b.len().saturating_sub(r.at) / 2 {
        return Err(Fault::Malformed);
    }
    let mut empty = alloc::vec![false; n];
    let mut empty_file: Vec<bool> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut attributes: Vec<Option<u32>> = alloc::vec![None; n];
    loop {
        let t = r.byte()?;
        if t == 0 {
            break;
        }
        let size = r.count(1 << 30)?;
        let body = r.bytes(size)?;
        let mut p = Reader { b: body, at: 0 };
        match t {
            0x0E => empty = p.bits(n)?,
            0x0F => {
                let count = empty.iter().filter(|e| **e).count();
                empty_file = p.bits(count)?;
            }
            0x11 => {
                if p.byte()? != 0 {
                    return Err(Fault::Unsupported);
                }
                let units: Vec<u16> = body[1..]
                    .chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect();
                for name in units.split(|u| *u == 0).take(n) {
                    names.push(
                        char::decode_utf16(name.iter().copied())
                            .map(|c| c.unwrap_or('\u{FFFD}'))
                            .collect(),
                    );
                }
            }
            0x15 => {
                let defined = p.defined(n)?;
                if p.byte()? != 0 {
                    return Err(Fault::Unsupported);
                }
                for (i, d) in defined.into_iter().enumerate() {
                    if d {
                        attributes[i] = Some(p.u32()?);
                    }
                }
            }
            _ => (),
        }
    }
    if names.len() != n {
        return Err(Fault::Malformed);
    }
    let mut entries = Vec::with_capacity(n);
    let mut folder = 0;
    let mut sub = 0;
    let mut empty_index = 0;
    for i in 0..n {
        let path = names[i].replace('\\', "/");
        if empty[i] {
            let is_file = empty_file.get(empty_index).copied().unwrap_or(false);
            empty_index += 1;
            entries.push(Entry {
                path,
                folder: !is_file,
                size: 0,
                encrypted: false,
                stream: None,
                crc: None,
            });
            continue;
        }
        while streams
            .folders
            .get(folder)
            .is_some_and(|f| sub >= f.sizes.len())
        {
            folder += 1;
            sub = 0;
        }
        let f = streams.folders.get(folder).ok_or(Fault::Malformed)?;
        entries.push(Entry {
            path,
            folder: false,
            size: f.sizes[sub],
            encrypted: f.encrypted(),
            stream: Some((folder, sub)),
            crc: f.crcs.get(sub).copied().flatten(),
        });
        sub += 1;
    }
    Ok(entries)
}

// ---- Writing ----

fn put_number(out: &mut Vec<u8>, v: u64) {
    let mut first = 0u8;
    let mut mask = 0x80u8;
    let mut extra = 0;
    while extra < 8 {
        if v < 1u64 << (7 * (extra + 1)) {
            first |= (v >> (8 * extra)) as u8;
            break;
        }
        first |= mask;
        mask >>= 1;
        extra += 1;
    }
    out.push(first);
    for j in 0..extra {
        out.push((v >> (8 * j)) as u8);
    }
}
fn put_bits(out: &mut Vec<u8>, bits: &[bool]) {
    for chunk in bits.chunks(8) {
        let mut b = 0u8;
        for (i, bit) in chunk.iter().enumerate() {
            if *bit {
                b |= 0x80 >> i;
            }
        }
        out.push(b);
    }
}
/// 7zAES coder properties (cycles 19, a 16-byte salt and IV) and the
/// AES-256-CBC encryption of `data` (zero-padded to whole blocks).
fn aes_encode(data: &[u8], password: &str, salt: [u8; 16], iv: [u8; 16]) -> (Vec<u8>, Vec<u8>) {
    use aes::cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
    const CYCLES: u8 = 19;
    let mut props = alloc::vec![0xC0 | CYCLES, 0xFF];
    props.extend_from_slice(&salt);
    props.extend_from_slice(&iv);
    let key = aes_key(u32::from(CYCLES), &salt, password);
    let cipher = aes::Aes256::new(GenericArray::from_slice(&key));
    let mut out = data.to_vec();
    out.resize(data.len().next_multiple_of(16), 0);
    let mut prev = iv;
    for block in out.chunks_exact_mut(16) {
        for (b, p) in block.iter_mut().zip(prev) {
            *b ^= p;
        }
        cipher.encrypt_block(GenericArray::from_mut_slice(block));
        prev.copy_from_slice(block);
    }
    (props, out)
}
/// One folder's coder list and unpack sizes as 7z header bytes: LZMA, or
/// LZMA then 7zAES (bond: LZMA's input <- AES's output).
fn put_folder(h: &mut Vec<u8>, lzma_props: &[u8; 5], aes: Option<&[u8]>) {
    match aes {
        None => {
            put_number(h, 1);
            h.push(0x23);
            h.extend_from_slice(LZMA);
            put_number(h, 5);
            h.extend_from_slice(lzma_props);
        }
        Some(props) => {
            put_number(h, 2);
            h.push(0x23);
            h.extend_from_slice(LZMA);
            put_number(h, 5);
            h.extend_from_slice(lzma_props);
            h.push(0x24);
            h.extend_from_slice(AES);
            put_number(h, props.len() as u64);
            h.extend_from_slice(props);
            // Bond: in stream 0 (LZMA) is fed by out stream 1 (AES).
            put_number(h, 0);
            put_number(h, 1);
        }
    }
}
/// A 7z archive of `entries` (path, bytes or None for a folder): one solid
/// LZMA folder; with a password, the data and the header (names) are
/// encrypted with 7zAES. `random` supplies salt and IV bytes.
pub fn write(
    entries: &[(String, Option<Vec<u8>>)],
    password: Option<&str>,
    random: impl Fn() -> [u8; 16],
) -> Vec<u8> {
    let files: Vec<&(String, Option<Vec<u8>>)> = entries
        .iter()
        .filter(|(_, d)| d.as_ref().is_some_and(|d| !d.is_empty()))
        .collect();
    let mut solid = Vec::new();
    for (_, d) in &files {
        solid.extend_from_slice(d.as_ref().unwrap());
    }
    let mut packs: Vec<Vec<u8>> = Vec::new();
    let mut h = alloc::vec![0x01u8];
    if !files.is_empty() {
        let (lzma_props, lzma) = lzma::encode_lzma(&solid);
        let lzma_len = lzma.len() as u64;
        let aes = password.map(|p| aes_encode(&lzma, p, random(), random()));
        let (packed, aes_props) = match aes {
            Some((props, data)) => (data, Some(props)),
            None => (lzma, None),
        };
        h.push(0x04);
        h.push(0x06);
        put_number(&mut h, 0);
        put_number(&mut h, 1);
        h.push(0x09);
        put_number(&mut h, packed.len() as u64);
        h.push(0x00);
        h.push(0x07);
        h.push(0x0B);
        put_number(&mut h, 1);
        h.push(0x00);
        put_folder(&mut h, &lzma_props, aes_props.as_deref());
        h.push(0x0C);
        put_number(&mut h, solid.len() as u64);
        if aes_props.is_some() {
            put_number(&mut h, lzma_len);
        }
        h.push(0x00);
        h.push(0x08);
        h.push(0x0D);
        put_number(&mut h, files.len() as u64);
        h.push(0x09);
        for (_, d) in &files[..files.len() - 1] {
            put_number(&mut h, d.as_ref().unwrap().len() as u64);
        }
        h.push(0x0A);
        h.push(0x01);
        for (_, d) in &files {
            h.extend_from_slice(&crc32(d.as_ref().unwrap()).to_le_bytes());
        }
        h.push(0x00);
        h.push(0x00);
        packs.push(packed);
    }
    // Files: data entries first (stream order), then empty files and
    // folders, as 7-Zip lists them.
    let empties: Vec<&(String, Option<Vec<u8>>)> = entries
        .iter()
        .filter(|(_, d)| d.as_ref().is_none_or(|d| d.is_empty()))
        .collect();
    let all: Vec<&(String, Option<Vec<u8>>)> = files
        .iter()
        .copied()
        .chain(empties.iter().copied())
        .collect();
    h.push(0x05);
    put_number(&mut h, all.len() as u64);
    if !empties.is_empty() {
        let stream_bits: Vec<bool> = all
            .iter()
            .map(|(_, d)| d.as_ref().is_none_or(|d| d.is_empty()))
            .collect();
        let mut body = Vec::new();
        put_bits(&mut body, &stream_bits);
        h.push(0x0E);
        put_number(&mut h, body.len() as u64);
        h.extend_from_slice(&body);
        let file_bits: Vec<bool> = empties.iter().map(|(_, d)| d.is_some()).collect();
        let mut body = Vec::new();
        put_bits(&mut body, &file_bits);
        h.push(0x0F);
        put_number(&mut h, body.len() as u64);
        h.extend_from_slice(&body);
    }
    let mut names = alloc::vec![0u8];
    for (path, _) in &all {
        for u in path.encode_utf16() {
            names.extend_from_slice(&u.to_le_bytes());
        }
        names.extend_from_slice(&[0, 0]);
    }
    h.push(0x11);
    put_number(&mut h, names.len() as u64);
    h.extend_from_slice(&names);
    // Attributes: FILE_ATTRIBUTE_DIRECTORY for folders, ARCHIVE for files.
    let mut attrs = alloc::vec![1u8, 0];
    for (_, d) in &all {
        attrs.extend_from_slice(&(if d.is_none() { 0x10u32 } else { 0x20 }).to_le_bytes());
    }
    h.push(0x15);
    put_number(&mut h, attrs.len() as u64);
    h.extend_from_slice(&attrs);
    h.push(0x00);
    h.push(0x00);
    // With a password the header itself is encrypted (names hidden).
    let header = match password {
        Some(p) => {
            let (props, enc) = aes_encode(&h, p, random(), random());
            let pack_pos: u64 = packs.iter().map(|p| p.len() as u64).sum();
            let mut e = alloc::vec![0x17u8, 0x06];
            put_number(&mut e, pack_pos);
            put_number(&mut e, 1);
            e.push(0x09);
            put_number(&mut e, enc.len() as u64);
            e.push(0x00);
            e.push(0x07);
            e.push(0x0B);
            put_number(&mut e, 1);
            e.push(0x00);
            put_number(&mut e, 1);
            e.push(0x24);
            e.extend_from_slice(AES);
            put_number(&mut e, props.len() as u64);
            e.extend_from_slice(&props);
            e.push(0x0C);
            put_number(&mut e, h.len() as u64);
            e.push(0x0A);
            e.push(0x01);
            e.extend_from_slice(&crc32(&h).to_le_bytes());
            e.push(0x00);
            e.push(0x00);
            packs.push(enc);
            e
        }
        None => h,
    };
    let body: Vec<u8> = packs.concat();
    let mut out = Vec::with_capacity(32 + body.len() + header.len());
    out.extend_from_slice(&SIGNATURE);
    out.extend_from_slice(&[0, 4]);
    let mut start = Vec::new();
    start.extend_from_slice(&(body.len() as u64).to_le_bytes());
    start.extend_from_slice(&(header.len() as u64).to_le_bytes());
    start.extend_from_slice(&crc32(&header).to_le_bytes());
    out.extend_from_slice(&crc32(&start).to_le_bytes());
    out.extend_from_slice(&start);
    out.extend_from_slice(&body);
    out.extend_from_slice(&header);
    out
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    fn sample() -> Vec<(String, Option<Vec<u8>>)> {
        let mut text = Vec::new();
        for i in 0..2000 {
            text.extend_from_slice(
                alloc::format!("Line {i}: Doors OS writes 7-Zip archives.\n").as_bytes(),
            );
        }
        let mut noise = Vec::new();
        let mut seed = 7u32;
        for _ in 0..30_000 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            noise.push((seed >> 24) as u8);
        }
        alloc::vec![
            ("docs".into(), None),
            ("docs/readme.txt".into(), Some(text)),
            ("docs/empty.txt".into(), Some(Vec::new())),
            ("noise.bin".into(), Some(noise)),
            ("tiny.txt".into(), Some(b"hi".to_vec())),
        ]
    }
    fn check(archive: &Archive<'_>, entries: &[(String, Option<Vec<u8>>)], password: Option<&str>) {
        for (path, data) in entries {
            let i = archive
                .entries
                .iter()
                .position(|e| &e.path == path)
                .unwrap_or_else(|| panic!("{path}"));
            let e = &archive.entries[i];
            match data {
                None => assert!(e.folder, "{path}"),
                Some(d) => assert_eq!(&archive.read(i, password).unwrap(), d, "{path}"),
            }
        }
    }
    #[test]
    fn written_archives_round_trip_plain_and_encrypted() {
        let entries = sample();
        let plain = write(&entries, None, || [0x11; 16]);
        let a = Archive::open(&plain, None, 1 << 24).unwrap();
        assert!(!a.header_encrypted);
        check(&a, &entries, None);
        // LZMA compresses the text well; noise stays about its size.
        assert!(plain.len() < 40_000, "{}", plain.len());
        let locked = write(&entries, Some("correct horse"), || [0x22; 16]);
        assert_eq!(
            Archive::open(&locked, None, 1 << 24).err(),
            Some(Fault::Password)
        );
        assert!(Archive::open(&locked, Some("wrong"), 1 << 24).is_err());
        let a = Archive::open(&locked, Some("correct horse"), 1 << 24).unwrap();
        assert!(a.header_encrypted);
        check(&a, &entries, Some("correct horse"));
        assert!(a.entries.iter().any(|e| e.encrypted));
    }
    #[test]
    fn lzma_round_trips_edge_cases() {
        for data in [
            Vec::new(),
            b"a".to_vec(),
            alloc::vec![0u8; 100_000],
            b"abcabcabcabcabcabcabcabd".repeat(300),
        ] {
            let (props, packed) = lzma::encode_lzma(&data);
            assert_eq!(
                lzma::decode_lzma(&props, &packed, data.len(), 1 << 24).unwrap(),
                data
            );
        }
    }
    #[test]
    fn damaged_archives_are_refused() {
        let mut bytes = write(&sample(), None, || [0; 16]);
        assert!(Archive::open(&bytes[..20], None, 1 << 24).is_err());
        let n = bytes.len();
        bytes[n - 3] ^= 0xFF;
        assert!(Archive::open(&bytes, None, 1 << 24).is_err());
    }
    /// Archives from an independent writer (Windows' bsdtar, libarchive):
    /// `DOOR_7Z_REF=<dir with ref-*.7z and src/>`.
    #[test]
    #[ignore = "needs DOOR_7Z_REF reference archives"]
    fn reads_reference_archives() {
        let dir = std::env::var("DOOR_7Z_REF").expect("DOOR_7Z_REF");
        for method in ["lzma2", "lzma1", "deflate", "store"] {
            let bytes = std::fs::read(std::format!("{dir}/ref-{method}.7z")).unwrap();
            let a =
                Archive::open(&bytes, None, 1 << 26).unwrap_or_else(|e| panic!("{method}: {e:?}"));
            for (i, e) in a.entries.iter().enumerate() {
                if e.folder || e.size == 0 {
                    continue;
                }
                let want = std::fs::read(std::format!(
                    "{dir}/src/{}",
                    e.path.trim_start_matches("./")
                ))
                .unwrap();
                assert_eq!(
                    a.read(i, None)
                        .unwrap_or_else(|f| panic!("{method} {}: {f:?}", e.path)),
                    want,
                    "{method} {}",
                    e.path
                );
            }
        }
        // Ours, for the reference reader to list and extract.
        let ours = write(&sample(), None, || [0x33; 16]);
        std::fs::write(std::format!("{dir}/door.7z"), ours).unwrap();
    }
}

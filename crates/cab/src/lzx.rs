//! LZX as Microsoft cabinets use it (MS-PATCH 2.2's LZX DELTA without
//! deltas, and the format libmspack documents): a 2^15..2^21-byte window,
//! verbatim, aligned-offset and uncompressed blocks that may span frames,
//! repeated-offset registers R0..R2 and the E8 call translation. A folder
//! decodes frame by frame: each CFDATA block holds one 32 KiB frame (the
//! last may be shorter) whose bits start on its own.
use alloc::{vec, vec::Vec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error;

const FRAME: usize = 32 * 1024;
const PRETREE: usize = 20;
const LENGTH_TREE: usize = 249;
const ALIGNED_TREE: usize = 8;
const MIN_MATCH: usize = 2;
const PRIMARY_LENGTHS: usize = 7;

/// Bits from 16-bit little-endian words, most significant first.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buffer: u32,
    left: u32,
}
impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            buffer: 0,
            left: 0,
        }
    }
    fn word(&mut self) -> u32 {
        // Past the end, zeros (the encoder's last word may be short).
        let lo = u32::from(*self.data.get(self.pos).unwrap_or(&0));
        let hi = u32::from(*self.data.get(self.pos + 1).unwrap_or(&0));
        self.pos += 2;
        lo | hi << 8
    }
    fn ensure(&mut self, n: u32) -> Result<(), Error> {
        while self.left < n {
            // Allow at most one word of zero padding past the data.
            if self.pos >= self.data.len() + 2 {
                return Err(Error);
            }
            let w = self.word();
            self.buffer |= w << (16 - self.left);
            self.left += 16;
        }
        Ok(())
    }
    fn peek(&mut self, n: u32) -> Result<u32, Error> {
        self.ensure(n)?;
        Ok(self.buffer >> (32 - n))
    }
    fn remove(&mut self, n: u32) {
        self.buffer = if n == 32 { 0 } else { self.buffer << n };
        self.left -= n;
    }
    fn take(&mut self, n: u32) -> Result<u32, Error> {
        if n == 0 {
            return Ok(0);
        }
        if n > 16 {
            let hi = self.take(16)?;
            let lo = self.take(n - 16)?;
            return Ok(hi << (n - 16) | lo);
        }
        let v = self.peek(n)?;
        self.remove(n);
        Ok(v)
    }
    /// Drop the rest of the current 16-bit word (uncompressed blocks start
    /// on a word boundary); a whole buffered word is given back.
    fn align(&mut self) {
        if self.left > 16 {
            self.pos -= 2;
        }
        self.buffer = 0;
        self.left = 0;
    }
    fn byte(&mut self) -> Result<u8, Error> {
        let b = *self.data.get(self.pos).ok_or(Error)?;
        self.pos += 1;
        Ok(b)
    }
}

/// A canonical Huffman code decoded a bit at a time.
struct Tree {
    counts: [u16; 17],
    symbols: Vec<u16>,
}
impl Tree {
    fn new(lengths: &[u8]) -> Result<Self, Error> {
        let mut counts = [0u16; 17];
        for &l in lengths {
            if l > 16 {
                return Err(Error);
            }
            counts[usize::from(l)] += 1;
        }
        counts[0] = 0;
        let mut left = 1i32;
        for c in &counts[1..] {
            left = (left << 1) - i32::from(*c);
            if left < 0 {
                return Err(Error);
            }
        }
        let mut offsets = [0u16; 17];
        for len in 1..16 {
            offsets[len + 1] = offsets[len] + counts[len];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (symbol, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[usize::from(offsets[usize::from(l)])] = symbol as u16;
                offsets[usize::from(l)] += 1;
            }
        }
        Ok(Self { counts, symbols })
    }
    fn decode(&self, bits: &mut Bits) -> Result<u16, Error> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..=16 {
            code |= bits.take(1)? as i32;
            let count = i32::from(self.counts[len]);
            if code - count < first {
                return self
                    .symbols
                    .get((index + code - first) as usize)
                    .copied()
                    .ok_or(Error);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(Error)
    }
}

fn tables() -> ([u8; 51], [u32; 51]) {
    let mut extra = [0u8; 51];
    let mut j = 0u8;
    let mut i = 0;
    while i < 51 {
        extra[i] = j;
        if i + 1 < 51 {
            extra[i + 1] = j;
        }
        if i != 0 && j < 17 {
            j += 1;
        }
        i += 2;
    }
    let mut base = [0u32; 51];
    let mut b = 0u32;
    for i in 0..51 {
        base[i] = b;
        b += 1 << extra[i];
    }
    (extra, base)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    None,
    Verbatim,
    Aligned,
    Uncompressed,
}

/// A folder's decoder: its window and block state persist across frames.
pub struct Decoder {
    window: Vec<u8>,
    position: usize,
    /// Bytes produced so far (offsets may not reach before the start).
    produced: u64,
    slots: usize,
    main_lengths: Vec<u8>,
    length_lengths: [u8; LENGTH_TREE],
    aligned_lengths: [u8; ALIGNED_TREE],
    main: Option<Tree>,
    length: Option<Tree>,
    aligned: Option<Tree>,
    r: [u32; 3],
    kind: Kind,
    remaining: usize,
    odd_block: bool,
    header_read: bool,
    intel_size: u32,
    intel_started: bool,
    intel_position: u32,
    frame: u32,
    /// Bytes a match ending the previous frame produced into this one.
    pending: usize,
    extra: [u8; 51],
    base: [u32; 51],
}
impl Decoder {
    pub fn new(window_bits: u8) -> Self {
        let slots = match window_bits {
            15 => 30,
            16 => 32,
            17 => 34,
            18 => 36,
            19 => 38,
            20 => 42,
            _ => 50,
        };
        let (extra, base) = tables();
        Self {
            window: vec![0; 1 << window_bits],
            position: 0,
            produced: 0,
            slots,
            main_lengths: vec![0; 256 + slots * 8],
            length_lengths: [0; LENGTH_TREE],
            aligned_lengths: [0; ALIGNED_TREE],
            main: None,
            length: None,
            aligned: None,
            r: [1, 1, 1],
            kind: Kind::None,
            remaining: 0,
            odd_block: false,
            header_read: false,
            intel_size: 0,
            intel_started: false,
            intel_position: 0,
            frame: 0,
            pending: 0,
            extra,
            base,
        }
    }
    /// Delta-coded lengths `lengths[first..last]` with a fresh pretree.
    fn read_lengths(
        bits: &mut Bits,
        lengths: &mut [u8],
        first: usize,
        last: usize,
    ) -> Result<(), Error> {
        let mut pre = [0u8; PRETREE];
        for p in &mut pre {
            *p = bits.take(4)? as u8;
        }
        let pretree = Tree::new(&pre)?;
        let mut x = first;
        while x < last {
            let z = pretree.decode(bits)?;
            match z {
                17 => {
                    let run = bits.take(4)? as usize + 4;
                    for _ in 0..run {
                        *lengths.get_mut(x).ok_or(Error)? = 0;
                        x += 1;
                    }
                }
                18 => {
                    let run = bits.take(5)? as usize + 20;
                    for _ in 0..run {
                        *lengths.get_mut(x).ok_or(Error)? = 0;
                        x += 1;
                    }
                }
                19 => {
                    let run = bits.take(1)? as usize + 4;
                    let delta = pretree.decode(bits)?;
                    if delta > 16 {
                        return Err(Error);
                    }
                    let value =
                        (17 + i32::from(*lengths.get(x).ok_or(Error)?) - i32::from(delta)) % 17;
                    for _ in 0..run {
                        *lengths.get_mut(x).ok_or(Error)? = value as u8;
                        x += 1;
                    }
                }
                0..=16 => {
                    let value = (17 + i32::from(lengths[x]) - i32::from(z)) % 17;
                    lengths[x] = value as u8;
                    x += 1;
                }
                _ => return Err(Error),
            }
        }
        Ok(())
    }
    fn block_header(&mut self, bits: &mut Bits) -> Result<(), Error> {
        let kind = bits.take(3)?;
        let size = (bits.take(16)? << 8 | bits.take(8)?) as usize;
        self.remaining = size;
        self.odd_block = size & 1 == 1;
        match kind {
            2 => {
                for l in &mut self.aligned_lengths {
                    *l = bits.take(3)? as u8;
                }
                self.aligned = Some(Tree::new(&self.aligned_lengths)?);
                self.kind = Kind::Aligned;
                self.read_trees(bits)?;
            }
            1 => {
                self.kind = Kind::Verbatim;
                self.read_trees(bits)?;
            }
            3 => {
                self.intel_started = true;
                self.kind = Kind::Uncompressed;
                bits.align();
                for r in &mut self.r {
                    let mut b = [0u8; 4];
                    for x in &mut b {
                        *x = bits.byte()?;
                    }
                    *r = u32::from_le_bytes(b);
                }
            }
            _ => return Err(Error),
        }
        Ok(())
    }
    fn read_trees(&mut self, bits: &mut Bits) -> Result<(), Error> {
        let elements = 256 + self.slots * 8;
        Self::read_lengths(bits, &mut self.main_lengths, 0, 256)?;
        Self::read_lengths(bits, &mut self.main_lengths, 256, elements)?;
        self.main = Some(Tree::new(&self.main_lengths)?);
        if self.main_lengths[0xe8] != 0 {
            self.intel_started = true;
        }
        Self::read_lengths(bits, &mut self.length_lengths, 0, LENGTH_TREE)?;
        self.length = Some(Tree::new(&self.length_lengths)?);
        Ok(())
    }
    fn put(&mut self, b: u8) {
        let size = self.window.len();
        self.window[self.position] = b;
        self.position = (self.position + 1) % size;
        self.produced += 1;
    }
    /// Decode one frame of `size` bytes from `data` into `out`.
    pub fn block(&mut self, data: &[u8], size: usize, out: &mut Vec<u8>) -> Result<(), Error> {
        if size > FRAME || size == 0 {
            return Err(Error);
        }
        let mut bits = Bits::new(data);
        if !self.header_read {
            if bits.take(1)? == 1 {
                let hi = bits.take(16)?;
                let lo = bits.take(16)?;
                self.intel_size = hi << 16 | lo;
            }
            self.header_read = true;
        }
        let window = self.window.len();
        let start = (self.position + window - self.pending) % window;
        // A match may run past the frame end (by up to 257 bytes): those
        // bytes open the next frame.
        let mut done = core::mem::take(&mut self.pending);
        if done > size {
            return Err(Error);
        }
        while done < size {
            if self.remaining == 0 {
                // An odd-length uncompressed block is padded to a word.
                if self.kind == Kind::Uncompressed && self.odd_block {
                    bits.byte()?;
                }
                self.block_header(&mut bits)?;
                if self.remaining == 0 {
                    return Err(Error);
                }
            }
            let want = (size - done).min(self.remaining);
            let made = match self.kind {
                Kind::Uncompressed => {
                    for _ in 0..want {
                        let b = bits.byte()?;
                        self.put(b);
                    }
                    want
                }
                Kind::Verbatim | Kind::Aligned => self.codes(&mut bits, want)?,
                Kind::None => return Err(Error),
            };
            done += made;
            self.remaining -= made.min(self.remaining);
        }
        self.pending = done - size;
        if self.pending > 257 {
            return Err(Error);
        }
        // The frame's bytes, then the E8 translation on them.
        let mut frame = Vec::with_capacity(size);
        for i in 0..size {
            frame.push(self.window[(start + i) % self.window.len()]);
        }
        if self.intel_started && self.intel_size != 0 && self.frame < 32768 && size > 10 {
            translate(&mut frame, self.intel_position, self.intel_size);
        }
        self.intel_position = self.intel_position.wrapping_add(size as u32);
        self.frame += 1;
        out.extend_from_slice(&frame);
        Ok(())
    }
    /// Literals and matches until about `want` bytes are produced (a
    /// match may run past it into the next frame's bytes; it never crosses
    /// the block).
    fn codes(&mut self, bits: &mut Bits, want: usize) -> Result<usize, Error> {
        let mut made = 0;
        while made < want {
            let main = self.main.as_ref().ok_or(Error)?.decode(bits)? as usize;
            if main < 256 {
                self.put(main as u8);
                made += 1;
                continue;
            }
            let element = main - 256;
            let mut length = element & 7;
            if length == PRIMARY_LENGTHS {
                length += self.length.as_ref().ok_or(Error)?.decode(bits)? as usize;
            }
            length += MIN_MATCH;
            let slot = element >> 3;
            let offset = if slot > 2 {
                let extra = self.extra[slot];
                let base = self.base[slot];
                let offset = if self.kind == Kind::Aligned {
                    let mut o = base.wrapping_sub(2);
                    if extra > 3 {
                        o += bits.take(u32::from(extra - 3))? << 3;
                        o += u32::from(self.aligned.as_ref().ok_or(Error)?.decode(bits)?);
                        o
                    } else if extra == 3 {
                        o + u32::from(self.aligned.as_ref().ok_or(Error)?.decode(bits)?)
                    } else if extra > 0 {
                        o + bits.take(u32::from(extra))?
                    } else {
                        o = 1;
                        o
                    }
                } else if slot == 3 {
                    1
                } else {
                    base - 2 + bits.take(u32::from(extra))?
                };
                self.r = [offset, self.r[0], self.r[1]];
                offset
            } else if slot == 0 {
                self.r[0]
            } else if slot == 1 {
                self.r.swap(0, 1);
                self.r[0]
            } else {
                self.r.swap(0, 2);
                self.r[0]
            };
            let offset = offset as usize;
            if offset == 0 || offset as u64 > self.produced || offset > self.window.len() {
                return Err(Error);
            }
            // A match never runs past its block (it may past the frame).
            if length > self.remaining.saturating_sub(made) {
                return Err(Error);
            }
            let size = self.window.len();
            for _ in 0..length {
                let b = self.window[(self.position + size - offset) % size];
                self.put(b);
            }
            made += length;
        }
        Ok(made)
    }
}

/// Undo the encoder's E8 (CALL) translation in one frame starting at
/// stream position `position`.
fn translate(frame: &mut [u8], position: u32, file_size: u32) {
    let end = frame.len() - 10;
    let mut i = 0;
    let mut current = position as i64;
    let size = i64::from(file_size);
    while i < end {
        if frame[i] != 0xe8 {
            i += 1;
            current += 1;
            continue;
        }
        let absolute = i64::from(i32::from_le_bytes(frame[i + 1..i + 5].try_into().unwrap()));
        if absolute >= -current && absolute < size {
            let relative = if absolute >= 0 {
                absolute - current
            } else {
                absolute + size
            };
            frame[i + 1..i + 5].copy_from_slice(&(relative as i32).to_le_bytes());
        }
        i += 5;
        current += 5;
    }
}

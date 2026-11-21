//! DEFLATE (RFC 1951) with a window that outlives a block, as MSZIP needs:
//! each CFDATA block of a folder is a complete deflate stream whose
//! back-references may reach into the previous blocks' 32 KiB of output.

pub const WINDOW: usize = 32 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The stream ended early or holds an invalid code.
    Corrupt,
    /// The output would exceed what the block declared.
    Overflow,
}

struct Bits<'a> {
    data: &'a [u8],
    at: usize,
    bit: u32,
    held: u32,
}
impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            at: 0,
            bit: 0,
            held: 0,
        }
    }
    fn need(&mut self, n: u32) -> Result<(), Error> {
        while self.bit < n {
            let byte = *self.data.get(self.at).ok_or(Error::Corrupt)?;
            self.at += 1;
            self.held |= u32::from(byte) << self.bit;
            self.bit += 8;
        }
        Ok(())
    }
    fn take(&mut self, n: u32) -> Result<u32, Error> {
        if n == 0 {
            return Ok(0);
        }
        self.need(n)?;
        let v = self.held & ((1u32 << n) - 1);
        self.held = if n == 32 { 0 } else { self.held >> n };
        self.bit -= n;
        Ok(v)
    }
    fn align(&mut self) {
        let drop = self.bit % 8;
        self.held >>= drop;
        self.bit -= drop;
    }
}

/// A canonical Huffman code: counts per length and symbols by code order.
struct Huffman {
    counts: [u16; 16],
    symbols: [u16; 320],
}
impl Huffman {
    fn new(lengths: &[u8]) -> Result<Self, Error> {
        let mut h = Self {
            counts: [0; 16],
            symbols: [0; 320],
        };
        for &l in lengths {
            h.counts[usize::from(l)] += 1;
        }
        h.counts[0] = 0;
        // Over-subscribed codes are invalid; incomplete ones are allowed
        // (a single distance code).
        let mut left = 1i32;
        for len in 1..16 {
            left <<= 1;
            left -= i32::from(h.counts[len]);
            if left < 0 {
                return Err(Error::Corrupt);
            }
        }
        let mut offsets = [0u16; 16];
        for len in 1..15 {
            offsets[len + 1] = offsets[len] + h.counts[len];
        }
        for (symbol, &l) in lengths.iter().enumerate() {
            if l != 0 {
                h.symbols[usize::from(offsets[usize::from(l)])] = symbol as u16;
                offsets[usize::from(l)] += 1;
            }
        }
        Ok(h)
    }
    fn decode(&self, bits: &mut Bits) -> Result<u16, Error> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= bits.take(1)? as i32;
            let count = i32::from(self.counts[len]);
            if code - count < first {
                return Ok(self.symbols[(index + (code - first)) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(Error::Corrupt)
    }
}

const LENGTH_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LENGTH_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DISTANCE_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DISTANCE_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// A decoder whose window persists from one stream to the next.
pub struct Inflater {
    window: alloc::vec::Vec<u8>,
}
impl Default for Inflater {
    fn default() -> Self {
        Self::new()
    }
}
impl Inflater {
    pub fn new() -> Self {
        Self {
            window: alloc::vec::Vec::new(),
        }
    }
    /// Inflate one complete deflate stream into `out` (at most `limit`
    /// bytes); back-references may reach the earlier streams' output.
    pub fn inflate(
        &mut self,
        data: &[u8],
        limit: usize,
        out: &mut alloc::vec::Vec<u8>,
    ) -> Result<(), Error> {
        let start = out.len();
        let mut bits = Bits::new(data);
        loop {
            let last = bits.take(1)? == 1;
            match bits.take(2)? {
                0 => {
                    bits.align();
                    let len = bits.take(16)? as usize;
                    let nlen = bits.take(16)? as usize;
                    if len != !nlen & 0xffff {
                        return Err(Error::Corrupt);
                    }
                    for _ in 0..len {
                        let b = bits.take(8)? as u8;
                        self.emit(out, start, limit, b)?;
                    }
                }
                1 => {
                    let mut lengths = [0u8; 288];
                    lengths[..144].fill(8);
                    lengths[144..256].fill(9);
                    lengths[256..280].fill(7);
                    lengths[280..].fill(8);
                    let literal = Huffman::new(&lengths)?;
                    let distance = Huffman::new(&[5u8; 30])?;
                    self.codes(&mut bits, &literal, &distance, out, start, limit)?;
                }
                2 => {
                    let (literal, distance) = dynamic(&mut bits)?;
                    self.codes(&mut bits, &literal, &distance, out, start, limit)?;
                }
                _ => return Err(Error::Corrupt),
            }
            if last {
                break;
            }
        }
        // Keep the last 32 KiB as the next stream's history.
        let total = self.window.len();
        if total > WINDOW {
            self.window.drain(..total - WINDOW);
        }
        Ok(())
    }
    fn emit(
        &mut self,
        out: &mut alloc::vec::Vec<u8>,
        start: usize,
        limit: usize,
        b: u8,
    ) -> Result<(), Error> {
        if out.len() - start >= limit {
            return Err(Error::Overflow);
        }
        out.push(b);
        self.window.push(b);
        if self.window.len() > 2 * WINDOW {
            self.window.drain(..WINDOW);
        }
        Ok(())
    }
    fn codes(
        &mut self,
        bits: &mut Bits,
        literal: &Huffman,
        distance: &Huffman,
        out: &mut alloc::vec::Vec<u8>,
        start: usize,
        limit: usize,
    ) -> Result<(), Error> {
        loop {
            let symbol = literal.decode(bits)?;
            match symbol {
                0..=255 => self.emit(out, start, limit, symbol as u8)?,
                256 => return Ok(()),
                257..=285 => {
                    let i = usize::from(symbol - 257);
                    let length = usize::from(LENGTH_BASE[i])
                        + bits.take(u32::from(LENGTH_EXTRA[i]))? as usize;
                    let d = usize::from(distance.decode(bits)?);
                    if d >= 30 {
                        return Err(Error::Corrupt);
                    }
                    let back = usize::from(DISTANCE_BASE[d])
                        + bits.take(u32::from(DISTANCE_EXTRA[d]))? as usize;
                    if back > self.window.len() {
                        return Err(Error::Corrupt);
                    }
                    for _ in 0..length {
                        let b = self.window[self.window.len() - back];
                        self.emit(out, start, limit, b)?;
                    }
                }
                _ => return Err(Error::Corrupt),
            }
        }
    }
}
fn dynamic(bits: &mut Bits) -> Result<(Huffman, Huffman), Error> {
    const ORDER: [usize; 19] = [
        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
    ];
    let nlen = bits.take(5)? as usize + 257;
    let ndist = bits.take(5)? as usize + 1;
    let ncode = bits.take(4)? as usize + 4;
    if nlen > 286 || ndist > 30 {
        return Err(Error::Corrupt);
    }
    let mut code_lengths = [0u8; 19];
    for &i in &ORDER[..ncode] {
        code_lengths[i] = bits.take(3)? as u8;
    }
    let code = Huffman::new(&code_lengths)?;
    let mut lengths = [0u8; 316];
    let mut n = 0;
    while n < nlen + ndist {
        let symbol = code.decode(bits)?;
        let (value, repeat) = match symbol {
            0..=15 => (symbol as u8, 1),
            16 => {
                let previous = *lengths[..n].last().ok_or(Error::Corrupt)?;
                (previous, 3 + bits.take(2)? as usize)
            }
            17 => (0, 3 + bits.take(3)? as usize),
            18 => (0, 11 + bits.take(7)? as usize),
            _ => return Err(Error::Corrupt),
        };
        if n + repeat > nlen + ndist {
            return Err(Error::Corrupt);
        }
        lengths[n..n + repeat].fill(value);
        n += repeat;
    }
    if lengths[256] == 0 {
        return Err(Error::Corrupt);
    }
    Ok((
        Huffman::new(&lengths[..nlen])?,
        Huffman::new(&lengths[nlen..nlen + ndist])?,
    ))
}

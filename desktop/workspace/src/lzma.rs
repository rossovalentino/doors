//! LZMA and LZMA2 decoding (the 7-Zip formats' main coders), from the
//! public LZMA specification (Igor Pavlov's `lzma-specification.txt`,
//! public domain): the range decoder, literal/match/rep coding and the
//! LZMA2 chunk framing. Output goes to one contiguous buffer, which is the
//! dictionary; every distance is checked against it.
extern crate alloc;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Corrupt data: a bad range-coder state, a distance before the start,
    /// or a chunk that disagrees with its sizes.
    Corrupt,
    /// The decoded size would pass the caller's limit.
    TooLarge,
}

const PROB_INIT: u16 = 1024;
struct Range<'a> {
    input: &'a [u8],
    at: usize,
    range: u32,
    code: u32,
}
impl<'a> Range<'a> {
    fn new(input: &'a [u8]) -> Result<Self, Error> {
        if input.len() < 5 || input[0] != 0 {
            return Err(Error::Corrupt);
        }
        let code = u32::from_be_bytes([input[1], input[2], input[3], input[4]]);
        let r = Self {
            input,
            at: 5,
            range: 0xFFFF_FFFF,
            code,
        };
        if r.code == r.range {
            return Err(Error::Corrupt);
        }
        Ok(r)
    }
    #[inline]
    fn next(&mut self) -> u8 {
        // Reading past the end yields zeros; a valid stream never needs
        // them, and the output-size checks end a corrupt one.
        let b = self.input.get(self.at).copied().unwrap_or(0);
        self.at += 1;
        b
    }
    #[inline]
    fn normalize(&mut self) {
        if self.range < 1 << 24 {
            self.range <<= 8;
            self.code = (self.code << 8) | u32::from(self.next());
        }
    }
    #[inline]
    fn bit(&mut self, prob: &mut u16) -> u32 {
        let bound = (self.range >> 11) * u32::from(*prob);
        let bit = if self.code < bound {
            *prob += (2048 - *prob) >> 5;
            self.range = bound;
            0
        } else {
            *prob -= *prob >> 5;
            self.code -= bound;
            self.range -= bound;
            1
        };
        self.normalize();
        bit
    }
    fn direct(&mut self, n: u32) -> u32 {
        let mut res = 0u32;
        for _ in 0..n {
            self.range >>= 1;
            self.code = self.code.wrapping_sub(self.range);
            let t = 0u32.wrapping_sub(self.code >> 31);
            self.code = self.code.wrapping_add(self.range & t);
            self.normalize();
            res = (res << 1).wrapping_add(t.wrapping_add(1));
        }
        res
    }
    fn tree(&mut self, probs: &mut [u16], bits: u32) -> u32 {
        let mut m = 1usize;
        for _ in 0..bits {
            m = (m << 1) + self.bit(&mut probs[m]) as usize;
        }
        (m - (1 << bits)) as u32
    }
    fn reverse(&mut self, probs: &mut [u16], bits: u32) -> u32 {
        let mut m = 1usize;
        let mut sym = 0u32;
        for i in 0..bits {
            let b = self.bit(&mut probs[m]);
            m = (m << 1) + b as usize;
            sym |= b << i;
        }
        sym
    }
}

struct Lengths {
    choice: u16,
    choice2: u16,
    low: [[u16; 8]; 16],
    mid: [[u16; 8]; 16],
    high: [u16; 256],
}
impl Lengths {
    fn new() -> Self {
        Self {
            choice: PROB_INIT,
            choice2: PROB_INIT,
            low: [[PROB_INIT; 8]; 16],
            mid: [[PROB_INIT; 8]; 16],
            high: [PROB_INIT; 256],
        }
    }
    fn decode(&mut self, rc: &mut Range<'_>, pos_state: usize) -> u32 {
        if rc.bit(&mut self.choice) == 0 {
            return rc.tree(&mut self.low[pos_state], 3);
        }
        if rc.bit(&mut self.choice2) == 0 {
            return 8 + rc.tree(&mut self.mid[pos_state], 3);
        }
        16 + rc.tree(&mut self.high, 8)
    }
}

/// LZMA properties: literal context and position bits, position bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Props {
    pub lc: u32,
    pub lp: u32,
    pub pb: u32,
}
impl Props {
    pub fn from_byte(b: u8) -> Result<Self, Error> {
        if b >= 9 * 5 * 5 {
            return Err(Error::Corrupt);
        }
        let b = u32::from(b);
        Ok(Self {
            lc: b % 9,
            lp: (b / 9) % 5,
            pb: b / 45,
        })
    }
}

/// The adaptive state of one LZMA stream.
struct State {
    props: Props,
    literal: Vec<u16>,
    is_match: [u16; 192],
    is_rep: [u16; 12],
    is_rep_g0: [u16; 12],
    is_rep_g1: [u16; 12],
    is_rep_g2: [u16; 12],
    is_rep0_long: [u16; 192],
    pos_slot: [[u16; 64]; 4],
    pos_special: [u16; 115],
    align: [u16; 16],
    len: Lengths,
    rep_len: Lengths,
    state: usize,
    reps: [u32; 4],
}
impl State {
    fn new(props: Props) -> Self {
        Self {
            props,
            literal: alloc::vec![PROB_INIT; 0x300 << (props.lc + props.lp)],
            is_match: [PROB_INIT; 192],
            is_rep: [PROB_INIT; 12],
            is_rep_g0: [PROB_INIT; 12],
            is_rep_g1: [PROB_INIT; 12],
            is_rep_g2: [PROB_INIT; 12],
            is_rep0_long: [PROB_INIT; 192],
            pos_slot: [[PROB_INIT; 64]; 4],
            pos_special: [PROB_INIT; 115],
            align: [PROB_INIT; 16],
            len: Lengths::new(),
            rep_len: Lengths::new(),
            state: 0,
            reps: [0; 4],
        }
    }
    fn distance(&mut self, rc: &mut Range<'_>, len: u32) -> u32 {
        let len_state = len.min(3) as usize;
        let slot = rc.tree(&mut self.pos_slot[len_state], 6);
        if slot < 4 {
            return slot;
        }
        let direct = (slot >> 1) - 1;
        let mut dist = (2 | (slot & 1)) << direct;
        if slot < 14 {
            let base = (dist - slot) as usize;
            dist += rc.reverse(&mut self.pos_special[base..], direct);
        } else {
            dist = dist.wrapping_add(rc.direct(direct - 4) << 4);
            dist = dist.wrapping_add(rc.reverse(&mut self.align, 4));
        }
        dist
    }
    /// Decode into `out` until it holds `end` bytes or the end marker;
    /// `floor` is where this stream's dictionary starts.
    fn run(
        &mut self,
        rc: &mut Range<'_>,
        out: &mut Vec<u8>,
        end: usize,
        floor: usize,
        marker_ok: bool,
    ) -> Result<bool, Error> {
        let pb_mask = (1usize << self.props.pb) - 1;
        let lp_mask = (1usize << self.props.lp) - 1;
        let lc = self.props.lc;
        while out.len() < end {
            let pos = out.len() - floor;
            let pos_state = pos & pb_mask;
            let s = self.state;
            if rc.bit(&mut self.is_match[(s << 4) + pos_state]) == 0 {
                let prev = if out.len() > floor {
                    out[out.len() - 1]
                } else {
                    0
                };
                let lit_state = ((pos & lp_mask) << lc) + (usize::from(prev) >> (8 - lc));
                let probs = &mut self.literal[0x300 * lit_state..0x300 * lit_state + 0x300];
                let mut symbol = 1usize;
                if s >= 7 {
                    let back = self.reps[0] as usize + 1;
                    if back > out.len() - floor {
                        return Err(Error::Corrupt);
                    }
                    let mut match_byte = usize::from(out[out.len() - back]);
                    while symbol < 0x100 {
                        let match_bit = (match_byte >> 7) & 1;
                        match_byte <<= 1;
                        let bit = rc.bit(&mut probs[((1 + match_bit) << 8) + symbol]) as usize;
                        symbol = (symbol << 1) | bit;
                        if match_bit != bit {
                            break;
                        }
                    }
                }
                while symbol < 0x100 {
                    symbol = (symbol << 1) | rc.bit(&mut probs[symbol]) as usize;
                }
                out.push((symbol - 0x100) as u8);
                self.state = if s < 4 {
                    0
                } else if s < 10 {
                    s - 3
                } else {
                    s - 6
                };
                continue;
            }
            let len;
            if rc.bit(&mut self.is_rep[s]) != 0 {
                if out.len() == floor {
                    return Err(Error::Corrupt);
                }
                if rc.bit(&mut self.is_rep_g0[s]) == 0 {
                    if rc.bit(&mut self.is_rep0_long[(s << 4) + pos_state]) == 0 {
                        // Short rep: one byte at rep0.
                        self.state = if s < 7 { 9 } else { 11 };
                        let back = self.reps[0] as usize + 1;
                        if back > out.len() - floor {
                            return Err(Error::Corrupt);
                        }
                        out.push(out[out.len() - back]);
                        continue;
                    }
                } else {
                    let dist;
                    if rc.bit(&mut self.is_rep_g1[s]) == 0 {
                        dist = self.reps[1];
                    } else {
                        if rc.bit(&mut self.is_rep_g2[s]) == 0 {
                            dist = self.reps[2];
                        } else {
                            dist = self.reps[3];
                            self.reps[3] = self.reps[2];
                        }
                        self.reps[2] = self.reps[1];
                    }
                    self.reps[1] = self.reps[0];
                    self.reps[0] = dist;
                }
                len = self.rep_len.decode(rc, pos_state);
                self.state = if s < 7 { 8 } else { 11 };
            } else {
                self.reps[3] = self.reps[2];
                self.reps[2] = self.reps[1];
                self.reps[1] = self.reps[0];
                len = self.len.decode(rc, pos_state);
                self.state = if s < 7 { 7 } else { 10 };
                self.reps[0] = self.distance(rc, len);
                if self.reps[0] == 0xFFFF_FFFF {
                    // End marker.
                    return if marker_ok {
                        Ok(true)
                    } else {
                        Err(Error::Corrupt)
                    };
                }
            }
            let back = self.reps[0] as usize + 1;
            if back > out.len() - floor {
                return Err(Error::Corrupt);
            }
            let n = (len as usize + 2).min(end - out.len());
            for _ in 0..n {
                out.push(out[out.len() - back]);
            }
        }
        Ok(false)
    }
}

/// Decode a raw LZMA stream (7z coder 030101: five property bytes, no
/// size header) producing exactly `size` bytes.
pub fn decode_lzma(
    props: &[u8],
    input: &[u8],
    size: usize,
    limit: usize,
) -> Result<Vec<u8>, Error> {
    if props.len() < 5 {
        return Err(Error::Corrupt);
    }
    if size > limit {
        return Err(Error::TooLarge);
    }
    let p = Props::from_byte(props[0])?;
    let mut out = Vec::new();
    out.try_reserve_exact(size).map_err(|_| Error::TooLarge)?;
    let mut rc = Range::new(input)?;
    let mut state = State::new(p);
    state.run(&mut rc, &mut out, size, 0, true)?;
    if out.len() != size {
        return Err(Error::Corrupt);
    }
    Ok(out)
}
/// Decode an LZMA2 stream (7z coder 21) of `size` bytes.
pub fn decode_lzma2(input: &[u8], size: usize, limit: usize) -> Result<Vec<u8>, Error> {
    if size > limit {
        return Err(Error::TooLarge);
    }
    let mut out = Vec::new();
    out.try_reserve_exact(size).map_err(|_| Error::TooLarge)?;
    let mut at = 0;
    let mut state: Option<State> = None;
    let mut floor = 0;
    loop {
        let control = *input.get(at).ok_or(Error::Corrupt)?;
        at += 1;
        if control == 0 {
            break;
        }
        if control == 1 || control == 2 {
            if control == 1 {
                floor = out.len();
            }
            let n = usize::from(u16::from_be_bytes([
                *input.get(at).ok_or(Error::Corrupt)?,
                *input.get(at + 1).ok_or(Error::Corrupt)?,
            ])) + 1;
            at += 2;
            let chunk = input.get(at..at + n).ok_or(Error::Corrupt)?;
            if out.len() + n > size {
                return Err(Error::Corrupt);
            }
            out.extend_from_slice(chunk);
            at += n;
            continue;
        }
        if control < 0x80 {
            return Err(Error::Corrupt);
        }
        let unpacked = ((usize::from(control & 0x1F) << 16)
            | (usize::from(*input.get(at).ok_or(Error::Corrupt)?) << 8)
            | usize::from(*input.get(at + 1).ok_or(Error::Corrupt)?))
            + 1;
        let packed = usize::from(u16::from_be_bytes([
            *input.get(at + 2).ok_or(Error::Corrupt)?,
            *input.get(at + 3).ok_or(Error::Corrupt)?,
        ])) + 1;
        at += 4;
        let reset = (control >> 5) & 3;
        if reset == 3 {
            floor = out.len();
        }
        if reset >= 2 {
            let props = Props::from_byte(*input.get(at).ok_or(Error::Corrupt)?)?;
            at += 1;
            if props.lc + props.lp > 4 {
                return Err(Error::Corrupt);
            }
            state = Some(State::new(props));
        } else if reset == 1 {
            let props = state.as_ref().ok_or(Error::Corrupt)?.props;
            state = Some(State::new(props));
        }
        let s = state.as_mut().ok_or(Error::Corrupt)?;
        let chunk = input.get(at..at + packed).ok_or(Error::Corrupt)?;
        let end = out.len() + unpacked;
        if end > size {
            return Err(Error::Corrupt);
        }
        let mut rc = Range::new(chunk)?;
        s.run(&mut rc, &mut out, end, floor, false)?;
        if out.len() != end {
            return Err(Error::Corrupt);
        }
        at += packed;
    }
    if out.len() != size {
        return Err(Error::Corrupt);
    }
    Ok(out)
}

/// Undo the x86 branch converter (7z coder 03030103, "BCJ"), in place.
pub fn bcj_x86_decode(buf: &mut [u8]) {
    const ALLOWED: [bool; 8] = [true, true, true, false, true, false, false, false];
    const BIT: [usize; 8] = [0, 1, 2, 2, 3, 3, 3, 3];
    let test = |b: u8| b == 0 || b == 0xFF;
    if buf.len() <= 4 {
        return;
    }
    let size = buf.len() - 4;
    let mut prev_pos: usize = usize::MAX;
    let mut prev_mask: u32 = 0;
    let mut i = 0;
    while i < size {
        if buf[i] & 0xFE != 0xE8 {
            i += 1;
            continue;
        }
        let gap = i.wrapping_sub(prev_pos);
        if gap > 3 {
            prev_mask = 0;
        } else {
            prev_mask = (prev_mask << (gap - 1)) & 7;
            if prev_mask != 0 {
                let b = buf[i + 4 - BIT[prev_mask as usize]];
                if !ALLOWED[prev_mask as usize] || test(b) {
                    prev_pos = i;
                    prev_mask = (prev_mask << 1) | 1;
                    i += 1;
                    continue;
                }
            }
        }
        prev_pos = i;
        if test(buf[i + 4]) {
            let mut src = u32::from_le_bytes([buf[i + 1], buf[i + 2], buf[i + 3], buf[i + 4]]);
            let mut dest;
            loop {
                dest = src.wrapping_sub(i as u32 + 5);
                if prev_mask == 0 {
                    break;
                }
                let j = BIT[prev_mask as usize] * 8;
                let b = (dest >> (24 - j)) as u8;
                if !test(b) {
                    break;
                }
                src = dest ^ ((1u32 << (32 - j)) - 1);
            }
            dest &= 0x01FF_FFFF;
            dest |= 0u32.wrapping_sub(dest & 0x0100_0000);
            buf[i + 1..i + 5].copy_from_slice(&dest.to_le_bytes());
            i += 5;
        } else {
            prev_mask = (prev_mask << 1) | 1;
            i += 1;
        }
    }
}

// ---- Encoder ----

struct RangeEncoder {
    out: Vec<u8>,
    low: u64,
    range: u32,
    cache: u8,
    cache_size: u64,
}
impl RangeEncoder {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            low: 0,
            range: 0xFFFF_FFFF,
            cache: 0,
            cache_size: 1,
        }
    }
    fn shift_low(&mut self) {
        if (self.low as u32) < 0xFF00_0000 || (self.low >> 32) != 0 {
            let carry = (self.low >> 32) as u8;
            let mut temp = self.cache;
            loop {
                self.out.push(temp.wrapping_add(carry));
                temp = 0xFF;
                self.cache_size -= 1;
                if self.cache_size == 0 {
                    break;
                }
            }
            self.cache = (self.low >> 24) as u8;
        }
        self.cache_size += 1;
        self.low = u64::from((self.low as u32) << 8);
    }
    #[inline]
    fn bit(&mut self, prob: &mut u16, bit: u32) {
        let bound = (self.range >> 11) * u32::from(*prob);
        if bit == 0 {
            self.range = bound;
            *prob += (2048 - *prob) >> 5;
        } else {
            self.low += u64::from(bound);
            self.range -= bound;
            *prob -= *prob >> 5;
        }
        while self.range < 1 << 24 {
            self.range <<= 8;
            self.shift_low();
        }
    }
    fn direct(&mut self, value: u32, bits: u32) {
        for i in (0..bits).rev() {
            self.range >>= 1;
            if (value >> i) & 1 != 0 {
                self.low += u64::from(self.range);
            }
            while self.range < 1 << 24 {
                self.range <<= 8;
                self.shift_low();
            }
        }
    }
    fn tree(&mut self, probs: &mut [u16], bits: u32, value: u32) {
        let mut m = 1usize;
        for i in (0..bits).rev() {
            let b = (value >> i) & 1;
            self.bit(&mut probs[m], b);
            m = (m << 1) | b as usize;
        }
    }
    fn reverse(&mut self, probs: &mut [u16], bits: u32, value: u32) {
        let mut m = 1usize;
        for i in 0..bits {
            let b = (value >> i) & 1;
            self.bit(&mut probs[m], b);
            m = (m << 1) | b as usize;
        }
    }
    fn finish(mut self) -> Vec<u8> {
        for _ in 0..5 {
            self.shift_low();
        }
        self.out
    }
}
impl Lengths {
    fn encode(&mut self, rc: &mut RangeEncoder, pos_state: usize, len: u32) {
        if len < 8 {
            rc.bit(&mut self.choice, 0);
            rc.tree(&mut self.low[pos_state], 3, len);
        } else if len < 16 {
            rc.bit(&mut self.choice, 1);
            rc.bit(&mut self.choice2, 0);
            rc.tree(&mut self.mid[pos_state], 3, len - 8);
        } else {
            rc.bit(&mut self.choice, 1);
            rc.bit(&mut self.choice2, 1);
            rc.tree(&mut self.high, 8, len - 16);
        }
    }
}
fn slot_of(dist: u32) -> u32 {
    if dist < 4 {
        return dist;
    }
    let n = 31 - dist.leading_zeros();
    (n << 1) | ((dist >> (n - 1)) & 1)
}
impl State {
    fn encode_distance(&mut self, rc: &mut RangeEncoder, len: u32, dist: u32) {
        let len_state = len.min(3) as usize;
        let slot = slot_of(dist);
        rc.tree(&mut self.pos_slot[len_state], 6, slot);
        if slot < 4 {
            return;
        }
        let direct = (slot >> 1) - 1;
        let base = (2 | (slot & 1)) << direct;
        let rem = dist - base;
        if slot < 14 {
            rc.reverse(&mut self.pos_special[(base - slot) as usize..], direct, rem);
        } else {
            rc.direct(rem >> 4, direct - 4);
            rc.reverse(&mut self.align, 4, rem & 15);
        }
    }
    fn literal(&mut self, rc: &mut RangeEncoder, data: &[u8], pos: usize) {
        let pos_state = pos & ((1 << self.props.pb) - 1);
        rc.bit(&mut self.is_match[(self.state << 4) + pos_state], 0);
        let prev = if pos > 0 { data[pos - 1] } else { 0 };
        let lp_mask = (1usize << self.props.lp) - 1;
        let lit_state =
            ((pos & lp_mask) << self.props.lc) + (usize::from(prev) >> (8 - self.props.lc));
        let probs = &mut self.literal[0x300 * lit_state..0x300 * lit_state + 0x300];
        let b = u32::from(data[pos]);
        let mut symbol = 1usize;
        if self.state >= 7 {
            let mb = u32::from(data[pos - self.reps[0] as usize - 1]);
            let mut matched = true;
            for i in (0..8).rev() {
                let bit = (b >> i) & 1;
                if matched {
                    let mbit = (mb >> i) & 1;
                    rc.bit(&mut probs[((1 + mbit as usize) << 8) + symbol], bit);
                    matched = mbit == bit;
                } else {
                    rc.bit(&mut probs[symbol], bit);
                }
                symbol = (symbol << 1) | bit as usize;
            }
        } else {
            for i in (0..8).rev() {
                let bit = (b >> i) & 1;
                rc.bit(&mut probs[symbol], bit);
                symbol = (symbol << 1) | bit as usize;
            }
        }
        let s = self.state;
        self.state = if s < 4 {
            0
        } else if s < 10 {
            s - 3
        } else {
            s - 6
        };
    }
    fn short_rep(&mut self, rc: &mut RangeEncoder, pos_state: usize) {
        let s = self.state;
        rc.bit(&mut self.is_match[(s << 4) + pos_state], 1);
        rc.bit(&mut self.is_rep[s], 1);
        rc.bit(&mut self.is_rep_g0[s], 0);
        rc.bit(&mut self.is_rep0_long[(s << 4) + pos_state], 0);
        self.state = if s < 7 { 9 } else { 11 };
    }
    fn rep0(&mut self, rc: &mut RangeEncoder, pos_state: usize, len: u32) {
        let s = self.state;
        rc.bit(&mut self.is_match[(s << 4) + pos_state], 1);
        rc.bit(&mut self.is_rep[s], 1);
        rc.bit(&mut self.is_rep_g0[s], 0);
        rc.bit(&mut self.is_rep0_long[(s << 4) + pos_state], 1);
        self.rep_len.encode(rc, pos_state, len - 2);
        self.state = if s < 7 { 8 } else { 11 };
    }
    fn matched(&mut self, rc: &mut RangeEncoder, pos_state: usize, len: u32, dist: u32) {
        let s = self.state;
        rc.bit(&mut self.is_match[(s << 4) + pos_state], 1);
        rc.bit(&mut self.is_rep[s], 0);
        self.len.encode(rc, pos_state, len - 2);
        self.encode_distance(rc, len - 2, dist);
        self.reps = [dist, self.reps[0], self.reps[1], self.reps[2]];
        self.state = if s < 7 { 7 } else { 10 };
    }
}
const MATCH_MAX: usize = 273;
const HASH_BITS: u32 = 16;
/// The 7z coder properties this encoder writes (lc 3, lp 0, pb 2 and the
/// dictionary size) and the raw LZMA stream for `data` (no end marker:
/// 7z records the size). Greedy parsing with one-step lazy matching over
/// a hash chain (32 candidates).
pub fn encode_lzma(data: &[u8]) -> ([u8; 5], Vec<u8>) {
    let props = Props {
        lc: 3,
        lp: 0,
        pb: 2,
    };
    let dict = (data.len() as u32)
        .next_power_of_two()
        .clamp(1 << 16, 1 << 26);
    let mut header = [0u8; 5];
    header[0] = 0x5D;
    header[1..].copy_from_slice(&dict.to_le_bytes());
    let mut s = State::new(props);
    let mut rc = RangeEncoder::new();
    let mut head = alloc::vec![u32::MAX; 1 << HASH_BITS];
    let mut chain = alloc::vec![u32::MAX; data.len()];
    let hash = |p: usize| -> usize {
        let v = u32::from(data[p]) | (u32::from(data[p + 1]) << 8) | (u32::from(data[p + 2]) << 16);
        (v.wrapping_mul(2_654_435_761) >> (32 - HASH_BITS)) as usize
    };
    let insert = |head: &mut Vec<u32>, chain: &mut Vec<u32>, p: usize| {
        if p + 3 <= data.len() {
            let h = hash(p);
            chain[p] = head[h];
            head[h] = p as u32;
        }
    };
    let longest = |head: &Vec<u32>, chain: &Vec<u32>, p: usize| -> (usize, u32) {
        if p + 3 > data.len() {
            return (0, 0);
        }
        let limit = (data.len() - p).min(MATCH_MAX);
        let mut best = (0usize, 0u32);
        let mut cand = head[hash(p)];
        for _ in 0..32 {
            if cand == u32::MAX {
                break;
            }
            let c = cand as usize;
            let dist = p - c - 1;
            if dist as u64 >= u64::from(dict) {
                break;
            }
            let mut n = 0;
            while n < limit && data[c + n] == data[p + n] {
                n += 1;
            }
            if n > best.0 {
                best = (n, dist as u32);
                if n == limit {
                    break;
                }
            }
            cand = chain[c];
        }
        best
    };
    let rep_len = |p: usize, rep: u32| -> usize {
        let back = rep as usize + 1;
        if back > p {
            return 0;
        }
        let limit = (data.len() - p).min(MATCH_MAX);
        let mut n = 0;
        while n < limit && data[p - back + n] == data[p + n] {
            n += 1;
        }
        n
    };
    let mut p = 0;
    while p < data.len() {
        let pos_state = p & 3;
        let rep = rep_len(p, s.reps[0]);
        let (mut len, dist) = longest(&head, &chain, p);
        // A distance-only match must beat the cheap rep0 by two bytes.
        if rep >= 2 && rep + 1 >= len {
            s.rep0(&mut rc, pos_state, rep as u32);
            for q in p..p + rep {
                insert(&mut head, &mut chain, q);
            }
            p += rep;
            continue;
        }
        if len >= 3 || (len == 2 && dist < 128) {
            // Lazy: a longer match one byte later wins.
            insert(&mut head, &mut chain, p);
            let (next, _) = longest(&head, &chain, p + 1);
            if next > len + 1 && p + 1 < data.len() {
                if rep_len(p, s.reps[0]) == 0 || data[p] != data[p - s.reps[0] as usize - 1] {
                    s.literal(&mut rc, data, p);
                } else {
                    s.short_rep(&mut rc, pos_state);
                }
                p += 1;
                continue;
            }
            len = len.min(MATCH_MAX);
            s.matched(&mut rc, pos_state, len as u32, dist);
            for q in p + 1..p + len {
                insert(&mut head, &mut chain, q);
            }
            p += len;
            continue;
        }
        if rep >= 1 {
            s.short_rep(&mut rc, pos_state);
        } else {
            s.literal(&mut rc, data, p);
        }
        insert(&mut head, &mut chain, p);
        p += 1;
    }
    (header, rc.finish())
}

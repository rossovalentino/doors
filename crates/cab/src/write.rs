//! Building single cabinets: one folder, stored or MSZIP (deflate with
//! fixed Huffman codes and greedy LZ77 matches inside each 32 KiB block),
//! every CFDATA block checksummed as makecab does.
use crate::checksum;
use alloc::{string::String, vec, vec::Vec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    Stored,
    MsZip,
}

const BLOCK: usize = 32 * 1024;

pub struct Writer {
    compression: Compression,
    /// (name, data, MS-DOS date, time, attributes).
    files: Vec<(String, Vec<u8>, u16, u16, u16)>,
    set_id: u16,
}
impl Writer {
    pub fn new(compression: Compression) -> Self {
        Self {
            compression,
            files: Vec::new(),
            set_id: 0,
        }
    }
    pub fn set_id(&mut self, id: u16) {
        self.set_id = id;
    }
    /// A file; names that are not ASCII get the UTF-8 attribute.
    pub fn add(&mut self, name: &str, data: &[u8], date: u16, time: u16, attributes: u16) {
        let attributes = if name.is_ascii() {
            attributes
        } else {
            attributes | crate::ATTRIBUTE_UTF8
        };
        self.files
            .push((String::from(name), data.to_vec(), date, time, attributes));
    }
    pub fn finish(&self) -> Vec<u8> {
        let stream: Vec<u8> = self
            .files
            .iter()
            .flat_map(|f| f.1.iter().copied())
            .collect();
        let mut blocks = Vec::new();
        for chunk in stream.chunks(BLOCK) {
            let data = match self.compression {
                Compression::Stored => chunk.to_vec(),
                Compression::MsZip => {
                    let mut d = vec![b'C', b'K'];
                    d.extend_from_slice(&deflate(chunk));
                    d
                }
            };
            blocks.push((data, chunk.len()));
        }
        let header = 36usize;
        let folder = 8usize;
        let files_at = header + folder;
        let files_bytes: usize = self.files.iter().map(|f| 16 + f.0.len() + 1).sum();
        let data_at = files_at + files_bytes;
        let data_bytes: usize = blocks.iter().map(|b| 8 + b.0.len()).sum();
        let total = data_at + data_bytes;

        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"MSCF");
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(files_at as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&[3, 1]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&(self.files.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&self.set_id.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        // CFFOLDER
        out.extend_from_slice(&(data_at as u32).to_le_bytes());
        out.extend_from_slice(&(blocks.len() as u16).to_le_bytes());
        let kind: u16 = match self.compression {
            Compression::Stored => 0,
            Compression::MsZip => 1,
        };
        out.extend_from_slice(&kind.to_le_bytes());
        // CFFILE
        let mut offset = 0u32;
        for (name, data, date, time, attributes) in &self.files {
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&offset.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&date.to_le_bytes());
            out.extend_from_slice(&time.to_le_bytes());
            out.extend_from_slice(&attributes.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.push(0);
            offset += data.len() as u32;
        }
        // CFDATA
        for (data, size) in &blocks {
            let mut header = [0u8; 4];
            header[..2].copy_from_slice(&(data.len() as u16).to_le_bytes());
            header[2..].copy_from_slice(&(*size as u16).to_le_bytes());
            let csum = checksum(&header, checksum(data, 0));
            out.extend_from_slice(&csum.to_le_bytes());
            out.extend_from_slice(&header);
            out.extend_from_slice(data);
        }
        out
    }
}

struct BitWriter {
    out: Vec<u8>,
    held: u32,
    bits: u32,
}
impl BitWriter {
    fn put(&mut self, value: u32, n: u32) {
        self.held |= value << self.bits;
        self.bits += n;
        while self.bits >= 8 {
            self.out.push(self.held as u8);
            self.held >>= 8;
            self.bits -= 8;
        }
    }
    /// Huffman codes go most significant bit first.
    fn code(&mut self, code: u32, n: u32) {
        let mut reversed = 0;
        for i in 0..n {
            reversed |= ((code >> i) & 1) << (n - 1 - i);
        }
        self.put(reversed, n);
    }
    fn finish(mut self) -> Vec<u8> {
        if self.bits > 0 {
            self.out.push(self.held as u8);
        }
        self.out
    }
}

fn literal(w: &mut BitWriter, symbol: u32) {
    match symbol {
        0..=143 => w.code(0x30 + symbol, 8),
        144..=255 => w.code(0x190 + symbol - 144, 9),
        256..=279 => w.code(symbol - 256, 7),
        _ => w.code(0xc0 + symbol - 280, 8),
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

/// One final fixed-Huffman deflate block of `data` (matches stay inside
/// `data`, so every block decodes on its own as well as in sequence).
fn deflate(data: &[u8]) -> Vec<u8> {
    const HASH: usize = 1 << 13;
    const CHAIN: usize = 32;
    let mut head = vec![usize::MAX; HASH];
    let mut previous = vec![usize::MAX; data.len()];
    let hash = |i: usize| -> usize {
        let v = u32::from(data[i]) << 16 | u32::from(data[i + 1]) << 8 | u32::from(data[i + 2]);
        (v.wrapping_mul(2654435761) >> 19) as usize & (HASH - 1)
    };
    let mut w = BitWriter {
        out: Vec::new(),
        held: 0,
        bits: 0,
    };
    w.put(1, 1); // final
    w.put(1, 2); // fixed codes
    let mut i = 0;
    let insert = |i: usize, head: &mut Vec<usize>, previous: &mut Vec<usize>| {
        if i + 2 < data.len() {
            let h = hash(i);
            previous[i] = head[h];
            head[h] = i;
        }
    };
    while i < data.len() {
        let (mut best, mut distance) = (0usize, 0usize);
        if i + 2 < data.len() {
            let mut candidate = head[hash(i)];
            let mut tries = 0;
            while candidate != usize::MAX && tries < CHAIN {
                let d = i - candidate;
                if d > 32768 {
                    break;
                }
                let max = (data.len() - i).min(258);
                let mut l = 0;
                while l < max && data[candidate + l] == data[i + l] {
                    l += 1;
                }
                if l > best {
                    best = l;
                    distance = d;
                    if l == max {
                        break;
                    }
                }
                candidate = previous[candidate];
                tries += 1;
            }
        }
        if best >= 3 {
            let li = LENGTH_BASE
                .iter()
                .rposition(|b| usize::from(*b) <= best)
                .unwrap();
            literal(&mut w, 257 + li as u32);
            w.put(
                (best - usize::from(LENGTH_BASE[li])) as u32,
                u32::from(LENGTH_EXTRA[li]),
            );
            let di = DISTANCE_BASE
                .iter()
                .rposition(|b| usize::from(*b) <= distance)
                .unwrap();
            w.code(di as u32, 5);
            w.put(
                (distance - usize::from(DISTANCE_BASE[di])) as u32,
                u32::from(DISTANCE_EXTRA[di]),
            );
            for k in i..i + best {
                insert(k, &mut head, &mut previous);
            }
            i += best;
        } else {
            literal(&mut w, u32::from(data[i]));
            insert(i, &mut head, &mut previous);
            i += 1;
        }
    }
    literal(&mut w, 256);
    w.finish()
}

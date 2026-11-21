//! The block and stream ciphers CryptoAPI's providers offer besides AES:
//! DES (FIPS 46-3) and triple DES (two- and three-key EDE), RC2 (RFC 2268)
//! and RC4. Software implementations checked against the standards' test
//! vectors and against Windows' providers (`tests/reference`).

/// DES's tables (FIPS 46-3), 1-based bit positions.
const IP: [u8; 64] = [
    58, 50, 42, 34, 26, 18, 10, 2, 60, 52, 44, 36, 28, 20, 12, 4, 62, 54, 46, 38, 30, 22, 14, 6,
    64, 56, 48, 40, 32, 24, 16, 8, 57, 49, 41, 33, 25, 17, 9, 1, 59, 51, 43, 35, 27, 19, 11, 3, 61,
    53, 45, 37, 29, 21, 13, 5, 63, 55, 47, 39, 31, 23, 15, 7,
];
const FP: [u8; 64] = [
    40, 8, 48, 16, 56, 24, 64, 32, 39, 7, 47, 15, 55, 23, 63, 31, 38, 6, 46, 14, 54, 22, 62, 30,
    37, 5, 45, 13, 53, 21, 61, 29, 36, 4, 44, 12, 52, 20, 60, 28, 35, 3, 43, 11, 51, 19, 59, 27,
    34, 2, 42, 10, 50, 18, 58, 26, 33, 1, 41, 9, 49, 17, 57, 25,
];
const E: [u8; 48] = [
    32, 1, 2, 3, 4, 5, 4, 5, 6, 7, 8, 9, 8, 9, 10, 11, 12, 13, 12, 13, 14, 15, 16, 17, 16, 17, 18,
    19, 20, 21, 20, 21, 22, 23, 24, 25, 24, 25, 26, 27, 28, 29, 28, 29, 30, 31, 32, 1,
];
const P: [u8; 32] = [
    16, 7, 20, 21, 29, 12, 28, 17, 1, 15, 23, 26, 5, 18, 31, 10, 2, 8, 24, 14, 32, 27, 3, 9, 19,
    13, 30, 6, 22, 11, 4, 25,
];
const PC1: [u8; 56] = [
    57, 49, 41, 33, 25, 17, 9, 1, 58, 50, 42, 34, 26, 18, 10, 2, 59, 51, 43, 35, 27, 19, 11, 3, 60,
    52, 44, 36, 63, 55, 47, 39, 31, 23, 15, 7, 62, 54, 46, 38, 30, 22, 14, 6, 61, 53, 45, 37, 29,
    21, 13, 5, 28, 20, 12, 4,
];
const PC2: [u8; 48] = [
    14, 17, 11, 24, 1, 5, 3, 28, 15, 6, 21, 10, 23, 19, 12, 4, 26, 8, 16, 7, 27, 20, 13, 2, 41, 52,
    31, 37, 47, 55, 30, 40, 51, 45, 33, 48, 44, 49, 39, 56, 34, 53, 46, 42, 50, 36, 29, 32,
];
const SHIFTS: [u8; 16] = [1, 1, 2, 2, 2, 2, 2, 2, 1, 2, 2, 2, 2, 2, 2, 1];
const S: [[u8; 64]; 8] = [
    [
        14, 4, 13, 1, 2, 15, 11, 8, 3, 10, 6, 12, 5, 9, 0, 7, 0, 15, 7, 4, 14, 2, 13, 1, 10, 6, 12,
        11, 9, 5, 3, 8, 4, 1, 14, 8, 13, 6, 2, 11, 15, 12, 9, 7, 3, 10, 5, 0, 15, 12, 8, 2, 4, 9,
        1, 7, 5, 11, 3, 14, 10, 0, 6, 13,
    ],
    [
        15, 1, 8, 14, 6, 11, 3, 4, 9, 7, 2, 13, 12, 0, 5, 10, 3, 13, 4, 7, 15, 2, 8, 14, 12, 0, 1,
        10, 6, 9, 11, 5, 0, 14, 7, 11, 10, 4, 13, 1, 5, 8, 12, 6, 9, 3, 2, 15, 13, 8, 10, 1, 3, 15,
        4, 2, 11, 6, 7, 12, 0, 5, 14, 9,
    ],
    [
        10, 0, 9, 14, 6, 3, 15, 5, 1, 13, 12, 7, 11, 4, 2, 8, 13, 7, 0, 9, 3, 4, 6, 10, 2, 8, 5,
        14, 12, 11, 15, 1, 13, 6, 4, 9, 8, 15, 3, 0, 11, 1, 2, 12, 5, 10, 14, 7, 1, 10, 13, 0, 6,
        9, 8, 7, 4, 15, 14, 3, 11, 5, 2, 12,
    ],
    [
        7, 13, 14, 3, 0, 6, 9, 10, 1, 2, 8, 5, 11, 12, 4, 15, 13, 8, 11, 5, 6, 15, 0, 3, 4, 7, 2,
        12, 1, 10, 14, 9, 10, 6, 9, 0, 12, 11, 7, 13, 15, 1, 3, 14, 5, 2, 8, 4, 3, 15, 0, 6, 10, 1,
        13, 8, 9, 4, 5, 11, 12, 7, 2, 14,
    ],
    [
        2, 12, 4, 1, 7, 10, 11, 6, 8, 5, 3, 15, 13, 0, 14, 9, 14, 11, 2, 12, 4, 7, 13, 1, 5, 0, 15,
        10, 3, 9, 8, 6, 4, 2, 1, 11, 10, 13, 7, 8, 15, 9, 12, 5, 6, 3, 0, 14, 11, 8, 12, 7, 1, 14,
        2, 13, 6, 15, 0, 9, 10, 4, 5, 3,
    ],
    [
        12, 1, 10, 15, 9, 2, 6, 8, 0, 13, 3, 4, 14, 7, 5, 11, 10, 15, 4, 2, 7, 12, 9, 5, 6, 1, 13,
        14, 0, 11, 3, 8, 9, 14, 15, 5, 2, 8, 12, 3, 7, 0, 4, 10, 1, 13, 11, 6, 4, 3, 2, 12, 9, 5,
        15, 10, 11, 14, 1, 7, 6, 0, 8, 13,
    ],
    [
        4, 11, 2, 14, 15, 0, 8, 13, 3, 12, 9, 7, 5, 10, 6, 1, 13, 0, 11, 7, 4, 9, 1, 10, 14, 3, 5,
        12, 2, 15, 8, 6, 1, 4, 11, 13, 12, 3, 7, 14, 10, 15, 6, 8, 0, 5, 9, 2, 6, 11, 13, 8, 1, 4,
        10, 7, 9, 5, 0, 15, 14, 2, 3, 12,
    ],
    [
        13, 2, 8, 4, 6, 15, 11, 1, 10, 9, 3, 14, 5, 0, 12, 7, 1, 15, 13, 8, 10, 3, 7, 4, 12, 5, 6,
        11, 0, 14, 9, 2, 7, 11, 4, 1, 9, 12, 14, 2, 0, 6, 10, 13, 15, 3, 5, 8, 2, 1, 14, 7, 4, 10,
        8, 13, 15, 12, 9, 0, 3, 5, 6, 11,
    ],
];
/// `input`'s bits (the top `width` bits of a u64, bit 1 first) taken in
/// `table`'s order.
fn permute(input: u64, width: u32, table: &[u8]) -> u64 {
    let mut out = 0u64;
    for &position in table {
        out = (out << 1) | ((input >> (width - u32::from(position))) & 1);
    }
    out
}
/// One DES key's 16 round keys (48 bits each).
#[derive(Clone, Copy)]
pub struct Des {
    rounds: [u64; 16],
}
impl Des {
    pub fn new(key: &[u8; 8]) -> Self {
        let key = u64::from_be_bytes(*key);
        let cd = permute(key, 64, &PC1);
        let (mut c, mut d) = ((cd >> 28) & 0x0fff_ffff, cd & 0x0fff_ffff);
        let mut rounds = [0u64; 16];
        for (round, &shift) in rounds.iter_mut().zip(SHIFTS.iter()) {
            let s = u32::from(shift);
            c = ((c << s) | (c >> (28 - s))) & 0x0fff_ffff;
            d = ((d << s) | (d >> (28 - s))) & 0x0fff_ffff;
            *round = permute((c << 28) | d, 56, &PC2);
        }
        Self { rounds }
    }
    fn feistel(r: u32, k: u64) -> u32 {
        let x = permute(u64::from(r), 32, &E) ^ k;
        let mut out = 0u32;
        for (i, sbox) in S.iter().enumerate() {
            let six = ((x >> (42 - 6 * i)) & 0x3f) as usize;
            let row = ((six & 0x20) >> 4) | (six & 1);
            let column = (six >> 1) & 0xf;
            out = (out << 4) | u32::from(sbox[row * 16 + column]);
        }
        permute(u64::from(out), 32, &P) as u32
    }
    fn crypt(&self, block: [u8; 8], decrypt: bool) -> [u8; 8] {
        let x = permute(u64::from_be_bytes(block), 64, &IP);
        let (mut l, mut r) = ((x >> 32) as u32, x as u32);
        for i in 0..16 {
            let k = self.rounds[if decrypt { 15 - i } else { i }];
            let next = l ^ Self::feistel(r, k);
            l = r;
            r = next;
        }
        permute((u64::from(r) << 32) | u64::from(l), 64, &FP).to_be_bytes()
    }
    pub fn encrypt(&self, block: [u8; 8]) -> [u8; 8] {
        self.crypt(block, false)
    }
    pub fn decrypt(&self, block: [u8; 8]) -> [u8; 8] {
        self.crypt(block, true)
    }
}
/// Triple DES, encrypt-decrypt-encrypt with three keys (two-key: the
/// first key again as the third).
#[derive(Clone, Copy)]
pub struct TripleDes {
    keys: [Des; 3],
}
impl TripleDes {
    /// 16 bytes (two-key) or 24 (three-key).
    pub fn new(key: &[u8]) -> Option<Self> {
        let part = |i: usize| -> Option<Des> {
            Some(Des::new(key.get(8 * i..8 * i + 8)?.try_into().ok()?))
        };
        let (a, b) = (part(0)?, part(1)?);
        let c = match key.len() {
            16 => a,
            24 => part(2)?,
            _ => return None,
        };
        Some(Self { keys: [a, b, c] })
    }
    pub fn encrypt(&self, block: [u8; 8]) -> [u8; 8] {
        self.keys[2].encrypt(self.keys[1].decrypt(self.keys[0].encrypt(block)))
    }
    pub fn decrypt(&self, block: [u8; 8]) -> [u8; 8] {
        self.keys[0].decrypt(self.keys[1].encrypt(self.keys[2].decrypt(block)))
    }
}

/// RC2's PITABLE (RFC 2268 section 2): a permutation of 0..=255 from the
/// digits of pi.
const PITABLE: [u8; 256] = [
    0xd9, 0x78, 0xf9, 0xc4, 0x19, 0xdd, 0xb5, 0xed, 0x28, 0xe9, 0xfd, 0x79, 0x4a, 0xa0, 0xd8, 0x9d,
    0xc6, 0x7e, 0x37, 0x83, 0x2b, 0x76, 0x53, 0x8e, 0x62, 0x4c, 0x64, 0x88, 0x44, 0x8b, 0xfb, 0xa2,
    0x17, 0x9a, 0x59, 0xf5, 0x87, 0xb3, 0x4f, 0x13, 0x61, 0x45, 0x6d, 0x8d, 0x09, 0x81, 0x7d, 0x32,
    0xbd, 0x8f, 0x40, 0xeb, 0x86, 0xb7, 0x7b, 0x0b, 0xf0, 0x95, 0x21, 0x22, 0x5c, 0x6b, 0x4e, 0x82,
    0x54, 0xd6, 0x65, 0x93, 0xce, 0x60, 0xb2, 0x1c, 0x73, 0x56, 0xc0, 0x14, 0xa7, 0x8c, 0xf1, 0xdc,
    0x12, 0x75, 0xca, 0x1f, 0x3b, 0xbe, 0xe4, 0xd1, 0x42, 0x3d, 0xd4, 0x30, 0xa3, 0x3c, 0xb6, 0x26,
    0x6f, 0xbf, 0x0e, 0xda, 0x46, 0x69, 0x07, 0x57, 0x27, 0xf2, 0x1d, 0x9b, 0xbc, 0x94, 0x43, 0x03,
    0xf8, 0x11, 0xc7, 0xf6, 0x90, 0xef, 0x3e, 0xe7, 0x06, 0xc3, 0xd5, 0x2f, 0xc8, 0x66, 0x1e, 0xd7,
    0x08, 0xe8, 0xea, 0xde, 0x80, 0x52, 0xee, 0xf7, 0x84, 0xaa, 0x72, 0xac, 0x35, 0x4d, 0x6a, 0x2a,
    0x96, 0x1a, 0xd2, 0x71, 0x5a, 0x15, 0x49, 0x74, 0x4b, 0x9f, 0xd0, 0x5e, 0x04, 0x18, 0xa4, 0xec,
    0xc2, 0xe0, 0x41, 0x6e, 0x0f, 0x51, 0xcb, 0xcc, 0x24, 0x91, 0xaf, 0x50, 0xa1, 0xf4, 0x70, 0x39,
    0x99, 0x7c, 0x3a, 0x85, 0x23, 0xb8, 0xb4, 0x7a, 0xfc, 0x02, 0x36, 0x5b, 0x25, 0x55, 0x97, 0x31,
    0x2d, 0x5d, 0xfa, 0x98, 0xe3, 0x8a, 0x92, 0xae, 0x05, 0xdf, 0x29, 0x10, 0x67, 0x6c, 0xba, 0xc9,
    0xd3, 0x00, 0xe6, 0xcf, 0xe1, 0x9e, 0xa8, 0x2c, 0x63, 0x16, 0x01, 0x3f, 0x58, 0xe2, 0x89, 0xa9,
    0x0d, 0x38, 0x34, 0x1b, 0xab, 0x33, 0xff, 0xb0, 0xbb, 0x48, 0x0c, 0x5f, 0xb9, 0xb1, 0xcd, 0x2e,
    0xc5, 0xf3, 0xdb, 0x47, 0xe5, 0xa5, 0x9c, 0x77, 0x0a, 0xa6, 0x20, 0x68, 0xfe, 0x7f, 0xc1, 0xad,
];
/// RC2 with a key of 1 to 128 bytes and an effective key length in bits.
#[derive(Clone, Copy)]
pub struct Rc2 {
    k: [u16; 64],
}
impl Rc2 {
    pub fn new(key: &[u8], effective_bits: usize) -> Option<Self> {
        let t = key.len();
        if t == 0 || t > 128 || effective_bits == 0 || effective_bits > 1024 {
            return None;
        }
        let mut l = [0u8; 128];
        l[..t].copy_from_slice(key);
        for i in t..128 {
            l[i] = PITABLE[usize::from(l[i - 1].wrapping_add(l[i - t]))];
        }
        let t8 = effective_bits.div_ceil(8);
        let tm = (0xffu32 >> (8 * t8 - effective_bits)) as u8;
        l[128 - t8] = PITABLE[usize::from(l[128 - t8] & tm)];
        for i in (0..128 - t8).rev() {
            l[i] = PITABLE[usize::from(l[i + 1] ^ l[i + t8])];
        }
        let mut k = [0u16; 64];
        for (i, word) in k.iter_mut().enumerate() {
            *word = u16::from_le_bytes([l[2 * i], l[2 * i + 1]]);
        }
        Some(Self { k })
    }
    pub fn encrypt(&self, block: [u8; 8]) -> [u8; 8] {
        let mut r = [0u16; 4];
        for (i, w) in r.iter_mut().enumerate() {
            *w = u16::from_le_bytes([block[2 * i], block[2 * i + 1]]);
        }
        let mut j = 0;
        let mix = |r: &mut [u16; 4], j: &mut usize| {
            for (i, s) in [1u32, 2, 3, 5].into_iter().enumerate() {
                r[i] = r[i]
                    .wrapping_add(self.k[*j])
                    .wrapping_add(r[(i + 3) % 4] & r[(i + 2) % 4])
                    .wrapping_add(!r[(i + 3) % 4] & r[(i + 1) % 4]);
                *j += 1;
                r[i] = r[i].rotate_left(s);
            }
        };
        let mash = |r: &mut [u16; 4]| {
            for i in 0..4 {
                r[i] = r[i].wrapping_add(self.k[usize::from(r[(i + 3) % 4] & 63)]);
            }
        };
        for round in 0..16 {
            mix(&mut r, &mut j);
            if round == 4 || round == 10 {
                mash(&mut r);
            }
        }
        let mut out = [0u8; 8];
        for (i, w) in r.iter().enumerate() {
            out[2 * i..2 * i + 2].copy_from_slice(&w.to_le_bytes());
        }
        out
    }
    pub fn decrypt(&self, block: [u8; 8]) -> [u8; 8] {
        let mut r = [0u16; 4];
        for (i, w) in r.iter_mut().enumerate() {
            *w = u16::from_le_bytes([block[2 * i], block[2 * i + 1]]);
        }
        let mut j = 63usize;
        let rmix = |r: &mut [u16; 4], j: &mut usize| {
            for (i, s) in [(3usize, 5u32), (2, 3), (1, 2), (0, 1)] {
                r[i] = r[i].rotate_right(s);
                r[i] = r[i]
                    .wrapping_sub(self.k[*j])
                    .wrapping_sub(r[(i + 3) % 4] & r[(i + 2) % 4])
                    .wrapping_sub(!r[(i + 3) % 4] & r[(i + 1) % 4]);
                *j = j.wrapping_sub(1);
            }
        };
        let rmash = |r: &mut [u16; 4]| {
            for i in (0..4).rev() {
                r[i] = r[i].wrapping_sub(self.k[usize::from(r[(i + 3) % 4] & 63)]);
            }
        };
        for round in (0..16).rev() {
            rmix(&mut r, &mut j);
            if round == 5 || round == 11 {
                rmash(&mut r);
            }
        }
        let mut out = [0u8; 8];
        for (i, w) in r.iter().enumerate() {
            out[2 * i..2 * i + 2].copy_from_slice(&w.to_le_bytes());
        }
        out
    }
}

/// RC4: its state after the key schedule, then advancing with each byte.
#[derive(Clone)]
pub struct Rc4 {
    s: [u8; 256],
    i: u8,
    j: u8,
}
impl Rc4 {
    pub fn new(key: &[u8]) -> Self {
        let mut s = [0u8; 256];
        for (i, v) in s.iter_mut().enumerate() {
            *v = i as u8;
        }
        let mut j = 0u8;
        for i in 0..256 {
            j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
            s.swap(i, usize::from(j));
        }
        Self { s, i: 0, j: 0 }
    }
    pub fn apply(&mut self, data: &mut [u8]) {
        for b in data {
            self.i = self.i.wrapping_add(1);
            self.j = self.j.wrapping_add(self.s[usize::from(self.i)]);
            self.s.swap(usize::from(self.i), usize::from(self.j));
            let k = self.s[usize::from(
                self.s[usize::from(self.i)].wrapping_add(self.s[usize::from(self.j)]),
            )];
            *b ^= k;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;
    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }
    fn block(s: &str) -> [u8; 8] {
        hex(s).try_into().unwrap()
    }
    #[test]
    fn des_matches_the_standard_vectors() {
        // The classic worked example and FIPS 81's.
        let des = Des::new(&block("133457799BBCDFF1"));
        assert_eq!(
            des.encrypt(block("0123456789ABCDEF")),
            block("85E813540F0AB405")
        );
        assert_eq!(
            des.decrypt(block("85E813540F0AB405")),
            block("0123456789ABCDEF")
        );
        let fips = Des::new(&block("0123456789ABCDEF"));
        assert_eq!(
            fips.encrypt(block("4E6F772069732074")),
            block("3FA40E8A984D4815")
        );
        // Triple DES with three equal keys is DES.
        let k = hex("0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF");
        let t = TripleDes::new(&k).unwrap();
        assert_eq!(
            t.encrypt(block("4E6F772069732074")),
            block("3FA40E8A984D4815")
        );
        assert_eq!(
            t.decrypt(block("3FA40E8A984D4815")),
            block("4E6F772069732074")
        );
        assert!(TripleDes::new(&k[..20]).is_none());
    }
    #[test]
    fn rc2_matches_rfc_2268() {
        let mut sorted = PITABLE;
        sorted.sort_unstable();
        assert!(sorted.iter().enumerate().all(|(i, &v)| usize::from(v) == i));
        for (key, bits, plain, cipher) in [
            (
                "0000000000000000",
                63,
                "0000000000000000",
                "ebb773f993278eff",
            ),
            (
                "ffffffffffffffff",
                64,
                "ffffffffffffffff",
                "278b27e42e2f0d49",
            ),
            (
                "3000000000000000",
                64,
                "1000000000000001",
                "30649edf9be7d2c2",
            ),
            ("88", 64, "0000000000000000", "61a8a244adacccf0"),
            ("88bca90e90875a", 64, "0000000000000000", "6ccf4308974c267f"),
            (
                "88bca90e90875a7f0f79c384627bafb2",
                64,
                "0000000000000000",
                "1a807d272bbe5db1",
            ),
            (
                "88bca90e90875a7f0f79c384627bafb2",
                128,
                "0000000000000000",
                "2269552ab0f85ca6",
            ),
        ] {
            let rc2 = Rc2::new(&hex(key), bits).unwrap();
            assert_eq!(rc2.encrypt(block(plain)), block(cipher), "{key}/{bits}");
            assert_eq!(rc2.decrypt(block(cipher)), block(plain), "{key}/{bits}");
        }
    }
    #[test]
    fn rc4_matches_the_published_vectors() {
        for (key, plain, cipher) in [
            ("4b6579", "506c61696e74657874", "bbf316e8d940af0ad3"),
            ("57696b69", "7065646961", "1021bf0420"),
            (
                "536563726574",
                "41747461636b206174206461776e",
                "45a01f645fc35b383552544b9bf5",
            ),
        ] {
            let mut data = hex(plain);
            Rc4::new(&hex(key)).apply(&mut data);
            assert_eq!(data, hex(cipher));
        }
    }
}

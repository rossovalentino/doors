//! MD4 (RFC 1320) and MD5 (RFC 1321), and HMAC-MD5 (RFC 2104): the hashes
//! NTLM and HTTP Digest are defined over. Neither is a secure hash today;
//! they are here because those protocols require them.

fn pad(message: &[u8]) -> alloc::vec::Vec<u8> {
    let mut m = message.to_vec();
    let bits = (message.len() as u64).wrapping_mul(8);
    m.push(0x80);
    while m.len() % 64 != 56 {
        m.push(0);
    }
    m.extend_from_slice(&bits.to_le_bytes());
    m
}
fn words(block: &[u8]) -> [u32; 16] {
    let mut x = [0u32; 16];
    for (i, w) in x.iter_mut().enumerate() {
        *w = u32::from_le_bytes(block[4 * i..4 * i + 4].try_into().unwrap());
    }
    x
}
fn digest(state: [u32; 4]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (i, s) in state.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&s.to_le_bytes());
    }
    out
}
const INITIAL: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];

pub fn md4(message: &[u8]) -> [u8; 16] {
    let mut s = INITIAL;
    for block in pad(message).chunks_exact(64) {
        let x = words(block);
        let [mut a, mut b, mut c, mut d] = s;
        let f = |x: u32, y: u32, z: u32| (x & y) | (!x & z);
        let g = |x: u32, y: u32, z: u32| (x & y) | (x & z) | (y & z);
        let h = |x: u32, y: u32, z: u32| x ^ y ^ z;
        for i in 0..16 {
            let k = i;
            let r = [3, 7, 11, 19][i % 4];
            let t = a.wrapping_add(f(b, c, d)).wrapping_add(x[k]).rotate_left(r);
            (a, b, c, d) = (d, t, b, c);
        }
        for i in 0..16 {
            let k = (i % 4) * 4 + i / 4;
            let r = [3, 5, 9, 13][i % 4];
            let t = a
                .wrapping_add(g(b, c, d))
                .wrapping_add(x[k])
                .wrapping_add(0x5a82_7999)
                .rotate_left(r);
            (a, b, c, d) = (d, t, b, c);
        }
        const ORDER: [usize; 16] = [0, 8, 4, 12, 2, 10, 6, 14, 1, 9, 5, 13, 3, 11, 7, 15];
        for (i, &k) in ORDER.iter().enumerate() {
            let r = [3, 9, 11, 15][i % 4];
            let t = a
                .wrapping_add(h(b, c, d))
                .wrapping_add(x[k])
                .wrapping_add(0x6ed9_eba1)
                .rotate_left(r);
            (a, b, c, d) = (d, t, b, c);
        }
        s[0] = s[0].wrapping_add(a);
        s[1] = s[1].wrapping_add(b);
        s[2] = s[2].wrapping_add(c);
        s[3] = s[3].wrapping_add(d);
    }
    digest(s)
}

const K5: [u32; 64] = [
    0xd76a_a478,
    0xe8c7_b756,
    0x2420_70db,
    0xc1bd_ceee,
    0xf57c_0faf,
    0x4787_c62a,
    0xa830_4613,
    0xfd46_9501,
    0x6980_98d8,
    0x8b44_f7af,
    0xffff_5bb1,
    0x895c_d7be,
    0x6b90_1122,
    0xfd98_7193,
    0xa679_438e,
    0x49b4_0821,
    0xf61e_2562,
    0xc040_b340,
    0x265e_5a51,
    0xe9b6_c7aa,
    0xd62f_105d,
    0x0244_1453,
    0xd8a1_e681,
    0xe7d3_fbc8,
    0x21e1_cde6,
    0xc337_07d6,
    0xf4d5_0d87,
    0x455a_14ed,
    0xa9e3_e905,
    0xfcef_a3f8,
    0x676f_02d9,
    0x8d2a_4c8a,
    0xfffa_3942,
    0x8771_f681,
    0x6d9d_6122,
    0xfde5_380c,
    0xa4be_ea44,
    0x4bde_cfa9,
    0xf6bb_4b60,
    0xbebf_bc70,
    0x289b_7ec6,
    0xeaa1_27fa,
    0xd4ef_3085,
    0x0488_1d05,
    0xd9d4_d039,
    0xe6db_99e5,
    0x1fa2_7cf8,
    0xc4ac_5665,
    0xf429_2244,
    0x432a_ff97,
    0xab94_23a7,
    0xfc93_a039,
    0x655b_59c3,
    0x8f0c_cc92,
    0xffef_f47d,
    0x8584_5dd1,
    0x6fa8_7e4f,
    0xfe2c_e6e0,
    0xa301_4314,
    0x4e08_11a1,
    0xf753_7e82,
    0xbd3a_f235,
    0x2ad7_d2bb,
    0xeb86_d391,
];
const R5: [u32; 16] = [7, 12, 17, 22, 5, 9, 14, 20, 4, 11, 16, 23, 6, 10, 15, 21];

pub fn md5(message: &[u8]) -> [u8; 16] {
    let mut s = INITIAL;
    for block in pad(message).chunks_exact(64) {
        let x = words(block);
        let [mut a, mut b, mut c, mut d] = s;
        for i in 0..64 {
            let (f, k) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let r = R5[(i / 16) * 4 + i % 4];
            let t = a
                .wrapping_add(f)
                .wrapping_add(K5[i])
                .wrapping_add(x[k])
                .rotate_left(r);
            (a, b, c, d) = (d, b.wrapping_add(t), b, c);
        }
        s[0] = s[0].wrapping_add(a);
        s[1] = s[1].wrapping_add(b);
        s[2] = s[2].wrapping_add(c);
        s[3] = s[3].wrapping_add(d);
    }
    digest(s)
}

pub fn hmac_md5(key: &[u8], parts: &[&[u8]]) -> [u8; 16] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..16].copy_from_slice(&md5(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner: alloc::vec::Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    for p in parts {
        inner.extend_from_slice(p);
    }
    let mut outer: alloc::vec::Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
    outer.extend_from_slice(&md5(&inner));
    md5(&outer)
}

pub fn hex(bytes: &[u8]) -> alloc::string::String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|b| {
            [
                DIGITS[usize::from(b >> 4)] as char,
                DIGITS[usize::from(b & 15)] as char,
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn md4_matches_rfc1320() {
        assert_eq!(hex(&md4(b"")), "31d6cfe0d16ae931b73c59d7e0c089c0");
        assert_eq!(hex(&md4(b"abc")), "a448017aaf21d8525fc10ae87aa6729d");
        assert_eq!(
            hex(&md4(
                b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"
            )),
            "e33b4ddc9c38f2199c3e7b164fcc0536"
        );
    }
    #[test]
    fn md5_matches_rfc1321() {
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            hex(&md5(
                b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"
            )),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
    }
    #[test]
    fn hmac_md5_matches_rfc2104() {
        assert_eq!(
            hex(&hmac_md5(b"Jefe", &[b"what do ya want for nothing?"])),
            "750c783e6ab0b503eaa86e310a5db738"
        );
    }
}

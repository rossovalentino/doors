//! Shared host-test font loading: owner-local Windows fonts from `assets/fonts`
//! are read once and leaked for the process lifetime (the guest maps them as
//! shared read-only sections instead).
#![allow(dead_code)]
use door_graphics_text::{FontRole, FontSet, TextSystem};
use std::path::PathBuf;
use std::sync::OnceLock;

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub const FILES: &[(&str, FontRole)] = &[
    ("assets/fonts/segoe-ui/SegUIVar.ttf", FontRole::UiVariable),
    ("assets/fonts/segoe-ui/segoeui.ttf", FontRole::Ui),
    ("assets/fonts/segoe-ui/seguisb.ttf", FontRole::Ui),
    (
        "assets/fonts/windows-unicode/seguisym.ttf",
        FontRole::Symbol,
    ),
    ("assets/fonts/windows-unicode/seguiemj.ttf", FontRole::Emoji),
    (
        "assets/fonts/windows-unicode/seguihis.ttf",
        FontRole::Historic,
    ),
    (
        "assets/fonts/windows-unicode/msyh.ttc",
        FontRole::ChineseSimplified,
    ),
    (
        "assets/fonts/windows-unicode/msjh.ttc",
        FontRole::ChineseTraditional,
    ),
    (
        "assets/fonts/windows-unicode/YuGothM.ttc",
        FontRole::Japanese,
    ),
    ("assets/fonts/windows-unicode/malgun.ttf", FontRole::Korean),
    ("assets/fonts/windows-unicode/Nirmala.ttc", FontRole::Indic),
    (
        "assets/fonts/cascadia-mono/CascadiaMono.ttf",
        FontRole::Monospace,
    ),
];

pub fn bytes() -> &'static [(&'static [u8], FontRole)] {
    static BYTES: OnceLock<Vec<(&'static [u8], FontRole)>> = OnceLock::new();
    BYTES.get_or_init(|| {
        FILES
            .iter()
            .map(|(path, role)| {
                let data = std::fs::read(root().join(path))
                    .unwrap_or_else(|e| panic!("owner-local font {path} missing: {e}"));
                (&*Box::leak(data.into_boxed_slice()), *role)
            })
            .collect()
    })
}

pub fn fonts() -> FontSet {
    FontSet::from_files(bytes()).expect("font set")
}

pub fn system() -> &'static TextSystem {
    static SYSTEM: OnceLock<TextSystem> = OnceLock::new();
    SYSTEM.get_or_init(|| TextSystem::new(fonts()))
}

/// Write premultiplied BGRA pixels as a binary PPM over white for inspection.
pub fn write_ppm(path: &std::path::Path, pixels: &[u8], width: u32, height: u32) {
    let mut out = format!("P6\n{width} {height}\n255\n").into_bytes();
    for p in pixels.chunks_exact(4) {
        out.extend_from_slice(&[p[2], p[1], p[0]]);
    }
    std::fs::write(path, out).unwrap();
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Minimal uncompressed PNG (stored deflate) of premultiplied BGRA pixels
/// composited over white, for visual inspection of host renders.
pub fn write_png(path: &std::path::Path, pixels: &[u8], width: u32, height: u32) {
    let mut raw = Vec::new();
    for row in pixels.chunks_exact(width as usize * 4) {
        raw.push(0u8);
        for p in row.chunks_exact(4) {
            let inv = 255 - u32::from(p[3]);
            for c in [p[2], p[1], p[0]] {
                raw.push((u32::from(c) + inv).min(255) as u8);
            }
        }
    }
    let mut z = vec![0x78, 0x01];
    for (i, chunk) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(u8::from(last));
        z.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(chunk.len() as u16)).to_le_bytes());
        z.extend_from_slice(chunk);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut chunk = |tag: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = tag.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    };
    let mut ihdr = width.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    std::fs::write(path, out).unwrap();
}

//! Immutable, authenticated desktop resource pack. No allocation or user paths.
//!
//! Version 2 (Settings parity, 12 November 2026): every built-in Windows 11
//! 26H2 wallpaper (`C:\Windows\Web\Wallpaper`), stored as its original JPEG
//! (`format` 1, decoded by the window server) or as lossless QOI
//! (`format` 0). Ids 0..=3 keep their version 1 meaning (Bloom, dark Bloom,
//! the first Glow and the first Sunrise picture), so stored appearance words
//! stay valid.
//!
//! Layout: magic, u32 count (`COUNT`), u32 total length, then `COUNT`
//! entries of four u32 (id, offset, size, format), then the payloads in id
//! order with no gaps; zero padding may follow `length`.
pub const COUNT: usize = 18;
pub const HEADER: usize = 16 + COUNT * 16;
pub const MAX_BYTES: usize = 32 * 1024 * 1024;
pub const MAGIC: &[u8; 8] = b"DOORART2";
/// Payload encodings.
pub const QOI: u32 = 0;
pub const JPEG: u32 = 1;
/// A built-in picture's theme folder (`C:\Windows\Web\Wallpaper\<folder>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Album {
    /// `Windows`: Bloom and dark Bloom (Doors (light) and (dark) themes).
    Windows,
    /// `ThemeA`: Glow.
    Glow,
    /// `ThemeB`: Captured Motion.
    CapturedMotion,
    /// `ThemeC`: Sunrise.
    Sunrise,
    /// `ThemeD`: Flow.
    Flow,
}
impl Album {
    /// Member pictures in slideshow order (the folder's file order).
    pub const fn members(self) -> &'static [u32] {
        match self {
            Self::Windows => &[0, 1],
            Self::Glow => &[2, 4, 5, 6],
            Self::CapturedMotion => &[7, 8, 9, 10],
            Self::Sunrise => &[3, 11, 12, 13],
            Self::Flow => &[14, 15, 16, 17],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wallpaper {
    /// File name in `C:\Windows\Web\Wallpaper\<folder>` and in
    /// `assets/wallpapers/windows/26h2`.
    pub file: &'static str,
    /// Display name (Settings tooltips and narration).
    pub name: &'static str,
    pub album: Album,
}
const fn w(file: &'static str, name: &'static str, album: Album) -> Wallpaper {
    Wallpaper { file, name, album }
}
/// Pack ids, in pack order.
pub const WALLPAPERS: [Wallpaper; COUNT] = [
    w("img0.jpg", "Bloom", Album::Windows),
    w("img19.jpg", "Dark Bloom", Album::Windows),
    w("img20.jpg", "Glow 1", Album::Glow),
    w("img28.jpg", "Sunrise 1", Album::Sunrise),
    w("img21.jpg", "Glow 2", Album::Glow),
    w("img22.jpg", "Glow 3", Album::Glow),
    w("img23.jpg", "Glow 4", Album::Glow),
    w("img24.jpg", "Captured Motion 1", Album::CapturedMotion),
    w("img25.jpg", "Captured Motion 2", Album::CapturedMotion),
    w("img26.jpg", "Captured Motion 3", Album::CapturedMotion),
    w("img27.jpg", "Captured Motion 4", Album::CapturedMotion),
    w("img29.jpg", "Sunrise 2", Album::Sunrise),
    w("img30.jpg", "Sunrise 3", Album::Sunrise),
    w("img31.jpg", "Sunrise 4", Album::Sunrise),
    w("img32.jpg", "Flow 1", Album::Flow),
    w("img33.jpg", "Flow 2", Album::Flow),
    w("img34.jpg", "Flow 3", Album::Flow),
    w("img35.jpg", "Flow 4", Album::Flow),
];
/// The next picture of `id`'s album after `id` (slideshow step).
pub fn next_in_album(id: u32) -> u32 {
    let Some(w) = WALLPAPERS.get(id as usize) else {
        return 0;
    };
    let members = w.album.members();
    let at = members.iter().position(|m| *m == id).unwrap_or(0);
    members[(at + 1) % members.len()]
}
const QOI_HEADER: [u8; 14] = [b'q', b'o', b'i', b'f', 0, 0, 15, 0, 0, 0, 9, 96, 4, 0];
#[derive(Clone, Copy, Debug)]
pub struct Pack<'a> {
    bytes: &'a [u8],
}
impl<'a> Pack<'a> {
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        let word = |at| {
            bytes
                .get(at..at + 4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
        };
        if bytes.get(..8)? != MAGIC || word(8)? != COUNT {
            return None;
        }
        let length = word(12)?;
        if !(HEADER..=MAX_BYTES).contains(&length)
            || length > bytes.len()
            || bytes[length..].iter().any(|&v| v != 0)
        {
            return None;
        }
        let mut end = HEADER;
        for id in 0..COUNT {
            let at = 16 + id * 16;
            let offset = word(at + 4)?;
            let size = word(at + 8)?;
            if word(at)? != id || offset != end || size < 22 {
                return None;
            }
            end = offset.checked_add(size)?;
            if end > length {
                return None;
            }
            let payload = &bytes[offset..end];
            let valid = match word(at + 12)? as u32 {
                QOI => payload.get(..14)? == QOI_HEADER,
                JPEG => {
                    payload.get(..3)? == [0xff, 0xd8, 0xff] && payload[size - 2..] == [0xff, 0xd9]
                }
                _ => false,
            };
            if !valid {
                return None;
            }
        }
        (end == length).then_some(Self {
            bytes: &bytes[..length],
        })
    }
    /// Picture `id`'s encoded bytes (QOI or JPEG: see `format`).
    pub fn background(self, id: u32) -> Option<&'a [u8]> {
        if id as usize >= COUNT {
            return None;
        }
        let at = 16 + id as usize * 16;
        let word = |at| u32::from_le_bytes(self.bytes[at..at + 4].try_into().unwrap()) as usize;
        Some(&self.bytes[word(at + 4)..word(at + 4) + word(at + 8)])
    }
    pub fn format(self, id: u32) -> Option<u32> {
        if id as usize >= COUNT {
            return None;
        }
        let at = 16 + id as usize * 16 + 12;
        Some(u32::from_le_bytes(
            self.bytes[at..at + 4].try_into().unwrap(),
        ))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;
    fn pack() -> std::vec::Vec<u8> {
        let mut bytes = vec![0; HEADER + COUNT * 24];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8..12].copy_from_slice(&(COUNT as u32).to_le_bytes());
        let n = bytes.len() as u32;
        bytes[12..16].copy_from_slice(&n.to_le_bytes());
        for id in 0..COUNT {
            let at = 16 + id * 16;
            let offset = HEADER + id * 24;
            let format = (id % 2) as u32;
            bytes[at..at + 4].copy_from_slice(&(id as u32).to_le_bytes());
            bytes[at + 4..at + 8].copy_from_slice(&(offset as u32).to_le_bytes());
            bytes[at + 8..at + 12].copy_from_slice(&24u32.to_le_bytes());
            bytes[at + 12..at + 16].copy_from_slice(&format.to_le_bytes());
            if format == QOI {
                bytes[offset..offset + 14].copy_from_slice(&QOI_HEADER);
            } else {
                bytes[offset..offset + 3].copy_from_slice(&[0xff, 0xd8, 0xff]);
                bytes[offset + 22..offset + 24].copy_from_slice(&[0xff, 0xd9]);
            }
        }
        bytes
    }
    #[test]
    fn rejects_aliases_truncation_wrong_ids_formats_and_padding() {
        let mut bytes = pack();
        let p = Pack::parse(&bytes).unwrap();
        assert_eq!(p.format(0), Some(QOI));
        assert_eq!(p.format(1), Some(JPEG));
        assert_eq!(p.background(1).unwrap()[..3], [0xff, 0xd8, 0xff]);
        let jpeg = HEADER + 24;
        for (at, value) in [
            (16, 1),
            (20, 0),
            (28, 4),
            (HEADER + 6, 0),
            (jpeg + 1, 0),
            (jpeg + 23, 0),
            (12, 0),
        ] {
            let mut bad = bytes.clone();
            bad[at] = value;
            assert!(Pack::parse(&bad).is_none(), "{at}");
        }
        assert!(Pack::parse(&bytes[..bytes.len() - 1]).is_none());
        assert!(
            Pack::parse(&bytes)
                .unwrap()
                .background(COUNT as u32)
                .is_none()
        );
        bytes.push(1);
        assert!(Pack::parse(&bytes).is_none());
    }
    #[test]
    fn albums_cover_every_picture_once_and_keep_version_one_ids() {
        let mut seen = [false; COUNT];
        for album in [
            Album::Windows,
            Album::Glow,
            Album::CapturedMotion,
            Album::Sunrise,
            Album::Flow,
        ] {
            for &id in album.members() {
                assert!(!seen[id as usize]);
                seen[id as usize] = true;
                assert_eq!(WALLPAPERS[id as usize].album, album);
            }
        }
        assert!(seen.iter().all(|s| *s));
        assert_eq!(
            [
                WALLPAPERS[0].file,
                WALLPAPERS[1].file,
                WALLPAPERS[2].file,
                WALLPAPERS[3].file
            ],
            ["img0.jpg", "img19.jpg", "img20.jpg", "img28.jpg"]
        );
        assert_eq!(next_in_album(2), 4);
        assert_eq!(next_in_album(6), 2);
        assert_eq!(next_in_album(1), 0);
    }
}

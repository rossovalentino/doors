use super::*;
use alloc::vec;

/// A 2x2 32-bit DIB frame: red, green / blue, half-transparent white, top
/// row first in the result (the DIB stores rows bottom-up).
fn dib32() -> Vec<u8> {
    let mut d = Vec::new();
    for v in [40u32, 2, 4] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&1u16.to_le_bytes());
    d.extend_from_slice(&32u16.to_le_bytes());
    d.extend_from_slice(&[0; 24]);
    // Bottom row: blue, white 50%; top row: red, green.
    d.extend_from_slice(&[255, 0, 0, 255, 255, 255, 255, 128]);
    d.extend_from_slice(&[0, 0, 255, 255, 0, 255, 0, 255]);
    d.extend_from_slice(&[0; 8]); // AND mask, 2 rows of 4 bytes
    d
}
/// A 2x2 4-bit DIB with a palette and an AND mask hiding the top-right pixel.
fn dib4() -> Vec<u8> {
    let mut d = Vec::new();
    for v in [40u32, 2, 4] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.extend_from_slice(&1u16.to_le_bytes());
    d.extend_from_slice(&4u16.to_le_bytes());
    d.extend_from_slice(&[0; 24]);
    let mut palette = vec![0u8; 64];
    palette[4..8].copy_from_slice(&[0, 0, 255, 0]); // 1: red
    d.extend_from_slice(&palette);
    d.extend_from_slice(&[0x11, 0, 0, 0]); // bottom: red, red
    d.extend_from_slice(&[0x10, 0, 0, 0]); // top: red, black
    d.extend_from_slice(&[0, 0, 0, 0]); // bottom mask: opaque
    d.extend_from_slice(&[0x40, 0, 0, 0]); // top mask: second pixel transparent
    d
}
/// A PE32+ file whose .rsrc holds RT_ICON 7 (`frame`) and RT_GROUP_ICON 1
/// listing it as `size` px.
fn program(frame: &[u8], size: u8) -> Vec<u8> {
    // Resource tree at .rsrc offset 0 (RVA 0x1000, file 0x200).
    let dir = |entries: &[(u32, u32)]| {
        let mut d = vec![0u8; 12];
        d.extend_from_slice(&0u16.to_le_bytes());
        d.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (id, target) in entries {
            d.extend_from_slice(&id.to_le_bytes());
            d.extend_from_slice(&target.to_le_bytes());
        }
        d
    };
    let mut group = vec![0, 0, 1, 0, 1, 0];
    group.extend_from_slice(&[size, size, 0, 0, 1, 0, 32, 0]);
    group.extend_from_slice(&(frame.len() as u32).to_le_bytes());
    group.extend_from_slice(&7u16.to_le_bytes());
    // Layout: root 0 (32), type3 32 (24), name3 56 (24), type14 80 (24),
    // name14 104 (24), data3 128 (16), data14 144 (16), blobs from 160.
    let (frame_at, group_at) = (160u32, 160 + frame.len() as u32);
    let high = 0x8000_0000;
    let mut rsrc = dir(&[(3, high | 32), (14, high | 80)]);
    rsrc.extend(dir(&[(7, high | 56)]));
    rsrc.extend(dir(&[(0x409, 128)]));
    rsrc.extend(dir(&[(1, high | 104)]));
    rsrc.extend(dir(&[(0x409, 144)]));
    for (at, len) in [(frame_at, frame.len()), (group_at, group.len())] {
        rsrc.extend_from_slice(&(0x1000 + at).to_le_bytes());
        rsrc.extend_from_slice(&(len as u32).to_le_bytes());
        rsrc.extend_from_slice(&[0; 8]);
    }
    rsrc.extend_from_slice(frame);
    rsrc.extend_from_slice(&group);
    while rsrc.len() % 0x200 != 0 {
        rsrc.push(0);
    }
    let mut f = vec![0u8; 0x200];
    f[..2].copy_from_slice(b"MZ");
    f[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    f[0x80..0x84].copy_from_slice(b"PE\0\0");
    f[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
    f[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
    f[0x94..0x96].copy_from_slice(&240u16.to_le_bytes());
    let opt = 0x98;
    f[opt..opt + 2].copy_from_slice(&0x20Bu16.to_le_bytes());
    f[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
    f[opt + 128..opt + 132].copy_from_slice(&0x1000u32.to_le_bytes());
    f[opt + 132..opt + 136].copy_from_slice(&(rsrc.len() as u32).to_le_bytes());
    let sec = opt + 240;
    f[sec..sec + 5].copy_from_slice(b".rsrc");
    f[sec + 8..sec + 12].copy_from_slice(&(rsrc.len() as u32).to_le_bytes());
    f[sec + 12..sec + 16].copy_from_slice(&0x1000u32.to_le_bytes());
    f[sec + 16..sec + 20].copy_from_slice(&(rsrc.len() as u32).to_le_bytes());
    f[sec + 20..sec + 24].copy_from_slice(&0x200u32.to_le_bytes());
    f.extend(rsrc);
    f
}
#[test]
fn dib_frames_decode_to_premultiplied_top_down_bgra() {
    let image = decode_dib(&dib32()).unwrap();
    assert_eq!((image.width, image.height), (2, 2));
    assert_eq!(&image.bgra[..8], &[0, 0, 255, 255, 0, 255, 0, 255]);
    assert_eq!(&image.bgra[8..], &[255, 0, 0, 255, 128, 128, 128, 128]);
    let image = decode_dib(&dib4()).unwrap();
    assert_eq!(&image.bgra[..8], &[0, 0, 255, 255, 0, 0, 0, 0]);
    assert_eq!(&image.bgra[8..], &[0, 0, 255, 255, 0, 0, 255, 255]);
    assert_eq!(decode_dib(b"too short"), None);
}
#[test]
fn the_first_group_names_its_frames_and_they_are_found() {
    let frame = dib32();
    let file = program(&frame, 2);
    let entries = group(&file[..]).unwrap().unwrap();
    assert_eq!(
        entries,
        [Entry {
            width: 2,
            height: 2,
            bits: 32,
            id: 7
        }]
    );
    assert_eq!(frame_bytes(&file, &entries[0]), Some(&frame[..]));
    assert_eq!(
        group(&b"MZ not a program at all, just text............................"[..]),
        Ok(None)
    );
}
fn frame_bytes<'a>(file: &'a [u8], e: &Entry) -> Option<&'a [u8]> {
    frame(file, e).unwrap()
}
#[test]
fn ranged_sources_ask_for_what_they_lack() {
    /// Holds only the first `held` bytes.
    struct Partial<'a>(&'a [u8], usize);
    impl Source for Partial<'_> {
        fn size(&self) -> u64 {
            self.0.len() as u64
        }
        fn bytes(&self, offset: u64, len: u64) -> Result<&[u8], Miss> {
            if offset + len > self.1 as u64 {
                return Err(Miss::Need(offset, len));
            }
            self.0.bytes(offset, len)
        }
    }
    let file = program(&dib32(), 2);
    let mut held = 64;
    let entries = loop {
        match group(&Partial(&file, held)) {
            Ok(found) => break found.unwrap(),
            Err(Miss::Need(at, len)) => held = (at + len) as usize,
            Err(Miss::Malformed) => panic!("malformed"),
        }
    };
    assert_eq!(entries.len(), 1);
}
#[test]
fn the_smallest_frame_that_covers_the_target_wins() {
    let e = |width, bits, id| Entry {
        width,
        height: width,
        bits,
        id,
    };
    let entries = [e(16, 8, 1), e(16, 32, 2), e(32, 32, 3), e(256, 32, 4)];
    assert_eq!(choose(&entries, 16).map(|e| e.id), Some(2));
    assert_eq!(choose(&entries, 24).map(|e| e.id), Some(3));
    assert_eq!(choose(&entries, 48).map(|e| e.id), Some(4));
    assert_eq!(choose(&entries[..2], 48).map(|e| e.id), Some(2));
    assert_eq!(choose(&[], 16), None);
}
/// Real programs: DOOR_PE_ICON_LIVE lists paths separated by `;`; each must
/// have an icon group whose 32 px (or nearest) frame decodes, or none when
/// the path ends in `=!` (Windows draws the generic icon). `--ignored`.
#[test]
#[ignore]
fn live_programs_have_icons() {
    let Ok(list) = std::env::var("DOOR_PE_ICON_LIVE") else {
        return;
    };
    for item in list.split(';').filter(|s| !s.is_empty()) {
        let (path, none) = match item.strip_suffix("=!") {
            Some(path) => (path, true),
            None => (item, false),
        };
        let bytes = std::fs::read(path).unwrap();
        let entries = group(&bytes[..]).unwrap().unwrap_or_default();
        if none {
            assert!(entries.is_empty(), "{path}");
            continue;
        }
        let chosen = choose(&entries, 32).unwrap_or_else(|| panic!("{path}: no icon"));
        let data = frame(&bytes[..], &chosen).unwrap().unwrap();
        std::println!(
            "{path}: {} frames, 32 px -> {}x{} {} bits {}",
            entries.len(),
            chosen.width,
            chosen.height,
            chosen.bits,
            if is_png(data) { "PNG" } else { "DIB" }
        );
        if !is_png(data) {
            let image = decode_dib(data).unwrap();
            assert_eq!(image.width, chosen.width, "{path}");
        }
    }
}
#[test]
fn groups_are_found_by_index_and_id_and_counted() {
    let file = program(&dib32(), 2);
    assert_eq!(group_count(&file[..]), Ok(1));
    let by_index = group_at(&file[..], Which::Index(0)).unwrap().unwrap();
    assert_eq!(
        by_index,
        group_at(&file[..], Which::Id(1)).unwrap().unwrap()
    );
    assert_eq!(group_at(&file[..], Which::Index(1)), Ok(None));
    assert_eq!(group_at(&file[..], Which::Id(2)), Ok(None));
    assert_eq!(group_count(&b"not a program"[..]), Ok(0));
}
/// An `.ico` file holding `frames` (each a DIB of the stated size).
fn ico(frames: &[(u8, &[u8])], kind: u16) -> Vec<u8> {
    let mut f = vec![0, 0];
    f.extend_from_slice(&kind.to_le_bytes());
    f.extend_from_slice(&(frames.len() as u16).to_le_bytes());
    let mut at = 6 + 16 * frames.len() as u32;
    for (size, data) in frames {
        f.extend_from_slice(&[*size, *size, 0, 0]);
        f.extend_from_slice(&[3, 0, 32, 0]);
        f.extend_from_slice(&(data.len() as u32).to_le_bytes());
        f.extend_from_slice(&at.to_le_bytes());
        at += data.len() as u32;
    }
    for (_, data) in frames {
        f.extend_from_slice(data);
    }
    f
}
#[test]
fn icon_and_cursor_files_list_their_frames() {
    let (a, b) = (dib32(), dib4());
    let file = ico(&[(2, &a), (2, &b)], 1);
    let (cursor, frames) = icon_file(&file).unwrap();
    assert!(!cursor);
    assert_eq!(frames.len(), 2);
    let first = &file[frames[0].offset as usize..][..frames[0].size as usize];
    assert_eq!(decode(first), decode_dib(&a));
    let (cursor, frames) = icon_file(&ico(&[(2, &a)], 2)).unwrap();
    assert!(cursor);
    assert_eq!(frames[0].hotspot, (3, 32));
    // Truncated, zero frames, a frame past the end, a bitmap file.
    let mut short = file.clone();
    short.truncate(file.len() - 1);
    for bad in [&short[..], &[0, 0, 1, 0, 0, 0][..], b"BM\0\0\0\0"] {
        assert!(icon_file(bad).is_none());
    }
}
#[test]
fn directories_and_sizes_choose_like_lookup_icon_id() {
    let mut group = vec![0, 0, 1, 0, 3, 0];
    for (size, bits, id) in [(16u8, 32u16, 1u16), (32, 8, 2), (32, 32, 3), (0, 32, 4)] {
        group.extend_from_slice(&[size, size, 0, 0, 1, 0]);
        group.extend_from_slice(&bits.to_le_bytes());
        group.extend_from_slice(&[0; 4]);
        group.extend_from_slice(&id.to_le_bytes());
    }
    group[4] = 4;
    let entries = directory(&group).unwrap();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[3].width, 256);
    assert_eq!(choose_size(&entries, 32, 32).unwrap().id, 3);
    assert_eq!(choose_size(&entries, 20, 20).unwrap().id, 3);
    assert_eq!(choose_size(&entries, 16, 16).unwrap().id, 1);
    assert_eq!(choose_size(&entries, 300, 300).unwrap().id, 4);
    assert!(directory(&group[..20]).is_none());
}
#[cfg(feature = "qoi")]
#[test]
fn qoi_assets_decode_as_stored() {
    let rgba = [10u8, 20, 30, 255, 0, 0, 0, 0];
    let encoded = qoi::encode_to_vec(rgba, 2, 1).unwrap();
    let image = decode_qoi(&encoded).unwrap();
    assert_eq!((image.width, image.height), (2, 1));
    assert_eq!(image.bgra, [30, 20, 10, 255, 0, 0, 0, 0]);
}

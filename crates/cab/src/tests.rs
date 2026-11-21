extern crate std;
use super::*;
use alloc::vec;

// makecab.exe (Windows 11) packing the same three files with LZX:21,
// LZX:15 and MSZIP: text.txt (60000 bytes of words), code.bin (30000 bytes
// rich in E8 call opcodes) and tiny.txt.
const LZX21: &[u8] = include_bytes!("../testdata/lzx21.cab");
const LZX15: &[u8] = include_bytes!("../testdata/lzx15.cab");
const MSZIP: &[u8] = include_bytes!("../testdata/mszip.cab");

fn files(cab: &Cabinet) -> Vec<(String, Vec<u8>)> {
    cab.files
        .iter()
        .map(|f| (f.name.clone(), cab.extract(f).expect("extract")))
        .collect()
}

#[test]
fn makecab_lzx_and_mszip_agree() {
    let mszip = Cabinet::parse(MSZIP).unwrap();
    assert_eq!(mszip.folders[0].method, Method::MsZip);
    let reference = files(&mszip);
    let names: Vec<&str> = reference.iter().map(|f| f.0.as_str()).collect();
    assert_eq!(names, ["text.txt", "code.bin", "tiny.txt"]);
    assert_eq!(reference[0].1.len(), 60000);
    assert_eq!(reference[1].1.len(), 30000);
    assert_eq!(reference[2].1, b"hello, cabinet\r\n");
    assert!(reference[0].1.iter().all(|b| b.is_ascii()));
    for (bytes, window) in [(LZX21, 21), (LZX15, 15)] {
        let cab = Cabinet::parse(bytes).unwrap();
        assert_eq!(cab.folders[0].method, Method::Lzx(window));
        assert!(cab.folders[0].blocks > 1, "frames span several blocks");
        assert_eq!(files(&cab), reference, "LZX:{window}");
    }
    // A file by name, case-insensitively.
    let cab = Cabinet::parse(LZX21).unwrap();
    let tiny = cab.file("TINY.TXT").unwrap();
    assert_eq!(cab.extract(tiny).unwrap(), b"hello, cabinet\r\n");
}

#[test]
fn writer_round_trips_and_checksums() {
    let mut big = Vec::new();
    for i in 0..90_000u32 {
        big.push((i * 7 % 251) as u8 ^ (i / 1000) as u8);
    }
    let text = b"Doors setup ".repeat(4000);
    for compression in [Compression::Stored, Compression::MsZip] {
        let mut w = Writer::new(compression);
        w.add("setup\\data.bin", &big, 0x5a21, 0x6000, 0x20);
        w.add("readme.txt", &text, 0x5a21, 0x6000, 0x20);
        w.add("ünïcode.txt", b"x", 0, 0, 0);
        let bytes = w.finish();
        let cab = Cabinet::parse(&bytes).unwrap();
        assert_eq!(cab.files.len(), 3);
        assert_eq!(cab.extract(&cab.files[0]).unwrap(), big);
        assert_eq!(cab.extract(&cab.files[1]).unwrap(), text);
        assert_eq!(cab.files[2].name, "ünïcode.txt");
        assert_ne!(cab.files[2].attributes & ATTRIBUTE_UTF8, 0);
        assert_eq!((cab.files[0].date, cab.files[0].time), (0x5a21, 0x6000));
        if compression == Compression::MsZip {
            assert!(bytes.len() < (big.len() + text.len()) / 2, "compresses");
        }
    }
}

#[test]
fn damaged_cabinets_are_refused() {
    let mut w = Writer::new(Compression::MsZip);
    w.add("a.txt", &b"abcdefgh".repeat(9000), 0, 0, 0);
    let good = w.finish();
    // A flipped data byte fails the checksum.
    let mut bad = good.clone();
    let last = bad.len() - 3;
    bad[last] ^= 0x40;
    let cab = Cabinet::parse(&bad).unwrap();
    assert_eq!(cab.extract(&cab.files[0]), Err(Error::Corrupt));
    // Not a cabinet; truncated; a declared size past the end.
    assert!(Cabinet::parse(b"MSCX").is_err());
    assert!(Cabinet::parse(&good[..30]).is_err());
    let mut long = good.clone();
    long[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Cabinet::parse(&long).is_err());
    // A cabinet set is not supported.
    let mut set = good.clone();
    set[30] |= 0x2;
    assert_eq!(Cabinet::parse(&set).err(), Some(Error::Unsupported));
    // A file reaching past its folder.
    let mut past = good;
    let file = u32::from_le_bytes(past[16..20].try_into().unwrap()) as usize;
    past[file..file + 4].copy_from_slice(&(10_000_000u32).to_le_bytes());
    let cab = Cabinet::parse(&past).unwrap();
    assert!(cab.extract(&cab.files[0]).is_err());
}

#[test]
fn hostile_lzx_input_fails_cleanly() {
    let cab = Cabinet::parse(LZX21).unwrap();
    let start = cab.folders[0].data as usize;
    // Corrupt every byte of the first block's payload in turn (checksums
    // zeroed so the decoder sees the damage): never a panic.
    let mut bytes = LZX21.to_vec();
    bytes[start..start + 4].fill(0);
    for at in (start + 8..start + 400).step_by(7) {
        let mut damaged = bytes.clone();
        damaged[at] ^= 0xa5;
        let cab = Cabinet::parse(&damaged).unwrap();
        let _ = cab.extract(&cab.files[0]);
    }
    // Random data as an LZX frame.
    let mut decoder = lzx::Decoder::new(16);
    let mut out = Vec::new();
    let noise: Vec<u8> = (0..2000u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
        .collect();
    let _ = decoder.block(&noise, 32768, &mut out);
    assert!(decoder.block(&noise, 0, &mut out).is_err());
    let mut inflater = inflate::Inflater::new();
    let _ = inflater.inflate(&noise, 32768, &mut vec![]);
}

#[test]
fn checksum_matches_ms_cab() {
    // MS-CAB 2.4 folds a 1..3-byte tail in from its first byte as the high
    // byte; a whole word is XORed little-endian.
    assert_eq!(checksum(&[1, 2, 3, 4], 0), 0x0403_0201);
    assert_eq!(checksum(&[1, 2, 3, 4, 5], 0), 0x0403_0201 ^ 5);
    assert_eq!(checksum(&[1, 2, 3, 4, 5, 6], 0), 0x0403_0201 ^ 0x0506);
    assert_eq!(checksum(&[], 7), 7);
}

/// Interop check against Windows' expand.exe: writes stored and MSZIP
/// cabinets to `DOOR_CAB_OUT` (run with `--ignored`).
#[test]
#[ignore]
fn write_cabinets_for_expand() {
    let Some(dir) = std::env::var_os("DOOR_CAB_OUT") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let data: Vec<u8> = (0..70_000u32)
        .map(|i| (i % 253) as u8 ^ (i >> 9) as u8)
        .collect();
    for (name, compression) in [
        ("stored.cab", Compression::Stored),
        ("mszip.cab", Compression::MsZip),
    ] {
        let mut w = Writer::new(compression);
        w.add("data.bin", &data, 0x5a21, 0x6000, 0x20);
        w.add("note.txt", b"written by door-cab\r\n", 0x5a21, 0x6000, 0x20);
        std::fs::write(dir.join(name), w.finish()).unwrap();
    }
    std::fs::write(dir.join("data.bin.expected"), &data).unwrap();
}

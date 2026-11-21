//! P07 adds a journal partition to the unchanged P06 filesystem fixture.
use super::{HOME_FIRST, HOME_SECTORS, LOG_FIRST, LOG_SECTORS};
use crate::storage::audit::crc32;

fn put(bytes: &mut [u8], at: usize, value: u64, width: usize) {
    bytes[at..at + width].copy_from_slice(&value.to_le_bytes()[..width]);
}
fn header() -> [u8; 4096] {
    let mut h = [0; 4096];
    h[..8].copy_from_slice(b"DOORJNL1");
    for (at, value, width) in [
        (8, 1, 4),
        (12, 4096, 4),
        (80, HOME_FIRST, 8),
        (88, HOME_SECTORS, 8),
        (96, LOG_FIRST, 8),
        (104, LOG_SECTORS, 8),
        (112, 512, 4),
        (116, 4096, 4),
    ] {
        put(&mut h, at, value, width);
    }
    h[32..48].copy_from_slice(b"DoorP07DiskGUID0");
    h[48..64].copy_from_slice(b"DoorP07ExtGUID00");
    h[64..80].copy_from_slice(b"DoorP07LogGUID00");
    let checksum = crc32(&h);
    put(&mut h, 120, u64::from(checksum), 4);
    h
}
pub fn image(tiny: bool) -> Vec<u8> {
    let mut bytes = crate::storage::format::image();
    if tiny {
        let home =
            &mut bytes[HOME_FIRST as usize * 512..(HOME_FIRST + HOME_SECTORS) as usize * 512];
        put(home, 1024 + 4, 32, 4);
        put(home, 1024 + 12, 22, 4);
        put(home, 4096 + 12, 22, 2);
        home[8192 + 4..12288].fill(0xff);
    }
    bytes[1040..1056].copy_from_slice(b"DoorP07FatGUID00");
    bytes[1168..1184].copy_from_slice(b"DoorP07ExtGUID00");
    let e = &mut bytes[1280..1408];
    e[..16].copy_from_slice(b"DoorRedoJournal1");
    e[16..32].copy_from_slice(b"DoorP07LogGUID00");
    put(e, 32, LOG_FIRST, 8);
    put(e, 40, LOG_FIRST + LOG_SECTORS - 1, 8);
    for (i, c) in "Door redo log".encode_utf16().enumerate() {
        put(e, 56 + i * 2, u64::from(c), 2);
    }
    let array = bytes[1024..17408].to_vec();
    bytes[262111 * 512..262143 * 512].copy_from_slice(&array);
    for lba in [1, 262143] {
        let h = &mut bytes[lba * 512..(lba + 1) * 512];
        h[56..72].copy_from_slice(b"DoorP07DiskGUID0");
        put(h, 88, u64::from(crc32(&array)), 4);
        put(h, 16, 0, 4);
        let checksum = crc32(&h[..92]);
        put(h, 16, u64::from(checksum), 4);
    }
    for index in 0..2 {
        let at = LOG_FIRST as usize * 512 + index * 4096;
        bytes[at..at + 4096].copy_from_slice(&header());
    }
    bytes
}

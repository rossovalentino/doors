//! Original deterministic P06 fixture formatter. No host filesystem utilities are used.
use super::{DISK_BYTES, EXT2_START, FAT_START};

fn u16_at(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}
fn u32_at(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn u64_at(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn crc(bytes: &[u8]) -> u32 {
    let mut value = !0u32;
    for byte in bytes {
        value ^= u32::from(*byte);
        for _ in 0..8 {
            value = (value >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(value & 1));
        }
    }
    !value
}
fn gpt_header(disk: &mut [u8], lba: usize, other: usize, array: usize, array_crc: u32) {
    let h = &mut disk[lba * 512..(lba + 1) * 512];
    h[..8].copy_from_slice(b"EFI PART");
    u32_at(h, 8, 0x10000);
    u32_at(h, 12, 92);
    u64_at(h, 24, lba as u64);
    u64_at(h, 32, other as u64);
    u64_at(h, 40, 34);
    u64_at(h, 48, 262110);
    h[56..72].copy_from_slice(b"DoorP06DiskGUID0");
    u64_at(h, 72, array as u64);
    u32_at(h, 80, 128);
    u32_at(h, 84, 128);
    u32_at(h, 88, array_crc);
    let checksum = crc(&h[..92]);
    u32_at(h, 16, checksum);
}
fn partition(entry: &mut [u8], kind: [u8; 16], id: &[u8; 16], first: u64, last: u64, name: &str) {
    entry[..16].copy_from_slice(&kind);
    entry[16..32].copy_from_slice(id);
    u64_at(entry, 32, first);
    u64_at(entry, 40, last);
    for (index, character) in name.encode_utf16().enumerate() {
        u16_at(entry, 56 + index * 2, character);
    }
}
fn fat(disk: &mut [u8]) {
    let base = FAT_START * 512;
    let b = &mut disk[base..base + 512];
    b[..3].copy_from_slice(&[0xeb, 0x58, 0x90]);
    b[3..11].copy_from_slice(b"DOORP006");
    u16_at(b, 11, 512);
    b[13] = 1;
    u16_at(b, 14, 32);
    b[16] = 2;
    b[21] = 0xf8;
    u16_at(b, 24, 63);
    u16_at(b, 26, 255);
    u32_at(b, 28, FAT_START as u32);
    u32_at(b, 32, 131072);
    u32_at(b, 36, 1009);
    u32_at(b, 44, 2);
    u16_at(b, 48, 1);
    u16_at(b, 50, 6);
    b[64] = 0x80;
    b[66] = 0x29;
    u32_at(b, 67, 0xd006_fa32);
    b[71..82].copy_from_slice(b"DOOR DATA  ");
    b[82..90].copy_from_slice(b"FAT32   ");
    b[510..512].copy_from_slice(&[0x55, 0xaa]);
    let boot: Vec<_> = b.to_vec();
    disk[base + 6 * 512..base + 7 * 512].copy_from_slice(&boot);
    let info = &mut disk[base + 512..base + 1024];
    u32_at(info, 0, 0x4161_5252);
    u32_at(info, 484, 0x6141_7272);
    u32_at(info, 488, 129020);
    u32_at(info, 492, 4);
    u32_at(info, 508, 0xaa55_0000);
    let info = info.to_vec();
    disk[base + 7 * 512..base + 8 * 512].copy_from_slice(&info);
    for fat in [32, 32 + 1009] {
        let at = base + fat * 512;
        for (index, value) in [0x0fff_fff8, 0x0fff_ffff, 0x0fff_ffff, 0x0fff_ffff]
            .into_iter()
            .enumerate()
        {
            u32_at(disk, at + index * 4, value);
        }
    }
    let root = base + (32 + 2 * 1009) * 512;
    disk[root..root + 11].copy_from_slice(b"HELLO   TXT");
    disk[root + 11] = 0x20;
    u16_at(disk, root + 26, 3);
    u32_at(disk, root + 28, 20);
    disk[root + 512..root + 512 + 20].copy_from_slice(b"Door FAT32 exchange\n");
}
fn inode(bytes: &mut [u8], number: usize, mode: u16, size: u32, links: u16, block: u32) {
    let at = 4 * 4096 + (number - 1) * 128;
    u16_at(bytes, at, mode);
    u32_at(bytes, at + 4, size);
    u16_at(bytes, at + 26, links);
    u32_at(bytes, at + 28, 8);
    u32_at(bytes, at + 40, block);
}
fn directory(bytes: &mut [u8], at: usize, inode: u32, length: u16, kind: u8, name: &[u8]) {
    u32_at(bytes, at, inode);
    u16_at(bytes, at + 4, length);
    bytes[at + 6] = name.len() as u8;
    bytes[at + 7] = kind;
    bytes[at + 8..at + 8 + name.len()].copy_from_slice(name);
}
fn ext2(disk: &mut [u8]) {
    let fs = &mut disk[EXT2_START * 512..(EXT2_START + 65536) * 512];
    let s = &mut fs[1024..2048];
    for (offset, value) in [
        (0, 128),
        (4, 8192),
        (12, 8182),
        (16, 117),
        (24, 2),
        (28, 2),
        (32, 8192),
        (36, 8192),
        (40, 128),
        (76, 1),
        (84, 11),
        (96, 2),
    ] {
        u32_at(s, offset, value);
    }
    u16_at(s, 56, 0xef53);
    u16_at(s, 58, 1);
    u16_at(s, 60, 1);
    u16_at(s, 88, 128);
    s[104..120].copy_from_slice(b"DoorP06Ext2UUID0");
    s[120..128].copy_from_slice(b"DOOR-P06");
    for (offset, value) in [(0, 2), (4, 3), (8, 4)] {
        u32_at(fs, 4096 + offset, value);
    }
    u16_at(fs, 4096 + 12, 8182);
    u16_at(fs, 4096 + 14, 117);
    u16_at(fs, 4096 + 16, 1);
    // Bitmap padding is allocated, preventing allocation outside this single group.
    fs[2 * 4096..4 * 4096].fill(0xff);
    fs[2 * 4096..2 * 4096 + 1024].fill(0);
    fs[3 * 4096..3 * 4096 + 16].fill(0);
    for bit in 0..10 {
        fs[2 * 4096 + bit / 8] |= 1 << (bit % 8);
    }
    for bit in 0..11 {
        fs[3 * 4096 + bit / 8] |= 1 << (bit % 8);
    }
    inode(fs, 2, 0x41ed, 4096, 2, 8);
    inode(fs, 11, 0x81a4, 15, 1, 9);
    directory(fs, 8 * 4096, 2, 12, 2, b".");
    directory(fs, 8 * 4096 + 12, 2, 12, 2, b"..");
    directory(fs, 8 * 4096 + 24, 11, 4072, 1, b"seed.txt");
    fs[9 * 4096..9 * 4096 + 15].copy_from_slice(b"Door ext2 seed\n");
}
pub fn image() -> Vec<u8> {
    let mut disk = vec![0; DISK_BYTES];
    disk[446 + 1..446 + 4].copy_from_slice(&[0, 2, 0]);
    disk[450] = 0xee;
    disk[451..454].fill(0xff);
    u32_at(&mut disk, 454, 1);
    u32_at(&mut disk, 458, 262143);
    disk[510..512].copy_from_slice(&[0x55, 0xaa]);
    partition(
        &mut disk[1024..1152],
        [
            0xa2, 0xa0, 0xd0, 0xeb, 0xe5, 0xb9, 0x33, 0x44, 0x87, 0xc0, 0x68, 0xb6, 0xb7, 0x26,
            0x99, 0xc7,
        ],
        b"DoorP06FatGUID00",
        2048,
        133119,
        "Door exchange",
    );
    partition(
        &mut disk[1152..1280],
        [
            0xaf, 0x3d, 0xc6, 0x0f, 0x83, 0x84, 0x72, 0x47, 0x8e, 0x79, 0x3d, 0x69, 0xd8, 0x47,
            0x7d, 0xe4,
        ],
        b"DoorP06ExtGUID00",
        133120,
        198655,
        "Door native",
    );
    let array = disk[1024..1024 + 16384].to_vec();
    disk[262111 * 512..262143 * 512].copy_from_slice(&array);
    let checksum = crc(&array);
    gpt_header(&mut disk, 1, 262143, 2, checksum);
    gpt_header(&mut disk, 262143, 1, 262111, checksum);
    fat(&mut disk);
    ext2(&mut disk);
    disk
}

//! Independent byte-level fixture auditor; deliberately does not call the formatter
//! or the guest filesystem parsers. Counts and ownership are derived from disk bytes.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

fn word(bytes: &[u8], at: usize, count: usize) -> Result<u64, String> {
    let part = bytes
        .get(at..at.checked_add(count).ok_or("offset overflow")?)
        .ok_or("truncated field")?;
    Ok(part
        .iter()
        .enumerate()
        .fold(0, |v, (i, b)| v | (u64::from(*b) << (i * 8))))
}
fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}
pub fn crc32(bytes: &[u8]) -> u32 {
    let table: [u32; 256] = std::array::from_fn(|i| {
        (0..8).fold(i as u32, |v, _| {
            if v & 1 == 1 {
                (v >> 1) ^ 0xedb8_8320
            } else {
                v >> 1
            }
        })
    });
    !bytes.iter().fold(!0u32, |v, b| {
        (v >> 8) ^ table[((v ^ u32::from(*b)) & 255) as usize]
    })
}
pub fn payload() -> Vec<u8> {
    (0..6000)
        .map(|i| {
            if (3584..4608).contains(&i) {
                (((i - 3584) * 13 + 7) % 251) as u8
            } else {
                ((i * 37 + 11) % 251) as u8
            }
        })
        .collect()
}
pub fn payload_crc() -> String {
    format!("{:08x}", crc32(&payload()))
}

fn gpt(disk: &[u8]) -> Result<(), String> {
    require(disk.len() == 134217728, "disk must be exactly 128 MiB")?;
    require(
        disk[510..512] == [0x55, 0xaa]
            && disk[450] == 0xee
            && word(disk, 454, 4)? == 1
            && word(disk, 458, 4)? == 262143,
        "protective MBR invalid",
    )?;
    require(
        disk[462..510].iter().all(|b| *b == 0),
        "extra MBR partitions",
    )?;
    for (here, other, entries) in [(1, 262143, 2), (262143, 1, 262111)] {
        let h = &disk[here * 512..(here + 1) * 512];
        require(
            &h[..8] == b"EFI PART"
                && word(h, 8, 4)? == 0x10000
                && word(h, 12, 4)? == 92
                && word(h, 20, 4)? == 0,
            "GPT header shape",
        )?;
        require(
            word(h, 24, 8)? == here as u64
                && word(h, 32, 8)? == other as u64
                && word(h, 40, 8)? == 34
                && word(h, 48, 8)? == 262110,
            "GPT extent",
        )?;
        require(
            h[56..72] == disk[512 + 56..512 + 72] && h[56..72].iter().any(|b| *b != 0),
            "GPT disk identity",
        )?;
        require(
            word(h, 72, 8)? == entries as u64 && word(h, 80, 4)? == 128 && word(h, 84, 4)? == 128,
            "GPT array shape",
        )?;
        let mut header = h[..92].to_vec();
        header[16..20].fill(0);
        require(
            u64::from(crc32(&header)) == word(h, 16, 4)?,
            "GPT header CRC",
        )?;
        let array = &disk[entries * 512..entries * 512 + 16384];
        require(
            u64::from(crc32(array)) == word(h, 88, 4)? && array == &disk[1024..17408],
            "GPT array CRC/mirror",
        )?;
    }
    for (index, first, last, kind) in [
        (
            0,
            2048,
            133119,
            [
                0xa2, 0xa0, 0xd0, 0xeb, 0xe5, 0xb9, 0x33, 0x44, 0x87, 0xc0, 0x68, 0xb6, 0xb7, 0x26,
                0x99, 0xc7,
            ],
        ),
        (
            1,
            133120,
            198655,
            [
                0xaf, 0x3d, 0xc6, 0x0f, 0x83, 0x84, 0x72, 0x47, 0x8e, 0x79, 0x3d, 0x69, 0xd8, 0x47,
                0x7d, 0xe4,
            ],
        ),
    ] {
        let e = &disk[1024 + index * 128..1152 + index * 128];
        require(
            e[..16] == kind
                && e[16..32].iter().any(|b| *b != 0)
                && word(e, 32, 8)? == first
                && word(e, 40, 8)? == last
                && word(e, 48, 8)? == 0,
            "GPT partition",
        )?;
    }
    require(
        disk[1040..1056] != disk[1168..1184] && disk[1280..17408].iter().all(|b| *b == 0),
        "GPT unique IDs/unused entries",
    )
}
pub(crate) fn fat(disk: &[u8]) -> Result<Value, String> {
    let fs = &disk[2048 * 512..133120 * 512];
    for (offset, width, value) in [
        (11, 2, 512),
        (13, 1, 1),
        (14, 2, 32),
        (16, 1, 2),
        (17, 2, 0),
        (19, 2, 0),
        (22, 2, 0),
        (28, 4, 2048),
        (32, 4, 131072),
        (36, 4, 1009),
        (40, 2, 0),
        (42, 2, 0),
        (44, 4, 2),
        (48, 2, 1),
        (50, 2, 6),
    ] {
        require(word(fs, offset, width)? == value, "FAT32 BPB geometry")?;
    }
    require(
        fs[510..512] == [0x55, 0xaa]
            && fs[..512] == fs[3072..3584]
            && fs[512..1024] == fs[3584..4096],
        "FAT32 boot/FSInfo mirrors",
    )?;
    let info = &fs[512..1024];
    require(
        word(info, 0, 4)? == 0x41615252
            && word(info, 484, 4)? == 0x61417272
            && word(info, 508, 4)? == 0xaa550000
            && word(info, 488, 4)? == 129020
            && word(info, 492, 4)? == 4,
        "FAT32 free count",
    )?;
    let table = &fs[32 * 512..1041 * 512];
    require(table == &fs[1041 * 512..2050 * 512], "FAT32 FAT mirrors")?;
    let mut free = 0;
    for cluster in 0..129024 {
        let value = word(table, cluster * 4, 4)?;
        if cluster < 4 {
            require(
                value == if cluster == 0 { 0x0ffffff8 } else { 0x0fffffff },
                "FAT32 reserved/root/file chain",
            )?;
        } else {
            require(value == 0, "unexpected FAT32 allocation")?;
            free += 1;
        }
    }
    require(free == 129020, "FAT32 allocation count")?;
    let root = &fs[2050 * 512..2051 * 512];
    require(
        &root[..11] == b"HELLO   TXT"
            && root[11] == 0x20
            && word(root, 20, 2)? == 0
            && word(root, 26, 2)? == 3
            && word(root, 28, 4)? == 20
            && root[32] == 0,
        "FAT32 root fixture",
    )?;
    require(
        &fs[2051 * 512..2051 * 512 + 20] == b"Door FAT32 exchange\n",
        "FAT32 seed content",
    )?;
    Ok(json!({"clusters":129022,"free_clusters":free,"seed":"HELLO.TXT","bytes":20}))
}
fn ext2(disk: &[u8], persisted: bool) -> Result<Value, String> {
    let fs = &disk[133120 * 512..198656 * 512];
    let s = &fs[1024..2048];
    for (offset, width, value) in [
        (0, 4, 128),
        (4, 4, 8192),
        (20, 4, 0),
        (24, 4, 2),
        (28, 4, 2),
        (32, 4, 8192),
        (36, 4, 8192),
        (40, 4, 128),
        (56, 2, 0xef53),
        (58, 2, 1),
        (76, 4, 1),
        (84, 4, 11),
        (88, 2, 128),
        (92, 4, 0),
        (96, 4, 2),
        (100, 4, 0),
    ] {
        require(
            word(s, offset, width)? == value,
            "ext2 superblock geometry/features",
        )?;
    }
    let gd = &fs[4096..4128];
    require(
        word(gd, 0, 4)? == 2
            && word(gd, 4, 4)? == 3
            && word(gd, 8, 4)? == 4
            && word(gd, 16, 2)? == 1,
        "ext2 group layout",
    )?;
    let blocks = &fs[8192..12288];
    let inodes = &fs[12288..16384];
    let allocated = |map: &[u8], bit: usize| map[bit / 8] & (1 << (bit % 8)) != 0;
    let free_blocks = (0..8192).filter(|i| !allocated(blocks, *i)).count();
    let free_inodes = (0..128).filter(|i| !allocated(inodes, *i)).count();
    require(
        word(s, 12, 4)? == free_blocks as u64
            && word(gd, 12, 2)? == free_blocks as u64
            && word(s, 16, 4)? == free_inodes as u64
            && word(gd, 14, 2)? == free_inodes as u64,
        "ext2 bitmap/free counts disagree",
    )?;
    require(
        (8192..32768).all(|bit| allocated(blocks, bit))
            && (128..32768).all(|bit| allocated(inodes, bit)),
        "ext2 bitmap padding must be reserved",
    )?;
    require(
        (0..10).all(|bit| allocated(blocks, bit)) && (0..11).all(|bit| allocated(inodes, bit)),
        "ext2 required metadata/seed allocation",
    )?;
    let inode = |number: usize| -> Result<&[u8], String> {
        require(
            (1..=128).contains(&number) && allocated(inodes, number - 1),
            "ext2 unallocated inode reference",
        )?;
        Ok(&fs[16384 + (number - 1) * 128..16384 + number * 128])
    };
    let root = inode(2)?;
    require(
        word(root, 0, 2)? == 0x41ed
            && word(root, 4, 4)? == 4096
            && word(root, 26, 2)? == 2
            && word(root, 28, 4)? == 8
            && word(root, 40, 4)? == 8
            && root[44..100].iter().all(|v| *v == 0),
        "ext2 root inode",
    )?;
    let directory = &fs[32768..36864];
    let mut names = BTreeMap::new();
    let mut offset = 0;
    while offset < 4096 {
        let entry = &directory[offset..];
        let number = word(entry, 0, 4)? as usize;
        let length = word(entry, 4, 2)? as usize;
        let name_length = word(entry, 6, 1)? as usize;
        require(
            length >= 8
                && length.is_multiple_of(4)
                && offset + length <= 4096
                && name_length <= length - 8,
            "ext2 directory record extent",
        )?;
        if number != 0 {
            let name = std::str::from_utf8(&entry[8..8 + name_length])
                .map_err(|_| "ext2 filename encoding")?;
            require(
                !name.is_empty()
                    && name.is_ascii()
                    && !name.contains('/')
                    && names.insert(name, (number, entry[7])).is_none(),
                "ext2 invalid/duplicate name",
            )?;
        }
        offset += length;
    }
    require(
        names.remove(".") == Some((2, 2))
            && names.remove("..") == Some((2, 2))
            && names.remove("seed.txt") == Some((11, 1)),
        "ext2 required directory entries",
    )?;
    let mut used_blocks: BTreeSet<_> = (0..9).collect();
    let mut used_inodes: BTreeSet<_> = (1..11).collect();
    let mut read_file = |number: usize, expected: &[u8]| -> Result<Value, String> {
        require(used_inodes.insert(number), "ext2 inode aliases")?;
        let i = inode(number)?;
        require(
            word(i, 0, 2)? & 0xf000 == 0x8000
                && word(i, 4, 4)? == expected.len() as u64
                && word(i, 26, 2)? == 1
                && word(i, 28, 4)? == expected.len().div_ceil(4096) as u64 * 8
                && i[88..100].iter().all(|v| *v == 0),
            "ext2 regular inode size/links/indirection",
        )?;
        let mut actual = Vec::new();
        for index in 0..12 {
            let block = word(i, 40 + index * 4, 4)? as usize;
            if index < expected.len().div_ceil(4096) {
                require(
                    (9..8192).contains(&block)
                        && allocated(blocks, block)
                        && used_blocks.insert(block),
                    "ext2 duplicate/unallocated data block",
                )?;
                actual.extend_from_slice(&fs[block * 4096..(block + 1) * 4096]);
            } else {
                require(block == 0, "ext2 extra data pointer")?;
            }
        }
        actual.truncate(expected.len());
        require(actual == expected, "ext2 file bytes differ")?;
        Ok(
            json!({"inode":number,"bytes":actual.len(),"sha256":format!("{:x}",Sha256::digest(&actual)),"crc32":format!("{:08x}",crc32(&actual))}),
        )
    };
    read_file(11, b"Door ext2 seed\n")?;
    let persistence = if persisted {
        let (number, kind) = names.remove("persist.bin").ok_or("persist.bin missing")?;
        require(kind == 1, "persist.bin must be regular")?;
        read_file(number, &payload())?
    } else {
        Value::Null
    };
    require(
        names.is_empty(),
        "unexpected ext2 name, including pending.bin",
    )?;
    require(
        (0..8192).all(|b| allocated(blocks, b) == used_blocks.contains(&b))
            && (1..=128).all(|i| allocated(inodes, i - 1) == used_inodes.contains(&i)),
        "ext2 leaked/unaccounted allocation",
    )?;
    Ok(json!({"free_blocks":free_blocks,"free_inodes":free_inodes,"persistence":persistence}))
}
pub fn inspect(bytes: &[u8], persisted: bool) -> Result<Value, String> {
    gpt(bytes)?;
    let fat32 = fat(bytes)?;
    let ext2 = ext2(bytes, persisted)?;
    Ok(
        json!({"schema_version":1,"record_kind":"storage_audit","state":if persisted {"persisted"} else {"initial"},"bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(bytes)),"gpt":{"primary":true,"backup":true,"partitions":2},"fat32":fat32,"ext2":ext2}),
    )
}

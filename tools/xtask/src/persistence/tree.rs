//! Independent ext2 allocation-graph audit, used after replaying retained log bytes.
use super::{require, word};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub struct File {
    pub inode: u64,
    pub generation: u64,
    pub bytes: Vec<u8>,
    pub blocks: Vec<usize>,
}
pub struct Tree {
    pub files: BTreeMap<String, File>,
    pub free_blocks: u64,
    pub free_inodes: u64,
}
pub fn inspect(home: &[u8]) -> Result<Tree, String> {
    require(home.len() == 32 * 1024 * 1024, "ext2 home length")?;
    let sb = &home[1024..2048];
    for (at, width, value) in [
        (0, 4, 128),
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
            word(sb, at, width)? == value,
            "ext2 geometry/features/clean state",
        )?;
    }
    let block_count = word(sb, 4, 4)? as usize;
    require(matches!(block_count, 32 | 8192), "ext2 selected capacity")?;
    require(&sb[104..120] == b"DoorP06Ext2UUID0", "ext2 volume identity")?;
    let gd = &home[4096..4128];
    require(
        word(gd, 0, 4)? == 2
            && word(gd, 4, 4)? == 3
            && word(gd, 8, 4)? == 4
            && word(gd, 16, 2)? == 1,
        "ext2 group metadata",
    )?;
    if block_count == 32 {
        require(
            home[32 * 4096..].iter().all(|b| *b == 0),
            "tiny filesystem wrote beyond declared blocks",
        )?;
    }
    let blockmap = &home[8192..12288];
    let inodemap = &home[12288..16384];
    let bit = |map: &[u8], n: usize| map[n / 8] & (1 << (n % 8)) != 0;
    let free_blocks = (0..block_count).filter(|n| !bit(blockmap, *n)).count() as u64;
    let free_inodes = (0..128).filter(|n| !bit(inodemap, *n)).count() as u64;
    require(
        word(sb, 12, 4)? == free_blocks
            && word(gd, 12, 2)? == free_blocks
            && word(sb, 16, 4)? == free_inodes
            && word(gd, 14, 2)? == free_inodes,
        "ext2 free counts",
    )?;
    require(
        (block_count..32768).all(|n| bit(blockmap, n)) && (128..32768).all(|n| bit(inodemap, n)),
        "ext2 bitmap padding",
    )?;
    for number in 11..=128 {
        if !bit(inodemap, number - 1) {
            let unused = &home[16384 + (number - 1) * 128..16384 + number * 128];
            require(
                unused[..100].iter().all(|b| *b == 0) && unused[104..].iter().all(|b| *b == 0),
                "free inode contains more than generation tombstone",
            )?;
        }
    }
    let inode = |n: usize| -> Result<&[u8], String> {
        require(
            (1..=128).contains(&n) && bit(inodemap, n - 1),
            "ext2 inode allocation",
        )?;
        Ok(&home[16384 + (n - 1) * 128..16384 + n * 128])
    };
    for n in 1..=10 {
        let i = inode(n)?;
        if n != 2 {
            require(i.iter().all(|b| *b == 0), "ext2 reserved inode content")?;
        }
    }
    let root = inode(2)?;
    require(
        word(root, 0, 2)? == 0x41ed
            && word(root, 4, 4)? == 4096
            && word(root, 26, 2)? == 2
            && word(root, 28, 4)? == 8
            && word(root, 40, 4)? == 8
            && root[44..100].iter().all(|b| *b == 0),
        "ext2 root inode",
    )?;
    let dir = &home[32768..36864];
    let mut offset = 0;
    let mut names = BTreeMap::new();
    while offset < 4096 {
        let e = &dir[offset..];
        let n = word(e, 0, 4)? as usize;
        let len = word(e, 4, 2)? as usize;
        let size = word(e, 6, 1)? as usize;
        require(
            len >= 8 && len.is_multiple_of(4) && offset + len <= 4096 && size <= len - 8,
            "ext2 directory extent",
        )?;
        if n != 0 {
            let name =
                std::str::from_utf8(&e[8..8 + size]).map_err(|_| "ext2 filename encoding")?;
            require(
                !name.is_empty()
                    && name.len() <= 63
                    && name.is_ascii()
                    && !name.contains('/')
                    && names.insert(name, (n, e[7])).is_none(),
                "ext2 invalid/duplicate filename",
            )?;
        }
        offset += len;
    }
    require(
        names.remove(".") == Some((2, 2)) && names.remove("..") == Some((2, 2)),
        "ext2 root links",
    )?;
    let mut used_blocks: BTreeSet<usize> = (0..9).collect();
    let mut used_inodes: BTreeSet<usize> = (1..=10).collect();
    let mut files = BTreeMap::new();
    for (name, (n, kind)) in names {
        require(
            kind == 1 && used_inodes.insert(n),
            "ext2 file inode alias/type",
        )?;
        let i = inode(n)?;
        let size = word(i, 4, 4)? as usize;
        require(
            word(i, 0, 2)? & 0xf000 == 0x8000
                && size <= (12 + 1024) * 4096
                && word(i, 26, 2)? == 1
                && word(i, 28, 4)? == (size.div_ceil(4096) + usize::from(size > 49152)) as u64 * 8
                && i[92..100].iter().all(|b| *b == 0)
                && i[104..128].iter().all(|b| *b == 0),
            "ext2 file shape/indirection",
        )?;
        let mut content = Vec::new();
        let mut blocks = Vec::new();
        for index in 0..12 {
            let block = word(i, 40 + index * 4, 4)? as usize;
            if index < size.div_ceil(4096) {
                require(
                    (9..block_count).contains(&block)
                        && bit(blockmap, block)
                        && used_blocks.insert(block),
                    "ext2 block crosslink/allocation",
                )?;
                content.extend_from_slice(&home[block * 4096..(block + 1) * 4096]);
                blocks.push(block);
            } else {
                require(block == 0, "ext2 unused direct block")?;
            }
        }
        let indirect = word(i, 88, 4)? as usize;
        if size > 49152 {
            require(
                (9..block_count).contains(&indirect)
                    && bit(blockmap, indirect)
                    && used_blocks.insert(indirect),
                "ext2 indirect allocation/crosslink",
            )?;
            let pointers = &home[indirect * 4096..(indirect + 1) * 4096];
            for index in 0..1024 {
                let block = word(pointers, index * 4, 4)? as usize;
                if index < size.div_ceil(4096) - 12 {
                    require(
                        (9..block_count).contains(&block)
                            && bit(blockmap, block)
                            && used_blocks.insert(block),
                        "ext2 indirect data allocation/crosslink",
                    )?;
                    content.extend_from_slice(&home[block * 4096..(block + 1) * 4096]);
                    blocks.push(block);
                } else {
                    require(block == 0, "ext2 unused indirect pointer")?;
                }
            }
        } else {
            require(indirect == 0, "ext2 unused indirect block")?;
        }
        content.truncate(size);
        files.insert(
            name.to_owned(),
            File {
                inode: n as u64,
                generation: word(i, 100, 4)?,
                bytes: content,
                blocks,
            },
        );
    }
    require(
        (0..block_count).all(|b| bit(blockmap, b) == used_blocks.contains(&b))
            && (1..=128).all(|i| bit(inodemap, i - 1) == used_inodes.contains(&i)),
        "ext2 orphan/unaccounted allocations",
    )?;
    require(
        files
            .get("seed.txt")
            .is_some_and(|f| f.inode == 11 && f.bytes == b"Door ext2 seed\n"),
        "ext2 immutable seed",
    )?;
    Ok(Tree {
        files,
        free_blocks,
        free_inodes,
    })
}
impl Tree {
    pub fn summary(&self) -> Value {
        json!({"free_blocks":self.free_blocks,"free_inodes":self.free_inodes,"files":self.files.iter().map(|(name,file)|json!({"name":name,"inode":file.inode,"generation":file.generation,"bytes":file.bytes.len(),"blocks":file.blocks,"sha256":format!("{:x}",Sha256::digest(&file.bytes))})).collect::<Vec<_>>()})
    }
}

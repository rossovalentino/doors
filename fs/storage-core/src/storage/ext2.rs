//! Standard ext2 revision 1 with 4 KiB blocks and the FILETYPE feature, in
//! two shapes:
//!
//! * the original constrained volume (P06/P07): one block group, 128
//!   128-byte inodes, a one-block root directory and dense files of direct
//!   and single-indirect blocks, cross-checked whole at every mount;
//! * a volume at scale (P18 M0a): any number of 32,768-block groups (each
//!   with its descriptor, bitmaps and inode table; `sparse_super` backups
//!   written at format), inodes sized and counted at format, double- and
//!   triple-indirect blocks with `large_file` sizes, and a root directory
//!   of any number of blocks. Mount checks the superblock, the descriptors
//!   and every group's bitmap counts; the cross-check of every inode and
//!   block is an offline audit.
//!
//! Files stay dense (every block up to the size is allocated), as before.
use super::{
    BLOCK_BYTES, BlockDevice, Error, FileId, Metadata, Result, Vfs, cache::Cache, name, put16,
    put32, u16le, u32le,
};
const LEGACY_INODES: u32 = 128;
const ROOT: u32 = 2;
const FIRST_FILE: u32 = 11;
/// Pointers per mapping block.
const PER: u64 = (BLOCK_BYTES / 4) as u64;
const LEGACY_MAX_FILE: u64 = (12 + PER) * BLOCK_BYTES as u64;
const MAX_BLOCKS: u64 = 12 + PER + PER * PER + PER * PER * PER;
/// ext2's own limit with 4 KiB blocks: 2^32 - 1 sectors of i_blocks.
const MAX_FILE: u64 = 0x1FF_FFFF_F000 * 2;
const BLOCKS_PER_GROUP: u32 = 32_768;
const FEATURE_FILETYPE: u32 = 0x2;
const RO_SPARSE_SUPER: u32 = 0x1;
const RO_LARGE_FILE: u32 = 0x2;
/// Blocks allocated per bitmap pass.
const RUN: usize = 64;

pub struct Ext2<D> {
    cache: Cache<D>,
    volume: [u8; 16],
    blocks: u32,
    free_blocks: u32,
    free_inodes: u32,
    dirty: bool,
    legacy: bool,
    groups: u32,
    blocks_per_group: u32,
    inodes_per_group: u32,
    inode_size: u32,
    /// The group block allocation starts at.
    hint: u32,
}
#[derive(Clone, Copy)]
struct Dir {
    inode: u32,
    record: usize,
    bytes: usize,
    kind: u8,
}
/// A group descriptor: bitmaps, inode table, free counts, directories.
#[derive(Clone, Copy)]
struct Group {
    block_bitmap: u32,
    inode_bitmap: u32,
    inode_table: u32,
    free_blocks: u32,
    free_inodes: u32,
}
fn dir(bytes: &[u8; BLOCK_BYTES], p: usize, inodes: u32) -> Result<Dir> {
    if p + 8 > BLOCK_BYTES {
        return Err(Error::Corrupt);
    }
    let d = Dir {
        inode: u32le(bytes, p),
        record: usize::from(u16le(bytes, p + 4)),
        bytes: usize::from(bytes[p + 6]),
        kind: bytes[p + 7],
    };
    if d.record < 8
        || !d.record.is_multiple_of(4)
        || p + d.record > BLOCK_BYTES
        || d.bytes > d.record - 8
        || d.inode > inodes
    {
        return Err(Error::Corrupt);
    }
    Ok(d)
}
fn bit(bytes: &[u8], index: u32) -> bool {
    bytes[index as usize / 8] & (1 << (index % 8)) != 0
}
fn set(bytes: &mut [u8], index: u32) {
    bytes[index as usize / 8] |= 1 << (index % 8);
}
fn clear(bytes: &mut [u8], index: u32) {
    bytes[index as usize / 8] &= !(1 << (index % 8));
}
fn records(bytes: &[u8; BLOCK_BYTES], target: &str, inodes: u32) -> Result<Option<(usize, Dir)>> {
    let mut p = 0;
    while p < BLOCK_BYTES {
        let d = dir(bytes, p, inodes)?;
        if d.inode != 0 && &bytes[p + 8..p + 8 + d.bytes] == target.as_bytes() {
            return Ok(Some((p, d)));
        }
        p += d.record;
    }
    Ok(None)
}
/// Whether `entry` fits in the block (without changing it).
fn fits(bytes: &[u8; BLOCK_BYTES], entry: &str, inodes: u32) -> Result<bool> {
    let need = (8 + entry.len() + 3) & !3;
    let mut p = 0;
    while p < BLOCK_BYTES {
        let d = dir(bytes, p, inodes)?;
        let used = if d.inode == 0 {
            0
        } else {
            (8 + d.bytes + 3) & !3
        };
        if d.record - used >= need {
            return Ok(true);
        }
        p += d.record;
    }
    Ok(false)
}
fn insert(bytes: &mut [u8; BLOCK_BYTES], entry: &str, inode: u32, inodes: u32) -> Result<()> {
    let need = (8 + entry.len() + 3) & !3;
    let mut p = 0;
    while p < BLOCK_BYTES {
        let d = dir(bytes, p, inodes)?;
        let used = if d.inode == 0 {
            0
        } else {
            (8 + d.bytes + 3) & !3
        };
        if d.record - used >= need {
            let start = p + used;
            let available = d.record - used;
            if used != 0 {
                put16(bytes, p + 4, used as u16);
            }
            bytes[start..start + available].fill(0);
            put32(bytes, start, inode);
            put16(bytes, start + 4, available as u16);
            bytes[start + 6] = entry.len() as u8;
            bytes[start + 7] = 1;
            bytes[start + 8..start + 8 + entry.len()].copy_from_slice(entry.as_bytes());
            return Ok(());
        }
        p += d.record;
    }
    Err(Error::NoSpace)
}
/// Mapping blocks a dense file of `n` data blocks needs.
fn meta_blocks(n: u64) -> u64 {
    let mut m = 0;
    let mut left = n.saturating_sub(12);
    if left == 0 {
        return 0;
    }
    // Single indirect.
    m += 1;
    left = left.saturating_sub(PER);
    if left == 0 {
        return m;
    }
    // Double: the top block and one per 1024 data blocks.
    let double = left.min(PER * PER);
    m += 1 + double.div_ceil(PER);
    left -= double;
    if left == 0 {
        return m;
    }
    // Triple: the top, one per 1024^2 and one per 1024 data blocks.
    m + 1 + left.div_ceil(PER * PER) + left.div_ceil(PER)
}
/// Whether group `g` keeps a superblock backup (sparse_super: 0, 1 and
/// powers of 3, 5 and 7).
pub fn has_backup(g: u32) -> bool {
    if g <= 1 {
        return true;
    }
    [3u32, 5, 7].iter().any(|&base| {
        let mut p = base;
        while p < g {
            p = match p.checked_mul(base) {
                Some(v) => v,
                None => return false,
            };
        }
        p == g
    })
}
fn inode_size_of(inode: &[u8; 128]) -> u64 {
    u64::from(u32le(inode, 4)) | (u64::from(u32le(inode, 108)) << 32)
}

impl<D: BlockDevice> Ext2<D> {
    pub fn mount(device: D) -> Result<Self> {
        let g = device.geometry();
        if g.sector_size != 512 || !g.flush_supported {
            return Err(Error::Unsupported);
        }
        if g.read_only {
            return Err(Error::ReadOnly);
        }
        let mut cache = Cache::new(device);
        let bytes = cache.block(0)?;
        let s = &bytes[1024..2048];
        let blocks = u32le(s, 4);
        let common = u16le(s, 56) == 0xef53
            && u64::from(blocks) * 8 <= g.sectors
            && u32le(s, 20) == 0
            && u32le(s, 24) == 2
            && u32le(s, 28) == 2
            && u32le(s, 76) == 1
            && u32le(s, 84) == FIRST_FILE
            && u16le(s, 90) == 0
            && u32le(s, 92) == 0
            && u32le(s, 96) == FEATURE_FILETYPE;
        let legacy = common
            && u32le(s, 0) == LEGACY_INODES
            && (16..=8192).contains(&blocks)
            && u32le(s, 32) == 8192
            && u32le(s, 36) == 8192
            && u32le(s, 40) == LEGACY_INODES
            && u16le(s, 88) == 128
            && u32le(s, 100) == 0;
        let inode_size = u32::from(u16le(s, 88));
        let per_group = u32le(s, 40);
        let groups = blocks.div_ceil(BLOCKS_PER_GROUP);
        let scaled = common
            && !legacy
            && blocks >= 64
            && matches!(inode_size, 128 | 256)
            && u32le(s, 32) == BLOCKS_PER_GROUP
            && u32le(s, 36) == BLOCKS_PER_GROUP
            && per_group > 0
            && per_group <= BLOCKS_PER_GROUP
            && per_group.is_multiple_of(BLOCK_BYTES as u32 / inode_size)
            && u64::from(u32le(s, 0)) == u64::from(per_group) * u64::from(groups)
            && u32le(s, 100) & !(RO_SPARSE_SUPER | RO_LARGE_FILE) == 0;
        if !legacy && !scaled {
            return Err(Error::Unsupported);
        }
        if u16le(s, 58) != 1 {
            return Err(Error::Corrupt);
        }
        let volume: [u8; 16] = s[104..120].try_into().unwrap();
        if volume == [0; 16] {
            return Err(Error::Corrupt);
        }
        let free_blocks = u32le(s, 12);
        let free_inodes = u32le(s, 16);
        let mut fs = Self {
            cache,
            volume,
            blocks,
            free_blocks,
            free_inodes,
            dirty: false,
            legacy,
            groups: if legacy { 1 } else { groups },
            blocks_per_group: if legacy { 8192 } else { BLOCKS_PER_GROUP },
            inodes_per_group: if legacy { LEGACY_INODES } else { per_group },
            inode_size: if legacy { 128 } else { inode_size },
            hint: 0,
        };
        if legacy {
            fs.validate_disk()?;
        } else {
            fs.validate_groups()?;
        }
        Ok(fs)
    }
    pub fn into_inner(mut self) -> Result<D> {
        self.flush()?;
        Ok(self.cache.device)
    }
    pub fn is_poisoned(&self) -> bool {
        self.cache.poisoned
    }
    /// Transaction controllers may commit only after this filesystem has staged
    /// its clean superblock. The callback must preserve the exposed block view.
    pub(crate) fn with_flushed_device<T>(
        &mut self,
        action: impl FnOnce(&mut D) -> Result<T>,
    ) -> Result<T> {
        self.flush()?;
        let result = action(&mut self.cache.device);
        self.failed(result)
    }
    fn inodes(&self) -> u32 {
        self.inodes_per_group * self.groups
    }
    fn max_file(&self) -> u64 {
        if self.legacy {
            LEGACY_MAX_FILE
        } else {
            MAX_FILE.min(MAX_BLOCKS * BLOCK_BYTES as u64)
        }
    }
    // ---- group descriptors ---------------------------------------------------------------
    fn group(&mut self, g: u32) -> Result<Group> {
        let at = g as usize * 32;
        let bytes = self.cache.block(1 + (at / BLOCK_BYTES) as u32)?;
        let d = &bytes[at % BLOCK_BYTES..at % BLOCK_BYTES + 32];
        Ok(Group {
            block_bitmap: u32le(d, 0),
            inode_bitmap: u32le(d, 4),
            inode_table: u32le(d, 8),
            free_blocks: u32::from(u16le(d, 12)),
            free_inodes: u32::from(u16le(d, 14)),
        })
    }
    /// Adds `blocks` and `inodes` (signed) to group `g`'s free counts.
    fn adjust_group(&mut self, g: u32, blocks: i32, inodes: i32) -> Result<()> {
        let at = g as usize * 32;
        let index = 1 + (at / BLOCK_BYTES) as u32;
        let mut b = [0; BLOCK_BYTES];
        self.cache.read(index, &mut b)?;
        let p = at % BLOCK_BYTES;
        let free_blocks = i64::from(u16le(&b, p + 12)) + i64::from(blocks);
        let free_inodes = i64::from(u16le(&b, p + 14)) + i64::from(inodes);
        if !(0..=i64::from(u16::MAX)).contains(&free_blocks)
            || !(0..=i64::from(u16::MAX)).contains(&free_inodes)
        {
            return Err(Error::Corrupt);
        }
        put16(&mut b, p + 12, free_blocks as u16);
        put16(&mut b, p + 14, free_inodes as u16);
        self.cache.write(index, &b)
    }
    /// Blocks of group `g`: [first, end).
    fn group_range(&self, g: u32) -> (u32, u32) {
        let first = g * self.blocks_per_group;
        (first, (first + self.blocks_per_group).min(self.blocks))
    }
    // ---- validation ----------------------------------------------------------------------
    fn validate_groups(&mut self) -> Result<()> {
        let table_blocks = self.inodes_per_group * self.inode_size / BLOCK_BYTES as u32;
        let mut free_blocks = 0u64;
        let mut free_inodes = 0u64;
        let mut bitmap = [0u8; BLOCK_BYTES];
        for g in 0..self.groups {
            let d = self.group(g)?;
            let (first, end) = self.group_range(g);
            let inside = |b: u32| b >= first && b < end;
            if !inside(d.block_bitmap)
                || !inside(d.inode_bitmap)
                || !inside(d.inode_table)
                || d.inode_table + table_blocks > end
            {
                return Err(Error::Corrupt);
            }
            self.cache.read(d.block_bitmap, &mut bitmap)?;
            let span = end - first;
            let free = (0..span).filter(|&i| !bit(&bitmap, i)).count() as u32;
            if free != d.free_blocks
                || !bit(&bitmap, d.block_bitmap - first)
                || !bit(&bitmap, d.inode_bitmap - first)
                || (0..table_blocks).any(|i| !bit(&bitmap, d.inode_table - first + i))
            {
                return Err(Error::Corrupt);
            }
            self.cache.read(d.inode_bitmap, &mut bitmap)?;
            let free = (0..self.inodes_per_group)
                .filter(|&i| !bit(&bitmap, i))
                .count() as u32;
            if free != d.free_inodes || (g == 0 && (0..FIRST_FILE - 1).any(|i| !bit(&bitmap, i))) {
                return Err(Error::Corrupt);
            }
            free_blocks += u64::from(d.free_blocks);
            free_inodes += u64::from(d.free_inodes);
        }
        if free_blocks != u64::from(self.free_blocks) || free_inodes != u64::from(self.free_inodes)
        {
            return Err(Error::Corrupt);
        }
        let root = self.inode(ROOT)?;
        let size = inode_size_of(&root);
        if u16le(&root, 0) & 0xf000 != 0x4000
            || u16le(&root, 26) < 2
            || size == 0
            || !size.is_multiple_of(BLOCK_BYTES as u64)
        {
            return Err(Error::Corrupt);
        }
        Ok(())
    }
    /// The original volume's whole cross-check (one group, 128 inodes).
    fn validate_disk(&mut self) -> Result<()> {
        let group = self.cache.block(1)?;
        if u32le(group, 0) != 2
            || u32le(group, 4) != 3
            || u32le(group, 8) != 4
            || u16le(group, 16) != 1
            || u32::from(u16le(group, 12)) != self.free_blocks
            || u32::from(u16le(group, 14)) != self.free_inodes
            || group[18..32].iter().any(|b| *b != 0)
        {
            return Err(Error::Corrupt);
        }
        // Only the declared 8192 block bits and 128 inode bits are needed across
        // cache replacement; retain their exact bounds instead of whole pages.
        let mut blocks = [0; 1024];
        let mut inodes = [0; 16];
        blocks.copy_from_slice(&self.cache.block(2)?[..1024]);
        inodes.copy_from_slice(&self.cache.block(3)?[..16]);
        if (0..self.blocks).filter(|&i| !bit(&blocks, i)).count() as u32 != self.free_blocks
            || (0..LEGACY_INODES).filter(|&i| !bit(&inodes, i)).count() as u32 != self.free_inodes
            || (0..8).any(|i| !bit(&blocks, i))
            || (0..10).any(|i| !bit(&inodes, i))
        {
            return Err(Error::Corrupt);
        }
        let mut seen = [0u8; 1024];
        for i in 0..8 {
            set(&mut seen, i);
        }
        for number in 1..=LEGACY_INODES {
            if !bit(&inodes, number - 1) {
                continue;
            }
            let inode = self.inode(number)?;
            if number < FIRST_FILE && number != ROOT {
                if inode.iter().any(|b| *b != 0) {
                    return Err(Error::Unsupported);
                }
                continue;
            }
            let count = self.check_inode(&inode, number == ROOT)?;
            if number == ROOT && u32le(&inode, 40) != 8 {
                return Err(Error::Unsupported);
            }
            if count > 12 {
                let b = u32le(&inode, 88);
                if !bit(&blocks, b) || bit(&seen, b) {
                    return Err(Error::Corrupt);
                }
                set(&mut seen, b);
            }
            for i in 0..count {
                let b = self.physical(&inode, i as u64)?;
                if !bit(&blocks, b) || bit(&seen, b) {
                    return Err(Error::Corrupt);
                }
                set(&mut seen, b);
            }
        }
        for i in 0..self.blocks {
            if bit(&blocks, i) != bit(&seen, i) {
                return Err(Error::Corrupt);
            }
        }
        let root = self.cache.block(8)?;
        let mut refs = [0u8; 128];
        let mut names = [0u16; 128];
        let mut count = 0;
        let mut p = 0;
        while p < BLOCK_BYTES {
            let d = dir(root, p, LEGACY_INODES)?;
            if d.inode != 0 {
                if count == names.len() || !bit(&inodes, d.inode - 1) || d.bytes == 0 {
                    return Err(Error::Corrupt);
                }
                let n = core::str::from_utf8(&root[p + 8..p + 8 + d.bytes])
                    .map_err(|_| Error::Unsupported)?;
                for &old in &names[..count] {
                    let old = usize::from(old);
                    let prior = dir(root, old, LEGACY_INODES)?;
                    if root[old + 8..old + 8 + prior.bytes] == root[p + 8..p + 8 + d.bytes] {
                        return Err(Error::Corrupt);
                    }
                }
                names[count] = p as u16;
                count += 1;
                if matches!(n, "." | "..") {
                    if d.inode != ROOT || d.kind != 2 {
                        return Err(Error::Corrupt);
                    }
                } else {
                    name(n)?;
                    if d.inode < FIRST_FILE || d.kind != 1 {
                        return Err(Error::Unsupported);
                    }
                }
                refs[d.inode as usize - 1] = refs[d.inode as usize - 1]
                    .checked_add(1)
                    .ok_or(Error::Corrupt)?;
            }
            p += d.record;
        }
        if refs[1] != 2
            || records(root, ".", LEGACY_INODES)?.is_none()
            || records(root, "..", LEGACY_INODES)?.is_none()
        {
            return Err(Error::Corrupt);
        }
        for n in FIRST_FILE..=LEGACY_INODES {
            if refs[n as usize - 1] != u8::from(bit(&inodes, n - 1)) {
                return Err(Error::Corrupt);
            }
        }
        Ok(())
    }
    // ---- inodes ----------------------------------------------------------------------------
    /// The block and byte offset of inode `number`.
    fn inode_at(&mut self, number: u32) -> Result<(u32, usize)> {
        if number == 0 || number > self.inodes() {
            return Err(Error::Corrupt);
        }
        let index = number - 1;
        let table = if self.legacy {
            4
        } else {
            self.group(index / self.inodes_per_group)?.inode_table
        };
        let within = (index % self.inodes_per_group) as usize * self.inode_size as usize;
        Ok((table + (within / BLOCK_BYTES) as u32, within % BLOCK_BYTES))
    }
    fn inode(&mut self, number: u32) -> Result<[u8; 128]> {
        let (block, at) = self.inode_at(number)?;
        let bytes = self.cache.block(block)?;
        Ok(bytes[at..at + 128].try_into().unwrap())
    }
    fn write_inode(&mut self, number: u32, inode: &[u8; 128]) -> Result<()> {
        let (block, at) = self.inode_at(number)?;
        let mut bytes = [0; BLOCK_BYTES];
        self.cache.read(block, &mut bytes)?;
        bytes[at..at + 128].copy_from_slice(inode);
        self.cache.write(block, &bytes)
    }
    /// Whether inode `number` is allocated (its group's bitmap).
    fn inode_used(&mut self, number: u32) -> Result<bool> {
        let index = number - 1;
        let bitmap = if self.legacy {
            3
        } else {
            self.group(index / self.inodes_per_group)?.inode_bitmap
        };
        Ok(bit(
            self.cache.block(bitmap)?,
            index % self.inodes_per_group,
        ))
    }
    // ---- block mapping -----------------------------------------------------------------------
    fn valid_block(&self, b: u32) -> bool {
        b >= 8 && b < self.blocks
    }
    /// The physical block of file block `index`.
    fn physical(&mut self, inode: &[u8; 128], index: u64) -> Result<u32> {
        let b = if index < 12 {
            u32le(inode, 40 + index as usize * 4)
        } else {
            let (top, depth, within) = Self::route(index)?;
            if self.legacy && depth > 1 {
                return Err(Error::Corrupt);
            }
            let mut b = u32le(inode, top);
            for level in (0..depth).rev() {
                if !self.valid_block(b) || (self.legacy && b < 9) {
                    return Err(Error::Corrupt);
                }
                let slot = (within / PER.pow(level)) % PER;
                b = u32le(self.cache.block(b)?, slot as usize * 4);
            }
            b
        };
        if !self.valid_block(b) {
            return Err(Error::Corrupt);
        }
        Ok(b)
    }
    /// A file block past the direct ones: the inode field of its top
    /// mapping block, the levels below it, and its index under that top.
    fn route(index: u64) -> Result<(usize, u32, u64)> {
        let i = index - 12;
        if i < PER {
            return Ok((88, 1, i));
        }
        let i = i - PER;
        if i < PER * PER {
            return Ok((92, 2, i));
        }
        let i = i - PER * PER;
        if i < PER * PER * PER {
            return Ok((96, 3, i));
        }
        Err(Error::Corrupt)
    }
    /// Maps file block `index` to a new block from `spare`, its new mapping
    /// blocks (zeroed here) taken first, as the original volume allocated
    /// them: the data block.
    fn map(&mut self, inode: &mut [u8; 128], index: u64, spare: &mut Spare) -> Result<u32> {
        if index < 12 {
            let b = spare.take(self)?;
            put32(inode, 40 + index as usize * 4, b);
            return Ok(b);
        }
        let (top, depth, within) = Self::route(index)?;
        let mut parent = u32le(inode, top);
        if parent == 0 {
            parent = spare.take(self)?;
            self.cache.write(parent, &[0; BLOCK_BYTES])?;
            put32(inode, top, parent);
        }
        let mut table = [0u8; BLOCK_BYTES];
        for level in (0..depth).rev() {
            let slot = ((within / PER.pow(level)) % PER) as usize * 4;
            self.cache.read(parent, &mut table)?;
            if level == 0 {
                let b = spare.take(self)?;
                put32(&mut table, slot, b);
                self.cache.write(parent, &table)?;
                return Ok(b);
            }
            let mut child = u32le(&table, slot);
            if child == 0 {
                child = spare.take(self)?;
                self.cache.write(child, &[0; BLOCK_BYTES])?;
                put32(&mut table, slot, child);
                self.cache.write(parent, &table)?;
            }
            parent = child;
        }
        Err(Error::Corrupt)
    }
    /// The data blocks of a file of `count` blocks (the original volume's
    /// inode check; volumes at scale check the shape only).
    fn check_inode(&mut self, inode: &[u8; 128], directory: bool) -> Result<usize> {
        let mode = u16le(inode, 0) & 0xf000;
        let size = if self.legacy {
            u64::from(u32le(inode, 4))
        } else {
            inode_size_of(inode)
        };
        if self.legacy {
            if mode != if directory { 0x4000 } else { 0x8000 }
                || u16le(inode, 26) != if directory { 2 } else { 1 }
                || size > LEGACY_MAX_FILE
                || u32le(inode, 20) != 0
                || u32le(inode, 32) != 0
                || u32le(inode, 36) != 0
                || inode[92..100].iter().any(|v| *v != 0)
                || inode[104..128].iter().any(|v| *v != 0)
                || (directory && size != BLOCK_BYTES as u64)
            {
                return Err(Error::Unsupported);
            }
        } else if mode != if directory { 0x4000 } else { 0x8000 }
            || (!directory && u16le(inode, 26) != 1)
            || size > self.max_file()
            || u32le(inode, 20) != 0
            || u32le(inode, 32) != 0
            || u32le(inode, 104) != 0
        {
            return Err(Error::Unsupported);
        }
        let count = size.div_ceil(BLOCK_BYTES as u64);
        if u64::from(u32le(inode, 28)) != (count + meta_blocks(count)) * 8 {
            return Err(Error::Corrupt);
        }
        if !self.legacy {
            return Ok(count as usize);
        }
        for i in 0..12 {
            let b = u32le(inode, 40 + i * 4);
            if ((i as u64) < count && (b < 8 || b >= self.blocks)) || (i as u64 >= count && b != 0)
            {
                return Err(Error::Corrupt);
            }
        }
        let indirect = u32le(inode, 88);
        if count > 12 {
            if indirect < 9 || indirect >= self.blocks {
                return Err(Error::Corrupt);
            }
            let mut table = [0; BLOCK_BYTES];
            self.cache.read(indirect, &mut table)?;
            for i in 0..BLOCK_BYTES / 4 {
                let b = u32le(&table, i * 4);
                if ((i as u64) < count - 12 && (b < 9 || b >= self.blocks))
                    || (i as u64 >= count - 12 && b != 0)
                {
                    return Err(Error::Corrupt);
                }
            }
        } else if indirect != 0 {
            return Err(Error::Corrupt);
        }
        Ok(count as usize)
    }
    fn identified(&mut self, id: FileId) -> Result<[u8; 128]> {
        if id.volume != self.volume || id.inode < FIRST_FILE || id.inode > self.inodes() {
            return Err(Error::NotFound);
        }
        if !self.inode_used(id.inode)? {
            return Err(Error::NotFound);
        }
        let inode = self.inode(id.inode)?;
        self.check_inode(&inode, false)?;
        if u32le(&inode, 100) != id.generation {
            return Err(Error::NotFound);
        }
        Ok(inode)
    }
    // ---- allocation ----------------------------------------------------------------------------
    /// Takes up to `out.len()` free blocks of one group (one bitmap pass).
    fn allocate(&mut self, out: &mut [u32]) -> Result<usize> {
        for step in 0..self.groups {
            let g = (self.hint + step) % self.groups;
            let d = if self.legacy {
                Group {
                    block_bitmap: 2,
                    inode_bitmap: 3,
                    inode_table: 4,
                    free_blocks: self.free_blocks,
                    free_inodes: self.free_inodes,
                }
            } else {
                self.group(g)?
            };
            if d.free_blocks == 0 {
                continue;
            }
            let (first, end) = self.group_range(g);
            let mut bitmap = [0u8; BLOCK_BYTES];
            self.cache.read(d.block_bitmap, &mut bitmap)?;
            let mut n = 0;
            // The original volume starts after its root directory block.
            let start = if self.legacy { 9 } else { 0 };
            for i in start..end - first {
                if n == out.len() {
                    break;
                }
                if !bit(&bitmap, i) {
                    set(&mut bitmap, i);
                    out[n] = first + i;
                    n += 1;
                }
            }
            if n == 0 {
                return Err(Error::Corrupt);
            }
            self.cache.write(d.block_bitmap, &bitmap)?;
            self.adjust_group(g, -(n as i32), 0)?;
            self.free_blocks = self
                .free_blocks
                .checked_sub(n as u32)
                .ok_or(Error::Corrupt)?;
            self.hint = g;
            return Ok(n);
        }
        Err(Error::NoSpace)
    }
    fn free_block(&mut self, b: u32) -> Result<()> {
        let g = b / self.blocks_per_group;
        let bitmap_block = if self.legacy {
            2
        } else {
            self.group(g)?.block_bitmap
        };
        let mut bitmap = [0u8; BLOCK_BYTES];
        self.cache.read(bitmap_block, &mut bitmap)?;
        let i = b - g * self.blocks_per_group;
        if !bit(&bitmap, i) {
            return Err(Error::Corrupt);
        }
        clear(&mut bitmap, i);
        self.cache.write(bitmap_block, &bitmap)?;
        self.adjust_group(g, 1, 0)?;
        self.free_blocks += 1;
        Ok(())
    }
    /// A free inode number (not yet marked).
    fn find_inode(&mut self) -> Result<u32> {
        for g in 0..self.groups {
            let d = if self.legacy {
                Group {
                    block_bitmap: 2,
                    inode_bitmap: 3,
                    inode_table: 4,
                    free_blocks: 0,
                    free_inodes: self.free_inodes,
                }
            } else {
                self.group(g)?
            };
            if d.free_inodes == 0 {
                continue;
            }
            let bitmap = self.cache.block(d.inode_bitmap)?;
            let start = if g == 0 { FIRST_FILE - 1 } else { 0 };
            if let Some(i) = (start..self.inodes_per_group).find(|&i| !bit(bitmap, i)) {
                return Ok(g * self.inodes_per_group + i + 1);
            }
        }
        Err(Error::NoSpace)
    }
    fn mark_inode(&mut self, number: u32, used: bool) -> Result<()> {
        let index = number - 1;
        let g = index / self.inodes_per_group;
        let bitmap_block = if self.legacy {
            3
        } else {
            self.group(g)?.inode_bitmap
        };
        let mut bitmap = [0u8; BLOCK_BYTES];
        self.cache.read(bitmap_block, &mut bitmap)?;
        let i = index % self.inodes_per_group;
        if used {
            set(&mut bitmap, i);
        } else {
            clear(&mut bitmap, i);
        }
        self.cache.write(bitmap_block, &bitmap)?;
        self.adjust_group(g, 0, if used { -1 } else { 1 })?;
        self.free_inodes = if used {
            self.free_inodes.checked_sub(1).ok_or(Error::Corrupt)?
        } else {
            self.free_inodes + 1
        };
        Ok(())
    }
    // ---- the root directory -------------------------------------------------------------------
    fn root_blocks(&mut self) -> Result<u64> {
        if self.legacy {
            return Ok(1);
        }
        let root = self.inode(ROOT)?;
        Ok(inode_size_of(&root) / BLOCK_BYTES as u64)
    }
    fn root_block(&mut self, index: u64) -> Result<u32> {
        if self.legacy {
            return Ok(8);
        }
        let root = self.inode(ROOT)?;
        self.physical(&root, index)
    }
    /// The root block holding `entry` and its record.
    fn lookup(&mut self, entry: &str) -> Result<Option<(u32, usize, Dir)>> {
        let inodes = self.inodes();
        for i in 0..self.root_blocks()? {
            let b = self.root_block(i)?;
            let mut bytes = [0; BLOCK_BYTES];
            self.cache.read(b, &mut bytes)?;
            if let Some((p, d)) = records(&bytes, entry, inodes)? {
                return Ok(Some((b, p, d)));
            }
        }
        Ok(None)
    }
    /// A root block with room for `entry` (None: the directory must grow).
    fn room(&mut self, entry: &str) -> Result<Option<u32>> {
        let inodes = self.inodes();
        for i in 0..self.root_blocks()? {
            let b = self.root_block(i)?;
            let mut bytes = [0; BLOCK_BYTES];
            self.cache.read(b, &mut bytes)?;
            if fits(&bytes, entry, inodes)? {
                return Ok(Some(b));
            }
        }
        Ok(None)
    }
    /// Places `entry` in the root directory (after start_write), growing
    /// it by a block on a volume at scale.
    fn place(&mut self, entry: &str, number: u32, room: Option<u32>) -> Result<()> {
        let inodes = self.inodes();
        let block = match room {
            Some(b) => b,
            None => {
                let mut root = self.inode(ROOT)?;
                let blocks = inode_size_of(&root) / BLOCK_BYTES as u64;
                let need = 1 + meta_blocks(blocks + 1) - meta_blocks(blocks);
                let mut spare = Spare::new();
                spare.owe(need as usize);
                let b = self.map(&mut root, blocks, &mut spare)?;
                let mut fresh = [0u8; BLOCK_BYTES];
                put16(&mut fresh, 4, BLOCK_BYTES as u16);
                self.cache.write(b, &fresh)?;
                let size = (blocks + 1) * BLOCK_BYTES as u64;
                put32(&mut root, 4, size as u32);
                put32(&mut root, 108, (size >> 32) as u32);
                put32(
                    &mut root,
                    28,
                    ((blocks + 1 + meta_blocks(blocks + 1)) * 8) as u32,
                );
                self.write_inode(ROOT, &root)?;
                b
            }
        };
        let mut bytes = [0; BLOCK_BYTES];
        self.cache.read(block, &mut bytes)?;
        insert(&mut bytes, entry, number, inodes)?;
        self.cache.write(block, &bytes)
    }
    // ---- superblock ------------------------------------------------------------------------------
    fn start_write(&mut self) -> Result<()> {
        if self.cache.poisoned {
            return Err(Error::Poisoned);
        }
        if self.dirty {
            return Ok(());
        }
        let mut block = [0; BLOCK_BYTES];
        self.cache.read(0, &mut block)?;
        put16(&mut block, 1024 + 58, 0);
        self.cache.write(0, &block)?;
        self.dirty = true;
        self.cache.flush()
    }
    /// Writes the superblock's free counts.
    fn counts(&mut self) -> Result<()> {
        let mut superblock = [0; BLOCK_BYTES];
        self.cache.read(0, &mut superblock)?;
        put32(&mut superblock, 1024 + 12, self.free_blocks);
        put32(&mut superblock, 1024 + 16, self.free_inodes);
        self.cache.write(0, &superblock)
    }
    fn failed<T>(&mut self, result: Result<T>) -> Result<T> {
        if result.is_err() {
            self.cache.poisoned = true;
            self.cache.invalidate();
        }
        result
    }
    // ---- truncation ------------------------------------------------------------------------------
    /// Frees the blocks of a mapping subtree at `block` (`level` 1: its
    /// entries are data blocks) that cover file blocks at or past `keep`
    /// (`start` is the first file block it covers). Whether it is now empty.
    fn release(&mut self, block: u32, level: u32, start: u64, keep: u64) -> Result<bool> {
        let span = PER.pow(level - 1);
        let mut table = [0u8; BLOCK_BYTES];
        self.cache.read(block, &mut table)?;
        let mut changed = false;
        let mut empty = true;
        for slot in 0..PER {
            let child = u32le(&table, slot as usize * 4);
            if child == 0 {
                continue;
            }
            let first = start + slot * span;
            let whole = first >= keep;
            let gone = if level == 1 {
                if whole {
                    self.free_block(child)?;
                }
                whole
            } else if first + span > keep {
                let emptied = self.release(child, level - 1, first, keep)?;
                if emptied {
                    self.free_block(child)?;
                }
                emptied
            } else {
                false
            };
            if gone {
                put32(&mut table, slot as usize * 4, 0);
                changed = true;
            } else {
                empty = false;
            }
        }
        if changed && !empty {
            self.cache.write(block, &table)?;
        }
        Ok(empty)
    }
}

/// Blocks taken for one change, handed out as data and mapping blocks.
struct Spare {
    blocks: [u32; RUN],
    taken: usize,
    count: usize,
    /// Blocks still to take from the bitmaps.
    owed: usize,
}
impl Spare {
    fn new() -> Self {
        Self {
            blocks: [0; RUN],
            taken: 0,
            count: 0,
            owed: 0,
        }
    }
    /// `n` more blocks (taken from the bitmaps a run at a time).
    fn owe(&mut self, n: usize) {
        self.owed += n;
    }
    fn refill<D: BlockDevice>(&mut self, fs: &mut Ext2<D>) -> Result<()> {
        if self.taken < self.count || self.owed == 0 {
            return Ok(());
        }
        let want = self.owed.min(RUN);
        let n = fs.allocate(&mut self.blocks[..want])?;
        self.taken = 0;
        self.count = n;
        self.owed -= n;
        Ok(())
    }
    fn take<D: BlockDevice>(&mut self, fs: &mut Ext2<D>) -> Result<u32> {
        self.refill(fs)?;
        if self.taken == self.count {
            return Err(Error::Corrupt);
        }
        self.taken += 1;
        Ok(self.blocks[self.taken - 1])
    }
}

impl<D: BlockDevice> Vfs for Ext2<D> {
    fn mutation_uncertain(&self) -> bool {
        self.cache.poisoned
    }
    fn scaled(&self) -> bool {
        !self.legacy
    }
    fn usage(&self) -> Option<(u64, u64)> {
        let unit = BLOCK_BYTES as u64;
        Some((
            u64::from(self.blocks) * unit,
            u64::from(self.free_blocks) * unit,
        ))
    }
    fn open(&mut self, entry: &str) -> Result<FileId> {
        name(entry)?;
        let (_, _, d) = self.lookup(entry)?.ok_or(Error::NotFound)?;
        if d.kind != 1 {
            return Err(Error::IsDirectory);
        }
        let inode = self.inode(d.inode)?;
        self.check_inode(&inode, false)?;
        Ok(FileId {
            volume: self.volume,
            inode: d.inode,
            generation: u32le(&inode, 100),
        })
    }
    fn metadata(&mut self, id: FileId) -> Result<Metadata> {
        let inode = self.identified(id)?;
        Ok(Metadata {
            id,
            bytes: inode_size_of(&inode) & if self.legacy { 0xFFFF_FFFF } else { u64::MAX },
            directory: false,
            read_only: false,
        })
    }
    fn read(&mut self, id: FileId, offset: u64, out: &mut [u8]) -> Result<usize> {
        let inode = self.identified(id)?;
        let size = if self.legacy {
            u64::from(u32le(&inode, 4))
        } else {
            inode_size_of(&inode)
        };
        if offset >= size {
            return Ok(0);
        }
        let count = out.len().min((size - offset) as usize);
        let mut done = 0;
        let mut block = [0; BLOCK_BYTES];
        while done < count {
            let pos = offset + done as u64;
            let index = pos / BLOCK_BYTES as u64;
            let within = (pos % BLOCK_BYTES as u64) as usize;
            let n = (BLOCK_BYTES - within).min(count - done);
            let physical = self.physical(&inode, index)?;
            self.cache.read(physical, &mut block)?;
            out[done..done + n].copy_from_slice(&block[within..within + n]);
            done += n;
        }
        Ok(done)
    }
    fn create(&mut self, entry: &str) -> Result<FileId> {
        name(entry)?;
        if self.lookup(entry)?.is_some() {
            return Err(Error::AlreadyExists);
        }
        let number = self.find_inode()?;
        let room = self.room(entry)?;
        if room.is_none() {
            if self.legacy {
                return Err(Error::NoSpace);
            }
            let blocks = self.root_blocks()?;
            let need = 1 + meta_blocks(blocks + 1) - meta_blocks(blocks);
            if need > u64::from(self.free_blocks) {
                return Err(Error::NoSpace);
            }
        }
        let old = self.inode(number)?;
        let generation = u32le(&old, 100).checked_add(1).ok_or(Error::NoSpace)?;
        let mut inode = [0; 128];
        put16(&mut inode, 0, 0x81a4);
        put16(&mut inode, 26, 1);
        put32(&mut inode, 100, generation);
        self.start_write()?;
        let result = (|| {
            self.write_inode(number, &inode)?;
            self.mark_inode(number, true)?;
            self.place(entry, number, room)?;
            self.counts()?;
            Ok(FileId {
                volume: self.volume,
                inode: number,
                generation,
            })
        })();
        self.failed(result)
    }
    fn write(&mut self, id: FileId, offset: u64, input: &[u8]) -> Result<usize> {
        let mut inode = self.identified(id)?;
        if input.is_empty() {
            return Ok(0);
        }
        let end = offset
            .checked_add(input.len() as u64)
            .ok_or(Error::Overflow)?;
        if end > self.max_file() {
            return Err(Error::NoSpace);
        }
        let old_size = if self.legacy {
            u64::from(u32le(&inode, 4))
        } else {
            inode_size_of(&inode)
        };
        let old_count = old_size.div_ceil(BLOCK_BYTES as u64);
        let size = end.max(old_size);
        let count = size.div_ceil(BLOCK_BYTES as u64);
        let extra = (count - old_count) + (meta_blocks(count) - meta_blocks(old_count));
        if extra > u64::from(self.free_blocks) {
            return Err(Error::NoSpace);
        }
        self.start_write()?;
        let result = (|| {
            let mut spare = Spare::new();
            spare.owe(extra as usize);
            // Blocks before the write that it extends over (zeros), the
            // blocks it writes, and new blocks: first and last touched.
            let first = offset.min(old_size) / BLOCK_BYTES as u64;
            for index in first..count {
                let start = index * BLOCK_BYTES as u64;
                let stop = start + BLOCK_BYTES as u64;
                let touches_input = stop > offset && start < end;
                let gap_start = start.max(old_size);
                let gap_end = stop.min(offset);
                if index < old_count && !touches_input && gap_start >= gap_end {
                    continue;
                }
                let mut block = [0u8; BLOCK_BYTES];
                let physical = if index < old_count {
                    let p = self.physical(&inode, index)?;
                    self.cache.read(p, &mut block)?;
                    p
                } else {
                    self.map(&mut inode, index, &mut spare)?
                };
                // A valid imported ext2 file may contain arbitrary bytes beyond
                // EOF in its last allocated block. Never expose those bytes
                // when a later write extends the file across a gap.
                if gap_start < gap_end {
                    block[(gap_start - start) as usize..(gap_end - start) as usize].fill(0);
                }
                let from = start.max(offset);
                let to = stop.min(end);
                if from < to {
                    let source = (from - offset) as usize;
                    block[(from - start) as usize..(to - start) as usize]
                        .copy_from_slice(&input[source..source + (to - from) as usize]);
                }
                self.cache.write(physical, &block)?;
            }
            if count != old_count {
                self.counts()?;
            }
            put32(&mut inode, 4, size as u32);
            if !self.legacy {
                put32(&mut inode, 108, (size >> 32) as u32);
            }
            put32(&mut inode, 28, ((count + meta_blocks(count)) * 8) as u32);
            self.write_inode(id.inode, &inode)?;
            Ok(input.len())
        })();
        self.failed(result)
    }
    fn rename(&mut self, old: &str, new: &str) -> Result<()> {
        name(old)?;
        name(new)?;
        let (block, position, entry) = self.lookup(old)?.ok_or(Error::NotFound)?;
        if old == new {
            return Ok(());
        }
        if self.lookup(new)?.is_some() {
            return Err(Error::AlreadyExists);
        }
        let inodes = self.inodes();
        let mut bytes = [0; BLOCK_BYTES];
        self.cache.read(block, &mut bytes)?;
        put32(&mut bytes, position, 0);
        // The freed record first: the new name fits where the old one was.
        let room = if fits(&bytes, new, inodes)? {
            Some(block)
        } else {
            let room = self.room(new)?;
            if room.is_none() {
                if self.legacy {
                    return Err(Error::NoSpace);
                }
                let blocks = self.root_blocks()?;
                if 1 + meta_blocks(blocks + 1) - meta_blocks(blocks) > u64::from(self.free_blocks) {
                    return Err(Error::NoSpace);
                }
            }
            room
        };
        self.start_write()?;
        let result = (|| {
            self.cache.write(block, &bytes)?;
            self.place(new, entry.inode, room)?;
            self.counts()
        })();
        self.failed(result)
    }
    fn truncate(&mut self, id: FileId, bytes: u64) -> Result<()> {
        let mut inode = self.identified(id)?;
        let previous = if self.legacy {
            u64::from(u32le(&inode, 4))
        } else {
            inode_size_of(&inode)
        };
        if bytes > previous {
            self.write(id, bytes - 1, &[0])?;
            return Ok(());
        }
        if bytes == previous {
            return Ok(());
        }
        let old_count = previous.div_ceil(BLOCK_BYTES as u64);
        let count = bytes.div_ceil(BLOCK_BYTES as u64);
        // Zeroing the new last block's tail needs its block before the
        // mappings change.
        let last = if !bytes.is_multiple_of(BLOCK_BYTES as u64) {
            Some(self.physical(&inode, count - 1)?)
        } else {
            None
        };
        self.start_write()?;
        let result = (|| {
            if let Some(block) = last {
                let mut data = [0; BLOCK_BYTES];
                self.cache.read(block, &mut data)?;
                data[(bytes % BLOCK_BYTES as u64) as usize..].fill(0);
                self.cache.write(block, &data)?;
            }
            for i in count.min(12)..old_count.min(12) {
                self.free_block(u32le(&inode, 40 + i as usize * 4))?;
                put32(&mut inode, 40 + i as usize * 4, 0);
            }
            for (field, level, start) in [
                (88usize, 1u32, 12u64),
                (92, 2, 12 + PER),
                (96, 3, 12 + PER + PER * PER),
            ] {
                let top = u32le(&inode, field);
                if top == 0 || old_count <= start {
                    continue;
                }
                if self.release(top, level, start, count)? {
                    self.free_block(top)?;
                    put32(&mut inode, field, 0);
                }
            }
            self.counts()?;
            put32(&mut inode, 4, bytes as u32);
            if !self.legacy {
                put32(&mut inode, 108, (bytes >> 32) as u32);
            }
            put32(&mut inode, 28, ((count + meta_blocks(count)) * 8) as u32);
            self.write_inode(id.inode, &inode)
        })();
        self.failed(result)
    }
    fn unlink(&mut self, entry: &str, expected: FileId) -> Result<()> {
        if self.open(entry)? != expected {
            return Err(Error::NotFound);
        }
        let (block, position, _) = self.lookup(entry)?.ok_or(Error::NotFound)?;
        let mut root = [0; BLOCK_BYTES];
        self.cache.read(block, &mut root)?;
        put32(&mut root, position, 0);
        // Capture every fallible lookup before the first mutation. Truncation
        // preserves these directory/inode allocation records; after it succeeds
        // all remaining failures pass through the poison wrapper below.
        self.truncate(expected, 0)?;

        let mut inode = [0; 128];
        // Retain the incarnation even while unallocated. Reuse increments it.
        put32(&mut inode, 100, expected.generation);
        self.start_write()?;
        let result = (|| {
            self.write_inode(expected.inode, &inode)?;
            self.mark_inode(expected.inode, false)?;
            self.counts()?;
            self.cache.write(block, &root)
        })();
        self.failed(result)
    }
    fn flush(&mut self) -> Result<()> {
        self.cache.flush()?;
        if !self.dirty {
            return Ok(());
        }
        let mut block = [0; BLOCK_BYTES];
        if let Err(e) = self.cache.read(0, &mut block) {
            self.cache.poisoned = true;
            return Err(e);
        }
        put16(&mut block, 1024 + 58, 1);
        self.cache.write(0, &block)?;
        self.cache.flush()?;
        self.dirty = false;
        Ok(())
    }
}

/// A volume at scale's shape: its size in 4 KiB blocks, inodes per group
/// (a multiple of 32), and its identity.
#[derive(Clone, Copy, Debug)]
pub struct Format {
    pub blocks: u32,
    pub inodes_per_group: u32,
    pub volume: [u8; 16],
}
impl Format {
    /// One inode per 16 KiB of volume, as mke2fs's default ratio.
    pub fn sized(bytes: u64, volume: [u8; 16]) -> Self {
        let blocks = (bytes / BLOCK_BYTES as u64).min(u64::from(u32::MAX)) as u32;
        let groups = blocks.div_ceil(BLOCKS_PER_GROUP).max(1);
        let per_group = (blocks / groups / 4).max(32).div_ceil(32) * 32;
        Self {
            blocks,
            inodes_per_group: per_group.min(BLOCKS_PER_GROUP),
            volume,
        }
    }
}
/// Formats `device` as an empty volume at scale: superblock and
/// descriptor backups in the sparse groups, every group's bitmaps and
/// (zeroed) inode table, and a one-block root directory. `zeroed`: the
/// device already reads as zeros (a new sparse image), so zero blocks are
/// not written.
pub fn format<D: BlockDevice>(device: &mut D, shape: Format, zeroed: bool) -> Result<()> {
    let blocks = shape.blocks;
    if blocks < 64
        || u64::from(blocks) * 8 > device.geometry().sectors
        || shape.inodes_per_group == 0
        || !shape.inodes_per_group.is_multiple_of(32)
        || shape.inodes_per_group > BLOCKS_PER_GROUP
        || shape.volume == [0; 16]
    {
        return Err(Error::Unsupported);
    }
    let groups = blocks.div_ceil(BLOCKS_PER_GROUP);
    let gdt_blocks = (groups as usize * 32).div_ceil(BLOCK_BYTES) as u32;
    let table_blocks = shape.inodes_per_group * 128 / BLOCK_BYTES as u32;
    let mut block = [0u8; BLOCK_BYTES];
    let write = |device: &mut D, b: u32, bytes: &[u8; BLOCK_BYTES]| {
        device.write_sectors(u64::from(b) * 8, bytes)
    };
    // Each group's layout: backups, bitmaps, inode table (and in group 0
    // the root directory).
    let layout = |g: u32| {
        let first = g * BLOCKS_PER_GROUP;
        let end = (first + BLOCKS_PER_GROUP).min(blocks);
        let backup = if has_backup(g) { 1 + gdt_blocks } else { 0 };
        let block_bitmap = first + backup;
        (first, end, block_bitmap, block_bitmap + 1, block_bitmap + 2)
    };
    let mut free_blocks = 0u32;
    // Descriptors, written after the counts are known.
    let mut descriptors_left = groups;
    let mut gdt_index = 0;
    while descriptors_left > 0 {
        block.fill(0);
        let here = descriptors_left.min((BLOCK_BYTES / 32) as u32);
        for k in 0..here {
            let g = gdt_index * (BLOCK_BYTES / 32) as u32 + k;
            let (first, end, block_bitmap, inode_bitmap, table) = layout(g);
            let used = table + table_blocks - first + u32::from(g == 0);
            if used > end - first {
                return Err(Error::Unsupported);
            }
            let free = end - first - used;
            free_blocks += free;
            let free_inodes = shape.inodes_per_group - if g == 0 { FIRST_FILE - 1 } else { 0 };
            let d = &mut block[k as usize * 32..k as usize * 32 + 32];
            put32(d, 0, block_bitmap);
            put32(d, 4, inode_bitmap);
            put32(d, 8, table);
            put16(d, 12, free as u16);
            put16(d, 14, free_inodes as u16);
            put16(d, 16, u16::from(g == 0));
        }
        for g in (0..groups).filter(|&g| has_backup(g)) {
            write(device, g * BLOCKS_PER_GROUP + 1 + gdt_index, &block)?;
        }
        descriptors_left -= here;
        gdt_index += 1;
    }
    for g in 0..groups {
        let (first, end, block_bitmap, inode_bitmap, table) = layout(g);
        // Block bitmap: metadata (and group 0's root block) used, the
        // bits past a short last group set.
        block.fill(0);
        let used = table + table_blocks - first + u32::from(g == 0);
        for i in 0..used {
            set(&mut block, i);
        }
        for i in end - first..BLOCKS_PER_GROUP {
            set(&mut block, i);
        }
        write(device, block_bitmap, &block)?;
        block.fill(0);
        if g == 0 {
            for i in 0..FIRST_FILE - 1 {
                set(&mut block, i);
            }
        }
        // Bits past the group's inodes.
        for i in shape.inodes_per_group..(BLOCK_BYTES * 8) as u32 {
            set(&mut block, i);
        }
        write(device, inode_bitmap, &block)?;
        if !zeroed {
            block.fill(0);
            for b in table..table + table_blocks {
                write(device, b, &block)?;
            }
        }
    }
    // The root directory: "." and "..".
    let (_, _, _, _, table) = layout(0);
    let root_block = table + table_blocks;
    block.fill(0);
    put32(&mut block, 0, ROOT);
    put16(&mut block, 4, 12);
    block[6] = 1;
    block[7] = 2;
    block[8] = b'.';
    put32(&mut block, 12, ROOT);
    put16(&mut block, 16, (BLOCK_BYTES - 12) as u16);
    block[18] = 2;
    block[19] = 2;
    block[20..22].copy_from_slice(b"..");
    write(device, root_block, &block)?;
    // The root inode (number 2, the second record of group 0's table).
    block.fill(0);
    let inode = &mut block[128..256];
    put16(inode, 0, 0x41ed);
    put32(inode, 4, BLOCK_BYTES as u32);
    put16(inode, 26, 2);
    put32(inode, 28, 8);
    put32(inode, 40, root_block);
    write(device, table, &block)?;
    // Superblocks: the primary at byte 1024 of block 0, backups at the
    // start of their groups' first blocks.
    for g in (0..groups).filter(|&g| has_backup(g)) {
        block.fill(0);
        let at = if g == 0 { 1024 } else { 0 };
        let s = &mut block[at..at + 1024];
        for (p, v) in [
            (0, shape.inodes_per_group * groups),
            (4, blocks),
            (12, free_blocks),
            (16, shape.inodes_per_group * groups - (FIRST_FILE - 1)),
            (20, 0),
            (24, 2),
            (28, 2),
            (32, BLOCKS_PER_GROUP),
            (36, BLOCKS_PER_GROUP),
            (40, shape.inodes_per_group),
            (76, 1),
            (84, FIRST_FILE),
            (96, FEATURE_FILETYPE),
            (100, RO_SPARSE_SUPER | RO_LARGE_FILE),
        ] {
            put32(s, p, v);
        }
        put16(s, 56, 0xef53);
        put16(s, 58, 1);
        put16(s, 60, 1);
        put16(s, 88, 128);
        put16(s, 90, g as u16);
        s[104..120].copy_from_slice(&shape.volume);
        s[120..130].copy_from_slice(b"Doors home");
        write(device, g * BLOCKS_PER_GROUP, &block)?;
    }
    device.flush()
}

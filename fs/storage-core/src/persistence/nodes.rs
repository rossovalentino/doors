//! Catalog v6 (P18 M0a): the namespace's nodes in one B+tree file
//! (`door.nodes`), so the number of files and folders is bounded by the
//! volume, not by an array, and a change writes a few pages instead of the
//! whole catalog. Records, all keys big-endian:
//!
//! * `N id` -> the node (parent, kind, flags, name, backing, ACL, times);
//! * `C parent folded-name id` -> nothing: lookup by name (folded as
//!   `Name::same(.., true)` compares) and its exact spellings;
//! * `P parent id` -> nothing: a folder's children in object-ID order (the
//!   order listings have always used);
//! * `D id` -> nothing: a delete-pending node, reclaimed at mount.
use super::{
    Access, Ace, Acl, Error, Name, Principal, Result,
    namespace::{FileTimes, Node},
    path::fold_ascii,
};
use crate::storage::{
    FileId, Vfs,
    btree::{PAGE, Pages, Tree, Work},
};

pub const NODES_NAME: &str = "door.nodes";
/// The longest key: `C`, the parent, a 255-unit name, its terminator and
/// the id.
pub const KEY_BYTES: usize = 1 + 8 + 2 * super::path::NAME_UNITS + 2 + 8;
/// The longest record: the fixed fields (`ACES` bytes), four ACEs, a
/// 255-unit name.
pub const RECORD_BYTES: usize = ACES + 4 * 12 + 2 * super::path::NAME_UNITS;
const _: () = assert!(KEY_BYTES <= crate::storage::btree::MAX_KEY);
const _: () = assert!(RECORD_BYTES <= crate::storage::btree::MAX_VALUE);

/// The tree's pages: 4 KiB pages of an ext2 file.
pub struct FilePages<'f, F> {
    pub fs: &'f mut F,
    pub file: FileId,
}
impl<F: Vfs> Pages for FilePages<'_, F> {
    fn read(&mut self, page: u32, out: &mut [u8; PAGE]) -> crate::storage::Result<()> {
        let n = self
            .fs
            .read(self.file, u64::from(page) * PAGE as u64, out)?;
        if n != PAGE {
            return Err(crate::storage::Error::Corrupt);
        }
        Ok(())
    }
    fn write(&mut self, page: u32, bytes: &[u8; PAGE]) -> crate::storage::Result<()> {
        let n = self
            .fs
            .write(self.file, u64::from(page) * PAGE as u64, bytes)?;
        if n != PAGE {
            return Err(crate::storage::Error::Corrupt);
        }
        Ok(())
    }
}
pub fn open<'f, F: Vfs>(
    fs: &'f mut F,
    file: FileId,
    work: &mut Work,
) -> Result<Tree<FilePages<'f, F>>> {
    Ok(Tree::open(FilePages { fs, file }, work)?)
}
pub fn create<'f, F: Vfs>(
    fs: &'f mut F,
    file: FileId,
    work: &mut Work,
) -> Result<Tree<FilePages<'f, F>>> {
    Ok(Tree::create(FilePages { fs, file }, work)?)
}

pub fn node_key(id: u64) -> [u8; 9] {
    let mut k = [b'N'; 9];
    k[1..].copy_from_slice(&id.to_be_bytes());
    k
}
pub fn pending_key(id: u64) -> [u8; 9] {
    let mut k = [b'D'; 9];
    k[1..].copy_from_slice(&id.to_be_bytes());
    k
}
pub fn list_key(parent: u64, id: u64) -> [u8; 17] {
    let mut k = [b'P'; 17];
    k[1..9].copy_from_slice(&parent.to_be_bytes());
    k[9..].copy_from_slice(&id.to_be_bytes());
    k
}
/// `C parent folded-name 0000` (the prefix every spelling of the name
/// shares; units are never 0, so the terminator keeps longer names after
/// it) and, with an id, the whole key: its length in `out`.
pub fn child_key(parent: u64, name: &Name, id: Option<u64>, out: &mut [u8; KEY_BYTES]) -> usize {
    out[0] = b'C';
    out[1..9].copy_from_slice(&parent.to_be_bytes());
    let mut n = 9;
    for &u in name.units() {
        out[n..n + 2].copy_from_slice(&fold_ascii(u).to_be_bytes());
        n += 2;
    }
    out[n..n + 2].fill(0);
    n += 2;
    if let Some(id) = id {
        out[n..n + 8].copy_from_slice(&id.to_be_bytes());
        n += 8;
    }
    n
}

fn put16(b: &mut [u8], p: usize, v: u16) {
    b[p..p + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], p: usize, v: u32) {
    b[p..p + 4].copy_from_slice(&v.to_le_bytes());
}
fn put64(b: &mut [u8], p: usize, v: u64) {
    b[p..p + 8].copy_from_slice(&v.to_le_bytes());
}
fn get16(b: &[u8], p: usize) -> u16 {
    u16::from_le_bytes(b[p..p + 2].try_into().unwrap())
}
fn get32(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(b[p..p + 4].try_into().unwrap())
}
fn get64(b: &[u8], p: usize) -> u64 {
    u64::from_le_bytes(b[p..p + 8].try_into().unwrap())
}
/// A node record: the fields at these offsets, then 12 bytes per ACE, then
/// the name's units.
const PARENT: usize = 0;
const KIND: usize = 8;
const PENDING: usize = 9;
const NAME_LEN: usize = 10;
const ATTRIBUTES: usize = 12;
const VERSION: usize = 16;
const INODE: usize = 24;
const GENERATION: usize = 28;
const VOLUME: usize = 32;
const OWNER: usize = 48;
const INTEGRITY: usize = 52;
const COUNT: usize = 53;
const CREATED: usize = 54;
const MODIFIED: usize = 62;
const ACCESSED: usize = 70;
const ACES: usize = 78;
/// The record's length (the name's units follow the ACEs).
pub fn encode(n: &Node, out: &mut [u8; RECORD_BYTES + 16]) -> usize {
    out.fill(0);
    put64(out, PARENT, n.parent);
    out[KIND] = n.kind;
    out[PENDING] = u8::from(n.pending);
    put16(out, NAME_LEN, u16::from(n.name.len));
    put32(out, ATTRIBUTES, n.attributes);
    put64(out, VERSION, n.version);
    put32(out, INODE, n.backing.inode);
    put32(out, GENERATION, n.backing.generation);
    out[VOLUME..VOLUME + 16].copy_from_slice(&n.backing.volume);
    put32(out, OWNER, n.acl.owner);
    out[INTEGRITY] = n.acl.integrity;
    out[COUNT] = n.acl.count;
    put64(out, CREATED, n.times.created);
    put64(out, MODIFIED, n.times.modified);
    put64(out, ACCESSED, n.times.accessed);
    let count = usize::from(n.acl.count).min(4);
    for (j, a) in n.acl.entries[..count].iter().enumerate() {
        let p = ACES + j * 12;
        let (kind, id) = match a.principal {
            Principal::Everyone => (0, 0),
            Principal::User(id) => (1, id),
            Principal::Group(id) => (2, u32::from(id)),
        };
        out[p] = kind;
        out[p + 1] = u8::from(a.deny);
        out[p + 2] = u8::from(a.inherit);
        put16(out, p + 4, a.access.0);
        put32(out, p + 8, id);
    }
    let names = ACES + count * 12;
    for (i, &u) in n.name.units().iter().enumerate() {
        put16(out, names + 2 * i, u);
    }
    names + 2 * n.name.units().len()
}
/// The node `id` from its record, checked as catalog v5 checked its nodes.
pub fn decode(id: u64, b: &[u8]) -> Result<Node> {
    if b.len() < ACES {
        return Err(Error::Corrupt);
    }
    let count = usize::from(b[COUNT]);
    let len = usize::from(get16(b, NAME_LEN));
    if count > 4 || len == 0 || len > super::path::NAME_UNITS || b[PENDING] > 1 {
        return Err(Error::Corrupt);
    }
    let names = ACES + count * 12;
    if b.len() != names + 2 * len {
        return Err(Error::Corrupt);
    }
    let mut acl = Acl::empty(get32(b, OWNER), b[INTEGRITY]);
    for j in 0..count {
        let p = ACES + j * 12;
        if b[p + 1] > 1 || b[p + 2] > 1 || b[p + 3] != 0 || b[p + 6] != 0 || b[p + 7] != 0 {
            return Err(Error::Corrupt);
        }
        let principal_id = get32(b, p + 8);
        let principal = match (b[p], principal_id) {
            (0, 0) => Principal::Everyone,
            (1, 1..) => Principal::User(principal_id),
            (2, 0..=31) => Principal::Group(principal_id as u8),
            _ => return Err(Error::Corrupt),
        };
        acl.push(Ace {
            principal,
            deny: b[p + 1] != 0,
            inherit: b[p + 2] != 0,
            access: Access(get16(b, p + 4)),
        })
        .map_err(|_| Error::Corrupt)?;
    }
    let mut units = [0u16; super::path::NAME_UNITS];
    for (i, u) in units.iter_mut().take(len).enumerate() {
        *u = get16(b, names + 2 * i);
    }
    let name = Name::new(&units[..len]).map_err(|_| Error::Corrupt)?;
    let node = Node {
        id,
        parent: get64(b, PARENT),
        kind: b[KIND],
        pending: b[PENDING] != 0,
        name,
        backing: FileId {
            volume: b[VOLUME..VOLUME + 16].try_into().unwrap(),
            inode: get32(b, INODE),
            generation: get32(b, GENERATION),
        },
        acl,
        attributes: get32(b, ATTRIBUTES),
        version: get64(b, VERSION),
        times: FileTimes {
            created: get64(b, CREATED),
            modified: get64(b, MODIFIED),
            accessed: get64(b, ACCESSED),
        },
    };
    let empty = FileId {
        volume: [0; 16],
        inode: 0,
        generation: 0,
    };
    if id == 0
        || !matches!(node.kind, 1 | 2)
        || !node.acl.valid()
        || node.version == 0
        || node.attributes & !0x22 != 0
        || !node.times.valid()
        || node.name.units() == [46]
        || node.name.units() == [46, 46]
        || node.kind == 2 && node.backing != empty
        || node.kind == 1
            && (node.backing.inode == 0
                || node.backing.generation == 0
                || node.backing.volume == [0; 16])
        || node.parent == 0 && (!node.pending || node.kind != 1)
    {
        return Err(Error::Corrupt);
    }
    Ok(node)
}
/// What the tree holds for a cache slot: the node's id and a digest of its
/// record (FNV-1a), so a changed node is found without a second copy.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Stored {
    pub id: u64,
    pub digest: u64,
}
impl Stored {
    pub const NONE: Self = Self { id: 0, digest: 0 };
    pub fn of(n: &Node) -> Self {
        let mut record = [0u8; RECORD_BYTES + 16];
        let length = encode(n, &mut record);
        let mut digest = 0xcbf2_9ce4_8422_2325u64;
        for &b in n.id.to_le_bytes().iter().chain(&record[..length]) {
            digest = (digest ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        Self { id: n.id, digest }
    }
}
/// Keeps `Work` the only large borrow: what a lookup reads back.
pub struct Scan {
    pub key: [u8; KEY_BYTES],
    pub value: [u8; RECORD_BYTES + 16],
}
impl Scan {
    pub const fn new() -> Self {
        Self {
            key: [0; KEY_BYTES],
            value: [0; RECORD_BYTES + 16],
        }
    }
}
impl Default for Scan {
    fn default() -> Self {
        Self::new()
    }
}

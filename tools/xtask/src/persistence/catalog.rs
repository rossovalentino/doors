//! Independent decoding of the original P07 catalog and its ext2 associations.
use super::{require, tree::Tree, word};
use crate::storage::audit::crc32;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub(crate) struct Node {
    pub id: u64,
    pub parent: u64,
    pub kind: u8,
    pub name: String,
    pub inode: u64,
    pub generation: u64,
    pub attributes: u64,
    pub owner: u64,
    pub integrity: u8,
    pub aces: Vec<(u8, u64, bool, u64)>,
    pub inherited: Vec<bool>,
    pub pending: bool,
}
pub(crate) struct Catalog<'a> {
    pub bytes: &'a [u8],
    /// Registry hive payload records (`door.registry`, checked): key
    /// records then value records past the catalog's 24 + 24 slots.
    hive_keys: &'a [u8],
    hive_values: &'a [u8],
    pub revision: u64,
    pub next_object: u64,
    pub next_key: u64,
    pub associations: Vec<Value>,
    nodes: BTreeMap<u64, Node>,
}
impl Catalog<'_> {
    /// Every registry key record (128 bytes): the catalog's, then the hive's.
    pub fn key_records(&self) -> impl Iterator<Item = &[u8]> {
        self.bytes[7296..10368]
            .chunks_exact(128)
            .chain(self.hive_keys.chunks_exact(128))
    }
    /// Every registry value record (192 bytes): the catalog's, then the hive's.
    pub fn value_records(&self) -> impl Iterator<Item = &[u8]> {
        self.bytes[10368..14976]
            .chunks_exact(192)
            .chain(self.hive_values.chunks_exact(192))
    }
    pub fn paths(&self) -> Result<BTreeMap<String, &Node>, String> {
        let mut paths = BTreeMap::new();
        for node in self.nodes.values() {
            let spelling = path(node, &self.nodes)?;
            require(
                paths.insert(spelling, node).is_none(),
                "duplicate catalog path",
            )?;
        }
        Ok(paths)
    }
}
/// Catalog v4/v5 access-time entry of node slot `index`.
pub(crate) fn accessed_offset(index: usize) -> usize {
    if index < 32 {
        15488 + index * 8
    } else {
        66560 + (index - 32) * 8
    }
}
/// Record start and slot index of live node `id` (both node areas).
pub(crate) fn record_offset(b: &[u8], id: u64) -> Option<(usize, usize)> {
    let slots = if b.len() >= 66560 { 256 } else { 32 };
    (0..slots)
        .map(|i| {
            (
                if i < 32 {
                    128 + i * 224
                } else {
                    16384 + (i - 32) * 224
                },
                i,
            )
        })
        .find(|(start, _)| id != 0 && b[*start..*start + 8] == id.to_le_bytes())
}
pub(crate) fn name(b: &[u8], at: usize, len: usize) -> Result<String, String> {
    require(
        len <= 48 && b[at + len * 2..at + 96].iter().all(|v| *v == 0),
        "catalog name bounds/padding",
    )?;
    let units = (0..len)
        .map(|i| word(b, at + i * 2, 2).map(|v| v as u16))
        .collect::<Result<Vec<_>, _>>()?;
    require(!units.contains(&0), "catalog embedded NUL")?;
    String::from_utf16(&units).map_err(|_| "fixture catalog UTF16".into())
}
pub(crate) fn allows(node: &Node, user: u64, access: u64) -> bool {
    let mut remaining = access;
    for &(kind, id, deny, bits) in &node.aces {
        if kind == 0 || (kind == 1 && id == user) {
            if deny && bits & remaining != 0 {
                return false;
            }
            if !deny {
                remaining &= !bits;
            }
        }
    }
    remaining == 0
}
fn path(node: &Node, nodes: &BTreeMap<u64, Node>) -> Result<String, String> {
    let mut parts = Vec::new();
    let mut cursor = node;
    let mut seen = BTreeSet::new();
    while cursor.id != 1 {
        require(
            seen.insert(cursor.id) && parts.len() < 32,
            "catalog parent cycle",
        )?;
        parts.push(cursor.name.clone());
        cursor = nodes.get(&cursor.parent).ok_or("catalog absent parent")?;
        require(cursor.kind == 2, "catalog parent not directory")?;
    }
    require(cursor.parent == 1, "catalog root identity")?;
    parts.reverse();
    Ok(format!("/{}", parts.join("/")))
}
pub(crate) fn decode(tree: &Tree) -> Result<Option<Catalog<'_>>, String> {
    decode_with_pending(tree, false)
}
/// Explicit diagnostic decoder for the checked-removal pending checkpoint.
/// Ordinary fixture policies continue to reject every pending catalog node.
pub(crate) fn decode_pending(tree: &Tree) -> Result<Option<Catalog<'_>>, String> {
    decode_with_pending(tree, true)
}
fn decode_with_pending(tree: &Tree, allow_pending: bool) -> Result<Option<Catalog<'_>>, String> {
    let Some(file) = tree.files.get("door.catalog") else {
        require(tree.files.len() == 1, "unowned ext2 file without catalog")?;
        return Ok(None);
    };
    let b = &file.bytes;
    let version = word(b, 8, 4)?;
    require(
        matches!((b.len(), version), (16384, 1 | 4 | 6) | (69632, 3 | 5 | 6))
            && &b[..8] == b"DOORCAT1"
            && word(b, 12, 4)? == b.len() as u64
            && word(b, 44, 4)? == 2,
        "catalog version/size",
    )?;
    let mut checked = b.clone();
    checked[40..44].fill(0);
    require(u64::from(crc32(&checked)) == word(b, 40, 4)?, "catalog CRC")?;
    // v4/v5 store accessed times per node index at 15488 (nodes 0..32) and
    // 66560 (nodes 32..256); v1/v3 keep those areas zero.
    let timed = version >= 4;
    // Catalog v6 (P18): every node in `door.nodes`, the node area zero.
    let tree_nodes = version == 6;
    let (tail, production_tail) = if timed {
        (15744, 68352)
    } else {
        (15488, 66560)
    };
    require(
        b[48..128].iter().all(|v| *v == 0)
            && b[tail..16384].iter().all(|v| *v == 0)
            && b[production_tail.min(b.len())..].iter().all(|v| *v == 0),
        "catalog header/footer reserved",
    )?;
    let accessed = |index: usize| {
        let at = if index < 32 {
            15488 + index * 8
        } else {
            66560 + (index - 32) * 8
        };
        word(b, at, 8)
    };
    let next_object = word(b, 16, 8)?;
    let next_key = word(b, 24, 8)?;
    let revision = word(b, 32, 8)?;
    require(
        next_object >= 17 && next_key > 0 && revision > 0,
        "catalog counters",
    )?;
    let mut nodes = BTreeMap::new();
    let mut backings = BTreeSet::new();
    let mut associations = Vec::new();
    if tree_nodes {
        require(
            b[128..7296].iter().all(|v| *v == 0)
                && b[16384.min(b.len())..66560.min(b.len())]
                    .iter()
                    .all(|v| *v == 0)
                && b[15488..15744].iter().all(|v| *v == 0)
                && b[66560.min(b.len())..b.len()].iter().all(|v| *v == 0),
            "catalog v6 node area",
        )?;
        let file = tree
            .files
            .get(NODES)
            .ok_or("catalog v6 without its node tree")?;
        for (id, r) in super::nodes::decode(&file.bytes)? {
            let count = r.aces.len();
            require(
                id < next_object
                    && matches!(r.kind, 1 | 2)
                    && (!r.pending || allow_pending)
                    && r.attributes & !0x22 == 0
                    && r.version > 0
                    && r.owner > 0
                    && r.integrity <= 2
                    && count <= 4
                    && r.times.iter().all(|t| *t <= i64::MAX as u64),
                "catalog v6 node flags",
            )?;
            let mut aces = Vec::new();
            let mut inherited = Vec::new();
            for &(kind, principal, deny, access, inherit) in &r.aces {
                require(
                    access & !63 == 0
                        && match kind {
                            0 => principal == 0,
                            1 => principal > 0,
                            2 => principal < 32,
                            _ => false,
                        },
                    "catalog v6 ACE encoding",
                )?;
                aces.push((kind, principal, deny, access));
                inherited.push(inherit);
            }
            require(!r.name.contains(&0), "catalog embedded NUL")?;
            let node = Node {
                id,
                parent: r.parent,
                kind: r.kind,
                name: String::from_utf16(&r.name).map_err(|_| "catalog v6 name UTF16")?,
                inode: r.inode,
                generation: r.generation,
                attributes: r.attributes,
                owner: r.owner,
                integrity: r.integrity,
                aces,
                inherited,
                pending: r.pending,
            };
            require(
                !node.name.is_empty()
                    && !matches!(node.name.as_str(), "." | "..")
                    && !node.name.contains(['/', '\\']),
                "catalog node name",
            )?;
            if r.kind == 1 {
                let backing = format!("obj{id:016x}");
                let data = tree
                    .files
                    .get(&backing)
                    .ok_or("catalog missing file backing")?;
                require(
                    &r.volume == b"DoorP06Ext2UUID0"
                        && node.inode == data.inode
                        && node.generation == data.generation
                        && node.generation > 0
                        && backings.insert(backing.clone()),
                    "catalog backing identity mismatch",
                )?;
                associations.push(json!({"object":id,"backing":backing,"inode":data.inode,"generation":data.generation,"owner":r.owner}));
            } else {
                require(
                    r.inode == 0 && r.generation == 0 && r.volume == [0; 16],
                    "directory has ext2 backing",
                )?;
            }
            require(nodes.insert(id, node).is_none(), "duplicate object ID")?;
        }
    }
    for (index, r) in b[128..7296]
        .chunks_exact(224)
        .chain(b[16384..66560.min(b.len())].chunks_exact(224))
        .enumerate()
        .filter(|_| !tree_nodes)
    {
        let id = word(r, 0, 8)?;
        if id == 0 {
            require(
                r.iter().all(|v| *v == 0) && accessed(index)? == 0,
                "empty catalog node",
            )?;
            continue;
        }
        let times = [word(r, 208, 8)?, word(r, 216, 8)?, accessed(index)?];
        require(
            times.iter().all(|t| *t <= i64::MAX as u64) && (timed || times == [0; 3]),
            "catalog timestamps",
        )?;
        let count = r[157] as usize;
        let kind = r[16];
        let owner = word(r, 152, 4)?;
        require(
            id < next_object
                && matches!(kind, 1 | 2)
                && (r[17] == 0 || (allow_pending && r[17] == 1))
                && r[19] == 0
                && word(r, 20, 4)? & !0x22 == 0
                && word(r, 24, 8)? > 0
                && owner > 0
                && r[156] <= 2
                && count <= 4
                && r[158..160].iter().all(|v| *v == 0)
                && r[160 + count * 12..208].iter().all(|v| *v == 0),
            "catalog node flags/ACL extent",
        )?;
        let mut aces = Vec::new();
        let mut inherited = Vec::new();
        for a in r[160..160 + count * 12].chunks_exact(12) {
            let principal = word(a, 8, 4)?;
            let access = word(a, 4, 2)?;
            require(
                a[1] <= 1
                    && a[2] <= 1
                    && a[3] == 0
                    && word(a, 6, 2)? == 0
                    && access & !63 == 0
                    && match a[0] {
                        0 => principal == 0,
                        1 => principal > 0,
                        2 => principal < 32,
                        _ => false,
                    },
                "catalog ACE encoding",
            )?;
            aces.push((a[0], principal, a[1] != 0, access));
            inherited.push(a[2] != 0);
        }
        let node = Node {
            id,
            parent: word(r, 8, 8)?,
            kind,
            name: name(r, 56, r[18] as usize)?,
            inode: word(r, 32, 4)?,
            generation: word(r, 36, 4)?,
            attributes: word(r, 20, 4)?,
            owner,
            integrity: r[156],
            aces,
            inherited,
            pending: r[17] != 0,
        };
        require(
            !node.name.is_empty()
                && !matches!(node.name.as_str(), "." | "..")
                && !node.name.contains(['/', '\\']),
            "catalog node name",
        )?;
        if kind == 1 {
            let backing = format!("obj{id:016x}");
            let data = tree
                .files
                .get(&backing)
                .ok_or("catalog missing file backing")?;
            require(
                &r[40..56] == b"DoorP06Ext2UUID0"
                    && node.inode == data.inode
                    && node.generation == data.generation
                    && node.generation > 0
                    && backings.insert(backing.clone()),
                "catalog backing identity mismatch",
            )?;
            associations.push(json!({"object":id,"backing":backing,"inode":data.inode,"generation":data.generation,"owner":owner}));
        } else {
            require(
                r[32..56].iter().all(|v| *v == 0),
                "directory has ext2 backing",
            )?;
        }
        require(nodes.insert(id, node).is_none(), "duplicate object ID")?;
    }
    for (id, parent, spelling) in [
        (1, 1, "root"),
        (2, 1, "system"),
        (3, 1, "users"),
        (4, 1, "state"),
        (5, 1, "apps"),
        (6, 1, "cache"),
        (7, 1, "tmp"),
        (8, 1, "volumes"),
        (9, 1, "devices"),
        (10, 1, "compat"),
        (11, 10, "Windows"),
        (12, 11, "System32"),
        (13, 2, "versions"),
        (14, 2, "current"),
        (15, 4, "registry"),
        (16, 4, "machine"),
    ] {
        let n = nodes.get(&id).ok_or("protected directory missing")?;
        require(
            n.parent == parent && n.name == spelling && n.kind == 2 && n.owner == 1,
            "protected directory identity",
        )?;
    }
    require(
        tree.files.keys().all(|name| {
            name == "seed.txt"
                || name == "door.catalog"
                || name == HIVE
                || (tree_nodes && name == NODES)
                || backings.contains(name)
        }),
        "unowned file backing",
    )?;
    let (hive_keys, hive_values) = match tree.files.get(HIVE) {
        Some(file) => hive(&file.bytes)?,
        None => (&[][..], &[][..]),
    };
    let catalog = Catalog {
        bytes: b,
        hive_keys,
        hive_values,
        revision,
        next_object,
        next_key,
        associations,
        nodes,
    };
    // Complete graph validation is mandatory even if a caller only requests
    // layout metadata; fixture policies cannot skip unreachable/cyclic objects.
    catalog.paths()?;
    Ok(Some(catalog))
}

pub fn inspect(tree: &Tree) -> Result<Value, String> {
    let Some(catalog) = decode(tree)? else {
        return Ok(json!({"state":"initial"}));
    };
    let b = catalog.bytes;
    let revision = catalog.revision;
    let next_object = catalog.next_object;
    let next_key = catalog.next_key;
    let associations = &catalog.associations;
    let paths = catalog.paths()?;
    let user = paths
        .get("/users/1001")
        .ok_or("fixture user directory missing")?;
    require(user.kind == 2 && user.owner == 1001, "fixture user owner")?;
    let note = paths.get("/users/1001/note-Ω.txt");
    let revised = paths.get("/users/1001/revised-Ω.txt");
    let state = if note.is_some() && revised.is_none() {
        "old"
    } else if revised.is_some() && note.is_none() {
        "new"
    } else if note.is_none() && revised.is_none() {
        "capacity"
    } else {
        return Err("both old and new note names reachable".into());
    };
    let mut file_count = 0;
    if state == "capacity" {
        let all: Vec<_> = paths.iter().filter(|(_, n)| n.kind == 1).collect();
        file_count = all.len();
        for index in 0..file_count {
            let n = paths
                .get(&format!("/users/1001/full{index:02}"))
                .ok_or("capacity file sequence gap")?;
            let content = &tree.files[&format!("obj{:016x}", n.id)].bytes;
            require(
                content.len() == 8192
                    && content.iter().all(|v| *v == index as u8 + 0x40)
                    && n.owner == 1001,
                "capacity retained file bytes/owner",
            )?;
        }
    } else {
        let new = state == "new";
        let n = if new { revised.unwrap() } else { note.unwrap() };
        require(
            n.kind == 1
                && n.owner == 1001
                && allows(n, 1001, 63)
                && !allows(n, 1002, 2)
                && allows(n, 1002, 1) != new,
            "note ACL state",
        )?;
        let content = &tree.files[&format!("obj{:016x}", n.id)].bytes;
        let expected: Vec<_> = (0..6000)
            .map(|i| {
                if new {
                    ((i * 19 + 23) % 251) as u8
                } else {
                    ((i * 37 + 11) % 251) as u8
                }
            })
            .collect();
        require(*content == expected, "note data state")?;
        let keep = paths
            .get("/users/1001/keep.bin")
            .ok_or("unrelated file missing")?;
        require(
            tree.files[&format!("obj{:016x}", keep.id)].bytes == b"Door P07 unrelated\n"
                && keep.owner == 1001,
            "unrelated file changed",
        )?;
        require(associations.len() == 2, "unexpected fixture data file")?;
    }
    let settings = inspect_settings(b, state)?;
    let registry = inspect_registry(b, state, next_key)?;
    Ok(
        json!({"state":state,"revision":revision,"next_object":next_object,"next_key":next_key,"associations":associations,"paths":paths.keys().collect::<Vec<_>>(),"settings":settings,"registry":registry,"capacity_files":file_count}),
    )
}
fn inspect_settings(b: &[u8], state: &str) -> Result<Value, String> {
    let mut found = Vec::new();
    for r in b[14976..15488].chunks_exact(64) {
        let schema = word(r, 4, 4)?;
        if schema == 0 {
            require(r.iter().all(|v| *v == 0), "empty setting")?;
            continue;
        }
        require(
            (schema == 2 && r[24..].iter().all(|v| *v == 0)
                || matches!(schema, 3 | 4)
                    && word(r, 24, 4)? < 4
                    && r[28..].iter().all(|v| *v == 0))
                && word(r, 16, 8)? > 0,
            "setting encoding",
        )?;
        found.push((word(r, 0, 4)?, word(r, 8, 4)?, word(r, 12, 4)?));
    }
    let expected = if state == "new" {
        (1001, 2, 150)
    } else {
        (1001, 1, 125)
    };
    require(
        if state == "capacity" {
            found.is_empty()
        } else {
            found == [expected]
        },
        "native setting state",
    )?;
    Ok(json!(found))
}
fn inspect_registry(b: &[u8], state: &str, next_key: u64) -> Result<Value, String> {
    let mut keys = BTreeMap::new();
    for r in b[7296..10368].chunks_exact(128) {
        let id = word(r, 0, 8)?;
        if id == 0 {
            require(r.iter().all(|v| *v == 0), "empty registry key")?;
            continue;
        }
        require(
            id < next_key
                && word(r, 16, 4)? == 1001
                && r[20] == 2
                && r[21] == 64
                && r[22] == 0
                && word(r, 24, 8)? > 0,
            "registry key owner/view",
        )?;
        require(
            keys.insert(id, (word(r, 8, 8)?, name(r, 32, r[23] as usize)?))
                .is_none(),
            "duplicate registry key",
        )?;
    }
    let mut parent = 0;
    let mut fixture = 0;
    for part in ["", "Software", "Door", "Fixture"] {
        let matching: Vec<_> = keys
            .iter()
            .filter(|(_, v)| v.0 == parent && v.1 == part)
            .collect();
        if state == "capacity" {
            break;
        }
        require(matching.len() == 1, "registry hierarchy")?;
        fixture = *matching[0].0;
        parent = fixture;
    }
    require(
        keys.len() == if state == "capacity" { 0 } else { 4 },
        "unexpected registry keys",
    )?;
    let mut values = BTreeMap::new();
    for r in b[10368..14976].chunks_exact(192) {
        let key = word(r, 0, 8)?;
        if key == 0 {
            require(r.iter().all(|v| *v == 0), "empty registry value")?;
            continue;
        }
        let len = word(r, 10, 2)? as usize;
        require(
            key == fixture
                && len <= 64
                && r[12..16].iter().all(|v| *v == 0)
                && r[112 + len..].iter().all(|v| *v == 0),
            "registry value extent/association",
        )?;
        require(
            values
                .insert(
                    name(r, 16, r[8] as usize)?,
                    (r[9], r[112..112 + len].to_vec()),
                )
                .is_none(),
            "duplicate registry value",
        )?;
    }
    let epoch = if state == "new" { 2u32 } else { 1u32 };
    if state == "capacity" {
        require(
            values.is_empty(),
            "capacity registry unexpectedly populated",
        )?;
    } else {
        require(
            values.remove("Epoch") == Some((4, epoch.to_le_bytes().to_vec()))
                && values.remove("Bytes") == Some((3, vec![0xd0, 7, epoch as u8, 0xa5]))
                && values.is_empty(),
            "registry typed data state",
        )?;
    }
    Ok(json!({"keys":keys.len(),"epoch":if state=="capacity"{None}else{Some(epoch)}}))
}
/// The registry hive file name (`door_storage_core::persistence::HIVE_NAME`).
pub(crate) const HIVE: &str = "door.registry";
/// Catalog v6's node tree.
pub(crate) const NODES: &str = "door.nodes";
/// Check a registry hive independently: magic, version, extent within the
/// file and the payload CRC. Returns (key records, value records).
pub(crate) fn hive(b: &[u8]) -> Result<(&[u8], &[u8]), String> {
    require(
        b.len() >= 64 && &b[..8] == b"DOORREG1",
        "registry hive magic",
    )?;
    require(word(b, 8, 4)? == 1, "registry hive version")?;
    require(b[24..64].iter().all(|v| *v == 0), "registry hive header")?;
    let keys = word(b, 12, 4)? as usize;
    let values = word(b, 16, 4)? as usize;
    require(keys <= 104 && values <= 360, "registry hive extent")?;
    let end = 64 + keys * 128 + values * 192;
    require(end <= b.len(), "registry hive length")?;
    require(
        word(b, 20, 4)? == u64::from(crc32(&b[64..end])),
        "registry hive checksum",
    )?;
    Ok((&b[64..64 + keys * 128], &b[64 + keys * 128..end]))
}

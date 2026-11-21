use super::{
    Access, Ace, Acl, CATALOG_BYTES, Error, Name, Principal, Result, State,
    config::{
        CATALOG_KEYS, CATALOG_VALUES, Entry, KEYS, Key, MAX_VALUE_BYTES, RECORD_DATA, RegistryType,
        Settings, VALUES, Value,
    },
    namespace::{EMPTY_ID, Node},
};
const NODE_START: usize = 128;
const KEY_START: usize = 7296;
const VALUE_START: usize = 10368;
const SETTINGS_START: usize = 14976;
/// Catalog versions: 1/3 (32/256 nodes, no timestamps) are read with every
/// time "not recorded"; 4/5 (32/256 nodes) carry timestamps and are written.
const TIMED_SMALL: u32 = 4;
const TIMED_PRODUCTION: u32 = 5;
/// Catalog version 6 (P18 M0a): the registry, settings and counters only;
/// every file and folder lives in the `door.nodes` tree, so the node
/// area is zero (either size).
const TREE: u32 = 6;
/// Accessed times: u64 per node index, nodes 0..32 at 15488 (inside the
/// formerly reserved tail of the first 16 KiB) and production nodes 32..256
/// at 66560 (formerly reserved tail of the 68 KiB catalog). Created and
/// modified occupy node bytes 208..224.
fn accessed_offset(index: usize) -> usize {
    if index < 32 {
        15488 + index * 8
    } else {
        66560 + (index - 32) * 8
    }
}
const ACCESSED_END: usize = 15488 + 32 * 8;
const PRODUCTION_ACCESSED_END: usize = 66560 + 224 * 8;
pub(crate) fn catalog_bytes(capacity: usize) -> Result<usize> {
    match capacity {
        32 => Ok(CATALOG_BYTES),
        256 => Ok(super::PRODUCTION_CATALOG_BYTES),
        _ => Err(Error::Unsupported),
    }
}
fn node_offset(index: usize) -> usize {
    if index < 32 {
        NODE_START + index * 224
    } else {
        CATALOG_BYTES + (index - 32) * 224
    }
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
fn put16(b: &mut [u8], p: usize, v: u16) {
    b[p..p + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], p: usize, v: u32) {
    b[p..p + 4].copy_from_slice(&v.to_le_bytes());
}
fn put64(b: &mut [u8], p: usize, v: u64) {
    b[p..p + 8].copy_from_slice(&v.to_le_bytes());
}
fn zero(b: &[u8]) -> bool {
    b.iter().all(|&u| u == 0)
}
pub(crate) fn checksum(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for (i, &byte) in bytes.iter().enumerate() {
        crc ^= u32::from(if (40..44).contains(&i) { 0 } else { byte });
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320u32.wrapping_mul(crc & 1));
        }
    }
    !crc
}
fn write_name<const N: usize>(b: &mut [u8], p: usize, n: super::path::Name<N>) {
    for (i, &u) in n.units().iter().enumerate() {
        put16(b, p + i * 2, u);
    }
}
/// What a stored name names: a file system entry, a registry key or a
/// registry value, each with its own character rules.
#[derive(Clone, Copy)]
enum NameKind {
    File,
    Key,
    Value,
}
fn read_name<const N: usize>(
    b: &[u8],
    p: usize,
    len: u8,
    allow_empty: bool,
    kind: NameKind,
) -> Result<super::path::Name<N>> {
    use super::path::Name as Named;
    if len > 48 || len == 0 && !allow_empty || !zero(&b[p + usize::from(len) * 2..p + 96]) {
        return Err(Error::Corrupt);
    }
    if len == 0 {
        return Ok(Named::EMPTY);
    }
    let mut units = [0; 48];
    for (i, u) in units.iter_mut().take(usize::from(len)).enumerate() {
        *u = get16(b, p + i * 2);
    }
    let units = &units[..usize::from(len)];
    match kind {
        NameKind::File => Named::new(units),
        NameKind::Key => Named::key(units),
        NameKind::Value => Named::value(units),
    }
    .map_err(|_| Error::Corrupt)
}
pub(crate) fn encode(state: &State, out: &mut [u8]) -> Result<()> {
    if out.len() != catalog_bytes(state.capacity)? {
        return Err(Error::BufferTooSmall);
    }
    validate(state)?;
    out.fill(0);
    out[..8].copy_from_slice(b"DOORCAT1");
    put32(
        out,
        8,
        if state.tree {
            TREE
        } else if state.capacity == 32 {
            TIMED_SMALL
        } else {
            TIMED_PRODUCTION
        },
    );
    put32(out, 12, out.len() as u32);
    put64(out, 16, state.next_object);
    put64(out, 24, state.config.next_key);
    put64(out, 32, state.revision);
    put32(out, 44, 2);
    for (i, n) in state.nodes[..state.capacity].iter().enumerate() {
        if n.id == 0 || state.tree {
            continue;
        }
        if usize::from(n.name.len) > super::path::CATALOG_NAME_UNITS {
            return Err(Error::Unsupported);
        }
        let offset = node_offset(i);
        let b = &mut out[offset..offset + 224];
        put64(b, 0, n.id);
        put64(b, 8, n.parent);
        b[16] = n.kind;
        b[17] = u8::from(n.pending);
        b[18] = n.name.len;
        put32(b, 20, n.attributes);
        put64(b, 24, n.version);
        put32(b, 32, n.backing.inode);
        put32(b, 36, n.backing.generation);
        b[40..56].copy_from_slice(&n.backing.volume);
        write_name(b, 56, n.name);
        put32(b, 152, n.acl.owner);
        b[156] = n.acl.integrity;
        b[157] = n.acl.count;
        for (j, a) in n.acl.entries[..n.acl.count as usize].iter().enumerate() {
            let p = 160 + j * 12;
            let (kind, id) = match a.principal {
                Principal::Everyone => (0, 0),
                Principal::User(id) => (1, id),
                Principal::Group(id) => (2, u32::from(id)),
            };
            b[p] = kind;
            b[p + 1] = u8::from(a.deny);
            b[p + 2] = u8::from(a.inherit);
            put16(b, p + 4, a.access.0);
            put32(b, p + 8, id);
        }
        put64(b, 208, n.times.created);
        put64(b, 216, n.times.modified);
        put64(out, accessed_offset(i), n.times.accessed);
    }
    for (i, k) in state.config.keys[..CATALOG_KEYS].iter().enumerate() {
        write_key(
            &mut out[KEY_START + i * KEY_RECORD..KEY_START + (i + 1) * KEY_RECORD],
            k,
        );
    }
    for (i, e) in state.config.values[..CATALOG_VALUES].iter().enumerate() {
        write_value(
            &mut out[VALUE_START + i * VALUE_RECORD..VALUE_START + (i + 1) * VALUE_RECORD],
            e,
        );
    }
    for (i, s) in state.config.settings.iter().enumerate() {
        if s.schema == 0 {
            continue;
        }
        let b = &mut out[SETTINGS_START + i * 64..SETTINGS_START + (i + 1) * 64];
        put32(b, 0, s.owner);
        put32(b, 4, s.schema);
        put32(b, 8, s.theme);
        put32(b, 12, s.scale);
        put64(b, 16, s.version);
        put32(b, 24, s.wallpaper);
    }
    let digest = checksum(out);
    put32(out, 40, digest);
    Ok(())
}
/// Decode and validate a catalog whose volume has no registry hive.
#[cfg(test)]
pub(crate) fn decode(bytes: &[u8], state: &mut State) -> Result<bool> {
    let migrated = decode_catalog(bytes, state)?;
    validate(state)?;
    Ok(migrated)
}
/// Decode a catalog without validating: registry records in the hive
/// (`decode_hive`) may still be missing. Callers validate afterwards.
pub(crate) fn decode_catalog(bytes: &[u8], state: &mut State) -> Result<bool> {
    let mut migrated = false;
    if bytes.len() < CATALOG_BYTES {
        return Err(Error::Corrupt);
    }
    let (capacity, timed) = match (get32(bytes, 8), bytes.len()) {
        (1, CATALOG_BYTES) => (32, false),
        (TIMED_SMALL, CATALOG_BYTES) => (32, true),
        (3, super::PRODUCTION_CATALOG_BYTES) => (256, false),
        (TIMED_PRODUCTION, super::PRODUCTION_CATALOG_BYTES) => (256, true),
        (TREE, CATALOG_BYTES) => (32, true),
        (TREE, super::PRODUCTION_CATALOG_BYTES) => (256, true),
        _ => return Err(Error::Unsupported),
    };
    if &bytes[..8] != b"DOORCAT1" || get32(bytes, 12) != bytes.len() as u32 || get32(bytes, 44) != 2
    {
        return Err(Error::Unsupported);
    }
    // Untimed versions keep the timestamp areas reserved zero.
    let reserved = if timed { ACCESSED_END } else { 15488 };
    let production_reserved = if timed {
        PRODUCTION_ACCESSED_END
    } else {
        66560
    };
    if get32(bytes, 40) != checksum(bytes)
        || !zero(&bytes[48..128])
        || !zero(&bytes[reserved..CATALOG_BYTES])
        || (capacity == 256 && !zero(&bytes[production_reserved..]))
    {
        return Err(Error::Corrupt);
    }
    state.clear();
    state.capacity = capacity;
    state.tree = get32(bytes, 8) == TREE;
    state.next_object = get64(bytes, 16);
    state.config.next_key = get64(bytes, 24);
    state.revision = get64(bytes, 32);
    for i in 0..capacity {
        let offset = node_offset(i);
        let b = &bytes[offset..offset + 224];
        let accessed = get64(bytes, accessed_offset(i));
        if get64(b, 0) == 0 {
            if !zero(b) || accessed != 0 {
                return Err(Error::Corrupt);
            }
            continue;
        }
        if state.tree {
            return Err(Error::Corrupt);
        }
        let times = super::FileTimes {
            created: get64(b, 208),
            modified: get64(b, 216),
            accessed,
        };
        if b[17] > 1
            || b[19] != 0
            || !zero(&b[158..160])
            || !times.valid()
            || !timed && times != super::FileTimes::NONE
            || b[157] > 4
        {
            return Err(Error::Corrupt);
        }
        let mut acl = Acl::empty(get32(b, 152), b[156]);
        for j in 0..b[157] as usize {
            let p = 160 + j * 12;
            if b[p + 1] > 1 || b[p + 2] > 1 || b[p + 3] != 0 || !zero(&b[p + 6..p + 8]) {
                return Err(Error::Corrupt);
            }
            let id = get32(b, p + 8);
            let principal = match (b[p], id) {
                (0, 0) => Principal::Everyone,
                (1, 1..) => Principal::User(id),
                (2, 0..=31) => Principal::Group(id as u8),
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
        if !zero(&b[160 + b[157] as usize * 12..208]) {
            return Err(Error::Corrupt);
        }
        state.nodes[i] = Node {
            id: get64(b, 0),
            parent: get64(b, 8),
            kind: b[16],
            pending: b[17] != 0,
            name: read_name(b, 56, b[18], false, NameKind::File)?,
            backing: crate::storage::FileId {
                volume: b[40..56].try_into().unwrap(),
                inode: get32(b, 32),
                generation: get32(b, 36),
            },
            acl,
            attributes: get32(b, 20),
            version: get64(b, 24),
            times,
        };
    }
    for i in 0..CATALOG_KEYS {
        state.config.keys[i] =
            read_key(&bytes[KEY_START + i * KEY_RECORD..KEY_START + (i + 1) * KEY_RECORD])?;
    }
    for i in 0..CATALOG_VALUES {
        state.config.values[i] = read_value(
            &bytes[VALUE_START + i * VALUE_RECORD..VALUE_START + (i + 1) * VALUE_RECORD],
        )?;
    }
    for i in 0..8 {
        let b = &bytes[SETTINGS_START + i * 64..SETTINGS_START + (i + 1) * 64];
        if get32(b, 4) == 0 {
            if !zero(b) {
                return Err(Error::Corrupt);
            }
            continue;
        }
        let schema = get32(b, 4);
        if !zero(&b[if schema >= 3 { 28 } else { 24 }..]) {
            return Err(Error::Corrupt);
        }
        if !matches!(schema, 1..=4) {
            return Err(Error::Unsupported);
        }
        // Schema 1 stored no scale, and schemas 2/3 stored 100 as the
        // default no UI could choose: both mean automatic (0) in schema 4.
        let stored = if schema == 1 { 0 } else { get32(b, 12) };
        let scale = if schema < 4 && stored == 100 {
            0
        } else {
            stored
        };
        migrated |= schema == 1 || scale != stored;
        state.config.settings[i] = Settings {
            owner: get32(b, 0),
            schema: 4,
            theme: get32(b, 8),
            scale,
            version: get64(b, 16),
            wallpaper: if schema >= 3 { get32(b, 24) } else { 0 },
        };
    }
    Ok(migrated)
}
pub(crate) fn validate(state: &State) -> Result<()> {
    catalog_bytes(state.capacity)?;
    if state.nodes[state.capacity..].iter().any(|n| n.id != 0) {
        return Err(Error::Corrupt);
    }
    if state.next_object < 17 || state.config.next_key == 0 || state.revision == 0 {
        return Err(Error::Corrupt);
    }
    // Catalog v6: the nodes are in the tree; the cache is checked as
    // nodes load (`namespace::decode_node`).
    let nodes: &[Node] = if state.tree { &[] } else { &state.nodes };
    for (i, n) in nodes.iter().enumerate() {
        if n.id == 0 {
            continue;
        }
        if n.id >= state.next_object
            || !matches!(n.kind, 1 | 2)
            || !n.acl.valid()
            || n.name.len == 0
            || n.version == 0
            || n.attributes & !0x22 != 0
            || !n.times.valid()
            || n.name.units() == [46]
            || n.name.units() == [46, 46]
        {
            return Err(Error::Corrupt);
        }
        if n.kind == 2 && n.backing != EMPTY_ID
            || n.kind == 1
                && (n.backing.inode == 0
                    || n.backing.generation == 0
                    || n.backing.volume == [0; 16])
        {
            return Err(Error::Corrupt);
        }
        if n.parent == 0 {
            if !n.pending || n.kind != 1 {
                return Err(Error::Corrupt);
            }
        } else {
            let mut id = n.id;
            for depth in 0..=32 {
                let a = state
                    .nodes
                    .iter()
                    .find(|a| a.id == id)
                    .ok_or(Error::Corrupt)?;
                if id != n.id && a.kind != 2 {
                    return Err(Error::Corrupt);
                }
                if id == 1 {
                    if a.parent != 1 {
                        return Err(Error::Corrupt);
                    }
                    break;
                }
                if depth == 32 {
                    return Err(Error::Corrupt);
                }
                id = a.parent;
            }
        }
        for old in &state.nodes[..i] {
            if old.id == 0 {
                continue;
            }
            if old.id == n.id
                || n.parent != 0
                    && old.parent == n.parent
                    && old.id != old.parent
                    && n.id != n.parent
                    && old.name.same(n.name, true)
                || n.kind == 1 && old.kind == 1 && old.backing == n.backing
            {
                return Err(Error::Corrupt);
            }
        }
    }
    for (id, parent, name) in [
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
        if state.tree {
            break;
        }
        let n = state
            .nodes
            .iter()
            .find(|n| n.id == id)
            .ok_or(Error::Corrupt)?;
        if n.parent != parent
            || n.kind != 2
            || n.pending
            || n.acl.owner != 1
            || !n.name.same(Name::ascii(name)?, false)
        {
            return Err(Error::Corrupt);
        }
    }
    for (i, k) in state.config.keys.iter().enumerate() {
        if k.id == 0 {
            continue;
        }
        if k.id >= state.config.next_key
            || !matches!(k.root, 1 | 2)
            || !matches!(k.view, 32 | 64)
            || k.root == 1 && k.owner != 0
            || k.root == 2 && k.owner == 0
            || k.version == 0
            || k.parent == 0 && k.name.len != 0
            || k.parent != 0 && k.name.len == 0
        {
            return Err(Error::Corrupt);
        }
        let mut parent = k.parent;
        for depth in 0..=24 {
            if parent == 0 {
                break;
            }
            if depth == 24 {
                return Err(Error::Corrupt);
            }
            let p = state
                .config
                .keys
                .iter()
                .find(|p| p.id == parent)
                .ok_or(Error::Corrupt)?;
            // Tombstones retain their ancestry until every child/handle is
            // retired. A live key can never descend from a deleted key.
            if p.deleted && !k.deleted || p.owner != k.owner || p.root != k.root || p.view != k.view
            {
                return Err(Error::Corrupt);
            }
            parent = p.parent;
        }
        for old in &state.config.keys[..i] {
            if old.id != 0
                && (old.id == k.id
                    || !old.deleted
                        && !k.deleted
                        && old.parent == k.parent
                        && old.owner == k.owner
                        && old.root == k.root
                        && old.view == k.view
                        && old.name.same(k.name, true))
            {
                return Err(Error::Corrupt);
            }
        }
    }
    for (i, e) in state.config.values.iter().enumerate() {
        if e.key == 0 {
            continue;
        }
        if !state
            .config
            .keys
            .iter()
            .any(|k| k.id == e.key && !k.deleted)
        {
            return Err(Error::Corrupt);
        }
        if state.config.values[..i]
            .iter()
            .any(|old| old.key == e.key && old.part == e.part && old.name.same(e.name, true))
        {
            return Err(Error::Corrupt);
        }
        // Every chain is whole: its parts in order, full but for the last,
        // of one type, and well formed as one value.
        let total = e.value.total();
        let parts = total.div_ceil(RECORD_DATA).max(1);
        let part = |p: usize| {
            state
                .config
                .values
                .iter()
                .find(|x| x.key == e.key && x.part as usize == p && x.name.same(e.name, true))
        };
        if e.part != 0 {
            let head = part(0).ok_or(Error::Corrupt)?;
            if e.part as usize >= head.value.total().div_ceil(RECORD_DATA) {
                return Err(Error::Corrupt);
            }
            continue;
        }
        let mut whole = [0u8; MAX_VALUE_BYTES];
        for p in 0..parts {
            let x = part(p).ok_or(Error::Corrupt)?;
            let expected = (total - p * RECORD_DATA).min(RECORD_DATA);
            if x.value.kind != e.value.kind || x.value.data().len() != expected {
                return Err(Error::Corrupt);
            }
            whole[p * RECORD_DATA..p * RECORD_DATA + expected].copy_from_slice(x.value.data());
        }
        Value::check(e.value.kind, &whole[..total]).map_err(|_| Error::Corrupt)?;
    }
    for (i, s) in state.config.settings.iter().enumerate() {
        if s.schema == 0 {
            continue;
        }
        if s.schema != 4
            || s.theme > 2
            // The desktop picture (18 pictures since the 26H2 themes).
            || s.wallpaper >= door_service_protocol::window::WALLPAPERS
            || !super::config::valid_scale(s.scale)
            || s.version == 0
            || state.config.settings[..i]
                .iter()
                .any(|old| old.schema != 0 && old.owner == s.owner)
        {
            return Err(Error::Corrupt);
        }
    }
    Ok(())
}
pub(crate) mod hive;
const KEY_RECORD: usize = 128;
const VALUE_RECORD: usize = 192;
fn write_key(b: &mut [u8], k: &Key) {
    b.fill(0);
    if k.id == 0 {
        return;
    }
    put64(b, 0, k.id);
    put64(b, 8, k.parent);
    put32(b, 16, k.owner);
    b[20] = k.root;
    b[21] = k.view;
    b[22] = u8::from(k.deleted);
    b[23] = k.name.len;
    put64(b, 24, k.version);
    write_name(b, 32, k.name);
}
fn write_value(b: &mut [u8], e: &Entry) {
    b.fill(0);
    if e.key == 0 {
        return;
    }
    put64(b, 0, e.key);
    b[8] = e.name.len;
    b[9] = e.value.kind as u8;
    put16(b, 10, u16::from(e.value.len));
    // A chained value: its part number, and on the head its whole length
    // (an unchained value keeps both zero, as records always had them).
    b[12] = e.part;
    if e.value.total() > usize::from(e.value.len) {
        put16(b, 13, e.value.total);
    }
    write_name(b, 16, e.name);
    b[112..112 + e.value.len as usize].copy_from_slice(e.value.data());
}
fn read_key(b: &[u8]) -> Result<Key> {
    if get64(b, 0) == 0 {
        return if zero(b) {
            Ok(Key::EMPTY)
        } else {
            Err(Error::Corrupt)
        };
    }
    if b[22] > 1 {
        return Err(Error::Corrupt);
    }
    Ok(Key {
        id: get64(b, 0),
        parent: get64(b, 8),
        owner: get32(b, 16),
        root: b[20],
        view: b[21],
        deleted: b[22] != 0,
        name: read_name(b, 32, b[23], true, NameKind::Key)?,
        version: get64(b, 24),
    })
}
fn read_value(b: &[u8]) -> Result<Entry> {
    if get64(b, 0) == 0 {
        return if zero(b) {
            Ok(Entry::EMPTY)
        } else {
            Err(Error::Corrupt)
        };
    }
    let len = get16(b, 10) as usize;
    let part = b[12];
    let total = get16(b, 13) as usize;
    if len > RECORD_DATA
        || b[15] != 0
        || !zero(&b[112 + len..VALUE_RECORD])
        || (part != 0 && total != 0)
        || (total != 0 && (total <= len || total > MAX_VALUE_BYTES))
    {
        return Err(Error::Corrupt);
    }
    let kind = RegistryType::decode(b[9])?;
    // A whole value is checked here; a chain once assembled (`validate`).
    if part == 0 && total == 0 {
        Value::check(kind, &b[112..112 + len]).map_err(|_| Error::Corrupt)?;
    }
    Ok(Entry {
        key: get64(b, 0),
        name: read_name(b, 16, b[8], true, NameKind::Value)?,
        value: Value::chunk(kind, &b[112..112 + len], total.max(len)),
        part,
    })
}

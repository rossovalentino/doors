//! Independent decoding of catalog v6's node tree (`door.nodes`, P18 M0a):
//! a B+tree of 4 KiB slotted pages. Every leaf is reached from the root and
//! along the leaf links, keys strictly ascend, and the records agree: each
//! `N` node has its `C` name and `P` listing keys, a `D` key exactly when it
//! is delete-pending, and no other key exists.
use super::{require, word};
use std::collections::{BTreeMap, BTreeSet};

/// A tree record: its key and value.
pub(crate) type Cell = (Vec<u8>, Vec<u8>);
/// A page awaiting its check: number, level and separator bounds.
type Pending = (usize, usize, Option<Vec<u8>>, Option<Vec<u8>>);

pub(crate) struct Record {
    pub parent: u64,
    pub kind: u8,
    pub pending: bool,
    pub name: Vec<u16>,
    pub attributes: u64,
    pub version: u64,
    pub inode: u64,
    pub generation: u64,
    pub volume: [u8; 16],
    pub owner: u64,
    pub integrity: u8,
    /// (kind, principal, deny, access, inherit)
    pub aces: Vec<(u8, u64, bool, u64, bool)>,
    pub times: [u64; 3],
}

fn cells(page: &[u8]) -> Result<Vec<Cell>, String> {
    let count = word(page, 2, 2)? as usize;
    let start = word(page, 4, 2)? as usize;
    require(
        12 + 2 * count <= start && start <= 4096,
        "node tree page slots",
    )?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let c = word(page, 12 + 2 * i, 2)? as usize;
        require(c >= start && c + 4 <= 4096, "node tree cell offset")?;
        let k = word(page, c, 2)? as usize;
        let v = word(page, c + 2, 2)? as usize;
        require(c + 4 + k + v <= 4096 && k > 0, "node tree cell extent")?;
        out.push((
            page[c + 4..c + 4 + k].to_vec(),
            page[c + 4 + k..c + 4 + k + v].to_vec(),
        ));
    }
    require(
        out.windows(2).all(|w| w[0].0 < w[1].0),
        "node tree keys ascend within a page",
    )?;
    Ok(out)
}

/// Every (key, value) of the tree in `bytes`, in key order.
pub(crate) fn records(bytes: &[u8]) -> Result<Vec<Cell>, String> {
    require(
        bytes.len() >= 8192 && bytes.len().is_multiple_of(4096) && &bytes[..8] == b"DOORBT01",
        "node tree header",
    )?;
    let root = word(bytes, 8, 4)? as usize;
    let height = word(bytes, 12, 4)? as usize;
    let pages = word(bytes, 16, 4)? as usize;
    let total = word(bytes, 24, 8)?;
    require(
        pages * 4096 <= bytes.len() && root > 0 && root < pages && (1..=10).contains(&height),
        "node tree geometry",
    )?;
    let page = |n: usize| -> Result<&[u8], String> {
        require(n > 0 && n < pages, "node tree page number")?;
        Ok(&bytes[n * 4096..(n + 1) * 4096])
    };
    // Depth-first from the root with separator bounds; the leaves in order.
    let mut leaves = Vec::new();
    let mut stack: Vec<Pending> = vec![(root, 0, None, None)];
    let mut seen = BTreeSet::new();
    while let Some((n, level, low, high)) = stack.pop() {
        require(seen.insert(n), "node tree page reached twice")?;
        let p = page(n)?;
        let leaf = level + 1 == height;
        require(p[0] == if leaf { 1 } else { 2 }, "node tree page kind")?;
        let c = cells(p)?;
        if let (Some(low), Some((first, _))) = (&low, c.first()) {
            require(first >= low, "node tree key below its separator")?;
        }
        if let (Some(high), Some((last, _))) = (&high, c.last()) {
            require(last < high, "node tree key past its separator")?;
        }
        if leaf {
            leaves.push(n);
            continue;
        }
        // Children right to left so the stack pops them left to right.
        let mut children = vec![(
            word(p, 8, 4)? as usize,
            low.clone(),
            c.first().map(|x| x.0.clone()),
        )];
        for (i, (k, v)) in c.iter().enumerate() {
            require(v.len() == 4, "node tree branch value")?;
            let next = c.get(i + 1).map(|x| x.0.clone()).or(high.clone());
            children.push((word(v, 0, 4)? as usize, Some(k.clone()), next));
        }
        for (child, lo, hi) in children.into_iter().rev() {
            stack.push((child, level + 1, lo, hi));
        }
    }
    // The leaf chain is exactly the leaves in order.
    let mut chain = Vec::new();
    let mut n = leaves.first().copied().ok_or("node tree has no leaf")?;
    loop {
        chain.push(n);
        let next = word(page(n)?, 8, 4)? as usize;
        if next == 0 {
            break;
        }
        require(chain.len() < pages, "node tree leaf chain loops")?;
        n = next;
    }
    require(chain == leaves, "node tree leaf links")?;
    let mut all: Vec<Cell> = Vec::new();
    for &n in &leaves {
        for cell in cells(page(n)?)? {
            if let Some((last, _)) = all.last() {
                require(&cell.0 > last, "node tree keys ascend across leaves")?;
            }
            all.push(cell);
        }
    }
    require(all.len() as u64 == total, "node tree record count")?;
    Ok(all)
}

fn fold(u: u16) -> u16 {
    if (65..=90).contains(&u) { u + 32 } else { u }
}

/// The nodes of a v6 catalog's tree, by id, with the index records checked.
pub(crate) fn decode(bytes: &[u8]) -> Result<BTreeMap<u64, Record>, String> {
    let all = records(bytes)?;
    let mut nodes = BTreeMap::new();
    let mut index = BTreeSet::new();
    for (k, v) in &all {
        match k[0] {
            b'N' => {
                require(k.len() == 9, "node key")?;
                let id = u64::from_be_bytes(k[1..9].try_into().unwrap());
                require(v.len() >= 78, "node record length")?;
                let count = v[53] as usize;
                let len = word(v, 10, 2)? as usize;
                require(
                    count <= 4 && (1..=255).contains(&len) && v.len() == 78 + count * 12 + 2 * len,
                    "node record extent",
                )?;
                let mut aces = Vec::new();
                for a in v[78..78 + count * 12].chunks_exact(12) {
                    require(
                        a[1] <= 1 && a[2] <= 1 && a[3] == 0 && a[6] == 0 && a[7] == 0,
                        "node ACE encoding",
                    )?;
                    aces.push((a[0], word(a, 8, 4)?, a[1] != 0, word(a, 4, 2)?, a[2] != 0));
                }
                let names = 78 + count * 12;
                let name = (0..len)
                    .map(|i| word(v, names + 2 * i, 2).map(|u| u as u16))
                    .collect::<Result<Vec<u16>, String>>()?;
                require(v[9] <= 1, "node pending flag")?;
                let record = Record {
                    parent: word(v, 0, 8)?,
                    kind: v[8],
                    pending: v[9] != 0,
                    name,
                    attributes: word(v, 12, 4)?,
                    version: word(v, 16, 8)?,
                    inode: word(v, 24, 4)?,
                    generation: word(v, 28, 4)?,
                    volume: v[32..48].try_into().unwrap(),
                    owner: word(v, 48, 4)?,
                    integrity: v[52],
                    aces,
                    times: [word(v, 54, 8)?, word(v, 62, 8)?, word(v, 70, 8)?],
                };
                nodes.insert(id, record);
            }
            b'C' | b'P' | b'D' => {
                require(v.is_empty(), "index record value")?;
                index.insert(k.clone());
            }
            _ => return Err("node tree key kind".into()),
        }
    }
    // Exactly the index records the nodes imply.
    let mut implied = BTreeSet::new();
    for (&id, n) in &nodes {
        let mut c = vec![b'C'];
        c.extend(n.parent.to_be_bytes());
        for &u in &n.name {
            c.extend(fold(u).to_be_bytes());
        }
        c.extend([0, 0]);
        c.extend(id.to_be_bytes());
        implied.insert(c);
        let mut p = vec![b'P'];
        p.extend(n.parent.to_be_bytes());
        p.extend(id.to_be_bytes());
        implied.insert(p);
        if n.pending {
            let mut d = vec![b'D'];
            d.extend(id.to_be_bytes());
            implied.insert(d);
        }
    }
    require(implied == index, "node tree index records")?;
    Ok(nodes)
}

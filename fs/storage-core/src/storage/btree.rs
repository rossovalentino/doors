//! A paged B+tree (P18 M0a): variable-length byte keys and values in
//! 4 KiB slotted pages, kept in one store of pages (an ext2 file written
//! through the redo journal). Allocation-free: the caller lends the page
//! buffers (`Work`).
//!
//! * Page 0 is the header: magic, root, height, page count, record count.
//! * A leaf holds (key, value) cells in key order and links to the next
//!   leaf. A branch holds (key, child) cells and its leftmost child: child
//!   0 holds the keys below key 0, child i + 1 the keys from key i.
//! * Inserting splits full pages upward (the root grows a level). Deleting
//!   leaves pages underfull: an empty leaf stays linked and scans skip it.
//!   No merges, so a change writes only the pages of one root-to-leaf path,
//!   the pages its splits add, and the header.
//! * Keys compare as bytes: callers encode their fields big-endian.
use super::{BLOCK_BYTES, Error, Result};

pub const PAGE: usize = BLOCK_BYTES;
/// The largest key and value: a page holds at least three of the largest
/// cells, so each half of a split has room for the cell that caused it.
pub const MAX_KEY: usize = 536;
pub const MAX_VALUE: usize = 700;
pub const MAX_HEIGHT: usize = 10;
const MAGIC: &[u8; 8] = b"DOORBT01";
const LEAF: u8 = 1;
const BRANCH: u8 = 2;
/// Page header: kind u8, pad u8, count u16, data start u16 (cells grow
/// down from the page end to it), pad u16, link u32 (a leaf's next leaf,
/// a branch's leftmost child).
const HEADER: usize = 12;
const SLOT: usize = 2;
/// Cells a page can hold at most (empty key and value: 4 + 2 bytes each).
const MAX_CELLS: usize = (PAGE - HEADER) / 6 + 1;

/// The pages of one tree (a file of 4 KiB pages).
pub trait Pages {
    fn read(&mut self, page: u32, out: &mut [u8; PAGE]) -> Result<()>;
    fn write(&mut self, page: u32, bytes: &[u8; PAGE]) -> Result<()>;
}

fn get16(b: &[u8], p: usize) -> usize {
    u16::from_le_bytes([b[p], b[p + 1]]) as usize
}
fn put16(b: &mut [u8], p: usize, v: usize) {
    b[p..p + 2].copy_from_slice(&(v as u16).to_le_bytes());
}
fn get32(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(b[p..p + 4].try_into().unwrap())
}
fn put32(b: &mut [u8], p: usize, v: u32) {
    b[p..p + 4].copy_from_slice(&v.to_le_bytes());
}
fn get64(b: &[u8], p: usize) -> u64 {
    u64::from_le_bytes(b[p..p + 8].try_into().unwrap())
}
fn put64(b: &mut [u8], p: usize, v: u64) {
    b[p..p + 8].copy_from_slice(&v.to_le_bytes());
}

fn init(page: &mut [u8; PAGE], kind: u8, link: u32) {
    page.fill(0);
    page[0] = kind;
    put16(page, 4, PAGE);
    put32(page, 8, link);
}
fn kind(p: &[u8; PAGE]) -> u8 {
    p[0]
}
fn count(p: &[u8; PAGE]) -> usize {
    get16(p, 2)
}
fn link(p: &[u8; PAGE]) -> u32 {
    get32(p, 8)
}
fn data_start(p: &[u8; PAGE]) -> usize {
    get16(p, 4)
}
fn cell(p: &[u8; PAGE], i: usize) -> usize {
    get16(p, HEADER + SLOT * i)
}
/// A cell: key length u16, value length u16, key, value.
fn key(p: &[u8; PAGE], i: usize) -> &[u8] {
    let c = cell(p, i);
    &p[c + 4..c + 4 + get16(p, c)]
}
fn value(p: &[u8; PAGE], i: usize) -> &[u8] {
    let c = cell(p, i);
    let k = get16(p, c);
    &p[c + 4 + k..c + 4 + k + get16(p, c + 2)]
}
fn child(p: &[u8; PAGE], i: usize) -> u32 {
    get32(value(p, i), 0)
}
const fn cell_bytes(k: usize, v: usize) -> usize {
    4 + k + v
}
/// Bytes the cells and slots use.
fn used(p: &[u8; PAGE]) -> usize {
    (0..count(p))
        .map(|i| cell_bytes(key(p, i).len(), value(p, i).len()) + SLOT)
        .sum()
}
fn fits(p: &[u8; PAGE], k: usize, v: usize) -> bool {
    HEADER + used(p) + cell_bytes(k, v) + SLOT <= PAGE
}
/// First index whose key is >= `k`, and whether it is equal.
fn search(p: &[u8; PAGE], k: &[u8]) -> (usize, bool) {
    let (mut low, mut high) = (0, count(p));
    while low < high {
        let mid = (low + high) / 2;
        match key(p, mid).cmp(k) {
            core::cmp::Ordering::Less => low = mid + 1,
            core::cmp::Ordering::Equal => return (mid, true),
            core::cmp::Ordering::Greater => high = mid,
        }
    }
    (low, false)
}
/// A branch's child for `k`: its index (0: the leftmost) and page.
fn route(p: &[u8; PAGE], k: &[u8]) -> (usize, u32) {
    let (i, equal) = search(p, k);
    let index = if equal { i + 1 } else { i };
    (
        index,
        if index == 0 {
            link(p)
        } else {
            child(p, index - 1)
        },
    )
}
fn compact(p: &mut [u8; PAGE], scratch: &mut [u8; PAGE]) {
    scratch.copy_from_slice(p);
    let mut end = PAGE;
    for i in 0..count(scratch) {
        let c = cell(scratch, i);
        let n = cell_bytes(get16(scratch, c), get16(scratch, c + 2));
        end -= n;
        p[end..end + n].copy_from_slice(&scratch[c..c + n]);
        put16(p, HEADER + SLOT * i, end);
    }
    put16(p, 4, end);
}
/// Inserts a cell at `i` (the caller checked `fits`).
fn insert(p: &mut [u8; PAGE], i: usize, k: &[u8], v: &[u8], scratch: &mut [u8; PAGE]) {
    let n = cell_bytes(k.len(), v.len());
    let n_count = count(p);
    if data_start(p) < HEADER + SLOT * (n_count + 1) + n {
        compact(p, scratch);
    }
    let start = data_start(p) - n;
    put16(p, start, k.len());
    put16(p, start + 2, v.len());
    p[start + 4..start + 4 + k.len()].copy_from_slice(k);
    p[start + 4 + k.len()..start + n].copy_from_slice(v);
    p.copy_within(
        HEADER + SLOT * i..HEADER + SLOT * n_count,
        HEADER + SLOT * (i + 1),
    );
    put16(p, HEADER + SLOT * i, start);
    put16(p, 2, n_count + 1);
    put16(p, 4, start);
}
fn remove(p: &mut [u8; PAGE], i: usize) {
    let n = count(p);
    p.copy_within(
        HEADER + SLOT * (i + 1)..HEADER + SLOT * n,
        HEADER + SLOT * i,
    );
    put16(p, 2, n - 1);
}

/// The tree's header (page 0).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    pub root: u32,
    pub height: u32,
    pub pages: u32,
    pub records: u64,
}
/// The buffers a tree operation borrows: one page per level, a scratch
/// page, the two halves of a split, and a split's gathered cells.
pub struct Work {
    path: [[u8; PAGE]; MAX_HEIGHT],
    numbers: [u32; MAX_HEIGHT],
    indices: [usize; MAX_HEIGHT],
    scratch: [u8; PAGE],
    left: [u8; PAGE],
    right: [u8; PAGE],
    cells: [u8; 2 * PAGE],
    spans: [(u16, u16, u16); MAX_CELLS + 1],
    separator: [u8; MAX_KEY],
}
impl Work {
    pub const fn new() -> Self {
        Self {
            path: [[0; PAGE]; MAX_HEIGHT],
            numbers: [0; MAX_HEIGHT],
            indices: [0; MAX_HEIGHT],
            scratch: [0; PAGE],
            left: [0; PAGE],
            right: [0; PAGE],
            cells: [0; 2 * PAGE],
            spans: [(0, 0, 0); MAX_CELLS + 1],
            separator: [0; MAX_KEY],
        }
    }
}
impl Default for Work {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Tree<P> {
    pub pages: P,
    header: Header,
}
impl<P: Pages> Tree<P> {
    /// Writes an empty tree (its header and an empty root leaf).
    pub fn create(mut pages: P, work: &mut Work) -> Result<Self> {
        init(&mut work.scratch, LEAF, 0);
        pages.write(1, &work.scratch)?;
        let mut tree = Self {
            pages,
            header: Header {
                root: 1,
                height: 1,
                pages: 2,
                records: 0,
            },
        };
        tree.store_header(work)?;
        Ok(tree)
    }
    pub fn open(mut pages: P, work: &mut Work) -> Result<Self> {
        pages.read(0, &mut work.scratch)?;
        let h = &work.scratch;
        if &h[..8] != MAGIC {
            return Err(Error::Corrupt);
        }
        let header = Header {
            root: get32(h, 8),
            height: get32(h, 12),
            pages: get32(h, 16),
            records: get64(h, 24),
        };
        if header.root == 0
            || header.root >= header.pages
            || header.height == 0
            || header.height as usize > MAX_HEIGHT
        {
            return Err(Error::Corrupt);
        }
        Ok(Self { pages, header })
    }
    pub fn header(&self) -> Header {
        self.header
    }
    pub fn into_pages(self) -> P {
        self.pages
    }
    fn store_header(&mut self, work: &mut Work) -> Result<()> {
        let h = &mut work.scratch;
        h.fill(0);
        h[..8].copy_from_slice(MAGIC);
        put32(h, 8, self.header.root);
        put32(h, 12, self.header.height);
        put32(h, 16, self.header.pages);
        put64(h, 24, self.header.records);
        self.pages.write(0, h)
    }
    fn allocate(&mut self) -> Result<u32> {
        let page = self.header.pages;
        self.header.pages = page.checked_add(1).ok_or(Error::NoSpace)?;
        Ok(page)
    }
    fn valid_page(&self, page: u32) -> bool {
        page != 0 && page < self.header.pages
    }
    /// Reads the root-to-leaf path of `k`: `work.path[..height]`, their
    /// numbers and the child index taken at each branch.
    fn descend(&mut self, k: &[u8], work: &mut Work) -> Result<usize> {
        let depth = self.header.height as usize;
        let mut page = self.header.root;
        for level in 0..depth {
            if !self.valid_page(page) {
                return Err(Error::Corrupt);
            }
            self.pages.read(page, &mut work.path[level])?;
            work.numbers[level] = page;
            let leaf = level + 1 == depth;
            let p = &work.path[level];
            if kind(p) != if leaf { LEAF } else { BRANCH } || count(p) > MAX_CELLS {
                return Err(Error::Corrupt);
            }
            if !leaf {
                let (index, next) = route(p, k);
                work.indices[level] = index;
                page = next;
            }
        }
        Ok(depth)
    }
    /// The value of `k` into `out`: its length, None when absent.
    pub fn get(&mut self, k: &[u8], out: &mut [u8], work: &mut Work) -> Result<Option<usize>> {
        let depth = self.descend(k, work)?;
        let leaf = &work.path[depth - 1];
        let (i, equal) = search(leaf, k);
        if !equal {
            return Ok(None);
        }
        let v = value(leaf, i);
        let n = v.len();
        out.get_mut(..n).ok_or(Error::Bounds)?.copy_from_slice(v);
        Ok(Some(n))
    }
    /// The first record after `k` (from `k` itself when `inclusive`): its
    /// key and value lengths in `key_out` / `value_out`.
    pub fn next(
        &mut self,
        k: &[u8],
        inclusive: bool,
        key_out: &mut [u8],
        value_out: &mut [u8],
        work: &mut Work,
    ) -> Result<Option<(usize, usize)>> {
        let depth = self.descend(k, work)?;
        let mut page = work.numbers[depth - 1];
        let (mut i, equal) = search(&work.path[depth - 1], k);
        if equal && !inclusive {
            i += 1;
        }
        let leaf = &mut work.path[depth - 1];
        // Bounded: a scan visits at most every page once.
        for _ in 0..self.header.pages {
            if i < count(leaf) {
                let (kk, vv) = (key(leaf, i), value(leaf, i));
                key_out
                    .get_mut(..kk.len())
                    .ok_or(Error::Bounds)?
                    .copy_from_slice(kk);
                value_out
                    .get_mut(..vv.len())
                    .ok_or(Error::Bounds)?
                    .copy_from_slice(vv);
                return Ok(Some((kk.len(), vv.len())));
            }
            let next = link(leaf);
            if next == 0 {
                return Ok(None);
            }
            if !self.valid_page(next) || next == page {
                return Err(Error::Corrupt);
            }
            page = next;
            self.pages.read(page, leaf)?;
            if kind(leaf) != LEAF {
                return Err(Error::Corrupt);
            }
            i = 0;
        }
        Err(Error::Corrupt)
    }
    /// Removes `k`: whether it was there.
    pub fn delete(&mut self, k: &[u8], work: &mut Work) -> Result<bool> {
        let depth = self.descend(k, work)?;
        let (i, equal) = search(&work.path[depth - 1], k);
        if !equal {
            return Ok(false);
        }
        remove(&mut work.path[depth - 1], i);
        self.pages
            .write(work.numbers[depth - 1], &work.path[depth - 1])?;
        self.header.records -= 1;
        self.store_header(work)?;
        Ok(true)
    }
    /// Inserts or replaces `k`: whether it is new.
    pub fn put(&mut self, k: &[u8], v: &[u8], work: &mut Work) -> Result<bool> {
        if k.is_empty() || k.len() > MAX_KEY || v.len() > MAX_VALUE {
            return Err(Error::Unsupported);
        }
        let depth = self.descend(k, work)?;
        let leaf_level = depth - 1;
        let (i, equal) = search(&work.path[leaf_level], k);
        if equal {
            remove(&mut work.path[leaf_level], i);
        }
        if fits(&work.path[leaf_level], k.len(), v.len()) {
            insert(&mut work.path[leaf_level], i, k, v, &mut work.scratch);
            self.pages
                .write(work.numbers[leaf_level], &work.path[leaf_level])?;
        } else {
            self.split_up(leaf_level, i, k, v, work)?;
        }
        if !equal {
            self.header.records += 1;
        }
        self.store_header(work)?;
        Ok(!equal)
    }
    /// Splits the full page at `level` with the cell (`k`, `v`) inserted at
    /// `at`, then inserts the separator into the parents, splitting them in
    /// turn; a split root gets a new root above it.
    fn split_up(
        &mut self,
        level: usize,
        at: usize,
        k: &[u8],
        v: &[u8],
        work: &mut Work,
    ) -> Result<()> {
        let mut level = level;
        let (mut length, mut right) = self.split(level, at, k, v, work)?;
        loop {
            if level == 0 {
                let root = self.allocate()?;
                init(&mut work.left, BRANCH, work.numbers[0]);
                let separator = work.separator;
                insert(
                    &mut work.left,
                    0,
                    &separator[..length],
                    &right.to_le_bytes(),
                    &mut work.scratch,
                );
                self.pages.write(root, &work.left)?;
                self.header.root = root;
                self.header.height += 1;
                if self.header.height as usize > MAX_HEIGHT {
                    return Err(Error::NoSpace);
                }
                return Ok(());
            }
            level -= 1;
            let index = work.indices[level];
            let separator = work.separator;
            if fits(&work.path[level], length, 4) {
                insert(
                    &mut work.path[level],
                    index,
                    &separator[..length],
                    &right.to_le_bytes(),
                    &mut work.scratch,
                );
                return self.pages.write(work.numbers[level], &work.path[level]);
            }
            (length, right) = self.split(
                level,
                index,
                &separator[..length],
                &right.to_le_bytes(),
                work,
            )?;
        }
    }
    /// Splits `work.path[level]` (with the cell inserted at `at`) into its
    /// own page and a new right page, writing both: the separator's length
    /// (the separator in `work.separator`) and the right page.
    fn split(
        &mut self,
        level: usize,
        at: usize,
        k: &[u8],
        v: &[u8],
        work: &mut Work,
    ) -> Result<(usize, u32)> {
        let page = &work.path[level];
        let branch = kind(page) == BRANCH;
        let total = count(page) + 1;
        if total > work.spans.len() {
            return Err(Error::Corrupt);
        }
        // Gather the cells, the new one included, back to back.
        let mut used_bytes = 0usize;
        for j in 0..total {
            let (kk, vv): (&[u8], &[u8]) = if j == at {
                (k, v)
            } else {
                let src = if j < at { j } else { j - 1 };
                (key(page, src), value(page, src))
            };
            let end = used_bytes + kk.len() + vv.len();
            if end > work.cells.len() {
                return Err(Error::Corrupt);
            }
            work.cells[used_bytes..used_bytes + kk.len()].copy_from_slice(kk);
            work.cells[used_bytes + kk.len()..end].copy_from_slice(vv);
            work.spans[j] = (used_bytes as u16, kk.len() as u16, vv.len() as u16);
            used_bytes = end;
        }
        // The split point: about half the bytes on each side, at least one
        // cell on each (a branch's middle cell climbs).
        let weight = |j: usize| {
            let (_, kk, vv) = work.spans[j];
            cell_bytes(kk as usize, vv as usize) + SLOT
        };
        let all: usize = (0..total).map(weight).sum();
        let mut mid = 0;
        let mut acc = 0;
        while mid + 1 < total && acc + weight(mid) <= all / 2 {
            acc += weight(mid);
            mid += 1;
        }
        let mid = mid.clamp(1, total - if branch { 2 } else { 1 });
        let old_link = link(page);
        let right = self.allocate()?;
        let span = |work: &Work, j: usize| {
            let (o, kk, vv) = work.spans[j];
            (o as usize, kk as usize, vv as usize)
        };
        let (left_link, right_link, skip) = if branch {
            let (o, kk, _) = span(work, mid);
            (old_link, get32(&work.cells, o + kk), Some(mid))
        } else {
            (right, old_link, None)
        };
        init(&mut work.left, kind(&work.path[level]), left_link);
        init(&mut work.right, kind(&work.path[level]), right_link);
        let mut l = 0;
        let mut r = 0;
        for j in 0..total {
            if Some(j) == skip {
                continue;
            }
            let (o, kk, vv) = span(work, j);
            let (target, index) = if j < mid {
                l += 1;
                (&mut work.left, l - 1)
            } else {
                r += 1;
                (&mut work.right, r - 1)
            };
            let (cells, scratch) = (&work.cells, &mut work.scratch);
            insert(
                target,
                index,
                &cells[o..o + kk],
                &cells[o + kk..o + kk + vv],
                scratch,
            );
        }
        let (o, kk, _) = span(work, mid);
        work.separator[..kk].copy_from_slice(&work.cells[o..o + kk]);
        work.path[level].copy_from_slice(&work.left);
        self.pages.write(work.numbers[level], &work.path[level])?;
        self.pages.write(right, &work.right)?;
        Ok((kk, right))
    }
    /// Checks the whole tree (offline audit, tests): page kinds, key order
    /// within pages and against the separators, leaf links in order, and
    /// the record count. The number of records.
    pub fn check(&mut self, work: &mut Work) -> Result<u64> {
        let mut records = 0;
        // Depth-first with an explicit stack of (page, level, low, high
        // bounds as indices into the parents' keys): bounds checked by
        // comparing against the parent's separators held in `path`.
        self.check_page(self.header.root, 0, None, None, &mut records, work)?;
        // The leaves in link order are ascending.
        let mut page = self.header.root;
        for level in 0..self.header.height as usize - 1 {
            self.pages.read(page, &mut work.path[level])?;
            page = link(&work.path[level]);
        }
        let mut last_len = 0usize;
        let mut first = true;
        for _ in 0..self.header.pages {
            self.pages.read(page, &mut work.scratch)?;
            for i in 0..count(&work.scratch) {
                let kk = key(&work.scratch, i);
                if !first && kk <= &work.separator[..last_len] {
                    return Err(Error::Corrupt);
                }
                work.separator[..kk.len()].copy_from_slice(kk);
                last_len = kk.len();
                first = false;
            }
            page = link(&work.scratch);
            if page == 0 {
                break;
            }
        }
        if records != self.header.records {
            return Err(Error::Corrupt);
        }
        Ok(records)
    }
    fn check_page(
        &mut self,
        page: u32,
        level: usize,
        low: Option<(usize, usize)>,
        high: Option<(usize, usize)>,
        records: &mut u64,
        work: &mut Work,
    ) -> Result<()> {
        if !self.valid_page(page) || level >= self.header.height as usize {
            return Err(Error::Corrupt);
        }
        self.pages.read(page, &mut work.path[level])?;
        let leaf = level + 1 == self.header.height as usize;
        let p = &work.path[level];
        if kind(p) != if leaf { LEAF } else { BRANCH } || HEADER + used(p) > PAGE {
            return Err(Error::Corrupt);
        }
        let n = count(p);
        for i in 0..n {
            let c = cell(p, i);
            if c < HEADER + SLOT * n
                || c + 4 > PAGE
                || c + cell_bytes(get16(p, c), get16(p, c + 2)) > PAGE
            {
                return Err(Error::Corrupt);
            }
            if i > 0 && key(p, i - 1) >= key(p, i) {
                return Err(Error::Corrupt);
            }
        }
        // Bounds: (level, index) of the parent separators.
        if let Some((l, i)) = low
            && n > 0
            && key(&work.path[level], 0) < key(&work.path[l], i)
        {
            return Err(Error::Corrupt);
        }
        if let Some((l, i)) = high
            && n > 0
            && key(&work.path[level], n - 1) >= key(&work.path[l], i)
        {
            return Err(Error::Corrupt);
        }
        if leaf {
            *records += n as u64;
            return Ok(());
        }
        for i in 0..=n {
            let (child_page, lo, hi) = {
                let p = &work.path[level];
                let child_page = if i == 0 { link(p) } else { child(p, i - 1) };
                let lo = if i == 0 { low } else { Some((level, i - 1)) };
                let hi = if i == n { high } else { Some((level, i)) };
                (child_page, lo, hi)
            };
            self.check_page(child_page, level + 1, lo, hi, records, work)?;
        }
        Ok(())
    }
}

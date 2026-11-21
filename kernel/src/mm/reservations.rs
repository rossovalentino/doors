//! Windows address-space reservations of one process (P18 M0b): VirtualAlloc
//! MEM_RESERVE ranges anywhere in the user half, sorted by start, with no
//! per-reservation page cap. Reserving costs no frames and no page-table
//! pages; committed pages live in the process's own page tables
//! (`user_table::pages`). The process space checks these against its
//! regions (images, stacks, sections) in both directions, so neither can be
//! placed over the other.
use super::Error;

/// Reservations per process. CoreCLR's GC and executable allocator and V8's
/// code space make a few hundred; 1,024 leaves headroom (16 KiB per
/// process slot).
pub const MAX_RESERVATIONS: usize = 1024;
/// Windows' allocation granularity: reservations start on 64 KiB.
pub const GRANULARITY: u64 = 0x1_0000;
/// MEM_PRIVATE and MEM_IMAGE.
pub const PRIVATE: u32 = 0x2_0000;
pub const IMAGE: u32 = 0x100_0000;
/// MEM_MAPPED: a view of a Windows section (`Vm::win_section_map`).
pub const MAPPED: u32 = 0x4_0000;
const PAGE: u64 = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reservation {
    pub start: u64,
    pub pages: u64,
    /// The protection VirtualAlloc was given (MEMORY_BASIC_INFORMATION
    /// AllocationProtect).
    pub protect: u32,
    /// MEMORY_BASIC_INFORMATION Type: MEM_PRIVATE, or MEM_IMAGE for a
    /// mapped executable or DLL.
    pub kind: u32,
    /// A placeholder (VirtualAlloc2 MEM_RESERVE_PLACEHOLDER): nothing may be
    /// committed in it; it is split, coalesced, or replaced by an allocation
    /// or a section view of exactly its range.
    pub placeholder: bool,
    /// A section view: its pages belong to the section (MapViewOfFile;
    /// `kind` is MEM_MAPPED, or MEM_IMAGE for an image section's view).
    pub view: bool,
}
impl Reservation {
    pub const fn end(self) -> u64 {
        self.start + self.pages * PAGE
    }
    pub const fn contains(self, address: u64) -> bool {
        address >= self.start && address < self.end()
    }
}

/// Where a free range is looked for (`MEM_TOP_DOWN` or not).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    BottomUp,
    TopDown,
}

pub struct Reservations {
    entries: [Reservation; MAX_RESERVATIONS],
    len: usize,
}
impl Reservations {
    pub const EMPTY: Self = Self {
        entries: [Reservation {
            start: 0,
            pages: 0,
            protect: 0,
            kind: 0,
            placeholder: false,
            view: false,
        }; MAX_RESERVATIONS],
        len: 0,
    };
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn iter(&self) -> impl Iterator<Item = Reservation> + '_ {
        self.entries[..self.len].iter().copied()
    }
    /// Index of the first entry whose end is above `address`.
    fn first_ending_after(&self, address: u64) -> usize {
        self.entries[..self.len].partition_point(|r| r.end() <= address)
    }
    /// The reservation holding `address`.
    pub fn find(&self, address: u64) -> Option<Reservation> {
        let i = self.first_ending_after(address);
        self.entries[..self.len]
            .get(i)
            .copied()
            .filter(|r| r.contains(address))
    }
    /// The reservation holding all of `[address, address + bytes)`.
    pub fn holding(&self, address: u64, bytes: u64) -> Option<Reservation> {
        let end = address.checked_add(bytes)?;
        self.find(address).filter(|r| end <= r.end())
    }
    /// The first reservation starting at or above `address` (VirtualQuery of
    /// a free address reports the gap up to it).
    pub fn next_at_or_after(&self, address: u64) -> Option<Reservation> {
        let i = self.entries[..self.len].partition_point(|r| r.start < address);
        self.entries[..self.len].get(i).copied()
    }
    /// Whether any reservation overlaps `[start, end)`.
    pub fn overlaps(&self, start: u64, end: u64) -> bool {
        let i = self.first_ending_after(start);
        self.entries[..self.len]
            .get(i)
            .is_some_and(|r| r.start < end)
    }
    /// Record a private reservation `[start, start + pages pages)`; it must
    /// not overlap another.
    pub fn insert(&mut self, start: u64, pages: u64, protect: u32) -> Result<(), Error> {
        self.insert_kind(start, pages, protect, PRIVATE)
    }
    /// `insert` with a MEMORY_BASIC_INFORMATION type.
    pub fn insert_kind(
        &mut self,
        start: u64,
        pages: u64,
        protect: u32,
        kind: u32,
    ) -> Result<(), Error> {
        if pages == 0 || !start.is_multiple_of(GRANULARITY) {
            return Err(Error::Alignment);
        }
        let end = pages
            .checked_mul(PAGE)
            .and_then(|b| start.checked_add(b))
            .ok_or(Error::Overflow)?;
        if self.overlaps(start, end) {
            return Err(Error::Overlap);
        }
        if self.len == MAX_RESERVATIONS {
            return Err(Error::MappingLimit);
        }
        let i = self.first_ending_after(start);
        self.entries.copy_within(i..self.len, i + 1);
        self.entries[i] = Reservation {
            start,
            pages,
            protect,
            kind,
            placeholder: false,
            view: false,
        };
        self.len += 1;
        Ok(())
    }
    /// Change the MEMORY_BASIC_INFORMATION type of the reservation based at
    /// `start`.
    pub fn set_kind(&mut self, start: u64, kind: u32) -> Result<(), Error> {
        let i = self.first_ending_after(start);
        let entry = self.entries[..self.len]
            .get_mut(i)
            .filter(|r| r.start == start)
            .ok_or(Error::NotCommitted)?;
        entry.kind = kind;
        Ok(())
    }
    /// The reservation based at `start`, to change in place.
    pub fn at_mut(&mut self, start: u64) -> Result<&mut Reservation, Error> {
        let i = self.first_ending_after(start);
        self.entries[..self.len]
            .get_mut(i)
            .filter(|r| r.start == start)
            .ok_or(Error::NotCommitted)
    }
    /// Split the reservation based at `start` after `pages` pages into two
    /// with its attributes (placeholders split at page granularity).
    pub fn split(&mut self, start: u64, pages: u64) -> Result<(), Error> {
        let i = self.first_ending_after(start);
        let r = *self.entries[..self.len]
            .get(i)
            .filter(|r| r.start == start)
            .ok_or(Error::NotCommitted)?;
        if pages == 0 || pages >= r.pages {
            return Err(Error::Alignment);
        }
        if self.len == MAX_RESERVATIONS {
            return Err(Error::MappingLimit);
        }
        self.entries.copy_within(i + 1..self.len, i + 2);
        self.entries[i].pages = pages;
        self.entries[i + 1] = Reservation {
            start: start + pages * PAGE,
            pages: r.pages - pages,
            ..r
        };
        self.len += 1;
        Ok(())
    }
    /// Merge the placeholders that exactly cover `[start, end)` into one
    /// (MEM_COALESCE_PLACEHOLDERS).
    pub fn coalesce(&mut self, start: u64, end: u64) -> Result<(), Error> {
        let i = self.first_ending_after(start);
        let mut j = i;
        let mut cursor = start;
        while cursor < end {
            let r = self.entries[..self.len]
                .get(j)
                .filter(|r| r.start == cursor && r.placeholder)
                .ok_or(Error::InvalidState)?;
            cursor = r.end();
            j += 1;
        }
        if cursor != end || j == i {
            return Err(Error::InvalidState);
        }
        self.entries[i].pages = (end - start) / PAGE;
        self.entries.copy_within(j..self.len, i + 1);
        self.len -= j - i - 1;
        Ok(())
    }
    /// Remove the reservation starting exactly at `start` (MEM_RELEASE
    /// takes the base address VirtualAlloc returned).
    pub fn remove(&mut self, start: u64) -> Result<Reservation, Error> {
        let i = self.first_ending_after(start);
        let r = *self.entries[..self.len]
            .get(i)
            .filter(|r| r.start == start)
            .ok_or(Error::NotCommitted)?;
        self.entries.copy_within(i + 1..self.len, i);
        self.len -= 1;
        Ok(r)
    }
    /// A free, granularity-aligned range of `bytes` in `[low, high)` that
    /// neither a reservation nor `busy` (the process's other regions)
    /// overlaps: the lowest (Windows' default) or the highest
    /// (`MEM_TOP_DOWN`). Gaps are taken from this table; `busy` is asked
    /// only about candidates, so the scan is bounded by the reservations
    /// plus the regions it steps over.
    pub fn free_range(
        &self,
        low: u64,
        high: u64,
        bytes: u64,
        direction: Direction,
        busy: &dyn Fn(u64, u64) -> Option<u64>,
    ) -> Option<u64> {
        let bytes = bytes.checked_next_multiple_of(GRANULARITY)?;
        let low = low.checked_next_multiple_of(GRANULARITY)?;
        if bytes == 0 || high <= low || high - low < bytes {
            return None;
        }
        match direction {
            Direction::BottomUp => {
                let mut candidate = low;
                // Each step moves past one blocker, so this ends.
                for _ in 0..(2 * MAX_RESERVATIONS + 512) {
                    let end = candidate.checked_add(bytes)?;
                    if end > high {
                        return None;
                    }
                    let i = self.first_ending_after(candidate);
                    if let Some(r) = self.entries[..self.len].get(i).filter(|r| r.start < end) {
                        candidate = r.end().checked_next_multiple_of(GRANULARITY)?;
                        continue;
                    }
                    match busy(candidate, end) {
                        Some(blocker_end) => {
                            candidate = blocker_end.checked_next_multiple_of(GRANULARITY)?;
                        }
                        None => return Some(candidate),
                    }
                }
                None
            }
            Direction::TopDown => {
                let mut candidate = (high - bytes) / GRANULARITY * GRANULARITY;
                for _ in 0..(2 * MAX_RESERVATIONS + 512) {
                    if candidate < low {
                        return None;
                    }
                    let end = candidate + bytes;
                    // The highest reservation starting below `end`.
                    let below = self.entries[..self.len].partition_point(|r| r.start < end);
                    if let Some(r) = below
                        .checked_sub(1)
                        .map(|i| self.entries[i])
                        .filter(|r| r.end() > candidate)
                    {
                        candidate = r.start.checked_sub(bytes)? / GRANULARITY * GRANULARITY;
                        continue;
                    }
                    match busy(candidate, end) {
                        // Top-down only needs the blocker's start: retry below it.
                        Some(_) => candidate = candidate.checked_sub(GRANULARITY)?,
                        None => return Some(candidate),
                    }
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::boxed::Box;
    fn table() -> Box<Reservations> {
        Box::new(Reservations::EMPTY)
    }
    const G: u64 = GRANULARITY;
    #[test]
    fn insert_keeps_order_and_refuses_overlap() {
        let mut t = table();
        t.insert(0x30_0000, 16, 4).unwrap();
        t.insert(0x10_0000, 16, 4).unwrap();
        t.insert(0x20_0000, 256, 4).unwrap();
        let starts: std::vec::Vec<u64> = t.iter().map(|r| r.start).collect();
        assert_eq!(starts, [0x10_0000, 0x20_0000, 0x30_0000]);
        assert_eq!(t.insert(0x20_0000 + G, 1, 4), Err(Error::Overlap));
        assert_eq!(t.insert(0x1F_0000, 32, 4), Err(Error::Overlap));
        assert_eq!(t.insert(0x10_1000, 1, 4), Err(Error::Alignment));
        assert_eq!(t.find(0x20_0FFF).map(|r| r.start), Some(0x20_0000));
        assert_eq!(t.find(0x10_0000 + 16 * 4096), None);
        assert!(t.holding(0x20_0000, 256 * 4096).is_some());
        assert!(t.holding(0x20_0000, 257 * 4096).is_none());
        assert_eq!(t.remove(0x20_1000), Err(Error::NotCommitted));
        assert_eq!(t.remove(0x20_0000).map(|r| r.pages), Ok(256));
        assert_eq!(t.len(), 2);
    }
    #[test]
    fn large_reservations_cost_nothing_extra() {
        // CoreCLR's regions GC reserves 256 GiB up front.
        let mut t = table();
        t.insert(0x100_0000_0000, (256u64 << 30) / 4096, 4).unwrap();
        assert!(t.find(0x100_0000_0000 + (255u64 << 30)).is_some());
    }
    #[test]
    fn free_ranges_skip_reservations_and_busy_regions() {
        let mut t = table();
        t.insert(0x10_0000, 16, 4).unwrap(); // [0x10_0000, 0x11_0000)
        t.insert(0x12_0000, 16, 4).unwrap(); // [0x12_0000, 0x13_0000)
        let none = |_: u64, _: u64| None;
        // The 64 KiB gap at 0x11_0000 fits one granule, not two.
        assert_eq!(
            t.free_range(0x10_0000, 0x100_0000, G, Direction::BottomUp, &none),
            Some(0x11_0000)
        );
        assert_eq!(
            t.free_range(0x10_0000, 0x100_0000, 2 * G, Direction::BottomUp, &none),
            Some(0x13_0000)
        );
        // An image region at [0x13_0000, 0x15_0000) is stepped over.
        let image = |s: u64, e: u64| (s < 0x15_0000 && 0x13_0000 < e).then_some(0x15_0000);
        assert_eq!(
            t.free_range(0x10_0000, 0x100_0000, 2 * G, Direction::BottomUp, &image),
            Some(0x15_0000)
        );
        // Top-down takes the highest fit.
        assert_eq!(
            t.free_range(0x10_0000, 0x20_0000, G, Direction::TopDown, &none),
            Some(0x1F_0000)
        );
        t.insert(0x1F_0000, 16, 4).unwrap();
        assert_eq!(
            t.free_range(0x10_0000, 0x20_0000, G, Direction::TopDown, &none),
            Some(0x1E_0000)
        );
        // Nothing fits.
        assert_eq!(
            t.free_range(0x10_0000, 0x11_0000, 2 * G, Direction::BottomUp, &none),
            None
        );
    }
    #[test]
    fn placeholders_split_and_coalesce_at_page_granularity() {
        let mut t = table();
        t.insert(0x10_0000, 16, 1).unwrap();
        t.at_mut(0x10_0000).unwrap().placeholder = true;
        t.insert(0x20_0000, 16, 4).unwrap();
        // CoreCLR takes image parts off the front of a placeholder.
        t.split(0x10_0000, 3).unwrap();
        t.split(0x10_3000, 5).unwrap();
        let parts: std::vec::Vec<(u64, u64, bool)> = t
            .iter()
            .map(|r| (r.start, r.pages, r.placeholder))
            .collect();
        assert_eq!(
            parts,
            [
                (0x10_0000, 3, true),
                (0x10_3000, 5, true),
                (0x10_8000, 8, true),
                (0x20_0000, 16, false)
            ]
        );
        assert_eq!(t.split(0x10_3000, 5), Err(Error::Alignment));
        assert_eq!(t.split(0x10_4000, 1), Err(Error::NotCommitted));
        // Only contiguous placeholders covering the range exactly merge.
        assert_eq!(t.coalesce(0x10_0000, 0x10_4000), Err(Error::InvalidState));
        t.coalesce(0x10_3000, 0x11_0000).unwrap();
        assert_eq!(
            t.find(0x10_F000).map(|r| (r.start, r.pages)),
            Some((0x10_3000, 13))
        );
        assert_eq!(t.len(), 3);
        t.at_mut(0x20_0000).unwrap().placeholder = false;
        assert_eq!(t.coalesce(0x10_0000, 0x21_0000), Err(Error::InvalidState));
        t.coalesce(0x10_0000, 0x11_0000).unwrap();
        assert_eq!(t.len(), 2);
    }
    #[test]
    fn the_table_has_a_bound() {
        let mut t = table();
        for i in 0..MAX_RESERVATIONS as u64 {
            t.insert(0x1_0000_0000 + i * G, 1, 4).unwrap();
        }
        assert_eq!(t.insert(0x2_0000_0000, 1, 4), Err(Error::MappingLimit));
    }
}

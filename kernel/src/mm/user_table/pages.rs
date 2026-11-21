//! Page-granular entries for Windows dynamic memory (P18 M0b): every
//! committed page of a sparse reservation has its own leaf, carrying its
//! Windows protection in software bits so VirtualQuery and VirtualProtect
//! answer per page, also while the page is not present (PAGE_NOACCESS,
//! PAGE_GUARD).
//!
//! Leaf encoding inside a paged reservation (bits the MMU ignores: 9-11
//! always, 52-58 in present entries and everything in non-present ones):
//! - 0: reserved, not committed;
//! - COMMITTED | frame | protection code | hardware flags: committed. A
//!   page without access (NOACCESS, or GUARD before its first touch) is
//!   not present; the frame stays recorded for decommit.
//! - COMMITTED | DEMAND | protection code: committed with no frame yet
//!   (P18 demand paging, as Windows commits: the commit charge is taken, a
//!   zeroed page arrives on the first touch). Never present.
//!
//! W^X per page (owner decision, 13 November 2026): EXECUTE_READWRITE is
//! never writable and executable at once. It starts writable (NX); an
//! instruction fetch turns it read-execute, a write turns it back
//! (`toggle`), each change retired on every CPU before the access retries.
use super::{ACCESSED_DIRTY, ADDRESS, NX, PRESENT, USER, UserTable, WRITE};
use crate::mm::{Error, PAGE_BYTES, UserAccess};

const COMMITTED: u64 = 1 << 9;
const GUARD: u64 = 1 << 10;
/// Committed by the commit call still in progress: a failed call takes back
/// exactly these pages (`take_fresh`), a successful one clears the bit
/// (`settle`). Any re-encoding (protect, fault toggle) also clears it.
const FRESH: u64 = 1 << 11;
/// A page a section view owns privately (a copy-on-write copy): unmapping
/// the view frees it. Section pages in views are never owned.
const OWNED: u64 = 1 << 55;
/// Committed without a frame: the first touch (`Fault::Demand`) or a kernel
/// access (`materialize`) supplies a zeroed page.
const DEMAND: u64 = 1 << 56;
const CODE_SHIFT: u64 = 52;
const CODE_MASK: u64 = 7 << CODE_SHIFT;

/// A committed page's Windows protection (without modifiers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protection {
    NoAccess = 0,
    ReadOnly = 1,
    ReadWrite = 2,
    Execute = 3,
    ExecuteRead = 4,
    ExecuteReadWrite = 5,
    /// Section views only (PAGE_WRITECOPY, PAGE_EXECUTE_WRITECOPY): the
    /// section's page is read (and executed); the first write copies it.
    WriteCopy = 6,
    ExecuteWriteCopy = 7,
}
impl Protection {
    /// From a Windows PAGE_* value: (protection, guard). WRITECOPY is kept
    /// (section views copy on write; `private` makes it READWRITE for
    /// private memory, as Windows reports it there); NOCACHE and
    /// WRITECOMBINE are refused (no device memory here). PAGE_TARGETS_INVALID
    /// / PAGE_TARGETS_NO_UPDATE (Control Flow Guard, 0x4000_0000) is
    /// accepted and has no effect: no CFG bitmap is kept.
    pub fn from_windows(value: u32) -> Option<(Self, bool)> {
        let value = value & !0x4000_0000;
        let guard = value & 0x100 != 0;
        let p = match value & !0x100 {
            0x01 => Self::NoAccess,
            0x02 => Self::ReadOnly,
            0x04 => Self::ReadWrite,
            0x08 => Self::WriteCopy,
            0x10 => Self::Execute,
            0x20 => Self::ExecuteRead,
            0x40 => Self::ExecuteReadWrite,
            0x80 => Self::ExecuteWriteCopy,
            _ => return None,
        };
        // PAGE_GUARD with PAGE_NOACCESS is invalid on Windows.
        (!(guard && p == Self::NoAccess)).then_some((p, guard))
    }
    /// The protection private memory takes for this one: WRITECOPY is
    /// READWRITE there (nothing is shared to copy).
    pub const fn private(self) -> Self {
        match self {
            Self::WriteCopy => Self::ReadWrite,
            Self::ExecuteWriteCopy => Self::ExecuteReadWrite,
            other => other,
        }
    }
    /// Whether a write (after copying, for WRITECOPY) may follow.
    pub const fn writable(self) -> bool {
        matches!(
            self,
            Self::ReadWrite | Self::ExecuteReadWrite | Self::WriteCopy | Self::ExecuteWriteCopy
        )
    }
    pub const fn executable(self) -> bool {
        matches!(
            self,
            Self::Execute | Self::ExecuteRead | Self::ExecuteReadWrite | Self::ExecuteWriteCopy
        )
    }
    pub const fn windows(self, guard: bool) -> u32 {
        let base = match self {
            Self::NoAccess => 0x01,
            Self::ReadOnly => 0x02,
            Self::ReadWrite => 0x04,
            Self::Execute => 0x10,
            Self::ExecuteRead => 0x20,
            Self::ExecuteReadWrite => 0x40,
            Self::WriteCopy => 0x08,
            Self::ExecuteWriteCopy => 0x80,
        };
        if guard { base | 0x100 } else { base }
    }
    const fn from_code(code: u64) -> Option<Self> {
        Some(match code {
            0 => Self::NoAccess,
            1 => Self::ReadOnly,
            2 => Self::ReadWrite,
            3 => Self::Execute,
            4 => Self::ExecuteRead,
            5 => Self::ExecuteReadWrite,
            6 => Self::WriteCopy,
            7 => Self::ExecuteWriteCopy,
            _ => return None,
        })
    }
    /// Hardware flags of a present page (EXECUTE_READWRITE starts writable).
    const fn hardware(self) -> u64 {
        match self {
            Self::NoAccess => 0,
            Self::ReadOnly | Self::WriteCopy => PRESENT | USER | NX,
            Self::ReadWrite | Self::ExecuteReadWrite => PRESENT | USER | NX | WRITE,
            Self::Execute | Self::ExecuteRead | Self::ExecuteWriteCopy => PRESENT | USER,
        }
    }
}

/// One committed page as recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Page {
    pub physical: u64,
    pub protection: Protection,
    pub guard: bool,
    /// A copy a section view owns (copy on write): freed with the view.
    pub owned: bool,
    /// Committed with no frame yet (`physical` is 0): reads as zeroes.
    pub demand: bool,
}
/// What a user fault on a paged reservation was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// An EXECUTE_READWRITE page changed sides; retry the access.
    Toggled,
    /// The first touch of a guard page: the guard is gone, Windows raises
    /// STATUS_GUARD_PAGE_VIOLATION once.
    Guard,
    /// A write to a WRITECOPY view page: the VM copies it (`copy_page`)
    /// and the access retries.
    CopyOnWrite,
    /// The first touch of a committed page with no frame: the VM supplies a
    /// zeroed page (`materialize`) and the access retries.
    Demand,
    /// A real access violation.
    Violation,
}

fn encode(physical: u64, protection: Protection, guard: bool) -> u64 {
    encode_owned(physical, protection, guard, false)
}
fn encode_owned(physical: u64, protection: Protection, guard: bool, owned: bool) -> u64 {
    let hardware = if guard { 0 } else { protection.hardware() };
    (physical & ADDRESS)
        | COMMITTED
        | if owned { OWNED } else { 0 }
        | if guard { GUARD } else { 0 }
        | ((protection as u64) << CODE_SHIFT)
        | hardware
}
/// A committed page without a frame: never present.
fn encode_demand(protection: Protection, guard: bool) -> u64 {
    COMMITTED | DEMAND | if guard { GUARD } else { 0 } | ((protection as u64) << CODE_SHIFT)
}
/// `page` re-encoded with `protection` and `guard`, keeping its frame (or
/// its lack of one) and ownership.
fn reencode(page: Page, protection: Protection, guard: bool) -> u64 {
    if page.demand {
        encode_demand(protection, guard)
    } else {
        encode_owned(page.physical, protection, guard, page.owned)
    }
}
fn decode(entry: u64) -> Option<Page> {
    if entry & COMMITTED == 0 {
        return None;
    }
    let demand = entry & DEMAND != 0;
    Some(Page {
        physical: if demand { 0 } else { entry & ADDRESS },
        protection: Protection::from_code((entry & CODE_MASK) >> CODE_SHIFT)?,
        guard: entry & GUARD != 0,
        owned: entry & OWNED != 0,
        demand,
    })
}

impl UserTable {
    /// The leaf offset of `address`, creating missing tables when
    /// `create`; None when a table is missing and not created.
    fn page_leaf(&mut self, address: u64, create: bool) -> Result<Option<usize>, Error> {
        let mut table = self.physical;
        for shift in [39, 30, 21] {
            let offset = self.entry_offset(table, (address >> shift) & 511)?;
            let mut entry = self.read(offset);
            if entry == 0 {
                if !create {
                    return Ok(None);
                }
                let (table_physical, _) = self.allocate_table()?;
                entry = table_physical | PRESENT | WRITE | USER;
                self.write(offset, entry);
            } else if entry & (PRESENT | WRITE | USER | NX | 0x180) != PRESENT | WRITE | USER {
                return Err(Error::InvalidTable);
            }
            table = entry & ADDRESS;
        }
        Ok(Some(self.entry_offset(table, (address >> 12) & 511)?))
    }
    /// The leaf offset of `address` without creating tables (None where a
    /// table is missing). Unlike `leaf`, a committed page that is not
    /// present (NOACCESS, guard) is a valid leaf here.
    fn page_lookup(&self, address: u64) -> Result<Option<usize>, Error> {
        let mut table = self.physical;
        for shift in [39, 30, 21] {
            let offset = self.entry_offset(table, (address >> shift) & 511)?;
            let entry = self.read(offset);
            if entry == 0 {
                return Ok(None);
            }
            if entry & (PRESENT | WRITE | USER | NX | 0x180) != PRESENT | WRITE | USER {
                return Err(Error::InvalidTable);
            }
            table = entry & ADDRESS;
        }
        Ok(Some(self.entry_offset(table, (address >> 12) & 511)?))
    }
    /// The committed page at `address`, if any.
    pub(in crate::mm) fn page(&self, address: u64) -> Result<Option<Page>, Error> {
        match self.page_lookup(address)? {
            Some(offset) => Ok(decode(self.read(offset))),
            None => Ok(None),
        }
    }
    /// Commit `physical` (a zeroed page the caller owns) at `address`.
    pub(in crate::mm) fn commit_page(
        &mut self,
        address: u64,
        physical: u64,
        protection: Protection,
        guard: bool,
    ) -> Result<(), Error> {
        let offset = self.page_leaf(address, true)?.ok_or(Error::InvalidTable)?;
        if self.read(offset) != 0 {
            return Err(Error::Overlap);
        }
        self.write(offset, encode(physical, protection, guard) | FRESH);
        (self.retire)(self.physical, address);
        Ok(())
    }
    /// Commit `address` with no frame (demand-zero): the first touch
    /// supplies one. Nothing was mapped, so nothing is retired.
    pub(in crate::mm) fn commit_demand(
        &mut self,
        address: u64,
        protection: Protection,
        guard: bool,
    ) -> Result<(), Error> {
        let offset = self.page_leaf(address, true)?.ok_or(Error::InvalidTable)?;
        if self.read(offset) != 0 {
            return Err(Error::Overlap);
        }
        self.write(offset, encode_demand(protection, guard) | FRESH);
        Ok(())
    }
    /// Give the demand page at `address` its frame `physical` (a zeroed
    /// page the caller owns), with the protection it was committed with.
    pub(in crate::mm) fn materialize(&mut self, address: u64, physical: u64) -> Result<(), Error> {
        let offset = self.page_leaf(address, false)?.ok_or(Error::NotCommitted)?;
        let entry = self.read(offset);
        let page = decode(entry).ok_or(Error::NotCommitted)?;
        if !page.demand {
            return Err(Error::InvalidState);
        }
        self.write(
            offset,
            encode(physical, page.protection, page.guard) | (entry & FRESH),
        );
        // A non-present entry was replaced: no stale translation can exist,
        // but the retire also covers paging-structure caches.
        (self.retire)(self.physical, address);
        Ok(())
    }
    /// The commit call that committed `address` succeeded: keep it.
    pub(in crate::mm) fn settle(&mut self, address: u64) -> Result<(), Error> {
        if let Some(offset) = self.page_lookup(address)? {
            let entry = self.read(offset);
            if entry & FRESH != 0 {
                // A software bit only: no translation changes.
                self.write(offset, entry & !FRESH);
            }
        }
        Ok(())
    }
    /// The commit call failed: if this call committed `address`, remove it
    /// and return its frame (retired on every CPU first).
    pub(in crate::mm) fn take_fresh(&mut self, address: u64) -> Result<Option<u64>, Error> {
        let Some(offset) = self.page_lookup(address)? else {
            return Ok(None);
        };
        let entry = self.read(offset);
        if entry & FRESH == 0 {
            return Ok(None);
        }
        self.write(offset, 0);
        (self.retire)(self.physical, address);
        // A demand page had no frame to give back.
        Ok((entry & DEMAND == 0).then_some(entry & ADDRESS))
    }
    /// The page at `address` and the next address worth looking at: past
    /// the page, or past the whole missing table level (queries over a
    /// mostly-reserved 256 GiB range take a few steps, not 64 million).
    pub(in crate::mm) fn page_or_gap(&self, address: u64) -> Result<(Option<Page>, u64), Error> {
        let mut table = self.physical;
        for shift in [39u64, 30, 21] {
            let offset = self.entry_offset(table, (address >> shift) & 511)?;
            let entry = self.read(offset);
            if entry == 0 {
                let next = ((address >> shift) + 1) << shift;
                return Ok((None, next));
            }
            if entry & (PRESENT | WRITE | USER | NX | 0x180) != PRESENT | WRITE | USER {
                return Err(Error::InvalidTable);
            }
            table = entry & ADDRESS;
        }
        let offset = self.entry_offset(table, (address >> 12) & 511)?;
        Ok((
            decode(self.read(offset)),
            (address & !(PAGE_BYTES - 1)) + PAGE_BYTES,
        ))
    }
    /// Change a committed page's protection; its previous one.
    pub(in crate::mm) fn protect_page(
        &mut self,
        address: u64,
        protection: Protection,
        guard: bool,
    ) -> Result<Page, Error> {
        let offset = self.page_leaf(address, false)?.ok_or(Error::NotCommitted)?;
        let entry = self.read(offset);
        let old = decode(entry).ok_or(Error::NotCommitted)?;
        // A page already copied is private: WRITECOPY is READWRITE there.
        let protection = if old.owned {
            protection.private()
        } else {
            protection
        };
        self.write(offset, reencode(old, protection, guard));
        (self.retire)(self.physical, address);
        Ok(old)
    }
    /// Copy on write: the page at `address` becomes `physical` (the caller
    /// copied the old page into it), owned, writable (READWRITE, or
    /// EXECUTE_READWRITE for EXECUTE_WRITECOPY). The old page, retired on
    /// every CPU before this returns.
    pub(in crate::mm) fn copy_page(&mut self, address: u64, physical: u64) -> Result<Page, Error> {
        let offset = self.page_leaf(address, false)?.ok_or(Error::NotCommitted)?;
        let old = decode(self.read(offset)).ok_or(Error::NotCommitted)?;
        let protection = match old.protection {
            Protection::WriteCopy => Protection::ReadWrite,
            Protection::ExecuteWriteCopy => Protection::ExecuteReadWrite,
            _ => return Err(Error::InvalidState),
        };
        self.write(offset, encode_owned(physical, protection, old.guard, true));
        (self.retire)(self.physical, address);
        Ok(old)
    }
    /// Decommit the page at `address`: the page as it was (its frame, if it
    /// has one, for the caller to free; the TLB entry is retired before this
    /// returns).
    pub(in crate::mm) fn decommit_page(&mut self, address: u64) -> Result<Option<Page>, Error> {
        let Some(offset) = self.page_leaf(address, false)? else {
            return Ok(None);
        };
        let Some(old) = decode(self.read(offset)) else {
            return Ok(None);
        };
        self.write(offset, 0);
        (self.retire)(self.physical, address);
        Ok(Some(old))
    }
    /// A user fault at `address` (`access` as the error code says): flip an
    /// EXECUTE_READWRITE page, consume a guard, or report a violation.
    pub(in crate::mm) fn page_fault(
        &mut self,
        address: u64,
        access: UserAccess,
    ) -> Result<Fault, Error> {
        // The faulting byte's page: TLB retirement takes page addresses.
        let address = address & !(PAGE_BYTES - 1);
        let Some(offset) = self.page_leaf(address, false)? else {
            return Ok(Fault::Violation);
        };
        let entry = self.read(offset);
        let Some(page) = decode(entry) else {
            return Ok(Fault::Violation);
        };
        if page.guard {
            self.write(offset, reencode(page, page.protection, false));
            (self.retire)(self.physical, address);
            return Ok(Fault::Guard);
        }
        if page.demand {
            let permitted = match access {
                UserAccess::Read => page.protection != Protection::NoAccess,
                UserAccess::Write => page.protection.writable(),
                UserAccess::Execute => page.protection.executable(),
            };
            return Ok(if permitted {
                Fault::Demand
            } else {
                Fault::Violation
            });
        }
        if access == UserAccess::Write
            && matches!(
                page.protection,
                Protection::WriteCopy | Protection::ExecuteWriteCopy
            )
        {
            return Ok(Fault::CopyOnWrite);
        }
        if page.protection != Protection::ExecuteReadWrite {
            return Ok(Fault::Violation);
        }
        let keep = entry & (ADDRESS | COMMITTED | CODE_MASK | ACCESSED_DIRTY | OWNED);
        let next = match access {
            // Executing a writable page: make it read-execute.
            UserAccess::Execute if entry & NX != 0 => keep | PRESENT | USER,
            // Writing a read-execute page: make it writable, not executable.
            UserAccess::Write if entry & WRITE == 0 => keep | PRESENT | USER | NX | WRITE,
            _ => return Ok(Fault::Violation),
        };
        self.write(offset, next);
        (self.retire)(self.physical, address);
        Ok(Fault::Toggled)
    }
    /// Unlink page tables left empty in `[start, end)` (after a decommit or
    /// release) so the table pool is not exhausted by address churn.
    pub(in crate::mm) fn reclaim_tables(&mut self, start: u64, end: u64) -> Result<(), Error> {
        let mut address = start & !((1u64 << 21) - 1);
        while address < end {
            self.reclaim_one(address)?;
            address += 1 << 21;
        }
        Ok(())
    }
    fn reclaim_one(&mut self, address: u64) -> Result<(), Error> {
        // The chain of (entry offset in parent, table index) from the root.
        let mut chain = [(0usize, 0usize); 3];
        let mut table = self.physical;
        for (level, shift) in [39u64, 30, 21].into_iter().enumerate() {
            let offset = self.entry_offset(table, (address >> shift) & 511)?;
            let entry = self.read(offset);
            if entry == 0 {
                return Ok(());
            }
            table = entry & ADDRESS;
            chain[level] = (offset, self.index_of(table).ok_or(Error::InvalidTable)?);
        }
        // Bottom-up: free each table that is now entirely empty.
        for &(offset, index) in chain.iter().rev() {
            let empty = (0..512).all(|i| self.read(index * 512 + i) == 0);
            if !empty {
                break;
            }
            self.write(offset, 0);
            // The callback covers paging-structure caches, not just a leaf.
            (self.retire)(self.physical, address);
            self.used.remove(index);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::mm::user_table::TABLE_PAGES;
    use core::ptr::NonNull;
    use std::{boxed::Box, vec};
    fn model() -> (Box<[u64]>, UserTable) {
        let mut pool = vec![0u64; TABLE_PAGES * 512].into_boxed_slice();
        let tree = unsafe {
            UserTable::new(
                NonNull::new(pool.as_mut_ptr()).unwrap(),
                0x200000,
                |_, _| {},
            )
        };
        (pool, tree)
    }
    const A: u64 = 0x1_0000_0000;
    #[test]
    fn windows_protections_round_trip() {
        for value in [
            0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x102, 0x104, 0x140,
        ] {
            let (p, guard) = Protection::from_windows(value).unwrap();
            assert_eq!(p.windows(guard), value);
        }
        assert_eq!(
            Protection::from_windows(0x08).map(|(p, _)| p.private()),
            Some(Protection::ReadWrite)
        );
        assert_eq!(
            Protection::from_windows(0x80).map(|(p, _)| p.private()),
            Some(Protection::ExecuteReadWrite)
        );
        assert_eq!(Protection::from_windows(0x101), None);
        assert_eq!(Protection::from_windows(0x204), None);
    }
    #[test]
    fn committed_pages_carry_their_protection_while_not_present() {
        let (_pool, mut t) = model();
        t.commit_page(A, 0x4000_0000, Protection::NoAccess, false)
            .unwrap();
        assert_eq!(t.translate(A, UserAccess::Read), Err(Error::NotCommitted));
        assert_eq!(
            t.page(A).unwrap(),
            Some(Page {
                physical: 0x4000_0000,
                protection: Protection::NoAccess,
                guard: false,
                owned: false,
                demand: false,
            })
        );
        let old = t.protect_page(A, Protection::ReadOnly, false).unwrap();
        assert_eq!(old.protection, Protection::NoAccess);
        assert_eq!(t.translate(A, UserAccess::Read), Ok(0x4000_0000));
        assert_eq!(t.translate(A, UserAccess::Write), Err(Error::AccessDenied));
        assert_eq!(
            t.commit_page(A, 0x4000_1000, Protection::ReadOnly, false),
            Err(Error::Overlap)
        );
        assert_eq!(
            t.decommit_page(A).map(|p| p.map(|p| p.physical)),
            Ok(Some(0x4000_0000))
        );
        assert_eq!(t.page(A), Ok(None));
    }
    #[test]
    fn demand_pages_fault_in_with_their_protection() {
        let (_pool, mut t) = model();
        t.commit_demand(A, Protection::ReadWrite, false).unwrap();
        // Committed, no frame, not present.
        let page = t.page(A).unwrap().unwrap();
        assert!(page.demand && page.physical == 0);
        assert_eq!(t.translate(A, UserAccess::Read), Err(Error::NotCommitted));
        // A read or write touch asks for a frame; execute is a violation.
        assert_eq!(t.page_fault(A + 8, UserAccess::Write), Ok(Fault::Demand));
        assert_eq!(t.page_fault(A, UserAccess::Execute), Ok(Fault::Violation));
        // Protection changes keep it frameless.
        t.protect_page(A, Protection::ReadOnly, false).unwrap();
        assert!(t.page(A).unwrap().unwrap().demand);
        assert_eq!(t.page_fault(A, UserAccess::Write), Ok(Fault::Violation));
        assert_eq!(t.page_fault(A, UserAccess::Read), Ok(Fault::Demand));
        t.materialize(A, 0x4000_2000).unwrap();
        assert_eq!(t.translate(A, UserAccess::Read), Ok(0x4000_2000));
        assert_eq!(t.translate(A, UserAccess::Write), Err(Error::AccessDenied));
        assert_eq!(t.materialize(A, 0x4000_3000), Err(Error::InvalidState));
        // A guard demand page: the guard fires first, then the frame.
        let b = A + PAGE_BYTES;
        t.commit_demand(b, Protection::ReadWrite, true).unwrap();
        assert_eq!(t.page_fault(b, UserAccess::Read), Ok(Fault::Guard));
        assert_eq!(t.page_fault(b, UserAccess::Read), Ok(Fault::Demand));
        // Decommit reports the frameless page; a failed commit gives none back.
        assert_eq!(t.decommit_page(b).unwrap().map(|p| p.demand), Some(true));
        t.commit_demand(b, Protection::ReadWrite, false).unwrap();
        assert_eq!(t.take_fresh(b), Ok(None));
        assert_eq!(t.page(b), Ok(None));
    }
    #[test]
    fn execute_readwrite_is_never_writable_and_executable_at_once() {
        let (_pool, mut t) = model();
        t.commit_page(A, 0x4000_0000, Protection::ExecuteReadWrite, false)
            .unwrap();
        assert!(t.translate(A, UserAccess::Write).is_ok());
        assert_eq!(
            t.translate(A, UserAccess::Execute),
            Err(Error::AccessDenied)
        );
        assert_eq!(t.page_fault(A, UserAccess::Execute), Ok(Fault::Toggled));
        assert!(t.translate(A, UserAccess::Execute).is_ok());
        assert_eq!(t.translate(A, UserAccess::Write), Err(Error::AccessDenied));
        assert_eq!(t.page_fault(A, UserAccess::Write), Ok(Fault::Toggled));
        assert!(t.translate(A, UserAccess::Write).is_ok());
        assert_eq!(
            t.translate(A, UserAccess::Execute),
            Err(Error::AccessDenied)
        );
        // A read-execute page stays a violation for writes.
        t.commit_page(A + 0x1000, 0x4000_1000, Protection::ExecuteRead, false)
            .unwrap();
        assert_eq!(
            t.page_fault(A + 0x1000, UserAccess::Write),
            Ok(Fault::Violation)
        );
    }
    #[test]
    fn writecopy_pages_read_shared_and_copy_on_the_first_write() {
        let (_pool, mut t) = model();
        t.commit_page(A, 0x4000_0000, Protection::WriteCopy, false)
            .unwrap();
        assert_eq!(t.translate(A, UserAccess::Read), Ok(0x4000_0000));
        assert_eq!(t.translate(A, UserAccess::Write), Err(Error::AccessDenied));
        assert_eq!(t.page_fault(A, UserAccess::Read), Ok(Fault::Violation));
        assert_eq!(t.page_fault(A, UserAccess::Write), Ok(Fault::CopyOnWrite));
        let old = t.copy_page(A, 0x5000_0000).unwrap();
        assert_eq!((old.physical, old.owned), (0x4000_0000, false));
        assert_eq!(t.translate(A, UserAccess::Write), Ok(0x5000_0000));
        let page = t.page(A).unwrap().unwrap();
        assert_eq!((page.protection, page.owned), (Protection::ReadWrite, true));
        // Re-protecting keeps ownership; a copied page is not copied again.
        t.protect_page(A, Protection::ReadOnly, false).unwrap();
        assert!(t.page(A).unwrap().unwrap().owned);
        assert_eq!(t.copy_page(A, 0x6000_0000), Err(Error::InvalidState));
        // EXECUTE_WRITECOPY executes the shared page and copies to RWX.
        t.commit_page(A + 0x1000, 0x4000_1000, Protection::ExecuteWriteCopy, false)
            .unwrap();
        assert!(t.translate(A + 0x1000, UserAccess::Execute).is_ok());
        assert_eq!(
            t.page_fault(A + 0x1000, UserAccess::Write),
            Ok(Fault::CopyOnWrite)
        );
        t.copy_page(A + 0x1000, 0x5000_1000).unwrap();
        assert_eq!(
            t.page(A + 0x1000).unwrap().unwrap().protection,
            Protection::ExecuteReadWrite
        );
    }
    #[test]
    fn a_guard_page_faults_once() {
        let (_pool, mut t) = model();
        t.commit_page(A, 0x4000_0000, Protection::ReadWrite, true)
            .unwrap();
        assert_eq!(t.translate(A, UserAccess::Read), Err(Error::NotCommitted));
        assert_eq!(t.page_fault(A, UserAccess::Read), Ok(Fault::Guard));
        assert!(t.translate(A, UserAccess::Write).is_ok());
        assert_eq!(t.page_fault(A, UserAccess::Read), Ok(Fault::Violation));
        assert_eq!(
            t.page_fault(A + (1 << 30), UserAccess::Read),
            Ok(Fault::Violation)
        );
    }
    #[test]
    fn faults_inside_a_page_retire_the_page() {
        use core::sync::atomic::{AtomicU64, Ordering};
        static RETIRED: AtomicU64 = AtomicU64::new(0);
        let mut pool = vec![0u64; TABLE_PAGES * 512].into_boxed_slice();
        let mut t = unsafe {
            UserTable::new(
                NonNull::new(pool.as_mut_ptr()).unwrap(),
                0x200000,
                |_, address| RETIRED.store(address, Ordering::Relaxed),
            )
        };
        // A guard touched mid-page (a stack growing) and an
        // EXECUTE_READWRITE page written mid-page (a JIT).
        t.commit_page(A, 0x4000_0000, Protection::ReadWrite, true)
            .unwrap();
        assert_eq!(t.page_fault(A + 0x7f8, UserAccess::Write), Ok(Fault::Guard));
        assert_eq!(RETIRED.load(Ordering::Relaxed), A);
        t.commit_page(A + 0x1000, 0x4000_1000, Protection::ExecuteReadWrite, false)
            .unwrap();
        let _ = t.page_fault(A + 0x1123, UserAccess::Execute);
        let _ = t.page_fault(A + 0x1123, UserAccess::Write);
        assert_eq!(RETIRED.load(Ordering::Relaxed), A + 0x1000);
    }
    #[test]
    fn empty_tables_are_reclaimed() {
        let (_pool, mut t) = model();
        let before = t.used_pages();
        t.commit_page(A, 0x4000_0000, Protection::ReadWrite, false)
            .unwrap();
        t.commit_page(A + 0x1000, 0x4000_1000, Protection::ReadWrite, false)
            .unwrap();
        assert_eq!(t.used_pages(), before + 3);
        t.decommit_page(A).unwrap();
        t.reclaim_tables(A, A + 0x2000).unwrap();
        assert_eq!(t.used_pages(), before + 3);
        t.decommit_page(A + 0x1000).unwrap();
        t.reclaim_tables(A, A + 0x2000).unwrap();
        assert_eq!(t.used_pages(), before);
    }
}

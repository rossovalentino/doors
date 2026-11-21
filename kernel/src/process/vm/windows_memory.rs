//! Windows dynamic memory (P18 M0b): VirtualAlloc, VirtualFree,
//! VirtualProtect and VirtualQuery with Windows semantics. Reservations
//! take any free 64 KiB-aligned range of the process's window (no fixed
//! dynamic range, no per-reservation cap); pages are committed, protected
//! and decommitted one by one from the page pool (`ProcessSpace::paged_*`).
//! The native fixed layout (`DYNAMIC_START..USER_SECTION_END`: the door-rt
//! heap, thread pages, stacks, startup block and section views) is never
//! handed out, so native and Windows memory cannot collide.
use super::*;
use door_kernel::mm::{Direction, PoolPages, Protection, Query};
use door_windows_abi::memory::{
    BasicInformation, RELEASE_COALESCE_PLACEHOLDERS, RELEASE_PRESERVE_PLACEHOLDER, RESERVE_IMAGE,
    RESERVE_PLACEHOLDER, RESERVE_REPLACE, RESERVE_TOP_DOWN,
};

/// Windows' lowest allocatable address (the first 64 KiB stay free).
const WINDOWS_LOW: u64 = 0x1_0000;
/// System commit limit for paged Windows memory: the pool's capacity, as on
/// Windows the commit limit is the machine's (no per-process quota).
pub(super) const COMMIT_LIMIT: u64 = mm::PAGE_POOL_BYTES;

fn windows_protection(value: u64) -> Result<(Protection, bool)> {
    u32::try_from(value)
        .ok()
        .and_then(Protection::from_windows)
        .ok_or(Status::InvalidArgument)
}
/// Reservation status: an occupied range is STATUS_CONFLICTING_ADDRESSES
/// (AlreadyExists), as on Windows.
fn reserve_status(error: mm::Error) -> Status {
    match error {
        mm::Error::Overlap | mm::Error::Address => Status::AlreadyExists,
        mm::Error::Exhausted => Status::ResourceExhausted,
        other => status(other),
    }
}
fn basic_information(q: Query) -> BasicInformation {
    BasicInformation {
        base: q.base,
        allocation_base: q.allocation_base,
        allocation_protect: q.allocation_protect,
        size: q.size,
        state: q.state,
        protect: q.protect,
        kind: q.kind,
    }
}
/// What a reservation is made as.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Reserve {
    Private,
    /// A section view (MEM_MAPPED).
    View,
    /// VirtualAlloc2 MEM_RESERVE_PLACEHOLDER.
    Placeholder,
}
impl Reserve {
    fn reserve(
        self,
        space: &mut ProcessSpace,
        address: Option<u64>,
        bytes: u64,
        protect: u32,
        window: (u64, u64),
        direction: Direction,
    ) -> core::result::Result<u64, mm::Error> {
        match self {
            Self::Private => space.paged_reserve(address, bytes, protect, window, direction),
            Self::View => space.paged_reserve_view(address, bytes, protect, window, direction),
            Self::Placeholder => space.paged_reserve_placeholder(address, bytes, window, direction),
        }
    }
}
impl Vm {
    /// The machine-wide commit budget left for `slot`'s pages: the limit
    /// less every other process's pages and the sections'.
    pub(super) fn commit_budget(&self, slot: usize) -> u64 {
        let others: u64 = self
            .paged_bytes
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != slot)
            .map(|(_, b)| *b)
            .sum::<u64>()
            + self.section_bytes();
        COMMIT_LIMIT.saturating_sub(others)
    }
    /// Where reservations of `slot` may go, in bottom-up order: below the
    /// native layout, and (64-bit processes) the rest of the user half.
    /// The DLL catalog's slots (`door_windows_abi::modules`: every system
    /// DLL's fixed base and the private slots after them) are never handed
    /// out, loaded or not, so a LoadLibrary after startup always finds its
    /// DLL's range free.
    fn reservation_windows(&self, slot: usize) -> [(u64, u64); 3] {
        use door_windows_abi::modules;
        if self.x86[slot] {
            [
                (WINDOWS_LOW, modules::PROVIDERS32_BASE),
                (
                    modules::private32_base(modules::PRIVATE_MODULES),
                    DYNAMIC_START,
                ),
                (0, 0),
            ]
        } else {
            [
                (WINDOWS_LOW, DYNAMIC_START),
                (USER_SECTION_END, modules::PROVIDERS[0].base),
                (
                    modules::private_base(modules::PRIVATE_MODULES),
                    mm::USER_END,
                ),
            ]
        }
    }
    /// MEM_RESERVE of `bytes` (page multiple) at `address` (64 KiB aligned)
    /// or, at 0, wherever it fits (`RESERVE_TOP_DOWN`: the highest fit).
    /// `protect` is recorded as the allocation protection.
    pub fn windows_reserve(
        &mut self,
        slot: usize,
        address: u64,
        bytes: u64,
        flags: u64,
        protect: u64,
    ) -> Result<u64> {
        if flags & RESERVE_REPLACE != 0 {
            // VirtualAlloc2 MEM_REPLACE_PLACEHOLDER: the placeholder of
            // exactly this range becomes an ordinary reservation.
            if flags & !RESERVE_REPLACE != 0 {
                return Err(Status::InvalidArgument);
            }
            windows_protection(protect)?;
            self.spaces[slot]
                .as_mut()
                .ok_or(Status::NotFound)?
                .paged_replace_placeholder(address, bytes, protect as u32, mm::mem::PRIVATE)
                .map_err(|_| Status::InvalidArgument)?;
            return Ok(address);
        }
        let how = if flags & RESERVE_PLACEHOLDER != 0 {
            if protect != u64::from(door_windows_abi::memory::page::NOACCESS) {
                return Err(Status::InvalidArgument);
            }
            Reserve::Placeholder
        } else {
            Reserve::Private
        };
        let base = self.reserve_range(
            slot,
            address,
            bytes,
            flags & !RESERVE_PLACEHOLDER,
            protect,
            how,
        )?;
        if flags & RESERVE_IMAGE != 0 {
            self.spaces[slot]
                .as_mut()
                .ok_or(Status::NotFound)?
                .paged_mark_image(base)
                .map_err(status)?;
        }
        Ok(base)
    }
    /// A reservation of `how` kind placed at `address` or by `flags`.
    pub(super) fn reserve_range(
        &mut self,
        slot: usize,
        address: u64,
        bytes: u64,
        flags: u64,
        protect: u64,
        how: Reserve,
    ) -> Result<u64> {
        if flags & !(RESERVE_TOP_DOWN | RESERVE_IMAGE) != 0 {
            return Err(Status::InvalidArgument);
        }
        windows_protection(protect)?;
        let windows = self.reservation_windows(slot);
        let space = self.spaces[slot].as_mut().ok_or(Status::NotFound)?;
        let protect = protect as u32;
        if address != 0 {
            let end = address.checked_add(bytes).ok_or(Status::InvalidArgument)?;
            if !windows
                .iter()
                .any(|&(low, high)| address >= low && end <= high)
            {
                return Err(Status::AlreadyExists);
            }
            return how
                .reserve(
                    space,
                    Some(address),
                    bytes,
                    protect,
                    (0, 0),
                    Direction::BottomUp,
                )
                .map_err(reserve_status);
        }
        let (direction, order) = if flags & RESERVE_TOP_DOWN != 0 {
            (Direction::TopDown, [windows[2], windows[1], windows[0]])
        } else {
            (Direction::BottomUp, windows)
        };
        for window in order {
            if window.1 <= window.0 {
                continue;
            }
            match how.reserve(space, None, bytes, protect, window, direction) {
                Ok(base) => return Ok(base),
                Err(mm::Error::Exhausted) => continue,
                Err(error) => return Err(reserve_status(error)),
            }
        }
        Err(Status::ResourceExhausted)
    }
    /// MEM_COMMIT of reserved pages with a PAGE_* protection; pages already
    /// committed keep their contents and take the new protection.
    pub unsafe fn windows_commit(
        &mut self,
        slot: usize,
        address: u64,
        bytes: u64,
        protect: u64,
    ) -> Result<u64> {
        if self.in_view(slot, address) {
            return self.view_commit(slot, address, bytes, protect);
        }
        let (protection, guard) = windows_protection(protect)?;
        let protection = protection.private();
        // The machine-wide limit: every process's paged pages together.
        let budget = self.commit_budget(slot);
        self.ensure_tables(slot, address, bytes);
        let space = self.spaces[slot].as_mut().ok_or(Status::NotFound)?;
        let mut pages = PoolPages {
            pool: &mut self.pool,
            frames: &mut self.frames,
            kernel: &mut self.tables,
        };
        unsafe {
            space.paged_commit(
                address,
                bytes,
                protection,
                guard,
                // As Windows commits: frames on first touch.
                true,
                &mut pages,
                &mut self.paged_bytes[slot],
                budget,
            )
        }
        .map_err(|error| match error {
            mm::Error::Address => Status::BadAddress,
            other => status(other),
        })?;
        Ok(address)
    }
    /// MEM_DECOMMIT; `bytes` 0 decommits from `address` (the reservation's
    /// base) to the reservation's end.
    pub unsafe fn windows_decommit(&mut self, slot: usize, address: u64, bytes: u64) -> Result<()> {
        if self.in_view(slot, address) {
            // A view's pages belong to its section.
            return Err(Status::InvalidArgument);
        }
        let space = self.spaces[slot].as_mut().ok_or(Status::NotFound)?;
        let bytes = if bytes == 0 {
            let r = space
                .paged_reservation(address)
                .filter(|r| r.start == address)
                .ok_or(Status::InvalidArgument)?;
            r.pages * PAGE_BYTES
        } else {
            bytes
        };
        let mut pages = PoolPages {
            pool: &mut self.pool,
            frames: &mut self.frames,
            kernel: &mut self.tables,
        };
        unsafe { space.paged_decommit(address, bytes, &mut pages, &mut self.paged_bytes[slot]) }
            .map_err(status)
    }
    /// MEM_RELEASE of the reservation based at `address`. With
    /// `RELEASE_PRESERVE_PLACEHOLDER` in `flags`: `[address, address +
    /// bytes)` inside a placeholder is split off as its own placeholder, or
    /// the allocation of exactly that range becomes a placeholder again;
    /// with `RELEASE_COALESCE_PLACEHOLDERS` the placeholders covering
    /// exactly that range merge.
    pub unsafe fn windows_release(
        &mut self,
        slot: usize,
        address: u64,
        bytes: u64,
        flags: u64,
    ) -> Result<()> {
        if self.in_view(slot, address) {
            return Err(Status::InvalidArgument);
        }
        let space = self.spaces[slot].as_mut().ok_or(Status::NotFound)?;
        let mut pages = PoolPages {
            pool: &mut self.pool,
            frames: &mut self.frames,
            kernel: &mut self.tables,
        };
        let result = match flags {
            0 if bytes == 0 => unsafe {
                space.paged_release(address, &mut pages, &mut self.paged_bytes[slot])
            },
            RELEASE_PRESERVE_PLACEHOLDER => {
                if space
                    .paged_reservation(address)
                    .is_some_and(|r| r.placeholder)
                {
                    space.paged_split_placeholder(address, bytes)
                } else {
                    unsafe {
                        space.paged_to_placeholder(
                            address,
                            bytes,
                            &mut pages,
                            &mut self.paged_bytes[slot],
                        )
                    }
                }
            }
            RELEASE_COALESCE_PLACEHOLDERS => space.paged_coalesce_placeholders(address, bytes),
            _ => return Err(Status::InvalidArgument),
        };
        result.map_err(|error| match error {
            mm::Error::Address | mm::Error::InvalidState | mm::Error::Alignment => {
                Status::InvalidArgument
            }
            other => status(other),
        })
    }
    /// VirtualProtect on committed pages; the first page's old protection.
    pub fn windows_protect(
        &mut self,
        slot: usize,
        address: u64,
        bytes: u64,
        protect: u64,
    ) -> Result<u64> {
        if self.in_view(slot, address) {
            return self.view_protect(slot, address, bytes, protect);
        }
        let (protection, guard) = windows_protection(protect)?;
        self.spaces[slot]
            .as_mut()
            .ok_or(Status::NotFound)?
            .paged_protect(address, bytes, protection.private(), guard)
            .map(u64::from)
            .map_err(|error| match error {
                // Windows: ERROR_INVALID_ADDRESS for uncommitted pages.
                mm::Error::NotCommitted | mm::Error::Address => Status::BadAddress,
                other => status(other),
            })
    }
    /// VirtualQuery at `address`.
    pub fn windows_query(&self, slot: usize, address: u64) -> Result<BasicInformation> {
        self.space(slot)?
            .query(address)
            .map(basic_information)
            .map_err(|_| Status::InvalidArgument)
    }
    /// A user page fault the paged memory may own: an EXECUTE_READWRITE
    /// page changing sides or a WRITECOPY view page copied (the access
    /// retries: `Toggled`), a guard page's one-shot fault, or a violation
    /// for the caller's ordinary path (also when no page is left to copy
    /// into: the commit limit is reached).
    pub fn windows_fault(
        &mut self,
        slot: usize,
        address: u64,
        access: UserAccess,
    ) -> door_kernel::mm::Fault {
        use door_kernel::mm::Fault;
        let budget = self.commit_budget(slot);
        let Some(space) = self.spaces.get_mut(slot).and_then(Option::as_mut) else {
            return Fault::Violation;
        };
        match space.paged_fault(address, access) {
            Ok(Fault::CopyOnWrite) => {
                let mut memory = PoolPages {
                    pool: &mut self.pool,
                    frames: &mut self.frames,
                    kernel: &mut self.tables,
                };
                match unsafe {
                    space.paged_copy_on_write(
                        address,
                        &mut memory,
                        &mut self.paged_bytes[slot],
                        budget,
                    )
                } {
                    Ok(()) => Fault::Toggled,
                    Err(_) => Fault::Violation,
                }
            }
            // The first touch of a demand-zero page: its frame now (the
            // commit charge was taken at commit), and the access retries.
            Ok(Fault::Demand) => {
                let mut pages = PoolPages {
                    pool: &mut self.pool,
                    frames: &mut self.frames,
                    kernel: &mut self.tables,
                };
                let page = address & !(mm::PAGE_BYTES - 1);
                match unsafe { space.paged_materialize(page, mm::PAGE_BYTES, &mut pages) } {
                    Ok(()) => Fault::Toggled,
                    Err(_) => Fault::Violation,
                }
            }
            Ok(fault) => fault,
            Err(_) => Fault::Violation,
        }
    }
    /// Windows' stack growth (P18 M0h). A guard-page fault at `address`
    /// while the thread's stack pointer `rsp` lies in the same private
    /// reservation is the thread's stack growing: the touched guard is
    /// already an ordinary committed page, and the page below becomes the
    /// new guard. At the reservation's last page no guard is left:
    /// STATUS_STACK_OVERFLOW. Any other guard page is the program's own.
    pub fn windows_stack_guard(&mut self, slot: usize, address: u64, rsp: u64) -> StackGrowth {
        let Some(space) = self.spaces.get(slot).and_then(Option::as_ref) else {
            return StackGrowth::NotStack;
        };
        let page = address & !(PAGE_BYTES - 1);
        let (Ok(at), Ok(stack)) = (space.query(page), space.query(rsp & !(PAGE_BYTES - 1))) else {
            return StackGrowth::NotStack;
        };
        if at.kind != mm::mem::PRIVATE || stack.allocation_base != at.allocation_base {
            return StackGrowth::NotStack;
        }
        if page < at.allocation_base + PAGE_BYTES {
            return StackGrowth::Overflow;
        }
        let below = page - PAGE_BYTES;
        match space.query(below) {
            Ok(q) if q.state == mm::mem::RESERVE && q.allocation_base == at.allocation_base => {}
            // Already committed below: the consumed guard was all there was.
            _ => return StackGrowth::Grown(page),
        }
        // PAGE_READWRITE | PAGE_GUARD.
        match unsafe { self.windows_commit(slot, below, PAGE_BYTES, 0x104) } {
            Ok(_) => StackGrowth::Grown(page),
            Err(_) => StackGrowth::Overflow,
        }
    }
    /// Commits the thread's stack down to `low` (exception delivery writes
    /// its record below the stack pointer): each guard page on the way down
    /// from `rsp` is consumed and replaced one page lower, as touching it
    /// would. The new lowest ordinary page, if the stack grew.
    pub fn windows_stack_reach(&mut self, slot: usize, low: u64, rsp: u64) -> Result<Option<u64>> {
        let mut limit = None;
        let last = low & !(PAGE_BYTES - 1);
        let mut page = rsp & !(PAGE_BYTES - 1);
        loop {
            let q = self
                .spaces
                .get(slot)
                .and_then(Option::as_ref)
                .ok_or(Status::NotFound)?
                .query(page)
                .map_err(|_| Status::BadAddress)?;
            if q.state != mm::mem::COMMIT {
                // Reserved with no guard above it: not a growing stack.
                return Ok(limit);
            }
            if q.protect & 0x100 != 0 {
                unsafe {
                    self.windows_commit(slot, page, PAGE_BYTES, u64::from(q.protect & !0x100))
                }?;
                match self.windows_stack_guard(slot, page, rsp) {
                    StackGrowth::Grown(p) => limit = Some(p),
                    StackGrowth::Overflow => return Err(Status::ResourceExhausted),
                    StackGrowth::NotStack => return Ok(limit),
                }
            }
            if page <= last {
                return Ok(limit);
            }
            page -= PAGE_BYTES;
        }
    }
    /// Paged pages committed by `slot` (diagnostics).
    pub fn windows_committed(&self, slot: usize) -> u64 {
        self.paged_bytes.get(slot).copied().unwrap_or(0)
    }
}

/// What a guard-page fault on a thread's stack did (`windows_stack_guard`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackGrowth {
    /// Not the faulting thread's stack: STATUS_GUARD_PAGE_VIOLATION.
    NotStack,
    /// The stack grew: its lowest ordinary page (the TEB's StackLimit).
    Grown(u64),
    /// No room for another guard: STATUS_STACK_OVERFLOW.
    Overflow,
}

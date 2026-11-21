//! Bounded size-class allocator over an exclusively owned real user mapping.
//! Metadata stays outside allocations. Splitting is transactional; free coalesces
//! neighbors. Exhausting metadata or bytes returns null without changing ownership.
//!
//! Extent metadata scales with the heap: small heaps (the 1 MiB default) keep
//! `INLINE_EXTENTS` records inside the allocator itself; larger heaps get one
//! record per `BYTES_PER_EXTENT` bytes in a table that is a *separate* mapping
//! above the issuable range, with an unmapped guard page between them, so a
//! write running past the last issued byte faults instead of corrupting it.
//!
//! Every extent (free or used) is a record in an address-ordered doubly linked
//! list. Free extents also sit on one of 32 power-of-two size-class lists (with a
//! nonempty bitmap); used extents are found by an open-addressed index keyed by
//! their start offset. Allocation probes at most `PROBE` entries per class before
//! an exhaustive fallback (so a fitting extent is always found if one exists), and
//! free is O(1) plus an index lookup: no operation moves the table. Freeing an
//! unknown pointer (including a double free) or with a different layout than it
//! was allocated with is refused without changing ownership, counted in
//! `HeapStats::rejected_frees`, and panics in debug builds.
use core::{
    alloc::{GlobalAlloc, Layout},
    cell::UnsafeCell,
    mem, ptr, slice,
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
};
use door_abi::{
    Status,
    platform::{HEAP_BASE, HEAP_BYTES, NATIVE_HEAP_MAX_BYTES},
    rights,
};
/// Extent records held inside the allocator itself (enough for 1 MiB heaps).
const INLINE_EXTENTS: usize = 256;
/// Index slots for the inline records (twice the records, a power of two).
const INLINE_SLOTS: usize = 2 * INLINE_EXTENTS;
/// Heap bytes per extent record for larger heaps.
const BYTES_PER_EXTENT: usize = 4096;
const PAGE: usize = 4096;
const NIL: u32 = u32::MAX;
const CLASSES: usize = 32;
/// Entries examined per size class before the exhaustive fallback scan.
const PROBE: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    /// Unused record on the spare stack.
    Spare,
    Free,
    Used,
}
#[derive(Clone, Copy)]
struct Extent {
    start: u32,
    bytes: u32,
    /// Address-order neighbours.
    prev: u32,
    next: u32,
    /// Size-class list neighbours while free; `link_next` chains spares.
    link_prev: u32,
    link_next: u32,
    align_log2: u8,
    state: State,
}
const EMPTY: Extent = Extent {
    start: 0,
    bytes: 0,
    prev: NIL,
    next: NIL,
    link_prev: NIL,
    link_next: NIL,
    align_log2: 0,
    state: State::Spare,
};
/// Extent records for a mapping of `bytes`: the inline table, or one record
/// per `BYTES_PER_EXTENT`.
const fn extent_capacity(bytes: usize) -> usize {
    let scaled = bytes / BYTES_PER_EXTENT;
    if scaled > INLINE_EXTENTS {
        scaled
    } else {
        INLINE_EXTENTS
    }
}
/// How a heap reservation of `bytes` is split. Offsets are from the base.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Plan {
    capacity: usize,
    slots: usize,
    /// Issuable bytes at the base (its own mapping).
    usable: usize,
    /// Separately mapped metadata (`table_bytes == 0` when inline); the page
    /// between `usable` and `table_offset` is never mapped.
    table_offset: usize,
    table_bytes: usize,
}
const fn plan(bytes: usize) -> Plan {
    let capacity = extent_capacity(bytes);
    if capacity == INLINE_EXTENTS {
        return Plan {
            capacity,
            slots: INLINE_SLOTS,
            usable: bytes,
            table_offset: 0,
            table_bytes: 0,
        };
    }
    let slots = (2 * capacity).next_power_of_two();
    let raw = capacity * mem::size_of::<Extent>() + slots * mem::size_of::<u32>();
    let table_bytes = raw.div_ceil(PAGE) * PAGE;
    let table_offset = bytes - table_bytes;
    Plan {
        capacity,
        slots,
        usable: table_offset - PAGE,
        table_offset,
        table_bytes,
    }
}
impl Plan {
    /// Every VM region the heap maps, as (offset, bytes), and how many are
    /// used: the issuable range and, for scaled heaps, the extent table above
    /// the unmapped guard page. They are mapped once at initialization; the
    /// heap never maps more, so allocation patterns cannot grow the count.
    const fn regions(&self) -> ([(usize, usize); 2], usize) {
        let regions = [(0, self.usable), (self.table_offset, self.table_bytes)];
        (regions, if self.table_bytes == 0 { 1 } else { 2 })
    }
}
const fn class(bytes: u32) -> usize {
    (31 - bytes.leading_zeros()) as usize
}
/// Why a free was refused; ownership and metadata are unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FreeError {
    /// Outside the issuable range, or the heap is not initialized.
    Foreign,
    /// Not the start of a live allocation: a double free or a stray pointer.
    NotAllocated,
    /// Size or alignment differs from the allocation's layout.
    WrongLayout,
}
struct Heap {
    base: *mut u8,
    /// Issuable bytes.
    bytes: usize,
    plan: Plan,
    inline: [Extent; INLINE_EXTENTS],
    inline_index: [u32; INLINE_SLOTS],
    /// Separately mapped records and index, or null for the inline arrays.
    table: *mut Extent,
    index: *mut u32,
    slot_bits: u32,
    /// Top of the spare-record stack.
    spare: u32,
    heads: [u32; CLASSES],
    nonempty: u32,
    /// Records holding extents (free or used).
    count: usize,
    used: usize,
    allocations: usize,
    rejected_frees: usize,
}
impl Heap {
    const fn empty() -> Self {
        Self {
            base: ptr::null_mut(),
            bytes: 0,
            plan: plan(0),
            inline: [EMPTY; INLINE_EXTENTS],
            inline_index: [NIL; INLINE_SLOTS],
            table: ptr::null_mut(),
            index: ptr::null_mut(),
            slot_bits: INLINE_SLOTS.trailing_zeros(),
            spare: NIL,
            heads: [NIL; CLASSES],
            nonempty: 0,
            count: 0,
            used: 0,
            allocations: 0,
            rejected_frees: 0,
        }
    }
    fn extents(&self) -> &[Extent] {
        if self.table.is_null() {
            &self.inline
        } else {
            // SAFETY: `initialize` wrote `capacity` records at `table`, a mapping
            // that is never issued and is guarded by the lock.
            unsafe { slice::from_raw_parts(self.table, self.plan.capacity) }
        }
    }
    fn extents_mut(&mut self) -> &mut [Extent] {
        if self.table.is_null() {
            &mut self.inline
        } else {
            // SAFETY: as in `extents`; `&mut self` makes the access exclusive.
            unsafe { slice::from_raw_parts_mut(self.table, self.plan.capacity) }
        }
    }
    fn index(&self) -> &[u32] {
        if self.index.is_null() {
            &self.inline_index
        } else {
            // SAFETY: `initialize` wrote `slots` entries after the records.
            unsafe { slice::from_raw_parts(self.index, self.plan.slots) }
        }
    }
    fn index_mut(&mut self) -> &mut [u32] {
        if self.index.is_null() {
            &mut self.inline_index
        } else {
            // SAFETY: as in `index`; `&mut self` makes the access exclusive.
            unsafe { slice::from_raw_parts_mut(self.index, self.plan.slots) }
        }
    }
    fn e(&self, i: u32) -> &Extent {
        &self.extents()[i as usize]
    }
    fn e_mut(&mut self, i: u32) -> &mut Extent {
        &mut self.extents_mut()[i as usize]
    }
    /// # Safety
    /// `[base, base + plan(bytes).usable)` and, for a scaled plan, the table range
    /// `[base + table_offset, base + bytes)` are exclusively owned and writable.
    /// The guard page between them is never accessed.
    unsafe fn initialize(&mut self, base: *mut u8, bytes: usize) -> Result<(), Status> {
        let plan = plan(bytes);
        if !self.base.is_null()
            || base.is_null()
            || bytes == 0
            || plan.usable > u32::MAX as usize
            || base.addr().checked_add(bytes).is_none()
        {
            return Err(Status::InvalidArgument);
        }
        let (table, index) = if plan.table_bytes == 0 {
            (ptr::null_mut(), ptr::null_mut())
        } else {
            // SAFETY: the table range lies inside the caller's owned reservation;
            // records are 4-aligned and the index follows them.
            unsafe {
                let table = base.add(plan.table_offset).cast::<Extent>();
                (table, table.add(plan.capacity).cast::<u32>())
            }
        };
        self.base = base;
        self.bytes = plan.usable;
        self.plan = plan;
        self.table = table;
        self.index = index;
        self.slot_bits = plan.slots.trailing_zeros();
        // Every record starts on the spare stack in index order.
        let spare = |i: usize| Extent {
            link_next: if i + 1 < plan.capacity {
                i as u32 + 1
            } else {
                NIL
            },
            ..EMPTY
        };
        if table.is_null() {
            for (i, record) in self.inline.iter_mut().enumerate() {
                *record = spare(i);
            }
            self.inline_index.fill(NIL);
        } else {
            for i in 0..plan.capacity {
                // SAFETY: aligned record `i` of the owned table mapping.
                unsafe { table.add(i).write(spare(i)) };
            }
            for i in 0..plan.slots {
                // SAFETY: entry `i` of the owned index after the records.
                unsafe { index.add(i).write(NIL) };
            }
        }
        self.spare = 0;
        self.heads = [NIL; CLASSES];
        self.nonempty = 0;
        self.count = 0;
        self.used = 0;
        self.allocations = 0;
        if plan.usable != 0 {
            let first = self.take_spare();
            *self.e_mut(first) = Extent {
                bytes: plan.usable as u32,
                ..EMPTY
            };
            self.push_free(first);
        }
        Ok(())
    }
    fn take_spare(&mut self) -> u32 {
        let i = self.spare;
        self.spare = self.e(i).link_next;
        self.count += 1;
        i
    }
    fn return_spare(&mut self, i: u32) {
        let next = self.spare;
        *self.e_mut(i) = Extent {
            link_next: next,
            ..EMPTY
        };
        self.spare = i;
        self.count -= 1;
    }
    fn push_free(&mut self, i: u32) {
        let c = class(self.e(i).bytes);
        let head = self.heads[c];
        let e = self.e_mut(i);
        e.state = State::Free;
        e.align_log2 = 0;
        e.link_prev = NIL;
        e.link_next = head;
        if head != NIL {
            self.e_mut(head).link_prev = i;
        }
        self.heads[c] = i;
        self.nonempty |= 1 << c;
    }
    /// Must run before the extent's size changes (the class is derived from it).
    fn unlink_free(&mut self, i: u32) {
        let e = *self.e(i);
        let c = class(e.bytes);
        if e.link_prev == NIL {
            self.heads[c] = e.link_next;
            if e.link_next == NIL {
                self.nonempty &= !(1 << c);
            }
        } else {
            self.e_mut(e.link_prev).link_next = e.link_next;
        }
        if e.link_next != NIL {
            self.e_mut(e.link_next).link_prev = e.link_prev;
        }
    }
    fn home(&self, start: u32) -> usize {
        ((u64::from(start)).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - self.slot_bits)) as usize
    }
    /// The used record starting at `start`. The index stays at most half full.
    fn lookup(&self, start: u32) -> Option<u32> {
        let mask = self.plan.slots - 1;
        let mut at = self.home(start);
        loop {
            let i = self.index()[at];
            if i == NIL {
                return None;
            }
            if self.e(i).start == start {
                return Some(i);
            }
            at = (at + 1) & mask;
        }
    }
    fn index_insert(&mut self, i: u32) {
        let mask = self.plan.slots - 1;
        let mut at = self.home(self.e(i).start);
        while self.index()[at] != NIL {
            at = (at + 1) & mask;
        }
        self.index_mut()[at] = i;
    }
    /// Linear-probing deletion with backward shift (no tombstones).
    fn index_remove(&mut self, i: u32) {
        let mask = self.plan.slots - 1;
        let mut hole = self.home(self.e(i).start);
        while self.index()[hole] != i {
            hole = (hole + 1) & mask;
        }
        let mut at = hole;
        loop {
            at = (at + 1) & mask;
            let j = self.index()[at];
            if j == NIL {
                break;
            }
            let home = self.home(self.e(j).start);
            if (at.wrapping_sub(home) & mask) >= (at.wrapping_sub(hole) & mask) {
                self.index_mut()[hole] = j;
                hole = at;
            }
        }
        self.index_mut()[hole] = NIL;
    }
    /// Aligned start inside free extent `i` and the records a split would add.
    fn fit(&self, i: u32, size: usize, align: usize) -> Option<(usize, usize)> {
        let e = self.e(i);
        let address = self
            .base
            .addr()
            .checked_add(e.start as usize)?
            .checked_add(align - 1)?
            & !(align - 1);
        let start = address - self.base.addr();
        let end = start.checked_add(size)?;
        let old_end = e.start as usize + e.bytes as usize;
        if end > old_end {
            return None;
        }
        let added = usize::from(start > e.start as usize) + usize::from(end < old_end);
        (added <= self.plan.capacity - self.count).then_some((start, added))
    }
    fn allocate(&mut self, layout: Layout) -> *mut u8 {
        let (size, align) = (layout.size(), layout.align());
        if self.base.is_null() || size == 0 || size > self.bytes || align > 1 << 31 {
            return ptr::null_mut();
        }
        let first = class(size as u32);
        // Bounded probe from the smallest class that may fit: past
        // `size + align - 1` every head fits, so this usually ends at a head.
        let mut classes = self.nonempty & (u32::MAX << first);
        while classes != 0 {
            let c = classes.trailing_zeros() as usize;
            classes &= classes - 1;
            let mut i = self.heads[c];
            for _ in 0..PROBE {
                if i == NIL {
                    break;
                }
                if let Some((start, _)) = self.fit(i, size, align) {
                    return self.carve(i, start, size, align);
                }
                i = self.e(i).link_next;
            }
        }
        // Exhaustive fallback: never fail while some free extent fits.
        let mut classes = self.nonempty & (u32::MAX << first);
        while classes != 0 {
            let c = classes.trailing_zeros() as usize;
            classes &= classes - 1;
            let mut i = self.heads[c];
            while i != NIL {
                if let Some((start, _)) = self.fit(i, size, align) {
                    return self.carve(i, start, size, align);
                }
                i = self.e(i).link_next;
            }
        }
        ptr::null_mut()
    }
    /// Issue `[start, start + size)` from free extent `i`; `fit` reserved the
    /// spare records the prefix/suffix need.
    fn carve(&mut self, i: u32, start: usize, size: usize, align: usize) -> *mut u8 {
        let e = *self.e(i);
        self.unlink_free(i);
        let end = start + size;
        let old_end = e.start as usize + e.bytes as usize;
        if start > e.start as usize {
            let p = self.take_spare();
            *self.e_mut(p) = Extent {
                start: e.start,
                bytes: (start - e.start as usize) as u32,
                prev: e.prev,
                next: i,
                ..EMPTY
            };
            if e.prev != NIL {
                self.e_mut(e.prev).next = p;
            }
            self.e_mut(i).prev = p;
            self.push_free(p);
        }
        if end < old_end {
            let s = self.take_spare();
            let next = self.e(i).next;
            *self.e_mut(s) = Extent {
                start: end as u32,
                bytes: (old_end - end) as u32,
                prev: i,
                next,
                ..EMPTY
            };
            if next != NIL {
                self.e_mut(next).prev = s;
            }
            self.e_mut(i).next = s;
            self.push_free(s);
        }
        let record = self.e_mut(i);
        record.start = start as u32;
        record.bytes = size as u32;
        record.state = State::Used;
        record.align_log2 = align.trailing_zeros() as u8;
        record.link_prev = NIL;
        record.link_next = NIL;
        self.index_insert(i);
        self.used += size;
        self.allocations += 1;
        // SAFETY: the checked extent lies exclusively inside the issuable mapping.
        unsafe { self.base.add(start) }
    }
    fn deallocate(&mut self, pointer: *mut u8, layout: Layout) -> Result<(), FreeError> {
        if self.base.is_null() {
            return Err(FreeError::Foreign);
        }
        let offset = pointer
            .addr()
            .checked_sub(self.base.addr())
            .filter(|offset| *offset < self.bytes)
            .ok_or(FreeError::Foreign)?;
        let mut i = self.lookup(offset as u32).ok_or(FreeError::NotAllocated)?;
        let e = *self.e(i);
        if e.bytes as usize != layout.size() || 1usize << e.align_log2 != layout.align() {
            return Err(FreeError::WrongLayout);
        }
        self.index_remove(i);
        self.used -= layout.size();
        self.allocations -= 1;
        if e.prev != NIL && self.e(e.prev).state == State::Free {
            let p = e.prev;
            self.unlink_free(p);
            let merged = self.e(p).bytes + e.bytes;
            let r = self.e_mut(p);
            r.bytes = merged;
            r.next = e.next;
            if e.next != NIL {
                self.e_mut(e.next).prev = p;
            }
            self.return_spare(i);
            i = p;
        }
        let current = *self.e(i);
        if current.next != NIL && self.e(current.next).state == State::Free {
            let n = current.next;
            let ne = *self.e(n);
            self.unlink_free(n);
            let r = self.e_mut(i);
            r.bytes = current.bytes + ne.bytes;
            r.next = ne.next;
            if ne.next != NIL {
                self.e_mut(ne.next).prev = i;
            }
            self.return_spare(n);
        }
        self.push_free(i);
        Ok(())
    }
    fn largest_free(&self) -> usize {
        if self.nonempty == 0 {
            return 0;
        }
        let mut i = self.heads[31 - self.nonempty.leading_zeros() as usize];
        let mut largest = 0;
        while i != NIL {
            largest = largest.max(self.e(i).bytes as usize);
            i = self.e(i).link_next;
        }
        largest
    }
    fn stats(&self) -> HeapStats {
        HeapStats {
            capacity: self.bytes,
            used: self.used,
            allocations: self.allocations,
            rejected_frees: self.rejected_frees,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeapStats {
    pub capacity: usize,
    pub used: usize,
    pub allocations: usize,
    /// Frees refused as double/foreign/wrong-layout (each panics in debug).
    pub rejected_frees: usize,
}
pub struct HeapAllocator {
    lock: AtomicBool,
    state: AtomicU8,
    heap: UnsafeCell<Heap>,
}
// Every metadata access is protected by lock. Issued bytes belong to callers;
// allocator operations never read/write the contents of another live allocation.
unsafe impl Sync for HeapAllocator {}
impl Default for HeapAllocator {
    fn default() -> Self {
        Self::new()
    }
}
impl HeapAllocator {
    pub const fn new() -> Self {
        Self {
            lock: AtomicBool::new(false),
            state: AtomicU8::new(0),
            heap: UnsafeCell::new(Heap::empty()),
        }
    }
    fn with<R>(&self, f: impl FnOnce(&mut Heap) -> R) -> R {
        while self
            .lock
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        let result = f(unsafe { &mut *self.heap.get() });
        self.lock.store(false, Ordering::Release);
        result
    }
    pub fn initialize(&self) -> Result<(), Status> {
        self.initialize_with_bytes(HEAP_BYTES)
    }
    /// Explicit resource profile; the kernel independently enforces the manifest
    /// quota. Scaled heaps map the issuable range and the metadata table as two
    /// regions with an unmapped guard page between them (one page less than
    /// `bytes` is committed).
    pub fn initialize_with_bytes(&self, bytes: u64) -> Result<(), Status> {
        if bytes == 0 || bytes > NATIVE_HEAP_MAX_BYTES || !bytes.is_multiple_of(4096) {
            return Err(Status::InvalidArgument);
        }
        self.state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Status::AlreadyExists)?;
        let plan = plan(bytes as usize);
        let (regions, count) = plan.regions();
        let result = (|| {
            for (i, &(offset, length)) in regions[..count].iter().enumerate() {
                let address = HEAP_BASE + offset as u64;
                // SAFETY: initialization is exclusive; these fixed ranges have
                // never been issued by this allocator. The VM rejects existing
                // reservations. Rollback releases only ranges mapped here.
                let mapped = unsafe {
                    door_sys::vm_allocate(address, length as u64, rights::READ | rights::WRITE)
                };
                if mapped != Ok(address) {
                    unsafe {
                        if let Ok(other) = mapped {
                            let _ = door_sys::vm_release(other, length as u64);
                        }
                        for &(offset, length) in &regions[..i] {
                            let _ = door_sys::vm_release(HEAP_BASE + offset as u64, length as u64);
                        }
                    }
                    return Err(mapped.err().unwrap_or(Status::IoFailure));
                }
            }
            let result =
                self.with(|heap| unsafe { heap.initialize(HEAP_BASE as *mut u8, bytes as usize) });
            if result.is_err() {
                unsafe { Self::release(plan) };
            }
            result
        })();
        self.state
            .store(if result.is_ok() { 2 } else { 0 }, Ordering::Release);
        result
    }
    /// # Safety
    /// No reference or access to either mapping of `plan` survives.
    unsafe fn release(plan: Plan) {
        let (regions, count) = plan.regions();
        for &(offset, length) in &regions[..count] {
            unsafe {
                let _ = door_sys::vm_release(HEAP_BASE + offset as u64, length as u64);
            }
        }
    }
    pub fn stats(&self) -> HeapStats {
        self.with(|heap| heap.stats())
    }
    pub fn largest_free(&self) -> usize {
        self.with(|heap| heap.largest_free())
    }
    /// Locked free with refusal accounting (no panic).
    fn free(&self, pointer: *mut u8, layout: Layout) -> Result<(), FreeError> {
        self.with(|heap| {
            let result = heap.deallocate(pointer, layout);
            if result.is_err() {
                heap.rejected_frees += 1;
            }
            result
        })
    }
    /// # Safety
    /// Every thread is joined and no future allocation or heap access can race
    /// this call. Every allocated object has been destroyed beforehand.
    pub unsafe fn shutdown(&self) -> Result<(), Status> {
        if self.state.load(Ordering::Acquire) != 2 {
            return Err(Status::InvalidArgument);
        }
        self.with(|heap| {
            if heap.allocations != 0 {
                return Err(Status::WouldBlock);
            }
            let (regions, count) = heap.plan.regions();
            for &(offset, length) in &regions[..count] {
                unsafe { door_sys::vm_release(HEAP_BASE + offset as u64, length as u64) }?;
            }
            *heap = Heap::empty();
            self.state.store(0, Ordering::Release);
            Ok(())
        })
    }
}
// SAFETY: locked extent metadata provides disjoint aligned ownership, exact-layout
// deallocation and failure atomicity. User mappings survive until explicit quiescent
// shutdown or process retirement. No allocator path allocates or unwinds itself
// while holding the lock (the debug panic below runs after it is released).
unsafe impl GlobalAlloc for HeapAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if self.state.load(Ordering::Acquire) != 2 {
            return ptr::null_mut();
        }
        self.with(|heap| heap.allocate(layout))
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if let Err(error) = self.free(pointer, layout)
            && cfg!(debug_assertions)
        {
            panic!("heap free refused: {error:?} at {pointer:p} with {layout:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::{collections::BTreeMap, vec::Vec};

    /// Walks every structure and checks it against the others.
    fn check(heap: &Heap) {
        let records = heap.extents();
        let mut at = records
            .iter()
            .position(|e| e.state != State::Spare && e.start == 0)
            .map_or(NIL, |i| i as u32);
        assert_eq!(heap.e(at).prev, NIL);
        let (mut offset, mut walked, mut frees, mut used, mut allocations) = (0usize, 0, 0, 0, 0);
        let mut previous_free = false;
        while at != NIL {
            let e = heap.e(at);
            assert_eq!(e.start as usize, offset, "extents tile the heap");
            assert!(e.bytes != 0);
            match e.state {
                State::Free => {
                    assert!(!previous_free, "free neighbours are coalesced");
                    frees += 1;
                }
                State::Used => {
                    assert_eq!(heap.lookup(e.start), Some(at));
                    used += e.bytes as usize;
                    allocations += 1;
                }
                State::Spare => panic!("spare record in the address list"),
            }
            previous_free = e.state == State::Free;
            if e.next != NIL {
                assert_eq!(heap.e(e.next).prev, at);
            }
            offset += e.bytes as usize;
            walked += 1;
            at = e.next;
        }
        assert_eq!(offset, heap.bytes);
        assert_eq!(walked, heap.count);
        assert_eq!((used, allocations), (heap.used, heap.allocations));
        assert_eq!(
            heap.index().iter().filter(|i| **i != NIL).count(),
            allocations
        );
        let mut listed = 0;
        for c in 0..CLASSES {
            assert_eq!(heap.nonempty & (1 << c) != 0, heap.heads[c] != NIL);
            let (mut i, mut prev) = (heap.heads[c], NIL);
            while i != NIL {
                let e = heap.e(i);
                assert_eq!(
                    (e.state, class(e.bytes), e.link_prev),
                    (State::Free, c, prev)
                );
                listed += 1;
                prev = i;
                i = e.link_next;
            }
        }
        assert_eq!(listed, frees);
        let (mut spares, mut i) = (0, heap.spare);
        while i != NIL {
            assert_eq!(heap.e(i).state, State::Spare);
            spares += 1;
            i = heap.e(i).link_next;
        }
        assert_eq!(spares, heap.plan.capacity - heap.count);
    }
    /// A reservation laid out like the guest's (one host buffer stands in for
    /// both mappings; the guard page is filled with a canary instead). The base
    /// is 4 KiB past a 16 MiB boundary so alignment outcomes are deterministic.
    struct Backing {
        _words: Vec<u64>,
        base: *mut u8,
        plan: Plan,
    }
    const SKEW: usize = 16 << 20;
    impl Backing {
        fn new(bytes: usize) -> (Self, Heap) {
            let mut words = std::vec![0u64; (bytes + 2 * SKEW) / 8];
            let raw = words.as_mut_ptr().cast::<u8>();
            let base = unsafe { raw.add(raw.addr().next_multiple_of(SKEW) - raw.addr() + PAGE) };
            let plan = plan(bytes);
            if plan.table_bytes != 0 {
                unsafe { ptr::write_bytes(base.add(plan.usable), 0xA5, PAGE) };
            }
            let mut heap = Heap::empty();
            unsafe { heap.initialize(base, bytes).unwrap() };
            (
                Self {
                    _words: words,
                    base,
                    plan,
                },
                heap,
            )
        }
        fn guard_intact(&self) -> bool {
            self.plan.table_bytes == 0
                || unsafe { slice::from_raw_parts(self.base.add(self.plan.usable), PAGE) }
                    .iter()
                    .all(|b| *b == 0xA5)
        }
    }
    #[test]
    fn alignment_fragmentation_oom_and_full_reclamation() {
        let mut bytes = [0u8; 65536];
        let mut heap = Heap::empty();
        unsafe {
            heap.initialize(bytes.as_mut_ptr(), bytes.len()).unwrap();
        }
        let mut live = [(ptr::null_mut(), Layout::new::<u8>()); 100];
        for (i, item) in live.iter_mut().enumerate() {
            let layout = Layout::from_size_align(i * 7 + 1, 1usize << (i % 9)).unwrap();
            let p = heap.allocate(layout);
            assert!(!p.is_null());
            assert_eq!(p.addr() % layout.align(), 0);
            unsafe {
                ptr::write_bytes(p, i as u8, layout.size());
            }
            *item = (p, layout);
        }
        check(&heap);
        let before = heap.stats();
        assert!(
            heap.allocate(Layout::from_size_align(65536, 1).unwrap())
                .is_null()
        );
        assert_eq!(heap.stats(), before);
        for parity in 0..2 {
            for (i, (p, l)) in live.iter().enumerate() {
                if i % 2 == parity {
                    assert!(
                        unsafe { core::slice::from_raw_parts(*p, l.size()) }
                            .iter()
                            .all(|b| *b == i as u8)
                    );
                    heap.deallocate(*p, *l).unwrap();
                }
            }
            check(&heap);
        }
        assert_eq!(heap.count, 1);
        assert_eq!(heap.stats().used, 0);
        assert_eq!(heap.largest_free(), 65536);
        let full = heap.allocate(Layout::from_size_align(65536, 1).unwrap());
        assert_eq!(full, bytes.as_mut_ptr());
        heap.deallocate(full, Layout::from_size_align(65536, 1).unwrap())
            .unwrap();
        assert_eq!(
            heap.deallocate(full, Layout::from_size_align(65536, 1).unwrap()),
            Err(FreeError::NotAllocated)
        );
    }
    #[test]
    fn metadata_pressure_and_wrong_layout_are_atomic() {
        let mut bytes = [0u8; 4096];
        let mut heap = Heap::empty();
        unsafe {
            heap.initialize(bytes.as_mut_ptr(), bytes.len()).unwrap();
        }
        let layout = Layout::new::<u8>();
        let mut pointers = [ptr::null_mut(); 255];
        for p in &mut pointers {
            *p = heap.allocate(layout);
            assert!(!p.is_null());
        }
        let before = heap.stats();
        assert!(heap.allocate(layout).is_null());
        assert_eq!(heap.stats(), before);
        assert_eq!(
            heap.deallocate(pointers[0], Layout::new::<u16>()),
            Err(FreeError::WrongLayout)
        );
        assert_eq!(
            heap.deallocate(pointers[0], Layout::from_size_align(1, 8).unwrap()),
            Err(FreeError::WrongLayout)
        );
        assert_eq!(heap.stats(), before);
        check(&heap);
        for p in pointers {
            heap.deallocate(p, layout).unwrap();
        }
        assert_eq!(heap.count, 1);
        check(&heap);
    }
    #[test]
    fn double_free_and_interior_pointers_are_refused_without_change() {
        let (_backing, mut heap) = Backing::new(64 * 1024);
        let layout = Layout::from_size_align(48, 16).unwrap();
        let a = heap.allocate(layout);
        let b = heap.allocate(layout);
        let c = heap.allocate(layout);
        heap.deallocate(b, layout).unwrap();
        let before = heap.stats();
        assert_eq!(heap.deallocate(b, layout), Err(FreeError::NotAllocated));
        assert_eq!(
            heap.deallocate(unsafe { a.add(16) }, layout),
            Err(FreeError::NotAllocated)
        );
        assert_eq!(
            heap.deallocate(ptr::dangling_mut(), layout),
            Err(FreeError::Foreign)
        );
        assert_eq!(heap.stats(), before);
        check(&heap);
        // A reused address is a fresh allocation, not the stale pointer.
        let d = heap.allocate(layout);
        assert_eq!(d, b);
        for p in [a, c, d] {
            heap.deallocate(p, layout).unwrap();
        }
        assert_eq!(heap.deallocate(d, layout), Err(FreeError::NotAllocated));
        check(&heap);
    }
    #[test]
    fn global_free_refusals_are_counted_and_panic_in_debug() {
        let mut words = std::vec![0u64; 8192];
        let allocator = HeapAllocator::new();
        allocator.state.store(2, Ordering::Release);
        allocator
            .with(|heap| unsafe { heap.initialize(words.as_mut_ptr().cast(), 65536) })
            .unwrap();
        let layout = Layout::from_size_align(32, 8).unwrap();
        let p = unsafe { allocator.alloc(layout) };
        assert!(!p.is_null());
        assert_eq!(
            allocator.free(p, Layout::from_size_align(32, 16).unwrap()),
            Err(FreeError::WrongLayout)
        );
        allocator.free(p, layout).unwrap();
        assert_eq!(allocator.free(p, layout), Err(FreeError::NotAllocated));
        let stats = allocator.stats();
        assert_eq!((stats.allocations, stats.rejected_frees), (0, 2));
        let q = unsafe { allocator.alloc(layout) };
        unsafe { allocator.dealloc(q, layout) };
        let double = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| unsafe {
            allocator.dealloc(q, layout)
        }));
        assert_eq!(double.is_err(), cfg!(debug_assertions));
        // The lock was released before the panic: the allocator still works.
        assert_eq!(allocator.stats().rejected_frees, 3);
        assert!(!unsafe { allocator.alloc(layout) }.is_null());
    }
    #[test]
    fn scaled_metadata_is_a_separate_mapping_behind_a_guard_page() {
        for bytes in [(1 << 20) + 4096, 8 << 20, 96 << 20, 256 << 20] {
            let p = plan(bytes);
            assert_eq!(p.capacity, bytes / BYTES_PER_EXTENT);
            assert!(p.slots >= 2 * p.capacity && p.slots.is_power_of_two());
            assert_eq!(p.usable % PAGE, 0);
            assert_eq!(p.table_offset, p.usable + PAGE, "one unmapped guard page");
            assert_eq!(p.table_offset + p.table_bytes, bytes);
            assert!(p.table_bytes >= p.capacity * mem::size_of::<Extent>() + p.slots * 4);
        }
        let small = plan(HEAP_BYTES as usize);
        assert_eq!((small.usable, small.table_bytes), (HEAP_BYTES as usize, 0));
    }
    #[test]
    fn large_alignments_in_the_carved_layout() {
        const BYTES: usize = 8 << 20;
        let (backing, mut heap) = Backing::new(BYTES);
        let usable = backing.plan.usable;
        let base = backing.base.addr();
        let mut live = Vec::new();
        for (i, align) in [4096usize, 1 << 16, 1 << 20, 2 << 20, 64, 1 << 20]
            .into_iter()
            .enumerate()
        {
            let layout = Layout::from_size_align(align / 2 + 3, align).unwrap();
            let p = heap.allocate(layout);
            assert!(!p.is_null(), "align {align}");
            assert_eq!(p.addr() % align, 0);
            assert!(
                p.addr() + layout.size() <= base + usable,
                "never past the issuable range"
            );
            unsafe { ptr::write_bytes(p, i as u8 + 1, layout.size()) };
            live.push((p, layout, i as u8 + 1));
            check(&heap);
        }
        // No 16 MiB boundary lies inside the issuable range: fails atomically.
        let before = heap.stats();
        assert!(
            heap.allocate(Layout::from_size_align(16, SKEW).unwrap())
                .is_null()
        );
        assert_eq!(heap.stats(), before);
        // The last issuable byte is reachable and the next one is the guard.
        for (p, l, _) in live.drain(..) {
            heap.deallocate(p, l).unwrap();
        }
        let whole = Layout::from_size_align(usable, 4096).unwrap();
        let p = heap.allocate(whole);
        assert_eq!(p.addr(), base);
        unsafe { ptr::write_bytes(p, 0x5A, usable) };
        assert!(backing.guard_intact());
        heap.deallocate(p, whole).unwrap();
        assert!(
            heap.allocate(Layout::from_size_align(usable + 1, 1).unwrap())
                .is_null()
        );
        check(&heap);
    }
    #[test]
    fn large_heaps_scale_extent_metadata_outside_the_issuable_range() {
        const BYTES: usize = 8 << 20;
        let (backing, mut heap) = Backing::new(BYTES);
        let capacity = extent_capacity(BYTES);
        assert_eq!(capacity, 2048);
        assert_eq!(heap.stats().capacity, backing.plan.usable);
        let layout = Layout::from_size_align(24, 8).unwrap();
        let mut live = Vec::new();
        for i in 0..2000usize {
            let p = heap.allocate(layout);
            assert!(!p.is_null(), "allocation {i} failed");
            unsafe { ptr::write_bytes(p, i as u8, 24) };
            live.push((p, i as u8));
        }
        assert_eq!(heap.stats().allocations, 2000);
        for (p, _) in live.iter().step_by(2) {
            heap.deallocate(*p, layout).unwrap();
        }
        assert_eq!(heap.count, 2000 + 1);
        check(&heap);
        for (p, tag) in live.iter().skip(1).step_by(2) {
            let bytes = unsafe { core::slice::from_raw_parts(*p, 24) };
            assert!(
                bytes.iter().all(|b| b == tag),
                "allocations never touch metadata"
            );
            heap.deallocate(*p, layout).unwrap();
        }
        assert_eq!(heap.count, 1);
        assert_eq!(heap.stats().used, 0);
        // Exhausting the scaled metadata still fails atomically.
        let one = Layout::new::<u8>();
        let mut n = 0;
        while !heap.allocate(one).is_null() {
            n += 1;
        }
        assert_eq!(n, capacity - 1);
        assert_eq!(heap.count, capacity);
        check(&heap);
        assert!(backing.guard_intact());
    }
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }
    /// 100k random allocations/frees against a model of the live set: no
    /// overlap, alignment, preserved contents, exact accounting, and a null
    /// result only when no gap of the model can hold the request.
    #[test]
    fn random_churn_matches_a_reference_model() {
        const BYTES: usize = 16 << 20;
        let (backing, mut heap) = Backing::new(BYTES);
        let base = backing.base.addr();
        let usable = backing.plan.usable;
        let mut model: BTreeMap<usize, (Layout, u8)> = BTreeMap::new();
        let mut keys: Vec<usize> = Vec::new();
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let (mut nulls, mut used) = (0, 0usize);
        for step in 0..100_000u32 {
            // Alternate filling towards 1200 live and draining towards 100.
            let target = if (step / 10_000) % 2 == 0 { 1200 } else { 100 };
            let bias = if keys.len() < target { 70 } else { 30 };
            if keys.is_empty() || rng.next() % 100 < bias {
                let size = match rng.next() % 100 {
                    0 => 1 + (rng.next() % (8 << 20)) as usize,
                    1..=10 => 1 + (rng.next() % 65536) as usize,
                    11..=40 => 1 + (rng.next() % 2048) as usize,
                    _ => 1 + (rng.next() % 96) as usize,
                };
                let align =
                    1usize << (rng.next() % if rng.next().is_multiple_of(50) { 17 } else { 7 });
                let layout = Layout::from_size_align(size, align).unwrap();
                let p = heap.allocate(layout);
                if p.is_null() {
                    nulls += 1;
                    // Gaps of the model are exactly the heap's free extents.
                    let mut cursor = 0;
                    let fits = |gap_start: usize, gap_end: usize| {
                        let a = (base + gap_start).next_multiple_of(align) - base;
                        a + size <= gap_end
                    };
                    let mut any = false;
                    for (start, (l, _)) in &model {
                        any |= fits(cursor, *start);
                        cursor = start + l.size();
                    }
                    any |= fits(cursor, usable);
                    assert!(
                        !any || heap.count + 2 > heap.plan.capacity,
                        "step {step}: null with a fitting gap"
                    );
                    continue;
                }
                let offset = p.addr() - base;
                assert_eq!(p.addr() % align, 0);
                assert!(offset + size <= usable);
                if let Some((s, (l, _))) = model.range(..offset).next_back() {
                    assert!(s + l.size() <= offset, "overlaps the previous allocation");
                }
                if let Some((s, _)) = model.range(offset..).next() {
                    assert!(offset + size <= *s, "overlaps the next allocation");
                }
                let tag = (step % 251) as u8 + 1;
                unsafe { ptr::write_bytes(p, tag, size) };
                model.insert(offset, (layout, tag));
                keys.push(offset);
                used += size;
            } else {
                let k = keys.swap_remove((rng.next() % keys.len() as u64) as usize);
                let (layout, tag) = model.remove(&k).unwrap();
                let p = (base + k) as *mut u8;
                let bytes = unsafe { slice::from_raw_parts(p, layout.size()) };
                assert!(bytes.iter().all(|b| *b == tag), "contents preserved");
                heap.deallocate(p, layout).unwrap();
                used -= layout.size();
                assert_eq!(heap.deallocate(p, layout), Err(FreeError::NotAllocated));
            }
            assert_eq!((heap.used, heap.allocations), (used, model.len()));
            if step % 5000 == 0 {
                check(&heap);
            }
        }
        check(&heap);
        assert!(backing.guard_intact());
        std::println!("random churn: {nulls} null results, {} live", model.len());
        for (k, (layout, _)) in model {
            heap.deallocate((base + k) as *mut u8, layout).unwrap();
        }
        assert_eq!((heap.count, heap.largest_free()), (1, usable));
    }
    #[test]
    #[ignore = "host timing; run with --release -- --ignored"]
    fn churn_65536_records_timing() {
        const BYTES: usize = 256 << 20;
        let (_backing, mut heap) = Backing::new(BYTES);
        let layout = Layout::from_size_align(64, 16).unwrap();
        let mut kept = Vec::new();
        for _ in 0..65_000usize {
            let p = heap.allocate(layout);
            assert!(!p.is_null());
            kept.push(p);
        }
        let mut live = Vec::new();
        for (i, p) in kept.into_iter().enumerate() {
            if i % 2 == 1 {
                heap.deallocate(p, layout).unwrap();
            } else {
                live.push(p);
            }
        }
        let mut rng = Rng(0x1234_5678_9abc_def0);
        let start = std::time::Instant::now();
        for _ in 0..20_000 {
            let i = (rng.next() as usize) % live.len();
            heap.deallocate(live[i], layout).unwrap();
            let p = heap.allocate(layout);
            assert!(!p.is_null());
            live[i] = p;
        }
        std::println!(
            "churn: {} records, 20000 free+alloc pairs in {:?}",
            heap.count,
            start.elapsed()
        );
    }
    #[test]
    fn small_heaps_keep_the_inline_records() {
        assert_eq!(extent_capacity(64 * 1024), INLINE_EXTENTS);
        assert_eq!(extent_capacity(HEAP_BYTES as usize), INLINE_EXTENTS);
        assert_eq!(extent_capacity(256 << 20), 65_536);
    }
    /// Every kernel mapping a process heap costs is made at initialization, so
    /// Terminal's growth (96 MiB heap: scrollback doubling like `Vec`, a glyph
    /// atlas rebuilt per text size, many small line allocations) never adds
    /// one. Each mapping is also a record in the kernel's shared alias table.
    #[test]
    fn terminal_growth_pattern_needs_no_mapping_beyond_the_initial_two() {
        for bytes in [
            HEAP_BYTES as usize,
            32 << 20,
            96 << 20,
            128 << 20,
            NATIVE_HEAP_MAX_BYTES as usize,
        ] {
            let p = plan(bytes);
            let (regions, count) = p.regions();
            assert_eq!(count, if bytes <= HEAP_BYTES as usize { 1 } else { 2 });
            let mut end = 0;
            for &(offset, length) in &regions[..count] {
                assert!(offset >= end && length != 0 && offset % PAGE == 0 && length % PAGE == 0);
                end = offset + length;
            }
            assert!(end <= bytes);
        }
        const BYTES: usize = 96 << 20;
        let (backing, mut heap) = Backing::new(BYTES);
        let small = |n: usize| Layout::from_size_align(80 + n % 320, 8).unwrap();
        let (mut scrollback, mut atlas) = (None, None);
        let mut lines = Vec::new();
        for round in 0..14 {
            // Scrollback 4 KiB .. 32 MiB: the larger buffer exists before the old is freed.
            let next = Layout::from_size_align(4096 << round, 16).unwrap();
            let p = heap.allocate(next);
            assert!(!p.is_null(), "scrollback growth to {} bytes", next.size());
            if let Some((old, layout)) = scrollback.replace((p, next)) {
                heap.deallocate(old, layout).unwrap();
            }
            // A text-size change rebuilds the atlas (up to 2048 x 2048 x 4).
            let side = 256usize << (round % 4);
            let layout = Layout::from_size_align(side * side * 4, 64).unwrap();
            if let Some((old, layout)) = atlas.take() {
                heap.deallocate(old, layout).unwrap();
            }
            let p = heap.allocate(layout);
            assert!(!p.is_null(), "atlas rebuild at {side}px");
            atlas = Some((p, layout));
            for n in 0..200 {
                let layout = small(round * 200 + n);
                let p = heap.allocate(layout);
                assert!(!p.is_null());
                lines.push((p, layout));
            }
        }
        check(&heap);
        assert_eq!(plan(BYTES).regions().1, 2, "still the initial two mappings");
        for (p, layout) in lines.into_iter().chain(scrollback).chain(atlas) {
            heap.deallocate(p, layout).unwrap();
        }
        check(&heap);
        assert_eq!((heap.used, heap.allocations), (0, 0));
        assert!(backing.guard_intact());
    }
    #[test]
    fn host_initializer_reports_missing_guest_vm() {
        let a = HeapAllocator::new();
        assert_eq!(a.initialize(), Err(Status::Unsupported));
        assert_eq!(a.stats().capacity, 0);
    }
}

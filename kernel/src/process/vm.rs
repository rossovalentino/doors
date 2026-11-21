//! Privileged VM ownership. The process runtime owns the IRQ-safe VM guard and
//! executive leases; neither the scheduler nor executive lock spans shootdowns.
mod windows;
mod windows_memory;
mod windows_sections;
mod windows_tls;
use crate::arch;
use door_abi::{STARTUP_BYTES, Startup, Status, rights};
use door_kernel::{
    mm::{
        self, FrameAllocator, MappedRegion, PAGE_BYTES, PageCount, PagePool, PageTables,
        Permissions, ProcessSpace, RegionId, RegionTable, UserAccess, VirtualAddress,
    },
    process_image::ImagePlan,
};
pub use windows::{MAX_APPLICATION_CHECKPOINTS, WindowsStartup};
pub use windows_memory::StackGrowth;

const MAX_PROCESSES: usize = door_kernel::executive::MAX_PROCESSES;
const MAX_SECTIONS: usize = door_kernel::executive::MAX_SECTIONS;
// Kernel aliases of process memory, all in the shared top PML4 slot
// (0xffff_ff80_0000_0000..): the dynamic heaps (32 x 256 MiB), then the
// per-process arenas (32 x 64 MiB), the sections (32 x 32 MiB), and the
// threads near the top. Laid out for 32 processes on 13 November 2026.
const PROCESS_ARENA: u64 = 0xffff_fffc_0000_0000;
const PROCESS_STRIDE: u64 = 0x0400_0000;
const SECTION_ARENA: u64 = 0xffff_fffe_8000_0000;
const SECTION_STRIDE: u64 = door_abi::graphics::SECTION_MAX_BYTES;
const USER_STACK: u64 = 0x7000_1000;
const USER_STACK_PAGES: u64 = 16;
const SERVICE_STACK_PAGES: u64 = 64;
const USER_STARTUP: u64 = 0x7100_0000;
const USER_TLS: u64 = 0x7100_2000;
const USER_SECTION_START: u64 = 0x7200_0000;
const USER_SECTION_END: u64 = 0x8000_0000;
/// Thread stacks and pages (P18: moved below the page pool for 128 threads).
const THREAD_ARENA: u64 = 0xffff_fff4_0000_0000;
const THREAD_STRIDE: u64 = 0x0020_0000;
const USER_THREADS: u64 = 0x6000_0000;
const DYNAMIC_START: u64 = 0x5000_0000;
const DYNAMIC_END: u64 = DYNAMIC_START + door_abi::platform::NATIVE_HEAP_MAX_BYTES;
const DYNAMIC_ARENA: u64 = 0xffff_fff0_0000_0000;
const DYNAMIC_STRIDE: u64 = DYNAMIC_END - DYNAMIC_START;
/// The page pool's chunk aliases (P18 M0b), between the dynamic and process
/// arenas.
const PAGE_POOL_ARENA: u64 = 0xffff_fff8_0000_0000;
/// Where a process's extra page-table chunks are mapped in its kernel window
/// (P18): above the alias cursor's 32 MiB and the kernel stack.
const TABLE_WINDOW: u64 = 0x0280_0000;
/// Extra page-table chunks (1 MiB, one kernel mapping and one frame
/// allocation each) every process together may hold.
const TABLE_CHUNKS_TOTAL: usize = 512;
const MAX_THREADS: usize = door_kernel::task::MAX_TASKS;
const _: () = {
    assert!(DYNAMIC_ARENA + MAX_PROCESSES as u64 * DYNAMIC_STRIDE <= PAGE_POOL_ARENA);
    assert!(TABLE_WINDOW >= 0x0201_1000 + PAGE_BYTES);
    assert!(TABLE_WINDOW + mm::ProcessSpace::TABLE_WINDOW_BYTES <= PROCESS_STRIDE);
    assert!(PAGE_POOL_ARENA + mm::PAGE_POOL_BYTES <= PROCESS_ARENA);
    assert!(DYNAMIC_ARENA + MAX_PROCESSES as u64 * DYNAMIC_STRIDE <= THREAD_ARENA);
    assert!(THREAD_ARENA + MAX_THREADS as u64 * THREAD_STRIDE <= PAGE_POOL_ARENA);
    assert!(PROCESS_ARENA + MAX_PROCESSES as u64 * PROCESS_STRIDE <= SECTION_ARENA);
    // Below the kernel image, heap and device windows (0xffff_ffff_8000_0000..).
    assert!(SECTION_ARENA + MAX_SECTIONS as u64 * SECTION_STRIDE <= 0xffff_ffff_8000_0000);
    assert!(
        THREAD_ARENA
            .checked_add(MAX_THREADS as u64 * THREAD_STRIDE)
            .is_some()
    );
    assert!(USER_THREADS + MAX_THREADS as u64 * THREAD_STRIDE < USER_STACK);
    assert!(USER_STACK + SERVICE_STACK_PAGES * PAGE_BYTES < USER_STARTUP);
    assert!(DYNAMIC_END <= USER_THREADS);
    assert!(DYNAMIC_ARENA >= 0xffff_ff80_0000_0000);
};
/// Kernel alias records one native process holds: its table pool, kernel
/// stack, user stack, startup block, TLS, image segments, and a scaled
/// `door-rt` heap (the issuable range plus its extent table, mapped
/// separately behind an unmapped guard page).
const PROCESS_RECORDS: usize = 5 + door_kernel::process_image::MAX_SEGMENTS + 2;
/// An extra thread's stack, TLS and kernel stack.
const THREAD_RECORDS: usize = 3;
/// Boot and device mappings: four guarded IST stacks, the early heap, the
/// local APIC, up to six block BARs, their DMA ring and the PE scratch page.
const KERNEL_RECORDS: usize = 16;
/// Records the declared capacity (processes, threads, sections) needs at once.
const KERNEL_ALIASES: usize = MAX_PROCESSES * PROCESS_RECORDS
    + MAX_THREADS * THREAD_RECORDS
    + MAX_SECTIONS
    + KERNEL_RECORDS
    + mm::PAGE_POOL_CHUNKS
    + TABLE_CHUNKS_TOTAL;
// The kernel mapping table and the frame allocator's records each serve every
// process at once (each RAM alias owns one allocation), so both must hold the
// whole declared capacity, not one process.
const _: () = assert!(KERNEL_ALIASES <= mm::MAX_MAPPINGS && KERNEL_ALIASES <= mm::MAX_ALLOCATIONS);
const MAX_STARTUP_CHECKPOINTS: usize = 10 + 2 * door_kernel::process_image::MAX_SEGMENTS;
type Result<T> = core::result::Result<T, Status>;

/// Kernel-only construction budget. Unlimited production uses the same steps;
/// diagnostics exhaust exactly one checkpoint per attempt, never fake success.
struct StartupBudget {
    remaining: Option<usize>,
    injected: bool,
}
impl StartupBudget {
    fn step(&mut self) -> core::result::Result<(), mm::Error> {
        if let Some(remaining) = &mut self.remaining {
            if *remaining == 0 {
                self.injected = true;
                return Err(mm::Error::Exhausted);
            }
            *remaining -= 1;
        }
        Ok(())
    }
}

pub struct UserLayout {
    pub entry: u64,
    pub cr3: u64,
    /// Exclusive top. Native entry uses top - 8; Win64 reserves shadow space
    /// and a return slot at top - 40.
    pub user_stack_top: u64,
    pub kernel_stack_top: u64,
    pub tls: u64,
    pub startup: u64,
}
struct Construction {
    entry: u64,
    next_alias: VirtualAddress,
    stack_pages: u64,
}
struct Section {
    key: u64,
    mapping: MappedRegion,
}
struct ThreadMemory {
    process: usize,
    stack: u64,
    tls: u64,
    kernel: MappedRegion,
}
#[derive(Clone, Copy)]
pub struct VmStats {
    pub processes: usize,
    pub sections: usize,
    pub allocated_pages: u64,
    pub kernel_table_pages: u32,
}
pub struct Vm {
    frames: FrameAllocator<'static>,
    tables: PageTables<'static>,
    spaces: [Option<ProcessSpace>; MAX_PROCESSES],
    /// Each slot's region records (ProcessSpace points at its slot's).
    region_tables: [RegionTable; MAX_PROCESSES],
    sections: [Option<Section>; MAX_SECTIONS],
    kernel_stacks: [Option<MappedRegion>; MAX_PROCESSES],
    threads: [Option<ThreadMemory>; MAX_THREADS],
    dynamic_bytes: [u64; MAX_PROCESSES],
    /// Windows pages committed through the page pool (P18 M0b), per process.
    paged_bytes: [u64; MAX_PROCESSES],
    /// Single pages for paged Windows memory, shared by every process.
    pool: PagePool,
    /// Windows sections (CreateFileMapping) and their views, every process.
    win_sections: [Option<windows_sections::WinSection>; windows_sections::MAX_WIN_SECTIONS],
    win_views: [Option<windows_sections::WinView>; windows_sections::MAX_WIN_VIEWS],
    /// Pool pages the sections hold (data and radix nodes), in bytes.
    section_bytes: u64,
    static_tls: [[Option<windows_tls::StaticTls>; windows_tls::TLS_SLOTS]; MAX_PROCESSES],
    /// A 32-bit (x86) Windows process (P17): its threads' pages use the x86
    /// layout (`door_windows_abi::x86`, the TEB32 as their FS base).
    x86: [bool; MAX_PROCESSES],
}
impl Vm {
    /// Exclusive kernel allocation/mapping access under the process VM guard.
    /// Device owners retain their mapping tokens until DMA is quiesced.
    pub(crate) fn kernel_memory(
        &mut self,
    ) -> (&mut FrameAllocator<'static>, &mut PageTables<'static>) {
        (&mut self.frames, &mut self.tables)
    }

    /// Initialize the final static VM owner field by field. The expanded process
    /// space registry must never be materialized as a whole on the boot stack.
    /// # Safety
    /// `at` is valid, aligned, uninitialized storage exclusively owned by boot.
    /// Publish it only after this function returns; it must be initialized once.
    pub unsafe fn initialize_at(
        at: *mut Self,
        frames: FrameAllocator<'static>,
        tables: PageTables<'static>,
    ) {
        unsafe {
            core::ptr::addr_of_mut!((*at).frames).write(frames);
            core::ptr::addr_of_mut!((*at).tables).write(tables);
            PagePool::initialize_at(core::ptr::addr_of_mut!((*at).pool), PAGE_POOL_ARENA);
            for i in 0..MAX_PROCESSES {
                core::ptr::addr_of_mut!((*at).spaces)
                    .cast::<Option<ProcessSpace>>()
                    .add(i)
                    .write(None);
                RegionTable::initialize_at(
                    core::ptr::addr_of_mut!((*at).region_tables)
                        .cast::<RegionTable>()
                        .add(i),
                );
                core::ptr::addr_of_mut!((*at).kernel_stacks)
                    .cast::<Option<MappedRegion>>()
                    .add(i)
                    .write(None);
                core::ptr::addr_of_mut!((*at).dynamic_bytes)
                    .cast::<u64>()
                    .add(i)
                    .write(0);
                core::ptr::addr_of_mut!((*at).paged_bytes)
                    .cast::<u64>()
                    .add(i)
                    .write(0);
                core::ptr::addr_of_mut!((*at).static_tls)
                    .cast::<[Option<windows_tls::StaticTls>; windows_tls::TLS_SLOTS]>()
                    .add(i)
                    .write([None; windows_tls::TLS_SLOTS]);
                core::ptr::addr_of_mut!((*at).x86)
                    .cast::<bool>()
                    .add(i)
                    .write(false);
            }
            for i in 0..MAX_SECTIONS {
                core::ptr::addr_of_mut!((*at).sections)
                    .cast::<Option<Section>>()
                    .add(i)
                    .write(None);
            }
            for i in 0..MAX_THREADS {
                core::ptr::addr_of_mut!((*at).threads)
                    .cast::<Option<ThreadMemory>>()
                    .add(i)
                    .write(None);
            }
            for i in 0..windows_sections::MAX_WIN_SECTIONS {
                core::ptr::addr_of_mut!((*at).win_sections)
                    .cast::<Option<windows_sections::WinSection>>()
                    .add(i)
                    .write(None);
            }
            for i in 0..windows_sections::MAX_WIN_VIEWS {
                core::ptr::addr_of_mut!((*at).win_views)
                    .cast::<Option<windows_sections::WinView>>()
                    .add(i)
                    .write(None);
            }
            core::ptr::addr_of_mut!((*at).section_bytes).write(0);
        }
    }
    /// # Safety
    /// Sole VM guard is held with IRQs disabled and kernel CR3 active. Other CPUs
    /// remain able to service IPIs. The caller has allocated the process slot but
    /// has not made any context runnable. Startup grants were authorized by the
    /// executive. On error no new process, kernel stack or private frames remain.
    pub unsafe fn create_process(
        &mut self,
        slot: usize,
        image: &[u8],
        startup: &[u8],
        pid: u64,
    ) -> Result<UserLayout> {
        let mut budget = StartupBudget {
            remaining: None,
            injected: false,
        };
        unsafe { self.create_process_with_budget(slot, image, startup, pid, &mut budget) }
    }
    unsafe fn create_process_with_budget(
        &mut self,
        slot: usize,
        image: &[u8],
        startup: &[u8],
        pid: u64,
        budget: &mut StartupBudget,
    ) -> Result<UserLayout> {
        if slot >= MAX_PROCESSES {
            return Err(Status::InvalidArgument);
        }
        if self.spaces[slot].is_some() || self.kernel_stacks[slot].is_some() {
            return Err(Status::AlreadyExists);
        }
        let plan = ImagePlan::parse(image).map_err(|_| Status::InvalidArgument)?;
        let (block, stack_pages) = if startup.len() == STARTUP_BYTES {
            (Startup::decode(startup)?, USER_STACK_PAGES)
        } else {
            (
                door_abi::platform::StartupV2::decode(startup)?.base,
                SERVICE_STACK_PAGES,
            )
        };
        let stack_top = USER_STACK + stack_pages * PAGE_BYTES;
        if block.pid != pid
            || block.entry != plan.entry()
            || block.tls != USER_TLS
            || block.stack_top != stack_top
        {
            return Err(Status::InvalidArgument);
        }
        // Keep the original native image clear of stack/startup/TLS/section
        // policy, including their guard pages, even for an unusual valid ELF.
        if plan
            .segments()
            .any(|s| s.address < 0x0040_0000 || s.address + s.pages * PAGE_BYTES > 0x0140_0000)
        {
            return Err(Status::InvalidArgument);
        }
        unsafe {
            self.construct_process(slot, budget, |this, space, budget| {
                budget.step().map_err(status)?;
                let loaded = {
                    plan.load_with_checkpoints(
                        space,
                        &mut this.frames,
                        &mut this.tables,
                        va(PROCESS_ARENA + slot as u64 * PROCESS_STRIDE + 0x10_0000)?,
                        &mut || budget.step(),
                    )
                }
                .map_err(|error| match error {
                    door_kernel::process_image::Error::Memory(error) => status(error),
                    _ => Status::InvalidArgument,
                })?;
                let mut alias = loaded.next_alias;
                this.commit_common(space, &mut alias, stack_pages, startup, pid, budget)?;
                Ok(Construction {
                    entry: loaded.entry,
                    next_alias: alias,
                    stack_pages,
                })
            })
        }
    }
    /// Build an unpublished space and trusted stack as one ownership transaction.
    /// Native and PE plans use this same rollback and final publication path.
    unsafe fn construct_process(
        &mut self,
        slot: usize,
        budget: &mut StartupBudget,
        build: impl FnOnce(&mut Self, &mut ProcessSpace, &mut StartupBudget) -> Result<Construction>,
    ) -> Result<UserLayout> {
        if slot >= MAX_PROCESSES {
            return Err(Status::InvalidArgument);
        }
        if self.spaces[slot].is_some() || self.kernel_stacks[slot].is_some() {
            return Err(Status::AlreadyExists);
        }
        let arena = PROCESS_ARENA + slot as u64 * PROCESS_STRIDE;
        budget.step().map_err(status)?;
        let regions = core::ptr::NonNull::from(&mut self.region_tables[slot]);
        let mut space = unsafe {
            ProcessSpace::new(
                &mut self.frames,
                &mut self.tables,
                va(arena)?,
                retire_private,
                regions,
            )
        }
        .map_err(status)?;
        let mut kernel_stack = None;
        let result = (|| {
            let built = build(self, &mut space, budget)?;
            let stack_start = arena + 0x0200_1000;
            let stack_end = stack_start + 16 * PAGE_BYTES;
            budget.step().map_err(status)?;
            if built.next_alias.get() >= arena + 0x0200_0000
                || self
                    .tables
                    .is_mapped(va(stack_start - PAGE_BYTES)?)
                    .map_err(status)?
                || self.tables.is_mapped(va(stack_end)?).map_err(status)?
            {
                return Err(Status::AlreadyExists);
            }
            let owned = self
                .frames
                .allocate(PageCount::new(16).map_err(status)?, 1)
                .map_err(status)?;
            let mapping = match unsafe {
                self.tables
                    .map_frames(owned, va(stack_start)?, Permissions::ReadWrite)
            } {
                Ok(mapping) => mapping,
                Err(failure) => {
                    self.frames.free(failure.frames).map_err(status)?;
                    return Err(status(failure.error));
                }
            };
            kernel_stack = Some(mapping);
            budget.step().map_err(status)?;
            Ok(UserLayout {
                entry: built.entry,
                cr3: space.cr3().map_err(status)?,
                user_stack_top: USER_STACK + built.stack_pages * PAGE_BYTES,
                kernel_stack_top: stack_end,
                tls: USER_TLS,
                startup: USER_STARTUP,
            })
        })();
        match result {
            Ok(layout) => {
                self.spaces[slot] = Some(space);
                self.kernel_stacks[slot] = kernel_stack;
                Ok(layout)
            }
            Err(error) => {
                // Paged (image and Windows dynamic) pages first, back to
                // the pool, then the regions and tables.
                let mut pages = mm::PoolPages {
                    pool: &mut self.pool,
                    frames: &mut self.frames,
                    kernel: &mut self.tables,
                };
                unsafe { space.paged_release_all(&mut pages, &mut self.paged_bytes[slot]) }
                    .expect("failed startup paged rollback");
                self.paged_bytes[slot] = 0;
                unsafe { space.destroy(&mut self.frames, &mut self.tables) }
                    .expect("failed startup private rollback");
                if let Some(mapping) = kernel_stack {
                    unsafe { self.release_mapping(mapping) };
                }
                Err(error)
            }
        }
    }
    unsafe fn commit_common(
        &mut self,
        space: &mut ProcessSpace,
        alias: &mut VirtualAddress,
        stack_pages: u64,
        startup: &[u8],
        pid: u64,
        budget: &mut StartupBudget,
    ) -> Result<()> {
        unsafe {
            self.commit_region(
                space,
                alias,
                USER_STACK,
                stack_pages,
                Permissions::ReadWrite,
                &[],
                budget,
            )?;
            self.commit_region(
                space,
                alias,
                USER_STARTUP,
                1,
                Permissions::ReadOnly,
                startup,
                budget,
            )?;
            let mut tls = [0u8; 16];
            tls[..8].copy_from_slice(&USER_TLS.to_le_bytes());
            tls[8..].copy_from_slice(&pid.to_le_bytes());
            self.commit_region(
                space,
                alias,
                USER_TLS,
                1,
                Permissions::ReadWrite,
                &tls,
                budget,
            )?;
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    unsafe fn commit_region(
        &mut self,
        space: &mut ProcessSpace,
        alias: &mut VirtualAddress,
        start: u64,
        pages: u64,
        permissions: Permissions,
        bytes: &[u8],
        budget: &mut StartupBudget,
    ) -> Result<RegionId> {
        budget.step().map_err(status)?;
        let next = alias
            .checked_add(PageCount::new(pages + 1).map_err(status)?.bytes())
            .map_err(status)?;
        let id = space
            .reserve(start, PageCount::new(pages).map_err(status)?)
            .map_err(status)?;
        unsafe {
            space.commit(
                id,
                &mut self.frames,
                &mut self.tables,
                *alias,
                permissions,
                bytes,
            )
        }
        .map_err(status)?;
        *alias = next;
        budget.step().map_err(status)?;
        Ok(id)
    }
    /// Exercise every real startup boundary in an unused process slot. Each
    /// checkpoint is failed once, then one full construction succeeds and is
    /// retired before returning. All attempts use production construction and
    /// teardown. The returned count is the number of observed injected failures.
    ///
    /// # Safety
    /// Same guard/IRQ/startup-authority contract as create_process. This slot is
    /// not visible to scheduling and no context is created or made runnable during
    /// diagnostics. The caller must retain startup grants throughout all attempts.
    pub unsafe fn verify_startup_rollback(
        &mut self,
        slot: usize,
        image: &[u8],
        startup: &[u8; STARTUP_BYTES],
        pid: u64,
    ) -> Result<usize> {
        if slot >= MAX_PROCESSES {
            return Err(Status::InvalidArgument);
        }
        if self.spaces[slot].is_some() || self.kernel_stacks[slot].is_some() {
            return Err(Status::AlreadyExists);
        }
        let baseline_frames = self.frames.stats();
        let baseline = self.stats();
        self.verify_empty_arena(slot)?;
        for limit in 0..=MAX_STARTUP_CHECKPOINTS {
            let mut budget = StartupBudget {
                remaining: Some(limit),
                injected: false,
            };
            let result =
                unsafe { self.create_process_with_budget(slot, image, startup, pid, &mut budget) };
            let succeeded = result.is_ok();
            match result {
                Err(Status::ResourceExhausted) if budget.injected => {}
                Err(error) => return Err(error),
                Ok(layout) => {
                    // Validate the actually published private tree, including
                    // execute, data and immutable startup rights, before teardown.
                    let validation = (|| {
                        self.validate(slot, layout.entry, 1, UserAccess::Execute)?;
                        self.validate(slot, layout.user_stack_top - 8, 8, UserAccess::Write)?;
                        self.validate(slot, layout.tls, 16, UserAccess::Write)?;
                        self.validate(slot, layout.startup, STARTUP_BYTES, UserAccess::Read)?;
                        if budget.injected
                            || self.cr3(slot)? != layout.cr3
                            || self
                                .validate(slot, layout.startup, 1, UserAccess::Write)
                                .is_ok()
                            || self.stats().processes != baseline.processes + 1
                            || self.frames.stats().allocated_pages
                                <= baseline_frames.allocated_pages
                        {
                            return Err(Status::IoFailure);
                        }
                        Ok(())
                    })();
                    // No architecture context was ever created; this address
                    // space and kernel stack cannot be running on another CPU.
                    unsafe { self.destroy_process(slot) }?;
                    validation?;
                }
            }
            let current = self.stats();
            if self.frames.stats() != baseline_frames
                || current.processes != baseline.processes
                || current.sections != baseline.sections
                || self.spaces[slot].is_some()
                || self.kernel_stacks[slot].is_some()
            {
                return Err(Status::IoFailure);
            }
            // Inspect every page in the bounded 64 MiB arena, not only metadata
            // tokens. Retained intermediate kernel tables must have no leaves.
            self.verify_empty_arena(slot)?;
            if succeeded {
                if limit == 0 {
                    return Err(Status::IoFailure);
                }
                return Ok(limit);
            }
        }
        Err(Status::IoFailure)
    }
    fn verify_empty_arena(&self, slot: usize) -> Result<()> {
        let base = PROCESS_ARENA + slot as u64 * PROCESS_STRIDE;
        for page in 0..PROCESS_STRIDE / PAGE_BYTES {
            if self
                .tables
                .is_mapped(va(base + page * PAGE_BYTES)?)
                .map_err(status)?
            {
                return Err(Status::IoFailure);
            }
        }
        Ok(())
    }
    /// The user range is mapped for `access` (the kernel is about to copy
    /// from or into it). Demand-zero pages of the range get their frames
    /// first, as a user touch would supply them.
    pub fn validate(
        &mut self,
        slot: usize,
        address: u64,
        bytes: usize,
        access: UserAccess,
    ) -> Result<()> {
        self.materialize(slot, address, bytes)?;
        self.space(slot)?
            .validate_user_range(address, bytes, access)
            .map_err(|_| Status::BadAddress)
    }
    /// `validate` without supplying frames: a demand page fails (image and
    /// start-up checks, whose pages always have frames).
    pub fn check(&self, slot: usize, address: u64, bytes: usize, access: UserAccess) -> Result<()> {
        self.space(slot)?
            .validate_user_range(address, bytes, access)
            .map_err(|_| Status::BadAddress)
    }
    /// Page tables enough for a mapping of `[address, address + bytes)` in
    /// `space` (slot `slot`): one leaf per 2 MiB, one directory per 1 GiB and
    /// one directory pointer table per 512 GiB the range touches, at most;
    /// tables already there leave room over. Best effort: if no chunk can be
    /// added, the mapping itself reports exhaustion.
    pub(super) fn ensure_tables_in(
        frames: &mut FrameAllocator<'static>,
        tables: &mut PageTables<'static>,
        others: usize,
        space: &mut ProcessSpace,
        slot: usize,
        address: u64,
        bytes: u64,
    ) {
        if bytes == 0 {
            return;
        }
        let Some(last) = address.checked_add(bytes - 1) else {
            return;
        };
        let span = |shift: u32| ((last >> shift) - (address >> shift) + 1) as usize;
        let needed = span(21) + span(30) + span(39);
        let Ok(window) = va(PROCESS_ARENA + slot as u64 * PROCESS_STRIDE + TABLE_WINDOW) else {
            return;
        };
        let allowed = TABLE_CHUNKS_TOTAL.saturating_sub(others);
        // SAFETY: the VM guard is held; the window is this slot's own.
        let _ = unsafe { space.ensure_tables(needed, frames, tables, window, allowed) };
    }
    /// Extra page-table chunks every space but `slot` holds.
    pub(super) fn table_chunks_except(&self, slot: usize) -> usize {
        self.spaces
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != slot)
            .filter_map(|(_, s)| s.as_ref())
            .map(ProcessSpace::extra_table_chunks)
            .sum()
    }
    /// `ensure_tables_in` for the published space of `slot`.
    pub(super) fn ensure_tables(&mut self, slot: usize, address: u64, bytes: u64) {
        let others = self.table_chunks_except(slot);
        let Some(space) = self.spaces.get_mut(slot).and_then(Option::as_mut) else {
            return;
        };
        Self::ensure_tables_in(
            &mut self.frames,
            &mut self.tables,
            others,
            space,
            slot,
            address,
            bytes,
        );
    }
    /// Frames for the demand-zero pages of a user range (P18 demand paging).
    fn materialize(&mut self, slot: usize, address: u64, bytes: usize) -> Result<()> {
        let space = self
            .spaces
            .get_mut(slot)
            .and_then(Option::as_mut)
            .ok_or(Status::NotFound)?;
        let mut pages = mm::PoolPages {
            pool: &mut self.pool,
            frames: &mut self.frames,
            kernel: &mut self.tables,
        };
        // SAFETY: the VM guard is held; the pool owns every page involved.
        unsafe { space.paged_materialize(address, bytes as u64, &mut pages) }
            .map_err(|_| Status::BadAddress)
    }
    /// `validate` for a kernel write into user memory: EXECUTE_READWRITE
    /// pages of the range that are executable right now become writable
    /// first (W^X per page; the next instruction fetch flips them back), so
    /// ReadFile into a JIT buffer works as on Windows. WRITECOPY view pages
    /// are copied as a user write would copy them.
    pub fn validate_write(&mut self, slot: usize, address: u64, bytes: usize) -> Result<()> {
        let budget = self.commit_budget(slot);
        let space = self
            .spaces
            .get_mut(slot)
            .and_then(Option::as_mut)
            .ok_or(Status::NotFound)?;
        let mut memory = mm::PoolPages {
            pool: &mut self.pool,
            frames: &mut self.frames,
            kernel: &mut self.tables,
        };
        // SAFETY: the VM guard is held; the pool owns every page involved.
        unsafe {
            space.paged_prepare_write(
                address,
                bytes as u64,
                &mut memory,
                &mut self.paged_bytes[slot],
                budget,
            )
        }
        .map_err(|_| Status::BadAddress)?;
        self.validate(slot, address, bytes, UserAccess::Write)
    }
    pub fn cr3(&self, slot: usize) -> Result<u64> {
        self.space(slot)?.cr3().map_err(status)
    }
    pub fn validate_private_output(
        &mut self,
        slot: usize,
        address: u64,
        bytes: usize,
    ) -> Result<()> {
        let end = address
            .checked_add(bytes as u64)
            .ok_or(Status::BadAddress)?;
        // All writable Section aliases are confined to this arena. Reject any
        // overlap so the raw capture destination cannot alias its producer.
        if address < USER_SECTION_END && end > USER_SECTION_START {
            return Err(Status::AccessDenied);
        }
        self.validate_write(slot, address, bytes)
    }
    fn space(&self, slot: usize) -> Result<&ProcessSpace> {
        self.spaces
            .get(slot)
            .and_then(Option::as_ref)
            .ok_or(Status::NotFound)
    }
    pub fn monitor_memory(&self) -> (u64, u64, [u64; MAX_PROCESSES]) {
        let stats = self.frames.stats();
        (
            stats.managed_pages,
            stats.allocated_pages,
            core::array::from_fn(|i| {
                self.spaces[i]
                    .as_ref()
                    .map_or(0, |space| space.private_bytes() + self.windows_committed(i))
            }),
        )
    }
    /// At shutdown, after every process is gone: the page pool's empty
    /// chunks back to the frame allocator (it keeps one for reuse while
    /// running).
    pub fn release_idle_pool(&mut self) -> Result<usize> {
        // SAFETY: the VM guard is held (`with_vm`); no process maps a pool
        // page once every process is gone.
        unsafe { self.pool.release_empty(&mut self.frames, &mut self.tables) }.map_err(status)
    }
    pub fn stats(&self) -> VmStats {
        VmStats {
            processes: self.spaces.iter().flatten().count(),
            sections: self.sections.iter().flatten().count(),
            allocated_pages: self.frames.stats().allocated_pages,
            kernel_table_pages: self.tables.used_table_pages(),
        }
    }
    /// Occupancy of every bounded kernel VM resource, ending the line.
    fn write_occupancy(&self, out: &mut impl core::fmt::Write) {
        let _ = writeln!(
            out,
            "processes={} threads={} sections={} mappings={} tables={} free_pages={}",
            self.spaces.iter().flatten().count(),
            self.threads.iter().flatten().count(),
            self.sections.iter().flatten().count(),
            self.tables.used_mappings(),
            self.tables.used_table_pages(),
            self.frames.stats().free_pages,
        );
    }
    /// Serial report of a launch stage refused for want of a kernel resource.
    pub fn report_exhaustion(&self, stage: &str) {
        let mut serial = unsafe { door_boot_debug::Serial::new() };
        let _ = core::fmt::Write::write_fmt(
            &mut serial,
            format_args!("DOOR_P12 LAUNCH_EXHAUSTED stage={stage} "),
        );
        self.write_occupancy(&mut serial);
    }
    /// The main thread's native page (a 32-bit process's implicit TLS array
    /// and blocks live in it).
    pub const fn main_tls() -> u64 {
        USER_TLS
    }
    /// Whether process `slot` is a 32-bit (x86) Windows process.
    pub fn windows_x86(&self, slot: usize) -> bool {
        self.x86.get(slot).copied().unwrap_or(false)
    }
    pub const fn service_stack_top() -> u64 {
        USER_STACK + SERVICE_STACK_PAGES * PAGE_BYTES
    }
    /// # Safety
    /// The VM guard pins this process. Caller reserves the global thread slot
    /// before entry; no context can execute it until this fully succeeds.
    pub unsafe fn create_thread(
        &mut self,
        process: usize,
        slot: usize,
        entry: u64,
        pid: u64,
        key: u64,
        windows: bool,
    ) -> Result<UserLayout> {
        if slot >= MAX_THREADS || self.threads[slot].is_some() {
            return Err(Status::AlreadyExists);
        }
        self.validate(process, entry, 1, UserAccess::Execute)?;
        let stack = USER_THREADS + slot as u64 * THREAD_STRIDE + PAGE_BYTES;
        let tls = USER_THREADS + slot as u64 * THREAD_STRIDE + 0x10_0000;
        let alias = THREAD_ARENA + slot as u64 * THREAD_STRIDE;
        let space = self.spaces[process].as_mut().ok_or(Status::NotFound)?;
        let mut regions = [None; 2];
        let mut kernel = None;
        let outcome = (|| {
            // One page (64-bit threads), or two: a 32-bit thread's TEB32 is
            // a page of its own.
            let mut tls_bytes = [0u8; 8192];
            let tls_pages = if windows && self.x86[process] {
                door_windows_abi::x86::THREAD_PAGES
            } else {
                1
            };
            tls_bytes[..8].copy_from_slice(&tls.to_le_bytes());
            tls_bytes[8..16].copy_from_slice(&pid.to_le_bytes());
            tls_bytes[16..24].copy_from_slice(&key.to_le_bytes());
            if windows && self.x86[process] {
                // A 32-bit thread: its TEB32 (the FS base) in the page.
                use door_windows_abi::x86;
                let low = |value: u64| u32::try_from(value).map_err(|_| Status::InvalidArgument);
                door_windows_abi::x86::initial_teb(
                    &mut tls_bytes[x86::PAGE_TEB..x86::PAGE_TEB + x86::TEB_BYTES],
                    low(tls + x86::PAGE_TEB as u64)?,
                    low(stack + SERVICE_STACK_PAGES * PAGE_BYTES)?,
                    low(stack)?,
                    0,
                    0,
                    low(tls + x86::PAGE_TLS_ARRAY as u64)?,
                    low(door_windows_abi::PEB_ADDRESS)?,
                )
                .map_err(|_| Status::InvalidArgument)?;
                let first = <&mut [u8; 4096]>::try_from(&mut tls_bytes[..4096])
                    .map_err(|_| Status::InvalidArgument)?;
                windows_tls::fill_tls32(first, tls, &self.static_tls[process]);
            } else if windows {
                let offset = door_windows_abi::THREAD_TEB_FS_OFFSET;
                door_windows_abi::initial_thread_teb(
                    &mut tls_bytes[offset..],
                    tls + offset as u64,
                    stack + SERVICE_STACK_PAGES * PAGE_BYTES,
                    stack,
                )
                .map_err(|_| Status::InvalidArgument)?;
                let first = <&mut [u8; 4096]>::try_from(&mut tls_bytes[..4096])
                    .map_err(|_| Status::InvalidArgument)?;
                windows_tls::fill_tls(first, tls, &self.static_tls[process]);
            }
            for (i, (address, pages, offset, initial)) in [
                (stack, SERVICE_STACK_PAGES, 0, &[][..]),
                (
                    tls,
                    tls_pages as u64,
                    0x10_0000,
                    &tls_bytes[..tls_pages * 4096],
                ),
            ]
            .into_iter()
            .enumerate()
            {
                let id = space
                    .reserve(address, PageCount::new(pages).map_err(status)?)
                    .map_err(status)?;
                regions[i] = Some(id);
                unsafe {
                    space.commit(
                        id,
                        &mut self.frames,
                        &mut self.tables,
                        va(alias + offset)?,
                        Permissions::ReadWrite,
                        initial,
                    )
                }
                .map_err(status)?;
            }
            let kernel_start = alias + 0x18_1000;
            let kernel_end = kernel_start + 16 * PAGE_BYTES;
            if self
                .tables
                .is_mapped(va(kernel_start - PAGE_BYTES)?)
                .map_err(status)?
                || self.tables.is_mapped(va(kernel_end)?).map_err(status)?
            {
                return Err(Status::AlreadyExists);
            }
            let owned = self
                .frames
                .allocate(PageCount::new(16).map_err(status)?, 1)
                .map_err(status)?;
            let mapping = match unsafe {
                self.tables
                    .map_frames(owned, va(kernel_start)?, Permissions::ReadWrite)
            } {
                Ok(mapping) => mapping,
                Err(failure) => {
                    self.frames.free(failure.frames).map_err(status)?;
                    return Err(status(failure.error));
                }
            };
            kernel = Some(mapping);
            Ok(UserLayout {
                entry,
                cr3: space.cr3().map_err(status)?,
                user_stack_top: stack + SERVICE_STACK_PAGES * PAGE_BYTES,
                kernel_stack_top: kernel_end,
                tls,
                startup: 0,
            })
        })();
        match outcome {
            Ok(layout) => {
                self.threads[slot] = Some(ThreadMemory {
                    process,
                    stack,
                    tls,
                    kernel: kernel.unwrap(),
                });
                Ok(layout)
            }
            Err(error) => {
                for id in regions.into_iter().flatten() {
                    if space.get(id).map_err(status)?.physical.is_some() {
                        unsafe { space.decommit(id, &mut self.frames, &mut self.tables) }
                            .expect("thread creation backing rollback");
                    }
                    space
                        .release(id)
                        .expect("thread creation reservation rollback");
                }
                if let Some(mapping) = kernel {
                    unsafe { self.release_mapping(mapping) };
                }
                Err(error)
            }
        }
    }
    /// # Safety
    /// Architecture retirement proved this thread no longer uses its trusted or
    /// user stack. Caller holds the sole VM guard and keeps the process alive.
    pub unsafe fn destroy_thread(&mut self, slot: usize) -> Result<()> {
        let memory = self
            .threads
            .get(slot)
            .and_then(Option::as_ref)
            .ok_or(Status::NotFound)?;
        let space = self.spaces[memory.process]
            .as_mut()
            .ok_or(Status::NotFound)?;
        for address in [memory.stack, memory.tls] {
            let id = space.region_at(address).ok_or(Status::NotFound)?;
            unsafe { space.decommit(id, &mut self.frames, &mut self.tables) }.map_err(status)?;
            space.release(id).map_err(status)?;
        }
        let memory = self.threads[slot].take().unwrap();
        unsafe { self.release_mapping(memory.kernel) };
        Ok(())
    }
    pub fn vacant_thread(&self) -> Result<usize> {
        self.threads
            .iter()
            .position(Option::is_none)
            .ok_or(Status::ResourceExhausted)
    }
    pub fn kernel_stack(&self, process: usize, thread: Option<usize>) -> Option<(u64, u64)> {
        let mapping = if let Some(slot) = thread {
            &self.threads.get(slot)?.as_ref()?.kernel
        } else {
            self.kernel_stacks.get(process)?.as_ref()?
        };
        Some((mapping.start().get(), mapping.end().get()))
    }
    fn dynamic_extent(address: u64, bytes: u64) -> Result<()> {
        if bytes == 0
            || !address.is_multiple_of(PAGE_BYTES)
            || !bytes.is_multiple_of(PAGE_BYTES)
            || address < DYNAMIC_START
            || address.checked_add(bytes).is_none_or(|e| e > DYNAMIC_END)
        {
            return Err(Status::InvalidArgument);
        }
        Ok(())
    }
    /// # Safety
    /// Sole VM guard, checked owner and IRQ-off retirement contract. Only the
    /// bounded dynamic arena is exposed; startup, image and stacks are excluded.
    pub unsafe fn allocate(
        &mut self,
        slot: usize,
        address: u64,
        bytes: u64,
        access: u64,
        budget: u64,
    ) -> Result<u64> {
        Self::dynamic_extent(address, bytes)?;
        let permissions = dynamic_permissions(access)?;
        let used = *self.dynamic_bytes.get(slot).ok_or(Status::NotFound)?;
        let after = used
            .checked_add(bytes)
            .filter(|total| *total <= budget)
            .ok_or(Status::ResourceExhausted)?;
        let space = self
            .spaces
            .get_mut(slot)
            .and_then(Option::as_mut)
            .ok_or(Status::NotFound)?;
        let id = space
            .reserve(address, PageCount::new(bytes / PAGE_BYTES).map_err(status)?)
            .map_err(status)?;
        let alias = va(address)?
            .rebase(
                va(DYNAMIC_START)?,
                va(DYNAMIC_ARENA + slot as u64 * DYNAMIC_STRIDE)?,
            )
            .map_err(status)?;
        if let Err(error) = unsafe {
            space.commit(
                id,
                &mut self.frames,
                &mut self.tables,
                alias,
                permissions,
                &[],
            )
        } {
            space.release(id).expect("dynamic allocation rollback");
            return Err(status(error));
        }
        self.dynamic_bytes[slot] = after;
        Ok(address)
    }
    /// # Safety
    /// VM guard and acknowledged retirement contract hold. User owns this exact
    /// dynamic extent and promises no live accesses; captured kernel copies hold
    /// the same VM guard, preventing release while a copy remains in flight.
    pub unsafe fn release(&mut self, slot: usize, address: u64, bytes: u64) -> Result<()> {
        Self::dynamic_extent(address, bytes)?;
        let space = self
            .spaces
            .get_mut(slot)
            .and_then(Option::as_mut)
            .ok_or(Status::NotFound)?;
        let id = space.region_at(address).ok_or(Status::NotFound)?;
        let info = space.get(id).map_err(status)?;
        if info.start != address || info.pages * PAGE_BYTES != bytes || info.shared {
            return Err(Status::InvalidArgument);
        }
        unsafe { space.decommit(id, &mut self.frames, &mut self.tables) }.map_err(status)?;
        space.release(id).map_err(status)?;
        self.dynamic_bytes[slot] = self.dynamic_bytes[slot]
            .checked_sub(bytes)
            .expect("owned dynamic byte budget");
        Ok(())
    }
    /// # Safety
    /// Same guard/extent/retirement contract as release. Execute permission is
    /// deliberately unavailable to this first native heap interface.
    pub unsafe fn protect(
        &mut self,
        slot: usize,
        address: u64,
        bytes: u64,
        access: u64,
    ) -> Result<()> {
        Self::dynamic_extent(address, bytes)?;
        let permissions = dynamic_permissions(access)?;
        let space = self
            .spaces
            .get_mut(slot)
            .and_then(Option::as_mut)
            .ok_or(Status::NotFound)?;
        let id = space.region_at(address).ok_or(Status::NotFound)?;
        let info = space.get(id).map_err(status)?;
        if info.start != address || info.pages * PAGE_BYTES != bytes || info.shared {
            return Err(Status::InvalidArgument);
        }
        unsafe { space.protect(id, &mut self.tables, permissions) }.map_err(status)
    }
    /// # Safety
    /// Sole VM guard, IRQ-off and synchronous retirement contract hold. The key
    /// is reserved by executive authority and not published until this succeeds.
    pub unsafe fn create_section(&mut self, key: u64, bytes: u64) -> Result<()> {
        if key == 0 || bytes == 0 || !bytes.is_multiple_of(PAGE_BYTES) || bytes > SECTION_STRIDE {
            return Err(Status::InvalidArgument);
        }
        if self
            .sections
            .iter()
            .flatten()
            .any(|section| section.key == key)
        {
            return Err(Status::AlreadyExists);
        }
        let slot = self
            .sections
            .iter()
            .position(Option::is_none)
            .ok_or(Status::ResourceExhausted)?;
        let owned = self
            .frames
            .allocate(PageCount::new(bytes / PAGE_BYTES).map_err(status)?, 1)
            .map_err(status)?;
        let alias = va(SECTION_ARENA + slot as u64 * SECTION_STRIDE)?;
        let mapping = match unsafe { self.tables.map_frames(owned, alias, Permissions::ReadWrite) }
        {
            Ok(mapping) => mapping,
            Err(failure) => {
                use core::fmt::Write;
                let mut serial = unsafe { door_boot_debug::Serial::new() };
                let _ = write!(
                    serial,
                    "DOOR_P12 SECTION_MAP error={:?} bytes={bytes} ",
                    failure.error
                );
                self.write_occupancy(&mut serial);
                self.frames.free(failure.frames).map_err(status)?;
                return Err(status(failure.error));
            }
        };
        self.sections[slot] = Some(Section { key, mapping });
        Ok(())
    }
    /// Raw bounded alias for the assembly checked-copy boundary. A live executive
    /// lease pins this backing; callers never form a Rust source reference.
    pub fn section_capture_address(&self, key: u64, offset: u64, bytes: usize) -> Result<u64> {
        let section = self
            .sections
            .iter()
            .flatten()
            .find(|s| s.key == key)
            .ok_or(Status::NotFound)?;
        if offset
            .checked_add(bytes as u64)
            .is_none_or(|end| end > section.mapping.bytes().get())
        {
            return Err(Status::BadAddress);
        }
        let slot = self
            .sections
            .iter()
            .position(|s| s.as_ref().is_some_and(|s| s.key == key))
            .ok_or(Status::NotFound)?;
        Ok(SECTION_ARENA + slot as u64 * SECTION_STRIDE + offset)
    }
    /// # Safety
    /// VM guard and IRQ-off contract hold; caller owns a live executive section
    /// lease authorizing exactly these READ/WRITE rights and retains it until
    /// unmap or process destruction. No running context accesses the new extent.
    pub unsafe fn map_section(
        &mut self,
        slot: usize,
        key: u64,
        address: u64,
        access: u64,
    ) -> Result<()> {
        if access & !(rights::READ | rights::WRITE) != 0 || access & rights::READ == 0 {
            return Err(Status::InvalidArgument);
        }
        let section = self
            .sections
            .iter()
            .flatten()
            .find(|section| section.key == key)
            .ok_or(Status::NotFound)?;
        let end = address
            .checked_add(section.mapping.bytes().get())
            .ok_or(Status::InvalidArgument)?;
        // Writable views stay in the section window; read-only resource
        // views (`door_abi::platform::resource`) may also use their window.
        let sections = address >= USER_SECTION_START && end <= USER_SECTION_END;
        let resources = access == rights::READ
            && address >= door_abi::platform::resource::VIEW_START
            && end <= door_abi::platform::resource::VIEW_END;
        if !sections && !resources {
            return Err(Status::BadAddress);
        }
        let space = self
            .spaces
            .get_mut(slot)
            .and_then(Option::as_mut)
            .ok_or(Status::NotFound)?;
        let id = space
            .reserve(address, section.mapping.pages())
            .map_err(status)?;
        let permissions = if access & rights::WRITE != 0 {
            Permissions::ReadWrite
        } else {
            Permissions::ReadOnly
        };
        if let Err(error) =
            unsafe { space.map_shared(id, &section.mapping, permissions, permissions) }
        {
            space.release(id).expect("failed shared view reservation");
            return Err(status(error));
        }
        Ok(())
    }
    /// # Safety
    /// VM guard/IRQ-off/retirement contract holds. Caller keeps the matching
    /// executive mapping lease until this returns success, then drops it once.
    pub unsafe fn unmap(&mut self, slot: usize, address: u64, bytes: u64) -> Result<()> {
        let space = self
            .spaces
            .get_mut(slot)
            .and_then(Option::as_mut)
            .ok_or(Status::NotFound)?;
        let id = space.region_at(address).ok_or(Status::BadAddress)?;
        let info = space.get(id).map_err(status)?;
        if info.start != address || info.pages * PAGE_BYTES != bytes || !info.shared {
            return Err(Status::InvalidArgument);
        }
        unsafe { space.decommit(id, &mut self.frames, &mut self.tables) }.map_err(status)?;
        space.release(id).map_err(status)
    }
    /// # Safety
    /// Every context completed architecture retirement and can never return to
    /// this process CR3 or kernel stack. Caller retains all view leases until
    /// success, then drains them exactly once. Sole VM guard and IRQs off.
    pub unsafe fn destroy_process(&mut self, slot: usize) -> Result<()> {
        if self.threads.iter().flatten().any(|t| t.process == slot) {
            return Err(Status::WouldBlock);
        }
        if self.spaces.get(slot).and_then(Option::as_ref).is_none() {
            return Err(Status::NotFound);
        }
        // Views first (their pages stay with their sections), then paged
        // Windows memory: its pages go back to the pool while the root
        // still exists, then the regions and tables.
        self.win_views_drop(slot);
        let space = self
            .spaces
            .get_mut(slot)
            .and_then(Option::as_mut)
            .ok_or(Status::NotFound)?;
        let mut pages = mm::PoolPages {
            pool: &mut self.pool,
            frames: &mut self.frames,
            kernel: &mut self.tables,
        };
        unsafe { space.paged_release_all(&mut pages, &mut self.paged_bytes[slot]) }
            .map_err(status)?;
        unsafe { space.destroy(&mut self.frames, &mut self.tables) }.map_err(status)?;
        self.spaces[slot] = None;
        self.dynamic_bytes[slot] = 0;
        self.paged_bytes[slot] = 0;
        self.static_tls[slot] = [None; windows_tls::TLS_SLOTS];
        self.x86[slot] = false;
        if let Some(mapping) = self.kernel_stacks[slot].take() {
            unsafe { self.release_mapping(mapping) };
        }
        Ok(())
    }
    /// # Safety
    /// Executive confirms no handles, in-flight operations or view leases remain.
    /// VM guard/IRQ-off contract holds. Existing view identities are also checked
    /// here before the last physical backing can be returned to the allocator.
    pub unsafe fn destroy_section(&mut self, key: u64) -> Result<()> {
        let index = self
            .sections
            .iter()
            .position(|s| s.as_ref().is_some_and(|s| s.key == key))
            .ok_or(Status::NotFound)?;
        let physical = self.sections[index]
            .as_ref()
            .expect("found section")
            .mapping
            .physical()
            .expect("RAM section")
            .get();
        if self
            .spaces
            .iter()
            .flatten()
            .any(|space| space.has_shared_backing(physical))
        {
            return Err(Status::WouldBlock);
        }
        let section = self.sections[index].take().expect("found section");
        unsafe { self.release_mapping(section.mapping) };
        Ok(())
    }
    unsafe fn release_mapping(&mut self, mapping: MappedRegion) {
        let frames = unsafe { self.tables.unmap(mapping) }
            .expect("owned VM mapping teardown")
            .expect("owned RAM");
        self.frames.free(frames).expect("owned VM frame teardown");
    }
}
fn va(value: u64) -> Result<VirtualAddress> {
    VirtualAddress::new(value).map_err(status)
}
fn dynamic_permissions(access: u64) -> Result<Permissions> {
    match access {
        rights::READ => Ok(Permissions::ReadOnly),
        value if value == rights::READ | rights::WRITE => Ok(Permissions::ReadWrite),
        _ => Err(Status::AccessDenied),
    }
}
fn retire_private(_root: u64, address: u64) {
    // Every target switches to trusted CR3 before acknowledging the IPI. PCID is
    // disabled and private entries never global, so entry/return flush the root's
    // entire cached translation state. Zero requests the same root-wide barrier.
    arch::shootdown(address).expect("private address-space retirement");
    super::capture_vm_input_progress(address);
}
fn status(error: mm::Error) -> Status {
    match error {
        mm::Error::Exhausted | mm::Error::AllocationLimit | mm::Error::MappingLimit => {
            Status::ResourceExhausted
        }
        mm::Error::AccessDenied => Status::AccessDenied,
        mm::Error::Overlap => Status::AlreadyExists,
        mm::Error::Address | mm::Error::NotCommitted => Status::BadAddress,
        mm::Error::Overflow
        | mm::Error::Alignment
        | mm::Error::Empty
        | mm::Error::InvalidState
        | mm::Error::InvalidMapping => Status::InvalidArgument,
        mm::Error::InvalidMemoryMap
        | mm::Error::InvalidFree
        | mm::Error::InvalidTable
        | mm::Error::InterruptsEnabled => Status::IoFailure,
    }
}

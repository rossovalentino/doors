//! Unpublished PE mapping, fixup and protection transaction.
use super::*;
use door_pe_loader::{BundlePlan, Permissions as PePermissions, VisitError};
use door_windows_abi as win;

pub struct WindowsStartup<'a> {
    pub native: &'a [u8],
    pub bootstrap: &'a [u8],
    pub state: &'a [u8],
    pub entry: u64,
    pub pid: u64,
}
/// Runs of the plans this kernel constructs (Initial and DllCallbacks policies).
const MAX_RUNS: usize = door_pe_loader::LoadPolicy::DllCallbacks.limits().runs;
const _: () = assert!(door_pe_loader::LoadPolicy::Initial.limits().runs <= MAX_RUNS);
const MAX_CHECKPOINTS: usize = 4 * MAX_RUNS + 16;
/// Checkpoint bound of a P13 application construction: four per committed
/// run (two reserve-only), the fixed passes and common mappings, and one per
/// static TLS install.
pub const MAX_APPLICATION_CHECKPOINTS: usize =
    4 * door_pe_loader::LoadPolicy::Application.limits().runs
        + 16
        + door_pe_loader::MAX_TLS_MODULES;
// The successfully retired P05 slot-zero alias already has structural tables.
// Shared kernel tables intentionally persist after unmap; reuse that existing
// structure before any service is created instead of consuming diagnostic-only
// table slots. Both guards and the bounded copy stay in its first 2 MiB span.
const SCRATCH: u64 = PROCESS_ARENA + 0x0010_0000;
const SCRATCH_BYTES: usize = 0x000f_f000;

fn visitor(error: VisitError<Status>) -> Status {
    match error {
        VisitError::Callback(error) => error,
        VisitError::Plan(_) => Status::InvalidArgument,
    }
}
/// A section's protection as a committed page's (P18 image mapping).
fn page_protection(value: PePermissions) -> Option<mm::Protection> {
    match value {
        PePermissions::None => None,
        PePermissions::Read => Some(mm::Protection::ReadOnly),
        PePermissions::ReadWrite => Some(mm::Protection::ReadWrite),
        PePermissions::ReadExecute => Some(mm::Protection::ExecuteRead),
    }
}
impl Vm {
    /// Map the modules `wanted` selects (P18 M0b/M0h): each image becomes a
    /// MEM_IMAGE reservation of page-pool pages, so neither its size nor
    /// the kernel alias window limits it. Runs are committed writable and
    /// filled from the file, fixups applied, then every run takes its final
    /// protection. Gaps between runs stay reserved and inaccessible. On
    /// failure the caller releases what was reserved (`unmap_images`).
    unsafe fn map_images(
        &mut self,
        slot: usize,
        plan: &BundlePlan<'_>,
        space: &mut ProcessSpace,
        budget: &mut StartupBudget,
        wanted: &dyn Fn(usize) -> bool,
    ) -> Result<()> {
        let others: u64 = self
            .paged_bytes
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != slot)
            .map(|(_, b)| *b)
            .sum();
        let limit = super::windows_memory::COMMIT_LIMIT.saturating_sub(others);
        let other_chunks = self.table_chunks_except(slot);
        for index in 0..plan.module_count() {
            if !wanted(index) {
                continue;
            }
            let module = plan.module(index).ok_or(Status::NotFound)?;
            budget.step().map_err(status)?;
            let bytes = u64::from(module.image_bytes()).next_multiple_of(PAGE_BYTES);
            space
                .paged_reserve_image(module.source.base.0, bytes)
                .map_err(|error| match error {
                    mm::Error::Overlap => Status::AlreadyExists,
                    other => status(other),
                })?;
        }
        plan.visit_runs(|run| {
            if !wanted(run.module) || page_protection(run.permissions).is_none() {
                return Ok(());
            }
            budget.step().map_err(status)?;
            Self::ensure_tables_in(
                &mut self.frames,
                &mut self.tables,
                other_chunks,
                space,
                slot,
                run.address.0,
                u64::from(run.bytes),
            );
            let mut pages = mm::PoolPages {
                pool: &mut self.pool,
                frames: &mut self.frames,
                kernel: &mut self.tables,
            };
            unsafe {
                space.paged_commit(
                    run.address.0,
                    u64::from(run.bytes),
                    mm::Protection::ReadWrite,
                    false,
                    // Image pages get their frames now: the kernel fills them
                    // and applies fixups through the pool.
                    false,
                    &mut pages,
                    &mut self.paged_bytes[slot],
                    limit,
                )
            }
            .map_err(status)?;
            if let Some(file) = run.file {
                let image = plan
                    .module(run.module)
                    .ok_or(Status::NotFound)?
                    .source
                    .bytes;
                let start = file.offset.0 as usize;
                let bytes = image
                    .get(start..start + file.bytes as usize)
                    .ok_or(Status::InvalidArgument)?;
                unsafe { space.paged_write(run.address.0, bytes, &self.pool) }.map_err(status)?;
            }
            Ok(())
        })
        .map_err(visitor)?;
        // One checkpoint for every fixup, as before: a relocation-heavy image
        // must not multiply the rollback checkpoints.
        budget.step().map_err(status)?;
        plan.visit_fixups(|fixup| {
            if !wanted(fixup.module) {
                return Ok(());
            }
            let value = fixup.value.to_le_bytes();
            unsafe {
                space.paged_write(
                    fixup.address.0,
                    &value[..usize::from(fixup.width)],
                    &self.pool,
                )
            }
            .map_err(|_| Status::BadAddress)
        })
        .map_err(visitor)?;
        plan.visit_runs(|run| {
            let Some(protection) = page_protection(run.permissions) else {
                return Ok(());
            };
            if !wanted(run.module) {
                return Ok(());
            }
            budget.step().map_err(status)?;
            space
                .paged_protect(run.address.0, u64::from(run.bytes), protection, false)
                .map(|_| ())
                .map_err(status)
        })
        .map_err(visitor)
    }
    /// Release the image reservations of the modules `wanted` selects
    /// (unload, or a failed load's rollback): their pages go back to the
    /// pool. Modules never reserved are skipped.
    unsafe fn unmap_images(
        &mut self,
        slot: usize,
        plan: &BundlePlan<'_>,
        space: &mut ProcessSpace,
        wanted: &dyn Fn(usize) -> bool,
    ) -> Result<()> {
        for index in 0..plan.module_count() {
            if !wanted(index) {
                continue;
            }
            let base = plan.module(index).ok_or(Status::NotFound)?.source.base.0;
            if space
                .paged_reservation(base)
                .is_some_and(|r| r.start == base && r.kind == mm::mem::IMAGE)
            {
                let mut pages = mm::PoolPages {
                    pool: &mut self.pool,
                    frames: &mut self.frames,
                    kernel: &mut self.tables,
                };
                unsafe { space.paged_release(base, &mut pages, &mut self.paged_bytes[slot]) }
                    .map_err(status)?;
            }
        }
        Ok(())
    }
    /// Whether any run of the modules `wanted` selects is already taken
    /// (a region or a reservation): its module and address.
    fn image_collision(
        space: &ProcessSpace,
        plan: &BundlePlan<'_>,
        wanted: &dyn Fn(usize) -> bool,
    ) -> Result<Option<(usize, u64)>> {
        let mut collision = None;
        plan.visit_runs(|run| {
            if wanted(run.module)
                && collision.is_none()
                && (space.region_at(run.address.0).is_some()
                    || space.paged_reservation(run.address.0).is_some())
            {
                collision = Some((run.module, run.address.0));
            }
            Ok::<_, Status>(())
        })
        .map_err(visitor)?;
        Ok(collision)
    }
    /// LoadLibrary of system DLLs after startup: map modules `first..` of
    /// `plan` (the application's plan grown by them; earlier modules are
    /// mapped already and keep their places), apply their fixups, set their
    /// final protections and install their static TLS, all or nothing.
    pub unsafe fn windows_modules_load(
        &mut self,
        slot: usize,
        plan: &BundlePlan<'_>,
        first: usize,
    ) -> Result<()> {
        let mut budget = StartupBudget {
            remaining: None,
            injected: false,
        };
        let wanted = |m: usize| m >= first;
        let space = self
            .spaces
            .get(slot)
            .and_then(Option::as_ref)
            .ok_or(Status::NotFound)?;
        if let Some((module, address)) = Self::image_collision(space, plan, &wanted)? {
            super::super::log(format_args!(
                "DOOR_P16 PROVIDER_LOAD collision module={module} address={address:#x}\n"
            ));
            return Err(Status::AlreadyExists);
        }
        let mut space = self.spaces[slot].take().ok_or(Status::NotFound)?;
        let result = unsafe { self.map_images(slot, plan, &mut space, &mut budget, &wanted) };
        self.spaces[slot] = Some(space);
        let result = result.and_then(|()| {
            (first..plan.module_count())
                .try_for_each(|index| unsafe { self.windows_install_tls(slot, plan, index) })
        });
        if result.is_err() {
            let _ = unsafe { self.windows_modules_unload(slot, plan, first) };
        }
        result
    }
    /// Undo `windows_modules_load` (a failed load): modules `first..`.
    unsafe fn windows_modules_unload(
        &mut self,
        slot: usize,
        plan: &BundlePlan<'_>,
        first: usize,
    ) -> Result<()> {
        for index in first..plan.module_count() {
            if plan
                .module(index)
                .is_some_and(|m| m.tls().ok().flatten().is_some())
                && let Some(tls) = super::windows_tls::tls_slot(plan, index)
            {
                unsafe { self.windows_tls_clear(slot, tls) }?;
            }
        }
        let mut space = self.spaces[slot].take().ok_or(Status::NotFound)?;
        let result = unsafe { self.unmap_images(slot, plan, &mut space, &|m| m >= first) };
        self.spaces[slot] = Some(space);
        result
    }
    /// Application DLL mappings are published only after a complete private
    /// transaction. The VM lease serializes this with copies and thread teardown.
    pub unsafe fn windows_module_load(
        &mut self,
        slot: usize,
        plan: &BundlePlan<'_>,
    ) -> Result<u64> {
        let mut budget = StartupBudget {
            remaining: None,
            injected: false,
        };
        unsafe { self.windows_module_load_budget(slot, plan, &mut budget) }
    }
    unsafe fn windows_module_load_budget(
        &mut self,
        slot: usize,
        plan: &BundlePlan<'_>,
        budget: &mut StartupBudget,
    ) -> Result<u64> {
        let module = plan.module(3).ok_or(Status::NotFound)?;
        let wanted = |m: usize| m == 3;
        let space = self
            .spaces
            .get(slot)
            .and_then(Option::as_ref)
            .ok_or(Status::NotFound)?;
        if Self::image_collision(space, plan, &wanted)?.is_some() {
            return Err(Status::AlreadyExists);
        }
        let mut space = self.spaces[slot].take().ok_or(Status::NotFound)?;
        let result = unsafe { self.map_images(slot, plan, &mut space, budget, &wanted) };
        self.spaces[slot] = Some(space);
        let result = result.and_then(|()| unsafe { self.windows_install_tls(slot, plan, 3) });
        if result.is_err() {
            unsafe { self.windows_module_unload(slot, plan) }?;
        }
        result.map(|()| module.entry().map_or(0, |entry| entry.0))
    }
    /// Memory a Windows load holds: frames outside the page pool, pool
    /// pages in use and kernel table pages. A rolled-back load leaves all
    /// three as they were (the pool may keep an emptied chunk).
    fn windows_footprint(&self) -> (u64, u64, u32) {
        let chunk_pages = (mm::PAGE_POOL_BYTES / mm::PAGE_POOL_CHUNKS as u64) / PAGE_BYTES;
        (
            self.frames
                .stats()
                .allocated_pages
                .saturating_sub(self.pool.chunks() as u64 * chunk_pages),
            self.pool.pages_in_use(),
            self.tables.used_table_pages(),
        )
    }
    pub unsafe fn verify_windows_module_rollback(
        &mut self,
        slot: usize,
        plan: &BundlePlan<'_>,
    ) -> Result<usize> {
        let before = self.windows_footprint();
        for limit in 0..256 {
            let mut budget = StartupBudget {
                remaining: Some(limit),
                injected: false,
            };
            let result = unsafe { self.windows_module_load_budget(slot, plan, &mut budget) };
            let success = result.is_ok();
            match result {
                Ok(_) => unsafe { self.windows_module_unload(slot, plan) }?,
                Err(Status::ResourceExhausted) if budget.injected => (),
                Err(error) => return Err(error),
            }
            if self.windows_footprint().0 != before.0
                || self.windows_footprint().1 != before.1
                || self.windows_module_present(slot, plan, 3)
                || self.static_tls[slot][3].is_some()
            {
                return Err(Status::IoFailure);
            }
            if success {
                return Ok(limit);
            }
        }
        Err(Status::IoFailure)
    }
    pub fn windows_module_present(&self, slot: usize, plan: &BundlePlan<'_>, index: usize) -> bool {
        plan.module(index).is_some_and(|module| {
            self.check(slot, module.source.base.0, 1, UserAccess::Read)
                .is_ok()
        })
    }
    /// Final reference owner has completed application detach before calling.
    /// Copies and physical retirement remain serialized under the VM lease.
    pub unsafe fn windows_module_unload(
        &mut self,
        slot: usize,
        plan: &BundlePlan<'_>,
    ) -> Result<()> {
        unsafe { self.windows_tls_clear(slot, 3) }?;
        let mut space = self.spaces[slot].take().ok_or(Status::NotFound)?;
        let result = unsafe { self.unmap_images(slot, plan, &mut space, &|m| m == 3) };
        self.spaces[slot] = Some(space);
        result
    }
    /// Temporarily own a guarded NX buffer for immutable-image diagnostics.
    /// No reference may escape the closure. The VM guard owns its complete
    /// allocation, mapping, use and retirement; no user process can access it.
    pub unsafe fn with_windows_scratch<R>(
        &mut self,
        bytes: usize,
        inspect: impl FnOnce(&mut [u8]) -> Result<R>,
    ) -> Result<R> {
        if bytes == 0 || bytes > SCRATCH_BYTES {
            return Err(Status::InvalidArgument);
        }
        if self.spaces.iter().any(Option::is_some)
            || self.kernel_stacks.iter().any(Option::is_some)
            || self.threads.iter().any(Option::is_some)
        {
            return Err(Status::AlreadyExists);
        }
        let pages = (bytes as u64).div_ceil(PAGE_BYTES);
        let baseline = self.frames.stats();
        let baseline_tables = self.tables.used_table_pages();
        for address in
            (SCRATCH - PAGE_BYTES..=SCRATCH + pages * PAGE_BYTES).step_by(PAGE_BYTES as usize)
        {
            if self.tables.is_mapped(va(address)?).map_err(status)? {
                return Err(Status::AlreadyExists);
            }
        }
        let owned = self
            .frames
            .allocate(PageCount::new(pages).map_err(status)?, 1)
            .map_err(status)?;
        let mapping = match unsafe {
            self.tables
                .map_frames(owned, va(SCRATCH)?, Permissions::ReadWrite)
        } {
            Ok(mapping) => mapping,
            Err(failure) => {
                self.frames.free(failure.frames).map_err(status)?;
                return Err(status(failure.error));
            }
        };
        let result = inspect(unsafe { core::slice::from_raw_parts_mut(SCRATCH as *mut u8, bytes) });
        unsafe { self.release_mapping(mapping) };
        if self.frames.stats() != baseline || self.tables.used_table_pages() != baseline_tables {
            return Err(Status::IoFailure);
        }
        for address in
            (SCRATCH - PAGE_BYTES..=SCRATCH + pages * PAGE_BYTES).step_by(PAGE_BYTES as usize)
        {
            if self.tables.is_mapped(va(address)?).map_err(status)? {
                return Err(Status::IoFailure);
            }
        }
        result
    }
    /// # Safety
    /// The process VM guard, unpublished-context and immutable-input contracts
    /// of create_process hold. PE parsing and fixup validation already succeeded.
    pub unsafe fn create_windows_process(
        &mut self,
        slot: usize,
        plan: &BundlePlan<'_>,
        startup: &WindowsStartup<'_>,
    ) -> Result<UserLayout> {
        let mut budget = StartupBudget {
            remaining: None,
            injected: false,
        };
        unsafe { self.create_windows_process_budget(slot, plan, startup, &mut budget) }
    }
    /// Construction plus static TLS installation; each TLS module is its own
    /// checkpoint, so diagnostics also fail after every earlier install.
    unsafe fn create_windows_process_budget(
        &mut self,
        slot: usize,
        plan: &BundlePlan<'_>,
        startup: &WindowsStartup<'_>,
        budget: &mut StartupBudget,
    ) -> Result<UserLayout> {
        let layout = unsafe { self.create_windows_with_budget(slot, plan, startup, budget) }?;
        let manifest = door_abi::platform::StartupV2::decode(startup.native)?
            .identity
            .manifest;
        let result = (0..plan.module_count())
            .filter(|index| *index != 3 || manifest != door_abi::platform::manifest::WINDOWS_BASE)
            .try_for_each(|index| {
                if plan
                    .module(index)
                    .and_then(|module| module.tls().ok().flatten())
                    .is_some()
                {
                    budget.step().map_err(status)?;
                }
                unsafe { self.windows_install_tls(slot, plan, index) }
            });
        if let Err(error) = result {
            unsafe { self.destroy_process(slot) }?;
            return Err(error);
        }
        Ok(layout)
    }
    /// One P13 application rollback attempt, failing checkpoint `limit` of
    /// the production construction (every run, fixup pass, protection,
    /// common mapping and TLS install). Each attempt is one VM-guard hold,
    /// so the caller may release the guard between attempts: the baseline
    /// is taken and re-checked inside the hold, and the slot's private arena
    /// is inspected page by page. Returns `Some(tls)` once the full
    /// construction succeeded (and was retired), `None` after an injected
    /// failure that left nothing behind.
    ///
    /// # Safety
    /// Same requirements as create_windows_process; the slot is unpublished.
    pub unsafe fn verify_windows_application_attempt(
        &mut self,
        slot: usize,
        plan: &BundlePlan<'_>,
        startup: &WindowsStartup<'_>,
        limit: usize,
    ) -> Result<Option<usize>> {
        if slot >= MAX_PROCESSES
            || !plan.policy().application()
            || self.spaces[slot].is_some()
            || self.kernel_stacks[slot].is_some()
            || self.static_tls[slot].iter().any(Option::is_some)
        {
            return Err(Status::InvalidArgument);
        }
        let baseline_memory = self.windows_footprint();
        let baseline = self.stats();
        self.verify_empty_arena(slot)?;
        let mut budget = StartupBudget {
            remaining: Some(limit),
            injected: false,
        };
        let result =
            unsafe { self.create_windows_process_budget(slot, plan, startup, &mut budget) };
        let outcome = match result {
            Err(Status::ResourceExhausted) if budget.injected => None,
            Err(error) => return Err(error),
            Ok(layout) => {
                let expected = (0..plan.module_count())
                    .filter(|&index| {
                        plan.module(index)
                            .and_then(|module| module.tls().ok().flatten())
                            .is_some()
                    })
                    .count();
                let installed = self.static_tls[slot].iter().flatten().count();
                let mut validation = self.verify_windows_permissions(slot, plan, &layout);
                if validation.is_ok() && (budget.injected || installed != expected) {
                    validation = Err(Status::IoFailure);
                }
                unsafe { self.destroy_process(slot) }?;
                validation?;
                Some(installed)
            }
        };
        let current = self.stats();
        let memory = self.windows_footprint();
        if (memory.0, memory.1) != (baseline_memory.0, baseline_memory.1)
            || current.processes != baseline.processes
            || current.sections != baseline.sections
            || self.spaces[slot].is_some()
            || self.kernel_stacks[slot].is_some()
            || self.static_tls[slot].iter().any(Option::is_some)
        {
            return Err(Status::IoFailure);
        }
        self.verify_empty_arena(slot)?;
        Ok(outcome)
    }
    unsafe fn create_windows_with_budget(
        &mut self,
        slot: usize,
        plan: &BundlePlan<'_>,
        startup: &WindowsStartup<'_>,
        budget: &mut StartupBudget,
    ) -> Result<UserLayout> {
        let native = door_abi::platform::StartupV2::decode(startup.native)?;
        // The application path (10, 15) publishes DOORWIN2; P09/P10 keep DOORWIN1.
        let application = matches!(
            native.identity.manifest,
            door_abi::platform::manifest::WINDOWS_APPLICATION
                | door_abi::platform::manifest::WINDOWS_GUI
        );
        let (native_startup, teb, peb, parameters) = if application {
            let b = win::bootstrap::Bootstrap::decode(startup.bootstrap)
                .map_err(|_| Status::InvalidArgument)?;
            (b.native_startup, b.teb, b.peb, b.parameters)
        } else {
            let b =
                win::Bootstrap::decode(startup.bootstrap).map_err(|_| Status::InvalidArgument)?;
            (b.native_startup, b.teb, b.peb, b.parameters)
        };
        if slot >= MAX_PROCESSES
            || native.base.pid != startup.pid
            || !(application
                || matches!(
                    native.identity.manifest,
                    door_abi::platform::manifest::WINDOWS
                        | door_abi::platform::manifest::WINDOWS_BASE
                ))
            || native.base.entry != startup.entry
            || native.base.tls != USER_TLS
            || native.base.stack_top != Self::service_stack_top()
            || native_startup != USER_STARTUP
            || teb != win::TEB_ADDRESS
            || peb != win::PEB_ADDRESS
            || parameters != win::PARAMETERS_ADDRESS
            || startup.state.len() != win::STATE_BYTES
        {
            return Err(Status::InvalidArgument);
        }
        // Images live in page-pool memory (`map_images`); only the common
        // mappings below use the process's alias window.
        // Every construction attempt (rollback checks included) knows the
        // personality before any thread page or TLS block is written.
        self.x86[slot] = plan.policy() == door_pe_loader::LoadPolicy::Application32;
        unsafe {
            self.construct_process(slot, budget, |this, space, budget| {
                let mut alias = va(PROCESS_ARENA + slot as u64 * PROCESS_STRIDE + 0x10_0000)?;
                // A WINDOWS_BASE bundle maps module 3 later (its LoadLibrary
                // fixture); every other module is mapped now.
                let deferred =
                    native.identity.manifest == door_abi::platform::manifest::WINDOWS_BASE;
                this.map_images(slot, plan, space, budget, &|m| !(deferred && m == 3))?;
                this.commit_common(
                    space,
                    &mut alias,
                    SERVICE_STACK_PAGES,
                    startup.native,
                    startup.pid,
                    budget,
                )?;
                this.commit_region(
                    space,
                    &mut alias,
                    win::BOOTSTRAP_ADDRESS,
                    1,
                    Permissions::ReadOnly,
                    startup.bootstrap,
                    budget,
                )?;
                this.commit_region(
                    space,
                    &mut alias,
                    win::STATE_ADDRESS,
                    (win::STATE_BYTES as u64) / PAGE_BYTES,
                    Permissions::ReadWrite,
                    startup.state,
                    budget,
                )?;
                space
                    .validate_user_range(startup.entry, 1, UserAccess::Execute)
                    .map_err(status)?;
                Ok(Construction {
                    entry: startup.entry,
                    next_alias: alias,
                    stack_pages: SERVICE_STACK_PAGES,
                })
            })
        }
    }
    /// Exhaust each real construction checkpoint and inspect final permissions.
    /// No task exists during these attempts, so teardown cannot race execution.
    ///
    /// # Safety
    /// Same requirements as create_windows_process; the slot is unpublished.
    pub unsafe fn verify_windows_rollback(
        &mut self,
        slot: usize,
        plan: &BundlePlan<'_>,
        startup: &WindowsStartup<'_>,
    ) -> Result<usize> {
        if slot >= MAX_PROCESSES
            || self.spaces[slot].is_some()
            || self.kernel_stacks[slot].is_some()
        {
            return Err(Status::InvalidArgument);
        }
        let baseline_memory = self.windows_footprint();
        let baseline = self.stats();
        self.verify_empty_arena(slot)?;
        for limit in 0..=MAX_CHECKPOINTS {
            let mut budget = StartupBudget {
                remaining: Some(limit),
                injected: false,
            };
            let result =
                unsafe { self.create_windows_with_budget(slot, plan, startup, &mut budget) };
            let succeeded = result.is_ok();
            match result {
                Err(Status::ResourceExhausted) if budget.injected => (),
                Err(error) => return Err(error),
                Ok(layout) => {
                    let validation = self.verify_windows_permissions(slot, plan, &layout);
                    unsafe { self.destroy_process(slot) }?;
                    validation?;
                }
            }
            let memory = self.windows_footprint();
            if (memory.0, memory.1) != (baseline_memory.0, baseline_memory.1)
                || self.stats().processes != baseline.processes
                || self.stats().sections != baseline.sections
                || self.spaces[slot].is_some()
                || self.kernel_stacks[slot].is_some()
            {
                return Err(Status::IoFailure);
            }
            self.verify_empty_arena(slot)?;
            if succeeded {
                return Ok(limit);
            }
        }
        Err(Status::IoFailure)
    }
    fn verify_windows_permissions(
        &self,
        slot: usize,
        plan: &BundlePlan<'_>,
        layout: &UserLayout,
    ) -> Result<()> {
        plan.visit_runs(|run| {
            if plan.module_count() >= 4 && run.module == 3 {
                return Ok(());
            }
            for offset in (0..run.bytes).step_by(PAGE_BYTES as usize) {
                let address = run.address.0 + u64::from(offset);
                for (access, expected) in [
                    (UserAccess::Read, run.permissions != PePermissions::None),
                    (
                        UserAccess::Write,
                        run.permissions == PePermissions::ReadWrite,
                    ),
                    (
                        UserAccess::Execute,
                        run.permissions == PePermissions::ReadExecute,
                    ),
                ] {
                    if self.check(slot, address, 1, access).is_ok() != expected {
                        return Err(Status::IoFailure);
                    }
                }
            }
            Ok(())
        })
        .map_err(visitor)?;
        self.check(slot, layout.entry, 1, UserAccess::Execute)?;
        self.check(slot, win::TEB_ADDRESS, win::STATE_BYTES, UserAccess::Write)?;
        self.check(
            slot,
            win::BOOTSTRAP_ADDRESS,
            win::BOOTSTRAP_BYTES,
            UserAccess::Read,
        )?;
        if self
            .check(slot, win::BOOTSTRAP_ADDRESS, 1, UserAccess::Write)
            .is_ok()
            || self
                .check(slot, USER_STACK - PAGE_BYTES, 1, UserAccess::Read)
                .is_ok()
            || self
                .check(slot, layout.user_stack_top, 1, UserAccess::Read)
                .is_ok()
        {
            return Err(Status::IoFailure);
        }
        Ok(())
    }
}

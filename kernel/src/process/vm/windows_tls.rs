//! Compiler TLS templates belong to the process image lifetime. Private FS slots
//! supply the GS TLS vector without granting authority to public TEB bytes.
//!
//! The vector has one entry per static TLS *slot*, not per module: P09/P10
//! plans (at most five modules) keep slot = module index, and P13
//! `Application` plans (up to thirty-two modules since P18) use the planner's stable
//! `tls_slot`, bounded by `MAX_TLS_MODULES`. The slot is what the loader
//! writes into the image's `_tls_index`.
use super::*;
pub(super) const TLS_VECTOR: u64 = 768;
pub(super) const TLS_DATA: u64 = 2304;
pub(super) const TLS_SLOTS: usize = door_pe_loader::MAX_TLS_MODULES;
const _: () = {
    assert!(door_pe_loader::LoadPolicy::DllCallbacks.limits().modules <= TLS_SLOTS);
    assert!(door_pe_loader::LoadPolicy::Initial.limits().modules <= TLS_SLOTS);
    assert!(TLS_VECTOR + TLS_SLOTS as u64 * 8 <= 1024);
    assert!(TLS_DATA >= 2048 + 104);
    assert!(TLS_DATA + TLS_SLOTS as u64 * 256 <= 4096);
};
/// Static TLS slot of plan module `index`, if it has a TLS directory.
pub(super) fn tls_slot(plan: &door_pe_loader::BundlePlan<'_>, index: usize) -> Option<usize> {
    if plan.policy().application() {
        plan.tls_slot(index).filter(|slot| *slot < TLS_SLOTS)
    } else {
        (index < TLS_SLOTS).then_some(index)
    }
}
#[derive(Clone, Copy)]
pub(super) struct StaticTls {
    bytes: [u8; 256],
}
pub(super) fn fill_tls(
    bytes: &mut [u8; 4096],
    address: u64,
    templates: &[Option<StaticTls>; TLS_SLOTS],
) {
    bytes[(2048 + 88)..(2048 + 96)].copy_from_slice(&(address + TLS_VECTOR).to_le_bytes());
    for (index, template) in templates.iter().enumerate() {
        let vector = TLS_VECTOR as usize + index * 8;
        let data = TLS_DATA as usize + index * 256;
        if let Some(template) = template {
            bytes[vector..vector + 8].copy_from_slice(&(address + data as u64).to_le_bytes());
            bytes[data..data + 256].copy_from_slice(&template.bytes);
        }
    }
}
/// `fill_tls` for a 32-bit thread page (`door_windows_abi::x86`): 4-byte
/// array entries, the TEB32's ThreadLocalStoragePointer set by
/// `initial_teb`.
pub(super) fn fill_tls32(
    bytes: &mut [u8; 4096],
    address: u64,
    templates: &[Option<StaticTls>; TLS_SLOTS],
) {
    use door_windows_abi::x86::{PAGE_TLS_ARRAY, PAGE_TLS_BLOCK, PAGE_TLS_DATA};
    for (index, template) in templates.iter().enumerate() {
        let entry = PAGE_TLS_ARRAY + index * 4;
        let data = PAGE_TLS_DATA + index * PAGE_TLS_BLOCK;
        if let Some(template) = template {
            bytes[entry..entry + 4]
                .copy_from_slice(&((address + data as u64) as u32).to_le_bytes());
            bytes[data..data + PAGE_TLS_BLOCK].copy_from_slice(&template.bytes);
        }
    }
}
unsafe fn owned_write(
    space: &ProcessSpace,
    pool: &mm::PagePool,
    destination: u64,
    bytes: &[u8],
) -> Result<()> {
    // Image memory (P18) is page-pool memory: written through the pool.
    if space.paged_reservation(destination).is_some() {
        return unsafe { space.paged_write(destination, bytes, pool) }.map_err(status);
    }
    space
        .validate_user_range(destination, bytes.len(), UserAccess::Write)
        .map_err(status)?;
    for (offset, value) in bytes.iter().enumerate() {
        let address = destination + offset as u64;
        let region = space
            .get(space.region_at(address).ok_or(Status::BadAddress)?)
            .map_err(status)?;
        let alias = region.kernel_alias.ok_or(Status::BadAddress)? + (address - region.start);
        unsafe {
            (alias as *mut u8).write(*value);
        }
    }
    Ok(())
}
impl Vm {
    pub(super) unsafe fn windows_install_tls(
        &mut self,
        slot: usize,
        plan: &door_pe_loader::BundlePlan<'_>,
        index: usize,
    ) -> Result<()> {
        let module = plan.module(index).ok_or(Status::NotFound)?;
        let Some(tls) = module.tls().map_err(|_| Status::InvalidArgument)? else {
            return Ok(());
        };
        let tls_index = tls_slot(plan, index).ok_or(Status::ResourceExhausted)?;
        let mut template = StaticTls { bytes: [0; 256] };
        let space = self.spaces[slot].as_ref().ok_or(Status::NotFound)?;
        let source = module.source.base.0 + u64::from(tls.template_rva.0);
        space
            .validate_user_range(source, tls.template.len(), UserAccess::Read)
            .map_err(status)?;
        // The image fixups are complete. A pointer initializer in .tls must use
        // its relocated image value, not the immutable file's preferred VA.
        if space.paged_reservation(source).is_some() {
            unsafe {
                space.paged_read(
                    source,
                    &mut template.bytes[..tls.template.len()],
                    &self.pool,
                )
            }
            .map_err(status)?;
        } else {
            for (offset, output) in template.bytes[..tls.template.len()].iter_mut().enumerate() {
                let address = source + offset as u64;
                let region = space
                    .get(space.region_at(address).ok_or(Status::BadAddress)?)
                    .map_err(status)?;
                let alias =
                    region.kernel_alias.ok_or(Status::BadAddress)? + (address - region.start);
                *output = unsafe { (alias as *const u8).read() };
            }
        }
        unsafe {
            owned_write(
                space,
                &self.pool,
                module.source.base.0 + u64::from(tls.index.0),
                &(tls_index as u32).to_le_bytes(),
            )
        }?;
        self.static_tls[slot][tls_index] = Some(template);
        unsafe { self.windows_tls_publish(slot, tls_index, Some(template)) }
    }
    pub(super) unsafe fn windows_tls_clear(&mut self, slot: usize, index: usize) -> Result<()> {
        self.static_tls[slot][index] = None;
        unsafe { self.windows_tls_publish(slot, index, None) }
    }
    unsafe fn windows_tls_publish(
        &self,
        slot: usize,
        index: usize,
        template: Option<StaticTls>,
    ) -> Result<()> {
        let space = self.spaces[slot].as_ref().ok_or(Status::NotFound)?;
        let threads = core::iter::once((USER_TLS, door_windows_abi::TEB_ADDRESS)).chain(
            self.threads
                .iter()
                .flatten()
                .filter(|m| m.process == slot)
                .map(|m| (m.tls, m.tls + 2048)),
        );
        for (base, teb) in threads {
            if self.x86[slot] {
                // x86: the array of 4-byte pointers TEB32+0x2C names.
                use door_windows_abi::x86;
                let array = base + x86::PAGE_TLS_ARRAY as u64;
                let data = base + (x86::PAGE_TLS_DATA + index * x86::PAGE_TLS_BLOCK) as u64;
                let pointer = if template.is_some() { data as u32 } else { 0 };
                unsafe {
                    owned_write(
                        space,
                        &self.pool,
                        teb + x86::TEB_TLS_POINTER as u64,
                        &(array as u32).to_le_bytes(),
                    )?;
                    owned_write(
                        space,
                        &self.pool,
                        data,
                        &template.map_or([0; 256], |t| t.bytes),
                    )?;
                    owned_write(
                        space,
                        &self.pool,
                        array + index as u64 * 4,
                        &pointer.to_le_bytes(),
                    )?;
                }
                continue;
            }
            let data = base + TLS_DATA + index as u64 * 256;
            let pointer = if template.is_some() { data } else { 0 };
            unsafe {
                owned_write(
                    space,
                    &self.pool,
                    teb + 88,
                    &(base + TLS_VECTOR).to_le_bytes(),
                )?;
                owned_write(
                    space,
                    &self.pool,
                    data,
                    &template.map_or([0; 256], |t| t.bytes),
                )?;
                owned_write(
                    space,
                    &self.pool,
                    base + TLS_VECTOR + index as u64 * 8,
                    &pointer.to_le_bytes(),
                )?;
            }
        }
        Ok(())
    }
}

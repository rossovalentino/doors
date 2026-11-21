//! Per-thread state where each architecture's personality keeps it. x64:
//! the TEB through GS, Doors' private fields (last error, thread id, dynamic
//! TLS slots) on the separate native FS page. x86 (P17): FS is the TEB32,
//! whose Windows fields hold the thread id and last error and whose GDI
//! batch area holds the dynamic TLS slots (`door_windows_abi::x86`).
#[cfg(target_arch = "x86_64")]
mod imp {
    use door_windows_abi::{self as abi, tls};
    pub(crate) fn last_error() -> u32 {
        let value: u32;
        // SAFETY: the Windows personality keeps a separate native FS page.
        unsafe {
            core::arch::asm!("mov {value:e}, fs:[{offset}]", value=out(reg) value,
            offset=const abi::LAST_ERROR_FS_OFFSET, options(nostack, readonly, preserves_flags));
        }
        value
    }
    pub(crate) fn set_last_error(value: u32) {
        // SAFETY: the private FS slot is disjoint from the Windows GS view.
        unsafe {
            core::arch::asm!("mov fs:[{offset}], {value:e}", value=in(reg) value,
            offset=const abi::LAST_ERROR_FS_OFFSET, options(nostack, preserves_flags));
        }
    }
    pub(crate) fn thread_id() -> u32 {
        let id: u32;
        unsafe {
            core::arch::asm!("mov {id:e}, fs:[{offset}]", id=out(reg) id,
            offset=const abi::THREAD_ID_FS_OFFSET, options(nostack, readonly, preserves_flags));
        }
        id
    }
    pub(crate) fn set_thread_id(id: u32) {
        unsafe {
            core::arch::asm!("mov fs:[{offset}], {id:e}", offset=const abi::THREAD_ID_FS_OFFSET,
            id=in(reg) id, options(nostack, preserves_flags));
        }
    }
    /// (tag, value) of dynamic TLS slot `index`.
    pub(crate) fn tls_slot(index: u32) -> (u64, u64) {
        let offset = tls::FS_OFFSET + index as usize * tls::SLOT_BYTES;
        let (tag, value): (u64, u64);
        unsafe {
            core::arch::asm!("mov {tag}, fs:[{offset}]", "mov {value}, fs:[{offset} + 8]",
            tag=out(reg) tag, value=out(reg) value, offset=in(reg) offset,
            options(nostack, readonly, preserves_flags));
        }
        (tag, value)
    }
    pub(crate) fn set_tls_slot(index: u32, tag: u64, value: u64) {
        let offset = tls::FS_OFFSET + index as usize * tls::SLOT_BYTES;
        unsafe {
            core::arch::asm!("mov fs:[{offset}], {tag}", "mov fs:[{offset} + 8], {value}",
            offset=in(reg) offset, tag=in(reg) tag, value=in(reg) value,
            options(nostack, preserves_flags));
        }
    }
    /// The TEB and PEB addresses the personality published.
    pub(crate) fn teb() -> u64 {
        let teb: u64;
        // SAFETY: the trusted personality sets GS to the mapped TEB.
        unsafe {
            core::arch::asm!("mov {}, gs:[48]", out(reg) teb, options(nostack, readonly, preserves_flags));
        }
        teb
    }
    pub(crate) fn peb() -> u64 {
        let peb: u64;
        unsafe {
            core::arch::asm!("mov {}, gs:[96]", out(reg) peb, options(nostack, readonly, preserves_flags));
        }
        peb
    }
    /// PEB.ProcessHeap.
    pub(crate) const PEB_PROCESS_HEAP: u64 = 0x30;
    /// PEB.ImageBaseAddress.
    pub(crate) const PEB_IMAGE_BASE: u64 = 0x10;
    /// The thread-exit marker the native FS page's thread-id field takes.
    pub(crate) fn mark_thread_exiting() {
        unsafe {
            core::arch::asm!("mov dword ptr fs:[{offset}], 0xffffffff",
            offset=const abi::THREAD_ID_FS_OFFSET, options(nostack, preserves_flags));
        }
    }
}
#[cfg(target_arch = "x86")]
mod imp {
    use door_windows_abi::{tls, x86};
    fn read(offset: usize) -> u32 {
        let value: u32;
        // SAFETY: the personality sets FS to this thread's mapped TEB32.
        unsafe {
            core::arch::asm!("mov {value}, fs:[{offset}]", value=out(reg) value,
            offset=in(reg) offset, options(nostack, readonly, preserves_flags));
        }
        value
    }
    fn write(offset: usize, value: u32) {
        unsafe {
            core::arch::asm!("mov fs:[{offset}], {value}", value=in(reg) value,
            offset=in(reg) offset, options(nostack, preserves_flags));
        }
    }
    pub(crate) fn last_error() -> u32 {
        read(x86::TEB_LAST_ERROR)
    }
    /// The thread's file system redirection: 0 on, 1 off.
    pub(crate) fn fs_redirection() -> u32 {
        read(x86::TEB_FS_REDIRECTION)
    }
    pub(crate) fn set_fs_redirection(value: u32) {
        write(x86::TEB_FS_REDIRECTION, value)
    }
    pub(crate) fn set_last_error(value: u32) {
        write(x86::TEB_LAST_ERROR, value)
    }
    pub(crate) fn thread_id() -> u32 {
        read(x86::TEB_THREAD_ID)
    }
    pub(crate) fn set_thread_id(id: u32) {
        write(x86::TEB_THREAD_ID, id)
    }
    pub(crate) fn tls_slot(index: u32) -> (u64, u64) {
        let offset = x86::TEB_TLS_SLOTS + index as usize * tls::SLOT_BYTES;
        let word = |at: usize| u64::from(read(at)) | (u64::from(read(at + 4)) << 32);
        (word(offset), word(offset + 8))
    }
    pub(crate) fn set_tls_slot(index: u32, tag: u64, value: u64) {
        let offset = x86::TEB_TLS_SLOTS + index as usize * tls::SLOT_BYTES;
        write(offset, tag as u32);
        write(offset + 4, (tag >> 32) as u32);
        write(offset + 8, value as u32);
        write(offset + 12, (value >> 32) as u32);
        // Windows' TlsSlots, which programs may read inline.
        if (index as usize) < 64 {
            write(
                x86::TEB_WINDOWS_TLS_SLOTS + 4 * index as usize,
                value as u32,
            );
        }
    }
    pub(crate) fn teb() -> u64 {
        u64::from(read(x86::TEB_SELF))
    }
    pub(crate) fn peb() -> u64 {
        u64::from(read(x86::TEB_PEB))
    }
    pub(crate) const PEB_PROCESS_HEAP: u64 = x86::PEB_PROCESS_HEAP as u64;
    pub(crate) const PEB_IMAGE_BASE: u64 = x86::PEB_IMAGE_BASE as u64;
    pub(crate) fn mark_thread_exiting() {
        write(x86::TEB_THREAD_ID, u32::MAX)
    }
}
pub(crate) use imp::*;

/// Calls a DllMain or PIMAGE_TLS_CALLBACK (HINSTANCE, DWORD, LPVOID): the
/// image's own code at `entry`, with pointer-width arguments.
pub(crate) fn dll_main(entry: u64, base: u64, reason: u32, reserved: u64) -> i32 {
    // SAFETY: the loader verified `entry` inside an executable image mapping.
    let f: extern "system" fn(usize, u32, usize) -> i32 =
        unsafe { core::mem::transmute(entry as usize) };
    f(base as usize, reason, reserved as usize)
}
/// Calls a one-argument callback (PAPCFUNC, PVECTORED_EXCEPTION_HANDLER,
/// LPTHREAD_START_ROUTINE): its pointer-width result.
pub(crate) fn call_one(function: u64, argument: u64) -> usize {
    // SAFETY: registered by the program as a function of this signature.
    let f: extern "system" fn(usize) -> usize = unsafe { core::mem::transmute(function as usize) };
    f(argument as usize)
}

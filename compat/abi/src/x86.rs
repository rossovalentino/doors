//! The 32-bit (x86, WOW64) process view (P17). A 32-bit thread's FS segment
//! base is its TEB32, laid out where Windows puts the public fields programs
//! and their compilers read directly (`fs:[0]` SEH chain, `fs:[0x18]` self,
//! `fs:[0x2C]` implicit TLS, `fs:[0x30]` PEB); 32-bit ntdll keeps its own
//! per-thread state in fields Windows itself uses for it (the thread id,
//! the last error) and Doors' dynamic TLS slots in a TEB32 range programs
//! never address. Every other part of the process layout (heap, thread
//! stacks, the startup block) already lies below 4 GB and is shared with
//! the x64 view.

/// NT_TIB32.ExceptionList: the head of the SEH chain (`0xFFFFFFFF` ends it).
pub const TEB_EXCEPTION_LIST: usize = 0x00;
/// NT_TIB32.StackBase and StackLimit.
pub const TEB_STACK_BASE: usize = 0x04;
pub const TEB_STACK_LIMIT: usize = 0x08;
/// NT_TIB32.Self.
pub const TEB_SELF: usize = 0x18;
/// ClientId.UniqueProcess and UniqueThread.
pub const TEB_PROCESS_ID: usize = 0x20;
pub const TEB_THREAD_ID: usize = 0x24;
/// ThreadLocalStoragePointer: the implicit TLS block array.
pub const TEB_TLS_POINTER: usize = 0x2C;
/// ProcessEnvironmentBlock.
pub const TEB_PEB: usize = 0x30;
/// LastErrorValue.
pub const TEB_LAST_ERROR: usize = 0x34;
/// Doors' dynamic TLS slots (`tls::SLOTS` of `tls::SLOT_BYTES`): inside
/// the TEB32's GDI batch area, which no program addresses directly.
pub const TEB_TLS_SLOTS: usize = 0x400;
/// The thread's file system redirection (Wow64DisableWow64FsRedirection):
/// 0 on (a new thread's state), 1 off. In the same private area.
pub const TEB_FS_REDIRECTION: usize = 0x3F8;
/// msvcrt.dll's errno for the thread (32-bit), in the same private area.
pub const TEB_ERRNO: usize = 0x3F4;
const _: () = assert!(TEB_FS_REDIRECTION + 4 <= TEB_TLS_SLOTS);
/// The end of Doors' slot range.
pub const TEB_SLOTS_END: usize = TEB_TLS_SLOTS + crate::tls::SLOTS * crate::tls::SLOT_BYTES;
/// DeallocationStack: the stack's lowest address.
pub const TEB_DEALLOCATION_STACK: usize = 0xE0C;
/// TlsSlots[64]: the values of TlsAlloc's slots, as programs read them
/// inline (`fs:[0xE10 + 4 * index]`); Doors mirrors its slots here.
pub const TEB_WINDOWS_TLS_SLOTS: usize = 0xE10;
/// TlsExpansionSlots (slots past 64; none).
pub const TEB_TLS_EXPANSION: usize = 0xF94;
const _: () = assert!(TEB_SLOTS_END <= 0xBF4);
/// A TEB32 is one page, as Windows' (its last field near 0xFFC).
pub const TEB_BYTES: usize = 4096;
/// PEB32.ImageBaseAddress, ProcessParameters, ProcessHeap and SessionId.
pub const PEB_IMAGE_BASE: usize = 0x08;
pub const PEB_PROCESS_PARAMETERS: usize = 0x10;
pub const PEB_PROCESS_HEAP: usize = 0x18;
pub const PEB_SESSION_ID: usize = 0x1d4;
/// RTL_USER_PROCESS_PARAMETERS32: MaximumLength, Length, Flags, the
/// ImagePathName and CommandLine UNICODE_STRING32s and Environment, and the
/// structure's size (as Windows 10 and later fill both lengths).
pub const PARAMETERS32_MAXIMUM_LENGTH: usize = 0x00;
pub const PARAMETERS32_LENGTH: usize = 0x04;
pub const PARAMETERS32_FLAGS: usize = 0x08;
pub const PARAMETERS32_IMAGE_PATH: usize = 0x38;
pub const PARAMETERS32_COMMAND_LINE: usize = 0x40;
pub const PARAMETERS32_ENVIRONMENT: usize = 0x48;
pub const PARAMETERS32_BYTES: u32 = 0x2A0;
/// A 32-bit thread's pages (Doors maps `THREAD_PAGES` for every 32-bit
/// thread): the implicit TLS array at `PAGE_TLS_ARRAY`, the static TLS
/// blocks from `PAGE_TLS_DATA` (one of `PAGE_TLS_BLOCK` bytes per slot) and
/// the whole TEB32 page at `PAGE_TEB` (its FS base).
pub const PAGE_TLS_ARRAY: usize = 256;
pub const PAGE_TLS_DATA: usize = 512;
pub const PAGE_TLS_BLOCK: usize = 256;
pub const PAGE_TEB: usize = 4096;
pub const THREAD_PAGES: usize = 2;
const _: () = assert!(PAGE_TEB + TEB_BYTES <= THREAD_PAGES * 4096);
const _: () = assert!(PAGE_TLS_DATA + 5 * PAGE_TLS_BLOCK <= PAGE_TEB);
/// The end of the SEH chain.
pub const CHAIN_END: u32 = 0xffff_ffff;

/// Initialize a 32-bit thread's TEB32 (`output`, at least `TEB_BYTES`):
/// the SEH chain empty, its stack bounds, self, ids, implicit TLS array and
/// PEB.
#[allow(clippy::too_many_arguments)]
pub fn initial_teb(
    output: &mut [u8],
    teb: u32,
    stack_base: u32,
    stack_limit: u32,
    process_id: u32,
    thread_id: u32,
    tls_array: u32,
    peb: u32,
) -> Result<(), crate::Error> {
    if output.len() < TEB_BYTES {
        return Err(crate::Error::Size);
    }
    if teb == 0 || peb == 0 || stack_limit == 0 || stack_limit >= stack_base {
        return Err(crate::Error::Address);
    }
    output[..TEB_BYTES].fill(0);
    for (offset, value) in [
        (TEB_EXCEPTION_LIST, CHAIN_END),
        (TEB_STACK_BASE, stack_base),
        (TEB_STACK_LIMIT, stack_limit),
        (TEB_SELF, teb),
        (TEB_PROCESS_ID, process_id),
        (TEB_THREAD_ID, thread_id),
        (TEB_TLS_POINTER, tls_array),
        (TEB_PEB, peb),
        (TEB_DEALLOCATION_STACK, stack_limit),
    ] {
        output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn teb32_holds_the_public_fields_where_windows_does() {
        let mut teb = [0xffu8; TEB_BYTES];
        initial_teb(
            &mut teb,
            0x7100_8000,
            0x7001_0000,
            0x7000_1000,
            4,
            8,
            0x7100_2100,
            0x7100_a000,
        )
        .unwrap();
        let word = |at: usize| u32::from_le_bytes(teb[at..at + 4].try_into().unwrap());
        assert_eq!(word(0), CHAIN_END);
        assert_eq!(word(0x18), 0x7100_8000);
        assert_eq!(word(0x30), 0x7100_a000);
        assert_eq!(word(0x2c), 0x7100_2100);
        assert_eq!((word(0x20), word(0x24)), (4, 8));
        assert_eq!(word(0x34), 0);
        assert!(
            teb[TEB_TLS_SLOTS..TEB_DEALLOCATION_STACK]
                .iter()
                .all(|b| *b == 0)
        );
        assert_eq!(word(0xE0C), 0x7000_1000);
        assert!(teb[TEB_WINDOWS_TLS_SLOTS..].iter().all(|b| *b == 0));
        assert_eq!(TEB_BYTES, 0x1000);
        assert!(initial_teb(&mut teb[..16], 1, 2, 1, 0, 0, 0, 1).is_err());
        assert!(initial_teb(&mut teb, 1, 1, 2, 0, 0, 0, 1).is_err());
    }
}
#[cfg(test)]
mod state_tests {
    #[test]
    fn x86_state_has_a_teb32_and_peb32_over_the_shared_parameters() {
        let mut state = [0u8; crate::STATE_BYTES];
        let path: [u16; 3] = [b'a' as u16, b'.' as u16, b'e' as u16];
        crate::initial_state32(
            &mut state,
            0x7004_1000,
            0x7000_1000,
            1,
            &path,
            &path,
            0x40_0000,
            0x7100_2100,
        )
        .unwrap();
        let word = |at: usize| u32::from_le_bytes(state[at..at + 4].try_into().unwrap());
        assert_eq!(word(super::TEB_SELF), crate::TEB_ADDRESS as u32);
        assert_eq!(word(super::TEB_PEB), crate::PEB_ADDRESS as u32);
        assert_eq!(word(super::TEB_TLS_POINTER), 0x7100_2100);
        assert_eq!(word(super::TEB_EXCEPTION_LIST), super::CHAIN_END);
        assert_eq!(word(8192 + super::PEB_IMAGE_BASE), 0x40_0000);
        assert_eq!(
            word(8192 + super::PEB_PROCESS_PARAMETERS),
            crate::PARAMETERS32_ADDRESS as u32
        );
        assert_eq!(word(8192 + super::PEB_SESSION_ID), 1);
        // Doors' own DLLs keep the x64 parameters block (command line at
        // 112); the program's PEB32 names a 32-bit one whose strings are
        // that block's.
        assert_eq!(
            u16::from_le_bytes([state[12288 + 112], state[12288 + 113]]),
            6
        );
        let p = (crate::PARAMETERS32_ADDRESS - crate::STATE_ADDRESS) as usize;
        assert_eq!(
            word(p + super::PARAMETERS32_LENGTH),
            super::PARAMETERS32_BYTES
        );
        assert_eq!(word(p + super::PARAMETERS32_FLAGS), 1);
        assert_eq!(
            u16::from_le_bytes([state[p + 0x40], state[p + 0x41]]),
            6,
            "CommandLine.Length"
        );
        assert_eq!(
            word(p + super::PARAMETERS32_COMMAND_LINE + 4),
            crate::PARAMETERS_ADDRESS as u32 + 512
        );
        assert_eq!(
            word(p + super::PARAMETERS32_IMAGE_PATH + 4),
            crate::PARAMETERS_ADDRESS as u32 + 256
        );
        assert!(
            crate::initial_state32(
                &mut state,
                0x7004_1000,
                0x7000_1000,
                1,
                &path,
                &path,
                0x1_0000_0000,
                0
            )
            .is_err()
        );
    }
}

//! Native API calls Chromium and Electron import from ntdll (P18 M7): the
//! loader lock, thread and object queries, RtlInitUnicodeString and the
//! last NTSTATUS. Registry keys belong to advapi32's service connection in
//! Doors, so NtDeleteKey reports the operation as not supported here.

const STATUS_SUCCESS: u32 = 0;
const STATUS_INVALID_PARAMETER: u32 = 0xC000_000D;
const STATUS_INVALID_HANDLE: u32 = 0xC000_0008;
const STATUS_INFO_LENGTH_MISMATCH: u32 = 0xC000_0004;
const STATUS_INVALID_INFO_CLASS: u32 = 0xC000_0003;
const STATUS_NOT_SUPPORTED: u32 = 0xC000_00BB;
const STATUS_NOT_LOCKED: u32 = 0xC000_002A;
const POINTER: usize = core::mem::size_of::<usize>();

fn put_bytes(pointer: u64, bytes: &[u8]) -> bool {
    pointer != 0 && super::guest::put(pointer, bytes).is_ok()
}
fn put_pointer(out: &mut [u8], at: usize, value: u64) {
    out[at..at + POINTER].copy_from_slice(&value.to_le_bytes()[..POINTER]);
}

/// LdrLockLoaderLock: `state` LDR_LOCK_LOADER_LOCK_DISPOSITION_LOCK_ACQUIRED
/// (1); the cookie names the owner thread.
#[unsafe(no_mangle)]
pub extern "system" fn LdrLockLoaderLock(flags: u32, state: usize, cookie: usize) -> u32 {
    let (state, cookie) = (
        door_windows_abi::widen(state),
        door_windows_abi::widen(cookie),
    );
    if flags & !0x3 != 0 || cookie == 0 {
        return STATUS_INVALID_PARAMETER;
    }
    super::ldr::lock_loader();
    let id = u64::from(super::threads::DoorGetCurrentThreadId());
    if state != 0 {
        let _ = super::guest::put(state, &1u32.to_le_bytes());
    }
    let _ = super::guest::put(cookie, &id.to_le_bytes()[..POINTER]);
    STATUS_SUCCESS
}
#[unsafe(no_mangle)]
pub extern "system" fn LdrUnlockLoaderLock(_flags: u32, cookie: usize) -> u32 {
    if cookie == 0 {
        return STATUS_SUCCESS;
    }
    if super::ldr::unlock_loader() {
        STATUS_SUCCESS
    } else {
        STATUS_NOT_LOCKED
    }
}
/// NtQueryInformationThread: ThreadBasicInformation (exit status, TEB for
/// the calling thread, client id, affinity, priorities) and the start
/// address class are answered.
#[unsafe(no_mangle)]
pub extern "system" fn NtQueryInformationThread(
    thread: usize,
    class: u32,
    info: usize,
    length: u32,
    returned: usize,
) -> u32 {
    let thread = door_windows_abi::widen(thread);
    let info = door_windows_abi::widen(info);
    let returned = door_windows_abi::widen(returned);
    let current = thread == u64::MAX - 1 || thread == u64::from(u32::MAX - 1);
    let id = if current {
        super::threads::DoorGetCurrentThreadId()
    } else {
        super::waits::DoorGetThreadId(thread)
    };
    if id == 0 {
        return STATUS_INVALID_HANDLE;
    }
    let mut out = [0u8; 48];
    let size = match class {
        0 => {
            // ExitStatus STATUS_PENDING, TebBaseAddress, ClientId,
            // AffinityMask, Priority, BasePriority.
            out[..4].copy_from_slice(&0x103u32.to_le_bytes());
            let teb = if current { super::arch::teb() } else { 0 };
            put_pointer(&mut out, POINTER, teb);
            put_pointer(
                &mut out,
                2 * POINTER,
                u64::from(super::guest::DoorGetCurrentProcessId()),
            );
            put_pointer(&mut out, 3 * POINTER, u64::from(id));
            let processors = super::sysinfo::DoorProcessorCount();
            let mask = if processors >= 64 {
                u64::MAX
            } else {
                (1u64 << processors) - 1
            };
            put_pointer(&mut out, 4 * POINTER, mask);
            let at = 5 * POINTER;
            out[at..at + 4].copy_from_slice(&8i32.to_le_bytes());
            out[at + 4..at + 8].copy_from_slice(&8i32.to_le_bytes());
            at + 8
        }
        // ThreadQuerySetWin32StartAddress: not recorded per thread.
        9 => POINTER,
        _ => return STATUS_INVALID_INFO_CLASS,
    };
    if (length as usize) < size {
        return STATUS_INFO_LENGTH_MISMATCH;
    }
    if !put_bytes(info, &out[..size]) {
        return STATUS_INVALID_PARAMETER;
    }
    if returned != 0 {
        let _ = super::guest::put(returned, &(size as u32).to_le_bytes());
    }
    STATUS_SUCCESS
}
/// NtQueryObject: basic information (all access granted, one handle) and
/// the type name, from the handle's kind.
#[unsafe(no_mangle)]
pub extern "system" fn NtQueryObject(
    handle: usize,
    class: u32,
    info: usize,
    length: u32,
    returned: usize,
) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let info = door_windows_abi::widen(info);
    let returned = door_windows_abi::widen(returned);
    let mut out = [0u8; 128];
    let size = match class {
        // OBJECT_BASIC_INFORMATION (56 bytes).
        0 => {
            out[4..8].copy_from_slice(&0x001f_ffffu32.to_le_bytes());
            out[8..12].copy_from_slice(&1u32.to_le_bytes());
            out[12..16].copy_from_slice(&1u32.to_le_bytes());
            56
        }
        // OBJECT_NAME_INFORMATION: an empty name.
        1 => 2 * POINTER,
        // OBJECT_TYPE_INFORMATION: the type name (UNICODE_STRING, then
        // its text after the fixed part).
        2 => {
            // Files and pipes are both File objects on Windows.
            let name: &str = if door_windows_abi::file::is_file(handle)
                || door_windows_abi::pipe::is_pipe(handle)
            {
                "File"
            } else {
                "Event"
            };
            let fixed = if POINTER == 8 { 104 } else { 96 };
            let text = fixed;
            let bytes = 2 * name.len();
            out[..2].copy_from_slice(&(bytes as u16).to_le_bytes());
            out[2..4].copy_from_slice(&(bytes as u16 + 2).to_le_bytes());
            put_pointer(&mut out, POINTER, info + text as u64);
            for (i, u) in name.encode_utf16().enumerate() {
                out[text + 2 * i..text + 2 * i + 2].copy_from_slice(&u.to_le_bytes());
            }
            text + bytes + 2
        }
        _ => return STATUS_INVALID_INFO_CLASS,
    };
    if returned != 0 {
        let _ = super::guest::put(returned, &(size as u32).to_le_bytes());
    }
    if (length as usize) < size {
        return STATUS_INFO_LENGTH_MISMATCH;
    }
    if !put_bytes(info, &out[..size]) {
        return STATUS_INVALID_PARAMETER;
    }
    STATUS_SUCCESS
}
#[unsafe(no_mangle)]
pub extern "system" fn NtDeleteKey(_key: usize) -> u32 {
    STATUS_NOT_SUPPORTED
}
/// RtlGetLastNtStatus: Doors' calls record the Win32 error only.
#[unsafe(no_mangle)]
pub extern "system" fn RtlGetLastNtStatus() -> u32 {
    STATUS_SUCCESS
}
/// RtlInitUnicodeString: Length and MaximumLength of the NUL-terminated
/// `source`, Buffer pointing at it.
#[unsafe(no_mangle)]
pub extern "system" fn RtlInitUnicodeString(destination: usize, source: usize) {
    let destination = door_windows_abi::widen(destination);
    let source = door_windows_abi::widen(source);
    let mut length = 0u64;
    if source != 0 {
        loop {
            let mut unit = [0u8; 2];
            if super::guest::copy(unit.as_mut_ptr() as u64, source + 2 * length, 2).is_err()
                || unit == [0, 0]
                || length >= 0x7ffe
            {
                break;
            }
            length += 1;
        }
    }
    let mut out = [0u8; 16];
    let bytes = (2 * length) as u16;
    out[..2].copy_from_slice(&bytes.to_le_bytes());
    out[2..4].copy_from_slice(&(if source == 0 { 0 } else { bytes + 2 }).to_le_bytes());
    put_pointer(&mut out, POINTER, source);
    let _ = super::guest::put(destination, &out[..2 * POINTER]);
}
/// RtlCaptureStackBackTrace (x86): the return addresses along the EBP
/// frame chain from the caller, after `skip`, within the thread's stack;
/// it stops at the first frame without a frame pointer.
#[cfg(target_arch = "x86")]
#[unsafe(no_mangle)]
pub extern "system" fn RtlCaptureStackBackTrace(
    skip: u32,
    count: u32,
    out: usize,
    hash: usize,
) -> u16 {
    let (base, limit, mut ebp): (u32, u32, u32);
    // SAFETY: FS is the TEB; reading EBP has no effect.
    unsafe {
        core::arch::asm!("mov {}, fs:[4]", out(reg) base, options(nostack, readonly, preserves_flags));
        core::arch::asm!("mov {}, fs:[8]", out(reg) limit, options(nostack, readonly, preserves_flags));
        core::arch::asm!("mov {}, ebp", out(reg) ebp, options(nomem, nostack, preserves_flags));
    }
    let mut captured = 0u32;
    let mut sum = 0u32;
    let mut frame = 0u32;
    while captured < count && frame < 1024 {
        if ebp < limit || ebp.checked_add(8).is_none_or(|end| end > base) || ebp % 4 != 0 {
            break;
        }
        // SAFETY: [ebp, ebp + 8) lies inside the thread's stack.
        let (next, ret) = unsafe { (*(ebp as *const u32), *((ebp + 4) as *const u32)) };
        if ret == 0 {
            break;
        }
        if frame >= skip && out != 0 {
            unsafe { ((out + 4 * captured as usize) as *mut u32).write_unaligned(ret) };
            sum = sum.wrapping_add(ret);
            captured += 1;
        }
        if next <= ebp {
            break;
        }
        ebp = next;
        frame += 1;
    }
    if hash != 0 {
        unsafe { (hash as *mut u32).write_unaligned(sum) };
    }
    captured as u16
}

/// NtCompareObjects: STATUS_SUCCESS when both handles name the same
/// kernel object (the same object and generation), else
/// STATUS_NOT_SAME_OBJECT; a value that is no kernel handle matches only
/// itself.
#[unsafe(no_mangle)]
pub extern "system" fn NtCompareObjects(first: usize, second: usize) -> u32 {
    const STATUS_SUCCESS: u32 = 0;
    const STATUS_NOT_SAME_OBJECT: u32 = 0xC000_01AC;
    let (a, b) = (
        door_windows_abi::widen(first),
        door_windows_abi::widen(second),
    );
    if a == b {
        return STATUS_SUCCESS;
    }
    match (door_sys::inspect(a), door_sys::inspect(b)) {
        (Ok(x), Ok(y)) if x[0] == y[0] && x[2] == y[2] && x[3] == y[3] => STATUS_SUCCESS,
        _ => STATUS_NOT_SAME_OBJECT,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ZwCompareObjects(first: usize, second: usize) -> u32 {
    NtCompareObjects(first, second)
}

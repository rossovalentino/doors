//! VirtualAlloc, VirtualFree, VirtualProtect and VirtualQuery over the
//! kernel's page-granular Windows memory (P18 M0b): every private
//! protection (including executable and guard pages; EXECUTE_READWRITE is
//! kept W^X per page by the kernel), MEM_TOP_DOWN, MEM_RESET and partial
//! commits and decommits. Write watches, large pages and AWE are refused.
use super::guest::{RtlSetLastWin32Error, copy};
use door_abi::{Status, operation as op};
use door_windows_abi::{error, memory as vm};
fn call(operation: u32, args: [u64; 6]) -> Result<u64, Status> {
    unsafe { door_sys::raw(operation, args) }.result()
}
const INVALID_ADDRESS: u32 = 487;
const BAD_LENGTH: u32 = 24;
fn fail(status: Status) -> u64 {
    RtlSetLastWin32Error(match status {
        Status::ResourceExhausted => error::NOT_ENOUGH_MEMORY,
        Status::AccessDenied => 5,
        Status::BadAddress | Status::NotFound | Status::AlreadyExists => INVALID_ADDRESS,
        Status::Unsupported => error::NOT_SUPPORTED,
        _ => error::INVALID_PARAMETER,
    });
    0
}
fn invalid() -> u64 {
    RtlSetLastWin32Error(error::INVALID_PARAMETER);
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorVirtualAlloc(
    address: u64,
    bytes: u64,
    flags: u32,
    protection: u32,
) -> u64 {
    if flags & (vm::LARGE_PAGES | vm::PHYSICAL | vm::WRITE_WATCH) != 0 {
        // Not provided; callers (the CLR's GC among them) fall back.
        RtlSetLastWin32Error(error::NOT_SUPPORTED);
        return 0;
    }
    if !vm::protection_valid(protection) {
        return invalid();
    }
    // MEM_RESET / MEM_RESET_UNDO stand alone: the pages stay as they are
    // (contents may be kept), the address comes back.
    if flags == vm::RESET || flags == vm::RESET_UNDO {
        return match vm::range(address, bytes, false) {
            Some((base, _)) => base,
            None => invalid(),
        };
    }
    let known = vm::COMMIT | vm::RESERVE | vm::TOP_DOWN;
    if flags & (vm::COMMIT | vm::RESERVE) == 0 || flags & !known != 0 {
        return invalid();
    }
    let reserve = flags & vm::RESERVE != 0 || address == 0;
    let Some((base, size)) = vm::range(address, bytes, reserve) else {
        return invalid();
    };
    let base = if reserve {
        let top = if flags & vm::TOP_DOWN != 0 {
            vm::RESERVE_TOP_DOWN
        } else {
            0
        };
        let address = if address == 0 { 0 } else { base };
        match call(
            op::VM_RESERVE,
            [address, size, top, u64::from(protection), 0, 0],
        ) {
            Ok(base) => base,
            Err(error) => return fail(error),
        }
    } else {
        base
    };
    if flags & vm::COMMIT != 0
        && let Err(error) = call(op::VM_COMMIT, [base, size, u64::from(protection), 0, 0, 0])
    {
        if reserve {
            let _ = call(op::VM_WINDOWS_RELEASE, [base, 0, 0, 0, 0, 0]);
        }
        return fail(error);
    }
    // A commit inside a reservation returns the page-rounded address.
    base
}
/// VirtualAlloc2 for the current process: placeholders
/// (MEM_RESERVE_PLACEHOLDER, MEM_REPLACE_PLACEHOLDER) and, through
/// `requirements` (MEM_ADDRESS_REQUIREMENTS: lowest, highest, alignment;
/// 0 for none), a reservation inside a range; otherwise as VirtualAlloc.
#[unsafe(no_mangle)]
pub extern "system" fn DoorVirtualAlloc2(
    address: u64,
    bytes: u64,
    flags: u32,
    protection: u32,
    requirements: u64,
) -> u64 {
    if flags & vm::RESERVE_PLACEHOLDER_TYPE != 0 {
        // MEM_RESERVE | MEM_RESERVE_PLACEHOLDER, PAGE_NOACCESS.
        if flags != vm::RESERVE | vm::RESERVE_PLACEHOLDER_TYPE || protection != 0x01 {
            return invalid();
        }
        let Some((base, size)) = vm::range(address, bytes, true) else {
            return invalid();
        };
        let address = if address == 0 {
            match within(requirements, size) {
                Ok(at) => at,
                Err(error) => return fail(error),
            }
        } else {
            base
        };
        return match call(
            op::VM_RESERVE,
            [address, size, vm::RESERVE_PLACEHOLDER, 0x01, 0, 0],
        ) {
            Ok(base) => base,
            Err(error) => fail(error),
        };
    }
    if flags & vm::REPLACE_PLACEHOLDER != 0 {
        // MEM_RESERVE | MEM_REPLACE_PLACEHOLDER (| MEM_COMMIT): the
        // placeholder of exactly this range becomes an allocation.
        let known = vm::RESERVE | vm::REPLACE_PLACEHOLDER | vm::COMMIT;
        if flags & !known != 0 || flags & vm::RESERVE == 0 || !vm::protection_valid(protection) {
            return invalid();
        }
        if let Err(error) = call(
            op::VM_RESERVE,
            [
                address,
                bytes,
                vm::RESERVE_REPLACE,
                u64::from(protection),
                0,
                0,
            ],
        ) {
            return fail(error);
        }
        if flags & vm::COMMIT != 0
            && let Err(error) = call(
                op::VM_COMMIT,
                [address, bytes, u64::from(protection), 0, 0, 0],
            )
        {
            // Back to the placeholder it was.
            let _ = call(
                op::VM_WINDOWS_RELEASE,
                [address, bytes, vm::RELEASE_PRESERVE_PLACEHOLDER, 0, 0, 0],
            );
            return fail(error);
        }
        return address;
    }
    if address == 0 && requirements != 0 && flags & vm::RESERVE != 0 {
        let Some((_, size)) = vm::range(0, bytes, true) else {
            return invalid();
        };
        return match within(requirements, size) {
            Ok(at) => DoorVirtualAlloc(at, size, flags, protection),
            Err(error) => fail(error),
        };
    }
    DoorVirtualAlloc(address, bytes, flags, protection)
}
/// A free, aligned address for `bytes` inside MEM_ADDRESS_REQUIREMENTS at
/// `requirements` (0: anywhere, which is 0).
fn within(requirements: u64, bytes: u64) -> Result<u64, Status> {
    if requirements == 0 {
        return Ok(0);
    }
    let mut record = [0u8; 24];
    copy(record.as_mut_ptr() as u64, requirements, record.len()).map_err(|_| Status::BadAddress)?;
    let field = |i: usize| u64::from_le_bytes(record[i * 8..i * 8 + 8].try_into().unwrap());
    let (lowest, highest, alignment) = (field(0), field(1), field(2));
    if lowest == 0 && highest == 0 && alignment == 0 {
        return Ok(0);
    }
    let alignment = alignment.max(0x1_0000);
    if !alignment.is_power_of_two() {
        return Err(Status::InvalidArgument);
    }
    let highest = if highest == 0 {
        0x7FFF_FFFE_FFFF
    } else {
        highest
    };
    let mut at = lowest.max(0x1_0000).next_multiple_of(alignment);
    let mut info = [0u8; vm::QUERY_BYTES];
    while at.checked_add(bytes).is_some_and(|end| end - 1 <= highest) {
        call(op::VM_QUERY, [at, info.as_mut_ptr() as u64, 0, 0, 0, 0])?;
        let q = vm::BasicInformation::decode(&info);
        if q.state == 0x1_0000 && q.base + q.size >= at + bytes {
            return Ok(at);
        }
        at = (q.base + q.size)
            .max(at + alignment)
            .next_multiple_of(alignment);
    }
    Err(Status::ResourceExhausted)
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorVirtualFree(address: u64, bytes: u64, flags: u32) -> i32 {
    if flags == vm::RELEASE | vm::PRESERVE_PLACEHOLDER
        || flags == vm::RELEASE | vm::COALESCE_PLACEHOLDERS
    {
        let kind = if flags & vm::PRESERVE_PLACEHOLDER != 0 {
            vm::RELEASE_PRESERVE_PLACEHOLDER
        } else {
            vm::RELEASE_COALESCE_PLACEHOLDERS
        };
        let Some((base, size)) = vm::range(address, bytes, false) else {
            invalid();
            return 0;
        };
        return match call(op::VM_WINDOWS_RELEASE, [base, size, kind, 0, 0, 0]) {
            Ok(_) => 1,
            Err(error) => {
                fail(error);
                0
            }
        };
    }
    let result = if flags == vm::RELEASE {
        if bytes != 0 {
            invalid();
            return 0;
        }
        call(op::VM_WINDOWS_RELEASE, [address, 0, 0, 0, 0, 0])
    } else if flags == vm::DECOMMIT {
        let (base, size) = if bytes == 0 {
            (address, 0)
        } else {
            let Some(range) = vm::range(address, bytes, false) else {
                invalid();
                return 0;
            };
            range
        };
        call(op::VM_DECOMMIT, [base, size, 0, 0, 0, 0])
    } else {
        Err(Status::InvalidArgument)
    };
    match result {
        Ok(_) => 1,
        Err(error) => {
            fail(error);
            0
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorVirtualProtect(
    address: u64,
    bytes: u64,
    protection: u32,
    previous: u64,
) -> i32 {
    if !vm::protection_valid(protection) {
        invalid();
        return 0;
    }
    let Some((base, size)) = vm::range(address, bytes, false) else {
        invalid();
        return 0;
    };
    // The old-protection output must be writable before anything changes.
    let mut old = 0u32;
    if copy(&mut old as *mut u32 as u64, previous, 4).is_err()
        || copy(previous, &old as *const u32 as u64, 4).is_err()
    {
        RtlSetLastWin32Error(error::NOACCESS);
        return 0;
    }
    match call(
        op::VM_WINDOWS_PROTECT,
        [base, size, u64::from(protection), 0, 0, 0],
    ) {
        Ok(value) => {
            let value = value as u32;
            if copy(previous, &value as *const u32 as u64, 4).is_err() {
                RtlSetLastWin32Error(error::NOACCESS);
                return 0;
            }
            1
        }
        Err(error) => {
            fail(error);
            0
        }
    }
}
/// VirtualQuery: the 48-byte MEMORY_BASIC_INFORMATION at `buffer`; the
/// bytes written, or 0 with the last error set.
#[unsafe(no_mangle)]
pub extern "system" fn DoorVirtualQuery(address: u64, buffer: u64, length: u64) -> u64 {
    if length < vm::QUERY_BYTES as u64 {
        RtlSetLastWin32Error(BAD_LENGTH);
        return 0;
    }
    match call(op::VM_QUERY, [address, buffer, 0, 0, 0, 0]) {
        Ok(_) => vm::QUERY_BYTES as u64,
        Err(Status::BadAddress) => {
            RtlSetLastWin32Error(error::NOACCESS);
            0
        }
        Err(_) => invalid(),
    }
}
/// FlushProcessWriteBuffers: every CPU running this process serializes.
#[unsafe(no_mangle)]
pub extern "system" fn DoorFlushProcessWriteBuffers() {
    let _ = call(op::VM_FLUSH_WRITE_BUFFERS, [0; 6]);
}

/// The process's section window shared by the compatibility DLLs (window
/// surfaces in user32, audio buffers in winmm, ...): 4 MiB granules over
/// `0x7200_0000..0x7E00_0000`; the application asset view lives above.
const SECTION_WINDOW: u64 = 0x7200_0000;
const SECTION_GRANULE: u64 = 4 << 20;
const SECTION_GRANULES: u32 = 48;
static SECTION_RANGES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Reserves an unused range of the section window large enough for
/// `bytes`; 0 when none is free. The caller maps a section there and
/// gives the range back with `DoorReleaseSectionRange`.
#[unsafe(no_mangle)]
pub extern "system" fn DoorReserveSectionRange(bytes: u64) -> u64 {
    use core::sync::atomic::Ordering;
    let count = bytes.div_ceil(SECTION_GRANULE) as u32;
    if count == 0 || count > SECTION_GRANULES {
        return 0;
    }
    loop {
        let old = SECTION_RANGES.load(Ordering::Acquire);
        let Some((mask, index)) = (0..=SECTION_GRANULES - count)
            .map(|i| ((((1u64 << count) - 1) << i), i))
            .find(|(m, _)| old & m == 0)
        else {
            return 0;
        };
        if SECTION_RANGES
            .compare_exchange(old, old | mask, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return SECTION_WINDOW + u64::from(index) * SECTION_GRANULE;
        }
    }
}
/// Gives back a range `DoorReserveSectionRange` returned for `bytes`
/// (after its mapping is gone). FALSE for a range it did not hand out.
#[unsafe(no_mangle)]
pub extern "system" fn DoorReleaseSectionRange(address: u64, bytes: u64) -> i32 {
    use core::sync::atomic::Ordering;
    let count = bytes.div_ceil(SECTION_GRANULE) as u32;
    let Some(offset) = address
        .checked_sub(SECTION_WINDOW)
        .filter(|o| o % SECTION_GRANULE == 0)
    else {
        return 0;
    };
    let index = (offset / SECTION_GRANULE) as u32;
    if count == 0 || index + count > SECTION_GRANULES {
        return 0;
    }
    let mask = ((1u64 << count) - 1) << index;
    i32::from(SECTION_RANGES.fetch_and(!mask, Ordering::AcqRel) & mask == mask)
}

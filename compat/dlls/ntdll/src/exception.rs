//! Initial vectored fault dispatch. No Windows exception unwinds Rust/kernel
//! frames: successful continuation restores a validated native user trap frame.
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, Ordering},
};
use door_windows_abi::status;

#[derive(Clone, Copy)]
struct Handler {
    function: u64,
    token: u64,
}
impl Handler {
    const EMPTY: Self = Self {
        function: 0,
        token: 0,
    };
}
struct Registry {
    entries: [Handler; 8],
    count: usize,
    next: u64,
}
struct State {
    locked: AtomicBool,
    registry: UnsafeCell<Registry>,
}
unsafe impl Sync for State {}
static STATE: State = State {
    locked: AtomicBool::new(false),
    registry: UnsafeCell::new(Registry {
        entries: [Handler::EMPTY; 8],
        count: 0,
        next: 1,
    }),
};
struct Lease;
impl Lease {
    fn acquire() -> Result<Self, u32> {
        let deadline = door_sys::time()
            .map_err(|_| status::IO_DEVICE_ERROR)?
            .saturating_add(100);
        loop {
            if STATE
                .locked
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return Ok(Self);
            }
            if door_sys::time().map_err(|_| status::IO_DEVICE_ERROR)? >= deadline {
                return Err(status::IO_DEVICE_ERROR);
            }
            door_sys::yield_now().map_err(|_| status::IO_DEVICE_ERROR)?;
        }
    }
    fn value(&mut self) -> &mut Registry {
        unsafe { &mut *STATE.registry.get() }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        STATE.locked.store(false, Ordering::Release);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn RtlAddVectoredExceptionHandler(first: u32, function: usize) -> usize {
    let function = door_windows_abi::widen(function);
    door_windows_abi::narrow((move || -> u64 {
        if !(0x10000..0x8000_0000_0000).contains(&function) {
            return 0;
        }
        let Ok(mut lease) = Lease::acquire() else {
            return 0;
        };
        let registry = lease.value();
        if registry.count == registry.entries.len() {
            return 0;
        }
        let Some(next) = registry.next.checked_add(1) else {
            return 0;
        };
        let token = registry.next;
        registry.next = next;
        let index = if first != 0 {
            registry.entries.copy_within(0..registry.count, 1);
            0
        } else {
            registry.count
        };
        registry.entries[index] = Handler { function, token };
        registry.count += 1;
        token
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn RtlRemoveVectoredExceptionHandler(token: usize) -> u32 {
    let token = door_windows_abi::widen(token);
    {
        let Ok(mut lease) = Lease::acquire() else {
            return 0;
        };
        let registry = lease.value();
        let Some(index) = registry.entries[..registry.count]
            .iter()
            .position(|handler| handler.token == token)
        else {
            return 0;
        };
        registry
            .entries
            .copy_within(index + 1..registry.count, index);
        registry.count -= 1;
        registry.entries[registry.count] = Handler::EMPTY;
        1
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn NtContinue(context: usize, test_alert: u8) -> u32 {
    let context = door_windows_abi::widen(context);
    {
        if test_alert != 0 {
            return status::NOT_SUPPORTED;
        }
        match unsafe {
            door_sys::raw(
                door_abi::operation::CONTEXT_RESUME,
                [context, 0, 0, 0, 0, 0],
            )
        }
        .result()
        {
            Ok(_) => status::SUCCESS,
            Err(door_abi::Status::BadAddress) => status::ACCESS_VIOLATION,
            Err(door_abi::Status::AccessDenied) => 0xc000_0022,
            Err(_) => status::INVALID_PARAMETER,
        }
    }
}
/// The vectored handlers' functions, in call order (0 ends the list).
#[cfg(target_arch = "x86")]
pub(crate) fn vectored() -> Option<[u64; 8]> {
    let mut lease = Lease::acquire().ok()?;
    Some(lease.value().entries.map(|handler| handler.function))
}
/// The vectored handlers in order (x64, P18): true when one returned
/// EXCEPTION_CONTINUE_EXECUTION. A handler returning anything but
/// EXCEPTION_CONTINUE_SEARCH or _EXECUTION ends the process, as before.
#[cfg(target_arch = "x86_64")]
pub(crate) fn vectored_continue(record: u64, context: u64) -> bool {
    let handlers = {
        let Ok(mut lease) = Lease::acquire() else {
            door_sys::exit(-320);
        };
        lease.value().entries
    };
    let mut pointers = [record, context];
    for handler in handlers {
        if handler.function == 0 {
            break;
        }
        let result = super::arch::call_one(handler.function, pointers.as_mut_ptr() as u64) as i32;
        if result == -1 {
            return true;
        }
        if result != 0 {
            door_sys::exit(-322);
        }
    }
    false
}

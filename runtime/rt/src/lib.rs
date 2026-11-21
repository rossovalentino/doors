#![no_std]
pub mod heap;
pub mod resources;
pub mod thread;
pub use door_abi::{Startup, Status};
pub use door_sys as sys;
pub use heap::HeapAllocator;

/// Captures the complete bounded platform startup record before parsing it.
/// # Safety
/// The trusted loader supplies a readable immutable record for this invocation.
pub unsafe fn startup_v2(
    pointer: *const u8,
    length: usize,
) -> Result<door_abi::platform::StartupV2, Status> {
    if pointer.is_null() || length != door_abi::platform::STARTUP_V2_BYTES {
        return Err(Status::InvalidArgument);
    }
    let mut bytes = [0; door_abi::platform::STARTUP_V2_BYTES];
    unsafe {
        core::ptr::copy_nonoverlapping(pointer, bytes.as_mut_ptr(), length);
    }
    door_abi::platform::StartupV2::decode(&bytes)
}

/// Entry for the versioned native platform. Heap initialization is explicit.
#[macro_export]
macro_rules! entry_v2 {
    ($main:path) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn door_user_entry(pointer: *const u8, length: usize) -> ! {
            let startup = match unsafe { $crate::startup_v2(pointer, length) } {
                Ok(value) => value,
                Err(_) => $crate::sys::exit(-100),
            };
            let code = $main(startup);
            $crate::thread::run_destructors();
            $crate::sys::exit(code)
        }
    };
}

/// Copies the fixed startup record into process-owned stack storage before use.
/// # Safety
/// `pointer` must denote `length` readable stable bytes supplied by the trusted
/// loader. The loader initializes a guarded aligned stack and distinct FS TLS.
pub unsafe fn startup(pointer: *const u8, length: usize) -> Result<Startup, Status> {
    if pointer.is_null() || length != door_abi::STARTUP_BYTES {
        return Err(Status::InvalidArgument);
    }
    let mut owned = [0u8; door_abi::STARTUP_BYTES];
    unsafe {
        core::ptr::copy_nonoverlapping(pointer, owned.as_mut_ptr(), length);
    }
    Startup::decode(&owned)
}

/// Establishes the sole executable entry. The loader presents SysV function-entry
/// stack alignment (RSP mod16=8), RDI=startup pointer, RSI=byte count, and FS TLS.
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn door_user_entry(pointer: *const u8, length: usize) -> ! {
            let startup = match unsafe { $crate::startup(pointer, length) } {
                Ok(startup) => startup,
                Err(_) => $crate::sys::exit(-100),
            };
            let code = $main(startup);
            $crate::sys::exit(code)
        }
    };
}

pub fn panic() -> ! {
    let _ = sys::write(b"NATIVE PANIC\n");
    sys::exit(-101)
}

/// Optional, allocation-free diagnostic capture. A complete formatted record
/// must fit before one syscall is attempted; exhausted logging never gates work.
pub fn diagnostic(arguments: core::fmt::Arguments<'_>) {
    use core::fmt::Write;
    struct Record {
        bytes: [u8; 256],
        len: usize,
    }
    impl core::fmt::Write for Record {
        fn write_str(&mut self, text: &str) -> core::fmt::Result {
            let end = self.len.checked_add(text.len()).ok_or(core::fmt::Error)?;
            if end > self.bytes.len() {
                return Err(core::fmt::Error);
            }
            self.bytes[self.len..end].copy_from_slice(text.as_bytes());
            self.len = end;
            Ok(())
        }
    }
    let mut record = Record {
        bytes: [0; 256],
        len: 0,
    };
    if record.write_fmt(arguments).is_ok() {
        let _ = sys::write(&record.bytes[..record.len]);
    }
}

/// Allocation-free diagnostics, including when the allocator is exhausted.
pub fn panic_with_info(info: &core::panic::PanicInfo<'_>) -> ! {
    use core::fmt::Write;
    struct Diagnostic;
    impl core::fmt::Write for Diagnostic {
        fn write_str(&mut self, value: &str) -> core::fmt::Result {
            let _ = sys::write(value.as_bytes());
            Ok(())
        }
    }
    let _ = writeln!(Diagnostic, "NATIVE PANIC: {info}");
    sys::exit(-101)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_rejects_length_before_dereference() {
        assert_eq!(
            unsafe { startup(core::ptr::null(), 256) },
            Err(Status::InvalidArgument)
        );
        assert_eq!(
            unsafe { startup(core::ptr::dangling(), 255) },
            Err(Status::InvalidArgument)
        );
    }
}

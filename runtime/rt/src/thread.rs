//! Native thread entry and bounded per-thread destructors. TLS is an owned page
//! initialized by the kernel; no host TLS or libc is used in the guest.
use door_abi::{Handle, Status};
pub const TLS_RUNTIME_OFFSET: usize = 64;
#[cfg(any(test, all(target_arch = "x86_64", target_os = "none")))]
const MAX_DESTRUCTORS: usize = 8;
#[cfg(any(test, all(target_arch = "x86_64", target_os = "none")))]
#[derive(Clone, Copy)]
struct Destructor {
    callback: Option<unsafe extern "C" fn(u64)>,
    argument: u64,
}
#[cfg(any(test, all(target_arch = "x86_64", target_os = "none")))]
#[repr(C)]
struct Local {
    value: u64,
    closing: u64,
    destructors: [Destructor; MAX_DESTRUCTORS],
}
#[cfg(any(test, all(target_arch = "x86_64", target_os = "none")))]
impl Local {
    fn register(
        &mut self,
        callback: unsafe extern "C" fn(u64),
        argument: u64,
    ) -> Result<(), Status> {
        if self.closing != 0 {
            return Err(Status::Canceled);
        }
        let entry = self
            .destructors
            .iter_mut()
            .find(|d| d.callback.is_none())
            .ok_or(Status::ResourceExhausted)?;
        *entry = Destructor {
            callback: Some(callback),
            argument,
        };
        Ok(())
    }
    fn take(&mut self, index: usize) -> Destructor {
        core::mem::replace(
            &mut self.destructors[index],
            Destructor {
                callback: None,
                argument: 0,
            },
        )
    }
}
#[cfg(all(target_arch = "x86_64", target_os = "none"))]
fn local() -> *mut Local {
    let base: u64;
    unsafe {
        core::arch::asm!("mov {}, fs:[0]",out(reg)base,options(nostack,readonly,preserves_flags));
    }
    (base as usize + TLS_RUNTIME_OFFSET) as *mut Local
}
pub fn set_local(value: u64) -> Result<(), Status> {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        unsafe {
            (*local()).value = value;
        }
        Ok(())
    }
    #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
    {
        let _ = value;
        Err(Status::Unsupported)
    }
}
pub fn get_local() -> Result<u64, Status> {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        Ok(unsafe { (*local()).value })
    }
    #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
    {
        Err(Status::Unsupported)
    }
}
/// # Safety
/// Callback and argument remain valid until this thread exits; callback must not
/// unwind across the C boundary or refer to another thread's expired stack.
pub unsafe fn register_destructor(
    callback: unsafe extern "C" fn(u64),
    argument: u64,
) -> Result<(), Status> {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        unsafe { (*local()).register(callback, argument) }
    }
    #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
    {
        let _ = (callback, argument);
        Err(Status::Unsupported)
    }
}
pub fn run_destructors() {
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    {
        unsafe {
            (*local()).closing = 1;
        }
        for i in (0..MAX_DESTRUCTORS).rev() {
            // Clear before invocation, with no TLS reference retained across callback.
            let d = unsafe { (*local()).take(i) };
            if let Some(callback) = d.callback {
                unsafe {
                    callback(d.argument);
                }
            }
        }
    }
}

/// Raw native thread entry. Call `finish` to run TLS destructors and retire.
/// # Safety
/// Entry must be a nonreturning executable function; argument ownership must
/// survive until join, and shared data must use suitable synchronization.
pub unsafe fn spawn(
    entry: unsafe extern "C" fn(u64) -> !,
    argument: u64,
) -> Result<Handle, Status> {
    unsafe { door_sys::thread_create(entry as *const () as u64, argument) }
}
pub fn join(handle: Handle, deadline: u64) -> Result<i64, Status> {
    door_sys::thread_join(handle, deadline)
}
pub fn finish(code: i64) -> ! {
    run_destructors();
    door_sys::thread_exit(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_tls_fits_owned_page_and_host_cannot_claim_guest_tls() {
        assert!(TLS_RUNTIME_OFFSET + core::mem::size_of::<Local>() <= 4096);
        assert_eq!(get_local(), Err(Status::Unsupported));
        assert_eq!(set_local(3), Err(Status::Unsupported));
        assert_eq!(core::mem::size_of::<Destructor>(), 16);
    }
    #[test]
    fn destructors_are_bounded_removed_before_callbacks_and_cannot_rearm() {
        unsafe extern "C" fn callback(_argument: u64) {}
        let mut local = Local {
            value: 0,
            closing: 0,
            destructors: [Destructor {
                callback: None,
                argument: 0,
            }; MAX_DESTRUCTORS],
        };
        for i in 0..MAX_DESTRUCTORS {
            local.register(callback, i as u64).unwrap();
        }
        assert_eq!(local.register(callback, 9), Err(Status::ResourceExhausted));
        local.closing = 1;
        assert_eq!(local.register(callback, 9), Err(Status::Canceled));
        for i in (0..MAX_DESTRUCTORS).rev() {
            let d = local.take(i);
            assert_eq!(d.argument, i as u64);
            assert!(d.callback.is_some());
            assert!(local.take(i).callback.is_none());
        }
    }
}

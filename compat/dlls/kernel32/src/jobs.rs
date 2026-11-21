//! Job objects as programs use them to follow the processes they start
//! (PowerShell's Start-Process -Wait, .NET's Process tree waits): a job is
//! a kernel event handle (so CloseHandle and DuplicateHandle work on it)
//! recorded here with the process handles assigned to it. Basic accounting
//! reports how many of them still run; limits are accepted and kept by the
//! programs themselves (kill-on-close and the limits proper are M0k's,
//! with the kernel's process tree).
use core::sync::atomic::{AtomicBool, Ordering};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorCreateEventW(security: u64, manual: i32, initial: i32, name: u64) -> u64;
    fn DoorDuplicateKernelHandle(handle: u64, target: u64) -> i32;
    fn DoorWaitForSingleObjectEx(handle: u64, millis: u32, alertable: i32) -> u32;
    fn RtlSetLastWin32Error(value: u32);
}

const JOBS: usize = 16;
const MEMBERS: usize = 32;
#[derive(Clone, Copy)]
struct Job {
    handle: u64,
    members: [u64; MEMBERS],
    count: usize,
    total: u32,
}
struct Table {
    held: AtomicBool,
    jobs: core::cell::UnsafeCell<[Option<Job>; JOBS]>,
}
unsafe impl Sync for Table {}
static TABLE: Table = Table {
    held: AtomicBool::new(false),
    jobs: core::cell::UnsafeCell::new([None; JOBS]),
};
fn with<R>(f: impl FnOnce(&mut [Option<Job>; JOBS]) -> R) -> R {
    while TABLE.held.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    // SAFETY: `held` gives this closure the only access.
    let r = f(unsafe { &mut *TABLE.jobs.get() });
    TABLE.held.store(false, Ordering::Release);
    r
}
fn fail(code: u32) -> i32 {
    unsafe { RtlSetLastWin32Error(code) };
    0
}
/// Whether `handle` is a job this process made.
pub(crate) fn is(handle: u64) -> bool {
    handle != 0 && with(|jobs| jobs.iter().flatten().any(|j| j.handle == handle))
}

/// `CreateJobObjectW`: an unnamed job (a named one is the same here, as no
/// other process can open it before M0k).
#[unsafe(no_mangle)]
pub extern "system" fn CreateJobObjectW(_security: usize, _name: usize) -> usize {
    let handle = unsafe { DoorCreateEventW(0, 1, 0, 0) };
    if handle == 0 {
        return 0;
    }
    let placed = with(|jobs| {
        let slot = jobs.iter_mut().find(|j| j.is_none())?;
        *slot = Some(Job {
            handle,
            members: [0; MEMBERS],
            count: 0,
            total: 0,
        });
        Some(())
    });
    if placed.is_none() {
        crate::guest::CloseHandle(door_windows_abi::narrow(handle));
        fail(8);
        return 0;
    }
    door_windows_abi::narrow(handle)
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateJobObjectA(security: usize, name: usize) -> usize {
    CreateJobObjectW(security, name)
}
/// `AssignProcessToJobObject`: the job keeps its own handle to the process.
#[unsafe(no_mangle)]
pub extern "system" fn AssignProcessToJobObject(job: usize, process: usize) -> i32 {
    let job = door_windows_abi::widen(job);
    let process = door_windows_abi::widen(process);
    let mut own = 0u64;
    if process == door_windows_abi::CURRENT_PROCESS {
        own = process;
    } else if unsafe { DoorDuplicateKernelHandle(process, &mut own as *mut u64 as u64) } == 0 {
        return 0;
    }
    let done = with(|jobs| {
        let j = jobs.iter_mut().flatten().find(|j| j.handle == job)?;
        if j.count == MEMBERS {
            return None;
        }
        j.members[j.count] = own;
        j.count += 1;
        j.total += 1;
        Some(())
    });
    if done.is_none() {
        return fail(6);
    }
    1
}
/// Limits and notifications are accepted (see the module comment).
#[unsafe(no_mangle)]
pub extern "system" fn SetInformationJobObject(
    job: usize,
    _class: u32,
    _info: usize,
    _length: u32,
) -> i32 {
    if !is(door_windows_abi::widen(job)) {
        return fail(6);
    }
    1
}
/// JobObjectBasicAccountingInformation (1): the processes assigned and the
/// ones still running. Another class of a job of ours is
/// ERROR_INVALID_PARAMETER; the caller's own job (NULL) or another handle
/// is ERROR_INVALID_HANDLE, as no process runs in a job it did not make.
pub(crate) fn query(job: u64, class: u32, info: usize, length: u32, returned: usize) -> i32 {
    let Some((members, count, total)) = with(|jobs| {
        jobs.iter()
            .flatten()
            .find(|j| j.handle == job && job != 0)
            .map(|j| (j.members, j.count, j.total))
    }) else {
        return fail(6);
    };
    if class != 1 {
        return fail(87);
    }
    if info == 0 || length < 48 {
        return fail(24);
    }
    let active = members[..count]
        .iter()
        .filter(|p| unsafe { DoorWaitForSingleObjectEx(**p, 0, 0) } == 258)
        .count() as u32;
    let mut out = [0u8; 48];
    out[36..40].copy_from_slice(&total.to_le_bytes());
    out[40..44].copy_from_slice(&active.to_le_bytes());
    out[44..48].copy_from_slice(&(total - active).to_le_bytes());
    unsafe { core::ptr::copy_nonoverlapping(out.as_ptr(), info as *mut u8, 48) };
    if returned != 0 {
        unsafe { (returned as *mut u32).write_unaligned(48) };
    }
    1
}
/// A job's handle closes: its member handles go with it.
pub(crate) fn close(handle: u64) {
    let members = with(|jobs| {
        let slot = jobs
            .iter_mut()
            .find(|j| j.is_some_and(|j| j.handle == handle))?;
        let job = slot.take()?;
        Some((job.members, job.count))
    });
    if let Some((members, count)) = members {
        for member in &members[..count] {
            if *member != door_windows_abi::CURRENT_PROCESS {
                crate::guest::CloseHandle(door_windows_abi::narrow(*member));
            }
        }
    }
}
/// TerminateJobObject: every process assigned to the job ends with `code`.
#[unsafe(no_mangle)]
pub extern "system" fn TerminateJobObject(job: usize, code: u32) -> i32 {
    let job = door_windows_abi::widen(job);
    let members = with(|jobs| {
        jobs.iter()
            .flatten()
            .find(|j| j.handle == job)
            .map(|j| (j.members, j.count))
    });
    let Some((members, count)) = members else {
        return fail(6);
    };
    for &process in &members[..count] {
        if process != 0 {
            crate::application::TerminateProcess(door_windows_abi::narrow(process), code);
        }
    }
    1
}

//! Per-thread construction, stable joins and owned wait leases.
use super::*;

pub(super) fn thread_create(
    r: Record,
    entry: u64,
    argument: u64,
    flags: u64,
) -> Result<u64, Status> {
    if flags > 1 {
        return Err(Status::InvalidArgument);
    }
    let policy = abi::manifest_policy(r.role - 100).ok_or(Status::AccessDenied)?;
    with_meta(|m| {
        let count = m
            .records
            .iter()
            .flatten()
            .filter(|record| record.pid == r.pid)
            .count() as u32;
        let process = m.processes[r.pid.slot()].as_mut().ok_or(Status::Canceled)?;
        if process.personality == Personality::WindowsX64
            && !matches!(
                process.identity.manifest,
                abi::manifest::WINDOWS_BASE
                    | abi::manifest::WINDOWS_APPLICATION
                    | abi::manifest::WINDOWS_GUI
            )
        {
            return Err(Status::AccessDenied);
        }
        if process.stopping.is_some() {
            return Err(Status::Canceled);
        }
        if count + process.creating_threads >= policy.max_threads {
            return Err(Status::ResourceExhausted);
        }
        process.creating_threads += 1;
        Ok(())
    })?;
    let result = create_thread_inner(r, entry, argument, flags == 1);
    with_meta(|m| m.processes[r.pid.slot()].as_mut().unwrap().creating_threads -= 1);
    result
}
fn create_thread_inner(
    r: Record,
    entry: u64,
    argument: u64,
    suspended: bool,
) -> Result<u64, Status> {
    let key = next(&THREAD_KEY)?;
    let windows = matches!(r.role, 107 | 110 | 115);
    let (memory, layout, x86) = with_vm(|v| {
        let slot = v.vacant_thread()?;
        let layout =
            unsafe { v.create_thread(r.pid.slot(), slot, entry, r.pid.raw(), key, windows) }?;
        Ok::<_, Status>((slot, layout, windows && v.windows_x86(r.pid.slot())))
    })?;
    let mut task = None;
    let mut event = None;
    let mut handle = 0;
    let mut lease = None;
    let outcome = (|| {
        let context = unsafe {
            arch::create_user_context(arch::UserContext {
                // A 32-bit thread: FS is its TEB32, entered in compatibility
                // mode with the argument in ECX.
                compat: x86,
                entry: layout.entry,
                stack_pointer: layout.user_stack_top
                    - if x86 {
                        16
                    } else if windows {
                        40
                    } else {
                        8
                    },
                fs_base: if x86 {
                    layout.tls + door_windows_abi::x86::PAGE_TEB as u64
                } else {
                    layout.tls
                },
                user_gs_base: if windows && !x86 {
                    layout.tls + door_windows_abi::THREAD_TEB_FS_OFFSET as u64
                } else {
                    0
                },
                cr3: layout.cr3,
                kernel_stack_top: layout.kernel_stack_top,
                arguments: if windows {
                    arch::EntryArguments::Windows {
                        bootstrap: argument,
                    }
                } else {
                    arch::EntryArguments::Native {
                        startup: argument,
                        bytes: 0,
                    }
                },
            })
        }
        .map_err(|_| Status::InvalidArgument)?;
        event = Some(run::with(|s| s.create_event(true, false)).map_err(task_error)?);
        let created = run::with(|s| s.create(context as u64, 4, 1)).map_err(task_error)?;
        task = Some(created);
        let (grant, retained) =
            with_meta(|m| m.executive.create_thread(r.pid, key, event.unwrap()))?;
        handle = grant;
        lease = Some(retained);
        with_meta(|m| {
            if m.processes[r.pid.slot()]
                .as_ref()
                .unwrap()
                .stopping
                .is_some()
            {
                return Err(Status::Canceled);
            }
            m.records[created.slot()] = Some(Record {
                pid: r.pid,
                task: created,
                role: r.role,
                exited: event.unwrap(),
            });
            m.threads[created.slot()] = Some(Thread {
                key,
                memory: Some(memory),
                lease: lease.take(),
                reaping: false,
                exception: None,
                suspends: u32::from(suspended),
                own_exit: None,
            });
            Ok::<(), Status>(())
        })?;
        if suspended {
            // CREATE_SUSPENDED: started, but not runnable until ResumeThread.
            run::with(|s| s.suspend(created)).expect("constructed secondary task suspend");
        }
        run::with(|s| s.start(created)).expect("constructed secondary task start");
        Ok(handle)
    })();
    if outcome.is_err() {
        if let Some(task) = task {
            run::with(|s| {
                s.terminate(task, -1).unwrap();
                s.reap(task).unwrap();
            });
        }
        with_vm(|v| unsafe { v.destroy_thread(memory) }).expect("thread launch rollback");
        if let Some(lease) = lease {
            with_meta(|m| {
                m.executive.release_lease(lease);
                m.executive.close(r.pid, handle).unwrap();
            });
            cleanup_waits();
        } else if let Some(event) = event {
            run::with(|s| s.close_object(event)).unwrap();
        }
    }
    outcome
}

pub fn wait_process(r: Record, a: [u64; 6]) -> Result<u64, Status> {
    wait_record(r, a, false)
}
pub(super) fn wait_record(r: Record, a: [u64; 6], thread: bool) -> Result<u64, Status> {
    with_vm(|v| validate(v, r, a[1], 8))?;
    let retained = with_meta(|m| {
        if thread {
            m.executive.acquire_thread(r.pid, a[0])
        } else {
            m.executive.acquire_process_wait(r.pid, a[0])
        }
    })?;
    let own = if thread {
        with_meta(|m| m.threads[r.task.slot()].as_ref().unwrap().key)
    } else {
        r.pid.raw()
    };
    let outcome = (|| {
        if retained.key == own {
            return Err(Status::InvalidArgument);
        }
        loop {
            let result = with_meta(|m| {
                if thread {
                    m.executive.thread_result(&retained.lease)
                } else {
                    m.executive.process_result(&retained.lease)
                }
            })?;
            if let Some(code) = result {
                with_vm(|v| write(v, r, a[1], &code.to_le_bytes()))?;
                return Ok(0);
            }
            wake(run::wait_checked(&[retained.wait], Some(a[2])).map_err(task_error)?)?;
        }
    })();
    with_meta(|m| m.executive.release_lease(retained.lease));
    outcome
}

/// THREAD_OPEN_SELF: a new handle to the calling thread (DuplicateHandle
/// of GetCurrentThread, P18): suspend, context, join and wait take it as
/// they take any thread handle. The first thread has no thread object
/// until it opens itself; it gets one then, with an exit event of its own
/// (its record's is the process's).
pub(super) fn thread_open_self(r: Record) -> Result<u64, Status> {
    let (key, opened) = with_meta(|m| {
        let thread = m.threads[r.task.slot()].as_ref().ok_or(Status::Canceled)?;
        Ok::<_, Status>((thread.key, thread.lease.is_some()))
    })?;
    if opened {
        return with_meta(|m| m.executive.open_thread(r.pid, key));
    }
    let event = run::with(|s| s.create_event(true, false)).map_err(task_error)?;
    let created = with_meta(|m| {
        let (handle, lease) = m.executive.create_thread(r.pid, key, event)?;
        let thread = m.threads[r.task.slot()].as_mut().ok_or(Status::Canceled)?;
        if thread.lease.is_some() {
            // Raced by nothing (only this thread opens itself): unreachable,
            // but the new object goes rather than leak.
            m.executive.release_lease(lease);
            let _ = m.executive.close(r.pid, handle);
            return Err(Status::AlreadyExists);
        }
        thread.lease = Some(lease);
        thread.own_exit = Some(event);
        Ok(handle)
    });
    if created.is_err() {
        run::with(|s| s.close_object(event)).expect("unused thread exit event");
    }
    created
}
/// The task of a thread handle of the caller's own process (not itself).
fn own_thread(r: Record, handle: u64) -> Result<door_kernel::task::TaskId, Status> {
    let retained = with_meta(|m| m.executive.acquire_thread(r.pid, handle))?;
    let key = retained.key;
    with_meta(|m| m.executive.release_lease(retained.lease));
    with_meta(|m| {
        m.threads
            .iter()
            .enumerate()
            .find(|(slot, t)| {
                t.as_ref().is_some_and(|t| t.key == key && !t.reaping)
                    && m.records[*slot]
                        .as_ref()
                        .is_some_and(|rec| rec.pid == r.pid)
            })
            .and_then(|(slot, _)| m.records[slot].as_ref().map(|rec| rec.task))
            .ok_or(Status::Canceled)
    })
}
/// SuspendThread: count up; the first suspend stops the task.
pub(super) fn thread_suspend(r: Record, handle: u64) -> Result<u64, Status> {
    let task = own_thread(r, handle)?;
    if task == r.task {
        // Suspending oneself: Windows allows it, but nothing could resume a
        // single-threaded caller; refuse rather than hang the process.
        return Err(Status::InvalidArgument);
    }
    let previous = with_meta(|m| {
        let thread = m.threads[task.slot()].as_mut().ok_or(Status::Canceled)?;
        if thread.suspends == 127 {
            return Err(Status::ResourceExhausted);
        }
        thread.suspends += 1;
        Ok::<_, Status>(thread.suspends - 1)
    })?;
    if previous == 0 && run::with(|s| s.suspend(task)).is_err() {
        with_meta(|m| {
            if let Some(t) = m.threads[task.slot()].as_mut() {
                t.suspends = 0;
            }
        });
        return Err(Status::Canceled);
    }
    Ok(u64::from(previous))
}
/// ResumeThread: count down; the last resume makes the task runnable.
pub(super) fn thread_resume(r: Record, handle: u64) -> Result<u64, Status> {
    let task = own_thread(r, handle)?;
    let previous = with_meta(|m| {
        let thread = m.threads[task.slot()].as_mut().ok_or(Status::Canceled)?;
        let previous = thread.suspends;
        thread.suspends = previous.saturating_sub(1);
        Ok::<_, Status>(previous)
    })?;
    if previous == 1 {
        let _ = run::with(|s| s.resume(task));
    }
    Ok(u64::from(previous))
}
/// PROCESS_RESUME: run the first thread of a process launched suspended.
pub(super) fn process_resume(r: Record, handle: u64) -> Result<u64, Status> {
    let held = with_meta(|m| {
        let (target, _) = m.executive.wait_process(r.pid, handle)?;
        let pid = m.executive.process_by_raw(target)?;
        Ok::<_, Status>(m.processes[pid.slot()].as_mut().and_then(|p| p.held.take()))
    })?;
    match held {
        Some(task) => {
            run::with(|s| s.resume(task)).map_err(task_error)?;
            Ok(1)
        }
        None => Ok(0),
    }
}

/// GetThreadContext / SetThreadContext (P18 M0c) of a suspended thread of
/// the caller's own 64-bit Windows process: its saved user registers and
/// FXSAVE state (the trap frame its kernel stack holds while it is
/// stopped). A thread that has not stopped yet answers WouldBlock (the
/// caller retries, as Windows' GetThreadContext waits for the suspension).
pub(super) fn thread_context(
    r: Record,
    handle: u64,
    buffer: u64,
    set: bool,
) -> Result<u64, Status> {
    use door_windows_abi::exception as win;
    if !matches!(r.role, 107 | 110 | 115) {
        return Err(Status::AccessDenied);
    }
    let task = own_thread(r, handle)?;
    if task == r.task {
        // The current thread reads its own context in user mode.
        return Err(Status::InvalidArgument);
    }
    let snapshot = run::with(|s| s.snapshot(task)).map_err(task_error)?;
    if snapshot.state != door_kernel::task::State::Suspended
        || snapshot.current_cpu.is_some()
        || snapshot.retiring_cpu.is_some()
    {
        return Err(Status::WouldBlock);
    }
    let saved = snapshot.context as *mut arch::Context;
    // SAFETY: a suspended task's saved context and trap frame stay on its
    // kernel stack until it runs again, which needs this process's resume.
    let frame = unsafe { &mut *((*saved).frame as *mut arch::TrapFrame) };
    if frame.cs == u64::from(arch::USER_CODE32_SELECTOR) {
        return Err(Status::Unsupported);
    }
    let mut bytes = [0u8; win::CONTEXT_BYTES];
    if !set {
        let integer = win::IntegerContext {
            registers: [
                frame.rax, frame.rcx, frame.rdx, frame.rbx, frame.rsp, frame.rbp, frame.rsi,
                frame.rdi, frame.r8, frame.r9, frame.r10, frame.r11, frame.r12, frame.r13,
                frame.r14, frame.r15, frame.rip,
            ],
            flags: frame.rflags as u32,
        };
        if !integer.encode(&mut bytes) || !win::encode_float(unsafe { &(*saved).fx }, &mut bytes) {
            return Err(Status::InvalidArgument);
        }
        return with_vm(|v| write(v, r, buffer, &bytes)).map(|()| 0);
    }
    with_vm(|v| read(v, r, buffer, &mut bytes))?;
    let integer = win::IntegerContext::decode(&bytes).ok_or(Status::InvalidArgument)?;
    let regs = integer.registers;
    let mut next = *frame;
    next.rax = regs[0];
    next.rcx = regs[1];
    next.rdx = regs[2];
    next.rbx = regs[3];
    next.rsp = regs[4];
    next.rbp = regs[5];
    next.rsi = regs[6];
    next.rdi = regs[7];
    next.r8 = regs[8];
    next.r9 = regs[9];
    next.r10 = regs[10];
    next.r11 = regs[11];
    next.r12 = regs[12];
    next.r13 = regs[13];
    next.r14 = regs[14];
    next.r15 = regs[15];
    next.rip = regs[16];
    next.rflags = u64::from(integer.flags);
    if !arch::valid_user_return(unsafe { &*saved }, &next, arch::kernel_cr3()) {
        return Err(Status::InvalidArgument);
    }
    if let Some(fx) = win::decode_float(&bytes) {
        unsafe { (*saved).fx = fx };
    }
    *frame = next;
    Ok(0)
}

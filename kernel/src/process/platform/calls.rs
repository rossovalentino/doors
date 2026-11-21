//! Validated platform trap operations. Small buffers are captured before dispatch;
//! blocking calls retain authority without holding VM or metadata guards.
use super::*;

#[inline(never)]
/// Checked user copy. Both complete ranges are validated under the VM guard
/// before the first byte moves, so a partly invalid destination receives
/// nothing. WINDOWS_GUI (role 115) compat DLLs may move up to 64 KiB per call
/// (BitBlt/DIB transfers) in 4 KiB captured chunks under the same guard,
/// ordered so overlapping ranges keep memmove semantics.
fn captured_copy(r: Record, a: [u64; 6]) -> Result<u64, Status> {
    let bytes = usize::try_from(a[2]).map_err(|_| Status::InvalidArgument)?;
    let limit = match r.role {
        107 | 110 | 115 => door_windows_abi::BULK_COPY_BYTES,
        _ => 1024,
    };
    if bytes > limit {
        return Err(Status::InvalidArgument);
    }
    let (destination, source) = (a[0], a[1]);
    let mut captured = [0; 4096];
    with_vm(|v| {
        v.validate(r.pid.slot(), source, bytes, UserAccess::Read)?;
        v.validate_write(r.pid.slot(), destination, bytes)?;
        let chunks = bytes.div_ceil(captured.len());
        for step in 0..chunks {
            // Copy backwards when the destination starts inside the source.
            let index = if destination > source && destination < source + bytes as u64 {
                chunks - 1 - step
            } else {
                step
            };
            let offset = index * captured.len();
            let length = (bytes - offset).min(captured.len());
            read(v, r, source + offset as u64, &mut captured[..length])?;
            write(v, r, destination + offset as u64, &captured[..length])?;
        }
        Ok(())
    })?;
    Ok(bytes as u64)
}

fn wait_create(r: Record, a: [u64; 6]) -> Result<u64, Status> {
    let (kind, event) = run::with(|s| match a[0] {
        1 if a[1] <= 1 && a[2] <= 1 => s
            .create_event(a[2] != 0, a[1] != 0)
            .map(|e| (HandleKind::Event, e)),
        2 if a[1] == 0 && a[2] == 0 => s.create_mutex().map(|e| (HandleKind::Mutex, e)),
        3 if a[1] == 0 && a[2] == 0 => s.create_condition().map(|e| (HandleKind::Condition, e)),
        // Semaphore: a[1] the initial count, a[2] the maximum (Windows LONG).
        4 if a[2] >= 1 && a[2] <= i32::MAX as u64 && a[1] <= a[2] => s
            .create_semaphore(a[1] as u32, a[2] as u32)
            .map(|e| (HandleKind::Semaphore, e)),
        _ => Err(door_kernel::task::Error::InvalidCount),
    })
    .map_err(task_error)?;
    let grant = with_meta(|m| m.executive.create_wait(r.pid, kind, event));
    if grant.is_err() {
        run::with(|s| s.close_object(event)).unwrap();
    }
    grant
}
/// An address-wait key: the process slot and the user address (a waiter of
/// a process that is gone has left with it).
fn address_key(r: Record, address: u64) -> u64 {
    ((r.pid.slot() as u64) << 48) | (address & 0x0000_ffff_ffff_ffff)
}
/// WaitOnAddress: the bucket sequence first, then the user word (outside
/// the scheduling lock), then block unless a wake came in between.
fn wait_address(r: Record, a: [u64; 6]) -> Result<u64, Status> {
    let (address, expected, size) = (a[0], a[1], a[2]);
    if !matches!(size, 1 | 2 | 4 | 8) || !address.is_multiple_of(size) {
        return Err(Status::InvalidArgument);
    }
    let key = address_key(r, address);
    let sequence = run::with(|s| s.address_sequence(key));
    let mut word = [0u8; 8];
    with_vm(|v| read(v, r, address, &mut word[..size as usize]))?;
    let mask = if size == 8 {
        u64::MAX
    } else {
        (1u64 << (size * 8)) - 1
    };
    if u64::from_le_bytes(word) != expected & mask {
        return Ok(0);
    }
    wake(run::wait_address_checked(key, sequence, Some(a[3])).map_err(task_error)?)
}
fn wait_op(r: Record, operation: u32, a: [u64; 6]) -> Result<u64, Status> {
    let needed = if operation == op::WAIT || operation == op::CONDITION_WAIT {
        rights::WAIT
    } else {
        rights::WRITE
    };
    let retained = with_meta(|m| m.executive.acquire_wait(r.pid, a[0], needed))?;
    let outcome = (|| match operation {
        op::WAIT if retained.kind != HandleKind::Condition => {
            wake(run::wait_checked(&[retained.wait], Some(a[1])).map_err(task_error)?)
        }
        op::RESET if retained.kind == HandleKind::Event => {
            run::with(|s| s.reset_event(retained.wait)).map_err(task_error)?;
            Ok(0)
        }
        op::SIGNAL => {
            if retained.kind == HandleKind::Semaphore {
                // ReleaseSemaphore: a[1] the amount; the previous count back.
                let amount = u32::try_from(a[1]).map_err(|_| Status::InvalidArgument)?;
                return run::with(|s| {
                    let previous = s.semaphore_count(retained.wait)?;
                    s.release_semaphore(retained.wait, amount)?;
                    Ok(u64::from(previous))
                })
                .map_err(task_error);
            }
            run::with(|s| match retained.kind {
                HandleKind::Event if a[1] == 1 => s.signal_event(retained.wait),
                HandleKind::Mutex if a[1] == 1 => {
                    s.unlock_mutex(arch::current_cpu() as u8, retained.wait)
                }
                HandleKind::Condition if a[1] <= 1 => s.signal_condition(retained.wait, a[1] == 0),
                _ => Err(door_kernel::task::Error::InvalidCount),
            })
            .map_err(task_error)?;
            Ok(0)
        }
        op::CONDITION_WAIT if retained.kind == HandleKind::Condition => {
            let mutex = with_meta(|m| {
                m.executive
                    .acquire_wait(r.pid, a[1], rights::WAIT | rights::WRITE)
            })?;
            let result = if mutex.kind == HandleKind::Mutex {
                run::condition_wait_checked(retained.wait, mutex.wait, Some(a[2]))
                    .map_err(task_error)
                    .and_then(wake)
            } else {
                Err(Status::InvalidArgument)
            };
            with_meta(|m| m.executive.release_lease(mutex.lease));
            result
        }
        _ => Err(Status::InvalidArgument),
    })();
    with_meta(|m| m.executive.release_lease(retained.lease));
    outcome
}

pub fn dispatch(r: Record, operation: u32, a: [u64; 6]) -> Result<u64, Status> {
    if r.role < 100 {
        return Err(Status::InvalidOperation);
    }
    let used = match operation {
        op::IDENTITY | op::SESSION_QUERY | op::SYSTEM_SNAPSHOT => 2,
        op::VM_ALLOCATE | op::VM_PROTECT | op::USER_COPY => 3,
        op::VM_RELEASE => 2,
        op::PROCESS_LAUNCH | op::BOOT_FILE_READ => 6,
        op::PROCESS_EXECUTE => 5,
        op::CONSOLE_CONTROL => 3,
        op::SECTION_SNAPSHOT | op::SECTION_FILL | op::DISPLAY_WRITE => 4,
        op::DISPLAY_INFO | op::DISPLAY_INPUT | op::DISPLAY_CLOCK => 2,
        op::DISPLAY_RELEASE => 1,
        op::PROCESS_TERMINATE => 2,
        op::THREAD_CREATE => 3,
        op::PROCESS_RESUME | op::THREAD_SUSPEND | op::THREAD_RESUME => 1,
        op::THREAD_GET_CONTEXT | op::THREAD_SET_CONTEXT => 2,
        op::THREAD_OPEN_SELF => 0,
        op::PROCESS_OPEN_SELF => 0,
        op::PROCESS_MACHINE => 1,
        op::RESOURCE_OPEN => 1,
        op::THREAD_JOIN => 3,
        op::SLEEP => 1,
        op::PROCESS_QUIESCE => 1,
        op::WINDOWS_MODULE_LOAD | op::WINDOWS_MODULE_UNLOAD => 0,
        op::WINDOWS_MODULE_SYMBOL => 4,
        op::WINDOWS_TLS_CALLBACK => 2,
        op::WINDOWS_PROVIDER_LOAD => 3,
        op::WINDOWS_PRIVATE_LOAD => 4,
        op::VM_RESERVE => 4,
        op::VM_DECOMMIT | op::VM_QUERY => 2,
        op::VM_FLUSH_WRITE_BUFFERS => 0,
        op::VM_SECTION_CREATE => 4,
        op::VM_SECTION_MAP => 6,
        op::VM_SECTION_UNMAP => 2,
        op::VM_SECTION_SIZE => 1,
        op::VM_COMMIT | op::VM_WINDOWS_PROTECT => 3,
        op::VM_WINDOWS_RELEASE => 3,
        op::ENTROPY_STATUS | op::PORT_CREATE => 0,
        op::ENTROPY_READ => 2,
        op::PORT_CONNECT | op::PORT_ACCEPT => 1,
        op::WAIT_CREATE => 3,
        op::WAIT | op::SIGNAL => 2,
        op::RESET => 1,
        op::CONDITION_WAIT => 3,
        op::WAIT_ADDRESS => 4,
        op::WAKE_ADDRESS => 2,
        op::SESSION_CREATE => 3,
        op::SESSION_REVOKE | op::PERMIT_RENEW | op::BLOCK_INFO => 2,
        op::BLOCK_READ | op::BLOCK_WRITE => 4,
        op::BLOCK_FLUSH | op::CONSOLE_READ => 1,
        op::CONSOLE_WRITE => 3,
        op::NET_INFO => 2,
        op::NET_SEND | op::NET_RECEIVE => 3,
        op::AUDIO_INFO | op::AUDIO_CONTROL => 2,
        op::AUDIO_WRITE | op::AUDIO_READ => 3,
        _ => return Err(Status::InvalidOperation),
    };
    if a[used..].iter().any(|v| *v != 0) {
        return Err(Status::InvalidArgument);
    }
    match operation {
        op::SYSTEM_SNAPSHOT => super::monitor::snapshot(r, a),
        op::SECTION_SNAPSHOT | op::SECTION_FILL
            if with_meta(|m| {
                m.executive
                    .windows_section(r.pid, a[0], rights::READ)
                    .is_ok()
            }) =>
        {
            // A Windows section (P18): page by page through the pool.
            let length = usize::try_from(a[3]).map_err(|_| Status::InvalidArgument)?;
            if length == 0 || length > door_abi::graphics::SECTION_COPY_BYTES {
                return Err(Status::InvalidArgument);
            }
            let fill = operation == op::SECTION_FILL;
            let needed = if fill {
                rights::READ | rights::WRITE
            } else {
                rights::READ
            };
            let (key, _) = with_meta(|m| m.executive.windows_section(r.pid, a[0], needed))?;
            with_vm(|v| {
                if fill {
                    v.validate(r.pid.slot(), a[2], length, UserAccess::Read)?;
                } else {
                    v.validate_private_output(r.pid.slot(), a[2], length)?;
                }
                v.win_section_copy(r.pid.slot(), key, a[1], a[2], length, fill)
            })?;
            Ok(length as u64)
        }
        op::SECTION_FILL => {
            let length = usize::try_from(a[3]).map_err(|_| Status::InvalidArgument)?;
            if length == 0 || length > door_abi::graphics::SECTION_COPY_BYTES {
                return Err(Status::InvalidArgument);
            }
            let section = with_meta(|m| {
                m.executive
                    .acquire_section(r.pid, a[0], rights::READ | rights::WRITE)
            })?;
            let result = (|| {
                if a[1]
                    .checked_add(length as u64)
                    .is_none_or(|end| end > section.bytes)
                {
                    return Err(Status::BadAddress);
                }
                with_vm(|v| {
                    v.validate(r.pid.slot(), a[2], length, UserAccess::Read)?;
                    let destination = v.section_capture_address(section.key, a[1], length)?;
                    // SAFETY: validated user source; the WRITE lease and VM
                    // guard keep the section's alias for the copy.
                    let copied = unsafe {
                        arch::copy_user_to_shared(v.cr3(r.pid.slot())?, a[2], destination, length)
                    }
                    .map_err(|_| Status::BadAddress)?;
                    if copied != length {
                        return Err(Status::BadAddress);
                    }
                    Ok(())
                })?;
                Ok(length as u64)
            })();
            with_meta(|m| m.executive.release_lease(section.lease));
            result
        }
        op::SECTION_SNAPSHOT => {
            let length = usize::try_from(a[3]).map_err(|_| Status::InvalidArgument)?;
            if length == 0 || length > door_abi::graphics::SECTION_COPY_BYTES {
                return Err(Status::InvalidArgument);
            }
            let section = with_meta(|m| m.executive.acquire_section(r.pid, a[0], rights::READ))?;
            let result = (|| {
                if a[1]
                    .checked_add(length as u64)
                    .is_none_or(|end| end > section.bytes)
                {
                    return Err(Status::BadAddress);
                }
                with_vm(|v| {
                    v.validate_private_output(r.pid.slot(), a[2], length)?;
                    let source = v.section_capture_address(section.key, a[1], length)?;
                    // SAFETY: complete private output validation and READ backing
                    // lease/VM guard span the raw bounded assembly copy.
                    let copied = unsafe {
                        arch::copy_shared_to_user(v.cr3(r.pid.slot())?, a[2], source, length)
                    }
                    .map_err(|_| Status::BadAddress)?;
                    if copied != length {
                        return Err(Status::BadAddress);
                    }
                    Ok(())
                })?;
                Ok(length as u64)
            })();
            with_meta(|m| m.executive.release_lease(section.lease));
            result
        }
        op::PROCESS_EXECUTE => execute::execute(r, a),
        op::BOOT_FILE_READ => execute::boot_read(r, a),
        op::RESOURCE_OPEN => super::resource_open(r, a[0]),
        // LoadLibrary of a system DLL outside the startup closure: map it
        // and what it imports (catalog index a[0]); write their
        // `bootstrap::ModuleInit` records, in initialization order, to
        // a[1] (a[2] bytes); return how many.
        op::WINDOWS_PROVIDER_LOAD => {
            if !matches!(r.role, 110 | 115) {
                return Err(Status::AccessDenied);
            }
            let root = u8::try_from(a[0]).map_err(|_| Status::InvalidArgument)?;
            let provider = door_windows_abi::modules::PROVIDERS
                .get(usize::from(root))
                .filter(|p| p.system)
                .ok_or(Status::NotFound)?;
            let old = process_windows_image(r)?;
            let first = old.plan.module_count();
            if (0..first).any(|i| {
                old.plan
                    .module(i)
                    .is_some_and(|m| m.source.base.0 == provider.base)
            }) {
                super::super::log(format_args!(
                    "DOOR_P16 PROVIDER_LOAD refused root={root} reason=planned modules={first}\n"
                ));
                return Err(Status::AlreadyExists);
            }
            let planned = planned(r)?;
            let written = extend(r, &old, &planned, root, &planned.privates, a[1], a[2])?;
            with_meta(|m| {
                if let Some(p) = m.processes[r.pid.slot()].as_mut() {
                    p.windows_roots[planned.count] = root;
                    p.windows_root_count = planned.count + 1;
                }
            });
            Ok(written)
        }
        op::WINDOWS_PRIVATE_LOAD => {
            if !matches!(r.role, 110 | 115) {
                return Err(Status::AccessDenied);
            }
            let length = usize::try_from(a[1]).map_err(|_| Status::InvalidArgument)?;
            if !(64..=abi::EXECUTABLE_MAX_BYTES).contains(&length) {
                return Err(Status::InvalidArgument);
            }
            let old = process_windows_image(r)?;
            let planned = planned(r)?;
            let index = with_meta(|m| {
                m.processes[r.pid.slot()]
                    .as_ref()
                    .and_then(|p| p.windows_private.iter().position(Option::is_none))
            })
            .ok_or(Status::ResourceExhausted)?;
            // Captured before anything is planned: no user pointer survives.
            let capture = execute::capture_user(r, a[0], length)?;
            // A DLL (IMAGE_FILE_DLL), never an executable, for the
            // process's own machine (P17: PE32 in a 32-bit process).
            let image = door_image_format::PeImage::parse_any_with_delay_imports(capture.bytes())
                .map_err(|_| Status::InvalidArgument)?;
            let executable =
                door_image_format::PeImage::parse_any_with_delay_imports(planned.executable)
                    .map_err(|_| Status::InvalidArgument)?;
            if image.characteristics & 0x2000 == 0 || image.machine != executable.machine {
                return Err(Status::InvalidArgument);
            }
            let mut privates = planned.privates;
            privates[index] = capture.bytes();
            let root = super::super::windows::PRIVATE_ROOT | index as u8;
            let written =
                extend(r, &old, &planned, root, &privates, a[2], a[3]).inspect_err(|e| {
                    super::super::log(format_args!(
                        "DOOR_P16 PRIVATE_LOAD failed index={index} status={e:?} bytes={length}\n"
                    ))
                })?;
            with_meta(|m| {
                if let Some(p) = m.processes[r.pid.slot()].as_mut() {
                    p.windows_roots[planned.count] = root;
                    p.windows_root_count = planned.count + 1;
                    p.windows_private[index] = Some(capture);
                }
            });
            super::super::log(format_args!(
                "DOOR_P16 PRIVATE_LOAD index={index} base={:#x} bytes={length} modules={}\n",
                door_windows_abi::modules::private_base(index),
                old.plan.module_count() + written as usize
            ));
            Ok(written)
        }
        op::WINDOWS_TLS_CALLBACK => {
            if !matches!(r.role, 107 | 110 | 115) {
                return Err(Status::AccessDenied);
            }
            let image = process_windows_image(r)?;
            let index = usize::try_from(a[0]).map_err(|_| Status::InvalidArgument)?;
            let module = image.plan.module(index).ok_or(Status::NotFound)?;
            if !with_vm(|v| v.windows_module_present(r.pid.slot(), &image.plan, index)) {
                return Err(Status::NotFound);
            }
            let Some(tls) = module.tls().map_err(|_| Status::InvalidArgument)? else {
                return Ok(0);
            };
            if a[1] >= tls.callback_count as u64 {
                return Ok(0);
            }
            Ok(module.source.base.0 + u64::from(tls.callbacks[a[1] as usize].0))
        }
        op::WINDOWS_MODULE_LOAD | op::WINDOWS_MODULE_UNLOAD | op::WINDOWS_MODULE_SYMBOL => {
            if !matches!(r.role, 107 | 110 | 115) {
                return Err(Status::AccessDenied);
            }
            let image = process_windows_image(r)?;
            if matches!(r.role, 110 | 115) && operation != op::WINDOWS_MODULE_SYMBOL {
                return Err(Status::Unsupported);
            }
            if operation == op::WINDOWS_MODULE_LOAD {
                return with_vm(|v| unsafe { v.windows_module_load(r.pid.slot(), &image.plan) });
            }
            if operation == op::WINDOWS_MODULE_UNLOAD {
                return with_vm(|v| unsafe { v.windows_module_unload(r.pid.slot(), &image.plan) })
                    .map(|()| 0);
            }
            let index = (0..image.plan.module_count())
                .find(|&index| {
                    image
                        .plan
                        .module(index)
                        .is_some_and(|module| module.source.base.0 == a[0])
                })
                .ok_or(Status::NotFound)?;
            if !with_vm(|v| v.windows_module_present(r.pid.slot(), &image.plan, index)) {
                return Err(Status::NotFound);
            }
            let mut name = [0u8; 128];
            let symbol = if a[3] != 0 {
                if a[1] != 0 || a[2] != 0 || a[3] > 65535 {
                    return Err(Status::InvalidArgument);
                }
                door_pe_loader::Symbol::Ordinal(a[3] as u16)
            } else {
                let bytes = usize::try_from(a[2]).map_err(|_| Status::InvalidArgument)?;
                if bytes == 0 || bytes > name.len() {
                    return Err(Status::InvalidArgument);
                }
                with_vm(|v| read(v, r, a[1], &mut name[..bytes]))?;
                door_pe_loader::Symbol::Name(
                    core::str::from_utf8(&name[..bytes]).map_err(|_| Status::InvalidArgument)?,
                )
            };
            image
                .plan
                .resolve_export(index, symbol)
                .map(|address| address.0)
                .map_err(|_| Status::NotFound)
        }
        op::VM_RESERVE
        | op::VM_COMMIT
        | op::VM_DECOMMIT
        | op::VM_WINDOWS_PROTECT
        | op::VM_WINDOWS_RELEASE => {
            if !matches!(r.role, 107 | 110 | 115) {
                return Err(Status::AccessDenied);
            }
            with_vm(|v| match operation {
                op::VM_RESERVE => v.windows_reserve(r.pid.slot(), a[0], a[1], a[2], a[3]),
                op::VM_COMMIT => unsafe { v.windows_commit(r.pid.slot(), a[0], a[1], a[2]) },
                op::VM_DECOMMIT => {
                    unsafe { v.windows_decommit(r.pid.slot(), a[0], a[1]) }.map(|_| 0)
                }
                op::VM_WINDOWS_PROTECT => v.windows_protect(r.pid.slot(), a[0], a[1], a[2]),
                _ => unsafe { v.windows_release(r.pid.slot(), a[0], a[1], a[2]) }.map(|_| 0),
            })
        }
        op::VM_SECTION_CREATE | op::VM_SECTION_MAP | op::VM_SECTION_UNMAP | op::VM_SECTION_SIZE => {
            if !matches!(r.role, 107 | 110 | 115) {
                return Err(Status::AccessDenied);
            }
            windows_section(r, operation, a)
        }
        op::VM_QUERY => {
            if !matches!(r.role, 107 | 110 | 115) {
                return Err(Status::AccessDenied);
            }
            with_vm(|v| {
                let record = v.windows_query(r.pid.slot(), a[0])?.encode();
                write(v, r, a[1], &record).map(|_| 0)
            })
        }
        op::VM_FLUSH_WRITE_BUFFERS => {
            if !matches!(r.role, 107 | 110 | 115) {
                return Err(Status::AccessDenied);
            }
            // A root-wide retirement makes every CPU that may run this
            // process take an IPI and serialize before acknowledging, which
            // is what FlushProcessWriteBuffers promises.
            // The VM guard serializes shootdown owners.
            with_vm(|_| arch::shootdown(0)).map_err(|_| Status::IoFailure)?;
            Ok(0)
        }
        op::PROCESS_QUIESCE => super::quiesce(r, a[0]),
        op::USER_COPY => captured_copy(r, a),
        op::IDENTITY => {
            let (pid, authority) = with_meta(|m| {
                if a[0] == 0 {
                    m.executive.authority(r.pid).map(|v| (r.pid.raw(), v))
                } else {
                    m.executive.channel_identity(r.pid, a[0])
                }
            })?;
            let mut bytes = [0; abi::IDENTITY_BYTES];
            identity(pid, authority).encode(&mut bytes)?;
            with_vm(|v| write(v, r, a[1], &bytes))?;
            Ok(0)
        }
        op::VM_ALLOCATE => {
            let policy = abi::manifest_policy(r.role - 100).ok_or(Status::AccessDenied)?;
            with_vm(|v| unsafe {
                v.allocate(r.pid.slot(), a[0], a[1], a[2], policy.max_heap_bytes)
            })
        }
        op::VM_RELEASE => {
            with_vm(|v| unsafe { v.release(r.pid.slot(), a[0], a[1]) })?;
            Ok(0)
        }
        op::VM_PROTECT => {
            with_vm(|v| unsafe { v.protect(r.pid.slot(), a[0], a[1], a[2]) })?;
            Ok(0)
        }
        op::PROCESS_LAUNCH => {
            with_vm(|v| validate(v, r, a[5], abi::LAUNCH_RESULT_BYTES))?;
            let mut authority = with_meta(|m| {
                m.executive
                    .authority_for_launch(r.pid, a[0], a[1], a[2], next(&INSTANCE)?)
            })?;
            // The elevated Terminal: only the Launch authority's holder
            // (Init, after User Account Control) starts it, in a session.
            if a[4] == abi::mode::TERMINAL_ELEVATED {
                if a[1] != abi::manifest::WORKSPACE || authority.session == 0 {
                    return Err(Status::AccessDenied);
                }
                authority.groups |= abi::group::ADMINISTRATORS;
            }
            let prepared = prepare(Some(r), authority, a[3], a[4], None).inspect_err(|e| {
                super::super::log(format_args!(
                    "DOOR_P08 LAUNCH refused manifest={} mode={} status={e:?}\n",
                    a[1], a[4]
                ))
            })?;
            let mut bytes = [0; abi::LAUNCH_RESULT_BYTES];
            LaunchResult {
                process: prepared.process_handle,
                control: prepared.control,
                pid: prepared.pid.raw(),
            }
            .encode(&mut bytes)?;
            if let Err(error) = with_vm(|v| write(v, r, a[5], &bytes)) {
                with_meta(|m| {
                    m.processes[prepared.pid.slot()].as_mut().unwrap().starting = false;
                    m.executive.close(r.pid, prepared.process_handle).unwrap();
                    m.executive.close(r.pid, prepared.control).unwrap();
                });
                run::with(|s| s.terminate(prepared.task, -1)).unwrap();
                reap();
                return Err(error);
            }
            publish(&prepared)?;
            Ok(0)
        }
        op::PROCESS_TERMINATE => {
            let pid = with_meta(|m| m.executive.process_target(r.pid, a[0]))?;
            stop_process(pid, a[1] as i64);
            Ok(0)
        }
        op::PROCESS_MACHINE => {
            // From the process object, so a handle to a process that has
            // already exited still answers (as IsWow64Process does).
            let x86 = with_meta(|m| m.executive.process_x86(r.pid, a[0]))?;
            Ok(if x86 { 0x14c } else { 0x8664 })
        }
        op::THREAD_CREATE => thread_create(r, a[0], a[1], a[2]),
        op::THREAD_SUSPEND => super::threads::thread_suspend(r, a[0]),
        op::THREAD_GET_CONTEXT => super::threads::thread_context(r, a[0], a[1], false),
        op::THREAD_SET_CONTEXT => super::threads::thread_context(r, a[0], a[1], true),
        op::THREAD_OPEN_SELF => super::threads::thread_open_self(r),
        op::PROCESS_OPEN_SELF => with_meta(|m| {
            m.executive.process_handle(
                r.pid,
                r.pid,
                door_abi::rights::QUERY
                    | door_abi::rights::WAIT
                    | door_abi::rights::DUPLICATE
                    | door_abi::rights::TRANSFER,
            )
        }),
        op::THREAD_RESUME => super::threads::thread_resume(r, a[0]),
        op::PROCESS_RESUME => super::threads::process_resume(r, a[0]),
        op::THREAD_JOIN => wait_record(r, a, true),
        op::SLEEP => {
            if a[0] <= run::now() {
                return Ok(0);
            }
            let event = with_meta(|m| m.processes[r.pid.slot()].as_ref().unwrap().sleep);
            match run::wait_checked(&[event], Some(a[0]))
                .map_err(task_error)?
                .reason
            {
                WakeReason::Timeout => Ok(0),
                _ => Err(Status::Canceled),
            }
        }
        op::ENTROPY_STATUS => Ok(crate::entropy_source::status()),
        op::ENTROPY_READ => {
            let length = usize::try_from(a[1]).map_err(|_| Status::InvalidArgument)?;
            if length == 0 || length > door_abi::entropy::READ_MAX {
                return Err(Status::InvalidArgument);
            }
            // Init, the session service, and the apps of a signed-in session
            // (TLS keys, archive salts, as BCryptGenRandom serves any Windows
            // process), and door-network for WireGuard ephemeral keys and
            // indices (served like an app); sessionless ordinary processes
            // stay denied.
            let authority = with_meta(|m| m.executive.authority(r.pid))?;
            let system = authority.capabilities
                & (door_abi::platform::capability::SESSION | door_abi::platform::capability::INIT)
                != 0;
            let network = authority.capabilities & door_abi::platform::capability::NETWORK != 0;
            if !system && !network && authority.session == 0 {
                return Err(Status::AccessDenied);
            }
            with_vm(|v| validate(v, r, a[0], length))?;
            let mut bytes = [0; door_abi::entropy::READ_MAX];
            // Apps are served from their own DRBG: they cannot force raw
            // jitter runs or exhaust the pool Init and sessions rely on.
            let written = if system {
                crate::entropy_source::read(&mut bytes[..length])?
            } else {
                crate::entropy_source::read_app(&mut bytes[..length])?
            };
            let result = with_vm(|v| write(v, r, a[0], &bytes[..written]));
            bytes.fill(0);
            result.map(|()| written as u64)
        }
        op::PORT_CREATE => with_meta(|m| m.executive.create_port(r.pid)),
        op::PORT_CONNECT => with_meta(|m| m.executive.connect_port(r.pid, a[0])),
        op::PORT_ACCEPT => with_meta(|m| m.executive.accept_port(r.pid, a[0])),
        op::WAIT_CREATE => wait_create(r, a),
        op::WAIT | op::SIGNAL | op::RESET | op::CONDITION_WAIT => wait_op(r, operation, a),
        op::WAIT_ADDRESS => wait_address(r, a),
        op::WAKE_ADDRESS => {
            let count = if a[1] == 0 { usize::MAX } else { a[1] as usize };
            run::with(|s| s.wake_address(address_key(r, a[0]), count))
                .map(|n| n as u64)
                .map_err(task_error)
        }
        op::SESSION_QUERY => {
            let (id, uid) = with_meta(|m| m.executive.session_info(r.pid, a[0]))?;
            let mut bytes = [0; 24];
            bytes[..8].copy_from_slice(&id.to_le_bytes());
            bytes[8..16].copy_from_slice(&id.to_le_bytes());
            bytes[16..20].copy_from_slice(&uid.to_le_bytes());
            with_vm(|v| write(v, r, a[1], &bytes))?;
            Ok(0)
        }
        op::SESSION_CREATE => {
            with_meta(|m| m.executive.create_session_typed(r.pid, a[0], a[1], a[2]))
        }
        op::PERMIT_RENEW => {
            let uid = u32::try_from(a[1]).map_err(|_| Status::InvalidArgument)?;
            with_meta(|m| m.executive.renew_permit(r.pid, a[0], uid))
        }
        op::SESSION_REVOKE => {
            let targets = with_meta(|m| {
                let id = m.executive.revoke_session(r.pid, a[0], a[1])?;
                Ok::<_, Status>(m.executive.session_processes(id))
            })?;
            for pid in targets.into_iter().flatten() {
                stop_process(pid, -13);
            }
            Ok(0)
        }
        _ => device(r, operation, a),
    }
}

fn device(r: Record, operation: u32, a: [u64; 6]) -> Result<u64, Status> {
    let kind = if matches!(
        operation,
        op::DISPLAY_INFO
            | op::DISPLAY_WRITE
            | op::DISPLAY_INPUT
            | op::DISPLAY_RELEASE
            | op::DISPLAY_CLOCK
    ) {
        HandleKind::Display
    } else if matches!(
        operation,
        op::CONSOLE_READ | op::CONSOLE_WRITE | op::CONSOLE_CONTROL
    ) {
        HandleKind::Console
    } else if matches!(
        operation,
        op::AUDIO_INFO | op::AUDIO_WRITE | op::AUDIO_CONTROL | op::AUDIO_READ
    ) {
        HandleKind::Audio
    } else if matches!(operation, op::NET_INFO | op::NET_SEND | op::NET_RECEIVE) {
        HandleKind::Network
    } else {
        HandleKind::Block
    };
    let caller = with_meta(|m| {
        m.executive.require_authority(r.pid, a[0], kind)?;
        m.executive
            .authority(r.pid)
            .map(|authority| identity(r.pid.raw(), authority))
    })?;
    match operation {
        op::DISPLAY_CLOCK => {
            with_vm(|v| validate(v, r, a[1], door_abi::clock::BYTES))?;
            let bytes = crate::platform_device::display_clock(&caller)?.encode()?;
            with_vm(|v| write(v, r, a[1], &bytes))?;
            Ok(0)
        }
        op::DISPLAY_INFO => {
            with_vm(|v| validate(v, r, a[1], door_abi::graphics::INFO_BYTES))?;
            let info = crate::platform_device::display_info(&caller)?;
            let mut bytes = [0; door_abi::graphics::INFO_BYTES];
            info.encode(&mut bytes)?;
            with_vm(|v| write(v, r, a[1], &bytes))?;
            Ok(0)
        }
        op::DISPLAY_WRITE => {
            let len = usize::try_from(a[3]).map_err(|_| Status::InvalidArgument)?;
            if len == 0 || len > door_abi::graphics::MAX_COPY_BYTES || !len.is_multiple_of(4) {
                return Err(Status::InvalidArgument);
            }
            let mut pixels = [0; door_abi::graphics::MAX_COPY_BYTES];
            with_vm(|v| read(v, r, a[2], &mut pixels[..len]))?;
            crate::platform_device::display_write(&caller, a[1], &pixels[..len])?;
            Ok(0)
        }
        op::DISPLAY_INPUT => {
            with_vm(|v| validate(v, r, a[1], door_abi::graphics::INPUT_BYTES))?;
            let input = crate::platform_device::display_input(&caller)?;
            let mut bytes = [0; door_abi::graphics::INPUT_BYTES];
            input.encode(&mut bytes)?;
            with_vm(|v| write(v, r, a[1], &bytes))?;
            Ok(0)
        }
        op::DISPLAY_RELEASE => {
            crate::platform_device::display_release(&caller)?;
            Ok(0)
        }
        op::BLOCK_INFO => {
            let info = crate::platform_device::block_info(&caller)?;
            let mut bytes = [0; abi::BLOCK_INFO_BYTES];
            info.encode(&mut bytes)?;
            with_vm(|v| write(v, r, a[1], &bytes))?;
            Ok(0)
        }
        op::BLOCK_READ | op::BLOCK_WRITE => {
            let length = usize::try_from(a[3]).map_err(|_| Status::InvalidArgument)?;
            if length == 0 || length > 4096 || !length.is_multiple_of(512) {
                return Err(Status::InvalidArgument);
            }
            let mut bytes = [0; 4096];
            if operation == op::BLOCK_READ {
                with_vm(|v| validate(v, r, a[2], length))?;
                crate::platform_device::block_read(&caller, a[1], &mut bytes[..length])?;
                with_vm(|v| write(v, r, a[2], &bytes[..length]))?;
            } else {
                with_vm(|v| read(v, r, a[2], &mut bytes[..length]))?;
                crate::platform_device::block_write(&caller, a[1], &bytes[..length])?;
            }
            Ok(0)
        }
        op::BLOCK_FLUSH => {
            crate::platform_device::block_flush(&caller)?;
            Ok(0)
        }
        op::CONSOLE_WRITE => {
            let length = usize::try_from(a[2]).map_err(|_| Status::InvalidArgument)?;
            if length > door_abi::MAX_DIAGNOSTIC {
                return Err(Status::InvalidArgument);
            }
            let mut bytes = [0; door_abi::MAX_DIAGNOSTIC];
            with_vm(|v| read(v, r, a[1], &mut bytes[..length]))?;
            crate::platform_device::console_write(&caller, &bytes[..length]).map(|n| n as u64)
        }
        op::NET_INFO => {
            let info = crate::platform_device::net_info(&caller)?;
            let mut bytes = [0; abi::NET_INFO_BYTES];
            info.encode(&mut bytes)?;
            with_vm(|v| write(v, r, a[1], &bytes))?;
            Ok(0)
        }
        op::NET_SEND => {
            let length = usize::try_from(a[2]).map_err(|_| Status::InvalidArgument)?;
            if !(14..=abi::NET_FRAME_BYTES).contains(&length) {
                return Err(Status::InvalidArgument);
            }
            let mut bytes = [0; abi::NET_FRAME_BYTES];
            with_vm(|v| read(v, r, a[1], &mut bytes[..length]))?;
            crate::platform_device::net_send(&caller, &bytes[..length])?;
            Ok(0)
        }
        op::NET_RECEIVE => {
            let capacity = usize::try_from(a[2]).map_err(|_| Status::InvalidArgument)?;
            if capacity < abi::NET_FRAME_BYTES {
                return Err(Status::InvalidArgument);
            }
            with_vm(|v| validate(v, r, a[1], abi::NET_FRAME_BYTES))?;
            let mut bytes = [0; abi::NET_FRAME_BYTES];
            let n = crate::platform_device::net_receive(&caller, &mut bytes)?;
            if n > 0 {
                with_vm(|v| write(v, r, a[1], &bytes[..n]))?;
            }
            Ok(n as u64)
        }
        op::AUDIO_INFO => {
            let info = crate::platform_device::audio_info(&caller)?;
            let mut bytes = [0; abi::AUDIO_INFO_BYTES];
            info.encode(&mut bytes)?;
            with_vm(|v| write(v, r, a[1], &bytes))?;
            Ok(0)
        }
        op::AUDIO_WRITE => {
            if a[2] != abi::AUDIO_PERIOD_BYTES as u64 {
                return Err(Status::InvalidArgument);
            }
            let mut bytes = [0; abi::AUDIO_PERIOD_BYTES];
            with_vm(|v| read(v, r, a[1], &mut bytes))?;
            crate::platform_device::audio_write(&caller, &bytes)?;
            Ok(0)
        }
        op::AUDIO_CONTROL => {
            crate::platform_device::audio_control(&caller, a[1])?;
            Ok(0)
        }
        op::AUDIO_READ => {
            if a[2] != abi::AUDIO_PERIOD_BYTES as u64 {
                return Err(Status::InvalidArgument);
            }
            let mut bytes = [0; abi::AUDIO_PERIOD_BYTES];
            if !crate::platform_device::audio_read(&caller, &mut bytes)? {
                return Ok(0);
            }
            with_vm(|v| write(v, r, a[1], &bytes))?;
            Ok(1)
        }
        op::CONSOLE_READ => crate::platform_device::console_read(&caller),
        op::CONSOLE_CONTROL => crate::platform_device::console_control(&caller, a[1], a[2]),
        _ => Err(Status::InvalidOperation),
    }
}

/// Plan the application with `root` appended to its load order, map the
/// modules that adds and write their ModuleInit records (initialization
/// order) to `out`; every module already mapped keeps its index and base.
/// The caller records the root once this succeeds.
fn extend(
    r: Record,
    old: &super::super::windows::Image,
    planned: &Planned,
    root: u8,
    privates: &[&'static [u8]],
    out: u64,
    capacity: u64,
) -> Result<u64, Status> {
    let first = old.plan.module_count();
    let mut roots = planned.roots;
    if planned.count == roots.len() {
        return Err(Status::ResourceExhausted);
    }
    roots[planned.count] = root;
    let new = super::super::windows::application_with(
        planned.executable,
        r.role - 100,
        &roots[..=planned.count],
        privates,
    )?;
    for i in 0..first {
        let (Some(a0), Some(b0)) = (old.plan.module(i), new.plan.module(i)) else {
            return Err(Status::InvalidArgument);
        };
        if a0.source.base.0 != b0.source.base.0 {
            return Err(Status::InvalidArgument);
        }
    }
    let added = new.plan.module_count() - first;
    let records = added * door_windows_abi::bootstrap::MODULE_INIT_BYTES;
    if usize::try_from(capacity).map_err(|_| Status::InvalidArgument)? < records {
        return Err(Status::InvalidArgument);
    }
    with_vm(|v| validate(v, r, out, records))?;
    with_vm(|v| unsafe { v.windows_modules_load(r.pid.slot(), &new.plan, first) }).inspect_err(
        |e| {
            super::super::log(format_args!(
                "DOOR_P16 PROVIDER_LOAD failed root={root} status={e:?} first={first} modules={}\n",
                new.plan.module_count()
            ))
        },
    )?;
    let mut bytes =
        [0u8; door_pe_loader::MAX_MODULES * door_windows_abi::bootstrap::MODULE_INIT_BYTES];
    let mut written = 0;
    for &index in new.plan.dependency_order() {
        if index < first {
            continue;
        }
        let module = new.plan.module(index).ok_or(Status::NotFound)?;
        let (resource_rva, resource_bytes) = module
            .resource_directory()
            .map_or((0, 0), |(rva, bytes)| (rva.0, bytes));
        door_windows_abi::bootstrap::ModuleInit {
            base: module.source.base.0,
            entry: module.entry().map_or(0, |entry| entry.0),
            image_bytes: module.image_bytes().next_multiple_of(4096),
            resource_rva,
            resource_bytes,
            flags: 0,
        }
        .encode_record(
            &mut bytes[written * door_windows_abi::bootstrap::MODULE_INIT_BYTES
                ..(written + 1) * door_windows_abi::bootstrap::MODULE_INIT_BYTES],
        );
        written += 1;
    }
    with_vm(|v| write(v, r, out, &bytes[..records]))?;
    Ok(written as u64)
}
/// An application process's plan inputs: its executable, the roots
/// LoadLibrary added (in load order) and its private DLLs' bytes.
struct Planned {
    executable: &'static [u8],
    roots: [u8; door_pe_loader::MAX_MODULES],
    count: usize,
    privates: [&'static [u8]; door_windows_abi::modules::PRIVATE_MODULES],
}
fn planned(r: Record) -> Result<Planned, Status> {
    with_meta(|m| {
        m.processes[r.pid.slot()].as_ref().and_then(|p| {
            let executable = p.capture.as_ref()?.bytes();
            let mut privates = [&[][..]; door_windows_abi::modules::PRIVATE_MODULES];
            for (slot, capture) in privates.iter_mut().zip(&p.windows_private) {
                if let Some(capture) = capture {
                    *slot = capture.bytes();
                }
            }
            Some(Planned {
                executable,
                roots: p.windows_roots,
                count: p.windows_root_count,
                privates,
            })
        })
    })
    .ok_or(Status::Canceled)
}
fn process_windows_image(r: Record) -> Result<super::super::windows::Image, Status> {
    if matches!(r.role, 110 | 115) {
        let p = planned(r)?;
        super::super::windows::application_with(
            p.executable,
            r.role - 100,
            &p.roots[..p.count],
            &p.privates,
        )
    } else {
        super::super::windows::image(abi::manifest::WINDOWS_BASE)
    }
}
/// The Windows section calls (P18): CreateFileMapping's backing, views,
/// unmapping and the size of a section a handle names.
fn windows_section(r: Record, operation: u32, a: [u64; 6]) -> Result<u64, Status> {
    use door_kernel::mm::Protection;
    match operation {
        op::VM_SECTION_CREATE => {
            let key = super::next_section_key()?;
            with_vm(|v| v.win_section_create(key, a[0], a[1], a[2]))?;
            let handle = with_meta(|m| m.executive.create_windows_section(r.pid, key, a[0], a[3]));
            if handle.is_err() {
                with_vm(|v| v.win_section_retire(key)).expect("Windows section create rollback");
            }
            handle
        }
        op::VM_SECTION_MAP => {
            let write = u32::try_from(a[4])
                .ok()
                .and_then(Protection::from_windows)
                .is_some_and(|(p, _)| {
                    matches!(p, Protection::ReadWrite | Protection::ExecuteReadWrite)
                });
            let needed = if write {
                rights::READ | rights::WRITE
            } else {
                rights::READ
            };
            let (key, may_write) = with_meta(|m| {
                let (key, _) = m.executive.windows_section(r.pid, a[0], needed)?;
                let may_write = m
                    .executive
                    .windows_section(r.pid, a[0], rights::READ | rights::WRITE)
                    .is_ok();
                Ok::<_, Status>((key, may_write))
            })?;
            with_vm(|v| {
                v.win_section_map(r.pid.slot(), key, a[1], a[2], a[3], a[4], a[5], may_write)
            })
        }
        op::VM_SECTION_UNMAP => {
            with_vm(|v| v.win_section_unmap(r.pid.slot(), a[0], a[1])).map(|_| 0)
        }
        _ => with_meta(|m| m.executive.windows_section(r.pid, a[0], rights::READ))
            .map(|(_, bytes)| bytes),
    }
}

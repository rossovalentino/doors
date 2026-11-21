//! Bounded process-owned capture arena. A lease survives until VM retirement.
use super::*;
struct Arena(UnsafeCell<[[u8; abi::EXECUTABLE_MAX_BYTES]; 8]>);
unsafe impl Sync for Arena {}
static ARENA: Arena = Arena(UnsafeCell::new([[0; abi::EXECUTABLE_MAX_BYTES]; 8]));
static USED: [AtomicBool; 8] = [const { AtomicBool::new(false) }; 8];
static ARCHIVE: Storage<[Option<door_initfs::Entry<'static>>; door_initfs::MAX_ENTRIES]> =
    Storage(UnsafeCell::new(None));
pub(super) fn initialize(archive: &door_initfs::Archive<'static>) {
    // Initialization precedes the first runnable service; archive bytes are immutable.
    unsafe {
        *ARCHIVE.0.get() = Some(core::array::from_fn(|i| archive.entry(i).ok()));
    }
}
pub(super) struct Capture {
    slot: usize,
    len: usize,
}
impl Capture {
    fn acquire(len: usize) -> Result<Self, Status> {
        let slot = USED
            .iter()
            .position(|used| {
                used.compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
                    .is_ok()
            })
            .ok_or(Status::ResourceExhausted)?;
        Ok(Self { slot, len })
    }
    pub(super) fn bytes(&self) -> &'static [u8] {
        // The lease is retained in Process until every context and VM retires.
        unsafe {
            core::slice::from_raw_parts(
                core::ptr::addr_of!((*ARENA.0.get())[self.slot]).cast::<u8>(),
                self.len,
            )
        }
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        unsafe {
            core::ptr::write_bytes(
                core::ptr::addr_of_mut!((*ARENA.0.get())[self.slot]).cast::<u8>(),
                0,
                self.len,
            );
        }
        USED[self.slot].store(false, Ordering::Release);
    }
}
/// Capture `length` bytes at the caller's `pointer` into an arena lease
/// (a DLL image LoadLibrary passes): the whole range is validated first,
/// and no user pointer survives the copy.
pub(super) fn capture_user(r: Record, pointer: u64, length: usize) -> Result<Capture, Status> {
    let capture = Capture::acquire(length)?;
    with_vm(|v| {
        v.validate(r.pid.slot(), pointer, capture.len, UserAccess::Read)?;
        let output = unsafe {
            core::slice::from_raw_parts_mut(
                core::ptr::addr_of_mut!((*ARENA.0.get())[capture.slot]).cast::<u8>(),
                capture.len,
            )
        };
        door_kernel::process_image::capture_bounded(output, arch::MAX_USER_COPY, |offset, chunk| {
            read(v, r, pointer + offset, chunk)
        })
    })?;
    Ok(capture)
}
pub(super) fn execute(r: Record, a: [u64; 6]) -> Result<u64, Status> {
    // Check authority before touching any user-provided image or descriptor.
    with_meta(|m| {
        m.executive
            .require_authority(r.pid, a[0], HandleKind::Launch)
    })?;
    let mut bytes = [0; abi::EXECUTE_DESCRIPTOR_BYTES];
    with_vm(|v| {
        read(v, r, a[3], &mut bytes)?;
        validate(v, r, a[4], abi::LAUNCH_RESULT_BYTES)
    })?;
    let description = abi::ExecuteDescriptor::decode(&bytes).inspect_err(|e| {
        super::super::log(format_args!(
            "DOOR_P16 EXECUTE refused stage=descriptor status={e:?}\n"
        ))
    })?;
    let capture = Capture::acquire(description.image_bytes as usize)?;
    with_vm(|v| {
        // Pin and validate the entire range before capture; no user pointer survives.
        v.validate(
            r.pid.slot(),
            description.image_pointer,
            capture.len,
            UserAccess::Read,
        )?;
        let output = unsafe {
            core::slice::from_raw_parts_mut(
                core::ptr::addr_of_mut!((*ARENA.0.get())[capture.slot]).cast::<u8>(),
                capture.len,
            )
        };
        // One user copy is bounded by MAX_USER_COPY; images may be larger.
        door_kernel::process_image::capture_bounded(output, arch::MAX_USER_COPY, |offset, chunk| {
            read(v, r, description.image_pointer + offset, chunk)
        })
    })?;
    // The captured, immutable bytes select the personality manifest: a PE
    // GUI-subsystem image runs as WINDOWS_GUI, a console image as the
    // ordinary WINDOWS_APPLICATION. Session authority is checked afterwards.
    let manifest = if description.format == 1 {
        abi::manifest::APPLICATION
    } else {
        super::super::windows::subsystem_manifest(capture.bytes()).inspect_err(|e| {
            super::super::log(format_args!(
                "DOOR_P16 EXECUTE refused stage=subsystem status={e:?}\n"
            ))
        })?
    };
    let mut authority = with_meta(|m| {
        m.executive
            .authority_for_launch(r.pid, a[0], manifest, a[1], next(&INSTANCE)?)
    })
    .inspect_err(|e| {
        super::super::log(format_args!(
            "DOOR_P16 EXECUTE refused stage=authority status={e:?}\n"
        ))
    })?;
    // The full token: only the Launch authority's holder (Init, after User
    // Account Control) asks for it.
    if description.elevated {
        authority.groups |= abi::group::ADMINISTRATORS;
    }
    let prepared = prepare(
        Some(r),
        authority,
        a[2],
        abi::mode::SESSION,
        Some((capture, description)),
    )
    .inspect_err(|e| {
        super::super::log(format_args!(
            "DOOR_P16 EXECUTE refused stage=prepare status={e:?}\n"
        ))
    })?;
    let mut result = [0; abi::LAUNCH_RESULT_BYTES];
    LaunchResult {
        process: prepared.process_handle,
        control: prepared.control,
        pid: prepared.pid.raw(),
    }
    .encode(&mut result)?;
    if let Err(error) = with_vm(|v| write(v, r, a[4], &result)) {
        with_meta(|m| {
            m.processes[prepared.pid.slot()].as_mut().unwrap().starting = false;
            m.executive.close(r.pid, prepared.process_handle).unwrap();
            m.executive.close(r.pid, prepared.control).unwrap();
        });
        run::with(|s| s.terminate(prepared.task, -1)).unwrap();
        reap();
        return Err(error);
    }
    if description.suspended {
        // CREATE_SUSPENDED: the first thread starts held; the launcher's
        // ResumeThread (PROCESS_RESUME) releases it.
        run::with(|s| s.suspend(prepared.task)).map_err(task_error)?;
        with_meta(|m| {
            m.processes[prepared.pid.slot()].as_mut().unwrap().held = Some(prepared.task)
        });
    }
    publish(&prepared)?;
    Ok(0)
}
pub(super) fn boot_read(r: Record, a: [u64; 6]) -> Result<u64, Status> {
    with_meta(|m| {
        m.executive
            .require_authority(r.pid, a[0], HandleKind::Launch)
    })?;
    let length = usize::try_from(a[2]).map_err(|_| Status::InvalidArgument)?;
    let count = usize::try_from(a[5]).map_err(|_| Status::InvalidArgument)?;
    if length == 0 || length > 255 || count > 4096 {
        return Err(Status::InvalidArgument);
    }
    let mut path = [0; 255];
    with_vm(|v| {
        read(v, r, a[1], &mut path[..length])?;
        validate(v, r, a[4], count)
    })?;
    let path = core::str::from_utf8(&path[..length]).map_err(|_| Status::InvalidArgument)?;
    if path.starts_with('/')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || path.contains('\\')
        || path.contains('\0')
    {
        return Err(Status::InvalidArgument);
    }
    let archive = unsafe { (*ARCHIVE.0.get()).as_ref().ok_or(Status::NotFound)? };
    let file = archive
        .iter()
        .flatten()
        .find(|entry| entry.path == path && !entry.directory)
        .map(|entry| entry.data)
        .ok_or(Status::NotFound)?;
    let offset = usize::try_from(a[3]).map_err(|_| Status::InvalidArgument)?;
    if offset > file.len() {
        return Err(Status::InvalidArgument);
    }
    let bytes = &file[offset..offset.saturating_add(count).min(file.len())];
    // Initfs is a boot-time low identity mapping, absent from the user's CR3.
    // Capture into the shared supervisor stack before checked-copy switches CR3.
    let mut captured = [0; 4096];
    captured[..bytes.len()].copy_from_slice(bytes);
    with_vm(|v| write(v, r, a[4], &captured[..bytes.len()]))?;
    Ok(bytes.len() as u64)
}

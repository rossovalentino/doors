//! Safe captured platform calls and explicitly unsafe address-space/thread calls.
use crate::call;
use door_abi::{
    Handle, Status, operation as op,
    platform::{BlockInfo, Identity, LaunchResult},
};

pub fn display_info(authority: Handle) -> Result<door_abi::graphics::DisplayInfo, Status> {
    let mut bytes = [0; door_abi::graphics::INFO_BYTES];
    call(
        op::DISPLAY_INFO,
        [authority, bytes.as_mut_ptr() as u64, 0, 0, 0, 0],
    )?;
    door_abi::graphics::DisplayInfo::decode(&bytes)
}
pub fn display_clock(authority: Handle) -> Result<door_abi::clock::WallClock, Status> {
    let mut bytes = [0; door_abi::clock::BYTES];
    call(
        op::DISPLAY_CLOCK,
        [authority, bytes.as_mut_ptr() as u64, 0, 0, 0, 0],
    )?;
    door_abi::clock::WallClock::decode(&bytes)
}
pub fn display_write(authority: Handle, first_pixel: u64, pixels: &[u8]) -> Result<(), Status> {
    call(
        op::DISPLAY_WRITE,
        [
            authority,
            first_pixel,
            pixels.as_ptr() as u64,
            pixels.len() as u64,
            0,
            0,
        ],
    )
    .map(|_| ())
}
pub fn display_input(authority: Handle) -> Result<door_abi::graphics::Input, Status> {
    let mut bytes = [0; door_abi::graphics::INPUT_BYTES];
    call(
        op::DISPLAY_INPUT,
        [authority, bytes.as_mut_ptr() as u64, 0, 0, 0, 0],
    )?;
    door_abi::graphics::Input::decode(&bytes)
}
pub fn display_release(authority: Handle) -> Result<(), Status> {
    call(op::DISPLAY_RELEASE, [authority, 0, 0, 0, 0, 0]).map(|_| ())
}
pub fn section_snapshot(section: Handle, offset: u64, output: &mut [u8]) -> Result<usize, Status> {
    call(
        op::SECTION_SNAPSHOT,
        [
            section,
            offset,
            output.as_mut_ptr() as u64,
            output.len() as u64,
            0,
            0,
        ],
    )
    .map(|n| n as usize)
}
/// Copy `input` into `section` at `offset` (SECTION_FILL; WRITE on the
/// handle, at most `graphics::SECTION_COPY_BYTES` per call).
pub fn section_fill(section: Handle, offset: u64, input: &[u8]) -> Result<usize, Status> {
    call(
        op::SECTION_FILL,
        [
            section,
            offset,
            input.as_ptr() as u64,
            input.len() as u64,
            0,
            0,
        ],
    )
    .map(|n| n as usize)
}

/// Live Session identity (id, epoch, uid), requiring QUERY on the held grant.
pub fn session_info(session: Handle) -> Result<(u64, u64, u32), Status> {
    let mut bytes = [0; 24];
    call(
        op::SESSION_QUERY,
        [session, bytes.as_mut_ptr() as u64, 0, 0, 0, 0],
    )?;
    Ok((
        u64::from_le_bytes(bytes[..8].try_into().unwrap()),
        u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
        u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
    ))
}
pub fn identity(channel: Handle) -> Result<Identity, Status> {
    let mut b = [0; 64];
    call(op::IDENTITY, [channel, b.as_mut_ptr() as u64, 0, 0, 0, 0])?;
    Identity::decode(&b)
}
/// Copy through the kernel's fully validated captured buffer: at most1024 bytes,
/// or4096 for the trusted Windows-base manifest's whole SDK CONTEXT transfer.
/// Invalid user addresses return BadAddress without creating Rust references to them.
/// Overlap is supported. No process memory lock survives the call.
/// # Safety
/// Any successful destination write must respect the caller's Rust aliasing,
/// initialization and synchronization obligations. Addresses may be invalid but
/// cannot designate another live Rust object that the caller may not mutate.
pub unsafe fn user_copy(destination: u64, source: u64, bytes: usize) -> Result<(), Status> {
    call(op::USER_COPY, [destination, source, bytes as u64, 0, 0, 0]).map(|_| ())
}
/// # Safety
/// The requested range has no live mappings/references and is reserved by the caller.
pub unsafe fn vm_allocate(address: u64, bytes: u64, permissions: u64) -> Result<u64, Status> {
    call(op::VM_ALLOCATE, [address, bytes, permissions, 0, 0, 0])
}
/// # Safety
/// No thread, reference, instruction or in-flight operation can use this exact range.
pub unsafe fn vm_release(address: u64, bytes: u64) -> Result<(), Status> {
    call(op::VM_RELEASE, [address, bytes, 0, 0, 0, 0]).map(|_| ())
}
/// # Safety
/// All users of the exact range obey the new access permissions; executable/stack
/// mappings are not mutated through this native data-VM interface.
pub unsafe fn vm_protect(address: u64, bytes: u64, permissions: u64) -> Result<(), Status> {
    call(op::VM_PROTECT, [address, bytes, permissions, 0, 0, 0]).map(|_| ())
}
pub fn process_launch(
    authority: Handle,
    manifest: u64,
    session: Handle,
    registry: Handle,
    mode: u64,
) -> Result<LaunchResult, Status> {
    let mut b = [0; 24];
    call(
        op::PROCESS_LAUNCH,
        [
            authority,
            manifest,
            session,
            registry,
            mode,
            b.as_mut_ptr() as u64,
        ],
    )?;
    LaunchResult::decode(&b)
}
pub fn process_terminate(process: Handle, code: i64) -> Result<(), Status> {
    call(op::PROCESS_TERMINATE, [process, code as u64, 0, 0, 0, 0]).map(|_| ())
}
/// # Safety
/// Entry is an executable nonreturning SysV function in this process. Argument
/// ownership and every referenced object outlive the new thread's retirement.
pub unsafe fn thread_create(entry: u64, argument: u64) -> Result<Handle, Status> {
    call(op::THREAD_CREATE, [entry, argument, 0, 0, 0, 0])
}
/// # Safety
/// As `thread_create`; the thread does not run until `thread_resume`.
pub unsafe fn thread_create_suspended(entry: u64, argument: u64) -> Result<Handle, Status> {
    call(op::THREAD_CREATE, [entry, argument, 1, 0, 0, 0])
}
/// SuspendThread: the previous suspend count.
pub fn thread_suspend(thread: Handle) -> Result<u64, Status> {
    call(op::THREAD_SUSPEND, [thread, 0, 0, 0, 0, 0])
}
/// ResumeThread: the previous suspend count.
pub fn thread_resume(thread: Handle) -> Result<u64, Status> {
    call(op::THREAD_RESUME, [thread, 0, 0, 0, 0, 0])
}
/// A new handle to the calling thread (DuplicateHandle of GetCurrentThread).
pub fn thread_open_self() -> Result<Handle, Status> {
    call(op::THREAD_OPEN_SELF, [0; 6])
}
/// A new handle to the calling process (OpenProcess of its own ID).
pub fn process_open_self() -> Result<Handle, Status> {
    call(op::PROCESS_OPEN_SELF, [0; 6])
}
/// Start a process launched suspended: its previous suspend count.
pub fn process_resume(process: Handle) -> Result<u64, Status> {
    call(op::PROCESS_RESUME, [process, 0, 0, 0, 0, 0])
}
pub fn thread_exit(code: i64) -> ! {
    let _ = call(op::THREAD_EXIT, [code as u64, 0, 0, 0, 0, 0]);
    #[cfg(all(target_arch = "x86_64", target_os = "none"))]
    unsafe {
        core::arch::asm!("ud2", options(noreturn));
    }
    #[cfg(not(all(target_arch = "x86_64", target_os = "none")))]
    panic!("native thread exit is unavailable on host");
}
pub fn thread_join(thread: Handle, deadline: u64) -> Result<i64, Status> {
    let mut b = [0; 8];
    call(
        op::THREAD_JOIN,
        [thread, b.as_mut_ptr() as u64, deadline, 0, 0, 0],
    )?;
    Ok(i64::from_le_bytes(b))
}

/// Windows shutdown retires sibling contexts before user-mode DLL cleanup.
/// WouldBlock requires yielding and retrying. Native processes are denied.
pub fn process_quiesce(code: u32) -> Result<(), Status> {
    call(op::PROCESS_QUIESCE, [u64::from(code), 0, 0, 0, 0, 0]).map(|_| ())
}
pub fn sleep(deadline: u64) -> Result<(), Status> {
    call(op::SLEEP, [deadline, 0, 0, 0, 0, 0]).map(|_| ())
}
/// The kernel entropy status word (`door_abi::entropy` bits); any caller.
pub fn entropy_status() -> Result<u64, Status> {
    call(op::ENTROPY_STATUS, [0; 6])
}
/// Conditioned kernel entropy into `buffer` (1..=`door_abi::entropy::READ_MAX`
/// bytes). Session/Init-capability services only; AccessDenied for others,
/// Unsupported until the kernel pool is READY.
pub fn entropy_read(buffer: &mut [u8]) -> Result<usize, Status> {
    if buffer.is_empty() || buffer.len() > door_abi::entropy::READ_MAX {
        return Err(Status::InvalidArgument);
    }
    call(
        op::ENTROPY_READ,
        [buffer.as_mut_ptr() as u64, buffer.len() as u64, 0, 0, 0, 0],
    )
    .map(|n| n as usize)
}
pub fn port_create() -> Result<Handle, Status> {
    call(op::PORT_CREATE, [0; 6])
}
pub fn port_connect(port: Handle) -> Result<Handle, Status> {
    call(op::PORT_CONNECT, [port, 0, 0, 0, 0, 0])
}
pub fn port_accept(port: Handle) -> Result<Handle, Status> {
    call(op::PORT_ACCEPT, [port, 0, 0, 0, 0, 0])
}
pub fn wait_create(kind: u64, initial: u64, flags: u64) -> Result<Handle, Status> {
    call(op::WAIT_CREATE, [kind, initial, flags, 0, 0, 0])
}
pub fn wait(handle: Handle, deadline: u64) -> Result<u64, Status> {
    call(op::WAIT, [handle, deadline, 0, 0, 0, 0])
}
pub fn signal(handle: Handle, count: u64) -> Result<(), Status> {
    call(op::SIGNAL, [handle, count, 0, 0, 0, 0]).map(|_| ())
}
pub fn reset(handle: Handle) -> Result<(), Status> {
    call(op::RESET, [handle, 0, 0, 0, 0, 0]).map(|_| ())
}
pub fn condition_wait(condition: Handle, mutex: Handle, deadline: u64) -> Result<u64, Status> {
    call(op::CONDITION_WAIT, [condition, mutex, deadline, 0, 0, 0])
}
/// Session service only: a session for the permit's account.
/// `administrators`: bit `i` when `door_abi::platform::ACCOUNTS[i]` is an
/// administrator (its applications get the filtered token).
pub fn session_create(
    authority: Handle,
    permit: Handle,
    administrators: u64,
) -> Result<Handle, Status> {
    call(
        op::SESSION_CREATE,
        [authority, permit, administrators, 0, 0, 0],
    )
}
/// Init only, after a sign-out: a new logon permit for local account `uid`
/// (1001 or 1002) once its session has ended.
pub fn permit_renew(authority: Handle, uid: u32) -> Result<Handle, Status> {
    call(op::PERMIT_RENEW, [authority, u64::from(uid), 0, 0, 0, 0])
}
pub fn session_revoke(authority: Handle, session: Handle) -> Result<(), Status> {
    call(op::SESSION_REVOKE, [authority, session, 0, 0, 0, 0]).map(|_| ())
}
pub fn block_info(block: Handle) -> Result<BlockInfo, Status> {
    let mut b = [0; 32];
    call(op::BLOCK_INFO, [block, b.as_mut_ptr() as u64, 0, 0, 0, 0])?;
    BlockInfo::decode(&b)
}
pub fn block_read(block: Handle, lba: u64, output: &mut [u8]) -> Result<(), Status> {
    call(
        op::BLOCK_READ,
        [
            block,
            lba,
            output.as_mut_ptr() as u64,
            output.len() as u64,
            0,
            0,
        ],
    )
    .map(|_| ())
}
pub fn block_write(block: Handle, lba: u64, input: &[u8]) -> Result<(), Status> {
    call(
        op::BLOCK_WRITE,
        [block, lba, input.as_ptr() as u64, input.len() as u64, 0, 0],
    )
    .map(|_| ())
}
pub fn block_flush(block: Handle) -> Result<(), Status> {
    call(op::BLOCK_FLUSH, [block, 0, 0, 0, 0, 0]).map(|_| ())
}
/// The NIC's MAC address and link state (door-network's authority handle).
pub fn net_info(nic: Handle) -> Result<door_abi::platform::NetInfo, Status> {
    let mut b = [0; door_abi::platform::NET_INFO_BYTES];
    call(op::NET_INFO, [nic, b.as_mut_ptr() as u64, 0, 0, 0, 0])?;
    door_abi::platform::NetInfo::decode(&b)
}
/// Queue one Ethernet frame (14..=1514 bytes); `WouldBlock` while every
/// transmit buffer is in flight.
pub fn net_send(nic: Handle, frame: &[u8]) -> Result<(), Status> {
    call(
        op::NET_SEND,
        [nic, frame.as_ptr() as u64, frame.len() as u64, 0, 0, 0],
    )
    .map(|_| ())
}
/// The next received frame into `out` (at least 1514 bytes); its length,
/// 0 when none waits.
pub fn net_receive(nic: Handle, out: &mut [u8]) -> Result<usize, Status> {
    call(
        op::NET_RECEIVE,
        [nic, out.as_mut_ptr() as u64, out.len() as u64, 0, 0, 0],
    )
    .map(|n| n as usize)
}
/// The sound device's output format and queue (door-audio only).
pub fn audio_info(device: Handle) -> Result<door_abi::platform::AudioInfo, Status> {
    let mut b = [0; door_abi::platform::AUDIO_INFO_BYTES];
    call(op::AUDIO_INFO, [device, b.as_mut_ptr() as u64, 0, 0, 0, 0])?;
    door_abi::platform::AudioInfo::decode(&b)
}
/// Queue one period (`AUDIO_PERIOD_BYTES` of S16LE stereo); `WouldBlock`
/// while the device holds every period it can.
pub fn audio_write(device: Handle, period: &[u8]) -> Result<(), Status> {
    call(
        op::AUDIO_WRITE,
        [device, period.as_ptr() as u64, period.len() as u64, 0, 0, 0],
    )
    .map(|_| ())
}
/// Start (`AUDIO_START`) or stop (`AUDIO_STOP`) the output stream, or
/// capture (`AUDIO_CAPTURE_START`, `AUDIO_CAPTURE_STOP`).
pub fn audio_control(device: Handle, command: u64) -> Result<(), Status> {
    call(op::AUDIO_CONTROL, [device, command, 0, 0, 0, 0]).map(|_| ())
}
/// The next captured period (`AUDIO_PERIOD_BYTES` of S16LE stereo) into
/// `period`: false while none waits.
pub fn audio_read(device: Handle, period: &mut [u8]) -> Result<bool, Status> {
    call(
        op::AUDIO_READ,
        [
            device,
            period.as_mut_ptr() as u64,
            period.len() as u64,
            0,
            0,
            0,
        ],
    )
    .map(|v| v == 1)
}
pub fn console_write(console: Handle, input: &[u8]) -> Result<usize, Status> {
    call(
        op::CONSOLE_WRITE,
        [console, input.as_ptr() as u64, input.len() as u64, 0, 0, 0],
    )
    .map(|v| v as usize)
}
pub fn console_read(console: Handle) -> Result<u8, Status> {
    let value = call(op::CONSOLE_READ, [console, 0, 0, 0, 0, 0])?;
    u8::try_from(value).map_err(|_| Status::IoFailure)
}

/// Capture file bytes and admit an unprivileged application under a live session.
pub fn process_execute(
    authority: Handle,
    session: Handle,
    registry: Handle,
    image: &[u8],
    description: &door_abi::platform::ExecuteDescriptor,
) -> Result<LaunchResult, Status> {
    let mut description = *description;
    description.image_pointer = image.as_ptr() as u64;
    description.image_bytes = image.len() as u64;
    let mut descriptor = [0; door_abi::platform::EXECUTE_DESCRIPTOR_BYTES];
    description.encode(&mut descriptor)?;
    let mut result = [0; door_abi::platform::LAUNCH_RESULT_BYTES];
    call(
        op::PROCESS_EXECUTE,
        [
            authority,
            session,
            registry,
            descriptor.as_ptr() as u64,
            result.as_mut_ptr() as u64,
            0,
        ],
    )?;
    LaunchResult::decode(&result)
}
pub fn boot_file_read(
    authority: Handle,
    path: &str,
    offset: u64,
    output: &mut [u8],
) -> Result<usize, Status> {
    call(
        op::BOOT_FILE_READ,
        [
            authority,
            path.as_ptr() as u64,
            path.len() as u64,
            offset,
            output.as_mut_ptr() as u64,
            output.len() as u64,
        ],
    )
    .map(|n| n as usize)
}
/// A QUERY|READ|MAP section handle for an on-demand read-only resource
/// (`door_abi::platform::resource`), shared by every process that opens it.
pub fn resource_open(id: u64) -> Result<Handle, Status> {
    call(op::RESOURCE_OPEN, [id, 0, 0, 0, 0, 0])
}
pub fn console_control(authority: Handle, command: u64, value: u64) -> Result<(), Status> {
    call(op::CONSOLE_CONTROL, [authority, command, value, 0, 0, 0]).map(|_| ())
}

/// Only the immutable Monitor image in a live authenticated session is admitted.
pub fn system_snapshot() -> Result<door_abi::telemetry::Snapshot, Status> {
    let mut bytes = [0; door_abi::telemetry::BYTES];
    call(
        op::SYSTEM_SNAPSHOT,
        [bytes.as_mut_ptr() as u64, bytes.len() as u64, 0, 0, 0, 0],
    )?;
    door_abi::telemetry::Snapshot::decode(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_calls_do_not_emulate_guest_services_or_mutate_output() {
        assert_eq!(identity(0), Err(Status::Unsupported));
        assert_eq!(port_create(), Err(Status::Unsupported));
        assert_eq!(process_launch(1, 2, 0, 0, 1), Err(Status::Unsupported));
        assert_eq!(thread_join(1, 10), Err(Status::Unsupported));
        assert_eq!(entropy_status(), Err(Status::Unsupported));
        let mut e = [0x5a; 16];
        assert_eq!(entropy_read(&mut e), Err(Status::Unsupported));
        assert_eq!(e, [0x5a; 16]);
        assert_eq!(entropy_read(&mut []), Err(Status::InvalidArgument));
        assert_eq!(entropy_read(&mut [0; 65]), Err(Status::InvalidArgument));
        let mut b = [0xa5; 512];
        assert_eq!(block_read(1, 0, &mut b), Err(Status::Unsupported));
        assert_eq!(b, [0xa5; 512]);
    }
}

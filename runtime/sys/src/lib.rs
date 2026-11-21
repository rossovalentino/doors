#![no_std]
mod platform;
use door_abi::{AbiInfo, Handle, MAX_TRANSFERS, Metadata, Status, Transfer, operation as op};
pub use platform::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Return {
    pub status: u64,
    pub value: u64,
}
impl Return {
    pub fn result(self) -> Result<u64, Status> {
        match Status::from_raw(self.status) {
            Some(Status::Success) => Ok(self.value),
            Some(error) => Err(error),
            None => Err(Status::IoFailure),
        }
    }
}
/// Performs the raw ABI trap. The compiler observes both reads and writes to memory.
/// # Safety
/// All pointer arguments and operation-specific memory lifetimes must satisfy the
/// selected operation. Unmapping/reprotecting storage may invalidate Rust references.
#[cfg(all(
    target_arch = "x86_64",
    any(
        target_os = "none",
        all(target_os = "windows", feature = "door-personality", not(test))
    )
))]
pub unsafe fn raw(operation: u32, args: [u64; 6]) -> Return {
    let mut status = door_abi::selector(operation);
    let mut value = args[2];
    unsafe {
        core::arch::asm!("syscall",inlateout("rax")status,in("rdi")args[0],in("rsi")args[1],inlateout("rdx")value,in("r10")args[3],in("r8")args[4],in("r9")args[5],lateout("rcx")_,lateout("r11")_,options(nostack));
    }
    Return { status, value }
}
/// 32-bit (x86) Windows programs (P17): Door's trap is the 64-bit `syscall`
/// instruction, which compatibility mode does not have. The call widens
/// every argument explicitly into a block of 64-bit words, far-calls the
/// 64-bit user code segment (0x33), traps there and far-returns to the
/// 32-bit segment with the result. Kernel ABI structures use fixed-width
/// fields, so pointers below 4 GB mean the same thing on both sides.
/// # Safety
/// Same contract as the x64 trap.
#[cfg(all(
    target_arch = "x86",
    target_os = "windows",
    feature = "door-personality",
    not(test)
))]
pub unsafe fn raw(operation: u32, args: [u64; 6]) -> Return {
    // [selector, arguments 0..6, status, value].
    let mut block = [
        door_abi::selector(operation),
        args[0],
        args[1],
        args[2],
        args[3],
        args[4],
        args[5],
        0,
        0,
    ];
    unsafe {
        door_gate64(block.as_mut_ptr());
    }
    Return {
        status: block[7],
        value: block[8],
    }
}
#[cfg(all(
    target_arch = "x86",
    target_os = "windows",
    feature = "door-personality",
    not(test)
))]
unsafe extern "C" {
    fn door_gate64(block: *mut u64);
}
// The gate: 32-bit code saves the callee-saved registers and far-calls
// 0x33:long; the 64-bit part zero-extends RSP and the block pointer (their
// upper halves are undefined after compatibility mode), loads the Door
// argument registers, traps, stores status and value and far-returns
// (32-bit operand size: EIP and CS as the far call pushed them).
#[cfg(all(
    target_arch = "x86",
    target_os = "windows",
    feature = "door-personality",
    not(test)
))]
core::arch::global_asm!(
    ".globl _door_gate64",
    "_door_gate64:",
    "pushl %ebx",
    "pushl %esi",
    "pushl %edi",
    "pushl %ebp",
    "movl 20(%esp), %esi",
    "lcall $0x33, $door_gate64_long",
    "popl %ebp",
    "popl %edi",
    "popl %esi",
    "popl %ebx",
    "retl",
    ".code64",
    "door_gate64_long:",
    "movl %esp, %esp",
    "movl %esi, %esi",
    "movq %rsi, %r12",
    "movq (%r12), %rax",
    "movq 8(%r12), %rdi",
    "movq 16(%r12), %rsi",
    "movq 24(%r12), %rdx",
    "movq 32(%r12), %r10",
    "movq 40(%r12), %r8",
    "movq 48(%r12), %r9",
    "syscall",
    "movq %rax, 56(%r12)",
    "movq %rdx, 64(%r12)",
    "lretl",
    ".code32",
    options(att_syntax)
);
/// Host builds provide no host-OS syscall emulation.
/// # Safety
/// Same contract as the actual target trap; always returns Unsupported on a host.
#[cfg(not(any(
    all(
        target_arch = "x86_64",
        any(
            target_os = "none",
            all(target_os = "windows", feature = "door-personality", not(test))
        )
    ),
    all(
        target_arch = "x86",
        target_os = "windows",
        feature = "door-personality",
        not(test)
    )
)))]
pub unsafe fn raw(_operation: u32, _args: [u64; 6]) -> Return {
    Return {
        status: Status::Unsupported as u64,
        value: 0,
    }
}

fn call(operation: u32, args: [u64; 6]) -> Result<u64, Status> {
    // Each public wrapper establishes the pointer/lifetime contract or exposes it
    // as an explicit unsafe precondition for mapping mutation.
    unsafe { raw(operation, args) }.result()
}
pub fn query() -> Result<AbiInfo, Status> {
    let mut b = [0; door_abi::QUERY_BYTES];
    call(
        op::QUERY,
        [b.as_mut_ptr() as u64, b.len() as u64, 0, 0, 0, 0],
    )?;
    AbiInfo::decode(&b)
}
pub fn hardware_info() -> Result<door_abi::hardware::HardwareInfo, Status> {
    let mut bytes = [0; door_abi::hardware::BYTES];
    call(
        op::HARDWARE_INFO,
        [bytes.as_mut_ptr() as u64, bytes.len() as u64, 0, 0, 0, 0],
    )?;
    door_abi::hardware::HardwareInfo::decode(&bytes)
}
pub fn write(bytes: &[u8]) -> Result<usize, Status> {
    call(
        op::WRITE,
        [bytes.as_ptr() as u64, bytes.len() as u64, 0, 0, 0, 0],
    )
    .map(|v| v as usize)
}
pub fn exit(code: i64) -> ! {
    let _ = call(op::EXIT, [code as u64, 0, 0, 0, 0, 0]);
    // Successful exit cannot return. An unsupported/broken kernel must contain
    // this process instead of continuing in a caller that assumes termination.
    #[cfg(any(
        all(
            target_arch = "x86_64",
            any(
                target_os = "none",
                all(target_os = "windows", feature = "door-personality", not(test))
            )
        ),
        all(
            target_arch = "x86",
            target_os = "windows",
            feature = "door-personality",
            not(test)
        )
    ))]
    unsafe {
        core::arch::asm!("ud2", options(noreturn));
    }
    #[cfg(not(any(
        all(
            target_arch = "x86_64",
            any(
                target_os = "none",
                all(target_os = "windows", feature = "door-personality", not(test))
            )
        ),
        all(
            target_arch = "x86",
            target_os = "windows",
            feature = "door-personality",
            not(test)
        )
    )))]
    panic!("native exit is unavailable on host");
}
pub fn time() -> Result<u64, Status> {
    call(op::TIME, [0; 6])
}
/// UTC wall time as FILETIME (100 ns since 1601-01-01). `Unsupported`: no
/// valid RTC; `WouldBlock`: the RTC stayed mid-update (retry later).
pub fn wall_time() -> Result<u64, Status> {
    call(op::WALL_TIME, [0; 6])
}
/// Time since boot in 100 ns units (`operation::PRECISE_TIME`).
pub fn precise_time() -> Result<u64, Status> {
    call(op::PRECISE_TIME, [0; 6])
}
/// (physical bytes, free physical bytes, this process's private bytes,
/// commit limit), `operation::MEMORY_STATUS`.
pub fn memory_status() -> Result<[u64; 4], Status> {
    let mut bytes = [0u8; 32];
    call(
        op::MEMORY_STATUS,
        [bytes.as_mut_ptr() as u64, 0, 0, 0, 0, 0],
    )?;
    let mut out = [0u64; 4];
    for (value, chunk) in out.iter_mut().zip(bytes.chunks_exact(8)) {
        *value = u64::from_le_bytes(chunk.try_into().unwrap_or([0; 8]));
    }
    Ok(out)
}
pub fn yield_now() -> Result<(), Status> {
    call(op::YIELD, [0; 6]).map(|_| ())
}
pub fn close(handle: Handle) -> Result<(), Status> {
    call(op::CLOSE, [handle, 0, 0, 0, 0, 0]).map(|_| ())
}
pub fn duplicate(handle: Handle, rights: u64) -> Result<Handle, Status> {
    call(op::DUPLICATE, [handle, rights, 0, 0, 0, 0])
}
/// Returns [object kind, effective rights, opaque object identity, generation].
/// The machine of the process `handle` names (P17): 0x14C for a 32-bit
/// Windows program, 0x8664 otherwise.
pub fn process_machine(handle: Handle) -> Result<u16, Status> {
    call(op::PROCESS_MACHINE, [handle, 0, 0, 0, 0, 0]).map(|machine| machine as u16)
}
pub fn inspect(handle: Handle) -> Result<[u64; 4], Status> {
    let mut b = [0u8; 32];
    call(op::INSPECT, [handle, b.as_mut_ptr() as u64, 0, 0, 0, 0])?;
    let info = door_abi::HandleInfo::decode(&b)?;
    Ok([info.kind, info.rights, info.object_id, info.generation])
}
pub fn section_create(bytes: u64, rights: u64) -> Result<Handle, Status> {
    call(op::SECTION_CREATE, [bytes, rights, 0, 0, 0, 0])
}
/// # Safety
/// The chosen virtual range must be reserved for this mapping and must have no
/// live Rust references. Aliases and cross-process synchronization are the caller's responsibility.
pub unsafe fn section_map(handle: Handle, address: u64, rights: u64) -> Result<u64, Status> {
    call(op::SECTION_MAP, [handle, address, rights, 0, 0, 0])
}
/// # Safety
/// No live references, active instruction/stack, or in-flight operation may use the range.
pub unsafe fn section_unmap(address: u64, bytes: u64) -> Result<(), Status> {
    call(op::SECTION_UNMAP, [address, bytes, 0, 0, 0, 0]).map(|_| ())
}
pub fn process_wait(handle: Handle, deadline: u64) -> Result<i64, Status> {
    let mut output = [0u8; 8];
    call(
        op::PROCESS_WAIT,
        [handle, output.as_mut_ptr() as u64, deadline, 0, 0, 0],
    )?;
    Ok(i64::from_le_bytes(output))
}
pub fn channel_create() -> Result<[Handle; 2], Status> {
    let mut output = [0u8; 16];
    call(
        op::CHANNEL_CREATE,
        [output.as_mut_ptr() as u64, 0, 0, 0, 0, 0],
    )?;
    Ok([
        u64::from_le_bytes(output[..8].try_into().unwrap()),
        u64::from_le_bytes(output[8..].try_into().unwrap()),
    ])
}
pub fn channel_peer(handle: Handle) -> Result<u64, Status> {
    call(op::CHANNEL_PEER, [handle, 0, 0, 0, 0, 0])
}
pub fn channel_send(handle: Handle, packet: &[u8], transfers: &[Transfer]) -> Result<(), Status> {
    channel_send_with(op::CHANNEL_SEND, handle, packet, transfers, 0)
}
pub fn channel_send_deadline(
    handle: Handle,
    packet: &[u8],
    transfers: &[Transfer],
    deadline: u64,
) -> Result<(), Status> {
    channel_send_with(
        op::CHANNEL_SEND_DEADLINE,
        handle,
        packet,
        transfers,
        deadline,
    )
}
pub fn channel_cancel(handle: Handle, request_id: u64) -> Result<door_abi::CancelOutcome, Status> {
    let result = call(op::CHANNEL_CANCEL, [handle, request_id, 0, 0, 0, 0])?;
    door_abi::CancelOutcome::from_raw(result).ok_or(Status::IoFailure)
}
fn channel_send_with(
    operation: u32,
    handle: Handle,
    packet: &[u8],
    transfers: &[Transfer],
    deadline: u64,
) -> Result<(), Status> {
    if transfers.len() > MAX_TRANSFERS {
        return Err(Status::InvalidArgument);
    }
    let mut encoded = [0u8; MAX_TRANSFERS * door_abi::TRANSFER_BYTES];
    for (i, transfer) in transfers.iter().enumerate() {
        transfer.encode(&mut encoded[i * 32..i * 32 + 32])?;
    }
    call(
        operation,
        [
            handle,
            packet.as_ptr() as u64,
            packet.len() as u64,
            encoded.as_ptr() as u64,
            transfers.len() as u64,
            deadline,
        ],
    )
    .map(|_| ())
}
pub fn channel_receive(
    handle: Handle,
    packet: &mut [u8],
    transfers: &mut [Transfer],
) -> Result<Metadata, Status> {
    if transfers.len() > MAX_TRANSFERS {
        return Err(Status::InvalidArgument);
    }
    let mut encoded = [0u8; MAX_TRANSFERS * door_abi::TRANSFER_BYTES];
    let mut metadata = [0u8; door_abi::METADATA_BYTES];
    call(
        op::CHANNEL_RECEIVE,
        [
            handle,
            packet.as_mut_ptr() as u64,
            packet.len() as u64,
            encoded.as_mut_ptr() as u64,
            transfers.len() as u64,
            metadata.as_mut_ptr() as u64,
        ],
    )?;
    let result = Metadata::decode(&metadata)?;
    if result.handle_count > transfers.len() as u64 || result.packet_bytes > packet.len() as u64 {
        return Err(Status::IoFailure);
    }
    for (i, transfer) in transfers
        .iter_mut()
        .take(result.handle_count as usize)
        .enumerate()
    {
        *transfer = Transfer::decode(&encoded[i * 32..i * 32 + 32])?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_never_claims_native_execution_and_errors_keep_native_identity() {
        assert_eq!(query(), Err(Status::Unsupported));
        assert_eq!(write(b"x"), Err(Status::Unsupported));
        assert_eq!(channel_cancel(1, 1), Err(Status::Unsupported));
        assert_eq!(
            channel_send_deadline(1, &[], &[], 10),
            Err(Status::Unsupported)
        );
        assert_eq!(
            Return {
                status: 0,
                value: u64::MAX
            }
            .result(),
            Ok(u64::MAX)
        );
        assert_eq!(
            Return {
                status: 6,
                value: 0
            }
            .result(),
            Err(Status::AccessDenied)
        );
        assert_eq!(
            Return {
                status: 999,
                value: 0
            }
            .result(),
            Err(Status::IoFailure)
        );
    }
}

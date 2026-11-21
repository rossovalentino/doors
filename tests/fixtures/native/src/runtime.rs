#![no_std]
#![no_main]
use core::{
    fmt::{self, Write},
    sync::atomic::{AtomicU64, Ordering},
};
use door_abi::{
    CancelOutcome, Header, MAX_PACKET, MAX_TRANSFERS, Metadata, Startup, Status, Transfer,
    operation, rights, role,
};
use door_sys as sys;

// One thread per fixture process; distinct address spaces give each run its own
// diagnostic stage. These stable codes survive optimized builds without a heap.
#[repr(u64)]
enum Stage {
    Startup = 1,
    ClientSection = 10,
    ClientGrant = 11,
    ClientUpdate = 12,
    ClientCleanup = 13,
    ServerGrant = 20,
    ServerDenial = 21,
    ServerMap = 22,
    ServerUpdate = 23,
    ServerCleanup = 24,
    NegativePointers = 30,
    NegativeHandles = 31,
    NegativeQueue = 32,
    NegativeMove = 33,
    NegativePressure = 34,
    CancelQueued = 35,
    DeadlineExpired = 36,
    DeadlineQueued = 37,
    CancelDispatched = 38,
    SurvivorWait = 40,
}
static STAGE: AtomicU64 = AtomicU64::new(Stage::Startup as u64);
fn stage(value: Stage) {
    STAGE.store(value as u64, Ordering::Relaxed);
}
struct Diagnostic {
    bytes: [u8; 128],
    length: usize,
}
impl Write for Diagnostic {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let end = self.length.checked_add(value.len()).ok_or(fmt::Error)?;
        let output = self.bytes.get_mut(self.length..end).ok_or(fmt::Error)?;
        output.copy_from_slice(value.as_bytes());
        self.length = end;
        Ok(())
    }
}

const SECTION: u64 = 0x7200_0000;
const FIRST: u64 = 0x1122_3344_5566_7788;
const SECOND: u64 = 0x8877_6655_4433_2211;
const SECTION_RIGHTS: u64 = rights::QUERY
    | rights::DUPLICATE
    | rights::TRANSFER
    | rights::READ
    | rights::WRITE
    | rights::MAP;
const SHARED_RIGHTS: u64 = rights::QUERY | rights::READ | rights::MAP;
door_rt::entry!(main);

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    door_rt::panic()
}

fn main(start: Startup) -> i64 {
    match execute(start) {
        Ok(code) => code,
        Err(status) => {
            let stage = STAGE.load(Ordering::Relaxed);
            let mut diagnostic = Diagnostic {
                bytes: [0; 128],
                length: 0,
            };
            let _ = writeln!(
                &mut diagnostic,
                "DOOR_P05 USER FAILURE role={} stage={} status={}",
                start.role, stage, status as u64
            );
            let _ = sys::write(&diagnostic.bytes[..diagnostic.length]);
            -(1000 + stage as i64 * 100 + status as i64)
        }
    }
}
fn require(condition: bool) -> Result<(), Status> {
    if condition {
        Ok(())
    } else {
        Err(Status::IoFailure)
    }
}
fn execute(start: Startup) -> Result<i64, Status> {
    let query = sys::query()?;
    require(query.tick_hz == 100 && query.groups & 0x3f == 0x3f)?;
    let self_tls: u64;
    let tls_pid: u64;
    unsafe {
        core::arch::asm!("mov {}, fs:[0]",out(reg)self_tls,options(nostack,readonly));
        core::arch::asm!("mov {}, fs:[8]",out(reg)tls_pid,options(nostack,readonly));
    }
    require(self_tls == start.tls && tls_pid == start.pid)?;
    match start.role {
        role::CLIENT => client(&start),
        role::SERVER => server(&start),
        role::SURVIVOR => survivor(&start),
        role::MALICIOUS => {
            for _ in 0..100 {
                sys::yield_now()?;
            }
            // Intentional CPL3 read of supervisor kernel text. It must terminate
            // only this process and must never reach the failure return.
            unsafe {
                core::ptr::read_volatile(0xffff_ffff_8000_0000 as *const u64);
            }
            Err(Status::IoFailure)
        }
        role::INVALID_RETURN => {
            // A native caller cannot make a noncanonical return stack valid by
            // entering through a recognized syscall. No recovery code follows.
            unsafe {
                core::arch::asm!("mov rsp, 0x0000800000000000","syscall","ud2",in("rax")0xd001_0000_0000_0400u64,options(noreturn));
            }
        }
        _ => Err(Status::InvalidArgument),
    }
}
fn send(
    channel: u64,
    opcode: u32,
    id: u64,
    payload: &[u8],
    transfers: &[Transfer],
) -> Result<(), Status> {
    send_with_deadline(channel, opcode, id, payload, transfers, None)
}
fn send_with_deadline(
    channel: u64,
    opcode: u32,
    id: u64,
    payload: &[u8],
    transfers: &[Transfer],
    deadline: Option<u64>,
) -> Result<(), Status> {
    let mut packet = [0; MAX_PACKET];
    let header = Header {
        opcode,
        flags: 0,
        request_id: id,
        payload_bytes: payload.len() as u32,
        handle_count: transfers.len() as u16,
    };
    header.encode(&mut packet[..32])?;
    packet[32..32 + payload.len()].copy_from_slice(payload);
    match deadline {
        Some(deadline) => {
            sys::channel_send_deadline(channel, &packet[..32 + payload.len()], transfers, deadline)
        }
        None => sys::channel_send(channel, &packet[..32 + payload.len()], transfers),
    }
}
struct Received {
    metadata: Metadata,
    header: Header,
    packet: [u8; MAX_PACKET],
    transfers: [Transfer; MAX_TRANSFERS],
}
impl Received {
    fn payload(&self) -> &[u8] {
        &self.packet[32..self.metadata.packet_bytes as usize]
    }
}
fn receive(channel: u64) -> Result<Received, Status> {
    let deadline = sys::time()?.saturating_add(500);
    let mut packet = [0; MAX_PACKET];
    let mut transfers = [Transfer::default(); MAX_TRANSFERS];
    loop {
        match sys::channel_receive(channel, &mut packet, &mut transfers) {
            Ok(metadata) => {
                let (header, _) = Header::packet(&packet[..metadata.packet_bytes as usize])?;
                return Ok(Received {
                    metadata,
                    header,
                    packet,
                    transfers,
                });
            }
            Err(Status::WouldBlock) => {
                if sys::time()? >= deadline {
                    return Err(Status::Timeout);
                }
                sys::yield_now()?;
            }
            Err(error) => return Err(error),
        }
    }
}
fn peer(
    start: &Startup,
    received: &Received,
    opcode: u32,
    id: u64,
    payload: &[u8],
) -> Result<(), Status> {
    require(
        received.metadata.sender_pid == start.peer_pid
            && received.header.opcode == opcode
            && received.header.request_id == id
            && received.payload() == payload,
    )
}
fn client(start: &Startup) -> Result<i64, Status> {
    stage(Stage::ClientSection);
    let channel = start.inherited[1].handle;
    require(sys::channel_peer(channel)? == start.peer_pid)?;
    let section = sys::section_create(4096, SECTION_RIGHTS)?;
    require(unsafe { sys::section_map(section, SECTION, rights::READ | rights::WRITE) }? == 4096)?;
    require(unsafe { core::ptr::read_volatile(SECTION as *const u64) } == 0)?;
    unsafe {
        core::ptr::write_volatile(SECTION as *mut u64, FIRST);
    }
    let grant = Transfer {
        handle: section,
        rights: SHARED_RIGHTS,
        mode: 0,
    };
    stage(Stage::ClientGrant);
    send(channel, 0x10, 1, b"door-section", &[grant])?;
    let reply = receive(channel)?;
    peer(start, &reply, 0x11, 1, b"section-seen")?;
    require(reply.header.handle_count == 0)?;
    stage(Stage::ClientUpdate);
    unsafe {
        core::ptr::write_volatile(SECTION as *mut u64, SECOND);
    }
    send(channel, 0x12, 2, b"section-update", &[])?;
    let reply = receive(channel)?;
    peer(start, &reply, 0x13, 2, b"updated-seen")?;
    stage(Stage::ClientCleanup);
    unsafe { sys::section_unmap(SECTION, 4096) }?;
    sys::close(section)?;
    sys::close(channel)?;
    negative_calls()?;
    sys::write(b"DOOR_P05 CLIENT PASS\n")?;
    Ok(11)
}
fn server(start: &Startup) -> Result<i64, Status> {
    stage(Stage::ServerGrant);
    let channel = start.inherited[1].handle;
    require(sys::channel_peer(channel)? == start.peer_pid)?;
    let request = receive(channel)?;
    peer(start, &request, 0x10, 1, b"door-section")?;
    require(
        request.header.handle_count == 1
            && request.metadata.handle_count == 1
            && request.transfers[0].rights == SHARED_RIGHTS,
    )?;
    let section = request.transfers[0].handle;
    stage(Stage::ServerDenial);
    require(sys::duplicate(section, SECTION_RIGHTS) == Err(Status::AccessDenied))?;
    require(
        unsafe { sys::section_map(section, SECTION, rights::READ | rights::WRITE) }
            == Err(Status::AccessDenied),
    )?;
    require(
        unsafe { sys::section_map(section, SECTION, rights::WRITE) }
            == Err(Status::InvalidArgument),
    )?;
    stage(Stage::ServerMap);
    require(unsafe { sys::section_map(section, SECTION, rights::READ) }? == 4096)?;
    require(unsafe { core::ptr::read_volatile(SECTION as *const u64) } == FIRST)?;
    send(channel, 0x11, 1, b"section-seen", &[])?;
    stage(Stage::ServerUpdate);
    let request = receive(channel)?;
    peer(start, &request, 0x12, 2, b"section-update")?;
    require(unsafe { core::ptr::read_volatile(SECTION as *const u64) } == SECOND)?;
    stage(Stage::ServerCleanup);
    unsafe { sys::section_unmap(SECTION, 4096) }?;
    sys::close(section)?;
    send(channel, 0x13, 2, b"updated-seen", &[])?;
    sys::close(channel)?;
    sys::write(b"DOOR_P05 SERVER PASS\n")?;
    Ok(22)
}
fn survivor(start: &Startup) -> Result<i64, Status> {
    stage(Stage::SurvivorWait);
    let handle = start.inherited[0].handle;
    let deadline = sys::time()?.saturating_add(500);
    loop {
        match sys::process_wait(handle, deadline) {
            Ok(exit) => {
                require(exit == -14)?;
                break;
            }
            Err(Status::WouldBlock) => {
                if sys::time()? >= deadline {
                    return Err(Status::Timeout);
                }
                sys::yield_now()?;
            }
            Err(error) => return Err(error),
        }
    }
    for _ in 0..32 {
        sys::yield_now()?;
    }
    sys::write(b"DOOR_P05 SURVIVOR PASS\n")?;
    Ok(44)
}
fn negative_calls() -> Result<(), Status> {
    stage(Stage::NegativePointers);
    let mut partial = [0xa5u8; 64];
    let result = unsafe {
        sys::raw(
            operation::QUERY,
            [partial.as_mut_ptr() as u64, 63, 0, 0, 0, 0],
        )
    };
    require(result.status != 0 && result.value == 0 && partial == [0xa5; 64])?;
    let result = unsafe { sys::raw(operation::WRITE, [0x0000_7fff_ffff_ffff, 2, 0, 0, 0, 0]) };
    require(result.result() == Err(Status::BadAddress))?;
    require(sys::close(0) == Err(Status::BadHandle))?;
    stage(Stage::NegativeHandles);
    let section = sys::section_create(4096, SECTION_RIGHTS)?;
    let reduced = sys::duplicate(section, rights::READ | rights::MAP)?;
    require(sys::duplicate(reduced, SECTION_RIGHTS) == Err(Status::AccessDenied))?;
    sys::close(reduced)?;
    require(sys::inspect(reduced) == Err(Status::BadHandle))?;
    let replacement = sys::duplicate(section, rights::READ)?;
    require(replacement != reduced)?;
    sys::close(replacement)?;
    sys::close(section)?;
    stage(Stage::NegativeQueue);
    let pair = sys::channel_create()?;
    for id in 1..=4 {
        send(pair[0], 0x20, id, b"queue", &[])?;
    }
    require(send(pair[0], 0x20, 5, b"queue", &[]) == Err(Status::WouldBlock))?;
    let mut too_small = [0xa5; 1];
    let mut transfers = [];
    require(
        sys::channel_receive(pair[1], &mut too_small, &mut transfers).is_err()
            && too_small == [0xa5],
    )?;
    let item = receive(pair[1])?;
    require(item.header.request_id == 1)?;
    // Reusing the failed fifth request succeeds: queue-full did not consume it.
    send(pair[0], 0x20, 5, b"queue", &[])?;
    for id in 2..=5 {
        let item = receive(pair[1])?;
        require(item.header.request_id == id)?;
    }
    sys::close(pair[0])?;
    let mut packet = [0; MAX_PACKET];
    require(sys::channel_receive(pair[1], &mut packet, &mut transfers) == Err(Status::PeerClosed))?;
    sys::close(pair[1])?;
    // MOVE commits source invalidation with the queued grant. The receiver gets
    // a distinct generation-bound handle and only the explicitly reduced rights.
    stage(Stage::NegativeMove);
    let pair = sys::channel_create()?;
    let section = sys::section_create(4096, SECTION_RIGHTS)?;
    send(
        pair[0],
        0x21,
        1,
        b"move",
        &[Transfer {
            handle: section,
            rights: SHARED_RIGHTS,
            mode: 1,
        }],
    )?;
    require(sys::inspect(section) == Err(Status::BadHandle))?;
    let item = receive(pair[1])?;
    require(item.header.handle_count == 1 && item.transfers[0].rights == SHARED_RIGHTS)?;
    let received = item.transfers[0].handle;
    require(received != section && sys::inspect(received)?[1] == SHARED_RIGHTS)?;
    sys::close(received)?;
    sys::close(pair[0])?;
    sys::close(pair[1])?;
    // Bounded handle pressure must fail without damaging existing grants.
    stage(Stage::NegativePressure);
    let section = sys::section_create(4096, SECTION_RIGHTS)?;
    // The current executive admits64 handles. Retain every successful duplicate
    // until the real quota refuses one; never infer pressure from a local limit.
    let mut copies = [0u64; 64];
    let mut count = 0;
    loop {
        match sys::duplicate(section, rights::READ) {
            Ok(handle) => {
                require(count < copies.len())?;
                copies[count] = handle;
                count += 1;
            }
            Err(Status::ResourceExhausted) => break,
            Err(error) => return Err(error),
        }
    }
    require(count > 0 && sys::inspect(section)?[1] == SECTION_RIGHTS)?;
    for handle in &copies[..count] {
        sys::close(*handle)?;
    }
    sys::close(section)?;
    ipc_lifetimes()?;
    sys::write(b"DOOR_P05 NEGATIVE PASS\n")?;
    Ok(())
}
fn empty(channel: u64) -> Result<(), Status> {
    let mut packet = [0; MAX_PACKET];
    let mut transfers = [Transfer::default(); MAX_TRANSFERS];
    require(sys::channel_receive(channel, &mut packet, &mut transfers) == Err(Status::WouldBlock))
}
fn ipc_lifetimes() -> Result<(), Status> {
    let pair = sys::channel_create()?;
    stage(Stage::CancelQueued);
    let section = sys::section_create(4096, SECTION_RIGHTS)?;
    let grant = Transfer {
        handle: section,
        rights: SHARED_RIGHTS,
        mode: 1,
    };
    send(pair[0], 0x30, 1, b"cancel-move", &[grant])?;
    require(sys::inspect(section) == Err(Status::BadHandle))?;
    require(sys::channel_cancel(pair[0], 1)? == CancelOutcome::Canceled)?;
    require(sys::channel_cancel(pair[0], 1)? == CancelOutcome::AlreadyCanceled)?;
    empty(pair[1])?;
    // A canceled request's sequence number stays consumed, preventing confusion.
    require(send(pair[0], 0x30, 1, b"reuse", &[]) == Err(Status::InvalidArgument))?;
    send(pair[0], 0x30, 2, b"after-cancel", &[])?;
    require(receive(pair[1])?.header.request_id == 2)?;

    stage(Stage::DeadlineExpired);
    let section = sys::section_create(4096, SECTION_RIGHTS)?;
    let grant = Transfer {
        handle: section,
        rights: SHARED_RIGHTS,
        mode: 1,
    };
    require(
        send_with_deadline(
            pair[0],
            0x31,
            3,
            b"expired-before-send",
            &[grant],
            Some(sys::time()?),
        ) == Err(Status::Timeout),
    )?;
    require(sys::inspect(section)?[1] == SECTION_RIGHTS)?;
    // An operation rejected before enqueue keeps both source grant and sequence ID.
    send(pair[0], 0x31, 3, b"retry-after-timeout", &[grant])?;
    require(sys::inspect(section) == Err(Status::BadHandle))?;
    let received = receive(pair[1])?;
    require(received.header.request_id == 3 && received.header.handle_count == 1)?;
    sys::close(received.transfers[0].handle)?;

    stage(Stage::DeadlineQueued);
    let section = sys::section_create(4096, SECTION_RIGHTS)?;
    let grant = Transfer {
        handle: section,
        rights: SHARED_RIGHTS,
        mode: 1,
    };
    let limit = sys::time()?.saturating_add(500);
    let deadline = loop {
        let now = sys::time()?;
        if now >= limit {
            return Err(Status::Timeout);
        }
        let deadline = now.saturating_add(2);
        match send_with_deadline(
            pair[0],
            0x32,
            4,
            b"expire-queued-move",
            &[grant],
            Some(deadline),
        ) {
            Ok(()) => break deadline,
            // Scheduling may consume the two-tick window before entry. The ABI
            // guarantees this rejection is precommit, so retrying is unambiguous.
            Err(Status::Timeout) => sys::yield_now()?,
            Err(error) => return Err(error),
        }
    };
    require(sys::inspect(section) == Err(Status::BadHandle))?;
    while sys::time()? <= deadline {
        sys::yield_now()?;
    }
    require(sys::channel_cancel(pair[0], 4)? == CancelOutcome::Expired)?;
    empty(pair[1])?;
    require(sys::channel_cancel(pair[0], 4)? == CancelOutcome::Expired)?;

    stage(Stage::CancelDispatched);
    send(pair[0], 0x33, 5, b"already-dispatched", &[])?;
    require(receive(pair[1])?.header.request_id == 5)?;
    require(sys::channel_cancel(pair[0], 5)? == CancelOutcome::Dispatched)?;
    require(sys::channel_cancel(pair[0], 5)? == CancelOutcome::Dispatched)?;
    require(sys::channel_cancel(pair[0], 99) == Err(Status::NotFound))?;
    require(sys::channel_cancel(pair[0], 0) == Err(Status::InvalidArgument))?;
    sys::close(pair[0])?;
    sys::close(pair[1])?;
    sys::write(b"DOOR_P05 IPC_LIFETIME PASS\n")?;
    Ok(())
}

//! Independent protected native adversary. No compositor internals are linked.
//! This is admitted only by the separate desktop test image's trusted Init policy.
use core::sync::atomic::{AtomicBool, Ordering};
use door_abi::{
    Handle, HandleKind, Status, Transfer,
    platform::{StartupV2, inherited},
    rights,
};
use door_service_client::Init;
use door_service_protocol::{Code, Frame, Message, window as w};
static STOP_WRITER: AtomicBool = AtomicBool::new(false);
unsafe extern "C" fn producer_writer(_: u64) -> ! {
    while !STOP_WRITER.load(Ordering::Acquire) {
        // SAFETY: the main thread retains the checked producer mapping and joins
        // before unmapping. Both sides use raw volatile stores, with no Rust view.
        for i in 0..256 {
            unsafe {
                (0x7200_0000 as *mut u8).add(i * 4).write_volatile(96);
            }
        }
        let _ = door_sys::yield_now();
    }
    door_rt::thread::finish(0)
}
struct Rpc {
    channel: Handle,
    instance: u64,
    next: u64,
}
impl Drop for Rpc {
    fn drop(&mut self) {
        let _ = door_sys::close(self.channel);
    }
}
impl Rpc {
    fn call(
        &mut self,
        op: u32,
        body: &[u8],
        handles: &[Transfer],
    ) -> Result<(Code, [u8; 240], usize), Status> {
        self.next += 1;
        let packet = Message::new(
            op,
            self.next,
            self.instance,
            Code::Success,
            body,
            handles.len(),
        )?;
        let deadline = door_sys::time()? + 500;
        door_sys::channel_send_deadline(self.channel, packet.bytes(), handles, deadline)?;
        let mut bytes = [0; door_abi::MAX_PACKET];
        let mut transfers = [Transfer::default(); door_abi::MAX_TRANSFERS];
        loop {
            match door_sys::channel_receive(self.channel, &mut bytes, &mut transfers) {
                Ok(meta) => {
                    for t in &transfers[..meta.handle_count as usize] {
                        let _ = door_sys::close(t.handle);
                    }
                    let frame = Frame::decode(&bytes[..meta.packet_bytes as usize])?;
                    if frame.header.request_id != self.next
                        || frame.header.opcode != op
                        || frame.instance != self.instance
                        || meta.handle_count != 0
                        || frame.body.len() > 240
                    {
                        return Err(Status::InvalidArgument);
                    }
                    let mut out = [0; 240];
                    out[..frame.body.len()].copy_from_slice(frame.body);
                    return Ok((frame.code, out, frame.body.len()));
                }
                Err(Status::WouldBlock) => {
                    if door_sys::time()? >= deadline {
                        return Err(Status::Timeout);
                    }
                    door_sys::sleep(door_sys::time()? + 1)?;
                }
                Err(e) => return Err(e),
            }
        }
    }
    fn expect(
        &mut self,
        op: u32,
        b: &[u8],
        h: &[Transfer],
        expected: Code,
    ) -> Result<[u8; 240], Status> {
        let (code, out, _) = self.call(op, b, h)?;
        if code != expected {
            super::log(format_args!(
                "DOOR_P11 ADVERSARY mismatch op={} expected={} actual={}\n",
                op, expected as u32, code as u32
            ));
            return Err(Status::IoFailure);
        }
        Ok(out)
    }
    fn scalar(&mut self, op: u32, b: &[u8], h: &[Transfer]) -> Result<u64, Status> {
        let (code, out, n) = self.call(op, b, h)?;
        if code != Code::Success || n != 8 {
            return Err(Status::IoFailure);
        }
        Ok(u64::from_le_bytes(out[..8].try_into().unwrap()))
    }
    fn create(&mut self, role: u32, parent: u64) -> Result<u64, Status> {
        self.scalar(w::CREATE, &create(role, parent), &[])
    }
}
fn create(role: u32, parent: u64) -> [u8; 32] {
    let mut b = [0; 32];
    b[..4].copy_from_slice(&role.to_le_bytes());
    w::Rect {
        x: 760,
        y: 32,
        width: 220,
        height: 150,
    }
    .encode(&mut b[4..20]);
    b[24..32].copy_from_slice(&parent.to_le_bytes());
    b
}
fn pair(a: u64, b: u64) -> [u8; 16] {
    let mut out = [0; 16];
    out[..8].copy_from_slice(&a.to_le_bytes());
    out[8..].copy_from_slice(&b.to_le_bytes());
    out
}
fn descriptor(width: u32, height: u32, stride: u64, bytes: u64) -> [u8; 24] {
    let mut out = [0; 24];
    out[..4].copy_from_slice(&width.to_le_bytes());
    out[4..8].copy_from_slice(&height.to_le_bytes());
    out[8..16].copy_from_slice(&stride.to_le_bytes());
    out[16..].copy_from_slice(&bytes.to_le_bytes());
    out
}
fn checked(startup: StartupV2) -> Result<(), Status> {
    super::log(format_args!("DOOR_P11 ADVERSARY ENTER\n"));
    let registry = startup.base.inherited[inherited::REGISTRY].handle;
    let port = Init::connect(registry)
        .and_then(|mut c| c.resolve(door_abi::platform::manifest::WINDOW))
        .map_err(|e| {
            super::log(format_args!("DOOR_P11 ADVERSARY RESOLVE error={e:?}\n"));
            Status::IoFailure
        })?;
    super::log(format_args!("DOOR_P11 ADVERSARY RESOLVED\n"));
    let channel = door_sys::port_connect(port)?;
    door_sys::close(port)?;
    let identity = door_sys::identity(channel)?;
    if identity.manifest != door_abi::platform::manifest::WINDOW {
        return Err(Status::AccessDenied);
    }
    let mut rpc = Rpc {
        channel,
        instance: identity.instance,
        next: 0,
    };
    // Wait for two unrelated real windows before trying a foreign identity.
    let deadline = door_sys::time()? + 1000;
    let mut first = true;
    loop {
        let (_, out, _) = rpc.call(w::HELLO, &1u32.to_le_bytes(), &[])?;
        if first {
            super::log(format_args!("DOOR_P11 ADVERSARY HELLO\n"));
            first = false;
        }
        if u32::from_le_bytes(out[44..48].try_into().unwrap()) >= 2 {
            break;
        }
        if door_sys::time()? > deadline {
            return Err(Status::Timeout);
        }
        door_sys::sleep(door_sys::time()? + 10)?;
    }
    super::log(format_args!("DOOR_P11 ADVERSARY CLIENTS_READY\n"));
    for forbidden in [0u32, 4, 5, 6, 7, u32::MAX] {
        rpc.expect(
            w::LAUNCH_APP,
            &forbidden.to_le_bytes(),
            &[],
            Code::InvalidRequest,
        )?;
    }
    rpc.expect(w::HELLO, &2u32.to_le_bytes(), &[], Code::Unsupported)?;
    let mut hello = [0; 8];
    hello[..4].copy_from_slice(&w::VERSION.to_le_bytes());
    hello[4..].copy_from_slice(&(w::CAP_TRANSIENT_DISMISS | w::CAP_EPOCH_COMMIT).to_le_bytes());
    let (code, out, _) = rpc.call(w::HELLO, &hello, &[])?;
    let negotiated = w::CAP_CPU_SNAPSHOT
        | w::CAP_TRANSIENT_DISMISS
        | w::CAP_EPOCH_COMMIT
        | w::CAP_BAND_COMMIT
        | w::CAP_ACRYLIC;
    if code != Code::Success || u32::from_le_bytes(out[20..24].try_into().unwrap()) != negotiated {
        return Err(Status::IoFailure);
    }
    // Legacy clients retain exactly the original capability/event contract.
    let (code, out, _) = rpc.call(w::HELLO, &w::VERSION.to_le_bytes(), &[])?;
    if code != Code::Success
        || u32::from_le_bytes(out[20..24].try_into().unwrap()) != w::CAP_CPU_SNAPSHOT
    {
        return Err(Status::IoFailure);
    }
    rpc.expect(w::BIND_SESSION, &[0; 24], &[], Code::Denied)?;
    rpc.expect(w::CREATE, &create(1, 0), &[], Code::Unsupported)?;
    rpc.expect(w::CREATE, &create(2, 0), &[], Code::Unsupported)?;
    rpc.expect(w::CREATE, &create(3, 0x1_0000_0001), &[], Code::Denied)?;
    rpc.expect(
        w::DESTROY,
        &0x1_0000_0001u64.to_le_bytes(),
        &[],
        Code::Denied,
    )?;
    rpc.expect(
        w::SET_TITLE,
        &0x1_0000_0001u64.to_le_bytes(),
        &[],
        Code::Denied,
    )?;
    rpc.expect(w::CREATE, &[0; 7], &[], Code::InvalidRequest)?;
    let mut cursor = [0; 12];
    cursor[..8].copy_from_slice(&0x1_0000_0001u64.to_le_bytes());
    cursor[8..].copy_from_slice(&w::CURSOR_TEXT.to_le_bytes());
    rpc.expect(w::SET_CURSOR, &cursor, &[], Code::Denied)?;
    rpc.expect(w::SET_CURSOR, &cursor[..11], &[], Code::InvalidRequest)?;
    rpc.expect(0x5fff, &[], &[], Code::InvalidRequest)?;
    if !matches!(
        door_sys::display_info(0),
        Err(Status::BadHandle | Status::AccessDenied)
    ) {
        return Err(Status::AccessDenied);
    }
    let window = rpc.create(0, 0)?;
    super::log(format_args!("DOOR_P11 ADVERSARY CREATED\n"));
    cursor[..8].copy_from_slice(&window.to_le_bytes());
    rpc.expect(w::SET_CURSOR, &cursor, &[], Code::Success)?;
    // Every protocol shape is accepted (the splitter's horizontal resize
    // too); the first undefined one is refused.
    cursor[8..].copy_from_slice(&w::CURSOR_SIZE_WE.to_le_bytes());
    rpc.expect(w::SET_CURSOR, &cursor, &[], Code::Success)?;
    cursor[8..].copy_from_slice(&(w::CURSOR_SIZE_WE + 1).to_le_bytes());
    rpc.expect(w::SET_CURSOR, &cursor, &[], Code::InvalidRequest)?;
    let epoch = loop {
        let (code, out, _) = rpc.call(w::POLL, &[], &[])?;
        if code == Code::WouldBlock {
            door_sys::sleep(door_sys::time()? + 1)?;
            continue;
        }
        if code != Code::Success {
            return Err(Status::IoFailure);
        }
        let event = w::Event::decode(&out[..64]).map_err(|_| Status::InvalidArgument)?;
        if event.kind == w::CONFIGURED && event.window == window {
            break event.value;
        }
    };
    if epoch == 0 {
        return Err(Status::IoFailure);
    }
    super::log(format_args!("DOOR_P11 ADVERSARY CONFIGURED\n"));
    rpc.expect(w::ACK, &pair(window, epoch + 1), &[], Code::StaleInstance)?;
    rpc.expect(w::COMMIT, &window.to_le_bytes(), &[], Code::InvalidRequest)?;
    let section = door_sys::section_create(4096, rights::ALL & !rights::WAIT)?;
    let grant = [Transfer {
        handle: section,
        rights: rights::QUERY | rights::DUPLICATE | rights::TRANSFER | rights::MAP | rights::READ,
        mode: 0,
    }];
    if door_sys::inspect(section)?[0] != HandleKind::Section as u64 {
        return Err(Status::IoFailure);
    }
    rpc.expect(
        w::REGISTER,
        &descriptor(220, 118, 4, 4096),
        &grant,
        Code::InvalidRequest,
    )?;
    rpc.expect(
        w::REGISTER,
        &descriptor(u32::MAX, 3, u64::MAX, u64::MAX),
        &grant,
        Code::InvalidRequest,
    )?;
    let (small, _, _) = rpc.call(w::REGISTER, &descriptor(220, 118, 880, 103840), &grant)?;
    if small == Code::Success {
        return Err(Status::IoFailure);
    }
    let mut writable = grant;
    writable[0].rights |= rights::WRITE;
    rpc.expect(
        w::REGISTER,
        &descriptor(1, 1, 4, 4096),
        &writable,
        Code::Denied,
    )?;
    if !matches!(
        door_sys::section_snapshot(section, 4096, &mut [0]),
        Err(Status::BadAddress)
    ) {
        return Err(Status::IoFailure);
    }
    // Actual private snapshot and release under producer writes/reuse.
    let bytes = 220 * 118 * 4;
    let mapped = (bytes + 4095) & !4095;
    let pixels = door_sys::section_create(mapped, rights::ALL & !rights::WAIT)?;
    unsafe {
        door_sys::section_map(pixels, 0x7200_0000, rights::READ | rights::WRITE)?;
    }
    // Deliberate raw ABI negatives: no Rust reference is constructed at either
    // invalid output. Kernel validation must reject them before any copy.
    if unsafe {
        door_sys::raw(
            door_abi::operation::SECTION_SNAPSHOT,
            [pixels, 0, 0x7200_0000, 1, 0, 0],
        )
    }
    .result()
        != Err(Status::AccessDenied)
    {
        return Err(Status::IoFailure);
    }
    if unsafe {
        door_sys::raw(
            door_abi::operation::SECTION_SNAPSHOT,
            [pixels, 0, 1, 1, 0, 0],
        )
    }
    .result()
        != Err(Status::BadAddress)
    {
        return Err(Status::IoFailure);
    }
    for i in 0..bytes as usize {
        unsafe {
            (0x7200_0000 as *mut u8)
                .add(i)
                .write_volatile(if i % 4 == 3 { 255 } else { 80 })
        }
    }
    let capture_grant = [Transfer {
        handle: pixels,
        ..grant[0]
    }];
    let buffer = rpc.scalar(
        w::REGISTER,
        &descriptor(220, 118, 880, bytes),
        &capture_grant,
    )?;
    rpc.expect(w::ACK, &pair(window, epoch), &[], Code::Success)?;
    let mut attach = [0; 24];
    attach[..16].copy_from_slice(&pair(window, buffer));
    attach[16..].copy_from_slice(&epoch.to_le_bytes());
    rpc.expect(w::ATTACH, &attach, &[], Code::Success)?;
    rpc.expect(w::UNREGISTER, &buffer.to_le_bytes(), &[], Code::WouldBlock)?;
    STOP_WRITER.store(false, Ordering::Release);
    // SAFETY: static entry, process-owned retained mapping, explicit join below.
    let writer = unsafe { door_rt::thread::spawn(producer_writer, 0) }?;
    super::log(format_args!("DOOR_P11 ADVERSARY WRITER_STARTED\n"));
    rpc.expect(w::COMMIT, &window.to_le_bytes(), &[], Code::Success)?;
    STOP_WRITER.store(true, Ordering::Release);
    if door_rt::thread::join(writer, door_sys::time()? + 500)? != 0 {
        return Err(Status::IoFailure);
    }
    door_sys::close(writer)?;
    super::log(format_args!("DOOR_P11 ADVERSARY SNAPSHOT_PASS\n"));
    let mut released = 0;
    for _ in 0..64 {
        let (code, out, _) = rpc.call(w::POLL, &[], &[])?;
        if code == Code::WouldBlock {
            break;
        }
        let e = w::Event::decode(&out[..64]).map_err(|_| Status::IoFailure)?;
        if e.kind == w::RELEASED && e.value == buffer {
            released += 1;
        }
    }
    if released != 1 {
        return Err(Status::IoFailure);
    }
    // Reuse safely after release; malformed premultiplication must retain prior safe pixels.
    unsafe {
        (0x7200_0000 as *mut u8).add(3).write_volatile(0);
    }
    rpc.expect(w::ATTACH, &attach, &[], Code::Success)?;
    rpc.expect(w::COMMIT, &window.to_le_bytes(), &[], Code::InvalidRequest)?;
    let mut configure = [0; 24];
    configure[..8].copy_from_slice(&window.to_le_bytes());
    w::Rect {
        x: 760,
        y: 32,
        width: 230,
        height: 160,
    }
    .encode(&mut configure[8..]);
    let next = rpc.scalar(w::CONFIGURE, &configure, &[])?;
    rpc.expect(w::COMMIT, &pair(window, epoch), &[], Code::StaleInstance)?;
    rpc.expect(w::COMMIT, &pair(window, next), &[], Code::InvalidRequest)?;
    rpc.expect(w::ACK, &pair(window, epoch), &[], Code::StaleInstance)?;
    if next <= epoch {
        return Err(Status::IoFailure);
    }
    rpc.expect(w::UNREGISTER, &buffer.to_le_bytes(), &[], Code::Success)?;
    rpc.expect(w::ACK, &pair(window, next), &[], Code::Success)?;
    attach[16..].copy_from_slice(&next.to_le_bytes());
    rpc.expect(w::ATTACH, &attach, &[], Code::InvalidHandle)?;
    unsafe {
        door_sys::section_unmap(0x7200_0000, mapped)?;
    }
    door_sys::close(pixels)?;
    // Exercise actual registration and window quotas, then generational reuse.
    let mut buffers = [0; 8];
    for id in &mut buffers {
        *id = rpc.scalar(w::REGISTER, &descriptor(1, 1, 4, 4096), &grant)?;
    }
    rpc.expect(
        w::REGISTER,
        &descriptor(1, 1, 4, 4096),
        &grant,
        Code::Resources,
    )?;
    for id in buffers {
        rpc.expect(w::UNREGISTER, &id.to_le_bytes(), &[], Code::Success)?;
    }
    let popup = rpc.create(3, window)?;
    let third = rpc.create(0, 0)?;
    let fourth = rpc.create(0, 0)?;
    rpc.expect(w::CREATE, &create(0, 0), &[], Code::Resources)?;
    rpc.expect(w::DESTROY, &window.to_le_bytes(), &[], Code::Success)?;
    rpc.expect(w::DESTROY, &popup.to_le_bytes(), &[], Code::InvalidHandle)?;
    rpc.expect(w::DESTROY, &third.to_le_bytes(), &[], Code::Success)?;
    rpc.expect(w::DESTROY, &fourth.to_le_bytes(), &[], Code::Success)?;
    let replacement = rpc.create(0, 0)?;
    if replacement == window {
        return Err(Status::IoFailure);
    }
    rpc.expect(w::ACK, &pair(window, epoch), &[], Code::InvalidHandle)?;
    rpc.expect(w::DESTROY, &replacement.to_le_bytes(), &[], Code::Success)?;
    let popup_section = door_sys::section_create(128 * 1024, rights::ALL & !rights::WAIT)?;
    super::log(format_args!("DOOR_P11 ADVERSARY QUOTAS_PASS\n"));
    let popup_grant = [Transfer {
        handle: popup_section,
        ..grant[0]
    }];
    for cycle in 0..32 {
        let id = rpc.create(0, 0)?;
        let popup = rpc.create(3, id)?;
        let deadline = door_sys::time()? + 500;
        let epoch = loop {
            let (code, out, _) = rpc.call(w::POLL, &[], &[])?;
            if code == Code::Success {
                let e = w::Event::decode(&out[..64]).map_err(|_| Status::IoFailure)?;
                if e.kind == w::CONFIGURED && e.window == popup {
                    break e.value;
                }
            } else if code != Code::WouldBlock {
                return Err(Status::IoFailure);
            }
            if door_sys::time()? > deadline {
                return Err(Status::Timeout);
            }
            door_sys::sleep(door_sys::time()? + 1)?;
        };
        let buffer = rpc.scalar(
            w::REGISTER,
            &descriptor(220, 118, 880, 103840),
            &popup_grant,
        )?;
        rpc.expect(w::ACK, &pair(popup, epoch), &[], Code::Success)?;
        let mut attach = [0; 24];
        attach[..16].copy_from_slice(&pair(popup, buffer));
        attach[16..].copy_from_slice(&epoch.to_le_bytes());
        rpc.expect(w::ATTACH, &attach, &[], Code::Success)?;
        rpc.expect(w::DESTROY, &id.to_le_bytes(), &[], Code::Success)?;
        rpc.expect(w::UNREGISTER, &buffer.to_le_bytes(), &[], Code::Success)?;
        let (_, out, _) = rpc.call(w::HELLO, &1u32.to_le_bytes(), &[])?;
        if u32::from_le_bytes(out[52..56].try_into().unwrap()) != 0 {
            return Err(Status::IoFailure);
        }
        if cycle % 8 == 7 {
            super::log(format_args!(
                "DOOR_P11 ADVERSARY POPUPS cycles={}\n",
                cycle + 1
            ));
        }
    }
    door_sys::close(popup_section)?;
    // Intentionally abandon a pending window/registration. Channel/process death
    // must release these, while the unrelated Files/Notes clients keep running.
    let abandoned = rpc.create(0, 0)?;
    let _ = rpc.scalar(w::REGISTER, &descriptor(1, 1, 4, 4096), &grant)?;
    door_sys::close(section)?;
    let _ = door_sys::write(b"DOOR_P11 ADVERSARY ARMED press-Escape-in-probe\n");
    let deadline = door_sys::time()? + 1500;
    let mut foreign_closes = 0;
    loop {
        // The first real Files window belongs to another authenticated process.
        // The host holds native drag/caption gestures during these denied RPCs.
        rpc.expect(
            w::REQUEST_CLOSE,
            &0x1_0000_0001u64.to_le_bytes(),
            &[],
            Code::Denied,
        )?;
        foreign_closes += 1;
        if foreign_closes % 8 == 0 {
            let _ = door_sys::write(b"DOOR_P12 FOREIGN_CLOSE_DENIED\n");
        }
        let (code, out, _) = rpc.call(w::POLL, &[], &[])?;
        if code == Code::Success {
            let event = w::Event::decode(&out[..64]).map_err(|_| Status::IoFailure)?;
            if event.kind == w::INPUT
                && event.window == abandoned
                && event.detail == 1
                && event.a & 511 == 1
                && event.b == 1
            {
                break;
            }
        } else if code != Code::WouldBlock {
            return Err(Status::IoFailure);
        }
        if door_sys::time()? > deadline {
            return Err(Status::Timeout);
        }
        door_sys::sleep(door_sys::time()? + 1)?;
    }
    core::mem::forget(rpc);
    Ok(())
}
pub fn run(startup: StartupV2) -> i64 {
    if door_sys::system_snapshot() != Err(Status::AccessDenied) {
        return -274;
    }
    let _ = door_sys::write(b"DOOR_MONITOR FOREIGN_CALLER_DENIED\n");
    match checked(startup) {
        Ok(()) => {
            let _=door_sys::write(b"DOOR_P11 ADVERSARY PASS ownership=1 descriptor=1 epoch=1 release=1 quota=1 reuse=32 abandoned=1\n");
            73
        }
        Err(e) => {
            super::log(format_args!(
                "DOOR_P11 ADVERSARY FAIL status={}\n",
                e as u64
            ));
            -273
        }
    }
}
pub fn hang(startup: StartupV2) -> i64 {
    let result = (|| -> Result<(), Status> {
        let port = Init::connect(startup.base.inherited[inherited::REGISTRY].handle)
            .and_then(|mut c| c.resolve(door_abi::platform::manifest::WINDOW))
            .map_err(|_| Status::IoFailure)?;
        let channel = door_sys::port_connect(port)?;
        door_sys::close(port)?;
        let identity = door_sys::identity(channel)?;
        let mut rpc = Rpc {
            channel,
            instance: identity.instance,
            next: 0,
        };
        rpc.expect(w::HELLO, &1u32.to_le_bytes(), &[], Code::Success)?;
        let id = rpc.create(0, 0)?;
        let mut title = [0; 21];
        title[..8].copy_from_slice(&id.to_le_bytes());
        title[8..].copy_from_slice(b"Hung producer");
        rpc.expect(w::SET_TITLE, &title, &[], Code::Success)?;
        core::mem::forget(rpc);
        Ok(())
    })();
    if let Err(error) = result {
        super::log(format_args!("DOOR_P11 HANG FAIL status={}\n", error as u64));
        return -274;
    }
    let _ = door_sys::write(b"DOOR_P11 HANG ARMED close-via-caption\n");
    loop {
        let _ = door_sys::yield_now();
    }
}

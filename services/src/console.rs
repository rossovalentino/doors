//! The session's consoles: what programs write (to the kernel console, the
//! session's transcript, and to their console's mirror for the Terminal),
//! what they read (typed input), and each console screen's state as the
//! Windows console host keeps it.
//!
//! Console 0 is the session's; the Terminal opens a pseudoconsole (1..) for
//! each tab that runs a program, as Windows Terminal gives each tab its own
//! pseudoconsole, and its programs (and theirs: Init passes
//! `DOOR_CONSOLE=<n>` on) attach to it at startup. A client's requests act
//! on the console its channel is attached to.
//!
//! The Terminal shows a running program's output in a grid; the service
//! follows the same output with a cursor tracker (`door_vt::Tracker`), so
//! a program asking where its cursor is (GetConsoleScreenBufferInfo, the
//! DSR cursor report) gets the answer at once, as conhost gives it. The
//! Terminal sets each grid's size and passes typed keys as
//! win32-input-mode sequences (`CONSOLE_INPUT`); a console's input mode is
//! shared by its programs, as one console input buffer is on Windows. Ctrl+C
//! typed while a console's input is processed is a signal (`signals`) the
//! program's console control handlers take (`seen`), not input.
use crate::server::{self, Handler, Response, Server};
use door_abi::{
    Handle, Status, Transfer,
    platform::{Identity, StartupV2, inherited, mode},
};
use door_service_protocol::{CHUNK_BYTES, Code, console, manifest, opcode};
/// Typed input not yet read, per console.
const INPUT_BYTES: usize = 2048;
/// ENABLE_PROCESSED_INPUT: Ctrl+C is a signal.
const PROCESSED_INPUT: u32 = 1;
/// One console screen.
struct Screen {
    open: bool,
    input: [u8; INPUT_BYTES],
    queued: usize,
    mirror: Mirror,
    tracker: door_vt::Tracker,
    /// The console input buffer's mode (ENABLE_LINE_INPUT, ...).
    input_mode: u32,
    /// Ctrl+C signals sent, and the last one a program's handlers took.
    signals: u32,
    seen: u32,
}
impl Screen {
    const fn new() -> Self {
        Self {
            open: false,
            input: [0; INPUT_BYTES],
            queued: 0,
            mirror: Mirror::new(),
            tracker: door_vt::Tracker::new(console::COLUMNS, console::ROWS),
            input_mode: console::INPUT_MODE,
            signals: 0,
            seen: 0,
        }
    }
    fn reset(&mut self) {
        self.input.fill(0);
        self.queued = 0;
        self.mirror.clear();
        self.tracker.reset();
        self.input_mode = console::INPUT_MODE;
        self.signals = 0;
        self.seen = 0;
    }
    /// Queue input bytes, all or nothing (a key's sequence stays whole).
    fn queue(&mut self, bytes: &[u8]) -> bool {
        if self.queued + bytes.len() > self.input.len() {
            return false;
        }
        self.input[self.queued..self.queued + bytes.len()].copy_from_slice(bytes);
        self.queued += bytes.len();
        true
    }
    /// The program asked for reports (DSR, DA): the answers are input.
    fn answer_reports(&mut self) {
        let mut replies = [0u8; 128];
        let n = self.tracker.take_replies(&mut replies);
        let _ = self.queue(&replies[..n]);
    }
    fn take(&mut self, out: &mut [u8]) -> usize {
        let n = self.queued.min(out.len());
        out[..n].copy_from_slice(&self.input[..n]);
        self.input.copy_within(n..self.queued, 0);
        self.input[self.queued - n..self.queued].fill(0);
        self.queued -= n;
        n
    }
    fn state(&self, index: usize) -> Response {
        let c = self.tracker.cursor();
        let (x, y) = c.position();
        let (top, bottom) = c.margins();
        let state = console::State {
            columns: c.columns() as u16,
            rows: c.rows() as u16,
            x: x as u16,
            y: y as u16,
            top: top as u16,
            bottom: bottom as u16,
            flags: [
                (c.visible, console::VISIBLE),
                (c.alternate, console::ALTERNATE),
                (c.application_cursor, console::APPLICATION_CURSOR),
                (c.bracketed_paste, console::BRACKETED_PASTE),
                (c.newline, console::NEWLINE),
                (c.autowrap, console::AUTOWRAP),
            ]
            .into_iter()
            .filter(|(on, _)| *on)
            .fold(0, |flags, (_, bit)| flags | bit),
            input_mode: self.input_mode,
            console: index as u16,
            signals: self.signals,
            seen: self.seen,
        };
        Response::bytes(&state.encode())
    }
}
/// Whether `bytes` is exactly one Ctrl+C key-down event (a win32-input-mode
/// sequence whose character is 3, or the byte 3 itself).
fn control_c(bytes: &[u8]) -> bool {
    if bytes == [3] {
        return true;
    }
    match door_vt::input::decode(bytes, false) {
        door_vt::input::Decoded::Keys(keys, n) => {
            n == bytes.len() && keys.count == 1 && keys.events[0].down && keys.events[0].ch == 3
        }
        _ => false,
    }
}
/// The session's console and the pseudoconsoles the Terminal has open.
/// A pseudoconsole's screen is made on the service's heap when the
/// Terminal opens it and released when it closes, so there is no fixed
/// number of them (only the protocol's one-byte id).
struct Screens {
    session: Screen,
    pseudo: [*mut Screen; console::CONSOLES],
}
impl Screens {
    fn new() -> Self {
        let mut session = Screen::new();
        session.open = true;
        Self {
            session,
            pseudo: [core::ptr::null_mut(); console::CONSOLES],
        }
    }
    fn get(&mut self, index: usize) -> Option<&mut Screen> {
        match index {
            0 => Some(&mut self.session),
            // SAFETY: a non-null entry is a screen `open` made and only
            // `close` releases; the service handles one request at a time.
            i => self.pseudo.get(i).and_then(|p| unsafe { p.as_mut() }),
        }
    }
    /// A new pseudoconsole: its id, or None when every id is in use or the
    /// heap is full.
    fn open(&mut self) -> Option<usize> {
        let free = (1..console::CONSOLES).find(|&i| self.pseudo[i].is_null())?;
        let screen = allocate_screen();
        if screen.is_null() {
            return None;
        }
        self.pseudo[free] = screen;
        Some(free)
    }
    fn close(&mut self, index: usize) {
        if index == 0 {
            return;
        }
        let screen = core::mem::replace(&mut self.pseudo[index], core::ptr::null_mut());
        if !screen.is_null() {
            // SAFETY: made by `allocate_screen`, unreachable from now on.
            unsafe { release_screen(screen) };
        }
    }
    fn close_all(&mut self) {
        for i in 1..console::CONSOLES {
            self.close(i);
        }
        self.session.reset();
        self.session.open = true;
    }
}
impl core::ops::Index<usize> for Screens {
    type Output = Screen;
    /// Only consoles that exist: channels attach to open consoles, and
    /// closing one moves its channels back to the session's.
    fn index(&self, index: usize) -> &Screen {
        match index {
            0 => &self.session,
            // SAFETY: as in `get`.
            i => unsafe { self.pseudo[i].as_ref() }.expect("open console"),
        }
    }
}
impl core::ops::IndexMut<usize> for Screens {
    fn index_mut(&mut self, index: usize) -> &mut Screen {
        self.get(index).expect("open console")
    }
}
/// The service's heap: one pseudoconsole screen per allocation.
#[cfg(not(test))]
static HEAP: door_rt::HeapAllocator = door_rt::HeapAllocator::new();
#[cfg(not(test))]
fn allocate_screen() -> *mut Screen {
    use core::alloc::{GlobalAlloc, Layout};
    // SAFETY: a fresh allocation of a Screen's layout, written whole before
    // anything reads it.
    unsafe {
        let screen = HEAP.alloc(Layout::new::<Screen>()).cast::<Screen>();
        if !screen.is_null() {
            screen.write(Screen::new());
            (*screen).open = true;
        }
        screen
    }
}
#[cfg(not(test))]
unsafe fn release_screen(screen: *mut Screen) {
    use core::alloc::{GlobalAlloc, Layout};
    // SAFETY: the caller passes a screen `allocate_screen` made.
    unsafe {
        (*screen).reset();
        HEAP.dealloc(screen.cast(), Layout::new::<Screen>());
    }
}
#[cfg(test)]
fn allocate_screen() -> *mut Screen {
    extern crate std;
    let mut screen = std::boxed::Box::new(Screen::new());
    screen.open = true;
    std::boxed::Box::into_raw(screen)
}
#[cfg(test)]
unsafe fn release_screen(screen: *mut Screen) {
    extern crate std;
    // SAFETY: made by `allocate_screen` (Box::into_raw).
    drop(unsafe { std::boxed::Box::from_raw(screen) });
}

struct Console {
    authority: Handle,
    owner: Ownership,
    screens: Screens,
    /// The console each connected channel uses (0 until it attaches).
    attached: [(Handle, u8); server::CONNECTIONS],
}
impl Console {
    fn new(authority: Handle) -> Self {
        Self {
            authority,
            owner: Ownership::new(),
            screens: Screens::new(),
            attached: [(0, 0); server::CONNECTIONS],
        }
    }
    fn console_of(&self, channel: Handle) -> usize {
        self.attached
            .iter()
            .find(|(c, _)| *c == channel && channel != 0)
            .map_or(0, |(_, n)| usize::from(*n))
    }
    fn attach(&mut self, channel: Handle, index: u8) -> Result<(), Code> {
        if self.screens.get(usize::from(index)).is_none_or(|s| !s.open) {
            return Err(Code::NotFound);
        }
        if let Some(slot) = self.attached.iter_mut().find(|(c, _)| *c == channel) {
            slot.1 = index;
            return Ok(());
        }
        let slot = self
            .attached
            .iter_mut()
            .find(|(c, _)| *c == 0)
            .ok_or(Code::Resources)?;
        *slot = (channel, index);
        Ok(())
    }
}
/// Text written by a console's clients, drained by the Terminal
/// (`CONSOLE_MIRROR`). When full, the oldest bytes give way: the kernel's
/// `DOOR_CONSOLE` records remain the complete transcript.
struct Mirror {
    bytes: [u8; 8192],
    start: usize,
    len: usize,
}
impl Mirror {
    const fn new() -> Self {
        Self {
            bytes: [0; 8192],
            start: 0,
            len: 0,
        }
    }
    fn push(&mut self, input: &[u8]) {
        let cap = self.bytes.len();
        for &byte in input {
            if self.len == cap {
                self.start = (self.start + 1) % cap;
                self.len -= 1;
            }
            self.bytes[(self.start + self.len) % cap] = byte;
            self.len += 1;
        }
    }
    fn take(&mut self, out: &mut [u8]) -> usize {
        let n = out.len().min(self.len);
        for slot in &mut out[..n] {
            *slot = self.bytes[self.start];
            self.start = (self.start + 1) % self.bytes.len();
        }
        self.len -= n;
        n
    }
    fn clear(&mut self) {
        self.bytes.fill(0);
        self.start = 0;
        self.len = 0;
    }
}
struct Ownership {
    active: Option<(u64, u64)>,
    channels: [Handle; server::CONNECTIONS],
}
fn interrupt(
    input: &mut [u8; INPUT_BYTES],
    queued: &mut usize,
    mut read: impl FnMut() -> Result<u8, Status>,
) -> Result<(bool, bool), Status> {
    let mut overflow = false;
    for _ in 0..64 {
        match read() {
            Ok(3) => return Ok((true, overflow)),
            Ok(byte) if *queued < input.len() => {
                input[*queued] = byte;
                *queued += 1;
            }
            Ok(_) => overflow = true,
            Err(Status::WouldBlock) => break,
            Err(error) => return Err(error),
        }
    }
    Ok((false, overflow))
}
impl Ownership {
    const fn new() -> Self {
        Self {
            active: None,
            channels: [0; server::CONNECTIONS],
        }
    }
    fn attach(&mut self, channel: Handle, peer: Identity) -> Result<(), Code> {
        if !server::session_peer(peer) || channel == 0 {
            return Err(Code::Denied);
        }
        if self
            .active
            .is_some_and(|active| active != (peer.session, peer.epoch))
        {
            return Err(Code::Denied);
        }
        if !self.channels.contains(&channel) {
            let slot = self
                .channels
                .iter_mut()
                .find(|c| **c == 0)
                .ok_or(Code::Resources)?;
            *slot = channel;
        }
        self.active = Some((peer.session, peer.epoch));
        Ok(())
    }
    fn detach(&mut self, channel: Handle) {
        for entry in &mut self.channels {
            if *entry == channel {
                *entry = 0;
            }
        }
        if self.channels.iter().all(|&c| c == 0) {
            self.active = None;
        }
    }
}
impl Handler for Console {
    fn request(
        &mut self,
        channel: Handle,
        peer: Identity,
        operation: u32,
        body: &[u8],
        transfers: &[Transfer],
    ) -> Response {
        if !server::session_peer(peer) || !transfers.is_empty() {
            return Response::error(Code::Denied);
        }
        let terminal = peer.manifest == door_abi::platform::manifest::WORKSPACE;
        let command = console::Command::decode(body);
        match operation {
            opcode::CONSOLE_WRITE
                if body.len() <= CHUNK_BYTES && core::str::from_utf8(body).is_ok() => {}
            opcode::CONSOLE_READ if body.is_empty() => (),
            opcode::CONSOLE_CONTROL if body.len() == 8 => (),
            opcode::CONSOLE_INTERRUPT
                if body.is_empty() && peer.manifest == door_abi::platform::manifest::SHELL => {}
            opcode::CONSOLE_MIRROR if terminal && body.is_empty() => {}
            opcode::CONSOLE_INPUT if terminal && !body.is_empty() && body.len() <= CHUNK_BYTES => {}
            opcode::CONSOLE_STATE | opcode::CONSOLE_READ_INPUT if body.is_empty() => {}
            opcode::CONSOLE_STATE if command.is_some_and(|c| terminal || !c.terminal_only()) => {}
            opcode::CONSOLE_WRITE | opcode::CONSOLE_READ | opcode::CONSOLE_STATE => {
                return Response::error(Code::InvalidRequest);
            }
            _ => return Response::error(Code::Unsupported),
        }
        // The first authorized session owns these consoles until all its
        // clients disconnect. Future window/input authorities replace this
        // bounded policy.
        if self.owner.active.is_none() {
            // Discard any queued bytes from the prior terminal session before
            // assigning input ownership. A busy input source cannot bypass the bound.
            let mut drained = false;
            for _ in 0..256 {
                match door_sys::console_read(self.authority) {
                    Ok(_) => (),
                    Err(Status::WouldBlock) => {
                        drained = true;
                        break;
                    }
                    Err(error) => return Response::error(server::status(error)),
                }
            }
            if !drained {
                return Response::error(Code::WouldBlock);
            }
        }
        if let Err(error) = self.owner.attach(channel, peer) {
            return Response::error(error);
        }
        let index = self.console_of(channel);
        match operation {
            opcode::CONSOLE_WRITE => match door_sys::console_write(self.authority, body) {
                Ok(count) => {
                    let written = &body[..count.min(body.len())];
                    let screen = &mut self.screens[index];
                    screen.mirror.push(written);
                    screen.tracker.feed(written);
                    screen.answer_reports();
                    Response::scalar(count as u64)
                }
                Err(error) => Response::error(server::status(error)),
            },
            opcode::CONSOLE_MIRROR => {
                let mut out = [0; CHUNK_BYTES];
                let count = self.screens[index].mirror.take(&mut out);
                Response::bytes(&out[..count])
            }
            opcode::CONSOLE_INPUT => {
                let screen = &mut self.screens[index];
                if !screen.open {
                    return Response::error(Code::NotFound);
                }
                let payload = body;
                if screen.input_mode & PROCESSED_INPUT != 0 && control_c(payload) {
                    // A signal for the console's programs, not input.
                    screen.signals = screen.signals.wrapping_add(1);
                    return Response::scalar(payload.len() as u64);
                }
                Response::scalar(if screen.queue(payload) {
                    payload.len() as u64
                } else {
                    0
                })
            }
            opcode::CONSOLE_STATE => match command {
                Some(console::Command::Open { columns, rows, row }) => {
                    let Some(free) = self.screens.open() else {
                        return Response::error(Code::Resources);
                    };
                    let screen = &mut self.screens[free];
                    screen.tracker.resize(columns as usize, rows as usize);
                    screen.tracker.reset();
                    screen.tracker.place(0, row as usize);
                    screen.open = true;
                    screen.state(free)
                }
                Some(console::Command::Resize {
                    console,
                    columns,
                    rows,
                }) => {
                    let Some(screen) = self.screens.get(usize::from(console)) else {
                        return Response::error(Code::NotFound);
                    };
                    screen.tracker.resize(columns as usize, rows as usize);
                    screen.state(usize::from(console))
                }
                Some(console::Command::Close(console)) => {
                    let console = usize::from(console);
                    if self.screens.get(console).is_none() {
                        return Response::error(Code::NotFound);
                    }
                    self.screens.close(console);
                    // Programs still attached write to the session's console.
                    for slot in &mut self.attached {
                        if usize::from(slot.1) == console {
                            slot.1 = 0;
                        }
                    }
                    self.screens[0].state(0)
                }
                Some(console::Command::Query(console)) => {
                    match self.screens.get(usize::from(console)) {
                        Some(screen) => screen.state(usize::from(console)),
                        None => Response::error(Code::NotFound),
                    }
                }
                Some(console::Command::Attach(console)) => match self.attach(channel, console) {
                    Ok(()) => self.screens[usize::from(console)].state(usize::from(console)),
                    Err(error) => Response::error(error),
                },
                Some(console::Command::InputMode(mode)) => {
                    self.screens[index].input_mode = mode;
                    self.screens[index].state(index)
                }
                Some(console::Command::FlushInput) => {
                    let screen = &mut self.screens[index];
                    screen.input.fill(0);
                    screen.queued = 0;
                    screen.state(index)
                }
                Some(console::Command::Seen(signals)) => {
                    let screen = &mut self.screens[index];
                    screen.seen = signals.min(screen.signals);
                    screen.state(index)
                }
                None => self.screens[index].state(index),
            },
            opcode::CONSOLE_READ_INPUT => {
                let screen = &mut self.screens[index];
                if screen.queued == 0 {
                    if index != 0 {
                        return Response::error(Code::WouldBlock);
                    }
                    return match door_sys::console_read(self.authority) {
                        Ok(byte) => Response::bytes(&[byte]),
                        Err(error) => Response::error(server::status(error)),
                    };
                }
                let mut out = [0; CHUNK_BYTES];
                let n = screen.take(&mut out);
                Response::bytes(&out[..n])
            }
            opcode::CONSOLE_INTERRUPT => {
                let screen = &mut self.screens[0];
                match interrupt(&mut screen.input, &mut screen.queued, || {
                    door_sys::console_read(self.authority)
                }) {
                    Ok((cancel, overflow)) => {
                        if overflow {
                            let _ = door_sys::console_write(
                                self.authority,
                                b"\nTerminal typeahead full; excess input discarded.\n",
                            );
                        }
                        Response::scalar(u64::from(cancel))
                    }
                    Err(error) => Response::error(server::status(error)),
                }
            }
            opcode::CONSOLE_READ => {
                let screen = &mut self.screens[index];
                let byte = if screen.queued != 0 {
                    let mut one = [0u8; 1];
                    screen.take(&mut one);
                    Ok(one[0])
                } else if index == 0 {
                    door_sys::console_read(self.authority)
                } else {
                    Err(Status::WouldBlock)
                };
                match byte {
                    Ok(byte) => Response::scalar(byte as u64),
                    Err(error) => Response::error(server::status(error)),
                }
            }
            opcode::CONSOLE_CONTROL => {
                let command = u32::from_le_bytes(body[..4].try_into().unwrap());
                let value = u32::from_le_bytes(body[4..].try_into().unwrap());
                match door_sys::console_control(
                    self.authority,
                    u64::from(command),
                    u64::from(value),
                ) {
                    Ok(()) => Response::empty(),
                    Err(error) => Response::error(server::status(error)),
                }
            }
            _ => Response::error(Code::Unsupported),
        }
    }
    fn disconnected(&mut self, channel: Handle) {
        self.owner.detach(channel);
        for slot in &mut self.attached {
            if slot.0 == channel {
                *slot = (0, 0);
            }
        }
        if self.owner.active.is_none() {
            self.screens.close_all();
        }
    }
}
pub fn run(startup: StartupV2) -> i64 {
    if startup.mode == mode::FAILURE {
        loop {
            if door_sys::yield_now().is_err() {
                return -142;
            }
        }
    }
    let server = match Server::new(&startup) {
        Ok(server) => server,
        Err(_) => return -140,
    };
    // The pseudoconsoles' screens (the manifest's heap).
    #[cfg(not(test))]
    if HEAP
        .initialize_with_bytes(door_abi::platform::CONSOLE_HEAP_BYTES)
        .is_err()
    {
        return -143;
    }
    if server.ready(manifest::CONSOLE).is_err() {
        return -141;
    }
    server::serve(
        server,
        Console::new(startup.base.inherited[inherited::AUTHORITY].handle),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state(reply: &Response) -> console::State {
        console::State::decode(&reply.body[..reply.len]).unwrap()
    }
    #[test]
    fn buffered_input_preserves_text_cancels_when_full_and_is_zeroed_on_final_detach() {
        let mut input = [0; INPUT_BYTES];
        let mut queued = 0;
        let mut keys = b"text".iter();
        assert_eq!(
            interrupt(&mut input, &mut queued, || keys
                .next()
                .copied()
                .ok_or(Status::WouldBlock)),
            Ok((false, false))
        );
        assert_eq!(&input[..queued], b"text");
        input.fill(b'x');
        queued = INPUT_BYTES;
        let mut keys = [b'y', 3].into_iter();
        assert_eq!(
            interrupt(&mut input, &mut queued, || keys
                .next()
                .ok_or(Status::WouldBlock)),
            Ok((true, true))
        );
        assert_eq!(queued, INPUT_BYTES);
        assert!(input.iter().all(|b| *b == b'x'));
        let mut console = Console::new(0);
        console.screens[0].input = input;
        console.screens[0].queued = queued;
        console.owner.attach(1, peer(1)).unwrap();
        console.owner.attach(2, peer(1)).unwrap();
        console.disconnected(1);
        assert_eq!(console.screens[0].queued, INPUT_BYTES);
        console.disconnected(2);
        assert_eq!(console.screens[0].queued, 0);
        assert!(console.screens[0].input.iter().all(|b| *b == 0));
        console.owner.attach(3, peer(2)).unwrap();
        assert_eq!(console.screens[0].queued, 0);
    }
    fn peer(session: u64) -> Identity {
        Identity {
            pid: session,
            uid: 1001,
            integrity: 1,
            session,
            epoch: session,
            manifest: 5,
            instance: session,
            ..Identity::default()
        }
    }
    fn terminal(session: u64) -> Identity {
        Identity {
            manifest: door_abi::platform::manifest::WORKSPACE,
            ..peer(session)
        }
    }
    #[test]
    fn terminal_ownership_survives_one_client_close_and_never_crosses_sessions() {
        let mut owner = Ownership::new();
        assert_eq!(owner.attach(1, peer(1)), Ok(()));
        assert_eq!(owner.attach(2, peer(1)), Ok(()));
        assert_eq!(owner.attach(3, peer(2)), Err(Code::Denied));
        owner.detach(1);
        assert_eq!(owner.attach(3, peer(2)), Err(Code::Denied));
        owner.detach(2);
        assert_eq!(owner.active, None);
        assert_eq!(owner.attach(3, peer(2)), Ok(()));
        assert_eq!(owner.attach(4, peer(1)), Err(Code::Denied));
    }
    #[test]
    fn terminal_capacity_reuse_and_authority_are_bounded() {
        let mut owner = Ownership::new();
        for channel in 1..=server::CONNECTIONS as u64 {
            owner.attach(channel, peer(1)).unwrap();
        }
        assert_eq!(owner.attach(99, peer(1)), Err(Code::Resources));
        owner.detach(3);
        owner.attach(99, peer(1)).unwrap();
        let mut service = peer(1);
        service.capabilities = 1;
        assert_eq!(owner.attach(1, service), Err(Code::Denied));
        assert_eq!(owner.attach(1, peer(0)), Err(Code::Denied));
    }
    #[test]
    fn terminal_mirror_keeps_order_drops_oldest_and_is_terminal_only() {
        let mut mirror = Mirror::new();
        mirror.push(b"F1_START\r\n");
        let mut out = [0; 4];
        assert_eq!(mirror.take(&mut out), 4);
        assert_eq!(&out, b"F1_S");
        let mut rest = [0; 16];
        assert_eq!(mirror.take(&mut rest), 6);
        assert_eq!(&rest[..6], b"TART\r\n");
        assert_eq!(mirror.take(&mut rest), 0);
        let big: [u8; 9000] = core::array::from_fn(|i| (i % 251) as u8);
        mirror.push(&big);
        let mut first = [0; 1];
        mirror.take(&mut first);
        assert_eq!(first[0], big[9000 - 8192]);
        let mut console = Console::new(0);
        // Only Workspace (Terminal) peers drain the mirror or type input.
        for (operation, body) in [
            (opcode::CONSOLE_MIRROR, &[][..]),
            (opcode::CONSOLE_INPUT, &b"x"[..]),
        ] {
            assert_eq!(
                console.request(1, peer(1), operation, body, &[]).code,
                Code::Unsupported
            );
        }
        // Nor open, size, close or read another console.
        for command in [
            console::Command::Open {
                columns: 80,
                rows: 25,
                row: 0,
            },
            console::Command::Resize {
                console: 1,
                columns: 80,
                rows: 25,
            },
            console::Command::Close(1),
            console::Command::Query(1),
        ] {
            assert_eq!(
                console
                    .request(1, peer(1), opcode::CONSOLE_STATE, &command.encode(), &[])
                    .code,
                Code::InvalidRequest
            );
        }
        assert_eq!(console.owner.active, None);
    }
    #[test]
    fn typed_keys_reports_and_the_screen_state() {
        let mut console = Console::new(0);
        // (Host tests have no kernel console to drain before ownership.)
        console.owner.attach(1, terminal(1)).unwrap();
        let open = console::Command::Open {
            columns: 40,
            rows: 10,
            row: 2,
        }
        .encode();
        let s = state(&console.request(1, terminal(1), opcode::CONSOLE_STATE, &open, &[]));
        assert_eq!((s.console, s.columns, s.rows, s.x, s.y), (1, 40, 10, 0, 2));
        assert_eq!(s.input_mode, console::INPUT_MODE);
        // The Terminal's channel and the program's use console 1.
        let attach = console::Command::Attach(1).encode();
        for (channel, who) in [(1, terminal(1)), (2, peer(1))] {
            let s = state(&console.request(channel, who, opcode::CONSOLE_STATE, &attach, &[]));
            assert_eq!(s.console, 1);
        }
        // Output moves the tracked cursor; a cursor report becomes input.
        // (Host tests have no kernel console: the program's output is fed
        // to the screen as CONSOLE_WRITE does.)
        console.screens[1].tracker.feed(b"PS C:\\> \x1b[6n");
        console.screens[1].answer_reports();
        let typed = b"\x1b[65;30;97;1;0;1_";
        let reply = console.request(1, terminal(1), opcode::CONSOLE_INPUT, typed, &[]);
        assert_eq!(reply.body[..8], (typed.len() as u64).to_le_bytes());
        let expected = [&b"\x1b[3;9R"[..], &typed[..]].concat();
        let reply = console.request(2, peer(1), opcode::CONSOLE_READ_INPUT, &[], &[]);
        assert_eq!(&reply.body[..reply.len], &expected[..]);
        // The session's console saw none of it.
        assert_eq!(console.screens[0].queued, 0);
        let mode = console::Command::InputMode(0x200).encode();
        let s = state(&console.request(2, peer(1), opcode::CONSOLE_STATE, &mode, &[]));
        assert_eq!((s.x, s.y, s.input_mode), (8, 2, 0x200));
        assert_eq!(console.screens[0].input_mode, console::INPUT_MODE);
        // Input that does not fit is refused whole.
        let big = [b'x'; CHUNK_BYTES];
        while console.screens[1].queued + big.len() <= INPUT_BYTES {
            assert!(console.screens[1].queue(&big));
        }
        let reply = console.request(1, terminal(1), opcode::CONSOLE_INPUT, &big, &[]);
        assert_eq!(reply.body[..8], 0u64.to_le_bytes());
        let flush = console::Command::FlushInput.encode();
        console.request(2, peer(1), opcode::CONSOLE_STATE, &flush, &[]);
        assert_eq!(console.screens[1].queued, 0);
        console.disconnected(1);
        console.disconnected(2);
        // The last client gone: its pseudoconsoles are released.
        assert!(console.screens.get(1).is_none());
        assert_eq!(console.screens[0].input_mode, console::INPUT_MODE);
    }
    #[test]
    fn each_tab_has_its_own_console_and_ctrl_c_is_a_signal() {
        let mut console = Console::new(0);
        console.owner.attach(1, terminal(1)).unwrap();
        let open = console::Command::Open {
            columns: 80,
            rows: 25,
            row: 0,
        }
        .encode();
        // Terminal channels 1 and 2, programs 3 and 4.
        let a = state(&console.request(1, terminal(1), opcode::CONSOLE_STATE, &open, &[])).console;
        let b = state(&console.request(2, terminal(1), opcode::CONSOLE_STATE, &open, &[])).console;
        assert_eq!((a, b), (1, 2));
        for (channel, who, n) in [
            (1, terminal(1), a),
            (2, terminal(1), b),
            (3, peer(1), a),
            (4, peer(1), b),
        ] {
            let attach = console::Command::Attach(n as u8).encode();
            let reply = console.request(channel, who, opcode::CONSOLE_STATE, &attach, &[]);
            assert_eq!(state(&reply).console, n);
        }
        console.screens[1].mirror.push(b"tab one");
        console.screens[2].mirror.push(b"tab two");
        for (channel, text) in [(1, &b"tab one"[..]), (2, &b"tab two"[..])] {
            let reply = console.request(channel, terminal(1), opcode::CONSOLE_MIRROR, &[], &[]);
            assert_eq!(&reply.body[..reply.len], text);
        }
        console.request(2, terminal(1), opcode::CONSOLE_INPUT, b"two", &[]);
        let reply = console.request(3, peer(1), opcode::CONSOLE_READ_INPUT, &[], &[]);
        assert_eq!(reply.code, Code::WouldBlock);
        let reply = console.request(4, peer(1), opcode::CONSOLE_READ_INPUT, &[], &[]);
        assert_eq!(&reply.body[..reply.len], b"two");
        // Ctrl+C with processed input: a signal, not input.
        let ctrl_c = b"\x1b[67;46;3;1;8;1_";
        let reply = console.request(1, terminal(1), opcode::CONSOLE_INPUT, ctrl_c, &[]);
        assert_eq!(reply.body[..8], (ctrl_c.len() as u64).to_le_bytes());
        let query = console::Command::Query(a as u8).encode();
        let s = state(&console.request(1, terminal(1), opcode::CONSOLE_STATE, &query, &[]));
        assert_eq!((s.signals, s.seen), (1, 0));
        assert_eq!(console.screens[1].queued, 0);
        let seen = console::Command::Seen(1).encode();
        let s = state(&console.request(3, peer(1), opcode::CONSOLE_STATE, &seen, &[]));
        assert_eq!((s.signals, s.seen), (1, 1));
        // Without processed input Ctrl+C is the program's key.
        let raw = console::Command::InputMode(0).encode();
        console.request(3, peer(1), opcode::CONSOLE_STATE, &raw, &[]);
        console.request(1, terminal(1), opcode::CONSOLE_INPUT, ctrl_c, &[]);
        assert_eq!(console.screens[1].queued, ctrl_c.len());
        // Closing a console sends its programs back to the session's.
        let close = console::Command::Close(a as u8).encode();
        let s = state(&console.request(1, terminal(1), opcode::CONSOLE_STATE, &close, &[]));
        assert_eq!(s.console, 0);
        assert_eq!(console.console_of(3), 0);
        assert_eq!(console.console_of(4), 2);
        assert!(console.screens.get(1).is_none());
        let attach = console::Command::Attach(a as u8).encode();
        assert_eq!(
            console
                .request(3, peer(1), opcode::CONSOLE_STATE, &attach, &[])
                .code,
            Code::NotFound
        );
    }
    #[test]
    fn malformed_or_unknown_terminal_requests_do_not_acquire_input_ownership() {
        let mut console = Console::new(0);
        for (operation, body, expected) in [
            (opcode::CONSOLE_WRITE, &[0xff][..], Code::InvalidRequest),
            (opcode::CONSOLE_READ, &[1][..], Code::InvalidRequest),
            (opcode::CONSOLE_STATE, &[1, 2, 3][..], Code::InvalidRequest),
            (u32::MAX, &[][..], Code::Unsupported),
        ] {
            assert_eq!(
                console.request(1, peer(1), operation, body, &[]).code,
                expected
            );
            assert_eq!(console.owner.active, None);
            assert_eq!(console.owner.channels, [0; server::CONNECTIONS]);
        }
    }
}

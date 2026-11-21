//! Console input and modes as the Windows console host gives them to a
//! program. The Terminal sends typed keys as win32-input-mode sequences
//! (`door_vt::input`); they are decoded here into key events, which the
//! program reads as INPUT_RECORDs (ReadConsoleInput), as an edited,
//! echoed line (ReadFile/ReadConsole with ENABLE_LINE_INPUT), or as
//! characters and, with ENABLE_VIRTUAL_TERMINAL_INPUT, VT sequences.
//! The input mode is the console's (the console service keeps it for
//! every program sharing the console); the output mode is this
//! process's, applied to the shared screen as the VT modes it implies.
//!
//! One reader at a time owns the decoded input (`INPUT`); it takes the
//! console connection only for each short request, never while it waits,
//! so other threads keep writing while one waits for a key.
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use door_service_protocol::{CHUNK_BYTES, Code, console as wire};
use door_vt::input::{self, Decoded, KeyEvent};
use door_windows_abi as abi;

pub const PROCESSED_INPUT: u32 = 0x1;
pub const LINE_INPUT: u32 = 0x2;
pub const ECHO_INPUT: u32 = 0x4;
pub const VIRTUAL_TERMINAL_INPUT: u32 = 0x200;
pub const WRAP_AT_EOL_OUTPUT: u32 = 0x2;
pub const VIRTUAL_TERMINAL_PROCESSING: u32 = 0x4;
pub const DISABLE_NEWLINE_AUTO_RETURN: u32 = 0x8;
/// CTRL_C_EVENT.
const CTRL_C: u32 = 0;
/// STATUS_CONTROL_C_EXIT, the exit code of a process Ctrl+C ended.
const CONTROL_C_EXIT: u32 = 0xc000_013a;
const ERROR_OPERATION_ABORTED: u32 = 995;

const RAW_BYTES: usize = 256;
const KEYS: usize = 128;
struct Input {
    /// Bytes from the console not yet decoded.
    raw: [u8; RAW_BYTES],
    raw_len: usize,
    /// Key-down events not yet read.
    keys: [KeyEvent; KEYS],
    key_len: usize,
    /// Ctrl+C arrived while input was processed.
    interrupted: bool,
    /// The last read filled a whole chunk: the console has more queued,
    /// so a key sequence cut at the end of `raw` waits for its rest.
    cut: bool,
    /// The line being edited (ENABLE_LINE_INPUT) and, once Enter ends
    /// it, the line not yet read.
    line: abi::console::Line,
    /// Bytes of a key's character or VT sequence not yet read.
    pending: [u8; 32],
    pending_start: usize,
    pending_len: usize,
}
struct Shared {
    held: AtomicBool,
    value: UnsafeCell<Input>,
}
// `held` gives one thread the cell at a time.
unsafe impl Sync for Shared {}
static INPUT: Shared = Shared {
    held: AtomicBool::new(false),
    value: UnsafeCell::new(Input {
        raw: [0; RAW_BYTES],
        raw_len: 0,
        keys: [KeyEvent {
            down: false,
            repeat: 0,
            vk: 0,
            scan: 0,
            ch: 0,
            control: 0,
        }; KEYS],
        key_len: 0,
        interrupted: false,
        cut: false,
        line: abi::console::Line::new(),
        pending: [0; 32],
        pending_start: 0,
        pending_len: 0,
    }),
};
/// This process's output mode: ENABLE_PROCESSED_OUTPUT and
/// ENABLE_WRAP_AT_EOL_OUTPUT, as a console program starts.
static OUTPUT_MODE: AtomicU32 = AtomicU32::new(0x3);
/// kernel32's handler routine for console control events.
static DISPATCHER: AtomicU64 = AtomicU64::new(0);

/// Pause a tick between polls of the console.
fn pause() -> Result<(), u32> {
    let now = door_sys::time().map_err(|_| 1117u32)?;
    door_sys::sleep(now.saturating_add(1)).map_err(|_| 1117u32)
}
/// Run `f` as the only reader of the console's input.
fn with_input<R>(f: impl FnOnce(&mut Input) -> Result<R, u32>) -> Result<R, u32> {
    while INPUT
        .held
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        pause()?;
    }
    // SAFETY: `held` gives this closure the only access until released.
    let result = f(unsafe { &mut *INPUT.value.get() });
    INPUT.held.store(false, Ordering::Release);
    result
}

/// The console's state from the console service.
pub(crate) fn state() -> Result<wire::State, u32> {
    super::guest::with_console(|console| console.state().map_err(|_| 1117u32))
}
fn input_mode() -> Result<u32, u32> {
    state().map(|s| s.input_mode)
}

/// Move what the console has queued into decoded key events.
fn pump(input: &mut Input, mode: u32) -> Result<(), u32> {
    let room = RAW_BYTES - input.raw_len;
    if room >= CHUNK_BYTES && input.key_len < KEYS {
        let mut chunk = [0u8; CHUNK_BYTES];
        let got = super::guest::with_console(|console| match console.read_input(&mut chunk) {
            Ok(n) => Ok(n),
            Err(door_service_client::Error::Service(Code::WouldBlock)) => Ok(0),
            Err(_) => Err(1117u32),
        })?;
        input.raw[input.raw_len..input.raw_len + got].copy_from_slice(&chunk[..got]);
        input.raw_len += got;
        input.cut = got == CHUNK_BYTES;
    }
    // A read that filled the chunk may have left part of a sequence queued
    // (the service hands out its queue in chunks, cutting anywhere).
    let more = input.cut || input.raw_len == RAW_BYTES;
    let mut at = 0;
    while at < input.raw_len && input.key_len + 2 <= KEYS {
        match input::decode(&input.raw[at..input.raw_len], more) {
            Decoded::Keys(keys, n) => {
                at += n;
                for e in &keys.events[..keys.count] {
                    if !e.down {
                        continue;
                    }
                    if e.ch == 3 && mode & PROCESSED_INPUT != 0 {
                        input.interrupted = true;
                        continue;
                    }
                    input.keys[input.key_len] = *e;
                    input.key_len += 1;
                }
            }
            Decoded::Skip(n) => at += n,
            Decoded::Need => break,
        }
    }
    input.raw.copy_within(at..input.raw_len, 0);
    input.raw_len -= at;
    Ok(())
}
fn take_key(input: &mut Input) -> Option<KeyEvent> {
    if input.key_len == 0 {
        return None;
    }
    let e = input.keys[0];
    input.keys.copy_within(1..input.key_len, 0);
    input.key_len -= 1;
    Some(e)
}
fn echo(text: &[u8]) {
    let _ = super::guest::with_console(|console| {
        let text = core::str::from_utf8(text).unwrap_or("");
        console.write(text).map(|_| ()).map_err(|_| 1117u32)
    });
}

/// Ctrl+C with processed input: the program's handlers (SetConsoleCtrlHandler)
/// run; when none handles it, the process ends as Windows ends it.
fn control_c() -> Result<(), u32> {
    let routine = DISPATCHER.load(Ordering::Acquire);
    let handled = routine != 0 && {
        // SAFETY: kernel32 registered its handler routine (DoorSetControlDispatcher).
        let routine: extern "system" fn(u32) -> i32 =
            unsafe { core::mem::transmute(routine as usize) };
        routine(CTRL_C) != 0
    };
    if !handled {
        super::guest::RtlExitUserProcess(CONTROL_C_EXIT);
    }
    Err(ERROR_OPERATION_ABORTED)
}

/// A key's bytes for a program reading the console as a stream: VT input
/// (ENABLE_VIRTUAL_TERMINAL_INPUT) or the character typed, as UTF-8.
fn key_bytes(e: &KeyEvent, mode: u32, application_cursor: bool, out: &mut [u8; 16]) -> usize {
    if mode & VIRTUAL_TERMINAL_INPUT != 0 {
        return input::to_vt(e, application_cursor, out);
    }
    match char::from_u32(u32::from(e.ch)) {
        Some(c) if e.ch != 0 => c.encode_utf8(out).len(),
        _ => 0,
    }
}

/// Edit the line with the keys typed so far (echoing them with
/// ENABLE_ECHO_INPUT): true once Enter ended it.
fn edit_line(input: &mut Input, mode: u32) -> Result<bool, u32> {
    let echoing = mode & ECHO_INPUT != 0;
    while let Some(e) = take_key(input) {
        match (e.vk, e.ch) {
            (input::vk::RETURN, _) => {
                input.line.push(b'\r')?;
                input.line.push(b'\n')?;
                if echoing {
                    echo(b"\r\n");
                }
                return Ok(true);
            }
            (input::vk::BACK, _) => {
                // A whole UTF-8 character goes.
                let mut removed = false;
                while input.line.length > 0 {
                    let last = input.line.bytes[input.line.length - 1];
                    input.line.length -= 1;
                    removed = true;
                    if last & 0xc0 != 0x80 {
                        break;
                    }
                }
                if removed && echoing {
                    echo(b"\x08 \x08");
                }
            }
            (input::vk::ESCAPE, _) => {
                // Escape clears the line, as conhost's line editing does.
                let mut characters = 0;
                for &b in &input.line.bytes[..input.line.length] {
                    characters += usize::from(b & 0xc0 != 0x80);
                }
                input.line.length = 0;
                if echoing {
                    for _ in 0..characters {
                        echo(b"\x08 \x08");
                    }
                }
            }
            // Ctrl+Z at the start of a line is the end of the input: the
            // read returns nothing.
            (_, 0x1a) if input.line.length == 0 => {
                if echoing {
                    echo(b"^Z\r\n");
                }
                return Ok(true);
            }
            // Other controls are not text (Tab is).
            (_, ch) if ch < 0x20 && ch != 0x09 => {}
            (_, ch) => {
                let c = char::from_u32(u32::from(ch)).unwrap_or('\u{fffd}');
                let mut bytes = [0u8; 4];
                let text = c.encode_utf8(&mut bytes).as_bytes();
                if input.line.length + text.len() + 2 > input.line.bytes.len() {
                    continue;
                }
                for &b in text {
                    input.line.push(b)?;
                }
                if echoing {
                    echo(text);
                }
            }
        }
    }
    Ok(false)
}

/// ReadFile on the console input: a line (ENABLE_LINE_INPUT, with its CR
/// LF; what does not fit waits for the next read) or, without line input,
/// the bytes of the keys typed (at least one key's, waiting for it).
pub(crate) fn read(out: &mut [u8]) -> Result<usize, u32> {
    if out.is_empty() {
        return Ok(0);
    }
    loop {
        let state = state()?;
        let mode = state.input_mode;
        let application = state.flags & wire::APPLICATION_CURSOR != 0;
        let done = with_input(|input| {
            if input.line.ready {
                return Ok(Some(input.line.take(out)));
            }
            if input.pending_len > 0 {
                let n = input.pending_len.min(out.len());
                let start = input.pending_start;
                out[..n].copy_from_slice(&input.pending[start..start + n]);
                input.pending_start += n;
                input.pending_len -= n;
                return Ok(Some(n));
            }
            pump(input, mode)?;
            if input.interrupted {
                input.interrupted = false;
                input.line.length = 0;
                return control_c().map(|()| None);
            }
            if mode & LINE_INPUT != 0 {
                if edit_line(input, mode)? {
                    input.line.ready = true;
                    return Ok(Some(input.line.take(out)));
                }
                return Ok(None);
            }
            let mut n = 0;
            while let Some(e) = input.keys.first().copied().filter(|_| input.key_len > 0) {
                let mut bytes = [0u8; 16];
                let len = key_bytes(&e, mode, application, &mut bytes);
                if n + len > out.len() {
                    if n == 0 {
                        // The first key's bytes do not fit: the rest waits.
                        take_key(input);
                        out.copy_from_slice(&bytes[..out.len()]);
                        let rest = len - out.len();
                        input.pending[..rest].copy_from_slice(&bytes[out.len()..len]);
                        input.pending_start = 0;
                        input.pending_len = rest;
                        return Ok(Some(out.len()));
                    }
                    break;
                }
                take_key(input);
                out[n..n + len].copy_from_slice(&bytes[..len]);
                n += len;
            }
            Ok((n > 0).then_some(n))
        })?;
        if let Some(n) = done {
            return Ok(n);
        }
        pause()?;
    }
}

/// INPUT_RECORD (20 bytes) for a key event.
fn record(e: &KeyEvent) -> [u8; 20] {
    let mut r = [0u8; 20];
    r[..2].copy_from_slice(&1u16.to_le_bytes());
    r[4..8].copy_from_slice(&i32::from(e.down).to_le_bytes());
    r[8..10].copy_from_slice(&e.repeat.max(1).to_le_bytes());
    r[10..12].copy_from_slice(&e.vk.to_le_bytes());
    r[12..14].copy_from_slice(&e.scan.to_le_bytes());
    r[14..16].copy_from_slice(&e.ch.to_le_bytes());
    r[16..20].copy_from_slice(&e.control.to_le_bytes());
    r
}
/// VT input as key events: each byte of a key's sequence is a character
/// event (a key of one byte keeps its virtual key and modifiers); a
/// sequence the caller's buffer cuts waits in `pending`.
fn vt_events(
    input: &mut Input,
    mode: u32,
    application: bool,
    out: &mut [[u8; 20]],
    remove: bool,
) -> usize {
    let mut n = 0;
    let mut pending = input.pending;
    let (mut start, mut len) = (input.pending_start, input.pending_len);
    let mut next = 0;
    while n < out.len() {
        if len == 0 {
            if next == input.key_len {
                break;
            }
            let e = input.keys[next];
            next += 1;
            let mut bytes = [0u8; 16];
            let l = key_bytes(&e, mode, application, &mut bytes);
            if l == 1 {
                out[n] = record(&KeyEvent {
                    ch: u16::from(bytes[0]),
                    ..e
                });
                n += 1;
            } else {
                pending[..l].copy_from_slice(&bytes[..l]);
                (start, len) = (0, l);
            }
            continue;
        }
        out[n] = record(&KeyEvent {
            down: true,
            repeat: 1,
            ch: u16::from(pending[start]),
            ..Default::default()
        });
        n += 1;
        start += 1;
        len -= 1;
    }
    if remove {
        for _ in 0..next {
            take_key(input);
        }
        input.pending = pending;
        input.pending_start = start;
        input.pending_len = len;
    }
    n
}
/// ReadConsoleInput (`remove`, waiting for a key) and PeekConsoleInput:
/// up to `out.len()` key events. With ENABLE_VIRTUAL_TERMINAL_INPUT each
/// byte of a key's VT sequence is its own character event, as conhost
/// delivers VT input.
pub(crate) fn events(out: &mut [[u8; 20]], remove: bool, wait: bool) -> Result<usize, u32> {
    loop {
        let state = state()?;
        let mode = state.input_mode;
        let application = state.flags & wire::APPLICATION_CURSOR != 0;
        let n = with_input(|input| {
            pump(input, mode)?;
            if input.interrupted {
                input.interrupted = false;
                control_c()?;
            }
            if mode & VIRTUAL_TERMINAL_INPUT != 0 {
                return Ok(vt_events(input, mode, application, out, remove));
            }
            let mut n = 0;
            while n < out.len() && n < input.key_len {
                out[n] = record(&input.keys[n]);
                n += 1;
            }
            if remove {
                for _ in 0..n {
                    take_key(input);
                }
            }
            Ok(n)
        })?;
        if n > 0 || !wait {
            return Ok(n);
        }
        pause()?;
    }
}
/// GetNumberOfConsoleInputEvents.
pub(crate) fn pending_events() -> Result<usize, u32> {
    let mode = input_mode()?;
    with_input(|input| {
        pump(input, mode)?;
        Ok(input.key_len + input.pending_len)
    })
}
/// FlushConsoleInputBuffer: typed input not yet read goes, here and in
/// the console.
pub(crate) fn flush() -> Result<(), u32> {
    with_input(|input| {
        input.raw_len = 0;
        input.key_len = 0;
        input.pending_len = 0;
        input.interrupted = false;
        Ok(())
    })?;
    super::guest::with_console(|console| {
        console
            .command(wire::Command::FlushInput)
            .map(|_| ())
            .map_err(|_| 1117u32)
    })
}

/// GetConsoleMode/SetConsoleMode on the input handle (the console's).
pub(crate) fn set_input_mode(mode: u32) -> Result<(), u32> {
    super::guest::with_console(|console| {
        console
            .command(wire::Command::InputMode(mode))
            .map(|_| ())
            .map_err(|_| 1117u32)
    })
}
pub(crate) fn get_input_mode() -> Result<u32, u32> {
    input_mode()
}
pub(crate) fn output_mode() -> u32 {
    OUTPUT_MODE.load(Ordering::Acquire)
}
/// SetConsoleMode on an output handle: a line feed returns to the first
/// column unless VT processing is on with DISABLE_NEWLINE_AUTO_RETURN,
/// and lines wrap with ENABLE_WRAP_AT_EOL_OUTPUT.
pub(crate) fn set_output_mode(mode: u32) -> Result<(), u32> {
    let old = OUTPUT_MODE.swap(mode, Ordering::AcqRel);
    let returns = |m: u32| {
        m & (VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN)
            != VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN
    };
    let mut sequence = [0u8; 16];
    let mut n = 0;
    if returns(old) != returns(mode) {
        let s: &[u8] = if returns(mode) {
            b"\x1b[20h"
        } else {
            b"\x1b[20l"
        };
        sequence[n..n + s.len()].copy_from_slice(s);
        n += s.len();
    }
    if (old ^ mode) & WRAP_AT_EOL_OUTPUT != 0 {
        let s: &[u8] = if mode & WRAP_AT_EOL_OUTPUT != 0 {
            b"\x1b[?7h"
        } else {
            b"\x1b[?7l"
        };
        sequence[n..n + s.len()].copy_from_slice(s);
        n += s.len();
    }
    if n > 0 {
        echo(&sequence[..n]);
    }
    Ok(())
}

// ---- exports kernel32 builds the console API on ----------------------------------

fn fail(code: u32) -> i32 {
    super::guest::RtlSetLastWin32Error(code);
    0
}
/// The console's state: the first `LEGACY_STATE_BYTES` (20) of
/// `door_service_protocol::console::State`, which kernel32 reads.
#[unsafe(no_mangle)]
pub extern "system" fn DoorConsoleState(out: u64) -> i32 {
    match state() {
        Ok(s) => match super::guest::put(out, &s.encode()[..wire::LEGACY_STATE_BYTES]) {
            Ok(()) => 1,
            Err(_) => fail(998),
        },
        Err(code) => fail(code),
    }
}
/// `which`: 0 input, 1 output; `set`: change it to `mode`. Returns the
/// mode, or u32::MAX with the last error set.
#[unsafe(no_mangle)]
pub extern "system" fn DoorConsoleMode(which: u32, set: u32, mode: u32) -> u32 {
    let result = match (which, set != 0) {
        (0, false) => get_input_mode(),
        (0, true) => set_input_mode(mode).map(|()| mode),
        (1, false) => Ok(output_mode()),
        (1, true) => set_output_mode(mode).map(|()| mode),
        _ => Err(87),
    };
    result.unwrap_or_else(|code| {
        fail(code);
        u32::MAX
    })
}
/// Key events into `records` (INPUT_RECORDs): flags 1 removes them
/// (ReadConsoleInput), 2 waits for one; the count goes to `count`.
#[unsafe(no_mangle)]
pub extern "system" fn DoorConsoleReadKeys(
    records: u64,
    length: u32,
    count: u64,
    flags: u32,
) -> i32 {
    let mut out = [[0u8; 20]; 32];
    let wanted = (length as usize).min(out.len());
    match events(&mut out[..wanted], flags & 1 != 0, flags & 2 != 0) {
        Ok(n) => {
            for (i, r) in out[..n].iter().enumerate() {
                if super::guest::put(records + 20 * i as u64, r).is_err() {
                    return fail(998);
                }
            }
            match super::guest::put(count, &(n as u32).to_le_bytes()) {
                Ok(()) => 1,
                Err(_) => fail(998),
            }
        }
        Err(code) => fail(code),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorConsoleInputEvents(count: u64) -> i32 {
    match pending_events() {
        Ok(n) => match super::guest::put(count, &(n as u32).to_le_bytes()) {
            Ok(()) => 1,
            Err(_) => fail(998),
        },
        Err(code) => fail(code),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorConsoleFlushInput() -> i32 {
    match flush() {
        Ok(()) => 1,
        Err(code) => fail(code),
    }
}
/// WriteConsoleInput: key-down events (INPUT_RECORDs) join the input
/// after what was typed; other records are skipped.
#[unsafe(no_mangle)]
pub extern "system" fn DoorConsoleWriteKeys(records: u64, count: u32) -> i32 {
    let mut taken = [KeyEvent::default(); KEYS];
    let mut n = 0;
    for index in 0..count as u64 {
        let mut r = [0u8; 20];
        if super::guest::copy(r.as_mut_ptr() as u64, records + 20 * index, 20).is_err() {
            return fail(998);
        }
        let half = |at: usize| u16::from_le_bytes([r[at], r[at + 1]]);
        let down = u32::from_le_bytes(r[4..8].try_into().unwrap()) != 0;
        if half(0) != 1 || !down || n == KEYS {
            continue;
        }
        taken[n] = KeyEvent {
            down,
            repeat: half(8),
            vk: half(10),
            scan: half(12),
            ch: half(14),
            control: u32::from_le_bytes(r[16..20].try_into().unwrap()),
        };
        n += 1;
    }
    match with_input(|input| {
        let room = (KEYS - input.key_len).min(n);
        input.keys[input.key_len..input.key_len + room].copy_from_slice(&taken[..room]);
        input.key_len += room;
        Ok(())
    }) {
        Ok(()) => 1,
        Err(code) => fail(code),
    }
}
/// kernel32's routine that runs the program's console control handlers.
#[unsafe(no_mangle)]
pub extern "system" fn DoorSetControlDispatcher(routine: u64) -> i32 {
    DISPATCHER.store(routine, Ordering::Release);
    1
}

// ---- the Terminal tab's console and its Ctrl+C signals -----------------------

/// The Terminal tab console this process uses (0: the session's).
static ATTACHED: AtomicU32 = AtomicU32::new(0);
/// At startup: the console Init named (`DOOR_CONSOLE=<n>`, the Terminal
/// tab's that started this program or its parent), as a Windows program
/// starts attached to its parent's console.
pub(crate) fn attach(environment: &[u8]) {
    let Some(n) = environment.split(|b| *b == 0).find_map(|entry| {
        let value = entry.strip_prefix(wire::ENVIRONMENT.as_bytes())?;
        core::str::from_utf8(value).ok()?.parse::<u8>().ok()
    }) else {
        return;
    };
    if n == 0 || usize::from(n) >= wire::CONSOLES {
        return;
    }
    let attached = super::guest::with_console(|console| {
        console
            .command(wire::Command::Attach(n))
            .map_err(|_| 1117u32)
    });
    if attached.is_ok_and(|s| s.console == u16::from(n)) {
        ATTACHED.store(u32::from(n), Ordering::Release);
    }
}
/// Ctrl+C typed in a Terminal tab while its console's input is processed
/// is a signal for the console's programs (the console service counts
/// them), as Windows sends CTRL_C_EVENT to each process attached to the
/// console on a thread of its own: this process's handlers
/// (SetConsoleCtrlHandler: kernel32's dispatcher) run, newest first; none
/// handling it ends the process with STATUS_CONTROL_C_EXIT, as Windows'
/// default handler does. Each signal taken is reported (`Seen`), so the
/// Terminal ends a program itself only when no process took it.
static WATCHING: AtomicBool = AtomicBool::new(false);
extern "system" fn watch_signals(_argument: usize) -> u32 {
    let mut last = match state() {
        Ok(s) => s.signals,
        Err(_) => return 1,
    };
    loop {
        // Ten polls a second.
        super::threads::DoorSleep(100);
        let Ok(s) = state() else {
            continue;
        };
        while last != s.signals {
            last = last.wrapping_add(1);
            let routine = DISPATCHER.load(Ordering::Acquire);
            let handled = routine != 0 && {
                // SAFETY: kernel32 registered its handler routine.
                let routine: extern "system" fn(u32) -> i32 =
                    unsafe { core::mem::transmute(routine as usize) };
                routine(CTRL_C) != 0
            };
            let _ = super::guest::with_console(|console| {
                console
                    .command(wire::Command::Seen(last))
                    .map(|_| ())
                    .map_err(|_| 1117u32)
            });
            if !handled {
                super::guest::RtlExitUserProcess(CONTROL_C_EXIT);
            }
        }
    }
}
/// Before the program's entry: a process in a Terminal tab's console
/// watches for its Ctrl+C signals.
pub(crate) fn start_watching() {
    if ATTACHED.load(Ordering::Acquire) != 0 && !WATCHING.swap(true, Ordering::AcqRel) {
        let _ = super::threads::DoorCreateThread(watch_signals as *const () as usize as u64, 0);
    }
}

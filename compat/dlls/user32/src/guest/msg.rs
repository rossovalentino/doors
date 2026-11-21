//! Message queues: retrieval, dispatch, posting, synchronous and
//! cross-thread sends (nested, with timeouts and the hung rule), timers and
//! waits. Window procedures always run on the calling (owning) thread with
//! no lock held.
use super::{call, g, hwnd_or_error, server, structs, tid, with};
use alloc::vec::Vec;
use door_windows_gui::handles::Handle;
use door_windows_gui::keyboard;
use door_windows_gui::msg::{pm, wm};
use door_windows_gui::queue::{Filter, Message};
use door_windows_gui::win32_error as e;

/// Poll interval for the window service while a thread waits.
const POLL_MS: u64 = 8;
const HWND_TOP_FILTER: u64 = u64::MAX;

pub(crate) fn message(hwnd: u64, message: u32, wparam: u64, lparam: i64) -> Message {
    Message {
        hwnd,
        message,
        wparam,
        lparam,
        time: g::now() as u32,
        pt: with(|u| u.last_click.3),
    }
}

/// Run every message other threads sent to this thread.
pub(crate) fn run_incoming_sends(me: u32) {
    for _ in 0..1024 {
        let next = with(|u| {
            let (id, m) = u.queues.take_incoming(me)?;
            let target = u
                .windows
                .resolve(m.hwnd)
                .filter(|h| u.windows.get(*h).is_some_and(|w| w.thread == me))
                .map(|h| u.windows.get(h).unwrap().procedure);
            Some((id, m, target))
        });
        let Some((id, m, target)) = next else {
            return;
        };
        let result = match target {
            Some(p) => call(p, m.hwnd, m.message, m.wparam, m.lparam),
            None => 0,
        };
        with(|u| {
            if let Some(sender) = u.queues.finish(me, id, result) {
                u.wake(sender);
            }
        });
    }
}

/// Sends from other threads, then window-service events for this thread.
pub(crate) fn process_pending(me: u32) {
    run_incoming_sends(me);
    for _ in 0..256 {
        let Some(event) = with(|u| u.thread(me).events.pop_front()) else {
            break;
        };
        server::handle_event(event);
        run_incoming_sends(me);
    }
}

fn wake_handle(me: u32) -> u64 {
    with(|u| u.thread(me).wake)
}

/// Wait up to `millis` for this thread's wake event (or the poll interval),
/// then poll the window service. Counts as waiting for input.
pub(crate) fn idle_wait(me: u32, millis: u64) {
    let wake = wake_handle(me);
    with(|u| {
        u.queues.set_waiting(me, true, g::now());
        u.queues.note_input_wait(me);
    });
    g::wait_any(&[wake], millis.min(POLL_MS) as u32);
    with(|u| u.queues.set_waiting(me, false, g::now()));
    server::pump();
}

/// Retrieval with the hwnd and range filters of Get/PeekMessage.
pub(crate) fn retrieve(
    me: u32,
    hwnd: u64,
    min: u32,
    max: u32,
    remove: bool,
) -> Result<Option<Message>, u32> {
    with(|u| {
        u.thread(me);
        let target = match hwnd {
            0 | HWND_TOP_FILTER => None,
            v => Some(u.windows.resolve(v).ok_or(e::INVALID_WINDOW_HANDLE)?),
        };
        let windows = &u.windows;
        let accept = |m: &Message| -> bool {
            match (hwnd, target) {
                (0, _) => true,
                (HWND_TOP_FILTER, _) => m.hwnd == 0,
                (_, Some(t)) => windows
                    .resolve(m.hwnd)
                    .is_some_and(|h| h == t || windows.is_descendant(t, h)),
                _ => false,
            }
        };
        let filter = Filter {
            accept: &accept,
            min,
            max,
        };
        let mut paint = |f: &Filter| {
            windows
                .paint_candidate(me, &|h: Handle| {
                    (f.accept)(&Message {
                        hwnd: h.value(),
                        message: wm::PAINT,
                        ..Message::default()
                    })
                })
                .map(Handle::value)
        };
        Ok(u.queues.next(me, &filter, remove, g::now(), &mut paint))
    })
}

/// A MSG as the application lays it out (P17: 48 bytes on x64, 28 on x86).
pub(crate) fn msg_bytes(m: &Message) -> [u8; structs::MSG_BYTES] {
    let mut b = [0u8; structs::MSG_BYTES];
    for (index, value) in [
        m.hwnd,
        u64::from(m.message),
        m.wparam,
        m.lparam as u64,
        u64::from(m.time),
        u64::from(m.pt.x as u32),
        u64::from(m.pt.y as u32),
    ]
    .into_iter()
    .enumerate()
    {
        structs::set(&mut b, &structs::MSG, index, value);
    }
    b
}
fn write_msg(out: u64, m: &Message) -> bool {
    g::write(out, &msg_bytes(m))
}
pub(crate) fn read_msg(src: u64) -> Option<Message> {
    let mut b = [0u8; structs::MSG_BYTES];
    if !g::read(src, &mut b) {
        return None;
    }
    let field = |index| structs::get(&b, &structs::MSG, index);
    Some(Message {
        hwnd: field(structs::msg::HWND),
        message: field(structs::msg::MESSAGE) as u32,
        wparam: field(structs::msg::WPARAM),
        lparam: field(structs::msg::LPARAM) as i64,
        time: field(structs::msg::TIME) as u32,
        pt: door_windows_gui::Point::new(
            field(structs::msg::X) as u32 as i32,
            field(structs::msg::Y) as u32 as i32,
        ),
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn GetMessageW(out: usize, hwnd: usize, min: u32, max: u32) -> i32 {
    let out = door_windows_abi::widen(out);
    let hwnd = door_windows_abi::widen(hwnd);
    {
        let me = tid();
        // No queue slot for this thread: fail instead of waiting forever.
        if with(|u| {
            u.thread(me);
            u.queues.thread(me).is_none()
        }) {
            g::set_last_error(e::NOT_ENOUGH_QUOTA);
            return -1;
        }
        loop {
            server::pump();
            process_pending(me);
            match retrieve(me, hwnd, min, max, true) {
                Err(code) => {
                    g::set_last_error(code);
                    return -1;
                }
                Ok(Some(m)) => {
                    if !write_msg(out, &m) {
                        g::set_last_error(e::INVALID_PARAMETER);
                        return -1;
                    }
                    return i32::from(m.message != wm::QUIT);
                }
                Ok(None) => {
                    let due = with(|u| u.queues.next_due(me));
                    let now = g::now();
                    let wait = due.map_or(POLL_MS, |d| d.saturating_sub(now));
                    idle_wait(me, wait);
                }
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn PeekMessageW(
    out: usize,
    hwnd: usize,
    min: u32,
    max: u32,
    flags: u32,
) -> i32 {
    let out = door_windows_abi::widen(out);
    let hwnd = door_windows_abi::widen(hwnd);
    {
        let me = tid();
        server::pump();
        process_pending(me);
        match retrieve(me, hwnd, min, max, flags & pm::REMOVE != 0) {
            Err(code) => {
                g::set_last_error(code);
                0
            }
            Ok(Some(m)) => {
                if !write_msg(out, &m) {
                    g::set_last_error(e::INVALID_PARAMETER);
                    return 0;
                }
                1
            }
            Ok(None) => 0,
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn WaitMessage() -> i32 {
    let me = tid();
    loop {
        server::pump();
        process_pending(me);
        let (posted, input, _, quit) = with(|u| u.queues.pending(me));
        if posted || input || quit || with(|u| u.windows.paint_candidate(me, &|_| true).is_some()) {
            return 1;
        }
        let due = with(|u| u.queues.next_due(me));
        if due.is_some_and(|d| d <= g::now()) {
            return 1;
        }
        idle_wait(me, POLL_MS);
    }
}

/// Is there anything Get/PeekMessage would return now?
fn has_input(me: u32) -> bool {
    let (posted, input, sent, quit) = with(|u| u.queues.pending(me));
    posted
        || input
        || sent
        || quit
        || with(|u| {
            u.windows.paint_candidate(me, &|_| true).is_some()
                || u.queues.next_due(me).is_some_and(|d| d <= g::now())
                || !u.thread(me).events.is_empty()
        })
}

#[unsafe(no_mangle)]
pub extern "system" fn MsgWaitForMultipleObjects(
    count: u32,
    handles: usize,
    all: i32,
    millis: u32,
    mask: u32,
) -> u32 {
    let handles = door_windows_abi::widen(handles);
    MsgWaitForMultipleObjectsEx(
        count,
        door_windows_abi::narrow(handles),
        millis,
        mask,
        if all != 0 { 1 } else { 0 },
    )
}

#[unsafe(no_mangle)]
pub extern "system" fn MsgWaitForMultipleObjectsEx(
    count: u32,
    handles: usize,
    millis: u32,
    _mask: u32,
    flags: u32,
) -> u32 {
    let handles = door_windows_abi::widen(handles);
    {
        let me = tid();
        if count > 63 || flags & !0x7 != 0 {
            g::set_last_error(e::INVALID_PARAMETER);
            return g::WAIT_FAILED;
        }
        // MWMO_WAITALL with objects needs every object and input together.
        if flags & 1 != 0 && count != 0 {
            g::set_last_error(e::NOT_SUPPORTED);
            return g::WAIT_FAILED;
        }
        // The program's HANDLE array: a pointer-sized entry each (P17).
        let mut list = alloc::vec::Vec::new();
        for index in 0..count as u64 {
            match g::read_pointer(handles + index * g::POINTER as u64) {
                Some(handle) => list.push(handle),
                None => {
                    g::set_last_error(e::INVALID_PARAMETER);
                    return g::WAIT_FAILED;
                }
            }
        }
        list.push(wake_handle(me));
        let start = g::now();
        loop {
            server::pump();
            run_incoming_sends(me);
            if has_input(me) {
                return count;
            }
            let elapsed = g::now() - start;
            if millis != 0xffff_ffff && elapsed >= u64::from(millis) {
                return g::WAIT_TIMEOUT;
            }
            let remain = if millis == 0xffff_ffff {
                POLL_MS
            } else {
                (u64::from(millis) - elapsed).min(POLL_MS)
            };
            let due = with(|u| u.queues.next_due(me))
                .map_or(remain, |d| d.saturating_sub(g::now()).min(remain));
            with(|u| {
                u.queues.set_waiting(me, true, g::now());
                u.queues.note_input_wait(me);
            });
            let r = g::wait_any(&list, due as u32);
            with(|u| u.queues.set_waiting(me, false, g::now()));
            if r != g::WAIT_TIMEOUT && (r as usize) < list.len() - 1 {
                return r;
            }
            if r == g::WAIT_FAILED {
                return r;
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn TranslateMessage(src: usize) -> i32 {
    let src = door_windows_abi::widen(src);
    {
        let Some(m) = read_msg(src) else {
            g::set_last_error(e::INVALID_PARAMETER);
            return 0;
        };
        if !matches!(
            m.message,
            wm::KEYDOWN | wm::SYSKEYDOWN | wm::KEYUP | wm::SYSKEYUP
        ) {
            return 0;
        }
        if matches!(m.message, wm::KEYDOWN | wm::SYSKEYDOWN) {
            let ch = with(|u| {
                let (shift, ctrl, alt) = u.queues.key_state(super::tid()).modifiers();
                let caps = u
                    .queues
                    .key_state(super::tid())
                    .toggled(keyboard::vk::CAPITAL);
                keyboard::to_char((m.wparam & 0xff) as u8, shift, caps, ctrl, alt)
            });
            if let Some(c) = ch {
                let kind = if m.message == wm::SYSKEYDOWN {
                    wm::SYSCHAR
                } else {
                    wm::CHAR
                };
                let _ = post(m.hwnd, kind, u64::from(c), m.lparam);
            }
        }
        1
    }
}

/// WM_TIMER with a procedure: call it if that timer exists.
fn timer_procedure(m: &Message) -> Option<u64> {
    if m.message != wm::TIMER || m.lparam == 0 {
        return None;
    }
    with(|u| {
        u.queues
            .timers()
            .iter()
            .any(|t| t.hwnd == m.hwnd && t.id == m.wparam && t.procedure == m.lparam as u64)
            .then_some(m.lparam as u64)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn DispatchMessageW(src: usize) -> isize {
    let src = door_windows_abi::widen(src);
    ((move || -> i64 {
        let Some(m) = read_msg(src) else {
            g::set_last_error(e::INVALID_PARAMETER);
            return 0;
        };
        if let Some(p) = timer_procedure(&m) {
            call(p, m.hwnd, wm::TIMER, m.wparam, g::now() as u32 as i64);
            return 0;
        }
        if m.hwnd == 0 {
            return 0;
        }
        let me = tid();
        let target = with(|u| {
            let h = u.windows.resolve(m.hwnd)?;
            let w = u.windows.get(h)?;
            Some((w.thread, w.procedure))
        });
        match target {
            Some((thread, p)) if thread == me => call(p, m.hwnd, m.message, m.wparam, m.lparam),
            Some(_) => {
                g::set_last_error(e::MESSAGE_SYNC_ONLY);
                0
            }
            None => {
                g::set_last_error(e::INVALID_WINDOW_HANDLE);
                0
            }
        }
    })()) as isize
}

/// Post to a window's thread (0 = this thread's queue). Wakes the owner.
pub(crate) fn post(hwnd: u64, message_id: u32, wparam: u64, lparam: i64) -> Result<(), u32> {
    let m = message(hwnd, message_id, wparam, lparam);
    with(|u| {
        let thread = if hwnd == 0 {
            let me = tid();
            u.thread(me);
            me
        } else {
            let h = u.windows.resolve(hwnd).ok_or(e::INVALID_WINDOW_HANDLE)?;
            u.windows.get(h).unwrap().thread
        };
        u.queues.post(thread, m).map_err(|_| 1816u32)?;
        u.wake(thread);
        Ok(())
    })
}
pub(crate) fn post_input(hwnd: u64, message_id: u32, wparam: u64, lparam: i64) {
    let m = message(hwnd, message_id, wparam, lparam);
    with(|u| {
        if let Some(h) = u.windows.resolve(hwnd) {
            let thread = u.windows.get(h).unwrap().thread;
            if u.queues.post_input(thread, m).is_ok() {
                u.wake(thread);
            }
        }
    });
}

/// Messages whose parameters are pointers cannot be posted.
fn sync_only(message_id: u32) -> bool {
    matches!(
        message_id,
        wm::SETTEXT
            | wm::GETTEXT
            | wm::GETMINMAXINFO
            | wm::NCCREATE
            | wm::CREATE
            | wm::NCCALCSIZE
            | wm::WINDOWPOSCHANGING
            | wm::WINDOWPOSCHANGED
            | 0x004a
    )
}

#[unsafe(no_mangle)]
pub extern "system" fn PostMessageW(
    hwnd: usize,
    message_id: u32,
    wparam: usize,
    lparam: isize,
) -> i32 {
    let hwnd = door_windows_abi::widen(hwnd);
    let wparam = door_windows_abi::widen(wparam);
    let lparam = lparam as i64;
    {
        if sync_only(message_id) {
            g::set_last_error(e::MESSAGE_SYNC_ONLY);
            return 0;
        }
        match post(hwnd, message_id, wparam, lparam) {
            Ok(()) => 1,
            Err(code) => {
                g::set_last_error(code);
                0
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn PostThreadMessageW(
    thread: u32,
    message_id: u32,
    wparam: usize,
    lparam: isize,
) -> i32 {
    let wparam = door_windows_abi::widen(wparam);
    let lparam = lparam as i64;
    {
        let m = message(0, message_id, wparam, lparam);
        let r = with(|u| {
            u.queues.post(thread, m).map_err(|_| e::INVALID_THREAD_ID)?;
            u.wake(thread);
            Ok::<(), u32>(())
        });
        match r {
            Ok(()) => 1,
            Err(code) => {
                g::set_last_error(code);
                0
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn PostQuitMessage(code: i32) {
    let me = tid();
    with(|u| {
        u.thread(me);
        let _ = u.queues.post_quit(me, code);
    });
}

/// Outcome of a send that could not complete.
pub(crate) enum SendError {
    InvalidWindow,
    Timeout,
    Hung,
    /// A pointer parameter could not be read for marshalling.
    BadParameter,
}

/// How a cross-thread send's pointer parameter was copied into its
/// record-owned buffer (Windows marshals these system messages through the
/// kernel): the receiver reads and writes the copy, the sender copies the
/// output back only once it has the reply, so a sender that timed out never
/// leaves the receiver a pointer into its dead stack.
#[derive(Clone, Copy)]
enum Marshalled {
    /// WM_GETTEXT: `units` UTF-16 units out.
    Text { units: usize },
    /// A fixed structure in and out (MINMAXINFO, WINDOWPOS, NCCALCSIZE RECT).
    Out { bytes: usize },
    /// NCCALCSIZE_PARAMS (three RECTs and a WINDOWPOS pointer) and the
    /// WINDOWPOS it points to, placed after it.
    CalcSize { pos: u64 },
    /// Input only (WM_SETTEXT string, WM_COPYDATA).
    In,
}
const MAX_MARSHAL: usize = 1 << 20;
const WM_COPYDATA: u32 = 0x004a;

/// Copy a cross-thread send's pointer parameter. None: nothing to marshal.
fn marshal(
    message_id: u32,
    wparam: u64,
    lparam: i64,
) -> Result<Option<(Marshalled, Vec<u8>)>, SendError> {
    let p = lparam as u64;
    if p == 0 {
        return Ok(None);
    }
    let read = |at: u64, n: usize| -> Result<Vec<u8>, SendError> {
        let mut b = alloc::vec![0u8; n];
        if g::read(at, &mut b) {
            Ok(b)
        } else {
            Err(SendError::BadParameter)
        }
    };
    Ok(Some(match message_id {
        wm::GETTEXT => {
            let units = (wparam as usize).min(0x8000);
            (
                Marshalled::Text { units },
                alloc::vec![0u8; units.max(1) * 2],
            )
        }
        wm::SETTEXT => {
            let text = g::read_wide(p, 0x8000).ok_or(SendError::BadParameter)?;
            let mut b = Vec::with_capacity(text.len() * 2 + 2);
            for u in text.iter().chain(core::iter::once(&0)) {
                b.extend_from_slice(&u.to_le_bytes());
            }
            (Marshalled::In, b)
        }
        // MINMAXINFO holds no pointers (40 bytes in both builds).
        wm::GETMINMAXINFO => (Marshalled::Out { bytes: 40 }, read(p, 40)?),
        wm::WINDOWPOSCHANGING | wm::WINDOWPOSCHANGED => (
            Marshalled::Out {
                bytes: structs::WINDOWPOS_BYTES,
            },
            read(p, structs::WINDOWPOS_BYTES)?,
        ),
        wm::NCCALCSIZE if wparam == 0 => (Marshalled::Out { bytes: 16 }, read(p, 16)?),
        wm::NCCALCSIZE => {
            let mut b = read(p, structs::NCCALCSIZE_PARAMS_BYTES)?;
            let pos = structs::get(&b, &structs::NCCALCSIZE_PARAMS, 1);
            if pos != 0 {
                b.extend_from_slice(&read(pos, structs::WINDOWPOS_BYTES)?);
            }
            (Marshalled::CalcSize { pos }, b)
        }
        WM_COPYDATA => {
            let mut b = read(p, structs::COPYDATASTRUCT_BYTES)?;
            let bytes = structs::get(&b, &structs::COPYDATASTRUCT, 1) as usize;
            let data = structs::get(&b, &structs::COPYDATASTRUCT, 2);
            if bytes > MAX_MARSHAL {
                return Err(SendError::BadParameter);
            }
            if bytes != 0 {
                b.extend_from_slice(&read(data, bytes)?);
            }
            (Marshalled::In, b)
        }
        _ => return Ok(None),
    }))
}
/// Point the copy's nested pointers into the buffer; the new lParam.
fn marshalled_lparam(message_id: u32, kind: Marshalled, buffer: &mut [u8]) -> i64 {
    let base = buffer.as_ptr() as u64;
    match kind {
        Marshalled::CalcSize { pos } if pos != 0 => {
            let after = base + structs::NCCALCSIZE_PARAMS_BYTES as u64;
            structs::set(buffer, &structs::NCCALCSIZE_PARAMS, 1, after);
        }
        Marshalled::In
            if message_id == WM_COPYDATA && buffer.len() > structs::COPYDATASTRUCT_BYTES =>
        {
            let after = base + structs::COPYDATASTRUCT_BYTES as u64;
            structs::set(buffer, &structs::COPYDATASTRUCT, 2, after);
        }
        _ => (),
    }
    base as i64
}
/// Copy the receiver's output back into the sender's memory.
fn unmarshal(kind: Marshalled, original: u64, buffer: &[u8], result: i64) {
    let _ = match kind {
        Marshalled::Text { units } if units != 0 => {
            let n = (result.max(0) as usize).saturating_add(1).min(units);
            g::write(original, &buffer[..n * 2])
        }
        Marshalled::Out { bytes } => g::write(original, &buffer[..bytes]),
        Marshalled::CalcSize { pos } => {
            let rects = g::write(original, &buffer[..48]);
            let at = structs::NCCALCSIZE_PARAMS_BYTES;
            rects && (pos == 0 || g::write(pos, &buffer[at..at + structs::WINDOWPOS_BYTES]))
        }
        _ => true,
    };
}

/// SendMessage core. `timeout` None waits forever; `abort_if_hung` fails
/// at once for a hung receiver; `process` lets this thread run messages
/// sent to it while it waits (everything but SMTO_BLOCK).
pub(crate) fn send_message(
    hwnd: u64,
    message_id: u32,
    wparam: u64,
    lparam: i64,
    timeout: Option<u32>,
    abort_if_hung: bool,
    process: bool,
) -> Result<i64, SendError> {
    let me = tid();
    let target = with(|u| {
        let h = u.windows.resolve(hwnd)?;
        let w = u.windows.get(h)?;
        Some((w.thread, w.procedure))
    });
    let Some((thread, procedure)) = target else {
        return Err(SendError::InvalidWindow);
    };
    if thread == me {
        return Ok(call(procedure, hwnd, message_id, wparam, lparam));
    }
    if abort_if_hung && with(|u| u.queues.abandons_send(thread, g::now())) {
        return Err(SendError::Hung);
    }
    let mut m = message(hwnd, message_id, wparam, lparam);
    // Pointer parameters travel in a buffer the send record owns.
    let marshalled = marshal(message_id, wparam, lparam)?;
    let (kind, buffer) = match marshalled {
        Some((kind, mut buffer)) => {
            m.lparam = marshalled_lparam(message_id, kind, &mut buffer);
            if let Marshalled::Text { units } = kind {
                m.wparam = units as u64;
            }
            (Some(kind), Some(buffer))
        }
        None => (None, None),
    };
    let id = with(|u| {
        u.thread(me);
        let id = u.queues.begin_send_buffered(me, thread, m, buffer).ok()?;
        u.wake(thread);
        Some(id)
    });
    let Some(id) = id else {
        return Err(SendError::InvalidWindow);
    };
    let done = |reply: Option<(i64, Option<Vec<u8>>)>| -> Option<i64> {
        let (r, buffer) = reply?;
        if let (Some(kind), Some(buffer)) = (kind, buffer) {
            unmarshal(kind, lparam as u64, &buffer, r);
        }
        Some(r)
    };
    let start = g::now();
    let wake = wake_handle(me);
    loop {
        if let Some(r) = done(with(|u| u.queues.take_reply(id))) {
            return Ok(r);
        }
        if process {
            run_incoming_sends(me);
            if let Some(r) = done(with(|u| u.queues.take_reply(id))) {
                return Ok(r);
            }
        }
        let elapsed = g::now() - start;
        if let Some(t) = timeout
            && elapsed >= u64::from(t)
        {
            // A running send is abandoned: the receiver keeps writing into
            // the record's buffer, never into this caller's memory.
            let finished = with(|u| {
                let r = u.queues.take_reply(id);
                if r.is_none() {
                    u.queues.abandon(id);
                }
                r
            });
            return done(finished).ok_or(SendError::Timeout);
        }
        let wait = timeout.map_or(POLL_MS, |t| (u64::from(t) - elapsed).min(POLL_MS));
        with(|u| u.queues.set_waiting(me, true, g::now()));
        g::wait_any(&[wake], wait as u32);
        with(|u| u.queues.set_waiting(me, false, g::now()));
        server::pump();
    }
}

/// SendMessageW for internal callers (same semantics, no error report).
pub(crate) fn send(hwnd: u64, message_id: u32, wparam: u64, lparam: i64) -> i64 {
    send_message(hwnd, message_id, wparam, lparam, None, false, true).unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "system" fn SendMessageW(
    hwnd: usize,
    message_id: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    let hwnd = door_windows_abi::widen(hwnd);
    let wparam = door_windows_abi::widen(wparam);
    let lparam = lparam as i64;
    ((move || -> i64 {
        match send_message(hwnd, message_id, wparam, lparam, None, false, true) {
            Ok(r) => r,
            Err(SendError::BadParameter) => {
                g::set_last_error(e::INVALID_PARAMETER);
                0
            }
            Err(_) => {
                g::set_last_error(e::INVALID_WINDOW_HANDLE);
                0
            }
        }
    })()) as isize
}

const SMTO_BLOCK: u32 = 1;
const SMTO_ABORTIFHUNG: u32 = 2;

#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn SendMessageTimeoutW(
    hwnd: usize,
    message_id: u32,
    wparam: usize,
    lparam: isize,
    flags: u32,
    timeout: u32,
    result: usize,
) -> isize {
    let hwnd = door_windows_abi::widen(hwnd);
    let wparam = door_windows_abi::widen(wparam);
    let result = door_windows_abi::widen(result);
    let lparam = lparam as i64;
    ((move || -> i64 {
        match send_message(
            hwnd,
            message_id,
            wparam,
            lparam,
            Some(timeout),
            flags & SMTO_ABORTIFHUNG != 0,
            flags & SMTO_BLOCK == 0,
        ) {
            Ok(r) => {
                if result != 0 {
                    // lpdwResult is a DWORD_PTR.
                    g::write_pointer(result, r as u64);
                }
                1
            }
            Err(SendError::InvalidWindow) => {
                g::set_last_error(e::INVALID_WINDOW_HANDLE);
                0
            }
            Err(SendError::Timeout) => {
                g::set_last_error(e::TIMEOUT);
                0
            }
            Err(SendError::BadParameter) => {
                g::set_last_error(e::INVALID_PARAMETER);
                0
            }
            // An aborted send to a hung window leaves the last error untouched.
            Err(SendError::Hung) => 0,
        }
    })()) as isize
}

#[unsafe(no_mangle)]
pub extern "system" fn SendNotifyMessageW(
    hwnd: usize,
    message_id: u32,
    wparam: usize,
    lparam: isize,
) -> i32 {
    let hwnd = door_windows_abi::widen(hwnd);
    let wparam = door_windows_abi::widen(wparam);
    let lparam = lparam as i64;
    {
        let same =
            with(|u| u.windows.resolve(hwnd).and_then(|h| u.window_thread(h))) == Some(tid());
        if same {
            SendMessageW(
                door_windows_abi::narrow(hwnd),
                message_id,
                door_windows_abi::narrow(wparam),
                (lparam) as isize,
            );
            return 1;
        }
        PostMessageW(
            door_windows_abi::narrow(hwnd),
            message_id,
            door_windows_abi::narrow(wparam),
            (lparam) as isize,
        )
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn InSendMessage() -> i32 {
    let me = tid();
    i32::from(with(|u| u.queues.in_send(me)))
}

#[unsafe(no_mangle)]
pub extern "system" fn InSendMessageEx(_reserved: usize) -> u32 {
    {
        let me = tid();
        with(|u| {
            if !u.queues.in_send(me) {
                return 0; // ISMEX_NOSEND
            }
            // ISMEX_SEND, plus ISMEX_REPLIED once ReplyMessage answered it.
            if u.queues.current_replied(me) {
                1 | 8
            } else {
                1
            }
        })
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn ReplyMessage(result: isize) -> i32 {
    let result = result as i64;
    {
        let me = tid();
        with(|u| match u.queues.reply_current(me, result) {
            Some(sender) => {
                u.wake(sender);
                1
            }
            None => 0,
        })
    }
}

/// The time of the last message this thread retrieved (GetMessage, or
/// PeekMessage with PM_REMOVE).
#[unsafe(no_mangle)]
pub extern "system" fn GetMessageTime() -> i32 {
    let me = tid();
    with(|u| u.queues.thread(me).map_or(0, |t| t.message_time as i32))
}

#[unsafe(no_mangle)]
pub extern "system" fn SetTimer(hwnd: usize, id: usize, elapse: u32, procedure: usize) -> usize {
    let hwnd = door_windows_abi::widen(hwnd);
    let id = door_windows_abi::widen(id);
    let procedure = door_windows_abi::widen(procedure);
    door_windows_abi::narrow((move || -> u64 {
        let me = tid();
        let r = with(|u| {
            u.thread(me);
            if hwnd != 0 {
                let h = u.windows.resolve(hwnd).ok_or(e::INVALID_WINDOW_HANDLE)?;
                if u.windows.get(h).unwrap().thread != me {
                    return Err(e::ACCESS_DENIED);
                }
            }
            u.queues
                .set_timer(me, hwnd, id, elapse, procedure, g::now())
                .map_err(|err| err.win32())
        });
        match r {
            Ok(id) => id,
            Err(code) => {
                g::set_last_error(code);
                0
            }
        }
    })())
}

#[unsafe(no_mangle)]
pub extern "system" fn KillTimer(hwnd: usize, id: usize) -> i32 {
    let hwnd = door_windows_abi::widen(hwnd);
    let id = door_windows_abi::widen(id);
    {
        let me = tid();
        if with(|u| u.queues.kill_timer(me, hwnd, id)) {
            1
        } else {
            g::set_last_error(e::INVALID_PARAMETER);
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn IsHungAppWindow(hwnd: usize) -> i32 {
    let hwnd = door_windows_abi::widen(hwnd);
    {
        let Some(h) = super::hwnd(hwnd) else {
            return 0;
        };
        let now = g::now();
        i32::from(with(|u| {
            u.window_thread(h).is_some_and(|t| u.queues.is_hung(t, now))
        }))
    }
}

/// High word: QS_* kinds in the queue; low word: kinds added since the last
/// GetQueueStatus, GetMessage or PeekMessage; both masked by `flags`.
#[unsafe(no_mangle)]
pub extern "system" fn GetQueueStatus(flags: u32) -> u32 {
    let me = tid();
    with(|u| {
        u.thread(me);
        let paint = u.windows.paint_candidate(me, &|_: Handle| true).is_some();
        u.queues.queue_status(me, flags, g::now(), paint)
    })
}

/// Resolve and check a window belongs to this thread (DestroyWindow etc.).
pub(crate) fn own_window(value: u64) -> Result<Handle, u32> {
    let me = tid();
    let h = hwnd_or_error(value).ok_or(e::INVALID_WINDOW_HANDLE)?;
    if with(|u| u.window_thread(h)) != Some(me) {
        return Err(e::ACCESS_DENIED);
    }
    Ok(h)
}

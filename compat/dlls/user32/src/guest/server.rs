//! The window-service bridge: one authenticated connection per process,
//! native windows for visible top-level/owned/popup HWNDs, heap back buffers
//! committed as bands through a shared section, and translation of service
//! events into Win32 input, activation, sizing and close messages on the
//! owning thread.
//!
//! Geometry: a captioned Win32 window rectangle extends beyond its native
//! window by an invisible frame on the left, right and bottom (as the
//! Windows 10/11 resize borders do); the native caption is the Win32
//! non-client top. Surface pixel (0, 0) is window-relative
//! (`surface_left`, `surface_top`).
use super::{DoorGdiRegisterSurface, DoorGdiReleaseSurface, g, msg, point_lparam, tid, with};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use door_service_client::window::WindowClient;
use door_service_protocol::window::{self as w, Event};
use door_windows_gui::handles::Handle;
use door_windows_gui::keyboard::{self, vk};
use door_windows_gui::msg::{cs, ht, mk, sc, wa, wm, ws};
use door_windows_gui::present::{Fail, Presenter, Service};
use door_windows_gui::{Point, Rect};

pub(crate) struct Back {
    pub address: u64,
    pub width: u32,
    pub height: u32,
}

struct Section {
    handle: u64,
    address: u64,
    bytes: u64,
    width: u32,
    height: u32,
    buffer: u64,
}

pub(crate) struct Native {
    pub hwnd: Handle,
    /// Server window id; 0 when the service is unavailable (the window
    /// still paints into its back buffer, which is never presented).
    pub id: u64,
    pub epoch: u64,
    pub configured: bool,
    pub surface_left: i32,
    pub surface_top: i32,
    pub back: Back,
    section: Option<Section>,
    /// Attach/commit state: every successful commit consumes the attachment.
    presenter: Presenter,
    pub sizemove: bool,
    /// SET_RESPONDING last reported this window's thread hung.
    pub reported_hung: bool,
}

static CONN: g::Lock<Option<WindowClient>> = g::Lock::new(None);
static CONN_FAILED: AtomicBool = AtomicBool::new(false);
/// Timestamp of the last primary press delivered (MOVE_RESIZE serial).
static PRESS: AtomicU64 = AtomicU64::new(0);
static BUTTONS: AtomicU64 = AtomicU64::new(0);

/// Run `f` on the connection, connecting on first use. Callers never hold
/// a callback open, and `f` never touches the user state.
pub(crate) fn conn<R>(f: impl FnOnce(&mut WindowClient) -> R) -> Option<R> {
    if CONN_FAILED.load(Ordering::Relaxed) {
        return None;
    }
    let mut guard = CONN.lock();
    if guard.is_none() {
        let registry = g::inherited(door_abi::platform::inherited::REGISTRY);
        match WindowClient::connect_with(
            registry,
            w::CAP_WIN32 | w::CAP_MULTI_WINDOW | door_service_protocol::clipboard::CAP_CLIPBOARD,
        ) {
            Ok(c) => {
                let (width, height, _) = c.output();
                *guard = Some(c);
                drop(guard);
                if width != 0 && height != 0 {
                    with(|u| {
                        u.screen = (width as i32, height as i32);
                        u.work = Rect::new(0, 0, width as i32, height as i32 - 48);
                    });
                    // SAFETY: plain call into gdi32.
                    unsafe { super::DoorGdiSetScreen(width as i32, height as i32) };
                }
                guard = CONN.lock();
            }
            Err(_) => {
                CONN_FAILED.store(true, Ordering::Relaxed);
                return None;
            }
        }
    }
    guard.as_mut().map(f)
}

fn alloc_back(width: u32, height: u32) -> Option<Back> {
    let (width, height) = (width.max(1), height.max(1));
    let bytes = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    let layout = core::alloc::Layout::from_size_align(bytes, 16).ok()?;
    // SAFETY: nonzero size; released by `free_back` with the same layout.
    let address = unsafe { alloc::alloc::alloc_zeroed(layout) } as u64;
    if address == 0 {
        return None;
    }
    // SAFETY: registers memory we own for the GDI surface DCs.
    if unsafe { DoorGdiRegisterSurface(address, bytes as u64) } == 0 {
        unsafe { alloc::alloc::dealloc(address as *mut u8, layout) };
        return None;
    }
    // Start white, the default client background before WM_PAINT.
    // SAFETY: the block was just allocated with `bytes` bytes.
    let px = unsafe { core::slice::from_raw_parts_mut(address as *mut u32, bytes / 4) };
    px.fill(0x00ff_ffff);
    Some(Back {
        address,
        width,
        height,
    })
}
fn free_back(b: &Back) {
    // SAFETY: GDI stops drawing into the block before we free it.
    unsafe { DoorGdiReleaseSurface(b.address) };
    let bytes = b.width as usize * b.height as usize * 4;
    if let Ok(layout) = core::alloc::Layout::from_size_align(bytes, 16) {
        // SAFETY: allocated by `alloc_back` with this layout.
        unsafe { alloc::alloc::dealloc(b.address as *mut u8, layout) };
    }
}

fn alloc_section(width: u32, height: u32) -> Option<Section> {
    let bytes = (u64::from(width) * u64::from(height) * 4).div_ceil(4096) * 4096;
    // SAFETY: scalar call into ntdll's section window allocator.
    let address = unsafe { super::DoorReserveSectionRange(bytes) };
    if address == 0 {
        return None;
    }
    let release = || {
        // SAFETY: the range this call reserved, no longer mapped.
        unsafe { super::DoorReleaseSectionRange(address, bytes) };
    };
    let Ok(handle) =
        door_sys::section_create(bytes, door_abi::rights::ALL & !door_abi::rights::WAIT)
    else {
        release();
        return None;
    };
    // SAFETY: the reserved range is exclusively this section's; no Rust
    // reference into it exists.
    if unsafe {
        door_sys::section_map(
            handle,
            address,
            door_abi::rights::READ | door_abi::rights::WRITE,
        )
    }
    .is_err()
    {
        let _ = door_sys::close(handle);
        release();
        return None;
    }
    let buffer = conn(|c| c.register(handle, width, height, u64::from(width) * 4, bytes))
        .and_then(Result::ok);
    let Some(buffer) = buffer else {
        // SAFETY: our mapping, unreferenced.
        let _ = unsafe { door_sys::section_unmap(address, bytes) };
        let _ = door_sys::close(handle);
        release();
        return None;
    };
    Some(Section {
        handle,
        address,
        bytes,
        width,
        height,
        buffer,
    })
}
fn free_section(s: &Section) {
    conn(|c| c.unregister(s.buffer));
    // SAFETY: the mapping belongs to this section and nothing references it.
    if unsafe { door_sys::section_unmap(s.address, s.bytes) }.is_ok() {
        // SAFETY: the range alloc_section reserved, now unmapped.
        unsafe { super::DoorReleaseSectionRange(s.address, s.bytes) };
    }
    let _ = door_sys::close(s.handle);
}

/// ASCII rendering of a title (the window service accepts printable ASCII).
fn ascii(text: &[u16]) -> alloc::string::String {
    text.iter()
        .take(128)
        .map(|&u| {
            if (32..=126).contains(&u) {
                u as u8 as char
            } else {
                '?'
            }
        })
        .collect()
}

/// The native window rectangle of a Win32 window rectangle.
fn native_rect(r: Rect, frame: i32, top: i32) -> w::Rect {
    w::Rect {
        x: r.left + frame,
        y: r.top,
        width: (r.width() - 2 * frame).max(1) as u32,
        height: (r.height() - frame).max(top + 1) as u32,
    }
}

/// The window has its native window (or needs none: child, message-only).
pub(crate) fn has_native(h: Handle) -> bool {
    with(|u| {
        u.natives.iter().any(|n| n.hwnd == h)
            || u.windows
                .get(h)
                .is_none_or(|w| w.message_only || w.parent.is_some())
    })
}

/// Create the native window of a top-level HWND that became visible.
/// False when the window service refused it (its per-process window quota,
/// resources) or the back buffer could not be allocated: the caller fails
/// with ERROR_NOT_ENOUGH_QUOTA instead of showing a window that does not
/// exist. Without any window service the window still paints into its back
/// buffer (id 0, never presented).
pub(crate) fn create_native(h: Handle) -> bool {
    let info = with(|u| {
        if u.natives.iter().any(|n| n.hwnd == h) {
            return None;
        }
        let win = u.windows.get(h)?;
        if win.message_only || win.parent.is_some() {
            return None;
        }
        let owner = win
            .owner
            .and_then(|o| u.natives.iter().find(|n| n.hwnd == o))
            .map(|n| n.id)
            .filter(|id| *id != 0);
        Some((win.rect, win.style, ascii(&win.text), owner, win.enabled()))
    });
    let Some((rect, style, title, owner, enabled)) = info else {
        return has_native(h);
    };
    let captioned = style & ws::CAPTION == ws::CAPTION;
    let top = if captioned {
        door_windows_gui::metrics::CAPTION
    } else {
        0
    };
    let frame = if captioned {
        door_windows_gui::metrics::frame(style)
    } else {
        0
    };
    let r = native_rect(rect, frame, top);
    let Some(back) = alloc_back(r.width, r.height - top as u32) else {
        return false;
    };
    let id = match conn(|c| match (captioned, owner) {
        (true, Some(o)) => c.create_popup(o, r, &title),
        (true, None) => c.create(r, &title),
        (false, _) => c.create_client_caption(r, &title, 1, 138),
    }) {
        Some(Ok(id)) if id != 0 => id,
        // No window service at all: paint off-screen (id 0).
        None => 0,
        // Refused (quota, resources): no window rather than a fake one.
        Some(_) => {
            free_back(&back);
            return false;
        }
    };
    if id != 0 {
        // Without WS_CAPTION the window draws all of itself (popups, menus,
        // custom chrome): no trusted caption buttons over its client area.
        if !captioned {
            conn(|c| c.set_frameless(id, true));
        } else {
            // WS_MINIMIZEBOX / WS_MAXIMIZEBOX: the caption's buttons (a
            // dialog frame offers only Close).
            let buttons = u32::from(style & ws::MINIMIZEBOX != 0)
                | u32::from(style & ws::MAXIMIZEBOX != 0) << 1;
            if buttons != 3 {
                conn(|c| c.set_caption_buttons(id, buttons));
            }
        }
        if let Some(o) = owner {
            conn(|c| c.set_owner(id, o));
        }
        if !enabled {
            conn(|c| c.set_enabled(id, false));
        }
    }
    with(|u| {
        u.natives.push(Native {
            hwnd: h,
            id,
            epoch: 0,
            configured: false,
            surface_left: frame,
            surface_top: top,
            back,
            section: None,
            presenter: Presenter::default(),
            sizemove: false,
            reported_hung: false,
        });
        if let Some(win) = u.windows.get_mut(h) {
            win.native = (id != 0).then_some(id);
        }
    });
    true
}

/// Destroy the native window of a hidden or destroyed HWND.
pub(crate) fn destroy_native(h: Handle) {
    let native = with(|u| {
        let i = u.natives.iter().position(|n| n.hwnd == h)?;
        if let Some(win) = u.windows.get_mut(h) {
            win.native = None;
        }
        Some(u.natives.remove(i))
    });
    if let Some(n) = native {
        if n.id != 0 {
            conn(|c| c.destroy(n.id));
        }
        if let Some(s) = &n.section {
            free_section(s);
        }
        free_back(&n.back);
    }
}

fn native_id(h: Handle) -> Option<u64> {
    with(|u| u.windows.get(h).and_then(|w| w.native))
}
pub(crate) fn set_title(h: Handle) {
    let t = with(|u| Some((u.windows.get(h)?.native?, ascii(&u.windows.get(h)?.text))));
    if let Some((id, title)) = t {
        conn(|c| c.set_title(id, &title));
    }
}
pub(crate) fn set_enabled(h: Handle, enabled: bool) {
    if let Some(id) = native_id(h) {
        conn(|c| c.set_enabled(id, enabled));
    }
}
pub(crate) fn set_limits(h: Handle, min: (u32, u32), max: (u32, u32)) {
    if let Some(id) = native_id(h) {
        conn(|c| c.set_size_limits(id, min, max));
    }
}
/// Move/size the native window after a client SetWindowPos.
pub(crate) fn configure(h: Handle) {
    let t = with(|u| {
        let win = u.windows.get(h)?;
        let n = u.natives.iter().find(|n| n.hwnd == h)?;
        Some((
            win.native?,
            native_rect(win.rect, n.surface_left, n.surface_top),
        ))
    });
    if let Some((id, rect)) = t {
        conn(|c| c.configure(id, rect));
    }
}
pub(crate) fn activate(h: Handle) {
    if let Some(id) = native_id(h) {
        conn(|c| c.activate(id));
    }
}
pub(crate) fn request_state(h: Handle, state: u64) {
    if let Some(id) = native_id(h) {
        conn(|c| c.state(id, state));
    }
}
/// Continue the held press as a trusted move (edges 0) or resize.
pub(crate) fn move_resize(h: Handle, edges: u32) -> bool {
    let t = with(|u| {
        let root = u.windows.root(h);
        Some((root, u.windows.get(root)?.native?))
    });
    let Some((root, id)) = t else {
        return false;
    };
    let serial = PRESS.load(Ordering::Relaxed);
    msg::send(root.value(), wm::ENTERSIZEMOVE, 0, 0);
    let ok = conn(|c| c.move_resize(id, edges, serial)).is_some_and(|r| r.is_ok());
    with(|u| {
        if let Some(n) = u.natives.iter_mut().find(|n| n.hwnd == root) {
            n.sizemove = ok;
        }
    });
    if !ok {
        msg::send(root.value(), wm::EXITSIZEMOVE, 0, 0);
    }
    ok
}

/// Poll the service and route events to their owning threads.
pub(crate) fn pump() {
    let mut events: Vec<Event> = Vec::new();
    let mut lost = false;
    retry_reconnect();
    {
        let Some(mut guard) = CONN.try_lock() else {
            return;
        };
        let Some(c) = guard.as_mut() else {
            return;
        };
        for _ in 0..64 {
            match c.poll() {
                Ok(Some(e)) => events.push(e),
                // The service incarnation ended (restart): reconnect below.
                Err(door_service_client::Error::Transport(
                    door_abi::Status::PeerClosed | door_abi::Status::NotFound,
                )) => {
                    lost = true;
                    *guard = None;
                    break;
                }
                _ => break,
            }
        }
    }
    if lost {
        reconnect();
        return;
    }
    let me = tid();
    let mut retry = Vec::new();
    let mut reports = Vec::new();
    // Notification-icon clicks name a window, not a native surface.
    let (clicks, events): (Vec<Event>, Vec<Event>) = events
        .into_iter()
        .partition(|e| e.kind == w::NOTIFY_ICON_EVENT);
    for e in &clicks {
        super::notify::clicked(e);
    }
    with(|u| {
        for e in events {
            if e.window == 0 || matches!(e.kind, w::RELEASED | w::FRAME_READY | w::PRESENTED) {
                continue;
            }
            let Some(n) = u.natives.iter().find(|n| n.id == e.window) else {
                continue;
            };
            let Some(thread) = u.windows.get(n.hwnd).map(|w| w.thread) else {
                continue;
            };
            let t = u.thread(thread);
            if t.events.len() < 1024 {
                t.events.push_back(e);
            }
            u.wake(thread);
        }
        for n in &u.natives {
            if n.presenter.retry && u.windows.get(n.hwnd).is_some_and(|w| w.thread == me) {
                retry.push(n.hwnd);
            }
        }
        // The hung rule per thread (IsHungAppWindow): a window whose thread
        // has not retrieved messages for five seconds while messages wait.
        // This thread still polls, so the server cannot see it by itself.
        let now = g::now();
        for n in u.natives.iter_mut().filter(|n| n.id != 0) {
            let Some(thread) = u.windows.get(n.hwnd).map(|w| w.thread) else {
                continue;
            };
            let (posted, input, sent, _) = u.queues.pending(thread);
            let pending = posted
                || input
                || sent
                || u.threads
                    .iter()
                    .any(|t| t.id == thread && !t.events.is_empty());
            let hung = thread != me && pending && u.queues.is_hung(thread, now);
            if hung != n.reported_hung {
                n.reported_hung = hung;
                reports.push((n.id, !hung));
            }
        }
    });
    for (id, responding) in reports {
        conn(|c| c.set_responding(id, responding));
    }
    for h in retry {
        present(h, None);
    }
}

/// Handle one service event on the window's own thread (no lock held).
pub(crate) fn handle_event(e: Event) {
    let Some(h) = with(|u| {
        u.natives
            .iter()
            .find(|n| n.id == e.window && n.id != 0)
            .map(|n| n.hwnd)
    }) else {
        return;
    };
    match e.kind {
        w::CONFIGURED => configured(h, &e),
        w::INPUT => input(h, &e),
        w::FOCUS => focus(h, e.a != 0),
        w::CLOSE => {
            // Windows 11: the close is acknowledged the moment the thread
            // retrieves it; WM_CLOSE then takes as long as the application
            // needs (a save prompt may stay open indefinitely). Only the
            // server's hung rule (no retrieval for 5 s) can mark it.
            conn(|c| c.cancel_close(e.window));
            msg::send(h.value(), wm::SYSCOMMAND, sc::CLOSE as u64, 0);
            // Still there: the application declined (Cancel in its save
            // prompt). During a shutdown this cancels the shutdown.
            if native_id(h) == Some(e.window) {
                conn(|c| c.close_declined(e.window));
            }
        }
        w::CAPTURE_LOST => {
            let me = tid();
            if let Some(c) = with(|u| u.thread(me).capture.take()) {
                msg::send(c.value(), wm::CAPTURECHANGED, 0, 0);
            }
        }
        w::DISMISS_TRANSIENTS => super::menu::cancel_tracking(),
        _ => (),
    }
}

fn configured(h: Handle, e: &Event) {
    let (width, height) = (e.a, e.b);
    let Some((first, frame)) = with(|u| {
        let n = u.natives.iter_mut().find(|n| n.hwnd == h)?;
        let first = !n.configured;
        n.configured = true;
        n.epoch = e.value;
        n.presenter.configured();
        Some((first, n.surface_left))
    }) else {
        return;
    };
    let outer = Rect::from_size(
        e.rect.x - frame,
        e.rect.y,
        e.rect.width as i32 + 2 * frame,
        e.rect.height as i32 + frame,
    );
    // The first configure adopts the server position silently; size
    // changes, and later moves, reach the window as Win32 messages.
    let current = with(|u| u.windows.get(h).map(|w| w.rect)).unwrap_or(outer);
    let moved = current.top_left() != outer.top_left();
    let sized = current.size() != outer.size();
    if first && moved && !sized {
        with(|u| {
            if let Some(win) = u.windows.get_mut(h) {
                win.rect = outer;
            }
        });
    } else if moved || sized {
        super::wnd::apply_geometry(h, outer, !moved, !sized, false);
    }
    // Back buffer and section follow the exact content size.
    let need_back = with(|u| {
        let n = u.natives.iter().find(|n| n.hwnd == h)?;
        Some(n.back.width != width || n.back.height != height)
    })
    .unwrap_or(false);
    if need_back
        && width > 0
        && height > 0
        && let Some(back) = alloc_back(width, height)
    {
        let old = with(|u| {
            let n = u.natives.iter_mut().find(|n| n.hwnd == h)?;
            Some(core::mem::replace(&mut n.back, back))
        });
        if let Some(old) = old {
            free_back(&old);
        }
        with(|u| {
            u.windows.invalidate(h, None, true);
            for x in u.windows.subtree(h) {
                if let Some(w) = u.windows.get_mut(x) {
                    w.frame_dirty = true;
                }
            }
            if let Some(t) = u.window_thread(h) {
                u.wake(t);
            }
        });
    }
    let need_section = with(|u| {
        let n = u.natives.iter().find(|n| n.hwnd == h)?;
        Some(
            n.section
                .as_ref()
                .is_none_or(|s| s.width != width || s.height != height),
        )
    })
    .unwrap_or(false);
    if need_section && width > 0 && height > 0 {
        let old = with(|u| {
            u.natives
                .iter_mut()
                .find(|n| n.hwnd == h)
                .and_then(|n| n.section.take())
        });
        if let Some(s) = old {
            free_section(&s);
        }
        if let Some(s) = alloc_section(width, height) {
            with(|u| {
                if let Some(n) = u.natives.iter_mut().find(|n| n.hwnd == h) {
                    n.section = Some(s);
                    n.presenter.committed = false;
                }
            });
        }
    }
    let t = with(|u| {
        let n = u.natives.iter().find(|n| n.hwnd == h)?;
        n.section.as_ref()?;
        Some((n.id, n.epoch))
    });
    if let Some((id, epoch)) = t {
        // present() attaches the buffer before each commit.
        let ok = conn(|c| c.acknowledge(id, epoch)).is_some_and(|r| r.is_ok());
        with(|u| {
            if let Some(n) = u.natives.iter_mut().find(|n| n.hwnd == h) {
                n.presenter.configured();
                n.presenter.acknowledged = ok;
            }
        });
        present(h, None);
    }
}

/// The window service bound to one window, buffer and epoch.
struct Bound<'a> {
    client: &'a mut WindowClient,
    id: u64,
    buffer: u64,
    epoch: u64,
}
fn fail(e: door_service_client::Error) -> Fail {
    match e {
        door_service_client::Error::Service(door_service_protocol::Code::WouldBlock) => {
            Fail::WouldBlock
        }
        _ => Fail::Other,
    }
}
impl Service for Bound<'_> {
    fn attach(&mut self) -> Result<(), Fail> {
        self.client
            .attach(self.id, self.buffer, self.epoch)
            .map_err(fail)
    }
    fn commit(&mut self, band: Option<(u32, u32)>) -> Result<(), Fail> {
        match band {
            Some((top, bottom)) => self
                .client
                .commit_band(self.id, self.epoch, top, bottom - top),
            None => self.client.commit_epoch(self.id, self.epoch),
        }
        .map_err(fail)
    }
}

/// Copy changed rows (all when `rows` is None) of a top-level window's back
/// buffer into its section, alpha forced opaque, then attach (each commit
/// consumes the attachment) and commit them.
pub(crate) fn present(h: Handle, rows: Option<(u32, u32)>) {
    let t = with(|u| {
        let n = u.natives.iter().find(|n| n.hwnd == h)?;
        if n.id == 0 || !n.configured {
            return None;
        }
        let s = n.section.as_ref()?;
        if s.width != n.back.width || s.height != n.back.height {
            return None;
        }
        let (top, bottom) = n.presenter.rows(rows, s.height)?;
        Some((
            n.id,
            n.epoch,
            n.back.address,
            s.address,
            s.buffer,
            s.width,
            n.presenter,
            (top, bottom),
        ))
    });
    let Some((id, epoch, back, section, buffer, width, mut presenter, (top, bottom))) = t else {
        return;
    };
    let stride = width as usize;
    // SAFETY: both stores are this DLL's (heap back buffer and mapped
    // section of exactly width x height pixels); no references persist.
    unsafe {
        let src = back as *const u32;
        let dst = section as *mut u32;
        for i in top as usize * stride..bottom as usize * stride {
            dst.add(i).write(src.add(i).read() | 0xff00_0000);
        }
    }
    let done = conn(|c| {
        let band = c.supports_band_commit();
        let mut bound = Bound {
            client: c,
            id,
            buffer,
            epoch,
        };
        let _ = presenter.commit((top, bottom), rows.is_some(), band, &mut bound);
    });
    if done.is_none() {
        return;
    }
    // A configure handled meanwhile owns the state of its new epoch.
    with(|u| {
        if let Some(n) = u.natives.iter_mut().find(|n| n.hwnd == h)
            && n.epoch == epoch
        {
            n.presenter = presenter;
        }
    });
}

/// The deepest visible, enabled child of `top` containing screen point `p`.
fn child_at(top: Handle, p: Point) -> Handle {
    with(|u| {
        let mut cur = top;
        loop {
            let Some(win) = u.windows.get(cur) else {
                return cur;
            };
            let origin = u.windows.client_origin(cur);
            let local = Point::new(p.x - origin.x, p.y - origin.y);
            let next = win.children.iter().copied().find(|&c| {
                u.windows
                    .get(c)
                    .is_some_and(|cw| cw.visible() && cw.rect.contains(local))
            });
            match next {
                Some(c) if u.windows.get(c).is_some_and(|cw| cw.enabled()) => cur = c,
                _ => return cur,
            }
        }
    })
}

fn mk_flags() -> u64 {
    let b = BUTTONS.load(Ordering::Relaxed);
    let (shift, ctrl, _) = with(|u| u.async_keys.modifiers());
    let mut v = 0usize;
    if b & 1 != 0 {
        v |= mk::LBUTTON;
    }
    if b & 2 != 0 {
        v |= mk::RBUTTON;
    }
    if b & 4 != 0 {
        v |= mk::MBUTTON;
    }
    if shift {
        v |= mk::SHIFT;
    }
    if ctrl {
        v |= mk::CONTROL;
    }
    v as u64
}

fn input(h: Handle, e: &Event) {
    let me = tid();
    let sizemove = with(|u| {
        let n = u.natives.iter_mut().find(|n| n.hwnd == h)?;
        Some(core::mem::replace(&mut n.sizemove, false))
    })
    .unwrap_or(false);
    if sizemove {
        msg::send(h.value(), wm::EXITSIZEMOVE, 0, 0);
    }
    if e.detail == 1 {
        key(me, e);
        return;
    }
    if !(2..=4).contains(&e.detail) {
        return;
    }
    let (left, top) = with(|u| {
        u.natives
            .iter()
            .find(|n| n.hwnd == h)
            .map(|n| (n.surface_left, n.surface_top))
    })
    .unwrap_or((0, 0));
    let origin = with(|u| u.windows.get(h).map(|w| w.rect.top_left())).unwrap_or_default();
    let screen = Point::new(origin.x + left + e.rect.x, origin.y + top + e.rect.y);
    with(|u| u.last_click.3 = screen);
    if e.detail == w::INPUT_WHEEL {
        let target = with(|u| u.thread(me).focus).unwrap_or(h);
        let delta = (e.a as i32).saturating_mul(120);
        let wp = (mk_flags() & 0xffff) | (u64::from(delta as u32 & 0xffff) << 16);
        msg::post_input(
            target.value(),
            wm::MOUSEWHEEL,
            wp,
            point_lparam(screen.x, screen.y),
        );
        return;
    }
    let capture = with(|u| u.thread(me).capture.filter(|c| u.windows.get(*c).is_some()));
    let (target, hit) = match capture {
        Some(c) => (c, ht::CLIENT as i64),
        None => {
            let t = child_at(h, screen);
            (
                t,
                msg::send(
                    t.value(),
                    wm::NCHITTEST,
                    0,
                    point_lparam(screen.x, screen.y),
                ),
            )
        }
    };
    let client = with(|u| {
        let o = u.windows.client_origin(target);
        Point::new(screen.x - o.x, screen.y - o.y)
    });
    let in_client = hit == ht::CLIENT as i64;
    if e.detail == 2 {
        // TrackMouseEvent(TME_LEAVE): the window the pointer left.
        super::system::pointer_over(target);
        if in_client {
            msg::post_input(
                target.value(),
                wm::MOUSEMOVE,
                mk_flags(),
                point_lparam(client.x, client.y),
            );
        } else {
            msg::post_input(
                target.value(),
                wm::NCMOUSEMOVE,
                hit as u64,
                point_lparam(screen.x, screen.y),
            );
        }
        return;
    }
    let button = e.a.min(2);
    let down = e.b != 0;
    let bit = 1u64 << button;
    if down {
        BUTTONS.fetch_or(bit, Ordering::Relaxed);
        if button == 0 {
            PRESS.store(e.timestamp, Ordering::Relaxed);
        }
    } else {
        BUTTONS.fetch_and(!bit, Ordering::Relaxed);
    }
    let (cdown, ncdown) = [
        (wm::LBUTTONDOWN, wm::NCLBUTTONDOWN),
        (wm::RBUTTONDOWN, wm::NCRBUTTONDOWN),
        (wm::MBUTTONDOWN, wm::NCMBUTTONDOWN),
    ][button as usize];
    if !in_client {
        let m = if down { ncdown } else { ncdown + 1 };
        msg::post_input(
            target.value(),
            m,
            hit as u64,
            point_lparam(screen.x, screen.y),
        );
        return;
    }
    let m = if !down {
        cdown + 1
    } else {
        let double = with(|u| {
            let dblclks = u
                .windows
                .get(target)
                .and_then(|w| u.classes.by_atom(w.class))
                .and_then(|i| u.classes.get(i))
                .is_some_and(|c| c.style & cs::DBLCLKS != 0);
            let (last, last_msg, last_time, _) = u.last_click;
            let now = e.timestamp;
            let hit = dblclks
                && last == target.value()
                && last_msg == cdown
                && now.saturating_sub(last_time) <= 500;
            u.last_click = (if hit { 0 } else { target.value() }, cdown, now, screen);
            hit
        });
        if double { cdown + 2 } else { cdown }
    };
    msg::post_input(
        target.value(),
        m,
        mk_flags(),
        point_lparam(client.x, client.y),
    );
}

fn key(me: u32, e: &Event) {
    let code = e.a;
    let down = e.b != 0;
    let scan = (code & 0x1ff) as u16;
    let flags = (code >> 9) & 0x7f;
    let repeat = flags & door_abi::graphics::REPEAT != 0;
    let sided = keyboard::scan_to_vk_ex(scan);
    if sided == 0 {
        return;
    }
    let key = keyboard::generic(sided);
    // The device state changes now (GetAsyncKeyState); the thread's
    // GetKeyState changes when it retrieves the key message below.
    let (alt, ctrl) = with(|u| {
        u.async_keys.apply(sided, down);
        (u.async_keys.down(vk::MENU), u.async_keys.down(vk::CONTROL))
    });
    let target = with(|u| {
        let t = u.thread(me);
        let focus = t.focus.or(t.active);
        focus.filter(|f| u.windows.get(*f).is_some())
    });
    let Some(target) = target else {
        // No message carries this transition: apply it at once.
        with(|u| u.queues.apply_key(me, sided, down));
        return;
    };
    let sys = (alt && !ctrl) || key == vk::F10 || key == vk::MENU;
    let message = match (down, sys) {
        (true, false) => wm::KEYDOWN,
        (false, false) => wm::KEYUP,
        (true, true) => wm::SYSKEYDOWN,
        (false, true) => wm::SYSKEYUP,
    };
    let mut lp = keyboard::key_lparam(key, down, repeat, alt);
    // The physical scan code: it names the side (right Shift) the key state
    // applies when the message is retrieved.
    lp = (lp & !(0xff << 16)) | (i64::from(scan & 0xff) << 16);
    if scan & 0x100 != 0 {
        lp |= 1 << 24;
    }
    msg::post_input(target.value(), message, u64::from(key), lp);
}

fn focus(h: Handle, gained: bool) {
    let me = tid();
    if gained {
        let old = with(|u| {
            u.foreground = Some(h);
            u.thread(me).active.replace(h)
        });
        if let Some(o) = old.filter(|o| *o != h) {
            msg::send(o.value(), wm::NCACTIVATE, 0, 0);
            msg::send(
                o.value(),
                wm::ACTIVATE,
                wa::INACTIVE as u64,
                h.value() as i64,
            );
        }
        msg::send(h.value(), wm::NCACTIVATE, 1, 0);
        msg::send(
            h.value(),
            wm::ACTIVATE,
            wa::ACTIVE as u64,
            old.map_or(0, |o| o.value() as i64),
        );
    } else {
        let was = with(|u| {
            let t = u.thread(me);
            if t.active == Some(h) {
                t.active = None;
                true
            } else {
                false
            }
        });
        if was {
            msg::send(h.value(), wm::ACTIVATE, wa::INACTIVE as u64, 0);
            msg::send(h.value(), wm::NCACTIVATE, 0, 0);
            let lost = with(|u| {
                let focus = u.thread(me).focus;
                focus.filter(|f| *f == h || u.windows.is_descendant(h, *f))
            });
            if let Some(f) = lost {
                with(|u| u.thread(me).focus = None);
                msg::send(f.value(), wm::KILLFOCUS, 0, 0);
            }
        }
    }
}

/// Taskbar attention/progress of a top-level window (door.shell
/// SET_TASKBAR_STATE on the window's root native window). False without a
/// native window or service.
pub(crate) fn taskbar_state(h: Handle, attention: u32, progress: Option<(u8, u16)>) -> bool {
    let Some(id) = with(|u| {
        let root = u.windows.root(h);
        u.windows.get(root)?.native
    }) else {
        return false;
    };
    let (progress_state, progress) = progress.unwrap_or((0, 0));
    let state = door_service_protocol::shell::TaskbarState {
        window: id,
        attention,
        progress_state,
        progress,
    };
    conn(|c| c.set_taskbar_state(state)).is_some_and(|r| r.is_ok())
}

/// Top-level windows waiting for a new window-service incarnation, and the
/// tick of the last reconnection attempt.
static PENDING: g::Lock<Vec<Handle>> = g::Lock::new(Vec::new());
static RETRY_AT: AtomicU64 = AtomicU64::new(0);
/// GUI-H service reconnect: the window service restarted (its connection
/// reports closure). Forget the dead incarnation's windows and sections,
/// recreate every visible top-level window on the new one and repaint it
/// from the application's own state. The process and its HWNDs never die.
fn reconnect() {
    let lost: Vec<Native> = with(|u| {
        for n in &u.natives {
            if let Some(w) = u.windows.get_mut(n.hwnd) {
                w.native = None;
            }
        }
        core::mem::take(&mut u.natives)
    });
    for n in &lost {
        if let Some(s) = &n.section {
            // The server side is gone with its incarnation: unmap locally.
            // SAFETY: the mapping belongs to this section and nothing
            // references it once the native record is dropped.
            if unsafe { door_sys::section_unmap(s.address, s.bytes) }.is_ok() {
                // SAFETY: the range alloc_section reserved, now unmapped.
                unsafe { super::DoorReleaseSectionRange(s.address, s.bytes) };
            }
            let _ = door_sys::close(s.handle);
        }
        free_back(&n.back);
    }
    PENDING.lock().extend(lost.iter().map(|n| n.hwnd));
    let _ = door_sys::write(
        alloc::format!("DOOR_P14 USER32 SERVICE_LOST windows={}\n", lost.len()).as_bytes(),
    );
    retry_reconnect();
}
/// Connect to the new incarnation (at most every 100 ms) and recreate the
/// pending windows; until it answers they keep painting off-screen.
fn retry_reconnect() {
    if PENDING.lock().is_empty() {
        return;
    }
    let now = g::now();
    if now < RETRY_AT.load(Ordering::Relaxed) {
        return;
    }
    RETRY_AT.store(now.saturating_add(100), Ordering::Relaxed);
    CONN_FAILED.store(false, Ordering::Relaxed);
    if conn(|_| ()).is_none() {
        // Not ready yet: keep the windows and retry later.
        CONN_FAILED.store(false, Ordering::Relaxed);
        return;
    }
    let pending = core::mem::take(&mut *PENDING.lock());
    let mut recreated = 0u32;
    for h in &pending {
        let visible = with(|u| u.windows.get(*h).is_some_and(|w| w.visible()));
        if visible && create_native(*h) {
            configure(*h);
            // RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_ALLCHILDREN.
            super::paint::RedrawWindow(
                door_windows_abi::narrow(h.value()),
                0,
                0,
                0x1 | 0x4 | 0x400 | 0x80,
            );
            recreated += 1;
        }
    }
    let _ = door_sys::write(
        alloc::format!(
            "DOOR_P14 USER32 RECONNECTED windows={recreated} pending={}\n",
            pending.len()
        )
        .as_bytes(),
    );
}

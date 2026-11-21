//! PE exports of user32.
//!
//! Locking: one process lock (`USER`) protects the window, class, menu and
//! queue models; one connection lock (`CONN`) serializes window-service RPCs.
//! The order is USER then CONN, and neither is ever held while a window
//! procedure, hook or timer procedure runs: every callback first copies what
//! it needs out of the model, releases the lock, then calls.

mod controls;
mod dialog;
mod dpi;
mod icon;
mod menu;
mod misc;
mod msg;
mod notify;
mod paint;
mod server;
mod structs;
mod system;
mod wnd;

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use door_service_protocol::window::Event;
use door_windows_gui::handles::{Handle, Table, kind};
use door_windows_gui::keyboard::KeyState;
use door_windows_gui::menu::Menus;
use door_windows_gui::queue::Queues;
use door_windows_gui::resource::Accel;
use door_windows_gui::window::{Class, Classes, Windows};
use door_windows_gui::{Rect, win32_error as e};
use door_windows_gui_guest as g;

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

// The process section window, shared with the other compatibility DLLs.
#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    pub(crate) fn DoorReserveSectionRange(bytes: u64) -> u64;
    pub(crate) fn DoorReleaseSectionRange(address: u64, bytes: u64) -> i32;
}
#[link(name = "gdi32", kind = "dylib")]
unsafe extern "system" {
    pub(crate) fn DoorGdiRegisterSurface(address: u64, bytes: u64) -> i32;
    pub(crate) fn DoorGdiReleaseSurface(address: u64) -> i32;
    pub(crate) fn DoorGdiCreateSurfaceDC(
        address: u64,
        width: u32,
        height: u32,
        stride: u32,
        origin_x: i32,
        origin_y: i32,
        rects: u64,
        count: u32,
    ) -> u64;
    pub(crate) fn DoorGdiCreateDisplayDC() -> u64;
    pub(crate) fn DoorGdiObjectCount() -> u32;
    pub(crate) fn DoorGdiSetScreen(width: i32, height: i32) -> i32;
    pub(crate) fn DoorGdiSysColorBrush(index: i32) -> u64;
    pub(crate) fn DoorGdiFillRect(hdc: u64, rect: u64, brush: u64) -> i32;
    pub(crate) fn DoorGdiTextAdvances(hdc: u64, text: u64, count: u32, out: u64) -> i32;
    pub(crate) fn DeleteDC(hdc: usize) -> i32;
    pub(crate) fn SelectObject(hdc: usize, object: usize) -> usize;
    pub(crate) fn GetStockObject(index: i32) -> usize;
    pub(crate) fn SetBkMode(hdc: usize, mode: i32) -> i32;
    pub(crate) fn SetBkColor(hdc: usize, color: u32) -> u32;
    pub(crate) fn SetTextColor(hdc: usize, color: u32) -> u32;
    pub(crate) fn ExtTextOutW(
        hdc: usize,
        x: i32,
        y: i32,
        options: u32,
        rect: usize,
        text: usize,
        count: u32,
        dx: usize,
    ) -> i32;
    pub(crate) fn GetTextExtentPoint32W(hdc: usize, text: usize, count: i32, size: usize) -> i32;
    pub(crate) fn GetTextMetricsW(hdc: usize, metrics: usize) -> i32;
    pub(crate) fn CreateFontIndirectW(logfont: usize) -> usize;
}

/// Per-thread user state beyond the queue model.
pub(crate) struct ThreadState {
    pub id: u32,
    pub wake: u64,
    pub focus: Option<Handle>,
    pub active: Option<Handle>,
    pub capture: Option<Handle>,
    /// The thread's first top-level window was created (Windows creates
    /// the default IME window then).
    pub default_ime: bool,
    /// Window-service events routed to this thread, processed inside its
    /// Get/PeekMessage (they may call its window procedures).
    pub events: VecDeque<Event>,
}

/// Control state kept beside a window (EDIT model, button state, ...).
pub(crate) enum Control {
    Edit(door_windows_gui::edit::Edit),
    Button {
        checked: u32,
        pushed: bool,
        focused: bool,
    },
    List(controls::ListState),
}

/// Dialog runtime state (#32770 windows).
pub(crate) struct DialogState {
    pub ended: bool,
    pub result: i64,
    pub default_id: u32,
    pub modal: bool,
    pub focus_saved: Option<Handle>,
}

pub(crate) struct User {
    pub classes: Classes,
    pub windows: Windows,
    pub menus: Menus,
    pub accels: Table<Vec<Accel>>,
    pub queues: Queues,
    /// Device key state as events arrive (`GetAsyncKeyState`, mouse MK_*
    /// flags); each thread's synchronous `GetKeyState` lives in its queue.
    pub async_keys: KeyState,
    pub threads: Vec<ThreadState>,
    pub natives: Vec<server::Native>,
    pub controls: Vec<(Handle, Control)>,
    pub dialogs: Vec<(Handle, DialogState)>,
    /// Process-local clipboard text (no system clipboard bridge yet).
    pub clipboard: Vec<u16>,
    pub screen: (i32, i32),
    pub work: Rect,
    pub last_click: (u64, u32, u64, door_windows_gui::Point),
    pub foreground: Option<Handle>,
    /// Tracking limits from the last WM_GETMINMAXINFO of each window.
    pub limits: Vec<(Handle, Track, Track)>,
}

/// A (width, height) tracking size.
pub(crate) type Track = (i32, i32);

static USER: g::Lock<Option<User>> = g::Lock::new(None);

fn system_class(
    name: &str,
    procedure: u64,
    extra: u32,
    style: u32,
    cursor: u64,
    background: u64,
) -> Class {
    Class {
        atom: 0,
        name: name.encode_utf16().collect(),
        instance: 0,
        global: true,
        system: true,
        style,
        procedure,
        class_extra: Vec::new(),
        window_extra: extra,
        icon: 0,
        cursor,
        background,
        menu_name: 0,
        small_icon: 0,
        windows: 0,
    }
}

fn init() -> User {
    use door_windows_gui::msg::{color, cs};
    let mut classes = Classes::new();
    let arrow = misc::cursor_value(door_windows_gui::msg::idc::ARROW);
    let ibeam = misc::cursor_value(door_windows_gui::msg::idc::IBEAM);
    let face = (color::BTNFACE + 1) as u64;
    let window = (color::WINDOW + 1) as u64;
    for c in [
        system_class(
            "Button",
            controls::button_proc as *const () as u64,
            0,
            cs::DBLCLKS | cs::PARENTDC,
            arrow,
            0,
        ),
        system_class(
            "Static",
            controls::static_proc as *const () as u64,
            0,
            cs::PARENTDC,
            arrow,
            0,
        ),
        system_class(
            "Edit",
            controls::edit_proc as *const () as u64,
            0,
            cs::DBLCLKS | cs::PARENTDC,
            ibeam,
            0,
        ),
        system_class(
            "ListBox",
            controls::listbox_proc as *const () as u64,
            0,
            cs::DBLCLKS | cs::PARENTDC,
            arrow,
            window,
        ),
        system_class(
            "ScrollBar",
            controls::scrollbar_proc as *const () as u64,
            0,
            cs::PARENTDC,
            arrow,
            0,
        ),
        system_class(
            "#32770",
            dialog::DefDlgProcW as *const () as u64,
            dialog::DLGWINDOWEXTRA,
            cs::SAVEBITS | cs::DBLCLKS,
            arrow,
            face,
        ),
    ] {
        let _ = classes.register(c);
    }
    User {
        classes,
        windows: Windows::new(),
        menus: Menus::new(),
        accels: Table::new(kind::ACCEL, 1024),
        queues: Queues::new(),
        async_keys: KeyState::default(),
        threads: Vec::new(),
        natives: Vec::new(),
        controls: Vec::new(),
        dialogs: Vec::new(),
        clipboard: Vec::new(),
        screen: (1280, 720),
        work: Rect::new(0, 0, 1280, 672),
        last_click: (0, 0, 0, door_windows_gui::Point::default()),
        foreground: None,
        limits: Vec::new(),
    }
}

/// DLL entry point (`/ENTRY:DllMain`). The loader calls every DLL entry
/// with DLL_THREAD_DETACH (3) on an exiting thread: its windows and its
/// queue go, so sends to it complete and its queue slot serves later
/// threads. Every other reason needs no work.
#[unsafe(no_mangle)]
pub extern "system" fn DllMain(_module: usize, reason: u32, _reserved: usize) -> i32 {
    {
        if reason == 3 {
            thread_detached(tid());
        }
        1
    }
}

/// Release a terminating thread's user32 state. Windows destroys such a
/// thread's windows without sending it messages; senders waiting on it
/// complete with 0.
fn thread_detached(me: u32) {
    // A process that never used user32 has no state to create now.
    if USER.lock().is_none() {
        return;
    }
    if !with(|u| u.threads.iter().any(|t| t.id == me) || u.queues.thread(me).is_some()) {
        return;
    }
    wnd::free_thread_windows(me);
    let wake = with(|u| {
        for sender in u.queues.remove_thread(me) {
            u.wake(sender);
        }
        let i = u.threads.iter().position(|t| t.id == me)?;
        Some(u.threads.remove(i).wake)
    });
    if let Some(wake) = wake {
        g::close_handle(wake);
    }
}

/// Run `f` on the process user state (initialized on first use).
pub(crate) fn with<R>(f: impl FnOnce(&mut User) -> R) -> R {
    let mut guard = USER.lock();
    if guard.is_none() {
        *guard = Some(init());
    }
    f(guard.as_mut().unwrap())
}

impl User {
    /// The calling thread's state and queue, created on first use.
    pub fn thread(&mut self, id: u32) -> &mut ThreadState {
        if let Some(i) = self.threads.iter().position(|t| t.id == id) {
            // A thread that found every queue slot taken retries once an
            // exiting thread freed one.
            if self.queues.thread(id).is_none()
                && let Ok(q) = self.queues.ensure(id, g::now())
            {
                q.wake = self.threads[i].wake;
            }
            return &mut self.threads[i];
        }
        let wake = g::create_event();
        if let Ok(q) = self.queues.ensure(id, g::now()) {
            q.wake = wake;
        }
        self.threads.push(ThreadState {
            id,
            wake,
            focus: None,
            active: None,
            capture: None,
            default_ime: false,
            events: VecDeque::new(),
        });
        self.threads.last_mut().unwrap()
    }
    pub fn wake(&self, thread: u32) {
        if let Some(t) = self.threads.iter().find(|t| t.id == thread) {
            g::set_event(t.wake);
        }
    }
    pub fn window_thread(&self, h: Handle) -> Option<u32> {
        self.windows.get(h).map(|w| w.thread)
    }
    pub fn control(&mut self, h: Handle) -> Option<&mut Control> {
        self.controls
            .iter_mut()
            .find(|(x, _)| *x == h)
            .map(|(_, c)| c)
    }
    pub fn dialog(&mut self, h: Handle) -> Option<&mut DialogState> {
        self.dialogs
            .iter_mut()
            .find(|(x, _)| *x == h)
            .map(|(_, d)| d)
    }
}

/// Resolve an application HWND value.
pub(crate) fn hwnd(value: u64) -> Option<Handle> {
    with(|u| u.windows.resolve(value))
}
/// Resolve or fail with ERROR_INVALID_WINDOW_HANDLE.
pub(crate) fn hwnd_or_error(value: u64) -> Option<Handle> {
    let h = hwnd(value);
    if h.is_none() {
        g::set_last_error(e::INVALID_WINDOW_HANDLE);
    }
    h
}

/// WNDPROC (also DLGPROC and TIMERPROC, which have its shape): HWND, UINT,
/// WPARAM, LPARAM -> LRESULT, at the build's pointer width (P17).
type WndProc = extern "system" fn(usize, u32, usize, isize) -> isize;

/// Call a window procedure. No user32 lock is held by any caller.
pub(crate) fn call(procedure: u64, h: u64, message: u32, wparam: u64, lparam: i64) -> i64 {
    if procedure == 0 {
        return 0;
    }
    // SAFETY: `procedure` is a window procedure address registered by the
    // application (or one of ours) for this window; the application owns its
    // code. It is called on the window's own thread with no lock held.
    let f: WndProc = unsafe { core::mem::transmute(procedure as usize) };
    f(
        door_windows_abi::narrow(h),
        message,
        door_windows_abi::narrow(wparam),
        lparam as isize,
    ) as i64
}

pub(crate) fn tid() -> u32 {
    g::thread_id()
}

/// LOWORD/HIWORD helpers.
pub(crate) const fn makelong(lo: u32, hi: u32) -> u64 {
    ((lo & 0xffff) | ((hi & 0xffff) << 16)) as u64
}
pub(crate) const fn point_lparam(x: i32, y: i32) -> i64 {
    ((x as u16 as u32) | ((y as u16 as u32) << 16)) as i32 as i64
}

type EnumProc = extern "system" fn(usize, isize) -> i32;

/// Call an EnumWindowsProc-style callback (HWND, LPARAM) -> BOOL.
pub(crate) fn call_enum(procedure: u64, h: u64, lparam: i64) -> i32 {
    if procedure == 0 {
        return 0;
    }
    // SAFETY: an application callback address, called on its own thread
    // with no lock held.
    let f: EnumProc = unsafe { core::mem::transmute(procedure as usize) };
    f(door_windows_abi::narrow(h), lparam as isize)
}

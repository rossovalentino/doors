//! Classes and windows: registration, creation and destruction with the
//! Windows message sequences, visibility and geometry (SetWindowPos family),
//! text, window longs, relations, focus and activation, and DefWindowProcW.
use super::structs::{self, class, windowpos};
use super::{
    call, g, hwnd, hwnd_or_error, makelong, menu, msg, paint, point_lparam, server, tid, with,
};
use alloc::vec::Vec;
use door_windows_abi::layout::{self, Field};
use door_windows_gui::handles::Handle;
use door_windows_gui::metrics;
use door_windows_gui::msg::{color, gcl, gw, gwl, ht, sc, size, sw, swp, wa, wm, ws, ws_ex};
use door_windows_gui::window::{Class, ClassRef, Life, Window};
use door_windows_gui::{Point, Rect, win32_error as e};

const HWND_MESSAGE: u64 = (-3i64) as u64;
const CW_USEDEFAULT: i32 = i32::MIN;
const MAX_TEXT: usize = door_windows_gui::window::MAX_TEXT;

fn fail<T: Default>(code: u32) -> T {
    g::set_last_error(code);
    T::default()
}

/// A class argument: an atom (MAKEINTATOM) or a string pointer.
fn class_name(value: u64) -> Option<Result<u16, Vec<u16>>> {
    if value < 0x1_0000 {
        return (value != 0).then_some(Ok(value as u16));
    }
    g::read_wide(value, 256).map(Err)
}

// ---- classes -------------------------------------------------------------------

/// RegisterClass(Ex)W over a WNDCLASSEXW (`ex`) or WNDCLASSW's bytes.
fn register(b: &[u8], ex: bool) -> u16 {
    // WNDCLASSW is WNDCLASSEXW without cbSize (and hIconSm): its fields are
    // one index lower.
    let get = |index: usize| {
        if ex {
            structs::get(b, &structs::WNDCLASSEXW, index)
        } else {
            structs::get(b, &structs::WNDCLASSW, index - 1)
        }
    };
    let style = get(class::STYLE) as u32;
    let procedure = get(class::PROCEDURE);
    let cls_extra = get(class::CLASS_EXTRA) as u32 as i32;
    let wnd_extra = get(class::WINDOW_EXTRA) as u32 as i32;
    let instance = get(class::INSTANCE);
    let name_ptr = get(class::CLASS_NAME);
    if procedure == 0 || !(0..=4096).contains(&cls_extra) || !(0..=40_000).contains(&wnd_extra) {
        return fail(e::INVALID_PARAMETER);
    }
    let name = match class_name(name_ptr) {
        Some(Err(n)) => n,
        _ => return fail(e::INVALID_PARAMETER),
    };
    let class = Class {
        atom: 0,
        name,
        instance,
        global: style & door_windows_gui::msg::cs::GLOBALCLASS != 0,
        system: false,
        style,
        procedure,
        class_extra: alloc::vec![0; cls_extra as usize],
        window_extra: wnd_extra as u32,
        icon: get(class::ICON),
        cursor: get(class::CURSOR),
        background: get(class::BACKGROUND),
        menu_name: get(class::MENU_NAME),
        small_icon: if ex { get(class::SMALL_ICON) } else { 0 },
        windows: 0,
    };
    match with(|u| u.classes.register(class)) {
        Ok(atom) => atom,
        Err(code) => fail(code),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn RegisterClassExW(wc: usize) -> u16 {
    let wc = door_windows_abi::widen(wc);
    {
        let mut b = [0u8; structs::WNDCLASSEXW_BYTES];
        if !g::read(wc, &mut b)
            || structs::get(&b, &structs::WNDCLASSEXW, class::SIZE) != b.len() as u64
        {
            return fail(e::INVALID_PARAMETER);
        }
        register(&b, true)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn RegisterClassW(wc: usize) -> u16 {
    let wc = door_windows_abi::widen(wc);
    {
        let mut b = [0u8; structs::WNDCLASSW_BYTES];
        if !g::read(wc, &mut b) {
            return fail(e::INVALID_PARAMETER);
        }
        register(&b, false)
    }
}

fn find_class(name: u64, instance: u64) -> Result<usize, u32> {
    let r = class_name(name).ok_or(e::INVALID_PARAMETER)?;
    with(|u| match &r {
        Ok(atom) => u.classes.find(ClassRef::Atom(*atom), instance),
        Err(n) => u.classes.find(ClassRef::Name(n), instance),
    })
    .ok_or(e::CLASS_DOES_NOT_EXIST)
}

#[unsafe(no_mangle)]
pub extern "system" fn UnregisterClassW(name: usize, instance: usize) -> i32 {
    let name = door_windows_abi::widen(name);
    let instance = door_windows_abi::widen(instance);
    {
        let Some(r) = class_name(name) else {
            return fail(e::INVALID_PARAMETER);
        };
        let result = with(|u| match &r {
            Ok(atom) => u.classes.unregister(ClassRef::Atom(*atom), instance),
            Err(n) => u.classes.unregister(ClassRef::Name(n), instance),
        });
        match result {
            Ok(()) => 1,
            Err(code) => fail(code),
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetClassInfoExW(instance: usize, name: usize, out: usize) -> i32 {
    let instance = door_windows_abi::widen(instance);
    let name = door_windows_abi::widen(name);
    let out = door_windows_abi::widen(out);
    {
        let i = match find_class(name, instance) {
            Ok(i) => i,
            Err(_) => return fail(e::CLASS_DOES_NOT_EXIST),
        };
        let data = with(|u| u.classes.get(i).cloned());
        let Some(c) = data else {
            return fail(e::CLASS_DOES_NOT_EXIST);
        };
        let mut b = [0u8; structs::WNDCLASSEXW_BYTES];
        // lpszClassName keeps the caller's pointer (or atom), as Windows does.
        for (index, value) in [
            b.len() as u64,
            u64::from(c.style),
            c.procedure,
            c.class_extra.len() as u64,
            u64::from(c.window_extra),
            if c.system { 0 } else { c.instance },
            c.icon,
            c.cursor,
            c.background,
            c.menu_name,
            name,
            c.small_icon,
        ]
        .into_iter()
        .enumerate()
        {
            structs::set(&mut b, &structs::WNDCLASSEXW, index, value);
        }
        if !g::write(out, &b) {
            return fail(e::INVALID_PARAMETER);
        }
        i32::from(c.atom)
    }
}

fn class_of(h: Handle) -> Option<Class> {
    with(|u| {
        let w = u.windows.get(h)?;
        let i = u.classes.by_atom(w.class)?;
        u.classes.get(i).cloned()
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn GetClassLongPtrW(hwnd_value: usize, index: i32) -> usize {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    door_windows_abi::narrow((move || -> u64 {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        let Some(c) = class_of(h) else { return 0 };
        match index {
            gcl::STYLE => u64::from(c.style),
            gcl::WNDPROC => c.procedure,
            gcl::HCURSOR => c.cursor,
            gcl::HBRBACKGROUND => c.background,
            gcl::HICON => c.icon,
            gcl::HICONSM => c.small_icon,
            gcl::HMODULE => c.instance,
            gcl::MENUNAME => c.menu_name,
            gcl::CBWNDEXTRA => u64::from(c.window_extra),
            gcl::CBCLSEXTRA => c.class_extra.len() as u64,
            gcl::ATOM => u64::from(c.atom),
            i if i >= 0 && (i as usize) + structs::LONG_PTR <= c.class_extra.len() => {
                structs::get(&c.class_extra[i as usize..], &[Field::Pointer], 0)
            }
            _ => fail(e::INVALID_INDEX),
        }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetClassLongW(hwnd_value: usize, index: i32) -> u32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    GetClassLongPtrW(door_windows_abi::narrow(hwnd_value), index) as u32
}
#[unsafe(no_mangle)]
pub extern "system" fn SetClassLongPtrW(hwnd_value: usize, index: i32, value: usize) -> usize {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    let value = door_windows_abi::widen(value);
    door_windows_abi::narrow((move || -> u64 {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        with(|u| {
            let atom = u.windows.get(h)?.class;
            let i = u.classes.by_atom(atom)?;
            let c = u.classes.get_mut(i)?;
            let slot = match index {
                gcl::STYLE => {
                    let old = u64::from(c.style);
                    c.style = value as u32;
                    return Some(old);
                }
                gcl::WNDPROC => &mut c.procedure,
                gcl::HCURSOR => &mut c.cursor,
                gcl::HBRBACKGROUND => &mut c.background,
                gcl::HICON => &mut c.icon,
                _ => return None,
            };
            Some(core::mem::replace(slot, value))
        })
        .unwrap_or_else(|| fail(e::INVALID_INDEX))
    })())
}

#[unsafe(no_mangle)]
pub extern "system" fn GetClassNameW(hwnd_value: usize, out: usize, capacity: i32) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    let out = door_windows_abi::widen(out);
    {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        let Some(c) = class_of(h) else { return 0 };
        g::write_wide(out, capacity.max(0) as usize, &c.name).map_or(0, |n| n as i32)
    }
}

// ---- creation ----------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn create_struct(
    param: u64,
    instance: u64,
    menu: u64,
    parent: u64,
    r: Rect,
    style: u32,
    name: u64,
    class: u64,
    ex: u32,
) -> [u8; structs::CREATESTRUCTW_BYTES] {
    let mut b = [0u8; structs::CREATESTRUCTW_BYTES];
    for (index, value) in [
        param,
        instance,
        menu,
        parent,
        u64::from(r.height() as u32),
        u64::from(r.width() as u32),
        u64::from(r.top as u32),
        u64::from(r.left as u32),
        u64::from(style),
        name,
        class,
        u64::from(ex),
    ]
    .into_iter()
    .enumerate()
    {
        structs::set(&mut b, &structs::CREATESTRUCTW, index, value);
    }
    b
}

/// WM_GETMINMAXINFO with the defaults pre-filled: (min track, max track).
pub(crate) fn min_max(h: Handle) -> ((i32, i32), (i32, i32)) {
    let (sw_, sh) = with(|u| u.screen);
    let mut b = [0i32; 10];
    b[2] = sw_;
    b[3] = sh;
    b[6] = metrics::system_metric(34, (sw_, sh), Rect::EMPTY);
    b[7] = metrics::system_metric(35, (sw_, sh), Rect::EMPTY);
    b[8] = sw_ + 12;
    b[9] = sh + 12;
    msg::send(h.value(), wm::GETMINMAXINFO, 0, b.as_mut_ptr() as i64);
    with(|u| {
        u.limits
            .retain(|l| l.0 != h && u.windows.get(l.0).is_some());
        u.limits.push((h, (b[6], b[7]), (b[8], b[9])));
    });
    ((b[6], b[7]), (b[8], b[9]))
}

#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn CreateWindowExW(
    ex_style: u32,
    class: usize,
    name: usize,
    style: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    parent: usize,
    menu_or_id: usize,
    instance: usize,
    param: usize,
) -> usize {
    let class = door_windows_abi::widen(class);
    let name = door_windows_abi::widen(name);
    let parent = door_windows_abi::widen(parent);
    let menu_or_id = door_windows_abi::widen(menu_or_id);
    let instance = door_windows_abi::widen(instance);
    let param = door_windows_abi::widen(param);
    door_windows_abi::narrow((move || -> u64 {
        let me = tid();
        let Some(class_ref) = class_name(class) else {
            return fail(e::CANNOT_FIND_WND_CLASS);
        };
        // Windows' rule (as Wine documents it): a dialog or sizing frame
        // gets WS_EX_WINDOWEDGE, anything else loses it; measured, an
        // overlapped window's CREATESTRUCT shows 0x100 (x86gui.c).
        let ex_style = if ex_style & ws_ex::DLGMODALFRAME != 0
            || (ex_style & ws_ex::STATICEDGE == 0 && style & (ws::DLGFRAME | ws::THICKFRAME) != 0)
        {
            ex_style | ws_ex::WINDOWEDGE
        } else {
            ex_style & !ws_ex::WINDOWEDGE
        };
        // CREATESTRUCT's lpszClass is user32's own copy of a class name
        // (measured: not the caller's pointer); an atom stays an atom.
        let class_copy: Option<Vec<u16>> = match &class_ref {
            Err(n) => Some(n.iter().copied().chain([0]).collect()),
            Ok(_) => None,
        };
        let class_field = class_copy.as_ref().map_or(class, |c| c.as_ptr() as u64);
        let text = if name >= 0x1_0000 {
            match g::read_wide(name, MAX_TEXT) {
                Some(t) => t,
                None => return fail(e::INVALID_PARAMETER),
            }
        } else {
            Vec::new()
        };
        let _ = text;
        let child = style & ws::CHILD != 0;
        let prepared = with(|u| {
            u.thread(me);
            // Every message-queue slot is held by a live thread (exiting threads
            // free theirs): no window rather than one no queue can serve.
            if u.queues.thread(me).is_none() {
                return Err(e::NOT_ENOUGH_QUOTA);
            }
            let ci = match &class_ref {
                Ok(a) => u.classes.find(ClassRef::Atom(*a), instance),
                Err(n) => u.classes.find(ClassRef::Name(n), instance),
            }
            .ok_or(e::CANNOT_FIND_WND_CLASS)?;
            let (parent_h, owner, message_only) = if parent == HWND_MESSAGE {
                (None, None, true)
            } else if child {
                if parent == 0 {
                    return Err(e::TLW_WITH_WSCHILD);
                }
                (
                    Some(u.windows.resolve(parent).ok_or(e::INVALID_WINDOW_HANDLE)?),
                    None,
                    false,
                )
            } else if parent != 0 {
                let p = u.windows.resolve(parent).ok_or(e::INVALID_WINDOW_HANDLE)?;
                (None, Some(u.windows.root(p)), false)
            } else {
                (None, None, false)
            };
            let menu_value = if !child && menu_or_id != 0 {
                let m = u.menus.resolve(menu_or_id).ok_or(e::INVALID_MENU_HANDLE)?;
                m.value()
            } else {
                menu_or_id
            };
            let c = u.classes.get(ci).unwrap().clone();
            // Default geometry for top-level windows.
            let count = u.windows.top.len() as i32;
            let (sw_, sh) = u.screen;
            let (mut x, mut y, mut w, mut hh) = (x, y, width, height);
            if !child {
                if x == CW_USEDEFAULT {
                    x = 64 + 32 * (count % 8);
                    y = 48 + 32 * (count % 8);
                }
                if w == CW_USEDEFAULT {
                    w = sw_ * 3 / 5;
                    hh = sh * 3 / 5;
                }
            } else {
                if x == CW_USEDEFAULT {
                    x = 0;
                    y = 0;
                }
                if w == CW_USEDEFAULT {
                    w = 0;
                    hh = 0;
                }
            }
            let mut win = Window::new(c.atom, style & !ws::VISIBLE, ex_style, me);
            win.parent = parent_h;
            win.owner = owner;
            win.message_only = message_only;
            win.rect = Rect::from_size(x, y, w.max(0), hh.max(0));
            win.id = menu_value;
            win.instance = instance;
            win.procedure = c.procedure;
            win.extra = alloc::vec![0; c.window_extra as usize];
            win.need_size = !child;
            let h = u.windows.insert(win)?;
            u.classes.get_mut(ci).unwrap().windows += 1;
            Ok((h, menu_value, parent_h, win_rect(u, h)))
        });
        let (h, menu_value, parent_h, rect) = match prepared {
            Ok(v) => v,
            Err(code) => return fail(code),
        };
        let hv = h.value();
        if style & ws::THICKFRAME != 0 || style & (ws::POPUP | ws::CHILD) == 0 {
            min_max(h);
        }
        let parent_value = parent_h.map_or(parent, Handle::value);
        let cs = create_struct(
            param,
            instance,
            menu_value,
            parent_value,
            rect,
            style,
            name,
            class_field,
            ex_style,
        );
        // Windows creates the thread's default IME window with its first
        // top-level window, which leaves the last error 0: the reference reads 0
        // after the first window's WM_NCCREATE rejection (wingui-hang --reject)
        // and the caller's value after a later one (wingui-selftest probes).
        if !child && with(|u| !core::mem::replace(&mut u.thread(me).default_ime, true)) {
            g::set_last_error(0);
        }
        if msg::send(hv, wm::NCCREATE, 0, cs.as_ptr() as i64) == 0 {
            msg::send(hv, wm::NCDESTROY, 0, 0);
            free(h);
            return 0;
        }
        // Client area from WM_NCCALCSIZE on the window rectangle.
        let mut r = [rect.left, rect.top, rect.right, rect.bottom];
        msg::send(hv, wm::NCCALCSIZE, 0, r.as_mut_ptr() as i64);
        with(|u| {
            if let Some(w) = u.windows.get_mut(h) {
                let c = Rect::new(
                    r[0] - rect.left,
                    r[1] - rect.top,
                    r[2] - rect.left,
                    r[3] - rect.top,
                );
                w.client = Rect::new(c.left, c.top, c.right.max(c.left), c.bottom.max(c.top));
            }
        });
        if msg::send(hv, wm::CREATE, 0, cs.as_ptr() as i64) == -1 {
            destroy_tree(h, false);
            g::set_last_error(0);
            return 0;
        }
        if !with(|u| u.windows.get(h).is_some()) {
            return 0;
        }
        with(|u| {
            if let Some(w) = u.windows.get_mut(h) {
                w.life = Life::Live;
            }
        });
        if child && ex_style & ws_ex::NOPARENTNOTIFY == 0 {
            parent_notify(h, wm::CREATE);
        }
        if child {
            send_size_move(h);
        }
        if style & ws::VISIBLE != 0 {
            show(h, sw::SHOW);
            // A visible top-level window the window service refused (its
            // per-process quota): creation fails instead of returning a window
            // that is not on screen.
            if with(|u| u.windows.get(h).is_some()) && !server::has_native(h) {
                destroy_tree(h, false);
                g::set_last_error(e::NOT_ENOUGH_QUOTA);
                return 0;
            }
        }
        if with(|u| u.windows.get(h).is_some()) {
            hv
        } else {
            0
        }
    })())
}

/// The window rectangle in parent-client (or screen) coordinates.
fn win_rect(u: &super::User, h: Handle) -> Rect {
    u.windows.get(h).map_or(Rect::EMPTY, |w| w.rect)
}

fn parent_notify(h: Handle, event: u32) {
    let t = with(|u| {
        let w = u.windows.get(h)?;
        Some((w.parent?, w.id as u32))
    });
    if let Some((p, id)) = t {
        msg::send(
            p.value(),
            wm::PARENTNOTIFY,
            makelong(event, id),
            h.value() as i64,
        );
    }
}

pub(crate) fn send_size_move(h: Handle) {
    let t = with(|u| {
        let w = u.windows.get_mut(h)?;
        w.need_size = false;
        let (cw, ch) = w.client_size();
        let origin = Point::new(w.rect.left + w.client.left, w.rect.top + w.client.top);
        Some((cw, ch, origin))
    });
    if let Some((cw, ch, o)) = t {
        msg::send(
            h.value(),
            wm::SIZE,
            size::RESTORED as u64,
            point_lparam(cw, ch),
        );
        msg::send(h.value(), wm::MOVE, 0, point_lparam(o.x, o.y));
    }
}

// ---- destruction ------------------------------------------------------------------

/// Release one window's model and resources.
fn free(h: Handle) {
    server::destroy_native(h);
    super::system::window_destroyed(h);
    let menu = with(|u| {
        let w = u.windows.remove(h)?;
        if let Some(i) = u.classes.by_atom(w.class)
            && let Some(c) = u.classes.get_mut(i)
        {
            c.windows = c.windows.saturating_sub(1);
        }
        u.queues.forget_window(h.value());
        u.controls.retain(|(x, _)| *x != h);
        u.dialogs.retain(|(x, _)| *x != h);
        for t in &mut u.threads {
            for slot in [&mut t.focus, &mut t.active, &mut t.capture] {
                if *slot == Some(h) {
                    *slot = None;
                }
            }
        }
        if u.foreground == Some(h) {
            u.foreground = None;
        }
        (!w.is_child() && w.id != 0).then_some(w.id)
    });
    if let Some(m) = menu {
        with(|u| {
            if let Some(mh) = u.menus.resolve(m) {
                let _ = u.menus.destroy(mh);
            }
        });
    }
}

/// DLL_THREAD_DETACH: a terminating thread's windows (with the subtrees
/// under them) are freed without messages, children before parents, as
/// Windows destroys them in its thread cleanup.
pub(crate) fn free_thread_windows(me: u32) {
    let doomed: Vec<Handle> = with(|u| {
        let mut out: Vec<Handle> = Vec::new();
        let roots: Vec<Handle> = u
            .windows
            .iter()
            .filter(|(_, w)| {
                w.thread == me
                    && w.parent
                        .and_then(|p| u.windows.get(p))
                        .is_none_or(|p| p.thread != me)
            })
            .map(|(h, _)| h)
            .collect();
        for root in roots {
            for x in u.windows.subtree(root) {
                if !out.contains(&x) {
                    out.push(x);
                }
            }
        }
        out
    });
    for h in doomed.into_iter().rev() {
        if hwnd(h.value()).is_some() {
            free(h);
        }
    }
}

fn send_destroy(h: Handle) {
    msg::send(h.value(), wm::DESTROY, 0, 0);
    let children = with(|u| u.windows.get(h).map(|w| w.children.clone())).unwrap_or_default();
    for c in children {
        if hwnd(c.value()).is_some() {
            send_destroy(c);
        }
    }
}
fn send_ncdestroy(h: Handle) {
    let children = with(|u| u.windows.get(h).map(|w| w.children.clone())).unwrap_or_default();
    for c in children {
        if hwnd(c.value()).is_some() {
            send_ncdestroy(c);
        }
    }
    msg::send(h.value(), wm::NCDESTROY, 0, 0);
    free(h);
}

/// WM_DESTROY down the tree, then WM_NCDESTROY up it, freeing each window.
fn destroy_tree(h: Handle, hide: bool) {
    let owned: Vec<Handle> = with(|u| {
        for x in u.windows.subtree(h) {
            if let Some(w) = u.windows.get_mut(x) {
                w.life = Life::Destroying;
            }
        }
        u.windows
            .iter()
            .filter(|(_, w)| w.owner == Some(h) && w.life != Life::Destroying)
            .map(|(x, _)| x)
            .collect()
    });
    for o in owned {
        if hwnd(o.value()).is_some() {
            destroy_tree(o, true);
        }
    }
    if hide && with(|u| u.windows.get(h).is_some_and(|w| w.visible())) {
        set_window_pos(
            h,
            0,
            0,
            0,
            0,
            swp::HIDEWINDOW | swp::NOMOVE | swp::NOSIZE | swp::NOZORDER | swp::NOACTIVATE,
        );
    }
    send_destroy(h);
    send_ncdestroy(h);
    super::icon::forget_window(h.value());
    super::notify::forget_window(h.value());
}

#[unsafe(no_mangle)]
pub extern "system" fn DestroyWindow(hwnd_value: usize) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    {
        let h = match msg::own_window(hwnd_value) {
            Ok(h) => h,
            Err(code) => return fail(code),
        };
        let Some((life, child, notify)) = with(|u| {
            let w = u.windows.get(h)?;
            Some((
                w.life,
                w.is_child(),
                w.ex_style & ws_ex::NOPARENTNOTIFY == 0,
            ))
        }) else {
            return fail(e::INVALID_WINDOW_HANDLE);
        };
        if life == Life::Destroying {
            return 1;
        }
        if child && notify {
            let parent_alive = with(|u| {
                u.windows
                    .get(h)
                    .and_then(|w| w.parent)
                    .and_then(|p| u.windows.get(p))
                    .is_some_and(|p| p.life != Life::Destroying)
            });
            if parent_alive {
                parent_notify(h, wm::DESTROY);
            }
        }
        let parent = with(|u| u.windows.get(h).and_then(|w| w.parent));
        let area = with(|u| u.windows.get(h).map(|w| w.rect));
        destroy_tree(h, true);
        if let (Some(p), Some(r)) = (parent, area) {
            invalidate_in_parent(p, r);
        }
        1
    }
}

fn invalidate_in_parent(p: Handle, r: Rect) {
    with(|u| {
        if let Some(pw) = u.windows.get(p) {
            let c = pw.client;
            let _ = c;
            u.windows.invalidate(p, Some(r), true);
            if let Some(t) = u.window_thread(p) {
                u.wake(t);
            }
        }
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn IsWindow(hwnd_value: usize) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    i32::from(hwnd(hwnd_value).is_some())
}
#[unsafe(no_mangle)]
pub extern "system" fn IsWindowVisible(hwnd_value: usize) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    hwnd(hwnd_value).map_or(0, |h| i32::from(with(|u| u.windows.really_visible(h))))
}
#[unsafe(no_mangle)]
pub extern "system" fn IsWindowEnabled(hwnd_value: usize) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    {
        hwnd(hwnd_value).map_or(0, |h| {
            i32::from(with(|u| u.windows.get(h).is_some_and(|w| w.enabled())))
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn IsChild(parent: usize, child: usize) -> i32 {
    let parent = door_windows_abi::widen(parent);
    let child = door_windows_abi::widen(child);
    {
        match (hwnd(parent), hwnd(child)) {
            (Some(p), Some(c)) => i32::from(with(|u| u.windows.is_descendant(p, c))),
            _ => 0,
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn EnableWindow(hwnd_value: usize, enable: i32) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        let me = tid();
        // Resolve and update under one lock: another thread may destroy the
        // window between a separate lookup and this update.
        let Some((was_disabled, top)) = with(|u| {
            let w = u.windows.get_mut(h)?;
            let was = !w.enabled();
            if enable != 0 {
                w.style &= !ws::DISABLED;
            } else {
                w.style |= ws::DISABLED;
            }
            Some((was, w.parent.is_none()))
        }) else {
            g::set_last_error(e::INVALID_WINDOW_HANDLE);
            return 0;
        };
        if was_disabled == (enable == 0) {
            return i32::from(was_disabled);
        }
        if enable == 0 {
            let lose = with(|u| {
                u.thread(me)
                    .focus
                    .filter(|f| *f == h || u.windows.is_descendant(h, *f))
            });
            if lose.is_some() {
                set_focus(None);
            }
        }
        msg::send(hwnd_value, wm::ENABLE, u64::from(enable != 0), 0);
        if top {
            server::set_enabled(h, enable != 0);
        }
        i32::from(was_disabled)
    }
}

// ---- visibility and geometry --------------------------------------------------------

/// Apply new geometry from the server or SetWindowPos: client from
/// WM_NCCALCSIZE, WM_WINDOWPOSCHANGED (whose default sends WM_MOVE/WM_SIZE).
pub(crate) fn apply_geometry(
    h: Handle,
    rect: Rect,
    no_move: bool,
    no_size: bool,
    configure_server: bool,
) {
    let hv = h.value();
    let old = with(|u| u.windows.get(h).map(|w| (w.rect, w.parent)));
    let Some((old_rect, parent)) = old else {
        return;
    };
    with(|u| {
        if let Some(w) = u.windows.get_mut(h) {
            w.rect = rect;
        }
    });
    if !no_size {
        let mut p = [0i32; 16];
        p[..4].copy_from_slice(&[rect.left, rect.top, rect.right, rect.bottom]);
        p[4..8].copy_from_slice(&[old_rect.left, old_rect.top, old_rect.right, old_rect.bottom]);
        msg::send(hv, wm::NCCALCSIZE, 1, p.as_mut_ptr() as i64);
        with(|u| {
            if let Some(w) = u.windows.get_mut(h) {
                let c = Rect::new(
                    p[0] - rect.left,
                    p[1] - rect.top,
                    p[2] - rect.left,
                    p[3] - rect.top,
                );
                w.client = Rect::new(c.left, c.top, c.right.max(c.left), c.bottom.max(c.top));
            }
            u.windows.invalidate(h, None, true);
            if let Some(w) = u.windows.get_mut(h) {
                w.frame_dirty = true;
            }
        });
    }
    if let Some(p) = parent {
        invalidate_in_parent(p, old_rect);
        invalidate_in_parent(p, rect);
    }
    let mut wp = [0u8; structs::WINDOWPOS_BYTES];
    structs::set(&mut wp, &structs::WINDOWPOS, windowpos::HWND, hv);
    for (i, v) in [rect.left, rect.top, rect.width(), rect.height()]
        .iter()
        .enumerate()
    {
        structs::set_i32(&mut wp, &structs::WINDOWPOS, windowpos::X + i, *v);
    }
    let flags = swp::NOZORDER
        | swp::NOACTIVATE
        | if no_move { swp::NOMOVE } else { 0 }
        | if no_size { swp::NOSIZE } else { 0 };
    structs::set(
        &mut wp,
        &structs::WINDOWPOS,
        windowpos::FLAGS,
        u64::from(flags),
    );
    msg::send(hv, wm::WINDOWPOSCHANGED, 0, wp.as_ptr() as i64);
    if configure_server {
        server::configure(h);
    }
}

/// SetWindowPos core (no z-order beyond top/bottom; z-order of native
/// windows belongs to the server).
pub(crate) fn set_window_pos(h: Handle, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> bool {
    let hv = h.value();
    let Some((rect, visible, top, style)) = with(|u| {
        u.windows.get(h).map(|w| {
            (
                w.rect,
                w.visible(),
                w.parent.is_none() && !w.message_only,
                w.style,
            )
        })
    }) else {
        return false;
    };
    let mut wp = [0u8; structs::WINDOWPOS_BYTES];
    structs::set(&mut wp, &structs::WINDOWPOS, windowpos::HWND, hv);
    let (nx, ny) = if flags & swp::NOMOVE != 0 {
        (rect.left, rect.top)
    } else {
        (x, y)
    };
    // The requested size: DefWindowProc's WM_WINDOWPOSCHANGING applies the
    // tracking limits (the window procedure sees the size asked for first,
    // as on Windows: measured with x86gui.exe).
    let (ncx, ncy) = if flags & swp::NOSIZE != 0 {
        (rect.width(), rect.height())
    } else {
        (cx.max(0), cy.max(0))
    };
    for (i, v) in [nx, ny, ncx, ncy].iter().enumerate() {
        structs::set_i32(&mut wp, &structs::WINDOWPOS, windowpos::X + i, *v);
    }
    structs::set(
        &mut wp,
        &structs::WINDOWPOS,
        windowpos::FLAGS,
        u64::from(flags),
    );
    if flags & swp::NOSENDCHANGING == 0 {
        msg::send(hv, wm::WINDOWPOSCHANGING, 0, wp.as_mut_ptr() as i64);
    }
    let field = |index| structs::get_i32(&wp, &structs::WINDOWPOS, index);
    let new = Rect::from_size(
        field(windowpos::X),
        field(windowpos::Y),
        field(windowpos::CX),
        field(windowpos::CY),
    );
    let moved = flags & swp::NOMOVE == 0 && new.top_left() != rect.top_left();
    let sized = flags & swp::NOSIZE == 0 && new.size() != rect.size();
    let showing = flags & swp::SHOWWINDOW != 0 && !visible;
    let hiding = flags & swp::HIDEWINDOW != 0 && visible;
    with(|u| {
        if let Some(w) = u.windows.get_mut(h) {
            w.rect = new;
            if showing {
                w.style |= ws::VISIBLE;
            }
            if hiding {
                w.style &= !ws::VISIBLE;
            }
        }
    });
    if sized {
        let mut p = [0i32; 16];
        p[..4].copy_from_slice(&[new.left, new.top, new.right, new.bottom]);
        p[4..8].copy_from_slice(&[rect.left, rect.top, rect.right, rect.bottom]);
        msg::send(hv, wm::NCCALCSIZE, 1, p.as_mut_ptr() as i64);
        with(|u| {
            if let Some(w) = u.windows.get_mut(h) {
                let c = Rect::new(
                    p[0] - new.left,
                    p[1] - new.top,
                    p[2] - new.left,
                    p[3] - new.top,
                );
                w.client = Rect::new(c.left, c.top, c.right.max(c.left), c.bottom.max(c.top));
            }
        });
    }
    if showing || sized || flags & swp::FRAMECHANGED != 0 {
        with(|u| {
            u.windows.invalidate(h, None, true);
            if let Some(w) = u.windows.get_mut(h) {
                w.frame_dirty = true;
            }
            for c in u.windows.subtree(h).into_iter().skip(1) {
                if let Some(cw) = u.windows.get_mut(c) {
                    cw.frame_dirty = true;
                }
            }
        });
    }
    let parent = with(|u| u.windows.get(h).and_then(|w| w.parent));
    if let Some(p) = parent
        && (moved || sized || showing || hiding)
    {
        invalidate_in_parent(p, rect);
        invalidate_in_parent(p, new);
    }
    if top {
        if showing && !server::create_native(h) {
            // The window service refused the window (per-process quota):
            // it stays hidden instead of pretending to be on screen.
            with(|u| {
                if let Some(w) = u.windows.get_mut(h) {
                    w.style &= !ws::VISIBLE;
                }
            });
            g::set_last_error(e::NOT_ENOUGH_QUOTA);
        } else if showing {
            let cached = with(|u| u.limits.iter().find(|l| l.0 == h).map(|l| (l.1, l.2)));
            if let Some(((minx, miny), _)) = cached.filter(|_| style & ws::THICKFRAME != 0) {
                let f = if style & ws::CAPTION == ws::CAPTION {
                    metrics::frame(style)
                } else {
                    0
                };
                server::set_limits(
                    h,
                    ((minx - 2 * f).max(1) as u32, (miny - f).max(1) as u32),
                    (0, 0),
                );
            }
            if flags & swp::NOACTIVATE == 0 {
                server::activate(h);
            }
        } else if hiding {
            server::destroy_native(h);
        } else if moved || sized {
            server::configure(h);
        }
    }
    let mut changed = flags & !(swp::NOMOVE | swp::NOSIZE);
    if !moved {
        changed |= swp::NOMOVE;
    }
    if !sized {
        changed |= swp::NOSIZE;
    }
    structs::set(
        &mut wp,
        &structs::WINDOWPOS,
        windowpos::FLAGS,
        u64::from(changed),
    );
    msg::send(hv, wm::WINDOWPOSCHANGED, 0, wp.as_ptr() as i64);
    if let Some(t) = with(|u| u.window_thread(h)) {
        with(|u| u.wake(t));
    }
    true
}

pub(crate) fn show(h: Handle, cmd: i32) -> bool {
    let hv = h.value();
    let Some((visible, need_size, top)) = with(|u| {
        u.windows.get(h).map(|w| {
            (
                w.visible(),
                w.need_size,
                w.parent.is_none() && !w.message_only,
            )
        })
    }) else {
        return false;
    };
    match cmd {
        sw::HIDE => {
            if visible {
                msg::send(hv, wm::SHOWWINDOW, 0, 0);
                set_window_pos(
                    h,
                    0,
                    0,
                    0,
                    0,
                    swp::HIDEWINDOW | swp::NOMOVE | swp::NOSIZE | swp::NOZORDER | swp::NOACTIVATE,
                );
            }
        }
        sw::MINIMIZE | sw::SHOWMINIMIZED | sw::SHOWMINNOACTIVE | sw::FORCEMINIMIZE => {
            if !visible {
                show(h, sw::SHOWNOACTIVATE);
            }
            if top {
                server::request_state(h, 1);
            }
        }
        sw::SHOWMAXIMIZED => {
            if !visible {
                show(h, sw::SHOWNORMAL);
            }
            if top {
                server::request_state(h, 2);
            }
        }
        _ => {
            if !visible {
                msg::send(hv, wm::SHOWWINDOW, 1, 0);
                let activate = !matches!(cmd, sw::SHOWNOACTIVATE | sw::SHOWNA) && top;
                set_window_pos(
                    h,
                    0,
                    0,
                    0,
                    0,
                    swp::SHOWWINDOW
                        | swp::NOMOVE
                        | swp::NOSIZE
                        | swp::NOZORDER
                        | if activate { 0 } else { swp::NOACTIVATE },
                );
                if need_size {
                    send_size_move(h);
                }
            } else if cmd == sw::RESTORE && top {
                server::request_state(h, 0);
            }
        }
    }
    visible
}

#[unsafe(no_mangle)]
pub extern "system" fn ShowWindow(hwnd_value: usize, cmd: i32) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        i32::from(show(h, cmd))
    }
}

#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn SetWindowPos(
    hwnd_value: usize,
    _after: usize,
    x: i32,
    y: i32,
    cx: i32,
    cy: i32,
    flags: u32,
) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        if flags & !swp::VALID != 0 {
            return fail(e::INVALID_FLAGS);
        }
        let visible = with(|u| u.windows.get(h).is_some_and(|w| w.visible()));
        if flags & swp::SHOWWINDOW != 0 && !visible {
            msg::send(hwnd_value, wm::SHOWWINDOW, 1, 0);
        }
        if flags & swp::HIDEWINDOW != 0 && visible {
            msg::send(hwnd_value, wm::SHOWWINDOW, 0, 0);
        }
        i32::from(set_window_pos(h, x, y, cx, cy, flags))
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn MoveWindow(
    hwnd_value: usize,
    x: i32,
    y: i32,
    cx: i32,
    cy: i32,
    repaint: i32,
) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        let flags = swp::NOZORDER | swp::NOACTIVATE | if repaint == 0 { swp::NOREDRAW } else { 0 };
        i32::from(set_window_pos(h, x, y, cx, cy, flags))
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetWindowRect(hwnd_value: usize, out: usize) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    let out = door_windows_abi::widen(out);
    {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        let r = with(|u| u.windows.screen_rect(h));
        i32::from(g::write_i32s(out, &[r.left, r.top, r.right, r.bottom]))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetClientRect(hwnd_value: usize, out: usize) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    let out = door_windows_abi::widen(out);
    {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        let (w, hh) = with(|u| u.windows.get(h).map(|w| w.client_size())).unwrap_or((0, 0));
        i32::from(g::write_i32s(out, &[0, 0, w, hh]))
    }
}
fn map_point(h: Handle, p: u64, to_screen: bool) -> i32 {
    let Some([x, y]) = g::read_i32s::<2>(p) else {
        return fail(e::INVALID_PARAMETER);
    };
    let o = with(|u| u.windows.client_origin(h));
    let (nx, ny) = if to_screen {
        (x + o.x, y + o.y)
    } else {
        (x - o.x, y - o.y)
    };
    i32::from(g::write_i32s(p, &[nx, ny]))
}
#[unsafe(no_mangle)]
pub extern "system" fn ClientToScreen(hwnd_value: usize, point: usize) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    let point = door_windows_abi::widen(point);
    {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        map_point(h, point, true)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ScreenToClient(hwnd_value: usize, point: usize) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    let point = door_windows_abi::widen(point);
    {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        map_point(h, point, false)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MapWindowPoints(from: usize, to: usize, points: usize, count: u32) -> i32 {
    let from = door_windows_abi::widen(from);
    let to = door_windows_abi::widen(to);
    let points = door_windows_abi::widen(points);
    {
        let origin = |v: u64| -> Option<Point> {
            if v == 0 {
                Some(Point::default())
            } else {
                hwnd(v).map(|h| with(|u| u.windows.client_origin(h)))
            }
        };
        let (Some(a), Some(b)) = (origin(from), origin(to)) else {
            return fail(e::INVALID_WINDOW_HANDLE);
        };
        if count > 4096 {
            return fail(e::INVALID_PARAMETER);
        }
        let Some(bytes) = g::read_bytes(points, count as usize * 8) else {
            return fail(e::INVALID_PARAMETER);
        };
        let (dx, dy) = (a.x - b.x, a.y - b.y);
        let out: Vec<u8> = bytes
            .chunks_exact(4)
            .enumerate()
            .flat_map(|(i, c)| {
                let v =
                    i32::from_le_bytes(c.try_into().unwrap()) + if i % 2 == 0 { dx } else { dy };
                v.to_le_bytes()
            })
            .collect();
        if !g::write(points, &out) {
            return fail(e::INVALID_PARAMETER);
        }
        ((dx as u16 as u32) | ((dy as u16 as u32) << 16)) as i32
    }
}

// ---- text ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn GetWindowTextW(hwnd_value: usize, out: usize, capacity: i32) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    let out = door_windows_abi::widen(out);
    {
        // An invalid window returns 0 without touching the last error.
        if hwnd(hwnd_value).is_none() || capacity <= 0 {
            return 0;
        }
        msg::send(hwnd_value, wm::GETTEXT, capacity as u64, out as i64) as i32
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetWindowTextLengthW(hwnd_value: usize) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    {
        if hwnd_or_error(hwnd_value).is_none() {
            return 0;
        }
        msg::send(hwnd_value, wm::GETTEXTLENGTH, 0, 0) as i32
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetWindowTextW(hwnd_value: usize, text: usize) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    let text = door_windows_abi::widen(text);
    {
        if hwnd(hwnd_value).is_none() {
            // Measured on Windows 11: TRUE with ERROR_INVALID_WINDOW_HANDLE.
            g::set_last_error(e::INVALID_WINDOW_HANDLE);
            return 1;
        }
        i32::from(msg::send(hwnd_value, wm::SETTEXT, 0, text as i64) != 0)
    }
}
/// The stored title, never sending a message (works for hung windows).
#[unsafe(no_mangle)]
pub extern "system" fn InternalGetWindowText(hwnd_value: usize, out: usize, capacity: i32) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    let out = door_windows_abi::widen(out);
    {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        let text = with(|u| u.windows.get(h).map(|w| w.text.clone())).unwrap_or_default();
        g::write_wide(out, capacity.max(0) as usize, &text).map_or(0, |n| n as i32)
    }
}
pub(crate) fn set_text(h: Handle, text: Vec<u16>) {
    let top = with(|u| {
        let w = u.windows.get_mut(h)?;
        w.text = text;
        w.frame_dirty = true;
        Some(w.parent.is_none())
    });
    if top == Some(true) {
        server::set_title(h);
    }
}
pub(crate) fn text_of(h: Handle) -> Vec<u16> {
    with(|u| u.windows.get(h).map(|w| w.text.clone())).unwrap_or_default()
}

// ---- longs and relations ------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn GetWindowLongPtrW(hwnd_value: usize, index: i32) -> usize {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    door_windows_abi::narrow((move || -> u64 {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        let r = with(|u| {
            let w = u.windows.get(h)?;
            Some(match index {
                gwl::STYLE => u64::from(w.style),
                gwl::EXSTYLE => u64::from(w.ex_style),
                gwl::WNDPROC => w.procedure,
                gwl::HINSTANCE => w.instance,
                gwl::ID => w.id,
                gwl::USERDATA => w.user_data,
                gwl::HWNDPARENT => w.parent.or(w.owner).map_or(0, Handle::value),
                i if i >= 0 && (i as usize) + structs::LONG_PTR <= w.extra.len() => {
                    structs::get(&w.extra[i as usize..], &[Field::Pointer], 0)
                }
                i if i >= 0 && (i as usize) + 4 <= w.extra.len() => u64::from(u32::from_le_bytes(
                    w.extra[i as usize..i as usize + 4].try_into().unwrap(),
                )),
                _ => return None,
            })
        });
        r.unwrap_or_else(|| fail(e::INVALID_INDEX))
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn SetWindowLongPtrW(hwnd_value: usize, index: i32, value: usize) -> usize {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    let value = door_windows_abi::widen(value);
    door_windows_abi::narrow((move || -> u64 {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        let r = with(|u| {
            let w = u.windows.get_mut(h)?;
            let old = match index {
                gwl::STYLE => u64::from(core::mem::replace(&mut w.style, value as u32)),
                gwl::EXSTYLE => u64::from(core::mem::replace(&mut w.ex_style, value as u32)),
                gwl::WNDPROC => core::mem::replace(&mut w.procedure, value),
                gwl::HINSTANCE => core::mem::replace(&mut w.instance, value),
                gwl::ID => core::mem::replace(&mut w.id, value),
                gwl::USERDATA => core::mem::replace(&mut w.user_data, value),
                i if i >= 0 && (i as usize) + structs::LONG_PTR <= w.extra.len() => {
                    let slot = &mut w.extra[i as usize..];
                    let old = structs::get(slot, &[Field::Pointer], 0);
                    structs::set(slot, &[Field::Pointer], 0, value);
                    old
                }
                _ => return None,
            };
            Some(old)
        });
        match r {
            Some(old) => {
                g::set_last_error(0);
                old
            }
            None => fail(e::INVALID_INDEX),
        }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetWindowLongW(hwnd_value: usize, index: i32) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    GetWindowLongPtrW(door_windows_abi::narrow(hwnd_value), index) as i32
}
#[unsafe(no_mangle)]
pub extern "system" fn SetWindowLongW(hwnd_value: usize, index: i32, value: i32) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    {
        let v = if index == gwl::STYLE || index == gwl::EXSTYLE {
            u64::from(value as u32)
        } else {
            value as i64 as u64
        };
        SetWindowLongPtrW(
            door_windows_abi::narrow(hwnd_value),
            index,
            door_windows_abi::narrow(v),
        ) as i32
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetDlgCtrlID(hwnd_value: usize) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        with(|u| {
            u.windows
                .get(h)
                .filter(|w| w.is_child())
                .map_or(0, |w| w.id as i32)
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetParent(hwnd_value: usize) -> usize {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    door_windows_abi::narrow((move || -> u64 {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        with(|u| {
            let w = u.windows.get(h)?;
            if w.is_child() {
                w.parent
            } else if w.style & ws::POPUP != 0 {
                w.owner
            } else {
                None
            }
        })
        .map_or(0, Handle::value)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetAncestor(hwnd_value: usize, flags: u32) -> usize {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    door_windows_abi::narrow((move || -> u64 {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        with(|u| match flags {
            gw::GA_PARENT => u.windows.parent(h),
            gw::GA_ROOT => Some(u.windows.root(h)),
            gw::GA_ROOTOWNER => {
                let mut r = u.windows.root(h);
                while let Some(o) = u.windows.get(r).and_then(|w| w.owner) {
                    r = o;
                }
                Some(r)
            }
            _ => None,
        })
        .map_or(0, Handle::value)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetWindow(hwnd_value: usize, cmd: u32) -> usize {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    door_windows_abi::narrow((move || -> u64 {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        let r = with(|u| {
            let w = u.windows.get(h)?;
            let siblings: Vec<Handle> = match w.parent {
                Some(p) => u.windows.get(p)?.children.clone(),
                None => u.windows.top.clone(),
            };
            let pos = siblings.iter().position(|&x| x == h);
            match cmd {
                gw::HWNDFIRST => siblings.first().copied(),
                gw::HWNDLAST => siblings.last().copied(),
                gw::HWNDNEXT => pos.and_then(|p| siblings.get(p + 1).copied()),
                gw::HWNDPREV => pos
                    .and_then(|p| p.checked_sub(1))
                    .and_then(|p| siblings.get(p).copied()),
                gw::OWNER => w.owner,
                gw::CHILD => w.children.first().copied(),
                gw::ENABLEDPOPUP => u
                    .windows
                    .iter()
                    .find(|(_, x)| x.owner == Some(h) && x.visible() && x.enabled())
                    .map(|(x, _)| x),
                _ => None,
            }
        });
        r.map_or(0, Handle::value)
    })())
}

#[unsafe(no_mangle)]
pub extern "system" fn EnumThreadWindows(thread: u32, procedure: usize, lparam: isize) -> i32 {
    let procedure = door_windows_abi::widen(procedure);
    let lparam = lparam as i64;
    {
        let list = with(|u| u.windows.thread_windows(thread));
        if list.is_empty() {
            return 0;
        }
        for h in list {
            if hwnd(h.value()).is_some() && super::call_enum(procedure, h.value(), lparam) == 0 {
                return 0;
            }
        }
        1
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn EnumChildWindows(parent: usize, procedure: usize, lparam: isize) -> i32 {
    let parent = door_windows_abi::widen(parent);
    let procedure = door_windows_abi::widen(procedure);
    let lparam = lparam as i64;
    {
        let Some(p) = hwnd_or_error(parent) else {
            return 0;
        };
        let list = with(|u| u.windows.subtree(p));
        for h in list.into_iter().skip(1) {
            if hwnd(h.value()).is_some() && super::call_enum(procedure, h.value(), lparam) == 0 {
                break;
            }
        }
        1
    }
}

// ---- focus, activation, capture -------------------------------------------------------

pub(crate) fn set_focus(target: Option<Handle>) -> Option<Handle> {
    let me = tid();
    let old = with(|u| u.thread(me).focus);
    if old == target {
        return old;
    }
    with(|u| u.thread(me).focus = target);
    if let Some(o) = old.filter(|o| hwnd(o.value()).is_some()) {
        msg::send(o.value(), wm::KILLFOCUS, target.map_or(0, Handle::value), 0);
    }
    if let Some(t) = target {
        msg::send(t.value(), wm::SETFOCUS, old.map_or(0, Handle::value), 0);
    }
    old
}

#[unsafe(no_mangle)]
pub extern "system" fn SetFocus(hwnd_value: usize) -> usize {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    door_windows_abi::narrow((move || -> u64 {
        let me = tid();
        let target = if hwnd_value == 0 {
            None
        } else {
            let Some(h) = hwnd_or_error(hwnd_value) else {
                return 0;
            };
            if with(|u| u.window_thread(h)) != Some(me) {
                return fail(e::ACCESS_DENIED);
            }
            if !with(|u| u.windows.get(h).is_some_and(|w| w.enabled())) {
                return 0;
            }
            Some(h)
        };
        set_focus(target).map_or(0, Handle::value)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetFocus() -> usize {
    door_windows_abi::narrow((move || -> u64 {
        let me = tid();
        with(|u| u.thread(me).focus).map_or(0, Handle::value)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetActiveWindow() -> usize {
    door_windows_abi::narrow((move || -> u64 {
        let me = tid();
        with(|u| u.thread(me).active).map_or(0, Handle::value)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn SetActiveWindow(hwnd_value: usize) -> usize {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    door_windows_abi::narrow((move || -> u64 {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        let me = tid();
        let root = with(|u| u.windows.root(h));
        let old = with(|u| u.thread(me).active);
        server::activate(root);
        if old != Some(root) {
            with(|u| u.thread(me).active = Some(root));
            if let Some(o) = old.filter(|o| hwnd(o.value()).is_some()) {
                msg::send(
                    o.value(),
                    wm::ACTIVATE,
                    wa::INACTIVE as u64,
                    root.value() as i64,
                );
            }
            msg::send(
                root.value(),
                wm::ACTIVATE,
                wa::ACTIVE as u64,
                old.map_or(0, |o| o.value() as i64),
            );
        }
        old.map_or(0, Handle::value)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn SetForegroundWindow(hwnd_value: usize) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        let root = with(|u| u.windows.root(h));
        with(|u| u.foreground = Some(root));
        server::activate(root);
        1
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetForegroundWindow() -> usize {
    door_windows_abi::narrow((move || -> u64 {
        with(|u| u.foreground).map_or(0, Handle::value)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn SetCapture(hwnd_value: usize) -> usize {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    door_windows_abi::narrow((move || -> u64 {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        let me = tid();
        let old = with(|u| u.thread(me).capture.replace(h));
        let root = with(|u| u.windows.root(h));
        if let Some(id) = with(|u| u.windows.get(root).and_then(|w| w.native)) {
            server::conn(|c| c.capture(id));
        }
        old.map_or(0, Handle::value)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn ReleaseCapture() -> i32 {
    let me = tid();
    let old = with(|u| u.thread(me).capture.take());
    if let Some(o) = old {
        let root = with(|u| u.windows.root(o));
        if let Some(id) = with(|u| u.windows.get(root).and_then(|w| w.native)) {
            server::conn(|c| c.release_capture(id));
        }
        msg::send(o.value(), wm::CAPTURECHANGED, 0, 0);
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCapture() -> usize {
    door_windows_abi::narrow((move || -> u64 {
        let me = tid();
        with(|u| u.thread(me).capture).map_or(0, Handle::value)
    })())
}

#[unsafe(no_mangle)]
pub extern "system" fn CallWindowProcW(
    procedure: usize,
    hwnd_value: usize,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    let procedure = door_windows_abi::widen(procedure);
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    let wparam = door_windows_abi::widen(wparam);
    let lparam = lparam as i64;
    ((move || -> i64 { call(procedure, hwnd_value, message, wparam, lparam) })()) as isize
}

// ---- DefWindowProcW -------------------------------------------------------------------

fn insets_of(h: Handle) -> Rect {
    with(|u| {
        let w = u.windows.get(h)?;
        let has_menu = !w.is_child() && w.id != 0 && !w.message_only;
        Some(metrics::insets(w.style, w.ex_style, has_menu, w.is_child()))
    })
    .unwrap_or(Rect::EMPTY)
}

fn hit_test(h: Handle, screen: Point) -> i64 {
    let Some((win_rect, client, child, style, has_menu)) = with(|u| {
        let w = u.windows.get(h)?;
        Some((
            u.windows.screen_rect(h),
            w.client,
            w.is_child(),
            w.style,
            !w.is_child() && w.id != 0,
        ))
    }) else {
        return ht::NOWHERE as i64;
    };
    if !win_rect.contains(screen) {
        return ht::NOWHERE as i64;
    }
    let p = Point::new(screen.x - win_rect.left, screen.y - win_rect.top);
    if client.contains(p) {
        return ht::CLIENT as i64;
    }
    if !child && has_menu && style & ws::CAPTION == ws::CAPTION {
        let menu_top = metrics::CAPTION;
        if p.y >= menu_top && p.y < menu_top + metrics::MENU_HEIGHT {
            return ht::MENU as i64;
        }
    }
    if style & ws::VSCROLL != 0 && p.x >= client.right {
        return ht::VSCROLL as i64;
    }
    if style & ws::HSCROLL != 0 && p.y >= client.bottom {
        return ht::HSCROLL as i64;
    }
    if !child && style & ws::THICKFRAME != 0 && style & ws::CAPTION != ws::CAPTION {
        let (w, hh) = (win_rect.width(), win_rect.height());
        let (l, r, t, b) = (p.x < 4, p.x >= w - 4, p.y < 4, p.y >= hh - 4);
        return match (l, r, t, b) {
            (true, _, true, _) => ht::TOPLEFT,
            (_, true, true, _) => ht::TOPRIGHT,
            (true, _, _, true) => ht::BOTTOMLEFT,
            (_, true, _, true) => ht::BOTTOMRIGHT,
            (true, ..) => ht::LEFT,
            (_, true, ..) => ht::RIGHT,
            (_, _, true, _) => ht::TOP,
            (.., true) => ht::BOTTOM,
            _ => ht::BORDER,
        } as i64;
    }
    ht::BORDER as i64
}

fn edges_of(hit: i32) -> Option<u32> {
    use door_service_protocol::window::{
        EDGE_BOTTOM as B, EDGE_LEFT as L, EDGE_RIGHT as R, EDGE_TOP as T,
    };
    Some(match hit {
        ht::LEFT => L,
        ht::RIGHT => R,
        ht::TOP => T,
        ht::BOTTOM => B,
        ht::TOPLEFT => T | L,
        ht::TOPRIGHT => T | R,
        ht::BOTTOMLEFT => B | L,
        ht::BOTTOMRIGHT => B | R,
        _ => return None,
    })
}

fn paint_frame(h: Handle) {
    let (style, ex, child, client, rect) = match with(|u| {
        let w = u.windows.get(h)?;
        Some((w.style, w.ex_style, w.is_child(), w.client, w.rect))
    }) {
        Some(v) => v,
        None => return,
    };
    let hdc = paint::GetWindowDC(door_windows_abi::narrow(h.value()));
    if hdc == 0 {
        return;
    }
    let (w, hh) = (rect.width(), rect.height());
    if !child {
        menu::draw_bar(h, door_windows_abi::widen(hdc));
    } else {
        let mut r = Rect::new(0, 0, w, hh);
        if ex & ws_ex::CLIENTEDGE != 0 {
            paint::frame(
                door_windows_abi::widen(hdc),
                r,
                (color::BTNSHADOW + 1) as u64,
            );
            r = r.inflate(-1, -1);
            paint::frame(
                door_windows_abi::widen(hdc),
                r,
                (color::DKSHADOW_3D + 1) as u64,
            );
            r = r.inflate(-1, -1);
        }
        if ex & ws_ex::STATICEDGE != 0 || style & ws::BORDER != 0 {
            paint::frame(
                door_windows_abi::widen(hdc),
                r,
                (color::WINDOWFRAME + 1) as u64,
            );
        }
        if style & ws::VSCROLL != 0 {
            let bar = Rect::new(
                client.right,
                client.top,
                client.right + metrics::SCROLL,
                client.bottom,
            );
            paint::fill(
                door_windows_abi::widen(hdc),
                bar,
                (color::SCROLLBAR + 1) as u64,
            );
            let arrow = metrics::SCROLL.min(bar.height() / 2);
            paint::fill(
                door_windows_abi::widen(hdc),
                Rect::new(bar.left, bar.top, bar.right, bar.top + arrow),
                (color::BTNFACE + 1) as u64,
            );
            paint::fill(
                door_windows_abi::widen(hdc),
                Rect::new(bar.left, bar.bottom - arrow, bar.right, bar.bottom),
                (color::BTNFACE + 1) as u64,
            );
        }
    }
    paint::ReleaseDC(door_windows_abi::narrow(h.value()), hdc);
}

#[unsafe(no_mangle)]
pub extern "system" fn DefWindowProcW(
    hwnd_value: usize,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    let wparam = door_windows_abi::widen(wparam);
    let lparam = lparam as i64;
    ((move || -> i64 {
        let Some(h) = hwnd(hwnd_value) else { return 0 };
        match message {
            wm::NCCREATE => {
                let name = g::read_pointer(
                    lparam as u64
                        + layout::offset(&structs::CREATESTRUCTW, structs::create::NAME) as u64,
                )
                .unwrap_or(0);
                if name >= 0x1_0000
                    && let Some(t) = g::read_wide(name, MAX_TEXT)
                {
                    set_text(h, t);
                }
                1
            }
            wm::NCCALCSIZE => {
                let i = insets_of(h);
                if let Some([l, t, r, b]) = g::read_i32s::<4>(lparam as u64) {
                    let n = [
                        l + i.left,
                        t + i.top,
                        (r - i.right).max(l + i.left),
                        (b - i.bottom).max(t + i.top),
                    ];
                    g::write_i32s(lparam as u64, &n);
                }
                0
            }
            wm::NCHITTEST => {
                let p = door_windows_gui::Point::from_lparam(lparam as isize);
                hit_test(h, p)
            }
            wm::NCPAINT => {
                paint_frame(h);
                0
            }
            wm::ERASEBKGND => {
                let brush = with(|u| {
                    let w = u.windows.get(h)?;
                    let i = u.classes.by_atom(w.class)?;
                    Some(u.classes.get(i)?.background)
                })
                .unwrap_or(0);
                if brush == 0 {
                    return 0;
                }
                let (w, hh) = with(|u| u.windows.get(h).map(|w| w.client_size())).unwrap_or((0, 0));
                paint::fill(wparam, Rect::new(0, 0, w, hh), brush);
                1
            }
            wm::PAINT => {
                let mut ps = [0u8; structs::PAINTSTRUCT_BYTES];
                let hdc = paint::BeginPaint(
                    door_windows_abi::narrow(hwnd_value),
                    door_windows_abi::narrow(ps.as_mut_ptr() as u64),
                );
                if hdc != 0 {
                    paint::EndPaint(
                        door_windows_abi::narrow(hwnd_value),
                        door_windows_abi::narrow(ps.as_ptr() as u64),
                    );
                }
                0
            }
            wm::CLOSE => {
                DestroyWindow(door_windows_abi::narrow(hwnd_value));
                0
            }
            wm::SYSCOMMAND => {
                let root = with(|u| u.windows.root(h));
                match (wparam & 0xfff0) as usize {
                    sc::CLOSE => {
                        msg::send(root.value(), wm::CLOSE, 0, 0);
                    }
                    sc::MINIMIZE => server::request_state(root, 1),
                    sc::MAXIMIZE => server::request_state(root, 2),
                    sc::RESTORE => server::request_state(root, 0),
                    sc::KEYMENU => menu::track_keyboard(root, (lparam & 0xffff) as u16),
                    _ => (),
                }
                0
            }
            wm::NCLBUTTONDOWN => {
                let hit = wparam as i32;
                let root = with(|u| u.windows.root(h));
                match hit {
                    ht::CAPTION => {
                        server::move_resize(root, 0);
                    }
                    ht::MENU => menu::track_mouse(
                        root,
                        door_windows_gui::Point::from_lparam(lparam as isize),
                    ),
                    ht::CLOSE => {
                        msg::send(root.value(), wm::SYSCOMMAND, sc::CLOSE as u64, lparam);
                    }
                    ht::MINBUTTON => server::request_state(root, 1),
                    ht::MAXBUTTON => server::request_state(root, 2),
                    _ => {
                        if let Some(edges) = edges_of(hit) {
                            server::move_resize(root, edges);
                        }
                    }
                }
                0
            }
            wm::NCLBUTTONDBLCLK => {
                if wparam as i32 == ht::CAPTION {
                    let root = with(|u| u.windows.root(h));
                    server::request_state(root, 2);
                }
                0
            }
            wm::KEYUP | wm::SYSKEYUP => {
                let key = wparam as u8;
                if key == door_windows_gui::keyboard::vk::F10
                    || (message == wm::SYSKEYUP && key == door_windows_gui::keyboard::vk::MENU)
                {
                    let root = with(|u| u.windows.root(h));
                    msg::send(root.value(), wm::SYSCOMMAND, sc::KEYMENU as u64, 0);
                }
                0
            }
            wm::SYSKEYDOWN => {
                if wparam as u8 == door_windows_gui::keyboard::vk::F4 && lparam & (1 << 29) != 0 {
                    let root = with(|u| u.windows.root(h));
                    msg::send(root.value(), wm::SYSCOMMAND, sc::CLOSE as u64, 0);
                }
                0
            }
            wm::SYSCHAR => {
                let root = with(|u| u.windows.root(h));
                if wparam != 0x20 {
                    msg::send(
                        root.value(),
                        wm::SYSCOMMAND,
                        sc::KEYMENU as u64,
                        wparam as i64,
                    );
                }
                0
            }
            wm::GETTEXT => {
                let text = text_of(h);
                g::write_wide(lparam as u64, wparam as usize, &text).map_or(0, |n| n as i64)
            }
            wm::GETTEXTLENGTH => text_of(h).len() as i64,
            wm::SETICON => super::icon::set_window_icon(hwnd_value, wparam, lparam as u64) as i64,
            wm::GETICON => super::icon::window_icon(hwnd_value, wparam) as i64,
            wm::SETTEXT => {
                let text = if lparam == 0 {
                    Some(Vec::new())
                } else {
                    g::read_wide(lparam as u64, MAX_TEXT)
                };
                match text {
                    Some(t) => {
                        set_text(h, t);
                        1
                    }
                    None => 0,
                }
            }
            // The tracking limits (WM_GETMINMAXINFO) of a resizable or
            // top-level window, applied to the size SetWindowPos asks for.
            wm::WINDOWPOSCHANGING => {
                let mut wp = [0u8; structs::WINDOWPOS_BYTES];
                let limited = with(|u| {
                    u.windows.get(h).is_some_and(|w| {
                        w.style & ws::THICKFRAME != 0 || (w.parent.is_none() && !w.message_only)
                    })
                });
                if limited && g::read(lparam as u64, &mut wp) {
                    let flags = structs::get(&wp, &structs::WINDOWPOS, windowpos::FLAGS) as u32;
                    if flags & swp::NOSIZE == 0 {
                        let ((minx, miny), (maxx, maxy)) = min_max(h);
                        let field = |index| structs::get_i32(&wp, &structs::WINDOWPOS, index);
                        let cx = field(windowpos::CX).clamp(minx.min(maxx), maxx.max(minx));
                        let cy = field(windowpos::CY).clamp(miny.min(maxy), maxy.max(miny));
                        structs::set_i32(&mut wp, &structs::WINDOWPOS, windowpos::CX, cx);
                        structs::set_i32(&mut wp, &structs::WINDOWPOS, windowpos::CY, cy);
                        let _ = g::write(lparam as u64, &wp);
                    }
                }
                0
            }
            wm::WINDOWPOSCHANGED => {
                let mut wp = [0u8; structs::WINDOWPOS_BYTES];
                if g::read(lparam as u64, &mut wp) {
                    let flags = structs::get(&wp, &structs::WINDOWPOS, windowpos::FLAGS) as u32;
                    let t = with(|u| {
                        let w = u.windows.get(h)?;
                        let (cw, ch) = w.client_size();
                        Some((
                            cw,
                            ch,
                            Point::new(w.rect.left + w.client.left, w.rect.top + w.client.top),
                        ))
                    });
                    if let Some((cw, ch, o)) = t {
                        if flags & swp::NOMOVE == 0 {
                            msg::send(hwnd_value, wm::MOVE, 0, point_lparam(o.x, o.y));
                        }
                        if flags & swp::NOSIZE == 0 {
                            msg::send(
                                hwnd_value,
                                wm::SIZE,
                                size::RESTORED as u64,
                                point_lparam(cw, ch),
                            );
                        }
                    }
                }
                0
            }
            wm::ACTIVATE => {
                if wparam & 0xffff != wa::INACTIVE as u64 {
                    let me = tid();
                    let focus = with(|u| u.thread(me).focus);
                    let inside =
                        focus.is_some_and(|f| f == h || with(|u| u.windows.is_descendant(h, f)));
                    if !inside {
                        set_focus(Some(h));
                    }
                }
                0
            }
            wm::MOUSEACTIVATE => 1,
            wm::SETCURSOR => 0,
            0x0132..=0x0138 => {
                // WM_CTLCOLOR*: window colors for edits/lists, face otherwise.
                unsafe {
                    super::SetTextColor(
                        door_windows_abi::narrow(wparam),
                        door_windows_gui::metrics::SYS_COLORS[color::WINDOWTEXT as usize],
                    );
                }
                let index = if matches!(message, 0x0133 | 0x0134) {
                    color::WINDOW
                } else {
                    color::BTNFACE
                };
                unsafe {
                    super::SetBkColor(
                        door_windows_abi::narrow(wparam),
                        door_windows_gui::metrics::SYS_COLORS[index as usize],
                    );
                }
                paint::brush((index + 1) as u64) as i64
            }
            wm::NOTIFYFORMAT => 2,
            wm::QUERYENDSESSION => 1,
            wm::MOUSEWHEEL | wm::CONTEXTMENU => {
                let parent = with(|u| {
                    u.windows
                        .get(h)
                        .filter(|w| w.is_child())
                        .and_then(|w| w.parent)
                });
                match parent {
                    Some(p) => msg::send(p.value(), message, wparam, lparam),
                    None => 0,
                }
            }
            wm::GETFONT => with(|u| u.windows.get(h).map_or(0, |w| w.font)) as i64,
            _ => 0,
        }
    })()) as isize
}

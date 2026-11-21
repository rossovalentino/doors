//! Trusted desktop shell controller: taskbar, Start, tray flyouts, menus,
//! desktop icons, snap layouts and the lock screen. All shell painting and
//! geometry lives in the host-tested `door_window` library; this file wires
//! it to live engine state, input and real backends.
use super::*;
use door_graphics_text::{Clip, Target, TextRun, TextStyle};
pub(super) use door_window::apps::App;
use door_window::{
    menu::{Anchor, Command, Menu, MenuHit, Style},
    menus, motion, paint, snap, start,
    taskbar::{self, Slot},
    tray,
};
#[derive(Clone, Copy)]
pub(super) struct Item {
    pub(super) id: g::WindowId,
    pub(super) owner: g::Owner,
    pub(super) active: bool,
    pub(super) app: App,
}
pub(super) fn items(engine: &g::Engine, classes: &[(g::Owner, App)]) -> Vec<Item> {
    all_items(engine, classes)
        .into_iter()
        .filter(|item| {
            engine
                .window_info(item.id)
                .is_ok_and(|w| w.workspace == engine.active_workspace())
        })
        .collect()
}
fn all_items(engine: &g::Engine, classes: &[(g::Owner, App)]) -> Vec<Item> {
    let mut items: Vec<_> = engine
        .windows()
        .filter(|w| w.role == g::Role::TopLevel && w.state != g::WindowState::Closing)
        .filter_map(|w| {
            classes
                .iter()
                .find(|(owner, _)| *owner == w.owner)
                .map(|(_, app)| Item {
                    id: w.id,
                    owner: w.owner,
                    active: w.active,
                    app: *app,
                })
        })
        .collect();
    items.sort_by_key(|w| w.owner.pid);
    items
}
/// Taskbar buttons: pinned apps in pin order, then running unpinned apps.
pub(super) fn buttons(pins: &[App], items: &[Item]) -> Vec<taskbar::Button> {
    let running: Vec<(App, bool)> = items.iter().map(|i| (i.app, i.active)).collect();
    taskbar::buttons(pins, &running)
}
/// Shell-owned state that is not part of the window engine.
/// A title bar drag being watched for a shake (System > Multitasking).
#[derive(Clone, Copy, Default)]
pub(super) struct Shake {
    pub window: Option<g::WindowId>,
    pub anchor: i32,
    pub last: i32,
    pub direction: i32,
    pub turns: u8,
    pub started: u64,
}
pub(super) struct Shell {
    pub(super) start: start::Start,
    pub(super) start_motion: motion::Transition,
    pub(super) flyout_motion: motion::Transition,
    pub(super) flyout_kind: tray::Flyout,
    pub(super) menu: Option<Menu>,
    /// Tray/Start/desktop target a context menu was opened for.
    pub(super) menu_app: Option<App>,
    pub(super) menu_point: (i32, i32),
    pub(super) taskbar_pins: Vec<App>,
    pub(super) bar_pressed: Option<Slot>,
    /// Animated press and hover feedback of the taskbar buttons.
    pub(super) bar_feedback: taskbar::Feedback,
    pub(super) desktop_order: [App; door_window::apps::COUNT],
    pub(super) sorted: bool,
    /// Shell layout last applied from or written to the user's profile.
    pub(super) layout_saved: door_service_protocol::desktop::ShellLayout,
    /// Profile writes waiting for Init (storage persists them per user).
    pub(super) layout_pending: Option<door_service_protocol::desktop::ShellLayout>,
    pub(super) controls_pending: Option<door_service_protocol::desktop::Controls>,
    /// A preference word the shell changed in place (quick settings
    /// volume), persisted through Init (`PERSIST_SETTING`).
    pub(super) setting_pending: Vec<(u32, u32)>,
    pub(super) persist: Option<door_service_client::Init>,
    /// Start file search through the File Explorer provider.
    pub(super) files: SearchLink,
    pub(super) snap: Option<SnapBar>,
    pub(super) max_hover: snap::MaximizeHover,
    /// Logical regions painted by the last shell frame (dismissal repaint).
    pub(super) presented: Vec<g::Rect>,
    pub(super) locked: bool,
    pub(super) lock_pressed: bool,
    /// P14 preference words (`door_service_protocol::preferences`), the
    /// Light/Dark schedule and the sign-in-away idle timer.
    pub(super) prefs: door_window::preferences::Words,
    /// An auto-hidden taskbar is shown (pointer at its edge or on it).
    pub(super) taskbar_revealed: bool,
    /// Background slideshow: the picture shown and when the next is due.
    pub(super) slide: u32,
    pub(super) slide_due: u64,
    /// Sub-pixel motion and partial-notch remainders (Mouse settings).
    pub(super) mouse_rest: (i32, i32),
    pub(super) wheel_rest: i32,
    /// Title bar window shake detection for the dragged window.
    pub(super) shake: Shake,
    pub(super) theme_schedule: door_window::preferences::Schedule,
    pub(super) idle: door_window::preferences::Idle,
    pub(super) secondary: bool,
    /// Win+Z: layout chosen with the first digit.
    pub(super) snap_digit: Option<usize>,
    /// Win+R Run dialog (26H2).
    pub(super) run: door_window::run::Run,
    /// Snap Assist after a snap layout zone was filled, and its desktop.
    pub(super) assist: Option<door_window::snap_assist::Assist>,
    pub(super) assist_workspace: Option<g::WorkspaceId>,
    /// Desktop gesture diagnostics left (bounded serial use).
    pub(super) gesture_notes: u8,
    /// Apps a shutdown waits on while they ask about unsaved work.
    pub(super) shutdown_gate: door_window::policy::ShutdownGate,
    /// Win32 close requests, the five-second hung rule, ghosts and the
    /// "not responding" prompt.
    pub(super) hang: door_window::not_responding::Responsiveness,
    /// User Account Control consent prompt on the secure desktop: while
    /// open it owns all input (`door_window::uac`).
    pub(super) uac: Option<door_window::uac::Consent>,
    /// An approved Run as administrator waiting for Init (the request, the
    /// working-folder handle it borrowed, the program path).
    pub(super) elevated_launch: Option<(door_service_client::Init, Handle, alloc::string::String)>,
    /// The client whose Run as administrator of a program is under way,
    /// and the outcome waiting for its ELEVATION_STATUS.
    pub(super) elevation_requester: Option<u64>,
    pub(super) elevation_outcomes: Vec<(u64, Elevated)>,
    /// The secure desktop's frozen backdrop while the consent prompt is open.
    pub(super) uac_backdrop: Vec<u8>,
    /// "Doors Security Alert" while open, whether it has the keyboard, and
    /// when door.network is next asked for a blocked program.
    pub(super) firewall_alert: Option<door_window::firewall_alert::Alert>,
    pub(super) firewall_alert_focused: bool,
    pub(super) firewall_at: u64,
    /// Session clipboard and its Win+V history (door.clipboard).
    pub(super) clipboard: door_window::clipboard::Broker,
    /// The snip overlay (Win+Shift+S) while open.
    pub(super) snip: Option<door_window::snip::Snip>,
    /// A snip on the clipboard Snipping Tool has yet to be told about.
    pub(super) snip_ready: Option<(u32, u32)>,
    /// Magnifier's view (window::MAGNIFY) and its last magnified frame.
    pub(super) magnifier: Option<door_window::magnify::Magnify>,
    pub(super) magnified: Vec<u8>,
    /// Drag and drop (door.dnd) and its effect label at the pointer.
    pub(super) dnd: door_window::dnd::Broker,
    /// Keyboard layouts, input-method switching and IME routing (door.text-input).
    pub(super) text_input: door_window::text_input::Broker,
    pub(super) dnd_badge: Option<g::Rect>,
    /// The effect label painted in `dnd_badge` (repaint when it changes).
    pub(super) dnd_feedback: Option<door_window::dnd::Feedback>,
    /// The shell text fields' own text-input context and its last caret.
    pub(super) text_serial: Option<u32>,
    pub(super) text_caret: Option<w::Rect>,
    /// Win+V clipboard history panel.
    pub(super) clipboard_panel: door_window::clipboard_history::Panel,
    /// door.accessibility broker and the shell's own semantic trees.
    pub(super) a11y: door_window::accessibility::Broker,
    /// Signature of the shell state the published shell trees describe.
    pub(super) a11y_signature: u64,
    pub(super) a11y_locked: bool,
    /// Keyboard stop of the open tray flyout (`shell_semantics::tray_stops`)
    /// and the flyout it belongs to.
    pub(super) tray_focus: Option<usize>,
    pub(super) tray_focus_for: Option<tray::Flyout>,
    /// Windows 11 taskbar hover preview (door.shell; live thumbnails).
    pub(super) preview: Option<door_window::shell_view::Preview>,
    /// The hovered taskbar button's name (pinned apps that are not
    /// running, Start, Search, Task View), as Windows 11's tooltip.
    pub(super) tooltip: Option<door_window::shell_view::Tooltip>,
}
/// Start file search link to the provider client (File Explorer). Queries
/// are debounced; every query gets a new serial and results for any other
/// serial are dropped.
#[derive(Default)]
pub(super) struct SearchLink {
    pub(super) provider: Option<g::Owner>,
    serial: u32,
    /// Trimmed query text for `serial` (empty: none).
    query: alloc::string::String,
    /// Tick at which SEARCH_QUERY for `serial` is posted.
    due: Option<u64>,
    /// The session's search host, from Init's desktop catalog.
    pub(super) host: Option<g::Owner>,
    /// Provider events: (kind, a, b, detail).
    events: Vec<(u32, u32, u32, u32)>,
    /// OPEN_DOCUMENT waiting for its app to read it.
    document: Option<Document>,
    /// Serial of the Start > Recent list request (0: none).
    recent_serial: u32,
    /// Start is open: Recent was requested for this opening.
    recent_asked: bool,
    /// Recent count last written to the serial diagnostics.
    recent_logged: Option<usize>,
    /// Start is open with a query (`query` is the live one).
    active: bool,
}
/// A document the search host asked an app to open.
/// `pending_launches` bit of the elevated Terminal (apps and tools use the
/// low bits).
const ADMIN_TERMINAL_BIT: u8 = 31;
pub(super) struct Document {
    app: App,
    /// For the elevated Terminal (User Account Control approved it).
    elevated: bool,
    payload: Vec<u8>,
    /// DOCUMENT_OPEN was posted to the app.
    posted: bool,
    /// The app was not running: it opens the document in its first window.
    launched: bool,
    deadline: u64,
}
/// Start search debounce in monotonic ticks: about 60 ms.
const SEARCH_DEBOUNCE: u64 = 60 / door_abi::platform::MONOTONIC_TICK_MILLIS;
#[derive(Clone, Copy)]
pub(super) struct SnapBar {
    pub(super) window: g::WindowId,
    pub(super) owner: g::Owner,
    pub(super) rect: g::Rect,
    pub(super) hover: Option<(usize, usize)>,
    /// Shown while dragging (top of screen) rather than from a button.
    pub(super) drag: bool,
    /// Window bounds before a drag (restore size for built-in snaps).
    pub(super) restore: Option<g::Rect>,
    /// Opened by hovering a maximize button: closes when the pointer leaves.
    pub(super) from_hover: bool,
    /// The snapping window's app: its icon shows in the hovered pane.
    pub(super) app: Option<App>,
    /// The hovered Maximize button (logical), for the hover corridor.
    pub(super) button: g::Rect,
    pub(super) linger: snap::Linger,
    /// Entrance slide/fade (reduce motion: none).
    pub(super) motion: motion::Transition,
}
impl Default for Shell {
    fn default() -> Self {
        Self {
            start: start::Start::default(),
            start_motion: motion::Transition::start(),
            flyout_motion: motion::Transition::default(),
            flyout_kind: tray::Flyout::Calendar,
            menu: None,
            menu_app: None,
            menu_point: (0, 0),
            taskbar_pins: App::ALL.to_vec(),
            bar_pressed: None,
            bar_feedback: taskbar::Feedback::default(),
            desktop_order: App::ALL,
            sorted: false,
            layout_saved: door_service_protocol::desktop::ShellLayout::DEFAULT,
            layout_pending: None,
            controls_pending: None,
            setting_pending: Vec::new(),
            persist: None,
            files: SearchLink::default(),
            snap: None,
            max_hover: snap::MaximizeHover::default(),
            presented: Vec::new(),
            locked: false,
            lock_pressed: false,
            prefs: door_window::preferences::Words::default(),
            taskbar_revealed: false,
            slide: 0,
            slide_due: 0,
            mouse_rest: (0, 0),
            wheel_rest: 0,
            shake: Shake::default(),
            theme_schedule: door_window::preferences::Schedule::default(),
            idle: door_window::preferences::Idle::default(),
            secondary: false,
            snap_digit: None,
            run: door_window::run::Run::default(),
            assist: None,
            assist_workspace: None,
            gesture_notes: 4,
            shutdown_gate: door_window::policy::ShutdownGate::default(),
            hang: door_window::not_responding::Responsiveness::new(),
            uac: None,
            elevated_launch: None,
            elevation_requester: None,
            elevation_outcomes: Vec::new(),
            uac_backdrop: Vec::new(),
            firewall_alert: None,
            firewall_alert_focused: false,
            firewall_at: 0,
            clipboard: door_window::clipboard::Broker::new(),
            snip: None,
            snip_ready: None,
            magnifier: None,
            magnified: Vec::new(),
            dnd: door_window::dnd::Broker::new(),
            text_input: door_window::text_input::Broker::new(),
            dnd_badge: None,
            dnd_feedback: None,
            text_serial: None,
            text_caret: None,
            clipboard_panel: door_window::clipboard_history::Panel::default(),
            a11y: door_window::accessibility::Broker::default(),
            a11y_signature: 0,
            a11y_locked: false,
            tray_focus: None,
            tray_focus_for: None,
            preview: None,
            tooltip: None,
        }
    }
}
pub(super) fn caption(c: &mut g::Canvas<'_>, info: g::WindowInfo<'_>, app: Option<App>) {
    let scale = c.scale();
    // Native frames exist per whole density; fractional scales area-filter
    // the next density's frame to the exact physical size.
    let factor = scale.asset_factor();
    macro_rules! asset {
        ($name:literal) => {
            if factor == 3 {
                &include_bytes!(concat!("../../../assets/icons/windows/", $name, ".48.bgra"))[..]
            } else if factor == 2 {
                &include_bytes!(concat!("../../../assets/icons/windows/", $name, ".32.bgra"))[..]
            } else {
                &include_bytes!(concat!("../../../assets/icons/windows/", $name, ".16.bgra"))[..]
            }
        };
    }
    // The authenticated Files client supplies its current location as its title.
    // Use the corresponding original offline directory artwork in trusted chrome.
    let file_icon: &[u8] = match info.title {
        "This PC" => asset!("computer"),
        "Home" => asset!("explorer-home"),
        "Desktop" => asset!("desktop-current"),
        "Documents" => asset!("documents-current"),
        "Downloads" => asset!("downloads-current"),
        "Music" => asset!("music-current"),
        "Pictures" => asset!("pictures-current"),
        "Videos" => asset!("videos-current"),
        "C:\\" | "C:" | "Doors (C:)" => asset!("drive"),
        _ => asset!("folder"),
    };
    let n = scale.len(16);
    let (native, side): (&[u8], u32) = match app.unwrap_or(App::Files) {
        App::Files => (file_icon, 16 * factor),
        other => match door_window::apps::icon_pixels(other, n) {
            Some(pixels) => (pixels, n),
            None => door_window::apps::icon_pixels(other, 16 * factor)
                .map_or((file_icon, 16 * factor), |pixels| (pixels, 16 * factor)),
        },
    };
    let resampled;
    let pixels = if side == n {
        native
    } else {
        resampled = g::area_resample_bgra(native, side as usize, n as usize);
        &resampled[..]
    };
    let _ = c.blit_bgra_physical(
        g::Rect::new(
            scale.from_edge(info.outer.x(), 7),
            scale.from_edge(info.outer.y(), 8),
            n,
            n,
        )
        .unwrap(),
        pixels,
    );
}
pub(super) fn background(
    c: &mut g::Canvas<'_>,
    fonts: &FontAtlas,
    pixels: &[u8],
    icons: Option<&Icons>,
) {
    let _ = c.restore_bgra(pixels);
    if let Some(icons) = icons {
        paint_icons(c, fonts, icons);
    }
}
/// Desktop icon state for painting (only when icons are shown).
#[derive(Clone, Copy)]
pub(super) struct Icons {
    pub(super) order: [App; door_window::apps::COUNT],
    /// Desktop slots (`desktop_grid`): the Recycle Bin, then `order`.
    pub(super) selected: Option<usize>,
    pub(super) hover: Option<usize>,
    pub(super) controls: door_service_protocol::desktop::Controls,
    pub(super) recycle_full: bool,
}
/// What a desktop icon slot holds.
/// How a client's Run as administrator of a program ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Elevated {
    /// It runs: a handle to it (wait and query) for the requester.
    Started(Handle),
    Declined,
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DesktopItem {
    RecycleBin,
    App(App),
}
impl DesktopItem {
    /// The item in `slot` with the apps in `order`.
    pub(super) fn at(slot: usize, order: &[App; door_window::apps::COUNT]) -> Option<Self> {
        if slot == desktop_grid::BIN_SLOT {
            Some(Self::RecycleBin)
        } else {
            order.get(slot.checked_sub(1)?).copied().map(Self::App)
        }
    }
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::RecycleBin => "Recycle Bin",
            Self::App(app) => app.label(),
        }
    }
    pub(super) fn app(self) -> Option<App> {
        match self {
            Self::App(app) => Some(app),
            Self::RecycleBin => None,
        }
    }
}
use door_window::apps::desktop_grid;
/// Desktop icon hit band (logical pixels from the work-area top).
pub(super) fn icon_band(index: usize) -> core::ops::Range<i32> {
    desktop_grid::band(index)
}
/// Logical region covering every desktop icon with its hover/selection plate
/// and label; hover, invalidation, repaint and scanout all use it.
pub(super) fn icon_column(area: g::Rect) -> g::Rect {
    let height = 12 + desktop_grid::SLOTS as u32 * desktop_grid::PITCH as u32;
    g::Rect::new(
        area.x(),
        area.y(),
        area.width().min(116),
        area.height().min(height),
    )
    .expect("non-empty work area")
}
fn text(c: &mut g::Canvas<'_>, fonts: &FontAtlas, x: i32, y: i32, label: &str, color: [u8; 4]) {
    let (w, _, _) = c.dimensions();
    let (pw, ph, stride) = c.physical_dimensions();
    let scale = c.scale();
    let Some(clip) = c.clip_rect().and_then(|clip| {
        g::Rect::new(
            x,
            y,
            w.saturating_sub(x.max(0) as u32),
            fonts.line_height(TextStyle::Regular12),
        )
        .and_then(|r| scale.rect(r))
        .ok()?
        .intersection(clip)
    }) else {
        return;
    };
    // Desktop icon labels are a Win32 list view: ClearType, as on Windows.
    if let Ok(target) = Target::new(c.pixels_mut(), pw, ph, stride) {
        let mut target = target.cleartype();
        let _ = fonts.draw_physical(
            &mut target,
            Clip::new(clip.x(), clip.y(), clip.width(), clip.height()),
            TextRun {
                text: label,
                x: scale.px(x),
                top: scale.px(y),
                style: TextStyle::Regular12,
                color,
            },
        );
    }
}
/// Windows 11 "Medium icons": 48 px icon, 12 px white label with a subtle
/// shadow, translucent blue selection and lighter hover plates.
pub(super) fn paint_icons(c: &mut g::Canvas<'_>, fonts: &FontAtlas, icons: &Icons) {
    let (w, h, _) = c.dimensions();
    let origin = door_window::taskbar::Layout::with_controls(w, h, icons.controls).work_area();
    let ox = origin.x();
    let oy = origin.y();
    // Contrast themes: Highlight selection (HighlightText label) and a
    // Highlight hover outline instead of the translucent blue plates.
    let hc = icons
        .controls
        .contrast()
        .map(paint::Palette::contrast_theme);
    for i in 0..desktop_grid::SLOTS {
        let Some(item) = DesktopItem::at(i, &icons.order) else {
            continue;
        };
        let selected = icons.selected == Some(i);
        if let Some(p) = hc.as_ref().filter(|_| selected || icons.hover == Some(i)) {
            let (px, py, pw, ph) = desktop_grid::plate(i);
            let plate = g::Rect::new(ox + px, oy + py, pw, ph).unwrap();
            if selected {
                paint::fill(c, plate, p.accent);
            } else {
                paint::outline(c, plate, p.accent, 1);
            }
        } else if selected || icons.hover == Some(i) {
            let (px, py, pw, ph) = desktop_grid::plate(i);
            // Premultiplied plates; BGRA channels never exceed alpha.
            paint::rounded(
                c,
                g::Rect::new(ox + px, oy + py, pw, ph).unwrap(),
                2,
                if selected {
                    [180, 145, 100, 180]
                } else {
                    [96, 90, 80, 96]
                },
            );
            paint::rounded(
                c,
                g::Rect::new(ox + px + 1, oy + py + 1, pw - 2, ph - 2).unwrap(),
                1,
                if selected {
                    [100, 74, 40, 104]
                } else {
                    [54, 47, 36, 58]
                },
            );
        }
        let (ix, iy) = desktop_grid::icon(i);
        match item {
            DesktopItem::App(app) => {
                door_window::apps::draw_icon(c, ox + ix, oy + iy, app, desktop_grid::ICON)
            }
            DesktopItem::RecycleBin => door_window::apps::draw_recycle_bin(
                c,
                ox + ix,
                oy + iy,
                icons.recycle_full,
                desktop_grid::ICON,
            ),
        }
        let label = item.label();
        let width = fonts
            .measure(label, TextStyle::Regular12)
            .map_or(0, |m| m.advance_width) as i32;
        let (cx, top) = desktop_grid::label(i);
        let left = ox + cx - width / 2;
        if let Some(p) = &hc {
            // Contrast themes: WindowText on the solid Window desktop
            // (HighlightText on the selection).
            text(
                c,
                fonts,
                left,
                oy + top,
                label,
                if selected { p.accent_text } else { p.text },
            );
            continue;
        }
        // Windows draws desktop labels with a soft dark text shadow.
        text(c, fonts, left + 1, oy + top + 1, label, [0, 0, 0, 128]);
        text(c, fonts, left, oy + top, label, [255, 255, 255, 255]);
    }
}
/// One frame of shell overlay painting.
pub(super) struct Overlay<'a> {
    pub(super) fonts: &'a FontAtlas,
    pub(super) layout: taskbar::Layout,
    pub(super) buttons: &'a [taskbar::Button],
    /// door.shell extra buttons and catalog apps' attention/progress marks.
    pub(super) extras: &'a [door_window::shell_link::Extra],
    pub(super) marks: &'a [(App, door_window::shell_link::WindowState)],
    pub(super) shell: &'a mut Shell,
    pub(super) tray: &'a mut tray::State,
    pub(super) pointer: (i32, i32),
    pub(super) dark: bool,
    pub(super) material: &'a [u8],
    /// Full-output scene beneath the shell (the retained cache).
    pub(super) underlay: &'a [u8],
    pub(super) now: u64,
    pub(super) task_view: bool,
    /// Rebuild acrylic backdrops from `underlay` (the scene changed).
    pub(super) rebuild: bool,
}
const FLYOUT_SLIDE: i32 = 48;
/// Distance Start rises: from just past the screen edge behind the taskbar
/// to its place, as Windows does (VIS-01 `start.open`).
fn start_rise(layout: taskbar::Layout, panel: g::Rect) -> i32 {
    let out = layout.output();
    if slide_sign(layout) < 0 {
        panel.bottom() - out.y()
    } else {
        out.bottom() - panel.y()
    }
    .max(0)
}
fn slide_sign(layout: taskbar::Layout) -> i32 {
    if layout.controls().edge() == door_service_protocol::desktop::Edge::Top {
        -1
    } else {
        1
    }
}
/// Logical regions the shell paints this frame (bar, surfaces with their
/// shadow and slide travel, menus and the snap bar).
pub(super) fn regions(
    shell: &Shell,
    tray_state: &tray::State,
    layout: taskbar::Layout,
    now: u64,
    fonts: &FontAtlas,
) -> Vec<g::Rect> {
    let out = layout.output();
    let mut r = alloc::vec![layout.shown_bar()];
    let travel = |rect: g::Rect, distance: i32| {
        let moved = g::Rect::new(
            rect.x(),
            rect.y() + distance * slide_sign(layout),
            rect.width(),
            rect.height(),
        )
        .unwrap();
        paint::grow(paint::union(rect, moved), 26, out)
    };
    if shell.start_motion.visible(now) {
        let panel = shell.start.geometry(layout).panel;
        r.push(travel(panel, start_rise(layout, panel)));
    }
    if shell.flyout_motion.visible(now) {
        for rect in tray_state.surfaces(shell.flyout_kind, layout) {
            r.push(travel(rect, FLYOUT_SLIDE));
        }
    }
    if let Some(toasts) = door_window::shell_view::toast_bounds(layout, &tray_state.notices) {
        r.push(paint::grow(toasts, 26, out));
    }
    if let Some(menu) = &shell.menu {
        r.push(paint::grow(menu.extent(), 26, out));
    }
    if let Some(bar) = &shell.snap {
        let moved = g::Rect::new(
            bar.rect.x(),
            bar.rect.y() + snap::SLIDE,
            bar.rect.width(),
            bar.rect.height(),
        )
        .unwrap();
        r.push(paint::grow(paint::union(bar.rect, moved), 26, out));
    }
    if shell.run.open {
        r.push(paint::grow(shell.run.panel(layout), 26, out));
    }
    if let Some(p) = &shell.preview {
        r.push(paint::grow(p.rect, 26, out));
    }
    if shell.clipboard_panel.open {
        r.push(paint::grow(
            door_window::clipboard_view::geometry(layout, &shell.clipboard_panel).panel,
            26,
            out,
        ));
    }
    if let Some(badge) = shell.dnd_badge {
        r.push(paint::grow(badge, 26, out));
    }
    if let Some(zone) = shell.assist.as_ref().and_then(|a| a.region()) {
        r.push(paint::grow(zone, 26, out));
    }
    if let Some(frame) = candidate_region(shell, fonts, layout) {
        r.push(paint::grow(frame, 26, out));
    }
    if shell.hang.prompt().is_some() {
        let panel = door_window::not_responding::geometry(layout.work_area()).panel;
        r.push(paint::grow(panel, 26, out));
    }
    if shell.firewall_alert.is_some() {
        let panel = door_window::firewall_alert::geometry(layout.work_area()).panel;
        r.push(paint::grow(panel, 26, out));
    }
    // The secure desktop dims the whole output.
    if shell.uac.is_some() {
        r.push(out);
    }
    r
}
/// Logical bounds of the IME candidate window, laid out exactly as the
/// overlay paints it (`door_window::text_input::candidate::measure`).
fn candidate_region(shell: &Shell, fonts: &FontAtlas, layout: taskbar::Layout) -> Option<g::Rect> {
    let window = shell.text_input.candidates()?;
    let text = fonts.unicode()?;
    let scale = g::Scale::from_percent(window.scale)?;
    let work = paint::physical(layout.work_area(), scale);
    let work = w::Rect {
        x: work.x(),
        y: work.y(),
        width: work.width(),
        height: work.height(),
    };
    let f = door_window::text_input::candidate::measure(
        window,
        work,
        text,
        shell.text_input.method().tag,
    )
    .frame;
    let (x0, y0) = (scale.logical(f.x), scale.logical(f.y));
    let (x1, y1) = (
        scale.logical(f.x + f.width as i32) + 1,
        scale.logical(f.y + f.height as i32) + 1,
    );
    g::Rect::new(x0, y0, (x1 - x0).max(1) as u32, (y1 - y0).max(1) as u32).ok()
}
pub(super) fn overlay(c: &mut g::Canvas<'_>, o: Overlay<'_>) {
    let Overlay {
        fonts,
        layout,
        buttons,
        extras,
        marks,
        shell,
        tray: tray_state,
        pointer,
        dark,
        material,
        underlay,
        now,
        task_view,
        rebuild,
    } = o;
    let scale = c.scale();
    let stride = c.physical_dimensions().2;
    let controls = layout.controls();
    let transparency = !controls.reduce_transparency() && controls.contrast().is_none();
    let sign = slide_sign(layout);
    let start_visible = shell.start_motion.visible(now);
    let flyout_visible = shell.flyout_motion.visible(now);
    if start_visible && transparency {
        let panel = paint::physical(shell.start.geometry(layout).panel, scale);
        if rebuild || shell.start.backdrop.region() != Some(panel) {
            shell
                .start
                .backdrop
                .build(underlay, stride, panel, scale.len(8) as usize);
        }
    }
    if flyout_visible && transparency {
        let rects = tray_state.surfaces(shell.flyout_kind, layout);
        let bounds = rects
            .iter()
            .copied()
            .reduce(paint::union)
            .map(|r| paint::physical(r, scale));
        if let Some(bounds) = bounds
            && (rebuild || tray_state.backdrop.region() != Some(bounds))
        {
            tray_state
                .backdrop
                .build(underlay, stride, bounds, scale.len(8) as usize);
        }
    }
    let saved = c.clip_rect();
    let work = paint::physical(layout.work_area(), scale);
    if let Some(clip) = saved.and_then(|s| s.intersection(work)) {
        c.set_clip(clip);
        if start_visible {
            let g0 = shell.start.geometry(layout).panel;
            let dy = motion::rise(shell.start_motion.progress(now), start_rise(layout, g0)) * sign;
            start::draw(
                c,
                fonts,
                &shell.start,
                layout,
                &start::Frame {
                    pointer: if shell.menu.is_some() {
                        (-1, -1)
                    } else {
                        pointer
                    },
                    dark,
                    slide: dy,
                    transparency,
                    taskbar_pins: &shell.taskbar_pins,
                },
            );
        }
        if flyout_visible {
            let (offset, opacity) = motion::slide(shell.flyout_motion.progress(now), FLYOUT_SLIDE);
            let dy = offset * sign;
            tray::draw_flyout(
                c,
                fonts,
                tray_state,
                shell.flyout_kind,
                layout,
                &tray::Frame {
                    dark,
                    pointer,
                    slide: dy,
                    transparency,
                },
            );
            for rect in tray_state.surfaces(shell.flyout_kind, layout) {
                let moved =
                    g::Rect::new(rect.x(), rect.y() + dy, rect.width(), rect.height()).unwrap();
                paint::fade(
                    c,
                    paint::physical(paint::grow(moved, 26, layout.output()), scale),
                    underlay,
                    opacity,
                );
            }
        }
    }
    if let Some(saved) = saved {
        c.set_clip(saved);
    }
    taskbar::draw_with(
        c,
        fonts,
        layout,
        &taskbar::View {
            buttons,
            pointer: if shell.menu.is_some() {
                (-1, -1)
            } else {
                pointer
            },
            pressed: shell.bar_pressed,
            press: shell.bar_feedback.pressed(now),
            hover: shell.bar_feedback.hovered(now),
            start_open: shell.start_motion.open && !shell.start.search_panel(),
            search_open: shell.start_motion.open && shell.start.search_panel(),
            task_view_open: task_view,
            quick_open: tray_state.open == Some(tray::Flyout::Devices),
            calendar_open: tray_state.open == Some(tray::Flyout::Calendar),
            hidden_icons: (!tray_state.notify.is_empty())
                .then_some(matches!(tray_state.open, Some(tray::Flyout::Hidden(_)))),
            time: tray_state.time,
            region: tray_state.region,
            overrides: tray_state.overrides,
            network: tray_state.network,
            vpn: tray_state.vpns.iter().any(|v| v.connected),
            audio: tray_state.audio,
            dark,
            material: transparency.then_some(material),
        },
        &taskbar::Extras {
            buttons: extras,
            marks,
            badge: tray_state.notices.badge,
            dnd: tray_state.notices.dnd,
        },
    );
    // Input indicator from the session's installed methods.
    let indicator = shell
        .text_input
        .indicator()
        .map(|i| taskbar::InputIndicator::new(i, shell.text_input.method()));
    let input_open = matches!(tray_state.open, Some(tray::Flyout::Input(_)));
    let bar_pointer = if shell.menu.is_some() {
        (-1, -1)
    } else {
        pointer
    };
    taskbar::draw_input(c, fonts, layout, indicator, bar_pointer, input_open, dark);
    // Notification toasts (bottom right), hidden while the centre is open.
    if tray_state.open != Some(tray::Flyout::Calendar) {
        door_window::shell_view::draw_toasts(
            c,
            fonts,
            layout,
            &tray_state.notices,
            dark,
            if shell.menu.is_some() {
                (-1, -1)
            } else {
                pointer
            },
        );
    }
    if shell.run.open {
        door_window::run::draw(c, fonts, &shell.run, layout, dark, pointer);
    }
    if shell.clipboard_panel.open {
        door_window::clipboard_view::draw(
            c,
            fonts,
            &shell.clipboard_panel,
            &shell.clipboard,
            layout,
            dark,
            pointer,
        );
    }
    if let (Some(badge), Some(feedback)) = (shell.dnd_badge, shell.dnd.feedback()) {
        door_window::clipboard_view::draw_feedback(c, fonts, &feedback, badge, dark);
    }
    if let Some(assist) = &shell.assist {
        assist.draw(
            c,
            fonts,
            dark,
            if shell.menu.is_some() {
                (-1, -1)
            } else {
                pointer
            },
            now,
        );
    }
    if let Some(bar) = &shell.snap {
        // Top-centred flyout sliding down and fading in (Windows 11); the
        // hovered pane (pointer or keyboard) shows the window's app icon.
        let (offset, opacity) = motion::slide(bar.motion.progress(now), snap::SLIDE);
        let moved = g::Rect::new(
            bar.rect.x(),
            bar.rect.y() + offset,
            bar.rect.width(),
            bar.rect.height(),
        )
        .unwrap();
        let hover = bar.hover.or_else(|| {
            (!bar.drag && shell.menu.is_none())
                .then(|| snap::hit(bar.rect, pointer.0, pointer.1))
                .flatten()
        });
        snap::draw(c, moved, hover, dark, bar.app);
        paint::fade(
            c,
            paint::physical(paint::grow(moved, 26, layout.output()), scale),
            underlay,
            opacity,
        );
    }
    if let Some(p) = &shell.preview {
        door_window::shell_view::draw_preview(c, fonts, p, dark, pointer);
    }
    if let Some(t) = &shell.tooltip {
        door_window::shell_view::draw_tooltip(c, fonts, t, dark);
    }
    if let Some(menu) = &shell.menu {
        door_window::menu::draw(c, fonts, menu, &paint::Palette::new(dark), pointer);
    }
    // "<app> is not responding": End task or Wait (Win32 ghost close).
    if let Some(prompt) = shell.hang.prompt() {
        door_window::not_responding::draw(c, fonts, prompt, layout.work_area(), dark, pointer);
    }
    // "Doors Security Alert": a topmost dialog, under User Account Control.
    if let Some(alert) = &shell.firewall_alert {
        door_window::firewall_alert::draw(c, fonts, alert, layout.work_area(), dark, pointer);
    }
    // User Account Control above everything, on a secure desktop: as on
    // Windows, the backdrop is the screen frozen as the prompt opened, so
    // nothing an app draws afterwards (a fake button, a moving decoy) shows
    // around the prompt.
    if let Some(consent) = &shell.uac {
        let pixels = c.pixels_mut();
        if shell.uac_backdrop.len() == pixels.len() {
            pixels.copy_from_slice(&shell.uac_backdrop);
        } else {
            shell.uac_backdrop = Vec::from(&*pixels);
        }
        door_window::uac::draw(c, fonts, consent, layout.output(), pointer);
    }
    // IME candidate window: topmost, under the composition caret, CJK
    // shaped by the shell's Unicode TextSystem. Never on the secure desktop.
    if let Some(text) = fonts.unicode().filter(|_| shell.uac.is_none()) {
        let work = paint::physical(layout.work_area(), scale);
        let (width, height, stride) = c.physical_dimensions();
        let area = w::Rect {
            x: 0,
            y: 0,
            width,
            height,
        };
        let mut target = door_window::text_input::candidate::Target {
            pixels: c.pixels_mut(),
            stride,
            area,
        };
        Desktop::paint_candidates(&shell.text_input, text, work, dark, &mut target);
    }
}
/// What a primary press landed on; activation happens on a matching release.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Press {
    Bar(Slot),
    Tray(tray::Action),
    Start(start::Target),
    Menu(usize, usize),
    MenuInside,
    Run(door_window::run::Target),
    /// Win+V clipboard history panel.
    Clipboard(door_window::clipboard_history::Target),
    Snap(usize, usize),
    SnapInside,
    /// Snap Assist thumbnail (candidate index) or its zone background.
    Assist(usize),
    /// A notification toast control (door.shell).
    Toast(door_window::shell_view::NoticeHit),
    /// A taskbar hover preview card or its close button.
    Preview(door_window::shell_view::PreviewHit),
    AssistInside,
    Desktop(usize),
    Empty,
}
impl Desktop {
    pub(super) fn taskbar_layout(&self) -> door_window::taskbar::Layout {
        let r = self.engine.output_rect();
        let s = self.engine.scale();
        let revealed = if self.shell.taskbar_revealed {
            door_window::taskbar::REVEALED
        } else {
            0
        } | if self.engine.fullscreen_focused() {
            door_window::taskbar::UNDER_FULLSCREEN
        } else {
            0
        };
        door_window::taskbar::Layout::with_controls(
            s.logical_len(r.width()),
            s.logical_len(r.height()),
            self.controls,
        )
        .with_taskbar(
            self.shell
                .prefs
                .get(door_service_protocol::preferences::TASKBAR)
                | revealed,
        )
    }
    pub(super) fn logical_pointer(&self) -> (i32, i32) {
        let s = self.engine.scale();
        (s.logical(self.x), s.logical(self.y))
    }
    /// Shell animation time (µs) from the compositor's calibrated clock.
    pub(super) fn shell_now(&mut self) -> u64 {
        let tick = door_sys::time().unwrap_or(0);
        self.transition_clock.now_ms(
            door_window::telemetry::cycles(),
            tick,
            door_abi::platform::MONOTONIC_TICK_MILLIS,
        ) * 1000
    }
    pub(super) fn taskbar_buttons(&self) -> Vec<taskbar::Button> {
        buttons(
            &self.shell.taskbar_pins,
            &items(&self.engine, &self.app_classes),
        )
    }
    /// Logical rectangle of `app`'s taskbar button (minimize/restore anchor).
    pub(super) fn taskbar_anchor(&self, app: App) -> Option<g::Rect> {
        let buttons = self.taskbar_buttons();
        let index = buttons.iter().position(|b| b.app == app)?;
        Some(
            self.taskbar_layout()
                .slot(Slot::App(index), self.taskbar_slot_count()),
        )
    }
    pub(super) fn icons_visible(&self) -> bool {
        self.controls.show_desktop_icons()
    }
    pub(super) fn desktop_icons(&self) -> Option<Icons> {
        self.icons_visible().then(|| Icons {
            order: self.shell.desktop_order,
            selected: self.desktop_selection.selected(),
            hover: self.icon_hover,
            controls: self.controls,
            recycle_full: self.recycle_full,
        })
    }
    /// Desktop icon under the pointer (hover), when icons are shown.
    pub(super) fn desktop_icon_hover(&self) -> Option<usize> {
        if !self.icons_visible()
            || self.overlay_contains()
            || self.engine.hit_test(self.x, self.y).is_some()
        {
            return None;
        }
        let area = self.taskbar_layout().work_area();
        let (x, y) = self.logical_pointer();
        let (x, y) = (x - area.x(), y - area.y());
        if !desktop_grid::HIT_X.contains(&x) {
            return None;
        }
        (0..desktop_grid::SLOTS).find(|i| icon_band(*i).contains(&y))
    }
    /// Shell surfaces are visible (open or animating): scene frames must be
    /// complete and the cache must hold the whole scene beneath them.
    pub(super) fn shell_overlay_visible(&self) -> bool {
        self.launcher
            || self.tray.open.is_some()
            || self.shell.start_motion.open
            || self.shell.flyout_motion.open
            || self.shell.menu.is_some()
            || self.shell.snap.is_some()
            || self.shell.run.open
            || self.shell.hang.prompt().is_some()
            || self.shell.firewall_alert.is_some()
            || self.shell.uac.is_some()
            || self.shell.clipboard_panel.open
            || self.shell.snip.is_some()
            || self.shell.magnifier.is_some()
            || self.shell.dnd_badge.is_some()
            || self.shell.text_input.candidates().is_some()
            || self.shell.assist.is_some()
            || self.tray.notices.toasts().next().is_some()
            || self.shell.preview.is_some()
            || self.shell.locked
            || self.shell_animating()
    }
    fn shell_animating(&self) -> bool {
        // `presented` keeps last frame's surfaces; a non-bar region means a
        // surface was on screen and may still be fading out.
        self.shell.presented.len() > 1
    }
    /// Bring transitions in line with the open flags (other modules close
    /// Start/flyouts by clearing `launcher`/`tray.open`) and schedule frames
    /// while anything animates.
    /// Mouse wheel over Start scrolls its page.
    pub(super) fn start_wheel(&mut self, notches: i32) {
        if self.clipboard_panel_wheel(notches) {
            return;
        }
        if self.tray.open == Some(tray::Flyout::Devices) && notches != 0 {
            let layout = self.taskbar_layout();
            let (x, y) = self.logical_pointer();
            match self.tray.quick_page {
                tray::QuickPage::Main if tray::pages::tiles_band(layout).contains(x, y) => {
                    self.tray.activate(if notches < 0 {
                        tray::Action::TilesDown
                    } else {
                        tray::Action::TilesUp
                    });
                    self.engine.invalidate();
                }
                tray::QuickPage::Accessibility => {
                    let max = tray::pages::access_scroll_max(layout);
                    self.tray.list_scroll = (self.tray.list_scroll - notches * 48).clamp(0, max);
                    self.engine.invalidate();
                }
                _ => (),
            }
            return;
        }
        if !self.launcher || self.shell.menu.is_some() {
            return;
        }
        let (x, y) = self.logical_pointer();
        let g = self.shell.start.geometry(self.taskbar_layout());
        if g.body().contains(x, y)
            && !self.shell.start.searching()
            && self.shell.start.wheel(&g, notches)
        {
            self.bar_dirty = true;
        }
    }
    pub(super) fn sync_shell(&mut self) {
        let now = self.shell_now();
        let reduce = self.controls.reduce_motion();
        self.tray.now = now;
        self.tray.magnifier = self.shell.magnifier.is_some();
        if self.presentation.vision_tick(now) {
            self.engine.invalidate();
        }
        // Tile pages scrolling, and the Cast page's searching bar.
        if self.tray.open == Some(tray::Flyout::Devices)
            && (self.tray.tile_scroll.animating(now)
                || self.tray.quick_page == tray::QuickPage::Cast)
        {
            self.engine.invalidate();
        }
        // Taskbar input indicator and its switcher follow the broker's
        // installed methods (hidden with a single keyboard, as in Windows).
        let input = self
            .shell
            .text_input
            .indicator()
            .map(|_| self.shell.text_input.preferences());
        if self.tray.input != input {
            self.tray.input = input;
            if matches!(self.tray.open, Some(tray::Flyout::Input(_))) {
                self.tray.open = input.map(|p| tray::Flyout::Input(p.count() as u8));
            }
            self.bar_dirty = true;
        }
        // A drag that ended without a drop (reset, capture loss) retires its
        // snap bar.
        if self.drag.is_none() && self.shell.snap.is_some_and(|s| s.drag) {
            self.shell.snap = None;
            self.engine.invalidate();
        }
        if self.shell.start_motion.open != self.launcher {
            self.shell.start_motion.set(self.launcher, now, reduce);
            // Start's own menus (power, account, view, app) close with it.
            let start_menu = self.shell.menu.as_ref().is_some_and(|m| {
                matches!(
                    m.tag,
                    menus::POWER | menus::USER | menus::START_VIEW | menus::START_ITEM
                )
            });
            if !self.launcher && start_menu {
                self.close_menu();
            }
        }
        let flyout = self.tray.open;
        if let Some(kind) = flyout
            && self.shell.flyout_kind != kind
        {
            // Switching flyouts swaps instantly, as in Windows.
            self.shell.flyout_kind = kind;
            self.shell.flyout_motion.jump(false);
            self.tray.backdrop.clear();
        }
        if self.shell.flyout_motion.open != flyout.is_some() {
            self.shell.flyout_motion.set(flyout.is_some(), now, reduce);
        }
        // A leg that just ended still owes its exact final frame: the last
        // animated frame may have been drawn just short of the end.
        // Taskbar hover plates follow the pointer (no plate under a menu).
        let hovered = if self.shell.menu.is_some() {
            None
        } else {
            let (x, y) = self.logical_pointer();
            let apps = self.taskbar_slot_count();
            self.taskbar_layout().hit_slot(x, y, apps)
        };
        if self.shell.bar_feedback.hover(hovered, now, reduce)
            || self.shell.bar_feedback.animating(now)
        {
            self.bar_dirty = true;
        }
        let settled =
            self.shell.start_motion.take_settled(now) | self.shell.flyout_motion.take_settled(now);
        if settled
            || self.shell.start_motion.animating(now)
            || self.shell.flyout_motion.animating(now)
            || self.shell.snap.is_some_and(|s| s.motion.animating(now))
        {
            self.bar_dirty = true;
        }
        // Snap Assist yields to Start, flyouts, Task View, the lock screen
        // and other desktops.
        if self.shell.assist.is_some()
            && (self.launcher
                || self.tray.open.is_some()
                || self.overview.is_some()
                || self.shell.locked
                || self.shell.assist_workspace != Some(self.engine.active_workspace()))
        {
            self.assist_dismiss();
        }
        if self
            .shell
            .assist
            .as_ref()
            .is_some_and(|a| a.motion.animating(now))
        {
            self.bar_dirty = true;
        }
        // A surface that finished closing still needs one dismissal frame.
        let start_gone = !self.shell.start_motion.visible(now);
        let flyout_gone = !self.shell.flyout_motion.visible(now);
        if start_gone
            && flyout_gone
            && self.shell.menu.is_none()
            && self.shell.snap.is_none()
            && !self.shell.run.open
            && !self.shell.clipboard_panel.open
            && self.shell.dnd_badge.is_none()
            && self.shell.text_input.candidates().is_none()
            && self.shell.assist.is_none()
            && self.shell.presented.len() > 1
        {
            self.bar_dirty = true;
        }
    }
    pub(super) fn remember_shell_focus(&mut self) {
        if self.shell_focus.is_none() {
            self.shell_focus =
                door_window::policy::ReturnFocus::capture(&self.engine, self.session);
        }
    }
    pub(super) fn restore_shell_focus(&mut self) {
        if let Some(id) = self
            .shell_focus
            .take()
            .and_then(|saved| saved.resolve(&self.engine, self.session))
            && self.engine.activate(id).is_ok()
        {
            self.manual_focus(Some(id));
        }
    }
    pub(super) fn manual_focus(&mut self, id: Option<g::WindowId>) {
        if id.is_some() && !self.launcher && self.tray.open.is_none() && self.overview.is_none() {
            self.shell_focus = None;
        }
        let app = id
            .and_then(|id| self.engine.window_info(id).ok())
            .and_then(|window| {
                self.app_classes
                    .iter()
                    .find(|(owner, _)| *owner == window.owner)
                    .map(|(_, app)| app.command())
            });
        self.foreground.manual(app);
    }
    pub(super) fn invalidate_icons(&mut self) {
        let area = self.taskbar_layout().work_area();
        let region = door_window::taskbar::physical(icon_column(area), self.engine.scale());
        if self.shell_overlay_visible()
            || self.engine.windows().any(|w| {
                w.state != g::WindowState::Minimized && w.outer.intersection(region).is_some()
            })
        {
            // Includes shadow margin. Never erase a window to repaint an icon.
            self.engine.invalidate_region(region);
        } else {
            self.icons_dirty = true;
        }
    }
    /// The pointer is over shell UI (bar, an open surface, menu, snap bar),
    /// or the session is locked.
    pub(super) fn overlay_contains(&self) -> bool {
        if self.overview.is_some() || self.shell.locked || self.shell.uac.is_some() {
            return true;
        }
        let (x, y) = self.logical_pointer();
        let layout = self.taskbar_layout();
        layout.bar().contains(x, y)
            || self
                .shell
                .menu
                .as_ref()
                .is_some_and(|m| m.hit(x, y) != MenuHit::Outside)
            || self.shell.snap.is_some_and(|s| s.rect.contains(x, y))
            || self.tray.open.is_some_and(|kind| {
                self.tray
                    .surfaces(kind, layout)
                    .iter()
                    .any(|r| r.contains(x, y))
            })
            || self.launcher && self.shell.start.geometry(layout).panel.contains(x, y)
            || self.shell.run.open && self.shell.run.panel(layout).contains(x, y)
            || self.shell.hang.prompt().is_some()
                && door_window::not_responding::hit(layout.work_area(), x, y).is_some()
            || self.shell.firewall_alert.is_some()
                && door_window::firewall_alert::hit(layout.work_area(), x, y).is_some()
            || self.clipboard_panel_contains(x, y)
            || self.shell.assist.as_ref().is_some_and(|a| a.contains(x, y))
            || self.tray.open != Some(tray::Flyout::Calendar)
                && door_window::shell_view::hit_toasts(layout, &self.tray.notices, x, y).is_some()
            || self
                .shell
                .preview
                .as_ref()
                .is_some_and(|p| p.rect.contains(x, y))
    }
    pub(super) fn activate_desktop_window(&mut self, id: g::WindowId) -> bool {
        let Ok(info) = self.engine.window_info(id) else {
            return false;
        };
        let (owner, state, old_epoch) = (info.owner, info.state, info.epoch);
        let workspace = info.workspace;
        if state == g::WindowState::Closing {
            return false;
        }
        if workspace != self.engine.active_workspace() {
            self.switch_desktop(workspace);
        }
        let changed = if state == g::WindowState::Minimized {
            self.engine.restore_window(owner, id)
        } else {
            Ok(())
        };
        if changed.is_ok() {
            if self
                .engine
                .window_info(id)
                .is_ok_and(|w| w.epoch != old_epoch)
            {
                self.attachments.retain(|a| a.window != id);
            }
            if self.engine.activate(id).is_ok() {
                self.launcher = false;
                self.presentation_log.activated();
                if let Some((_, app)) = self.app_classes.iter().find(|(who, _)| *who == owner) {
                    // Short: the window server's serial budget is 16 KiB.
                    let _ = door_sys::write(
                        format!("DOOR_P12 APP_ACTIVATED app={}\n", app.command()).as_bytes(),
                    );
                }
                return true;
            }
        }
        false
    }
    pub(super) fn sign_in(&mut self) {
        if self.session.is_some() || self.login_waiting {
            return;
        }
        // Init's sign-in command for the account the gate accepted.
        let command =
            if self.signin.gate.account() == door_service_protocol::credential::ACCOUNT_UIDS[1] {
                door_service_protocol::application::SIGN_IN_SECOND_COMMAND
            } else {
                8
            };
        if let Ok(mut init) = door_service_client::Init::connect(self.registry)
            && init
                .begin_desktop(command, door_sys::time().unwrap_or(0))
                .is_ok()
        {
            self.login = Some(init);
            self.login_waiting = true;
            let scale = self.engine.scale();
            let output = self.engine.output_rect();
            let button = door_window::login::Layout::new(
                scale.logical_len(output.width()),
                scale.logical_len(output.height()),
            )
            .button;
            self.engine
                .invalidate_region(door_window::taskbar::physical(button, scale));
        }
    }
    pub(super) fn refresh_desktop(&mut self) -> Result<(), Status> {
        let now = door_sys::time()?;
        if let Some(init) = self.login.as_mut() {
            match init.poll_desktop_command(now) {
                Ok(None) => (),
                Ok(Some(())) => {
                    self.login = None;
                }
                Err(_) => {
                    self.login = None;
                    self.login_waiting = false;
                    self.engine.invalidate();
                }
            }
        }
        if self.session.is_none() {
            return Ok(());
        }
        self.login = None;
        self.login_waiting = false;
        self.sync_shell();
        self.maximize_hover_tick(now);
        self.shell_tick();
        self.preview_tick(now);
        self.persist_tick(now);
        self.elevation_tick(now);
        self.file_search_tick(now);
        self.assist_tick();
        self.preferences_tick(now);
        self.network_tick(now);
        self.vpn_tick(now);
        self.firewall_tick(now);
        self.audio_tick(now);
        self.brightness_tick();
        if now >= self.tray.poll_at {
            self.tray.poll_at = now.saturating_add(100);
            if let Ok(utc) = door_sys::display_clock(self.display)
                && let Some(time) = self.tray.localize(utc)
            {
                let initial = self.tray.time.is_none();
                if self.tray.update(time) {
                    if self.tray.open.is_some() || self.shell.locked {
                        self.engine.invalidate();
                    } else {
                        self.bar_dirty = true;
                    }
                }
                if initial {
                    let _ = door_sys::write(
                        format!(
                            "DOOR_P12 CLOCK UTC {:04}-{:02}-{:02} {:02}:{:02}:{:02}\n",
                            utc.year, utc.month, utc.day, utc.hour, utc.minute, utc.second
                        )
                        .as_bytes(),
                    );
                    let zone = door_locale::tz::zone_or_utc(u32::from(self.tray.zone));
                    let region =
                        door_locale::region::region_or_default(u32::from(self.tray.region));
                    let (line1, line2) = taskbar::clock_text(time, self.tray.region);
                    let _ = door_sys::write(
                        format!(
                            "DOOR_P12 CLOCK LOCAL zone={} region={} {line2} {line1}\n",
                            zone.iana, region.tag
                        )
                        .as_bytes(),
                    );
                }
            }
        }
        if let Some(command) = self.foreground.pending(now) {
            // The elevated Terminal answers its own command, never Terminal's.
            let admin = self.admin_terminal;
            let ready = items(&self.engine, &self.app_classes)
                .into_iter()
                .find(|item| {
                    let elevated = Some(item.owner) == admin;
                    (if elevated {
                        command == door_service_protocol::application::ELEVATED_TERMINAL_COMMAND
                    } else {
                        item.app.command() == command
                    }) && self.engine.has_snapshot(item.id)
                });
            if ready.is_some_and(|item| self.activate_desktop_window(item.id)) {
                self.foreground.complete();
            }
        }
        if self.unfreeze_pending && self.desktop_command(7).is_ok() {
            self.unfreeze_pending = false;
            self.shutdown_frozen = false;
        }
        let Some((session, _, _)) = self.session else {
            return Ok(());
        };
        if let Some((init, command)) = self.launching.as_mut() {
            match init.poll_desktop_command(now) {
                Ok(None) => (),
                result => {
                    if let Err(error) = result {
                        let _ = door_sys::write(
                            format!("DOOR_P12 LAUNCH_REJECTED app={command} error={error:?}\n")
                                .as_bytes(),
                        );
                    }
                    self.launching = None;
                    self.catalog_at = 0;
                }
            }
        }
        if self.launching.is_none() && self.pending_launches != 0 && self.shutdown.is_none() {
            let index = self.pending_launches.trailing_zeros() as u8;
            // Bits past the apps request the search-only tools; bit 31 the
            // elevated Terminal.
            let command = match App::all().get(index as usize) {
                Some(app) => app.command(),
                None if index == ADMIN_TERMINAL_BIT => {
                    door_service_protocol::application::ELEVATED_TERMINAL_COMMAND
                }
                None => door_window::apps::Tool::ALL[index as usize - App::all().len()].command(),
            };
            if let Ok(mut init) = door_service_client::Init::connect_now(self.registry)
                && init.begin_desktop(command, now).is_ok()
            {
                self.pending_launches &= !(1 << index);
                self.launching = Some((init, command));
            }
        }
        // Init can be busy constructing an address space. Never sleep waiting
        // for its catalog: cursor/input and client requests continue meanwhile.
        if self.catalog.is_none() && now >= self.catalog_at {
            self.catalog_at = now.saturating_add(10);
            if let Ok(mut init) = door_service_client::Init::connect_now(self.registry)
                && init.begin_desktop_state_v2(now).is_ok()
            {
                self.catalog = Some(init);
            }
        }
        let Some(init) = self.catalog.as_mut() else {
            return Ok(());
        };
        let data = match init.poll_desktop_state_v2(now) {
            Ok(None) => return Ok(()),
            Ok(Some(data)) => data,
            Err(_) => {
                self.catalog = None;
                return Ok(());
            }
        };
        self.catalog = None;
        self.catalog_fresh = true;
        // Start search shows tools that cannot start for lack of a process.
        let blocked = self.blocked_tools();
        if blocked != self.shell.start.blocked_tools {
            self.shell.start.blocked_tools = blocked;
            self.engine.invalidate();
        }
        let mut classes = Vec::new();
        for (index, process) in data.processes[..data.count].iter().enumerate() {
            if let Some(process) = process
                && let Some(app) = App::all().get(index)
            {
                classes.push((
                    g::Owner {
                        pid: process.pid,
                        session,
                        epoch: process.instance,
                    },
                    *app,
                ));
            }
        }
        let host = data
            .processes
            .get(door_service_protocol::application::SEARCH_HOST_SLOT)
            .copied()
            .flatten()
            .filter(|_| data.count > door_service_protocol::application::SEARCH_HOST_SLOT)
            .map(|p| g::Owner {
                pid: p.pid,
                session,
                epoch: p.instance,
            });
        if host != self.shell.files.host {
            self.shell.files.host = host;
            self.bar_dirty = true;
        }
        let admin = data
            .processes
            .get(door_service_protocol::application::ELEVATED_TERMINAL_SLOT)
            .copied()
            .flatten()
            .filter(|_| data.count > door_service_protocol::application::ELEVATED_TERMINAL_SLOT)
            .map(|p| g::Owner {
                pid: p.pid,
                session,
                epoch: p.instance,
            });
        if let Some(owner) = admin {
            classes.push((owner, App::Terminal));
        }
        self.admin_terminal = admin;
        if classes != self.app_classes {
            self.app_classes = classes;
            self.engine.invalidate();
        }
        let power = data.power;
        if matches!(power, 1 | 2) {
            self.begin_shutdown(power == 2);
        }
        Ok(())
    }
    fn begin_shutdown(&mut self, reboot: bool) {
        self.shell_focus = None;
        if self.shutdown.is_some() {
            return;
        }
        let windows: Vec<_> = self
            .engine
            .windows()
            .filter(|w| w.role == g::Role::TopLevel && w.state != g::WindowState::Closing)
            .map(|w| (w.owner, w.id))
            .collect();
        let now = door_sys::time().unwrap_or(0);
        for (owner, id) in windows {
            if self.close_for(owner, id, true).is_err() {
                return;
            }
        }
        self.shutdown = Some(reboot);
        self.shell.shutdown_gate.clear();
        self.pending_launches = 0;
        self.foreground.complete();
        self.shutdown_since = now;
        self.power_retry = door_window::policy::RetryGate::default();
        self.shutdown_frozen = false;
        self.unfreeze_pending = false;
        self.catalog_fresh = false;
        self.launcher = false;
        self.close_menu();
        self.engine.invalidate();
    }
    pub(super) fn finish_shutdown(&mut self) {
        let Some(reboot) = self.shutdown else {
            return;
        };
        let now = door_sys::time().unwrap_or(0);
        if !self.shutdown_frozen {
            if self.power_retry.claim(now) && self.desktop_command(6).is_ok() {
                self.shutdown_frozen = true;
                self.catalog_fresh = false;
                self.catalog_at = 0;
                self.catalog = None;
            }
            return;
        }
        // Apps asking about unsaved work keep the shutdown waiting; one that
        // exited (Save or Don't save) no longer does.
        let live: Vec<g::Owner> = self
            .clients
            .iter()
            .filter(|c| !c.revoked)
            .map(|c| c.owner)
            .collect();
        self.shell.shutdown_gate.retain_live(|o| live.contains(&o));
        // A live catalogued process can be reconnecting with unsaved state.
        // Close its newly recreated window rather than treating an empty scene
        // as permission to revoke the entire session. An app showing its close
        // prompt is never closed again or terminated while it asks.
        let windows: Vec<_> = self
            .engine
            .windows()
            .filter(|w| w.role == g::Role::TopLevel && w.state != g::WindowState::Closing)
            .filter(|w| !self.shell.shutdown_gate.waiting_on(w.owner))
            .map(|w| (w.owner, w.id))
            .collect();
        for (owner, id) in windows {
            let _ = self.close_for(owner, id, true);
        }
        if now.saturating_sub(self.shutdown_since) >= 1000 {
            let missing: Vec<_> = self
                .app_classes
                .iter()
                .filter(|(owner, _)| !self.engine.windows().any(|w| w.owner == *owner))
                .filter(|(owner, _)| !self.shell.shutdown_gate.waiting_on(*owner))
                .map(|v| v.0)
                .collect();
            for owner in missing {
                self.terminations.push(owner, now);
            }
        }
        if self.catalog_fresh
            && self.app_classes.is_empty()
            && self.terminations.is_empty()
            && !self.engine.windows().any(|w| w.role == g::Role::TopLevel)
            && self.power_retry.claim(now)
        {
            // Init forcibly retires this optional desktop service after power
            // is accepted. Snapshot before the first final RPC attempt; Drop
            // cannot run after forced retirement. The record labels this cutoff.
            self.report_motion(door_window::telemetry::Cutoff::PowerAttempt);
            let command = if self.signing_out {
                door_service_protocol::application::SIGN_OUT_COMMAND
            } else if reboot {
                5
            } else {
                4
            };
            if self.desktop_command(command).is_ok() {
                self.shutdown = None;
                self.signing_out = false;
            }
        }
    }
    /// Cancel in an app's close prompt during a shutdown (Windows 11): the
    /// shutdown stops, the session is unfrozen and stays fully usable; apps
    /// that already closed stay closed.
    pub(super) fn cancel_shutdown(&mut self, by: g::Owner) {
        if self.shutdown.take().is_none() {
            return;
        }
        self.signing_out = false;
        self.shell.shutdown_gate.clear();
        self.unfreeze_pending = true;
        self.power_retry = door_window::policy::RetryGate::default();
        self.engine.invalidate();
        let _ = door_sys::write(format!("DOOR_P12 SHUTDOWN CANCELLED pid={}\n", by.pid).as_bytes());
    }
    fn desktop_command(&mut self, command: u8) -> door_service_client::Result<()> {
        door_service_client::Init::connect(self.registry)?.desktop_command(command)
    }
    pub(super) fn open_app(&mut self, app: App) {
        self.shell_focus = None;
        if self.overview.take().is_some() {
            self.engine.invalidate();
        }
        let _ = if self.key_modifiers & door_abi::graphics::SHIFT != 0 {
            self.launch_window(app)
        } else {
            self.launch_app(app)
        };
    }
    /// Open Settings at a route (`door_window::search::page`), launching or
    /// activating it; the page arrives as SETTINGS_NAVIGATE.
    /// Settings at `page`; a route Windows elevates (Registry Editor,
    /// System Configuration, the management snap-ins) asks for
    /// administrator approval first.
    pub(super) fn open_settings(&mut self, page: u32) {
        if let Some(program) = door_settings_map::elevation(page) {
            let location = match program {
                "Registry Editor" => "C:\\Windows\\regedit.exe",
                "System Configuration" => "C:\\Windows\\System32\\msconfig.exe",
                _ => "C:\\Windows\\System32\\mmc.exe",
            };
            self.ask_consent(door_window::uac::Consent::new(
                program,
                Some(door_window::uac::PUBLISHER),
                location,
                door_window::uac::Icon::App(App::Settings),
                door_window::uac::Then::Settings(page),
            ));
            return;
        }
        self.open_settings_now(page);
    }
    /// Show the consent prompt (replacing none: one at a time, as Windows
    /// serializes elevation requests).
    pub(super) fn ask_consent(&mut self, consent: door_window::uac::Consent) {
        if self.shell.uac.is_some() {
            return;
        }
        let consent = self.consent_for_session(consent);
        self.close_start();
        self.tray.open = None;
        self.shell.menu = None;
        self.shell.run.open = false;
        let area = self.taskbar_layout().output();
        let g = door_window::uac::geometry(area, &consent);
        let scale = self.engine.scale();
        let r = |r: g::Rect| {
            let r = door_window::paint::physical(r, scale);
            alloc::format!("{},{},{},{}", r.x(), r.y(), r.width(), r.height())
        };
        let _ = door_sys::write(
            format!(
                "DOOR_P16 UAC PROMPT program={} publisher={} yes={} no={}\n",
                consent.program,
                consent.publisher.as_deref().unwrap_or("Unknown"),
                r(g.yes),
                r(g.no)
            )
            .as_bytes(),
        );
        self.shell.uac = Some(consent);
        self.bar_dirty = true;
        self.engine.invalidate();
    }
    /// The consent prompt was answered: approval runs what it guarded.
    pub(super) fn uac_answer(&mut self, answer: door_window::uac::Answer) {
        // A standard user's Yes checks the administrator's password first.
        if answer == door_window::uac::Answer::Yes
            && self
                .shell
                .uac
                .as_ref()
                .is_some_and(|c| c.credential.is_some())
        {
            if !self.signin.uac_checking {
                self.uac_check();
            }
            return;
        }
        if self.signin.uac_checking {
            // Declined while checking: the result is dropped.
            self.signin.uac_checking = false;
            if let Some((_, _, uid)) = self.session {
                door_window::login::Backend::select(&mut self.signin.service, uid);
            }
        }
        let Some(consent) = self.shell.uac.take() else {
            return;
        };
        let yes = answer == door_window::uac::Answer::Yes;
        // Back to the live desktop.
        self.shell.uac_backdrop = Vec::new();
        let _ = door_sys::write(
            format!("DOOR_P16 UAC ANSWER {}\n", if yes { "yes" } else { "no" }).as_bytes(),
        );
        self.bar_dirty = true;
        self.engine.invalidate();
        if !yes {
            if let door_window::uac::Then::Program { directory, .. } = consent.then {
                let _ = door_sys::close(directory);
                self.elevation_settled(Elevated::Declined);
            }
            return;
        }
        match consent.then {
            door_window::uac::Then::Settings(page) => self.open_settings_now(page),
            door_window::uac::Then::Firewall(body) => self.firewall_store(&body),
            door_window::uac::Then::Terminal(shell) => {
                let mut payload = alloc::vec::Vec::from(&b"admin:"[..]);
                payload.extend_from_slice(shell.payload());
                let _ = self.deliver_elevated(&payload);
            }
            door_window::uac::Then::Script(path) => {
                let mut payload = alloc::vec::Vec::from(&b"admin:"[..]);
                payload.extend_from_slice(path.as_bytes());
                let _ = self.deliver_elevated(&payload);
                let _ =
                    door_sys::write(format!("DOOR_P16 ELEVATED SCRIPT path={path}\n").as_bytes());
            }
            door_window::uac::Then::Program {
                path,
                cwd,
                directory,
                image,
                arguments,
            } => {
                let now = door_sys::time().unwrap_or(0);
                let started =
                    door_service_client::Init::connect(self.registry).and_then(|mut init| {
                        init.begin_execute_elevated(
                            &image, &path, &cwd, &arguments, directory, now,
                        )?;
                        Ok(init)
                    });
                match started {
                    Ok(init) => self.shell.elevated_launch = Some((init, directory, path)),
                    Err(_) => {
                        let _ = door_sys::close(directory);
                        let _ = door_sys::write(
                            format!("DOOR_P16 ELEVATED LAUNCH failed path={path}\n").as_bytes(),
                        );
                        self.elevation_settled(Elevated::Failed);
                    }
                }
            }
        }
    }
    /// Init's answer to an approved Run as administrator.
    fn elevation_tick(&mut self, now: u64) {
        let Some((init, _, _)) = self.shell.elevated_launch.as_mut() else {
            return;
        };
        let result = match init.poll_execute_elevated(now) {
            Ok(None) => return,
            result => result,
        };
        let Some((_, directory, path)) = self.shell.elevated_launch.take() else {
            return;
        };
        let _ = door_sys::close(directory);
        let line = match result {
            Ok(Some(child)) => {
                // The requester gets a handle to wait on and query (its
                // ShellExecuteEx hProcess); this server keeps none.
                let given = door_sys::duplicate(
                    child,
                    door_abi::rights::WAIT | door_abi::rights::QUERY | door_abi::rights::TRANSFER,
                );
                let _ = door_sys::close(child);
                match given {
                    Ok(handle) => self.elevation_settled(Elevated::Started(handle)),
                    Err(_) => self.elevation_settled(Elevated::Failed),
                }
                format!("DOOR_P16 ELEVATED LAUNCH started path={path}\n")
            }
            _ => {
                self.elevation_settled(Elevated::Failed);
                format!("DOOR_P16 ELEVATED LAUNCH failed path={path}\n")
            }
        };
        let _ = door_sys::write(line.as_bytes());
    }
    /// The requester's Run as administrator ended: its ELEVATION_STATUS
    /// answers this once.
    fn elevation_settled(&mut self, outcome: Elevated) {
        /// Outcomes kept for requesters that have not asked yet; the
        /// oldest goes first.
        const KEPT: usize = 8;
        if let Some(owner) = self.shell.elevation_requester.take() {
            self.elevation_forget(owner);
            if self.shell.elevation_outcomes.len() == KEPT
                && let (_, Elevated::Started(old)) = self.shell.elevation_outcomes.remove(0)
            {
                let _ = door_sys::close(old);
            }
            self.shell.elevation_outcomes.push((owner, outcome));
        } else if let Elevated::Started(handle) = outcome {
            let _ = door_sys::close(handle);
        }
    }
    /// Drop client `owner`'s uncollected outcome (and its process handle).
    pub(super) fn elevation_forget(&mut self, owner: u64) {
        self.shell.elevation_outcomes.retain(|&(o, outcome)| {
            if o != owner {
                return true;
            }
            if let Elevated::Started(handle) = outcome {
                let _ = door_sys::close(handle);
            }
            false
        });
    }
    /// Client `owner` is gone: its pending request has no one to answer and
    /// its outcome no one to collect.
    pub(super) fn elevation_revoked(&mut self, owner: u64) {
        if self.shell.elevation_requester == Some(owner) {
            self.shell.elevation_requester = None;
        }
        self.elevation_forget(owner);
    }
    /// `window::ELEVATION_STATUS` for client `owner`.
    pub(super) fn elevation_status(&mut self, owner: u64) -> Response {
        let found = self
            .shell
            .elevation_outcomes
            .iter()
            .position(|&(o, _)| o == owner);
        match found {
            Some(index) => {
                let (_, outcome) = self.shell.elevation_outcomes.remove(index);
                match outcome {
                    Elevated::Started(handle) => Response::handle(
                        handle,
                        door_abi::rights::WAIT | door_abi::rights::QUERY,
                        true,
                    ),
                    Elevated::Declined => Response::scalar(w::elevation::DECLINED),
                    Elevated::Failed => Response::scalar(w::elevation::FAILED),
                }
            }
            _ if self.shell.elevation_requester == Some(owner) => Response::error(Code::WouldBlock),
            _ => Response::error(Code::NotFound),
        }
    }
    fn open_settings_now(&mut self, page: u32) {
        self.close_start();
        self.tray.open = None;
        self.settings_navigation = Some(page);
        self.shell_focus = None;
        let _ = self.launch_app(App::Settings);
        let _ = door_sys::write(format!("DOOR_P12 SETTINGS_LINK page={page}\n").as_bytes());
    }
    pub(super) fn launch_window(&mut self, app: App) -> door_service_client::Result<()> {
        if self.shutdown.is_some() {
            return Err(door_service_client::Error::Service(Code::Closing));
        }
        if let Some(owner) = self
            .app_classes
            .iter()
            .find(|(_, a)| *a == app)
            .map(|(o, _)| *o)
        {
            let client = self
                .clients
                .iter_mut()
                .find(|c| c.owner == owner && !c.revoked && c.multiwindow)
                .ok_or(door_service_client::Error::Service(Code::Unsupported))?;
            if client.new_windows >= 4 {
                return Err(door_service_client::Error::Service(Code::WouldBlock));
            }
            client.new_windows += 1;
            Ok(())
        } else {
            self.launch_app(app)
        }
    }
    pub(super) fn launch_app(&mut self, app: App) -> door_service_client::Result<()> {
        if self.shutdown.is_some() {
            return Err(door_service_client::Error::Service(Code::Closing));
        }
        self.foreground.request(app.command(), door_sys::time()?);
        let admin = self.admin_terminal;
        if let Some(item) = all_items(&self.engine, &self.app_classes)
            .iter()
            .find(|item| item.app == app && Some(item.owner) != admin)
        {
            if self.engine.has_snapshot(item.id) && self.activate_desktop_window(item.id) {
                self.foreground.complete();
            }
        } else {
            self.app_workspaces[app.index()] = Some(self.engine.active_workspace());
            self.pending_launches |= 1 << app.index();
            let was_open = self.launcher;
            self.launcher = false;
            if was_open {
                self.engine.invalidate();
            }
        }
        Ok(())
    }
    /// Start search tool (gallery, Captions): Init starts it once per
    /// session; an already running tool keeps its window (Init ignores a
    /// second launch).
    /// Tools (bit = `Tool::index`) that cannot start: the kernel runs at
    /// most `MAX_PROCESSES` (32) and the five services plus every connected
    /// client process (apps, search host, IME, tools) already fill them. A
    /// running tool is never blocked (activating it brings it forward).
    pub(super) fn blocked_tools(&self) -> u16 {
        const SERVICES: usize = 5;
        let live = self.clients.iter().filter(|c| !c.revoked);
        if SERVICES + live.clone().count() < door_abi::telemetry::MAX_PROCESSES {
            return 0;
        }
        door_window::apps::Tool::ALL
            .into_iter()
            .filter(|t| !live.clone().any(|c| c.manifest == t.manifest()))
            .fold(0, |m, t| m | 1 << t.index())
    }
    pub(super) fn launch_tool(&mut self, tool: door_window::apps::Tool) {
        if self.shutdown.is_some() {
            return;
        }
        self.shell_focus = None;
        // Its first window takes the foreground (CREATE), as a launched app's.
        self.foreground
            .request(tool.command(), door_sys::time().unwrap_or(0));
        self.pending_launches |= 1 << (App::all().len() + tool.index());
        self.engine.invalidate();
    }
    /// A hashable identity of the shell element under the pointer: hover
    /// repaints happen only when it changes.
    pub(super) fn hover_target(&self) -> Option<usize> {
        if self.session.is_none() || self.shell.locked {
            return None;
        }
        let (x, y) = self.logical_pointer();
        let layout = self.taskbar_layout();
        if let Some(menu) = &self.shell.menu {
            match menu.hit(x, y) {
                MenuHit::Item(depth, index) => return Some(100_000 + depth * 1000 + index),
                MenuHit::Inside => return Some(99_999),
                MenuHit::Outside => (),
            }
        }
        if let Some(target) = self.shell.run.hit(layout, x, y) {
            use door_window::run::Target as R;
            return Some(match target {
                R::Row(i) => 80_000 + i,
                R::Input => 80_100,
                R::Ok => 80_101,
                R::Panel => 80_104,
            });
        }
        if let Some(target) = self.clipboard_panel_hit(x, y) {
            use door_window::clipboard_history::Target as C;
            return Some(match target {
                C::Item(id) => 70_000 + self.clipboard_index(id),
                C::More(id) => 70_100 + self.clipboard_index(id),
                C::Panel => 70_200,
                C::TurnOn => 70_201,
                C::ClearAll => 70_202,
                C::Pin => 70_203,
                C::Delete => 70_204,
            });
        }
        if let Some(bar) = &self.shell.snap
            && bar.rect.contains(x, y)
        {
            return Some(90_000 + snap::hit(bar.rect, x, y).map_or(999, |(l, z)| l * 10 + z));
        }
        if let Some(assist) = &self.shell.assist
            && assist.contains(x, y)
        {
            return Some(95_000 + assist.hit(x, y).map_or(999, |i| i));
        }
        if let Some(p) = &self.shell.preview
            && let Some(hit) = door_window::shell_view::hit_preview(p, x, y)
        {
            use door_window::shell_view::PreviewHit as P;
            return Some(match hit {
                P::Window(id) => 61_000 + (id as usize & 0xff) * 2,
                P::Close(id) => 61_001 + (id as usize & 0xff) * 2,
                P::Inside => 61_999,
            });
        }
        if self.tray.open != Some(tray::Flyout::Calendar)
            && let Some(hit) = door_window::shell_view::hit_toasts(layout, &self.tray.notices, x, y)
        {
            use door_window::shell_view::NoticeHit as N;
            return Some(match hit {
                N::Body(id) => 60_000 + (id as usize & 0xff) * 8,
                N::Button(id, i) => 60_001 + (id as usize & 0xff) * 8 + usize::from(i),
                N::Close(id) => 60_004 + (id as usize & 0xff) * 8,
                _ => 60_005,
            });
        }
        let apps = self.taskbar_slot_count();
        if let Some(slot) = layout.hit_slot(x, y, apps) {
            return Some(match slot {
                Slot::Start => 1,
                Slot::Search => 2,
                Slot::TaskView => 3,
                Slot::App(i) => 10 + i,
            });
        }
        if let Some(t) = layout.hit_tray(x, y) {
            return Some(40 + t as usize);
        }
        if self.launcher {
            let g = self.shell.start.geometry(layout);
            if let Some(target) = self.shell.start.hit(&g, &self.fonts, x, y) {
                return Some(1000 + start_key(target));
            }
        }
        if self.tray.open.is_some() {
            let (w, h) = layout.size();
            if let Some(action) = self.tray.hit_with_controls(w, h, x, y, self.controls)
                && action != tray::Action::Dismiss
            {
                // Hover tracks the pointer position inside flyouts (tiles,
                // calendar arrows); quantize to 8 px cells.
                return Some(50_000 + ((y / 8) as usize & 0xff) * 256 + ((x / 8) as usize & 0xff));
            }
        }
        None
    }
    pub(super) fn close_menu(&mut self) {
        if self.shell.menu.take().is_some() {
            self.shell.menu_app = None;
            self.bar_dirty = true;
        }
    }
    pub(super) fn close_start(&mut self) {
        if self.launcher {
            self.launcher = false;
            self.close_menu();
            self.bar_dirty = true;
        }
    }
    /// Toggle Start, or with `search` the 26H2 Search panel (taskbar Search,
    /// Win+S). Asking for the other panel while one is open switches to it;
    /// asking for the open one closes it.
    pub(super) fn toggle_start(&mut self, search: bool) {
        self.close_menu();
        self.overview = None;
        self.tray.open = None;
        if self.desktop_selection.active() {
            self.gesture_note("start", 0);
        }
        self.desktop_selection.cancel_gesture();
        let mode = if search {
            start::Mode::Search
        } else {
            start::Mode::Start
        };
        if self.launcher && self.shell.start.mode == mode {
            self.launcher = false;
            self.restore_shell_focus();
        } else {
            if !self.launcher {
                self.remember_shell_focus();
                self.shell.start.open_mode(mode);
                self.shell.start.backdrop.clear();
            } else {
                // Start <-> Search: same surface, new panel geometry.
                self.shell.start.open_mode(mode);
                self.shell.start.backdrop.clear();
            }
            let _ = door_sys::write(
                format!(
                    "DOOR_P12 SHELL_PANEL open={}\n",
                    if search { "search" } else { "start" }
                )
                .as_bytes(),
            );
            self.launcher = true;
            self.shell.start.focus = Some(start::Target::Search);
            self.shell.start.caret = true;
            self.manual_focus(None);
            self.engine.clear_focus();
        }
        self.sync_shell();
        self.engine.invalidate();
    }
    pub(super) fn open_menu(&mut self, mut menu: Menu, anchor: Anchor, app: Option<App>) {
        let layout = self.taskbar_layout();
        let area = layout.output();
        menu.place(&self.fonts, anchor, area);
        self.shell.menu = Some(menu);
        self.shell.menu_app = app;
        self.remember_shell_focus();
        self.engine.clear_focus();
        self.engine.invalidate();
    }
    fn open_submenu(&mut self, depth: usize, index: usize) {
        let icons = self.icons_visible();
        let sorted = self.shell.sorted;
        let output = self.taskbar_layout().output();
        let fonts = &self.fonts;
        let Some(menu) = self.shell.menu.as_mut() else {
            return;
        };
        let Some(parent) = menu.at_depth_mut(depth) else {
            return;
        };
        let Some(row) = parent.row(index) else {
            return;
        };
        let Some(item) = parent.item(index) else {
            return;
        };
        let Command::Submenu(id) = item.command else {
            parent.child = None;
            return;
        };
        if parent.child.is_some() && parent.focus == Some(index) {
            return;
        }
        let style = parent.style;
        if let Some(mut child) = menus::submenu(id, style, icons, sorted) {
            child.place(fonts, Anchor::Beside(row), output);
            parent.focus = Some(index);
            parent.child = Some(alloc::boxed::Box::new(child));
            self.engine.invalidate();
        }
    }
    /// Execute a menu command with a real backend.
    fn run_command(&mut self, command: Command) {
        let point = self.shell.menu_point;
        let classic = self
            .shell
            .menu
            .as_ref()
            .is_some_and(|m| m.style == Style::Classic);
        if !matches!(command, Command::Submenu(_)) {
            self.close_menu();
        }
        match command {
            Command::Launch(app) => {
                self.close_start();
                self.open_app(app);
            }
            Command::NewWindow(app) => {
                self.close_start();
                let running = all_items(&self.engine, &self.app_classes)
                    .iter()
                    .any(|i| i.app == app);
                if !running || self.launch_window(app).is_err() {
                    self.open_app(app);
                }
            }
            Command::PinTaskbar(app) => {
                if !self.shell.taskbar_pins.contains(&app) {
                    self.shell.taskbar_pins.push(app);
                }
            }
            Command::UnpinTaskbar(app) => self.shell.taskbar_pins.retain(|a| *a != app),
            Command::PinStart(app) => {
                if !self.shell.start.pins.contains(&app) {
                    self.shell.start.pins.push(app);
                }
            }
            Command::UnpinStart(app) => self.shell.start.pins.retain(|a| *a != app),
            Command::MoveToFront(app) => {
                self.shell.start.pins.retain(|a| *a != app);
                self.shell.start.pins.insert(0, app);
            }
            Command::CloseWindows(app) => {
                let windows: Vec<_> = items(&self.engine, &self.app_classes)
                    .into_iter()
                    .filter(|i| i.app == app)
                    .map(|i| (i.owner, i.id))
                    .collect();
                for (owner, id) in windows {
                    let _ = self.close(owner, id);
                }
            }
            Command::Settings(page) => self.open_settings(page),
            Command::Refresh => self.engine.invalidate(),
            Command::ToggleDesktopIcons => {
                // Written through to the user's profile like Settings: Init
                // stores it, then SET_DESKTOP_CONTROLS applies it everywhere.
                self.shell.controls_pending = Some(self.controls.with(
                    door_service_protocol::desktop::Controls::SHOW_DESKTOP_ICONS,
                    !self.icons_visible(),
                ));
                self.desktop_selection.clear();
                self.icon_hover = None;
                self.engine.invalidate();
            }
            Command::SortByName => {
                self.shell.sorted = true;
                self.shell.desktop_order = door_window::apps::alphabetical();
                self.desktop_selection.clear();
                self.engine.invalidate();
            }
            Command::ShowMoreOptions => {
                if !classic {
                    self.open_menu(
                        menus::desktop(true, true),
                        Anchor::Point(point.0, point.1),
                        None,
                    );
                }
            }
            Command::StartView(view) => {
                self.shell.start.view = start::View::from_index(view);
                self.shell.start.scroll = 0;
                self.shell.start.focus = Some(start::Target::View);
            }
            Command::Shutdown => self.begin_shutdown(false),
            Command::Restart => self.begin_shutdown(true),
            Command::SignOut => {
                // Windows' sign out: every app is asked to close (unsaved
                // work can still cancel it), then the session ends.
                if self.shutdown.is_none() {
                    self.begin_shutdown(false);
                    self.signing_out = self.shutdown.is_some();
                    let _ = door_sys::write(b"DOOR_P16 SIGNOUT requested\n");
                }
            }
            Command::Lock => self.lock(),
            Command::Search => self.toggle_start(true),
            Command::AdminTerminal => {
                self.close_start();
                // Windows names Terminal itself on the prompt.
                self.ask_consent(door_window::uac::Consent::new(
                    "Terminal",
                    Some(door_window::uac::PUBLISHER),
                    "C:\\Windows\\System32\\doorsh.exe",
                    door_window::uac::Icon::App(App::Terminal),
                    door_window::uac::Then::Terminal(door_window::apps::Shell::Doorsh),
                ));
            }
            Command::Run => {
                self.close_start();
                self.open_run();
            }
            Command::ShowDesktop => {
                self.close_start();
                self.tray.open = None;
                self.toggle_desktop();
            }
            Command::RecentLocation(index) => self.recent_open(usize::from(index), true),
            Command::RecentRemove(index) => self.recent_remove(usize::from(index)),
            Command::Shell(command, key) => {
                self.close_start();
                if self.shell_link.invoke(command, 0, key, 0) {
                    let _ = door_sys::write(
                        format!(
                            "DOOR_P14 SHELL invoke a={command} b={key}
"
                        )
                        .as_bytes(),
                    );
                }
            }
            Command::CloseExtra(key) => self.close_extra(key),
            Command::EndTask(app) => {
                let owners: Vec<_> = items(&self.engine, &self.app_classes)
                    .into_iter()
                    .filter(|i| i.app == app)
                    .map(|i| i.owner)
                    .collect();
                self.end_task(owners);
            }
            Command::EndTaskExtra(key) => {
                let owners: Vec<_> = self
                    .taskbar_extras()
                    .into_iter()
                    .filter(|e| e.key == key)
                    .flat_map(|e| e.ids)
                    .filter_map(|id| self.engine.window_info(id).ok().map(|w| w.owner))
                    .collect();
                self.end_task(owners);
            }
            Command::RecycleBin(action) => self.open_recycle_bin(action),
            Command::Submenu(_) | Command::Nothing => (),
        }
        self.persist_shell_layout();
        self.bar_dirty = true;
        self.engine.invalidate();
    }
    /// Win+X (or right-click Start): quick-link menu above the Start button.
    pub(super) fn open_quick_link(&mut self) {
        self.close_menu();
        self.close_start();
        self.tray.open = None;
        let layout = self.taskbar_layout();
        let start = layout.slot(Slot::Start, self.taskbar_slot_count());
        let edge = layout.controls().edge();
        self.open_menu(
            menus::quick_link(),
            Anchor::Bar(start, edge, door_window::menu::BarAlign::Start),
            None,
        );
        if let Some(menu) = self.shell.menu.as_mut() {
            menu.step(true);
        }
    }
    /// Win+M: minimize every window on the active desktop.
    pub(super) fn minimize_all(&mut self) {
        let ws = self.engine.active_workspace();
        let ids: Vec<_> = self
            .engine
            .windows()
            .filter(|w| {
                w.workspace == ws
                    && w.role == g::Role::TopLevel
                    && matches!(w.state, g::WindowState::Normal | g::WindowState::Maximized)
            })
            .map(|w| (w.id, w.owner))
            .collect();
        for (id, owner) in ids {
            if self
                .engine
                .request_state(owner, id, g::WindowState::Minimized)
                .is_ok()
                && !self.hidden_windows.contains(&id)
            {
                self.hidden_windows.push(id);
            }
        }
        self.manual_focus(None);
        self.engine.invalidate();
    }
    /// Win+R: the Fluent Run dialog above the Start corner.
    pub(super) fn open_run(&mut self) {
        self.close_menu();
        self.close_start();
        self.tray.open = None;
        self.overview = None;
        self.remember_shell_focus();
        self.engine.clear_focus();
        self.manual_focus(None);
        self.shell.run.show();
        self.engine.invalidate();
    }
    fn close_run(&mut self) {
        self.shell.run.open = false;
        self.engine.invalidate();
    }
    fn run_action(&mut self, action: door_window::run::Action) {
        self.engine.invalidate();
        let _ = door_sys::write(
            format!("DOOR_P12 RUN {}\n", door_window::run::action_label(action)).as_bytes(),
        );
        match action {
            door_window::run::Action::App(app) => self.open_app(app),
            door_window::run::Action::Tool(tool) => self.launch_tool(tool),
            door_window::run::Action::Settings(page) => self.open_settings(page),
            // File Explorer resolves the typed folder as its address bar does
            // (and reports one it can't find).
            door_window::run::Action::Path => {
                let mut payload = alloc::vec![b'T'];
                payload.extend_from_slice(self.shell.run.accepted.as_bytes());
                self.open_files_request(&payload);
            }
            door_window::run::Action::Snip => {
                self.start_snip(door_window::snip::Mode::Rectangle, 0, None)
            }
            // Terminal opens on a tab of that shell.
            door_window::run::Action::Profile(shell) => {
                let _ = self.deliver_document(App::Terminal, shell.payload());
            }
            // `accept` reports these in the dialog and runs nothing.
            door_window::run::Action::Missing | door_window::run::Action::Web => (),
        }
    }
    fn run_accept(&mut self) {
        if let Some(action) = self.shell.run.accept() {
            self.run_action(action);
        }
        self.bar_dirty = true;
    }
    /// Ctrl+Shift+Enter in Run: the command with administrator rights
    /// (User Account Control first). A shell opens in the elevated
    /// Terminal; anything else runs as Enter would.
    fn run_accept_elevated(&mut self) {
        match self.shell.run.accept() {
            Some(door_window::run::Action::Profile(shell)) => {
                let _ = door_sys::write(b"DOOR_P16 RUN elevated\n");
                self.shell_as_administrator(shell);
            }
            Some(action) => self.run_action(action),
            None => (),
        }
        self.bar_dirty = true;
    }
    fn run_target(&mut self, target: door_window::run::Target) {
        use door_window::run::Target as R;
        match target {
            R::Row(i) => {
                self.shell.run.selected = Some(i);
                self.run_accept();
            }
            R::Ok => self.run_accept(),
            R::Input | R::Panel => (),
        }
        self.bar_dirty = true;
    }
    fn run_key(&mut self, scan: u32) -> bool {
        let alt = self.key_modifiers & door_abi::graphics::ALT != 0;
        match scan {
            1 => {
                self.close_run();
                self.restore_shell_focus();
                self.shortcut_keys |= 4;
            }
            62 if alt => {
                self.close_run();
                self.restore_shell_focus();
                self.shortcut_keys |= 2;
            }
            62 => {
                self.shell.run.expanded = !self.shell.run.expanded;
                self.engine.invalidate();
            }
            28 | 0x11c
                if self.key_modifiers & door_abi::graphics::CONTROL != 0
                    && self.key_modifiers & door_abi::graphics::SHIFT != 0 =>
            {
                self.run_accept_elevated()
            }
            28 | 0x11c => self.run_accept(),
            0x148 => self.shell.run.step(false),
            0x150 => self.shell.run.step(true),
            _ => (),
        }
        self.bar_dirty = true;
        true
    }
    pub(super) fn lock(&mut self) {
        self.close_menu();
        self.launcher = false;
        self.tray.open = None;
        self.overview = None;
        self.shell.snap = None;
        self.drag = None;
        self.caption_press = None;
        // Nothing staged, dragged or shown from the clipboard survives a lock.
        self.shell.clipboard.lock();
        self.dnd_cancel();
        self.shell.clipboard_panel.open = false;
        self.remember_shell_focus();
        self.engine.clear_focus();
        self.manual_focus(None);
        self.prepare_lock_backdrop();
        self.shell.locked = true;
        self.shell.lock_pressed = false;
        self.engine.invalidate();
        let _ = door_sys::write(b"DOOR_P12 LOCKED\n");
    }
    // Unlocking is the credential gate's alone (`signin.rs`, P14-LOGON):
    // locked input is routed to the gate before the shell sees it, and no
    // key or click here can end a lock.
    /// Typing goes to the Run dialog or Start search: Unicode key text (the
    /// session layout) and IME commits at the caret, grapheme-safe
    /// Backspace/Delete and caret keys. `scan` 0 is a commit.
    pub(super) fn desktop_search(&mut self, scan: u32, text: &str) -> bool {
        let printable = text.chars().any(|ch| !ch.is_control());
        if self.shell.run.open && self.shell.menu.is_none() {
            let run = &mut self.shell.run;
            if scan == 14 {
                run.backspace();
            } else if !run.edit_key(scan) {
                if !printable {
                    return false;
                }
                run.type_text(text);
            }
            self.bar_dirty = true;
            return true;
        }
        if !self.launcher || self.shell.menu.is_some() {
            return false;
        }
        let start = &mut self.shell.start;
        let mode = start.mode;
        let changed = if scan == 14 {
            start.backspace();
            true
        } else if start.edit_key(scan) {
            true
        } else if printable {
            start.type_text(text)
        } else {
            return false;
        };
        if changed {
            self.bar_dirty = true;
        }
        self.panel_mode_changed(mode);
        true
    }
    /// Typing in Start switched to the Search panel (or clearing that query
    /// went back): repaint the whole surface and its acrylic for the new
    /// panel geometry.
    pub(super) fn panel_mode_changed(&mut self, before: start::Mode) {
        if self.shell.start.mode != before {
            self.shell.start.backdrop.clear();
            self.engine.invalidate();
        }
    }
    fn menu_key(&mut self, scan: u32) -> bool {
        let Some(menu) = self.shell.menu.as_mut() else {
            return false;
        };
        let depth = menu.depth();
        let Some(current) = menu.at_depth_mut(depth) else {
            return true;
        };
        match scan {
            1 => {
                if depth > 0 {
                    if let Some(parent) = menu.at_depth_mut(depth - 1) {
                        parent.child = None;
                    }
                } else {
                    self.close_menu();
                    if !self.launcher {
                        self.restore_shell_focus();
                    }
                    self.shortcut_keys |= 4;
                }
            }
            0x148 => current.step(false),
            0x150 | 15 => current.step(true),
            0x14b if depth > 0 => {
                if let Some(parent) = menu.at_depth_mut(depth - 1) {
                    parent.child = None;
                }
            }
            0x14d => {
                if let Some(index) = current.focus {
                    self.open_submenu(depth, index);
                    if let Some(child) = self
                        .shell
                        .menu
                        .as_mut()
                        .and_then(|m| m.at_depth_mut(depth + 1))
                    {
                        child.step(true);
                    }
                }
            }
            28 | 0x11c | 57 => {
                if let Some(command) = current.focused_command() {
                    if let Command::Submenu(_) = command {
                        let index = current.focus.unwrap_or(0);
                        self.open_submenu(depth, index);
                        if let Some(child) = self
                            .shell
                            .menu
                            .as_mut()
                            .and_then(|m| m.at_depth_mut(depth + 1))
                        {
                            child.step(true);
                        }
                    } else {
                        self.run_command(command);
                    }
                }
            }
            _ => (),
        }
        self.engine.invalidate();
        true
    }
    pub(super) fn desktop_key(&mut self, scan: u32) -> bool {
        let shift = self.key_modifiers & door_abi::graphics::SHIFT != 0;
        let control = self.key_modifiers & door_abi::graphics::CONTROL != 0;
        // Ctrl+Shift+Esc opens Task Manager (Monitor) from anywhere.
        if scan == 1 && control && shift {
            self.close_menu();
            self.close_start();
            self.shortcut_keys |= 4;
            self.open_app(App::Monitor);
            return true;
        }
        if scan == 0x15b || scan == 0x15c {
            self.toggle_start(false);
            return true;
        }
        if self.menu_key_active() {
            return self.menu_key(scan);
        }
        if let Some(assist) = self.shell.assist.as_mut() {
            match assist.key(scan) {
                door_window::snap_assist::Key::Moved => {
                    self.bar_dirty = true;
                    return true;
                }
                door_window::snap_assist::Key::Pick(index) => {
                    self.assist_pick(index);
                    return true;
                }
                door_window::snap_assist::Key::Dismiss => {
                    self.assist_dismiss();
                    self.shortcut_keys |= 4;
                    return true;
                }
                // Any other key closes the chooser and reaches the app.
                door_window::snap_assist::Key::Ignored => self.assist_dismiss(),
            }
        }
        if self
            .shell
            .snap
            .is_some_and(|s| snap::flyout_takes_keys(s.from_hover, s.drag))
        {
            return self.snap_key(scan);
        }
        if self.shell.run.open {
            return self.run_key(scan);
        }
        if self.shell.clipboard_panel.open {
            return self.clipboard_panel_key(scan);
        }
        if scan == 1 && self.tray.open.is_some() {
            self.tray.open = None;
            self.restore_shell_focus();
            self.shortcut_keys |= 4;
            self.engine.invalidate();
            return true;
        }
        // Tab, arrows, Enter and Space inside an open tray flyout.
        if self.tray.open.is_some() && self.tray_key(scan, shift) {
            return true;
        }
        let context_key = scan == 0x15d || scan == 68 && shift;
        if !self.launcher {
            if self.engine.focused_window().is_none() && self.overview.is_none() {
                let order = self.shell.desktop_order;
                match scan {
                    _ if context_key => {
                        let area = self.taskbar_layout().work_area();
                        let item = self
                            .desktop_selection
                            .selected()
                            .filter(|_| self.icons_visible())
                            .and_then(|i| DesktopItem::at(i, &order));
                        let menu = match item {
                            Some(item) => self.desktop_item_menu(item),
                            None if self.controls.classic_context_menu() => {
                                menus::desktop_win10()
                            }
                            None => menus::desktop(
                                // Shift+F10 is "Show more options": the classic menu.
                                self.controls.classic_context_menu() || scan == 68,
                                !self.controls.has(door_service_protocol::desktop::Controls::CONTEXT_HIDE_MORE_OPTIONS),
                            ),
                        };
                        let point = (area.x() + 96, area.y() + 24);
                        self.shell.menu_point = point;
                        let app = item.and_then(DesktopItem::app);
                        self.open_menu(menu, Anchor::Point(point.0, point.1), app);
                        if let Some(m) = self.shell.menu.as_mut() {
                            m.step(true);
                        }
                        return true;
                    }
                    28 | 0x11c if self.icons_visible() => {
                        if let Some(item) = self
                            .desktop_selection
                            .selected()
                            .and_then(|i| DesktopItem::at(i, &order))
                        {
                            self.open_desktop_item(item);
                            return true;
                        }
                    }
                    1 => {
                        self.desktop_selection.clear();
                        self.invalidate_icons();
                        return true;
                    }
                    0x148 | 0x150 if self.icons_visible() => {
                        let current =
                            self.desktop_selection
                                .selected()
                                .unwrap_or(if scan == 0x148 {
                                    0
                                } else {
                                    desktop_grid::SLOTS - 1
                                });
                        self.desktop_selection.select(
                            (current
                                + if scan == 0x148 {
                                    desktop_grid::SLOTS - 1
                                } else {
                                    1
                                })
                                % desktop_grid::SLOTS,
                        );
                        self.invalidate_icons();
                        return true;
                    }
                    _ => (),
                }
            }
            return false;
        }
        // Start keyboard.
        let layout = self.taskbar_layout();
        let g = self.shell.start.geometry(layout);
        let key = match scan {
            0x148 => Some(0),
            0x150 => Some(1),
            0x14b => Some(2),
            0x14d => Some(3),
            15 => Some(if shift { 5 } else { 4 }),
            _ => None,
        };
        if let Some(key) = key {
            let fonts = &self.fonts;
            self.shell.start.navigate(&g, fonts, key);
            self.bar_dirty = true;
            return true;
        }
        match scan {
            1 => {
                let mode = self.shell.start.mode;
                if self.shell.start.escape() {
                    self.launcher = false;
                    self.restore_shell_focus();
                    self.shortcut_keys |= 4;
                    self.engine.invalidate();
                } else if mode != self.shell.start.mode {
                    // Search typed in Start backed out: Start again.
                    self.shell.start.backdrop.clear();
                    self.engine.invalidate();
                }
                self.bar_dirty = true;
            }
            28 | 0x11c => {
                // A focused control (pill, result, action, tile) first;
                // from the search box, the chosen result.
                let focused = self
                    .shell
                    .start
                    .focus
                    .filter(|t| !matches!(t, start::Target::Search | start::Target::Panel));
                let target = if focused.is_some() {
                    focused
                } else if self.shell.start.searching() {
                    self.shell.start.enter_target()
                } else {
                    None
                };
                if let Some(target) = target {
                    self.start_activate(target);
                }
            }
            _ if context_key => {
                if let Some(target) = self.shell.start.focus
                    && let Some(r) = self.shell.start.rect(&g, &self.fonts, target)
                {
                    self.start_context(target, (r.x() + 16, r.y() + 16));
                }
            }
            _ => (),
        }
        true
    }
    fn menu_key_active(&self) -> bool {
        self.shell.menu.is_some()
    }
    /// A shell with administrator rights: User Account Control, then the
    /// elevated Terminal (Start's Run as administrator, Run's
    /// Ctrl+Shift+Enter).
    fn shell_as_administrator(&mut self, shell: door_window::apps::Shell) {
        let location = match shell {
            door_window::apps::Shell::CommandPrompt => "C:\\Windows\\System32\\cmd.exe",
            _ => "C:\\Windows\\System32\\doorsh.exe",
        };
        self.ask_consent(door_window::uac::Consent::new(
            shell.label(),
            Some(door_window::uac::PUBLISHER),
            location,
            door_window::uac::Icon::Shell(shell),
            door_window::uac::Then::Terminal(shell),
        ));
    }
    pub(super) fn start_activate(&mut self, target: start::Target) {
        let layout = self.taskbar_layout();
        let g = self.shell.start.geometry(layout);
        let fonts = &self.fonts;
        let rect = self.shell.start.rect(&g, fonts, target);
        match self.shell.start.activate(target) {
            start::Activation::Launch(app) => {
                self.launcher = false;
                self.open_app(app);
            }
            start::Activation::Shell(shell) => {
                let _ = self.deliver_document(App::Terminal, shell.payload());
            }
            start::Activation::RunAsAdministrator(shell) => self.shell_as_administrator(shell),
            start::Activation::LaunchTool(tool) => {
                // With every kernel process slot taken the launch would fail
                // in Init; Start keeps showing why instead.
                self.shell.start.blocked_tools = self.blocked_tools();
                if self.shell.start.blocked_tools & (1 << tool.index()) != 0 {
                    self.engine.invalidate();
                    return;
                }
                self.launcher = false;
                self.launch_tool(tool);
            }
            start::Activation::LaunchRegistered(key) => {
                self.launcher = false;
                self.launch_registered(key);
            }
            start::Activation::Settings(page) => self.open_settings(page),
            start::Activation::Folder(i) => {
                self.close_start();
                self.open_start_folder(i);
            }
            start::Activation::PowerMenu => {
                if let Some(r) = rect {
                    self.open_menu(menus::power(), Anchor::AboveRight(r), None);
                }
            }
            start::Activation::UserMenu => {
                if let Some(r) = rect {
                    self.open_menu(menus::user(), Anchor::AboveLeft(r), None);
                }
            }
            start::Activation::ViewMenu => {
                if let Some(r) = rect {
                    let view = self.shell.start.view;
                    self.open_menu(menus::start_view(view), Anchor::Below(r), None);
                }
            }
            start::Activation::OpenFile(index) => self.search_open(index, false),
            start::Activation::OpenFileLocation(index) => self.search_open(index, true),
            start::Activation::OpenRecent(index) => self.recent_open(index, false),
            start::Activation::ExploreFiles => self.search_explore(),
            start::Activation::Redraw | start::Activation::Nothing => (),
        }
        self.bar_dirty = true;
        self.engine.invalidate();
    }
    fn start_context(&mut self, target: start::Target, point: (i32, i32)) {
        if let start::Target::Recent(index) = target {
            if index < self.shell.start.recent.len() {
                self.shell.menu_point = point;
                self.open_menu(
                    menus::recent_item(index),
                    Anchor::Point(point.0, point.1),
                    None,
                );
            }
            return;
        }
        let Some(app) = self.shell.start.app(target) else {
            return;
        };
        let pinned_index = match target {
            start::Target::Pinned(i) => Some(i),
            _ => None,
        };
        let menu = menus::start_item(
            app,
            pinned_index,
            self.shell.start.pins.contains(&app),
            self.shell.taskbar_pins.contains(&app),
        );
        self.shell.menu_point = point;
        self.open_menu(menu, Anchor::Point(point.0, point.1), Some(app));
    }
    /// Taskbar app button click: launch, activate, cycle, or minimize the
    /// active window as Windows does.
    pub(super) fn taskbar_app(&mut self, app: App) {
        if self.key_modifiers & door_abi::graphics::SHIFT != 0 {
            self.open_app(app);
            return;
        }
        let ws = self.engine.active_workspace();
        let windows: Vec<(g::WindowId, g::Owner, bool, g::WindowState)> = self
            .engine
            .windows()
            .filter(|w| {
                w.role == g::Role::TopLevel
                    && w.state != g::WindowState::Closing
                    && w.workspace == ws
            })
            .filter(|w| {
                self.app_classes
                    .iter()
                    .any(|(o, a)| *o == w.owner && *a == app)
            })
            .map(|w| (w.id, w.owner, w.active, w.state))
            .collect();
        if windows.is_empty() {
            self.open_app(app);
            return;
        }
        // Doors extra "last active window": a group acts like one window (a
        // click on the active group minimizes it, never cycles).
        let last_active = self
            .shell
            .prefs
            .get(door_service_protocol::preferences::TASKBAR)
            & door_service_protocol::preferences::taskbar::LAST_ACTIVE
            != 0;
        if let Some(active) = windows.iter().position(|w| w.2) {
            if windows.len() == 1 || last_active {
                let (id, owner, ..) = windows[active];
                let _ = self
                    .engine
                    .request_state(owner, id, g::WindowState::Minimized);
                self.manual_focus(None);
            } else {
                // Cycle to the app's next window (engine order: back to front).
                let next = windows[(active + windows.len() - 1) % windows.len()].0;
                self.activate_desktop_window(next);
                self.manual_focus(Some(next));
            }
        } else if let Some(top) = windows.last() {
            let id = top.0;
            self.activate_desktop_window(id);
            self.manual_focus(Some(id));
        }
    }
    /// Win+1..9: activate or launch the Nth taskbar app.
    pub(super) fn taskbar_number(&mut self, n: usize) {
        let buttons = self.taskbar_buttons();
        if let Some(button) = buttons.get(n) {
            let app = button.app;
            self.close_start();
            self.taskbar_app(app);
        } else if let Some(extra) = self.taskbar_extra(n) {
            self.close_start();
            self.extra_click(&extra);
        }
    }
    /// Pointer target for the primary button at the current position.
    fn press_target(&self) -> Option<Press> {
        let (x, y) = self.logical_pointer();
        let layout = self.taskbar_layout();
        if let Some(menu) = &self.shell.menu {
            return Some(match menu.hit(x, y) {
                MenuHit::Item(d, i) => Press::Menu(d, i),
                MenuHit::Inside => Press::MenuInside,
                MenuHit::Outside => return None,
            });
        }
        if let Some(target) = self.shell.run.hit(layout, x, y) {
            return Some(Press::Run(target));
        }
        if let Some(target) = self.clipboard_panel_hit(x, y) {
            return Some(Press::Clipboard(target));
        }
        if let Some(bar) = &self.shell.snap
            && bar.rect.contains(x, y)
        {
            return Some(
                snap::hit(bar.rect, x, y).map_or(Press::SnapInside, |(l, z)| Press::Snap(l, z)),
            );
        }
        if let Some(assist) = &self.shell.assist
            && assist.contains(x, y)
        {
            return Some(assist.hit(x, y).map_or(Press::AssistInside, Press::Assist));
        }
        if let Some(p) = &self.shell.preview
            && let Some(hit) = door_window::shell_view::hit_preview(p, x, y)
        {
            return Some(Press::Preview(hit));
        }
        if self.tray.open != Some(tray::Flyout::Calendar)
            && let Some(hit) = door_window::shell_view::hit_toasts(layout, &self.tray.notices, x, y)
        {
            return Some(Press::Toast(hit));
        }
        let apps = self.taskbar_slot_count();
        if let Some(slot) = layout.hit_slot(x, y, apps) {
            return Some(Press::Bar(slot));
        }
        let (w, h) = layout.size();
        if let Some(action) = self.tray.hit_with_controls(w, h, x, y, self.controls)
            && action != tray::Action::Dismiss
        {
            return Some(Press::Tray(action));
        }
        if self.launcher {
            let g = self.shell.start.geometry(layout);
            if let Some(target) = self.shell.start.hit(&g, &self.fonts, x, y) {
                return Some(Press::Start(target));
            }
        }
        if layout.bar().contains(x, y) {
            return Some(Press::Empty);
        }
        if self.engine.hit_test(self.x, self.y).is_none() {
            if let Some(i) = self.desktop_icon_hover() {
                return Some(Press::Desktop(i));
            }
            return Some(Press::Empty);
        }
        None
    }
    pub(super) fn desktop_button(&mut self, down: bool, time: u64) -> bool {
        if self.engine.captured_window().is_some() || self.drag.is_some() {
            return false;
        }
        let (x, y) = self.logical_pointer();
        let layout = self.taskbar_layout();
        if down {
            // Outside a menu: dismiss it; the press itself is consumed.
            if let Some(menu) = &self.shell.menu
                && menu.hit(x, y) == MenuHit::Outside
            {
                self.close_menu();
                self.engine.invalidate();
                self.desktop_press = None;
                return true;
            }
            // Outside the Run dialog: close it and pass the press through.
            if self.shell.run.open && !self.shell.run.panel(layout).contains(x, y) {
                self.shell.run.open = false;
                self.engine.invalidate();
            }
            // Outside the Win+V panel: close it and pass the press through.
            if self.shell.clipboard_panel.open && !self.clipboard_panel_contains(x, y) {
                self.close_clipboard_panel(false);
            }
            // Windows: the window's own caption buttons act even while the
            // hover snap layouts flyout shows (Maximize maximizes or
            // restores); the flyout closes and never takes that press, and it
            // stays closed until the pointer leaves the button.
            if let Some(bar) = self.shell.snap.filter(|s| !s.drag)
                && snap::flyout_press(&self.engine, bar.window, bar.rect, self.x, self.y)
                    == snap::FlyoutPress::Caption
            {
                self.shell.snap = None;
                self.engine.invalidate();
            }
            if let Some((id, g::HitRegion::Maximize)) = self.engine.hit_test(self.x, self.y) {
                self.shell.max_hover.pressed(id);
            }
            // Outside Snap Assist: dismiss it and pass the press through.
            if self
                .shell
                .assist
                .as_ref()
                .is_some_and(|a| !a.contains(x, y))
            {
                self.assist_dismiss();
            }
            // Outside a non-drag snap flyout: dismiss it and pass through.
            if self
                .shell
                .snap
                .is_some_and(|s| !s.drag && !s.rect.contains(x, y))
            {
                self.shell.snap = None;
                self.engine.invalidate();
            }
            // Outside dismissal passes this very press through to the actual app.
            // It must never raise the saved origin over a newly clicked target.
            if self.tray.open.is_some()
                && !layout.bar().contains(x, y)
                && self.tray.open.is_some_and(|kind| {
                    !self
                        .tray
                        .surfaces(kind, layout)
                        .iter()
                        .any(|r| r.contains(x, y))
                })
            {
                self.tray.open = None;
                self.shell_focus = None;
                self.engine.invalidate();
                if !self.launcher {
                    return false;
                }
            }
        }
        let target = self.press_target();
        if down {
            if self.launcher
                && !layout.bar().contains(x, y)
                && !self.shell.start.geometry(layout).panel.contains(x, y)
            {
                // Outside click closes Start and reaches whatever is beneath.
                self.launcher = false;
                self.shell_focus = None;
                self.engine.invalidate();
                if self.engine.hit_test(self.x, self.y).is_some() {
                    return false;
                }
            }
            if matches!(
                target,
                Some(Press::Bar(Slot::Start | Slot::Search) | Press::Tray(_))
            ) {
                self.remember_shell_focus();
            } else if !matches!(
                target,
                Some(Press::Start(_) | Press::Menu(..) | Press::MenuInside | Press::Clipboard(_))
            ) {
                self.shell_focus = None;
            }
            if let Some(Press::Desktop(index)) = target {
                self.desktop_selection.press(index, self.x, self.y);
                self.invalidate_icons();
            } else {
                if self.desktop_selection.active() && target != Some(Press::Empty) {
                    self.gesture_note("press-elsewhere", 0);
                }
                self.desktop_selection.cancel_gesture();
                if target == Some(Press::Empty)
                    && self.desktop_selection.selected().is_some()
                    && !layout.bar().contains(x, y)
                {
                    self.desktop_selection.clear();
                    self.invalidate_icons();
                }
            }
            let pressed_slot = match target {
                Some(Press::Bar(slot)) => Some(slot),
                _ => None,
            };
            if self.shell.bar_pressed != pressed_slot {
                self.shell.bar_pressed = pressed_slot;
                self.bar_dirty = true;
            }
            let (now, reduce) = (self.shell_now(), self.controls.reduce_motion());
            if self.shell.bar_feedback.press(pressed_slot, now, reduce) {
                self.bar_dirty = true;
            }
            // Windows dismisses a button's hover thumbnails when it is
            // pressed; they stay away until the pointer rests on a button
            // again.
            match pressed_slot {
                Some(Slot::App(i)) => self.suppress_preview(i),
                Some(_) => self.suppress_tooltip(),
                None => (),
            }
            // Hovering a submenu parent opens it on press too.
            if let Some(Press::Menu(d, i)) = target {
                self.open_submenu(d, i);
            }
            // The volume slider follows the pointer from the press on.
            if target == Some(Press::Tray(tray::Action::Volume)) {
                self.tray.volume_drag = true;
            }
            if target == Some(Press::Tray(tray::Action::Brightness)) {
                self.tray.brightness_drag = true;
            }
            self.desktop_press = target;
            if target.is_some() {
                // A taskbar app button keeps the active window focused until
                // the release decides: clicking the active app minimizes it.
                if !matches!(target, Some(Press::Bar(Slot::App(_)))) {
                    self.engine.clear_focus();
                }
                return true;
            }
            return false;
        }
        if core::mem::take(&mut self.tray.volume_drag) {
            self.engine.invalidate();
        }
        if core::mem::take(&mut self.tray.brightness_drag) {
            self.engine.invalidate();
        }
        if self.shell.bar_pressed.take().is_some() {
            self.bar_dirty = true;
        }
        let (now, reduce) = (self.shell_now(), self.controls.reduce_motion());
        if self.shell.bar_feedback.press(None, now, reduce) {
            self.bar_dirty = true;
        }
        let Some(pressed) = self.desktop_press.take() else {
            if matches!(target, Some(Press::Desktop(_))) {
                self.gesture_note("press-lost", 0);
            }
            if let Some(Press::Bar(Slot::App(i))) = target {
                let _ = door_sys::write(
                    format!("DOOR_P14 TASKBAR RELEASE slot={i} pressed=none\n").as_bytes(),
                );
            }
            return false;
        };
        if let Press::Bar(Slot::App(i)) = pressed {
            let _ = door_sys::write(
                format!(
                    "DOOR_P14 TASKBAR RELEASE slot={i} same={}\n",
                    u8::from(target == Some(pressed))
                )
                .as_bytes(),
            );
        }
        if target == Some(pressed) {
            match pressed {
                Press::Bar(Slot::Start) => {
                    self.overview = None;
                    self.toggle_start(false);
                }
                // 26H2: the Search panel (switching from Start if open).
                Press::Bar(Slot::Search) => self.toggle_start(true),
                Press::Bar(Slot::TaskView) => {
                    self.close_start();
                    self.system_command(door_window::policy::SystemCommand::TaskView)
                }
                Press::Bar(Slot::App(i)) => {
                    if let Some(button) = self.taskbar_buttons().get(i).copied() {
                        self.close_start();
                        self.tray.open = None;
                        self.taskbar_app(button.app);
                    } else if let Some(extra) = self.taskbar_extra(i) {
                        self.close_start();
                        self.tray.open = None;
                        self.extra_click(&extra);
                    }
                }
                Press::Desktop(index) => {
                    use door_window::policy::Release;
                    match self
                        .desktop_selection
                        .release_outcome(index, time, self.x, self.y)
                    {
                        Release::Open => {
                            if let Some(item) = DesktopItem::at(index, &self.shell.desktop_order) {
                                self.open_desktop_item(item);
                            }
                        }
                        Release::First => (),
                        outcome => {
                            let dt = match outcome {
                                Release::TooSlow(dt) => dt,
                                _ => 0,
                            };
                            self.gesture_note(outcome.reason(), dt);
                        }
                    }
                    return true;
                }
                Press::Start(target) => {
                    if matches!(target, start::Target::Search) {
                        self.shell.start.focus = Some(start::Target::Search);
                        self.shell.start.caret = true;
                    } else {
                        self.start_activate(target);
                    }
                }
                Press::Menu(depth, index) => {
                    let command = self
                        .shell
                        .menu
                        .as_ref()
                        .and_then(|m| m.at_depth(depth))
                        .and_then(|m| m.item(index))
                        .filter(|i| i.enabled)
                        .map(|i| i.command);
                    match command {
                        Some(Command::Submenu(_)) => self.open_submenu(depth, index),
                        Some(command) => self.run_command(command),
                        None => (),
                    }
                }
                Press::Snap(l, z) => self.apply_snap(l, z),
                Press::Assist(index) => self.assist_pick(index),
                Press::Toast(hit) => self.notice_action(hit, true),
                Press::Preview(hit) => self.preview_action(hit),
                Press::Run(target) => self.run_target(target),
                Press::Clipboard(target) => self.clipboard_target(target),
                Press::Tray(action) => self.tray_action(action),
                Press::MenuInside | Press::SnapInside | Press::AssistInside | Press::Empty => (),
            }
            self.engine.invalidate();
        }
        self.desktop_selection.cancel_gesture();
        true
    }
    /// Quick settings > Accessibility: Magnifier starts or closes the
    /// Magnifier app, Color filters flips the VISION filter bit, Mono audio
    /// the output's mono flag.
    pub(super) fn access_toggle(&mut self, row: tray::AccessRow) {
        match row {
            tray::AccessRow::Magnifier => {
                // On while Magnifier holds the magnified view; off closes
                // its windows (the view ends with the process).
                if let Some(pid) = self.shell.magnifier.map(|m| m.pid) {
                    let windows: Vec<(g::Owner, g::WindowId)> = self
                        .engine
                        .windows()
                        .filter(|w| w.owner.pid == pid)
                        .map(|w| (w.owner, w.id))
                        .collect();
                    for (owner, id) in windows {
                        let _ = self.close(owner, id);
                    }
                } else {
                    self.launch_tool(door_window::apps::Tool::Magnifier);
                }
            }
            tray::AccessRow::ColorFilters => {
                use door_service_protocol::preferences::vision as v;
                self.set_vision(
                    self.shell
                        .prefs
                        .get(door_service_protocol::preferences::VISION)
                        ^ v::FILTER,
                );
            }
            tray::AccessRow::MonoAudio => {
                let output = self.sound_output();
                self.set_sound_output(door_service_protocol::audio::Output {
                    mono: !output.mono,
                    ..output
                });
            }
            tray::AccessRow::Narrator | tray::AccessRow::LiveCaptions => (),
        }
        self.engine.invalidate();
    }
    pub(super) fn tray_action(&mut self, action: tray::Action) {
        self.overview = None;
        self.launcher = false;
        match action {
            tray::Action::KeyboardSettings => {
                self.tray.open = None;
                self.open_settings(door_window::search::page::LANGUAGE_REGION);
                return;
            }
            tray::Action::Microphone => {
                self.tray.open = None;
                self.open_settings(door_settings_map::Page::Microphone.index());
                return;
            }
            tray::Action::NotifyIcon(index) => {
                self.notify_icon_click(index, false);
                return;
            }
            tray::Action::InputMethod(index) => {
                // Windows: choosing a method switches and closes the
                // switcher; focus returns to the app that had it.
                self.tray.open = None;
                self.restore_shell_focus();
                self.select_input_method(index);
                self.sync_shell();
                return;
            }
            tray::Action::NightLight => {
                use door_service_protocol::preferences::vision as v;
                self.set_vision(
                    self.shell
                        .prefs
                        .get(door_service_protocol::preferences::VISION)
                        ^ v::NIGHT_LIGHT,
                );
                return;
            }
            tray::Action::Vpn if self.tray.vpns.is_empty() => {
                // No VPN profile yet: Windows opens Settings to add one.
                self.open_settings(door_window::search::page::NETWORK);
                return;
            }
            tray::Action::Vpn => {
                self.tray.quick_page = tray::QuickPage::Vpn;
                return;
            }
            tray::Action::AccessToggle(row) => {
                self.access_toggle(row);
                return;
            }
            tray::Action::CastHelp => {
                self.open_settings(door_window::search::page::DISPLAY);
                return;
            }
            tray::Action::MoreSettings => {
                let page = match self.tray.quick_page {
                    tray::QuickPage::Accessibility => door_window::search::page::ACCESSIBILITY,
                    tray::QuickPage::Vpn => door_window::search::page::NETWORK,
                    tray::QuickPage::SoundOutput => door_window::search::page::SOUND,
                    _ => door_window::search::page::DISPLAY,
                };
                self.open_settings(page);
                return;
            }
            tray::Action::VpnConnect(index) => {
                self.vpn_toggle(index);
                return;
            }
            tray::Action::VpnSelect(index) => {
                self.vpn_select(index);
                return;
            }
            tray::Action::Brightness => {
                return;
            }
            tray::Action::Notice(hit) => {
                self.notice_action(hit, false);
                return;
            }
            tray::Action::Mute => {
                let output = self.sound_output();
                self.set_sound_output(door_service_protocol::audio::Output {
                    muted: !output.muted,
                    ..output
                });
                return;
            }
            // The slider acts while it is dragged (`audio_tick`); the one
            // output device and Spatial sound Off are already selected.
            tray::Action::Volume | tray::Action::OutputDevice | tray::Action::SpatialOff => {
                return;
            }
            tray::Action::MoreVolume => {
                self.tray.open = None;
                self.open_settings(door_window::search::page::SOUND);
                return;
            }
            _ => (),
        }
        let before = self.tray.open;
        self.tray.activate(action);
        if self.tray.open.is_some() && before.is_none() {
            self.tray.backdrop.clear();
            if self.tray.open == Some(tray::Flyout::Calendar) {
                self.notices_seen();
            }
        }
        if self.tray.open.is_none()
            && matches!(
                action,
                tray::Action::Clock
                    | tray::Action::Devices
                    | tray::Action::Language
                    | tray::Action::Dismiss
            )
        {
            self.restore_shell_focus();
        }
        if matches!(
            action,
            tray::Action::Clock
                | tray::Action::Devices
                | tray::Action::Previous
                | tray::Action::Next
        ) && self.tray.open.is_some()
        {
            // Openings and month changes only (diagnostic budget).
            let _ = door_sys::write(
                format!(
                    "DOOR_P12 TRAY open={:?} month={:?}\n",
                    self.tray.open, self.tray.month
                )
                .as_bytes(),
            );
        }
        if action == tray::Action::Settings {
            self.open_app(App::Settings);
        }
        if action == tray::Action::Desktop {
            self.toggle_desktop();
        }
        if self.tray.open.is_some() {
            self.engine.clear_focus();
        }
        self.sync_shell();
    }
    /// Secondary (right) button: context menus for the shell. Returns true
    /// when the shell consumed the press/release.
    pub(super) fn desktop_secondary(&mut self, down: bool) -> bool {
        if self.session.is_none() || self.drag.is_some() || self.engine.captured_window().is_some()
        {
            return false;
        }
        let (x, y) = self.logical_pointer();
        let layout = self.taskbar_layout();
        if down {
            if let Some(menu) = &self.shell.menu {
                if menu.hit(x, y) == MenuHit::Outside {
                    self.close_menu();
                    self.engine.invalidate();
                } else {
                    self.shell.secondary = true;
                    return true;
                }
            }
            let shell_area =
                self.overlay_contains() || self.engine.hit_test(self.x, self.y).is_none();
            self.shell.secondary = shell_area;
            return shell_area;
        }
        if !core::mem::take(&mut self.shell.secondary) {
            return false;
        }
        if self.shell.menu.is_some() {
            return true;
        }
        if self.notify_icon_secondary(x, y) {
            return true;
        }
        self.shell.menu_point = (x, y);
        let apps = self.taskbar_buttons();
        let slots = self.taskbar_slot_count();
        if let Some(slot) = layout.hit_slot(x, y, slots) {
            match slot {
                Slot::App(i) => {
                    if let Some(extra) = self.taskbar_extra(i) {
                        let rect = layout.slot(slot, slots);
                        self.extra_menu(&extra, rect);
                    } else if let Some(button) = apps.get(i).copied() {
                        let rect = layout.slot(slot, slots);
                        let pinned = self.shell.taskbar_pins.contains(&button.app);
                        self.open_menu(
                            menus::jump_list(
                                button.app,
                                pinned,
                                button.windows,
                                self.shell.prefs.end_task(),
                            ),
                            Anchor::Bar(
                                rect,
                                layout.controls().edge(),
                                door_window::menu::BarAlign::Center,
                            ),
                            Some(button.app),
                        );
                    }
                }
                Slot::Start => {
                    let rect = layout.slot(slot, slots);
                    self.close_start();
                    self.open_menu(
                        menus::quick_link(),
                        Anchor::Bar(
                            rect,
                            layout.controls().edge(),
                            door_window::menu::BarAlign::Start,
                        ),
                        None,
                    );
                }
                _ => self.open_menu(menus::taskbar(), Anchor::Point(x, y), None),
            }
            return true;
        }
        if layout.bar().contains(x, y) {
            if layout.hit_tray(x, y).is_none() {
                self.open_menu(menus::taskbar(), Anchor::Point(x, y), None);
            }
            return true;
        }
        if self.launcher {
            let g = self.shell.start.geometry(layout);
            if let Some(target) = self.shell.start.hit(&g, &self.fonts, x, y) {
                self.start_context(target, (x, y));
                return true;
            }
        }
        if self.overlay_contains() {
            return true;
        }
        if self.engine.hit_test(self.x, self.y).is_none() {
            self.close_start();
            self.tray.open = None;
            match self
                .desktop_icon_hover()
                .and_then(|i| Some((i, DesktopItem::at(i, &self.shell.desktop_order)?)))
            {
                Some((i, item)) => {
                    self.desktop_selection.select(i);
                    self.invalidate_icons();
                    let menu = self.desktop_item_menu(item);
                    self.open_menu(menu, Anchor::Point(x, y), item.app());
                }
                None => {
                    let more = !self
                        .controls
                        .has(door_service_protocol::desktop::Controls::CONTEXT_HIDE_MORE_OPTIONS);
                    let menu = if self.controls.classic_context_menu() {
                        menus::desktop_win10()
                    } else {
                        menus::desktop(false, more)
                    };
                    self.open_menu(menu, Anchor::Point(x, y), None);
                }
            }
            return true;
        }
        false
    }
    /// Pointer motion over shell surfaces: submenus open on hover.
    pub(super) fn shell_motion(&mut self) {
        let (x, y) = self.logical_pointer();
        let Some(menu) = &self.shell.menu else {
            return;
        };
        if let MenuHit::Item(depth, index) = menu.hit(x, y) {
            let opens = menu
                .at_depth(depth)
                .and_then(|m| m.item(index))
                .is_some_and(|i| matches!(i.command, Command::Submenu(_)) && i.enabled);
            let has_child_elsewhere = menu
                .at_depth(depth)
                .is_some_and(|m| m.child.is_some() && m.focus != Some(index));
            if opens {
                self.open_submenu(depth, index);
            } else if has_child_elsewhere {
                if let Some(m) = self.shell.menu.as_mut().and_then(|m| m.at_depth_mut(depth)) {
                    m.child = None;
                    m.focus = None;
                }
                self.engine.invalidate();
            }
        }
    }
    // Snap layouts.
    fn apply_snap(&mut self, layout_index: usize, zone: usize) {
        let Some(bar) = self.shell.snap.take() else {
            return;
        };
        self.engine.invalidate();
        let Ok(info) = self.engine.window_info(bar.window) else {
            return;
        };
        if info.owner != bar.owner {
            return;
        }
        let epoch = info.epoch;
        let area = self.engine.work_area();
        let result = match snap::builtin(layout_index, zone) {
            Some(s) => self
                .engine
                .snap_window(bar.owner, bar.window, s, bar.restore),
            // Any other zone is snapped too: it keeps restore-on-drag/unsnap.
            None => match snap::zone_rect(area, layout_index, zone) {
                Some(rect) => {
                    self.engine
                        .snap_window_rect(bar.owner, bar.window, rect, bar.restore)
                }
                None => return,
            },
        };
        if result.is_ok()
            && self
                .engine
                .window_info(bar.window)
                .is_ok_and(|w| w.epoch != epoch)
        {
            self.attachments.retain(|a| a.window != bar.window);
        }
        let _ = self.engine.activate(bar.window);
        self.manual_focus(Some(bar.window));
        let _ = door_sys::write(
            format!("DOOR_P12 SNAP layout={layout_index} zone={zone}\n").as_bytes(),
        );
        if result.is_ok() {
            self.assist_start(layout_index, zone, bar.window);
        }
    }
    /// Snap Assist after a built-in half/quarter snap (edge drag, Win+arrow).
    pub(super) fn assist_after_snap(&mut self, snap: g::Snap, id: g::WindowId) {
        let (layout, zone) = match snap {
            g::Snap::Left => (0, 0),
            g::Snap::Right => (0, 1),
            g::Snap::TopLeft => (3, 0),
            g::Snap::TopRight => (3, 1),
            g::Snap::BottomLeft => (3, 2),
            g::Snap::BottomRight => (3, 3),
            _ => return,
        };
        self.assist_start(layout, zone, id);
    }
    /// Snap Assist: the layout's other zones offer the other open windows on
    /// this desktop, most recently used first.
    fn assist_start(&mut self, layout_index: usize, zone: usize, snapped: g::WindowId) {
        // "Show what I can snap next to it when I snap a window" off.
        if self.multitask_off(door_service_protocol::preferences::multitask::NO_SNAP_ASSIST) {
            return;
        }
        let mut order: Vec<g::WindowId> = self.engine.windows().map(|w| w.id).collect();
        order.reverse();
        let mut others: Vec<Item> = items(&self.engine, &self.app_classes)
            .into_iter()
            .filter(|i| i.id != snapped)
            .collect();
        others.sort_by_key(|i| order.iter().position(|id| *id == i.id));
        let candidates = others
            .iter()
            .filter_map(|i| {
                let info = self.engine.window_info(i.id).ok()?;
                Some(door_window::snap_assist::Candidate::new(
                    i.id,
                    i.owner,
                    info.title,
                    Some(i.app),
                ))
            })
            .collect();
        let now = self.shell_now();
        let area = self.taskbar_layout().work_area();
        self.shell.assist = door_window::snap_assist::Assist::new(
            layout_index,
            zone,
            area,
            candidates,
            now,
            self.controls.reduce_motion(),
        );
        if let Some(assist) = self.shell.assist.as_mut() {
            assist.refresh(&self.engine);
            self.shell.assist_workspace = Some(self.engine.active_workspace());
        }
        self.engine.invalidate();
    }
    /// Snap Assist pick: snap the window into the offered zone (restoring
    /// it first when minimized), then offer the next empty zone.
    fn assist_pick(&mut self, index: usize) {
        let Some(assist) = self.shell.assist.as_ref() else {
            return;
        };
        let (Some((zone, _)), Some(candidate)) = (assist.zone(), assist.candidates.get(index))
        else {
            return;
        };
        let (id, owner, layout_index) = (candidate.id, candidate.owner, assist.layout);
        if self
            .engine
            .window_info(id)
            .is_ok_and(|w| w.state == g::WindowState::Minimized)
        {
            let _ = self.engine.restore_window(owner, id);
        }
        let epoch = self.engine.window_info(id).map(|w| w.epoch).ok();
        let area = self.engine.work_area();
        let result = match snap::builtin(layout_index, zone) {
            Some(s) => self.engine.snap_window(owner, id, s, None),
            None => match snap::zone_rect(area, layout_index, zone) {
                Some(rect) => self.engine.snap_window_rect(owner, id, rect, None),
                None => Err(g::Error::InvalidGeometry),
            },
        };
        if self
            .engine
            .window_info(id)
            .is_ok_and(|w| Some(w.epoch) != epoch)
        {
            self.attachments.retain(|a| a.window != id);
        }
        if result.is_ok() && self.engine.activate(id).is_ok() {
            self.manual_focus(Some(id));
        }
        let _ = (layout_index, zone, result.is_ok());
        let more = self
            .shell
            .assist
            .as_mut()
            .is_some_and(|a| a.picked(index) && a.refresh(&self.engine));
        if !more {
            self.shell.assist = None;
        }
        self.engine.invalidate();
    }
    /// `DOOR_P12 DESKTOP_GESTURE cancel reason=`: why a desktop icon press
    /// or first click did not complete (at most 24 lines per session).
    pub(super) fn gesture_note(&mut self, reason: &str, dt: u64) {
        if self.shell.gesture_notes == 0 {
            return;
        }
        self.shell.gesture_notes -= 1;
        let _ = door_sys::write(
            format!("DOOR_P12 DESKTOP_GESTURE cancel reason={reason} dt={dt}\n").as_bytes(),
        );
    }
    pub(super) fn assist_dismiss(&mut self) {
        if self.shell.assist.take().is_some() {
            self.shell.assist_workspace = None;
            self.engine.invalidate();
        }
    }
    /// Keep Snap Assist thumbnails current and drop it when nothing is left.
    fn assist_tick(&mut self) {
        if let Some(assist) = self.shell.assist.as_mut()
            && !assist.refresh(&self.engine)
        {
            self.assist_dismiss();
        }
    }
    /// Title-bar drag motion: reveal the snap bar near the top center.
    /// System > Multitasking: "Snap windows" is off.
    pub(super) fn snap_off(&self) -> bool {
        self.shell
            .prefs
            .get(door_service_protocol::preferences::MULTITASK)
            & door_service_protocol::preferences::multitask::NO_SNAP
            != 0
    }
    fn multitask_off(&self, bit: u32) -> bool {
        let word = self
            .shell
            .prefs
            .get(door_service_protocol::preferences::MULTITASK);
        word & (bit | door_service_protocol::preferences::multitask::NO_SNAP) != 0
    }
    pub(super) fn snap_drag_motion(&mut self, id: g::WindowId, owner: g::Owner, restore: g::Rect) {
        // "Show snap layouts when I drag a window to the top of my screen".
        if self.multitask_off(door_service_protocol::preferences::multitask::NO_DRAG_TOP) {
            self.shell.snap = None;
            return;
        }
        let (x, y) = self.logical_pointer();
        let area = self.taskbar_layout().work_area();
        let previous = self.shell.snap;
        if snap::reveals(area, x, y) || previous.is_some_and(|s| s.drag && s.rect.contains(x, y)) {
            let rect = snap::drag_flyout(area);
            let hover = snap::hit(rect, x, y);
            let motion = match previous {
                Some(p) => p.motion,
                None => self.snap_motion(),
            };
            self.shell.snap = Some(SnapBar {
                window: id,
                owner,
                rect,
                hover,
                drag: true,
                restore: Some(restore),
                from_hover: false,
                app: self
                    .app_classes
                    .iter()
                    .find(|(o, _)| *o == owner)
                    .map(|v| v.1),
                button: rect,
                linger: snap::Linger::default(),
                motion,
            });
            if previous.is_none() {
                self.engine.invalidate();
            } else if previous.map(|p| p.hover) != Some(hover) {
                self.bar_dirty = true;
            }
        } else if previous.is_some_and(|s| s.drag) {
            self.shell.snap = None;
            self.engine.invalidate();
        }
    }
    /// Title-bar drag release: snap to the hovered zone. True when handled.
    pub(super) fn snap_drag_drop(&mut self) -> bool {
        let Some(bar) = self.shell.snap.filter(|s| s.drag) else {
            return false;
        };
        match bar.hover {
            Some((l, z)) => {
                self.apply_snap(l, z);
                true
            }
            None => {
                self.shell.snap = None;
                self.engine.invalidate();
                false
            }
        }
    }
    /// Snap layouts under a hovered maximize button (after 500 ms), or for
    /// the focused window with Win+Z.
    fn maximize_hover_tick(&mut self, now: u64) {
        let shell_now = self.shell_now();
        if self.drag.is_some()
            || self.shell.menu.is_some()
            || self.launcher
            || snap::start_suspends_hover(&self.shell.start_motion, shell_now)
            || self.overview.is_some()
            || self.shell.locked
            || self.shell.snap.is_some_and(|s| s.drag)
            || self.multitask_off(door_service_protocol::preferences::multitask::NO_HOVER_LAYOUTS)
        {
            self.shell.max_hover.reset();
            return;
        }
        let (x, y) = self.logical_pointer();
        let over = match self.engine.hit_test(self.x, self.y) {
            Some((id, g::HitRegion::Maximize)) => Some(id),
            _ => None,
        };
        if self.shell.max_hover.over(over, now)
            && self.shell.snap.is_none()
            && let Some(id) = over
        {
            self.open_snap_flyout(id, true);
        }
        // A hover flyout stays while the pointer is on the button, near the
        // flyout, in the corridor between them or within the grace period.
        if let Some(bar) = self.shell.snap.as_mut()
            && !bar.drag
            && bar.from_hover
            && !bar.linger.keep(bar.button, bar.rect, x, y, now)
        {
            self.shell.snap = None;
            self.engine.invalidate();
        }
    }
    /// A new snap flyout's entrance (instant with reduced motion).
    fn snap_motion(&mut self) -> motion::Transition {
        let now = self.shell_now();
        let mut motion = motion::Transition::default();
        motion.set(true, now, self.controls.reduce_motion());
        motion
    }
    /// Snap layouts flyout for `id` at the top centre of the work area
    /// (Windows 11; Maximize hover, Win+Z and drag-to-top alike).
    fn open_snap_flyout(&mut self, id: g::WindowId, from_hover: bool) {
        let Ok(info) = self.engine.window_info(id) else {
            return;
        };
        if !matches!(
            info.state,
            g::WindowState::Normal | g::WindowState::Maximized
        ) {
            return;
        }
        let Some(button) =
            snap::maximize_button(info.outer, self.engine.scale(), info.caption_height)
        else {
            return;
        };
        let owner = info.owner;
        let area = self.taskbar_layout().work_area();
        let motion = self.snap_motion();
        self.shell.snap = Some(SnapBar {
            window: id,
            owner,
            rect: snap::drag_flyout(area),
            hover: None,
            drag: false,
            restore: None,
            from_hover,
            app: self
                .app_classes
                .iter()
                .find(|(o, _)| *o == owner)
                .map(|v| v.1),
            button,
            linger: snap::Linger::default(),
            motion,
        });
        self.shell.snap_digit = None;
        self.engine.invalidate();
    }
    /// Win+Z: snap layouts for the focused window.
    pub(super) fn snap_layouts_for_focus(&mut self) {
        let Some(id) = self.engine.focused_window() else {
            return;
        };
        self.open_snap_flyout(id, false);
    }
    fn snap_key(&mut self, scan: u32) -> bool {
        let digit = match scan {
            2..=10 => Some(scan as usize - 2),
            _ => None,
        };
        match (scan, digit, self.shell.snap_digit) {
            (1, ..) => {
                self.shell.snap = None;
                self.shortcut_keys |= 4;
            }
            (_, Some(l), None) if l < snap::LAYOUTS.len() => {
                self.shell.snap_digit = Some(l);
                if let Some(bar) = self.shell.snap.as_mut() {
                    bar.hover = Some((l, 0));
                }
            }
            (_, Some(z), Some(l)) if z < snap::LAYOUTS[l].len() => {
                self.shell.snap_digit = None;
                self.apply_snap(l, z);
            }
            _ => (),
        }
        self.engine.invalidate();
        true
    }
}
/// Per-user profile writes (shell layout, desktop controls) and Start file
/// search through the File Explorer provider.
impl Desktop {
    fn current_shell_layout(&self) -> Option<door_service_protocol::desktop::ShellLayout> {
        let taskbar: Vec<usize> = self.shell.taskbar_pins.iter().map(|a| a.index()).collect();
        let start: Vec<usize> = self.shell.start.pins.iter().map(|a| a.index()).collect();
        door_service_protocol::desktop::ShellLayout::new(
            &taskbar,
            &start,
            u32::from(self.shell.start.view.index()),
            self.shell.sorted,
        )
    }
    /// Queue a profile write when pins, the Start All view or the desktop
    /// sort changed. Init stores it for the session user.
    pub(super) fn persist_shell_layout(&mut self) {
        if self.session.is_none() {
            return;
        }
        if let Some(layout) = self.current_shell_layout()
            && layout != self.shell.layout_saved
        {
            self.shell.layout_saved = layout;
            self.shell.layout_pending = Some(layout);
        }
    }
    /// Init (session 0) delivers the user's persisted layout at sign-in.
    pub(super) fn apply_shell_layout(
        &mut self,
        b: &[u8],
        t: &[door_abi::Transfer],
    ) -> Result<Response, Code> {
        if b.len() != 8 || !t.is_empty() {
            return Err(Code::InvalidRequest);
        }
        let word = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let layout = door_service_protocol::desktop::ShellLayout::from_parts(word(0), word(4))
            .ok_or(Code::InvalidRequest)?;
        let apps = |(pins, count): ([usize; door_window::apps::COUNT], usize)| -> Vec<App> {
            pins[..count].iter().map(|i| App::ALL[*i]).collect()
        };
        self.shell.taskbar_pins = apps(layout.taskbar_pins());
        self.shell.start.pins = apps(layout.start_pins());
        self.shell.start.view = start::View::from_index(layout.view() as u8);
        self.shell.start.scroll = 0;
        self.shell.sorted = layout.sorted();
        self.shell.desktop_order = if layout.sorted() {
            door_window::apps::alphabetical()
        } else {
            App::ALL
        };
        self.shell.layout_saved = layout;
        self.shell.layout_pending = None;
        self.desktop_selection.clear();
        self.icons_dirty = true;
        self.bar_dirty = true;
        self.engine.invalidate();
        let _ = door_sys::write(
            format!(
                "DOOR_P12 SHELL_LAYOUT applied flags={} order={}\n",
                layout.flags(),
                layout.order()
            )
            .as_bytes(),
        );
        Ok(Response::empty())
    }
    /// One Init request at a time: desktop controls first (they repaint the
    /// desktop), then the shell layout. Failures are reported, not retried.
    fn persist_tick(&mut self, now: u64) {
        if let Some(init) = self.shell.persist.as_mut() {
            match init.poll_desktop_command(now) {
                Ok(None) => return,
                Ok(Some(())) => (),
                Err(_) => {
                    let _ = door_sys::write(b"DOOR_P12 PROFILE write failed\n");
                }
            }
            self.shell.persist = None;
        }
        let controls = self.shell.controls_pending.take();
        let layout = if controls.is_none() {
            self.shell.layout_pending.take()
        } else {
            None
        };
        let setting = if controls.is_none() && layout.is_none() {
            (!self.shell.setting_pending.is_empty()).then(|| self.shell.setting_pending.remove(0))
        } else {
            None
        };
        if controls.is_none() && layout.is_none() && setting.is_none() {
            return;
        }
        let started = door_service_client::Init::connect(self.registry).and_then(|mut init| {
            match (controls, layout, setting) {
                (Some(controls), _, _) => init.begin_controls_persist(controls, now)?,
                (None, Some(layout), _) => init.begin_shell_layout(layout, now)?,
                (None, None, Some((id, value))) => init.begin_persist_setting(id, value, now)?,
                (None, None, None) => (),
            }
            Ok(init)
        });
        match started {
            Ok(init) => self.shell.persist = Some(init),
            Err(_) => {
                let _ = door_sys::write(b"DOOR_P12 PROFILE write failed\n");
            }
        }
    }
    /// A workspace client said HELLO with CAP_SEARCH_PROVIDER.
    pub(super) fn search_provider_hello(&mut self, owner: g::Owner) {
        let link = &mut self.shell.files;
        link.provider = Some(owner);
        link.events.clear();
        if !link.query.is_empty() && link.active {
            link.due = Some(0);
        }
    }
    /// The live provider: the session's search host (Init-launched,
    /// windowless), connected and not revoked.
    fn search_provider(&self) -> Option<g::Owner> {
        let owner = self.shell.files.provider?;
        (self.shell.files.host == Some(owner)
            && self.clients.iter().any(|c| c.owner == owner && !c.revoked)
            && !self.terminations.contains(owner))
        .then_some(owner)
    }
    /// Start > Recent: ask the provider for the list each time Start opens
    /// (unless Recent or recently opened files are turned off). Without a
    /// provider the section keeps its honest empty state.
    fn recent_tick(&mut self) {
        if !self.launcher {
            self.shell.files.recent_asked = false;
            return;
        }
        // The Search panel's Recent list follows only the privacy toggle
        // (recently opened items), not Start's Recent section toggle.
        let wanted = (self.shell.start.search_panel()
            || !self
                .controls
                .has(door_service_protocol::desktop::Controls::START_HIDE_RECENT))
            && !self
                .controls
                .has(door_service_protocol::desktop::Controls::START_HIDE_RECENT_FILES);
        if !wanted || self.search_provider().is_none() {
            if !self.shell.start.recent.is_empty() {
                self.shell.start.recent.clear();
                self.bar_dirty = true;
            }
            return;
        }
        if !self.shell.files.recent_asked {
            self.shell.files.recent_asked = true;
            self.request_recent();
        }
    }
    fn request_recent(&mut self) {
        let link = &mut self.shell.files;
        link.serial = link.serial.wrapping_add(1).max(1);
        link.recent_serial = link.serial;
        // A query typed before this keeps its own (older) serial: re-ask.
        if link.active {
            link.active = false;
        }
        self.post_search_event(w::SEARCH_RECENT, 0, 0);
    }
    /// Start > Recent item `index`: opens like a search result (Notes for
    /// text, Preview for images, File Explorer otherwise or for `location`).
    fn recent_open(&mut self, index: usize, location: bool) {
        self.close_start();
        if self.search_provider().is_none() || index >= self.shell.start.recent.len() {
            return;
        }
        self.shell_focus = None;
        let serial = self.shell.files.recent_serial;
        let link = &mut self.shell.files;
        if link.events.len() >= 8 {
            link.events.remove(0);
        }
        link.events
            .push((w::SEARCH_OPEN, serial, index as u32, u32::from(location)));
        let _ = door_sys::write(
            format!(
                "DOOR_P12 START_RECENT_OPEN index={index} location={}\n",
                u32::from(location)
            )
            .as_bytes(),
        );
    }
    /// "Remove from list": the provider removes and saves; Start shows the
    /// list without it at once and asks for the fresh list.
    fn recent_remove(&mut self, index: usize) {
        if self.search_provider().is_none() || index >= self.shell.start.recent.len() {
            return;
        }
        let serial = self.shell.files.recent_serial;
        let link = &mut self.shell.files;
        if link.events.len() >= 8 {
            link.events.remove(0);
        }
        link.events
            .push((w::SEARCH_RECENT_REMOVE, serial, index as u32, 0));
        self.shell.start.recent.remove(index);
        if self.launcher {
            self.request_recent();
        }
        self.bar_dirty = true;
        let _ = door_sys::write(format!("DOOR_P12 START_RECENT_REMOVE index={index}\n").as_bytes());
    }
    /// Debounce Start queries into provider SEARCH_QUERY events.
    fn file_search_tick(&mut self, now: u64) {
        self.recent_tick();
        let active = self.launcher && self.shell.start.searching();
        if !active {
            // Keep the last query: SEARCH_EXPLORE/OPEN may still ask for it.
            self.shell.files.due = None;
            self.shell.files.events.retain(|e| e.0 != w::SEARCH_QUERY);
            self.shell.files.active = false;
            return;
        }
        let mut query = self.shell.start.query.trim();
        if query.len() > w::search::MAX_QUERY {
            let mut end = w::search::MAX_QUERY;
            while !query.is_char_boundary(end) {
                end -= 1;
            }
            query = &query[..end];
        }
        let link = &mut self.shell.files;
        if !link.active || link.query != query {
            link.active = true;
            link.serial = link.serial.wrapping_add(1).max(1);
            link.query = query.into();
            link.due = Some(now.saturating_add(SEARCH_DEBOUNCE));
            link.events.retain(|e| e.0 != w::SEARCH_QUERY);
            self.shell.start.files.clear();
            self.shell.start.file_search = start::FileSearch::Searching;
            self.bar_dirty = true;
        }
        if self.search_provider().is_none() {
            if self.shell.start.file_search != start::FileSearch::Unavailable {
                self.shell.start.file_search = start::FileSearch::Unavailable;
                self.shell.start.files.clear();
                self.bar_dirty = true;
            }
            return;
        }
        let link = &mut self.shell.files;
        if self.shell.start.file_search == start::FileSearch::Unavailable {
            // A provider appeared for the current query.
            self.shell.start.file_search = start::FileSearch::Searching;
            link.due.get_or_insert(now);
            self.bar_dirty = true;
        }
        if link.due.is_some_and(|due| now >= due) {
            link.due = None;
            link.events.retain(|e| e.0 != w::SEARCH_QUERY);
            link.events.push((w::SEARCH_QUERY, link.serial, 0, 0));
        }
    }
    fn post_search_event(&mut self, kind: u32, b: u32, detail: u32) {
        let link = &mut self.shell.files;
        if link.events.len() >= 8 {
            link.events.remove(0);
        }
        link.events.push((kind, link.serial, b, detail));
    }
    /// Start: open file result `index` through the provider: a folder in
    /// File Explorer, a file in its app (Notes, Preview) or, with
    /// `location`, File Explorer at its folder with the file selected.
    fn search_open(&mut self, index: usize, location: bool) {
        self.close_start();
        if self.search_provider().is_none() {
            self.open_app(App::Files);
            return;
        }
        self.shell_focus = None;
        self.post_search_event(w::SEARCH_OPEN, index as u32, u32::from(location));
        let _ = door_sys::write(
            format!(
                "DOOR_P12 START_FILE_OPEN serial={} index={index} location={}\n",
                self.shell.files.serial,
                u32::from(location)
            )
            .as_bytes(),
        );
    }
    /// Start: "Search in File Explorer" for the current query.
    fn search_explore(&mut self) {
        self.close_start();
        if self.search_provider().is_none() {
            self.open_app(App::Files);
            return;
        }
        self.shell_focus = None;
        self.post_search_event(w::SEARCH_EXPLORE, 0, 0);
        let _ = door_sys::write(
            format!(
                "DOOR_P12 START_FILE_EXPLORE serial={}\n",
                self.shell.files.serial
            )
            .as_bytes(),
        );
    }
    /// Shell events for `owner`: DOCUMENT_OPEN for the app a document is
    /// waiting for, and the provider's search events.
    pub(super) fn take_search_event(&mut self, owner: g::Owner) -> Option<w::Event> {
        let now = door_sys::time().unwrap_or(0);
        if self
            .shell
            .files
            .document
            .as_ref()
            .is_some_and(|d| now >= d.deadline)
        {
            self.shell.files.document = None;
        }
        let app = self
            .app_classes
            .iter()
            .find(|(o, _)| *o == owner)
            .map(|v| v.1);
        let capable = self
            .clients
            .iter()
            .any(|c| c.owner == owner && c.controls_events);
        let elevated = self.admin_terminal == Some(owner);
        if let Some(document) = self.shell.files.document.as_mut()
            && !document.posted
            && capable
            && app == Some(document.app)
            && document.elevated == elevated
        {
            document.posted = true;
            return Some(w::Event {
                kind: w::DOCUMENT_OPEN,
                a: document.payload.len() as u32,
                b: u32::from(document.launched),
                ..w::Event::default()
            });
        }
        let link = &mut self.shell.files;
        if link.provider != Some(owner) || link.events.is_empty() {
            return None;
        }
        let (kind, a, b, detail) = link.events.remove(0);
        Some(w::Event {
            kind,
            a,
            b,
            detail,
            ..w::Event::default()
        })
    }
    /// OPEN_DOCUMENT (search host and desktop-controls clients, i.e. File
    /// Explorer's Open / Open with): launch or activate the app and post it
    /// DOCUMENT_OPEN. File Explorer opens a new window unless it is
    /// launched for this document.
    pub(super) fn open_document(&mut self, owner: g::Owner, b: &[u8]) -> Result<Response, Code> {
        let controls = self
            .clients
            .iter()
            .any(|c| c.owner == owner && c.controls_events);
        door_window::document_open::authorize(self.search_provider() == Some(owner), controls)?;
        if b.len() < 5 || b.len() > 4 + w::MAX_DOCUMENT {
            return Err(Code::InvalidRequest);
        }
        let app = match u32::from_le_bytes(b[..4].try_into().unwrap()) {
            1 => App::Files,
            2 => App::Notes,
            9 => App::Terminal,
            10 => App::Preview,
            _ => return Err(Code::InvalidRequest),
        };
        let payload = &b[4..];
        // Paths are UTF-8 without NUL; Explorer requests use NUL separators.
        if core::str::from_utf8(payload).is_err() || app != App::Files && payload.contains(&0) {
            return Err(Code::InvalidRequest);
        }
        self.deliver_document(app, payload)
    }
    /// Hand `payload` to `app`: launch it, or post to the running one for
    /// DOCUMENT_GET.
    /// A document for the elevated Terminal ("admin:<shell>" or
    /// "admin:<batch file>"), after User Account Control approved it: the
    /// elevated Terminal opens it, started first when it is not running.
    pub(super) fn deliver_elevated(&mut self, payload: &[u8]) -> Result<Response, Code> {
        if self.shutdown.is_some() {
            return Err(Code::Closing);
        }
        let running = self.admin_terminal.is_some();
        self.shell.files.document = Some(Document {
            app: App::Terminal,
            elevated: true,
            payload: payload.to_vec(),
            posted: false,
            launched: !running,
            deadline: door_sys::time().unwrap_or(0).saturating_add(4000),
        });
        self.foreground.request(
            door_service_protocol::application::ELEVATED_TERMINAL_COMMAND,
            door_sys::time().unwrap_or(0),
        );
        if running {
            let admin = self.admin_terminal;
            if let Some(item) = all_items(&self.engine, &self.app_classes)
                .iter()
                .find(|i| Some(i.owner) == admin)
                && self.engine.has_snapshot(item.id)
                && self.activate_desktop_window(item.id)
            {
                self.foreground.complete();
            }
        } else {
            self.pending_launches |= 1 << ADMIN_TERMINAL_BIT;
        }
        let _ = door_sys::write(
            format!(
                "DOOR_P16 ELEVATED TERMINAL document launched={}\n",
                u32::from(!running)
            )
            .as_bytes(),
        );
        Ok(Response::empty())
    }
    pub(super) fn deliver_document(&mut self, app: App, payload: &[u8]) -> Result<Response, Code> {
        if self.shutdown.is_some() {
            return Err(Code::Closing);
        }
        let admin = self.admin_terminal;
        let running = all_items(&self.engine, &self.app_classes)
            .iter()
            .any(|i| i.app == app && Some(i.owner) != admin);
        self.shell.files.document = Some(Document {
            app,
            elevated: false,
            payload: payload.to_vec(),
            posted: false,
            launched: !running,
            deadline: door_sys::time().unwrap_or(0).saturating_add(2000),
        });
        if app == App::Files && running {
            // The new Explorer window takes the foreground when it opens.
            self.foreground.manual(Some(app.command()));
        } else {
            self.launch_app(app).map_err(|e| match e {
                door_service_client::Error::Service(code) => code,
                _ => Code::Io,
            })?;
        }
        let _ = door_sys::write(
            format!(
                "DOOR_P12 DOCUMENT_OPEN app={} launched={}\n",
                app.command(),
                u32::from(!running)
            )
            .as_bytes(),
        );
        Ok(Response::empty())
    }
    /// DOCUMENT_GET: the waiting document for the calling app (consumed).
    pub(super) fn document_get(&mut self, owner: g::Owner, b: &[u8]) -> Result<Response, Code> {
        if !b.is_empty() {
            return Err(Code::InvalidRequest);
        }
        let app = self
            .app_classes
            .iter()
            .find(|(o, _)| *o == owner)
            .map(|v| v.1);
        match self.shell.files.document.take() {
            Some(document) if door_sys::time().unwrap_or(0) >= document.deadline => {
                let _ = document;
                Err(Code::NotFound)
            }
            Some(document)
                if Some(document.app) == app
                    && document.posted
                    && document.elevated == (self.admin_terminal == Some(owner)) =>
            {
                Ok(Response::bytes(&document.payload))
            }
            other => {
                self.shell.files.document = other;
                Err(Code::NotFound)
            }
        }
    }
    /// SEARCH_QUERY_GET: the text of the current query serial.
    pub(super) fn search_query_get(&mut self, owner: g::Owner, b: &[u8]) -> Result<Response, Code> {
        if b.len() != 4 {
            return Err(Code::InvalidRequest);
        }
        let link = &self.shell.files;
        if link.provider != Some(owner) {
            return Err(Code::Denied);
        }
        let serial = u32::from_le_bytes(b.try_into().unwrap());
        if serial == 0 || serial != link.serial || link.query.is_empty() {
            return Err(Code::NotFound);
        }
        Ok(Response::bytes(link.query.as_bytes()))
    }
    /// SEARCH_RESULTS: one provider result; stale serials are dropped.
    pub(super) fn search_results(&mut self, owner: g::Owner, b: &[u8]) -> Result<Response, Code> {
        if self.shell.files.provider != Some(owner) {
            return Err(Code::Denied);
        }
        let message = w::search::decode(b)?;
        let recent = message.serial == self.shell.files.recent_serial && self.launcher;
        if !recent && (message.serial != self.shell.files.serial || !self.shell.files.active) {
            return Ok(Response::empty());
        }
        fn clean(text: &str) -> alloc::string::String {
            text.chars()
                .map(|c| if c.is_control() { '?' } else { c })
                .collect()
        }
        let start = &mut self.shell.start;
        let list = if recent {
            &mut start.recent
        } else {
            &mut start.files
        };
        match message.entry {
            None => list.clear(),
            Some((index, entry)) => {
                if index == 0 {
                    list.clear();
                }
                if usize::from(index) == list.len() {
                    list.push(start::FileResult {
                        folder: entry.kind == w::search::FOLDER,
                        name: clean(entry.name),
                        path: clean(entry.path),
                    });
                }
            }
        }
        list.truncate(usize::from(message.count).min(if recent {
            start::MAX_RECENT
        } else {
            usize::MAX
        }));
        if recent {
            // Logged only when the shown count changes (diagnostic budget).
            let complete = list.len() == usize::from(message.count).min(start::MAX_RECENT);
            if complete && self.shell.files.recent_logged != Some(list.len()) {
                self.shell.files.recent_logged = Some(list.len());
                let _ = door_sys::write(
                    format!("DOOR_P12 START_RECENT count={}\n", list.len()).as_bytes(),
                );
            }
            self.bar_dirty = true;
            return Ok(Response::empty());
        }
        start.file_search = match message.state {
            w::search::BUILDING => start::FileSearch::Searching,
            w::search::PARTIAL => start::FileSearch::Ready { partial: true },
            _ => start::FileSearch::Ready { partial: false },
        };
        let last = start.results().len().saturating_sub(1);
        start.selected = start.selected.min(last);
        self.bar_dirty = true;
        Ok(Response::empty())
    }
}
/// Context menu for a desktop icon: the app commands, tagged as a desktop
/// menu so closing Start never takes it down.
impl Desktop {
    /// The context menu of a desktop icon.
    fn desktop_item_menu(&self, item: DesktopItem) -> Menu {
        match item {
            DesktopItem::App(app) => desktop_item_menu(
                app,
                self.shell.start.pins.contains(&app),
                self.shell.taskbar_pins.contains(&app),
            ),
            DesktopItem::RecycleBin => {
                let mut menu = menus::recycle_bin(self.recycle_full);
                menu.tag = menus::DESKTOP;
                menu
            }
        }
    }
    /// Double-click or Enter on a desktop icon.
    fn open_desktop_item(&mut self, item: DesktopItem) {
        match item {
            DesktopItem::App(app) => self.open_app(app),
            DesktopItem::RecycleBin => self.open_recycle_bin(menus::BIN_OPEN),
        }
    }
    /// File Explorer at the Recycle Bin (`::RecycleBin`), emptying it
    /// (Explorer asks first) or showing its properties: an Explorer request
    /// exactly as Start search sends (OPEN_DOCUMENT).
    pub(super) fn open_recycle_bin(&mut self, action: u8) {
        let payload: &[u8] = match action {
            menus::BIN_EMPTY => b"BE",
            menus::BIN_PROPERTIES => b"BP",
            _ => b"L::RecycleBin",
        };
        if let Some(launched) = self.open_files_request(payload) {
            let _ = door_sys::write(
                format!(
                    "DOOR_P14 RECYCLE_BIN action={action} launched={}\n",
                    u32::from(launched)
                )
                .as_bytes(),
            );
        }
    }
    /// Start > Folders: Settings, or File Explorer at the folder.
    pub(super) fn open_start_folder(&mut self, index: usize) {
        let payload: &[u8] = match index {
            0 => {
                self.open_settings(0);
                return;
            }
            // Known folders resolve in Explorer as typed (`Open::Typed`).
            1 => b"THome",
            2 => b"Tshell:Documents",
            3 => b"Tshell:Downloads",
            4 => b"Tshell:Music",
            5 => b"Tshell:Pictures",
            6 => b"Tshell:Videos",
            7 => b"L::ThisPC",
            _ => b"T~",
        };
        self.open_files_request(payload);
        let _ = door_sys::write(format!("DOOR_P14 START_FOLDER index={index}\n").as_bytes());
    }
    /// Hand File Explorer a request (OPEN_DOCUMENT payload), starting it
    /// when needed: whether it was launched (None while shutting down).
    fn open_files_request(&mut self, payload: &[u8]) -> Option<bool> {
        if self.shutdown.is_some() {
            return None;
        }
        let running = all_items(&self.engine, &self.app_classes)
            .iter()
            .any(|i| i.app == App::Files);
        self.shell.files.document = Some(Document {
            app: App::Files,
            elevated: false,
            payload: payload.to_vec(),
            posted: false,
            launched: !running,
            deadline: door_sys::time().unwrap_or(0).saturating_add(2000),
        });
        if running {
            self.foreground.manual(Some(App::Files.command()));
        } else if self.launch_app(App::Files).is_err() {
            self.shell.files.document = None;
        }
        Some(!running)
    }
}
fn desktop_item_menu(app: App, start_pinned: bool, taskbar_pinned: bool) -> Menu {
    let mut menu = menus::start_item(app, None, start_pinned, taskbar_pinned);
    menu.tag = menus::DESKTOP;
    menu
}
/// Stable small integer for a Start target (hover identity).
fn start_key(target: start::Target) -> usize {
    use start::Target as T;
    match target {
        T::Search => 1,
        T::Pinned(i) => 10 + i,
        T::ShowAll => 2,
        T::View => 3,
        T::CategoryApp(c, s) => 100 + c * 8 + s,
        T::GridApp(i) => 300 + i,
        T::ListApp(i) => 400 + i,
        T::User => 4,
        T::Power => 5,
        T::Chip(f) => 500 + f.id() as usize,
        T::Result(i) => 600 + i,
        T::Action(i) => 700 + i,
        T::Recent(i) => 800 + i,
        T::TopApp(i) => 900 + i,
        T::QuickSearch(i) => 950 + i,
        T::History(i) => 1000 + i,
        T::Folder(i) => 1100 + i,
        T::Panel => 0,
    }
}

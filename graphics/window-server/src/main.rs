#![no_std]
#![no_main]
extern crate alloc;
mod access_glue;
mod accessibility_glue;
mod authority;
mod compositor;
mod desktop;
mod elevation_glue;
mod firewall_glue;
mod input;
mod multitask;
mod native_ui;
mod notify_glue;
mod present;
mod rescale;
mod shell_glue;
mod signin;
mod snip_glue;
mod text_input_glue;
mod vpn_glue;
use alloc::{format, vec::Vec};
use door_abi::{
    Handle, Status,
    platform::{Identity, StartupV2, inherited},
};
use door_graphics_core as g;
use door_graphics_text::FontAtlas;
use door_service_protocol::{Code, window as w};
use door_system_services::server::{Handler, Response, Server};
#[global_allocator]
static HEAP: door_rt::HeapAllocator = door_rt::HeapAllocator::new();
door_rt::entry_v2!(main);
struct Buffer {
    owner: g::Owner,
    id: g::BufferId,
    handle: Handle,
    bytes: usize,
}
struct Attachment {
    owner: g::Owner,
    window: g::WindowId,
    buffer: g::BufferId,
}
struct Client {
    /// Kernel-authenticated manifest of the client process (0 once evicted).
    manifest: u64,
    multiwindow: bool,
    new_windows: u8,
    channel: Handle,
    owner: g::Owner,
    revoked: bool,
    transient_events: bool,
    appearance_events: bool,
    appearance_pending: Option<u32>,
    controls_events: bool,
    controls_pending: Option<u32>,
    /// Negotiated CAP_WHEEL: INPUT wheel events may be delivered.
    wheel_events: bool,
    /// Negotiated CAP_DISPLAY_SCALE: fractional DPI and SCALE_CHANGED.
    scale_events: bool,
    scale_pending: bool,
    /// Granted CAP_WIN32 (Windows-personality manifests only): window
    /// relations and the Win32 close / not-responding semantics.
    win32: bool,
}
#[derive(Clone, Copy)]
struct Drag {
    id: g::WindowId,
    owner: g::Owner,
    rect: g::Rect,
    x: i32,
    y: i32,
    edges: u8,
    epoch: u64,
    restore_pending: bool,
    moved: bool,
}
struct Desktop {
    engine: g::Engine,
    captures: door_window::capture::Pool,
    display: Handle,
    registry: Handle,
    fonts: FontAtlas,
    wallpaper: Vec<u8>,
    assets: door_window::wallpaper::Pack<'static>,
    /// The lock screen picture while locked (Personalization > Lock
    /// screen), when it isn't the desktop background; empty otherwise.
    lock_pixels: Vec<u8>,
    background: u32,
    controls: door_service_protocol::desktop::Controls,
    /// The user's accent setting and its resolved colour 0xRRGGBB.
    accent: door_service_protocol::desktop::Accent,
    accent_rgb: u32,
    /// The accent setting and colour last logged (`apply_accent`): a theme
    /// that re-applies the same accent spends no diagnostic credit.
    accent_reported: Option<(u32, u32)>,
    settings_navigation: Option<u32>,
    /// Display scale preference bound by Init or set by Settings: 0 =
    /// automatic, else a percent (spec §57).
    scale_preference: u32,
    /// Host scale hint from DisplayInfo (0 = none).
    scale_hint: u32,
    overlay_underlay: Vec<u8>,
    presented_overlay: bool,
    presented_overview: Option<bool>,
    presentation_log: door_window::telemetry::PresentationLog,
    motion: door_window::telemetry::Motion,
    frame_clock: door_window::telemetry::FrameClock,
    frame_time_reported: bool,
    move_reuse_reported: bool,
    taskbar_material: door_window::material::Taskbar,
    tray: door_window::tray::State,
    bar_dirty: bool,
    buffers: Vec<Buffer>,
    attachments: Vec<Attachment>,
    clients: Vec<Client>,
    session: Option<(u64, u64, u32)>,
    login: Option<door_service_client::Init>,
    login_pressed: bool,
    login_waiting: bool,
    login_reported: bool,
    binding_presentation: Option<(u8, bool)>,
    x: i32,
    y: i32,
    buttons: u32,
    buttons_unknown: bool,
    drag: Option<Drag>,
    /// Win32 window relations (CAP_WIN32) and the held press a client may
    /// continue as a move/resize.
    win32: door_window::win32::Relations,
    win32_press: Option<door_window::win32::Press>,
    caption_press: Option<(g::WindowId, g::HitRegion)>,
    cursor_dirty: bool,
    last_cursor: Option<(i32, i32)>,
    closing: Vec<(g::Owner, g::WindowId, u64)>,
    shortcut_keys: u8,
    system_keys: door_window::policy::SystemKeys,
    overview: Option<door_window::overview::Overview>,
    overview_focus: Option<g::WindowId>,
    shell_focus: Option<door_window::policy::ReturnFocus>,
    app_workspaces: [Option<g::WorkspaceId>; door_service_protocol::application::DESKTOP_APPS],
    hidden_windows: Vec<g::WindowId>,
    /// Start is open (shell state and animation live in `shell`).
    launcher: bool,
    shell: desktop::Shell,
    /// Trusted credential gate for sign-in and unlock (P14-LOGON).
    signin: signin::SignIn,
    /// door.shell: the desktop shell process's role and publications.
    shell_link: door_window::shell_link::Link,
    /// (window id, observable signature) last reported to the shell.
    shell_seen: Vec<(u64, u64)>,
    shell_focus_id: u64,
    shell_tick_at: u64,
    /// Taskbar app slot under the pointer and since when (hover preview).
    preview_hover: Option<(usize, u64)>,
    /// Taskbar button under the pointer and since when (hover tooltip).
    tooltip_hover: Option<(door_window::taskbar::Slot, u64)>,
    /// Pointer position when a press dismissed the thumbnails/tooltip:
    /// they stay away until the pointer moves.
    preview_hold: Option<(i32, i32)>,
    preview_notes: u8,
    desktop_press: Option<desktop::Press>,
    desktop_selection: door_window::policy::DesktopSelection,
    icon_hover: Option<usize>,
    /// The session user's Recycle Bin holds items (File Explorer reports
    /// it): the desktop icon shows the full bin.
    recycle_full: bool,
    icons_dirty: bool,
    overlay_buttons: u32,
    dark: bool,
    app_classes: Vec<(g::Owner, desktop::App)>,
    /// The elevated Terminal's process (Init's ELEVATED_TERMINAL_SLOT): its
    /// windows group with Terminal, documents for it are marked elevated.
    admin_terminal: Option<g::Owner>,
    catalog_at: u64,
    catalog: Option<door_service_client::Init>,
    /// Launch requests: bit = app index, then `App::all().len() + Tool::index`.
    pending_launches: u32,
    key_modifiers: u32,
    launching: Option<(door_service_client::Init, u8)>,
    foreground: door_window::policy::Foreground,
    shutdown: Option<bool>,
    /// The shutdown gate ends in sign out (Init command 25), not power.
    signing_out: bool,
    shutdown_since: u64,
    power_retry: door_window::policy::RetryGate,
    terminations: door_window::policy::Terminations,
    shutdown_frozen: bool,
    unfreeze_pending: bool,
    catalog_fresh: bool,
    bar_reported: bool,
    /// The kernel negotiated a wheel mouse (display capability WHEEL).
    wheel_input: bool,
    /// Newest interactive-resize geometry, applied once per frame.
    resize_pending: Option<(g::Owner, g::WindowId, g::Rect)>,
    transition_clock: door_window::telemetry::TransitionClock,
    /// DISPLAY-01 presentation failure policy (and the test-image hook).
    presentation: door_window::scanout::Presentation,
    /// The last composed scene covered the whole output.
    last_present_full: bool,
}
fn main(startup: StartupV2) -> i64 {
    if let Err(e) = HEAP.initialize_with_bytes(door_window::policy::HEAP_BYTES as u64) {
        return -(200 + e as i64);
    }
    let result = run(startup);
    match result {
        Ok(()) => 0,
        Err(e) => {
            let _ = door_sys::write(b"DOOR_P11 SERVER FAILED\n");
            -(220 + e as i64)
        }
    }
}
fn run(startup: StartupV2) -> Result<(), Status> {
    let display = startup.base.inherited[inherited::AUTHORITY].handle;
    let info = door_sys::display_info(display)?;
    let limits = door_window::policy::limits();
    let mut captures = door_window::capture::Pool::new().ok_or(Status::ResourceExhausted)?;
    // Automatic display scale until Init binds the user's preference: the
    // host's scale hint when offered, else the integer heuristic (spec §57).
    let scale_hint = info.scale_hint();
    let scale = door_window::policy::resolve_scale(0, scale_hint, info.width, info.height);
    let mut engine = g::Engine::new(info.width, info.height, g::Theme::Light, limits)
        .map_err(|_| Status::Unsupported)?;
    engine
        .set_scale_percent(scale)
        .map_err(|_| Status::Unsupported)?;
    let logical = (
        scale.logical_len(info.width),
        scale.logical_len(info.height),
    );
    engine
        .set_work_area(door_window::taskbar::physical(
            door_window::taskbar::Layout::new(logical.0, logical.1).work_area(),
            scale,
        ))
        .map_err(|_| Status::Unsupported)?;
    // Distant simultaneous updates compose as separate small regions; the
    // overlay restores its own taskbar underlay (see present.rs).
    engine.set_damage_regions(g::MAX_DAMAGE_RECTS);
    // Unicode text (ADR 0018): the shared application asset pack, mapped
    // read-only for the process lifetime, supplies the shell's font set.
    let pack = startup.base.inherited[inherited::APP_ASSETS];
    if pack.kind == door_abi::HandleKind::Section as u64
        && pack.rights == door_abi::rights::QUERY | door_abi::rights::READ | door_abi::rights::MAP
        // SAFETY: 0x7E00_0000.. is reserved for this view; it is never unmapped.
        && let Ok(bytes) =
            unsafe { door_sys::section_map(pack.handle, 0x7E00_0000, door_abi::rights::READ) }
        && let Some(pack) = door_initfs::app_assets::Pack::parse(unsafe {
            core::slice::from_raw_parts(0x7E00_0000 as *const u8, bytes as usize)
        })
        && !door_window::display_scale::install_unicode(pack)
    {
        door_rt::diagnostic(format_args!("DOOR_P12 UNICODE_FONTS unavailable\n"));
    }
    let font_started = door_sys::time().ok();
    let fonts = door_window::display_scale::shell_fonts(scale).ok_or(Status::IoFailure)?;
    if let (Some(started), Ok(ended)) = (font_started, door_sys::time())
        && let Some(font_ticks) = ended.checked_sub(started)
    {
        door_rt::diagnostic(format_args!(
            "DOOR_P12 FONT_ATLAS packed={} factor={} ticks={font_ticks} bytes={} capacity={}\n",
            door_window::display_scale::factor_text(scale),
            u8::from(fonts.uses_packed_cache()),
            fonts.cache_bytes(),
            fonts.cache_capacity(),
        ));
    }
    let grant = startup.base.inherited[inherited::DESKTOP_ASSETS];
    let expected = door_abi::rights::QUERY | door_abi::rights::READ | door_abi::rights::MAP;
    if grant.kind != door_abi::HandleKind::Section as u64 || grant.rights != expected {
        return Err(Status::AccessDenied);
    }
    if !matches!(
        unsafe {
            door_sys::section_map(
                grant.handle,
                0x7600_0000,
                door_abi::rights::READ | door_abi::rights::WRITE,
            )
        },
        Err(Status::AccessDenied)
    ) {
        return Err(Status::AccessDenied);
    }
    let bytes =
        unsafe { door_sys::section_map(grant.handle, 0x7400_0000, door_abi::rights::READ) }?;
    if bytes == 0 || bytes > door_initfs::desktop_assets::MAX_BYTES as u64 {
        return Err(Status::InvalidArgument);
    }
    // The loader never issues a WRITE grant. This view remains mapped until
    // process teardown; immutable bytes cannot race a producer.
    let assets = door_window::wallpaper::Pack::parse(unsafe {
        core::slice::from_raw_parts(0x7400_0000 as *const u8, bytes as usize)
    })
    .ok_or(Status::InvalidArgument)?;
    let mut wallpaper = Vec::new();
    let mut scratch = captures.scratch().ok_or(Status::ResourceExhausted)?;
    door_window::wallpaper::load_with_scratch(
        assets.background(0).ok_or(Status::NotFound)?,
        info.width,
        info.height,
        &mut wallpaper,
        &mut scratch,
    )
    .map_err(|_| Status::IoFailure)?;
    captures.recycle(scratch);
    if (info.width, info.height) == (1024, 768)
        && let Ok(bgr) = miniz_oxide::inflate::decompress_to_vec_with_limit(
            include_bytes!("../../../assets/wallpapers/windows/bloom-1024x768.bgr.deflate"),
            1024 * 768 * 3,
        )
    {
        for (destination, source) in wallpaper.chunks_exact_mut(4).zip(bgr.chunks_exact(3)) {
            destination[..3].copy_from_slice(source);
        }
    }
    engine
        .set_wallpaper_material(&wallpaper)
        .map_err(|_| Status::IoFailure)?;
    let mut state = Desktop {
        engine,
        captures,
        display,
        registry: startup.base.inherited[inherited::REGISTRY].handle,
        fonts,
        wallpaper,
        lock_pixels: Vec::new(),
        assets,
        background: 0,
        controls: door_service_protocol::desktop::Controls::DEFAULT,
        accent: door_service_protocol::desktop::Accent::DEFAULT,
        accent_rgb: door_window::paint::DEFAULT_ACCENT,
        accent_reported: None,
        settings_navigation: None,
        scale_preference: 0,
        scale_hint,
        taskbar_material: door_window::material::Taskbar::new_scaled(
            info.width,
            info.height,
            scale,
        )
        .ok_or(Status::ResourceExhausted)?,
        overlay_underlay: alloc::vec![0; info.width as usize * door_window::policy::overlay_rows_scaled(info.height, true, scale) as usize * 4],
        presented_overlay: false,
        presented_overview: None,
        presentation_log: door_window::telemetry::PresentationLog::default(),
        motion: door_window::telemetry::Motion::default(),
        frame_clock: door_window::telemetry::FrameClock::new(),
        frame_time_reported: false,
        move_reuse_reported: false,
        tray: door_window::tray::State::default(),
        bar_dirty: false,
        buffers: Vec::new(),
        attachments: Vec::new(),
        clients: Vec::new(),
        session: None,
        login: None,
        login_pressed: false,
        login_waiting: false,
        login_reported: false,
        binding_presentation: None,
        x: info.width as i32 / 2,
        y: info.height as i32 / 2,
        buttons: 0,
        buttons_unknown: false,
        drag: None,
        win32: door_window::win32::Relations::new(),
        win32_press: None,
        caption_press: None,
        cursor_dirty: true,
        last_cursor: None,
        closing: Vec::new(),
        shortcut_keys: 0,
        system_keys: door_window::policy::SystemKeys::default(),
        overview: None,
        overview_focus: None,
        shell_focus: None,
        app_workspaces: [None; door_service_protocol::application::DESKTOP_APPS],
        hidden_windows: Vec::new(),
        launcher: false,
        shell: desktop::Shell::default(),
        signin: signin::SignIn::new(startup.base.inherited[inherited::REGISTRY].handle),
        shell_link: door_window::shell_link::Link::default(),
        shell_seen: Vec::new(),
        shell_focus_id: 0,
        shell_tick_at: 0,
        preview_hover: None,
        tooltip_hover: None,
        preview_hold: None,
        preview_notes: 8,
        desktop_press: None,
        desktop_selection: door_window::policy::DesktopSelection::default(),
        icon_hover: None,
        recycle_full: false,
        icons_dirty: false,
        overlay_buttons: 0,
        dark: false,
        app_classes: Vec::new(),
        admin_terminal: None,
        catalog_at: 0,
        catalog: None,
        pending_launches: 0,
        key_modifiers: 0,
        launching: None,
        foreground: door_window::policy::Foreground::default(),
        shutdown: None,
        signing_out: false,
        shutdown_since: 0,
        power_retry: door_window::policy::RetryGate::default(),
        terminations: door_window::policy::Terminations::default(),
        shutdown_frozen: false,
        unfreeze_pending: false,
        catalog_fresh: false,
        bar_reported: false,
        wheel_input: info.capabilities & door_abi::graphics::WHEEL != 0,
        resize_pending: None,
        transition_clock: door_window::telemetry::TransitionClock::default(),
        presentation: door_window::scanout::Presentation::default(),
        last_present_full: false,
    };
    state.apply_visual_preferences();
    // Calibrate the frame-timing TSC against the monotonic tick from here.
    state
        .frame_clock
        .start(door_window::telemetry::cycles(), door_sys::time()?);
    let mut server = Server::new(&startup)?;
    server.ready(door_abi::platform::manifest::WINDOW)?;
    let _ = door_sys::write(
        format!(
            "DOOR_P12 DENSITY physical={}x{} logical={}x{} factor={} percent={} hint={}\n",
            info.width,
            info.height,
            logical.0,
            logical.1,
            door_window::display_scale::factor_text(scale),
            scale.percent(),
            scale_hint,
        )
        .as_bytes(),
    );
    let _ = door_sys::write(b"DOOR_P11 DISPLAY READY cpu-copy=1 shared-readonly=1\n");
    while server.poll(&mut state)? {
        // Transitions started by this iteration's requests use current time.
        state.advance_transitions()?;
        state.gate_tick()?;
        state.refresh_desktop()?;
        state.input()?;
        // A pending scene update must not hold the visible pointer behind client
        // composition. It is restored once more against the completed frame.
        let presentable = state.presentation.ready(door_sys::time()?);
        if state.cursor_dirty && presentable {
            let result = state.present_cursor();
            state.presented("cursor", result)?;
        }
        state.track_responsiveness(door_sys::time()?);
        state.snip_tick(door_sys::time()?);
        state.expire_closed(door_sys::time()?);
        // Drag resting on a taskbar button; unfinished drops expire.
        state.dnd_tick();
        // door.accessibility: lock, shell trees and focus for readers.
        state.accessibility_tick();
        state.finish_shutdown();
        // Frame clock: the newest interactive size and the transition poses
        // for this frame; an idle scene queues nothing here.
        state.apply_resize();
        state.advance_transitions()?;
        if state.overview.is_some() && (state.bar_dirty || state.icons_dirty) {
            state.engine.invalidate();
        }
        // While presentation is backing off after a failed copy, nothing is
        // presented; requests and input are still serviced below.
        let presentable = state.presentation.ready(door_sys::time()?);
        if presentable {
            if state.engine.has_damage() {
                let result = state.present();
                if state.presented("scene", result)?
                    && state.session.is_none()
                    && !state.login_reported
                {
                    let _ = door_sys::write(b"DOOR_P12 SIGNIN DISPLAY READY\n");
                    state.login_reported = true;
                }
            } else if state.engine.has_caption_damage() {
                let result = state.present_captions();
                state.presented("captions", result)?;
            } else if state.icons_dirty {
                let result = state.present_icons();
                state.presented("icons", result)?;
            } else if state.bar_dirty {
                let result = state.present_bar();
                state.presented("bar", result)?;
            } else if state.cursor_dirty {
                let result = state.present_cursor();
                state.presented("cursor", result)?;
            }
        }
        if presentable
            && (state.engine.has_damage()
                || state.engine.has_caption_damage()
                || state.icons_dirty
                || state.bar_dirty
                || state.cursor_dirty
                || state.engine.animating())
        {
            // Input serviced during scanout may already have queued the next
            // repaint. Yield fairly instead of adding an unnecessary timer wait.
            door_sys::yield_now()?;
        } else {
            // A served POLL or other IPC reply can unblock a producer's next
            // read/export batch. Yield active bursts; only repeated empty
            // service scans back off. Pure idle/input retains timer polling.
            server.idle_backoff()?;
        }
    }
    Ok(())
}
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    door_rt::panic_with_info(info)
}
impl Drop for Desktop {
    fn drop(&mut self) {
        self.report_motion(door_window::telemetry::Cutoff::ScopeExit);
    }
}

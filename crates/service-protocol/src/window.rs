//! door.window v1. Owners always come from authenticated kernel metadata.
use crate::Code;
pub const BIND_SESSION: u32 = 0x5000;
pub const CREATE: u32 = 0x5001;
pub const REGISTER: u32 = 0x5002;
pub const ACK: u32 = 0x5003;
pub const ATTACH: u32 = 0x5004;
pub const COMMIT: u32 = 0x5005;
pub const UNREGISTER: u32 = 0x5006;
pub const POLL: u32 = 0x5007;
pub const DESTROY: u32 = 0x5008;
pub const STATE: u32 = 0x5009;
pub const CONFIGURE: u32 = 0x500a;
pub const FRAME: u32 = 0x500b;
pub const HELLO: u32 = 0x500c;
pub const SET_TITLE: u32 = 0x500d;
pub const REQUEST_CLOSE: u32 = 0x500e;
pub const PING: u32 = 0x500f;
/// Privileged Init broker operation; never a client window operation.
pub const TERMINATE_CLIENT: u32 = 0x5010;
pub const CAPTURE: u32 = 0x5011;
pub const RELEASE_CAPTURE: u32 = 0x5012;
pub const POLL_BATCH: u32 = 0x5013;
pub const CANCEL_CLOSE: u32 = 0x5014;
pub const SET_CURSOR: u32 = 0x5015;
/// Active session appearance, 0=light/1=dark title bars and trusted desktop.
pub const SET_THEME: u32 = 0x5016;
/// Session-scoped catalog activation, IDs1..3 only; never power/freeze control.
pub const LAUNCH_APP: u32 = 0x5017;
pub const SET_APPEARANCE: u32 = 0x5018;
/// Built-in wallpapers (`door_initfs::desktop_assets::COUNT`): the
/// background index of an appearance word (`SET_APPEARANCE`, setting 2) is
/// `value >> 1`, below this.
pub const WALLPAPERS: u32 = 18;
/// Appearance words are below this: bit 0 dark, bits 1.. the background.
pub const APPEARANCE_VALUES: u32 = WALLPAPERS << 1;
pub const SET_DESKTOP_CONTROLS: u32 = 0x5019;
pub const SET_TABBED_CAPTION: u32 = 0x501a;
/// Same-owner tab-drop target at the trusted current pointer position.
pub const TAB_TARGET: u32 = 0x501b;
pub const LAUNCH_WINDOW: u32 = 0x501c;
/// Init only (session 0): the session user's persisted shell layout (8 bytes:
/// `desktop::ShellLayout` flags and order), applied at login.
pub const SET_SHELL_LAYOUT: u32 = 0x501e;
pub const CAP_MULTI_WINDOW: u32 = 128;
/// Server-advertised (never requested): 24-byte COMMIT(window, epoch, top,
/// rows) patches only those rows of the trusted snapshot.
pub const CAP_BAND_COMMIT: u32 = 256;
/// Requested in HELLO; granted (echoed) only when the pointing device has a
/// wheel. Granted owners receive INPUT events with detail `INPUT_WHEEL`.
pub const CAP_WHEEL: u32 = 512;
/// Server-advertised to negotiating (8-byte HELLO) clients, never requested:
/// SET_MATERIAL accepts MATERIAL_ACRYLIC and MATERIAL_MICA.
pub const CAP_ACRYLIC: u32 = 1024;
/// 12 bytes: window u64, material u32. Translucent client pixels compose over
/// the selected material: the wallpaper material (default), a live blur of
/// the windows behind (acrylic, for flyouts and transient surfaces) or Mica
/// (the wallpaper under the theme tint, also behind the trusted title bar;
/// solid #202020 / #F3F3F3 while the window is inactive, as on Windows 11).
/// Reduce-transparency substitutes the solid fallback.
pub const SET_MATERIAL: u32 = 0x501d;
pub const MATERIAL_DEFAULT: u32 = 0;
pub const MATERIAL_ACRYLIC: u32 = 1;
pub const MATERIAL_MICA: u32 = 2;
/// INPUT details: 1 key, 2 pointer motion, 3 pointer button, 4 wheel. A wheel
/// event carries the content-local physical pointer position in rect.x/y and
/// signed detents in `a` (two's complement, > 0 = away from the user).
pub const INPUT_WHEEL: u32 = 4;
pub const CURSOR_ARROW: u32 = 0;
pub const CURSOR_TEXT: u32 = 1;
/// Horizontal resize (IDC_SIZEWE): splitters such as Explorer's navigation
/// pane edge.
pub const CURSOR_SIZE_WE: u32 = 2;
pub const VERSION: u32 = 1;
pub const CAP_CPU_SNAPSHOT: u32 = 1;
pub const CAP_TRANSIENT_DISMISS: u32 = 2;
pub const CAP_EPOCH_COMMIT: u32 = 4;
pub const CAP_CLIENT_CAPTION: u32 = 8;
pub const CAP_APPEARANCE: u32 = 16;
pub const CAP_DESKTOP_CONTROLS: u32 = 32;
pub const CLIENT_CAPTION_ROLE: u32 = 4;
/// Left-aligned interactive tab region, using existing trusted frame controls.
pub const TABBED_CAPTION_ROLE: u32 = 5;
pub const CAP_TABBED_CAPTION: u32 = 64;
pub const HELLO_BYTES: usize = 64;
pub const EVENT_BYTES: usize = 64;
pub const CONFIGURED: u32 = 1;
pub const RELEASED: u32 = 2;
pub const INPUT: u32 = 3;
pub const FOCUS: u32 = 4;
pub const CAPTURE_LOST: u32 = 5;
pub const CLOSE: u32 = 6;
pub const FRAME_READY: u32 = 7;
pub const PRESENTED: u32 = 8;
pub const DISMISS_TRANSIENTS: u32 = 9;
/// value = appearance (bit 0 dark, bits 1.. background); a = the system
/// accent (`desktop::Accent::from_event_word`; 0 from older servers).
pub const APPEARANCE_CHANGED: u32 = 10;
/// Checked desktop::Controls bits in Event.a; no window authority is conveyed.
pub const CONTROLS_CHANGED: u32 = 11;
/// Route an authenticated Settings singleton to Taskbar (15) or Context menu (16).
pub const SETTINGS_NAVIGATE: u32 = 12;
pub const NEW_WINDOW: u32 = 13;
/// Client capability (Start search): this client answers file and folder
/// name queries for the shell. The shell sends SEARCH_QUERY / SEARCH_OPEN /
/// SEARCH_EXPLORE events; the provider answers with SEARCH_QUERY_GET and
/// SEARCH_RESULTS. Only clients that request it receive these events.
pub const CAP_SEARCH_PROVIDER: u32 = 2048;
/// Search provider only, 4 bytes (serial): the query text for the current
/// serial as UTF-8 (at most `search::MAX_QUERY` bytes). Stale: NotFound.
pub const SEARCH_QUERY_GET: u32 = 0x501f;
/// Search provider only: one result per call (`search::encode`). Results for
/// anything but the current serial are dropped by the shell.
pub const SEARCH_RESULTS: u32 = 0x5020;
/// Event: a new query is ready (a = serial).
pub const SEARCH_QUERY: u32 = 14;
/// Event: open result `b` of query `a`: a folder opens in File Explorer, a
/// file opens in its app (Notes, Preview) or, with `detail` = 1 ("Open file
/// location") or no app, in File Explorer with the file selected.
pub const SEARCH_OPEN: u32 = 15;
/// Event: open File Explorer running query `a` ("Search in File Explorer").
pub const SEARCH_EXPLORE: u32 = 16;
/// Search provider only: open a document in an app. Body: app command u32
/// (1 File Explorer: an encoded Explorer location/search request; 2 Notes or
/// 10 Preview: a Door path) then 1..=`MAX_DOCUMENT` UTF-8 bytes. The shell
/// launches or activates the app and posts DOCUMENT_OPEN to it.
pub const OPEN_DOCUMENT: u32 = 0x5021;
/// App only, empty body: the pending document payload for this app (consumed).
pub const DOCUMENT_GET: u32 = 0x5022;
/// Owner of window (8 bytes, id): the user chose Cancel in the window's own
/// close prompt. CANCEL_CLOSE only defers a close to that prompt; during a
/// shutdown this cancels the shutdown (Windows 11), otherwise it is a no-op.
pub const CLOSE_DECLINED: u32 = 0x5023;
/// Event to the app chosen by OPEN_DOCUMENT (a = payload bytes); the app
/// reads it with DOCUMENT_GET. Delivered to CAP_DESKTOP_CONTROLS clients.
pub const DOCUMENT_OPEN: u32 = 17;
pub const MAX_DOCUMENT: usize = 224;
/// Event to the search provider when Start opens (a = serial): answer with
/// the user's recent items, newest first (at most `search::MAX_RECENT`), as
/// SEARCH_RESULTS for that serial. SEARCH_OPEN then opens one (b = index,
/// detail 1 = "Open file location").
pub const SEARCH_RECENT: u32 = 18;
/// Event to the search provider: "Remove from list" for recent item `b` of
/// serial `a`. The shell asks for a fresh list afterwards.
pub const SEARCH_RECENT_REMOVE: u32 = 19;
/// Requested in HELLO and always granted: the client renders at the
/// fractional display scale. Its HELLO reply reports DPI = 96 x percent / 100
/// (120, 144, 168, 192, 216, 240 or 288) and it receives SCALE_CHANGED
/// (initially and on every change). Clients without it see the nearest
/// integer DPI (96/192/288, exact halves rounding up) and no scale events.
pub const CAP_DISPLAY_SCALE: u32 = 4096;
/// 4 bytes: percent preference, 0 = automatic (host hint, else the integer
/// heuristic) or 100..=300 in steps of 25. From Init (session binding) or
/// a bound-session client (Settings). Scales that leave less than 1024x600
/// logical pixels are refused (InvalidRequest). The reply scalar is the
/// resulting effective percent. The caller persists the preference
/// (storage `ScalePercent`); the server never writes settings.
pub const SET_DISPLAY_SCALE: u32 = 0x5024;
/// No body; 16-byte reply: preference u32, effective percent u32,
/// recommended (automatic) percent u32, and a bit mask of offered percents
/// (bit i = `DISPLAY_SCALES[i]`).
pub const DISPLAY_SCALE_INFO: u32 = 0x5025;
/// Windows display scales, in the order of `DISPLAY_SCALE_INFO` bits.
pub const DISPLAY_SCALES: [u32; 8] = [100, 125, 150, 175, 200, 225, 250, 300];
/// Logical extent of a physical output dimension at `percent`: the largest
/// logical length whose snapped physical length (round(l x percent / 100),
/// halves down) fits, as the compositor's `Scale::logical_len`.
pub const fn logical_extent(physical: u32, percent: u32) -> u32 {
    ((physical as u64 * 100 + 50) / percent as u64) as u32
}
/// Whether a display scale is offered for an output: a Windows scale that
/// keeps at least 1024x600 logical pixels (1080p offers 100..=175%).
pub const fn display_scale_offered(width: u32, height: u32, percent: u32) -> bool {
    let mut i = 0;
    let mut known = false;
    while i < DISPLAY_SCALES.len() {
        known |= DISPLAY_SCALES[i] == percent;
        i += 1;
    }
    known && logical_extent(width, percent) >= 1024 && logical_extent(height, percent) >= 600
}
/// Event for CAP_DISPLAY_SCALE clients: a = effective percent, b = DPI,
/// rect = physical output (0, 0, width, height), value = preference << 32 |
/// recommended percent. Precedes the CONFIGURED events that resize every
/// window to keep its logical size.
pub const SCALE_CHANGED: u32 = 20;
/// 8 bytes: time zone id u32 (`door_locale::tz::Zone::id`), regional format
/// id u32 (`door_locale::region::Region::id`). From Init (session binding)
/// or a bound-session workspace client (Settings). The taskbar clock and
/// calendar show local time in that region's format. Unknown ids are
/// InvalidRequest. The caller persists the settings (`TimeZone`,
/// `RegionalFormat`); the server never writes settings.
pub const SET_TIME_LOCALE: u32 = 0x5026;
/// Win32 window relations (P13, native-window-v1 "Win32 window relations").
/// Requested in HELLO; servers that implement SET_OWNER, SET_ENABLED,
/// MOVE_RESIZE and SET_SIZE_LIMITS echo it. Clients without it keep the
/// exact native behavior.
pub const CAP_WIN32: u32 = 8192;
/// 16 bytes: window u64, owner window u64 (0 clears). Both windows belong
/// to the caller; self and cyclic ownership are InvalidRequest. Destroying
/// an owner leaves its owned windows unowned.
pub const SET_OWNER: u32 = 0x5027;
/// 12 bytes: window u64, enabled u32 (0 or 1). A disabled window receives no
/// pointer or key input and cannot be closed from its caption; a press on it
/// activates its most recent enabled owned window (the modal dialog chain).
pub const SET_ENABLED: u32 = 0x5028;
/// 24 bytes: window u64, edges u32 (0 = move, else `EDGE_*` bits), reserved
/// u32 zero, press serial u64 (the timestamp of the INPUT button event the
/// client is answering). Starts the trusted interactive move/resize exactly
/// as a caption or border drag would, only while that primary-button press
/// on that window is still held; otherwise StaleInstance. This is how
/// client-drawn chrome answers its own non-client hit test (WM_NCHITTEST).
pub const MOVE_RESIZE: u32 = 0x5029;
pub const EDGE_LEFT: u32 = 1;
pub const EDGE_TOP: u32 = 2;
pub const EDGE_RIGHT: u32 = 4;
pub const EDGE_BOTTOM: u32 = 8;
/// 24 bytes: window u64, minimum width u32, minimum height u32, maximum
/// width u32, maximum height u32 (outer pixels; a zero maximum is unbounded;
/// minimum above maximum is InvalidRequest). Interactive resizes and
/// CONFIGURE requests are clamped to them (WM_GETMINMAXINFO tracking sizes).
pub const SET_SIZE_LIMITS: u32 = 0x502a;
/// 12 bytes: window u64, frameless u32 (0 or 1). A frameless window (a Win32
/// window without WS_CAPTION: popups, menus, client-drawn chrome) has no
/// trusted caption buttons, resize border or title bar: every pixel is client
/// area and the client answers its own non-client hit test (MOVE_RESIZE).
pub const SET_FRAMELESS: u32 = 0x502b;
/// 12 bytes: window u64, buttons u32 (bit 0 Minimize, bit 1 Maximize: a
/// Win32 window's WS_MINIMIZEBOX/WS_MAXIMIZEBOX). With neither the caption
/// shows only Close; with one, both show and the other is greyed and does
/// nothing (CAP_WIN32).
pub const SET_CAPTION_BUTTONS: u32 = 0x5039;
/// 12 bytes: window u64, responding u32 (0 or 1). A CAP_WIN32 client reports
/// that the thread owning this window has not retrieved messages for five
/// seconds while messages are pending (0), or retrieves them again (1), while
/// another of its threads still polls the service. The server then shows the
/// window as not responding exactly as when the whole client stops polling
/// (native-window-v1 "Win32 close and not responding").
pub const SET_RESPONDING: u32 = 0x502c;
/// 4 bytes: the `desktop::Accent` setting word. From Init (session binding)
/// or a bound-session workspace client (Settings). The server resolves the
/// colour (Automatic picks it from the desktop background, again whenever
/// that changes), paints the shell with its palette and sends every
/// CAP_APPEARANCE client APPEARANCE_CHANGED with `a` =
/// `desktop::Accent::event_word(resolved 0xRRGGBB)`. Invalid words are
/// InvalidRequest. The caller persists the `AccentColor` setting; the server
/// never writes settings.
pub const SET_ACCENT: u32 = 0x502d;
/// 4 bytes: 1 when the session user's Recycle Bin holds items, else 0.
/// From bound-session workspace clients (File Explorer reports it whenever
/// it reads the bin). The desktop's Recycle Bin icon shows the full or
/// empty art.
pub const SET_RECYCLE_BIN: u32 = 0x502e;
/// 8 bytes: `[id u32][value u32]`, one `preferences` word (ids 10 and up,
/// `door_service_protocol::preferences`). From Init (session binding: every
/// word) or a bound-session workspace client (Settings, when the user
/// changes it). Unknown ids and invalid words are InvalidRequest. The
/// caller persists the word (Storage SET_SETTING); the server never writes
/// settings. The server applies the words it consumes (accent surfaces,
/// theme schedule, sign-in-away lock, lock screen, regional overrides,
/// clipboard history, search history, End task) and relays the rest to
/// the shell provider where one consumes them.
pub const SET_PREFERENCE: u32 = 0x502f;
/// Event to the search provider (Start search history, P14): `detail` 0
/// asks for the user's recent searches now (answer with SEARCH_HISTORY_SET,
/// newest first); `detail` 1 says the user committed a query (Enter, or
/// opening a result) while "Search history on this device" is on: read it
/// with SEARCH_HISTORY_GET(`a`), remember it and send the list back. The
/// shell posts neither while the setting is off.
pub const SEARCH_HISTORY: u32 = 21;
/// Event to Snipping Tool (manifest `SNIPPING_TOOL`): a snip was taken and
/// is on the clipboard as `clipboard::IMAGE` (a = width, b = height in
/// physical pixels); the app reads it with the clipboard READ it may make
/// while its window is focused (the shell focuses it first).
pub const SNIP_READY: u32 = 22;
/// Event: a notification icon was clicked (`NOTIFY_ICON`). `a` = its id,
/// `b` = the click (1 primary, 2 secondary, 3 double, 4 keyboard), `detail`
/// = the icon's NOTIFYICON_VERSION, `value` = the owner's window (as it
/// registered it), `rect` = the icon's screen rectangle.
pub const NOTIFY_ICON_EVENT: u32 = 23;
/// The newest event kind `Event::decode` accepts.
pub const LAST_EVENT: u32 = NOTIFY_ICON_EVENT;
pub const HISTORY_SEND: u32 = 0;
pub const HISTORY_COMMITTED: u32 = 1;
/// Search provider only, 4 bytes (commit id from SEARCH_HISTORY): the
/// committed query as UTF-8, already trimmed and clipped to
/// `search_history::MAX_UNITS` UTF-16 units. Unknown or old ids: NotFound.
pub const SEARCH_HISTORY_GET: u32 = 0x5030;
/// Search provider only: one recent search per call
/// (`search_history::encode`); index 0 replaces the list, count 0 empties
/// it. Start's Search home shows them in its Recent list while the setting
/// is on; a click fills the search box and runs the query.
pub const SEARCH_HISTORY_SET: u32 = 0x5031;
/// Snipping Tool only (manifest `SNIPPING_TOOL`), 8 bytes `[mode u32]
/// [delay seconds u32]`: hide the caller's windows and open the shell's
/// snip overlay (Win+Shift+S) after the delay (0, 3, 5 or 10 s). Modes:
/// `snip::RECTANGLE`, `WINDOW`, `FULL_SCREEN`, `FREE_FORM`. The result
/// arrives as SNIP_READY; a cancelled snip restores the windows only.
pub const SNIP_BEGIN: u32 = 0x5032;
pub mod snip {
    pub const RECTANGLE: u32 = 0;
    pub const WINDOW: u32 = 1;
    pub const FULL_SCREEN: u32 = 2;
    pub const FREE_FORM: u32 = 3;
}
/// Magnifier only (manifest `MAGNIFIER`), 12 bytes `[percent u32][view
/// u32][flags u32]`: the shell magnifies its composed output by
/// `percent` (100 turns it off, 100..=1600), `view` 0 full screen
/// (following the pointer), 1 lens (a rectangle around the pointer), 2
/// docked (a band across the top). The shell undoes it when the Magnifier
/// process exits.
pub const MAGNIFY: u32 = 0x5033;
/// On-Screen Keyboard only (manifest `ON_SCREEN_KEYBOARD`), 8 bytes
/// `[scan u32][flags u32]`: deliver a key press and release (flags bit 0
/// Shift, 1 Ctrl, 2 Alt, 3 Win; bit 8 press only, bit 9 release only) to
/// the focused window. The keyboard's own windows never take focus.
pub const INJECT_KEY: u32 = 0x5034;
/// 12 bytes `[window u64][on u32]`: enter (1) or leave (0) full screen.
/// The window covers the whole output with no frame and the taskbar stays
/// hidden while it is focused; leaving restores its placement. Entering is
/// refused unless the window is focused (no background takeover).
pub const SET_FULLSCREEN: u32 = 0x5035;
/// Run as administrator, from File Explorer or a Windows program's
/// ShellExecuteEx "runas" (the kernel-authenticated manifest decides). `[flags u32][path len
/// u16][path UTF-8]` with transfers `[cwd directory, image section]`; a
/// batch file (`elevation::SCRIPT`) sends no image. The window server, not
/// the app, does the rest, as Windows' consent service does: it verifies
/// the image's Authenticode signature, shows User Account Control with the
/// verified publisher (or the unknown-publisher prompt) on its secure
/// desktop and, on approval, starts the program itself (Init
/// `application::EXECUTE_ELEVATED`, bound to the image's SHA-256) or an
/// administrator Terminal tab for a batch file. The call answers once the
/// prompt opened; `ELEVATION_STATUS` then tells the caller what the user
/// decided. One prompt at a time.
pub const REQUEST_ELEVATION: u32 = 0x5036;
/// The outcome of the caller's last REQUEST_ELEVATION of a program, as
/// ShellExecuteEx's "runas" returns it: WouldBlock while the prompt is
/// open or Init is starting it, then (once) a process handle (wait and
/// query rights) when it started, `elevation::DECLINED` when the user
/// answered No, `elevation::FAILED` when it could not start; NotFound when
/// nothing is pending for the caller.
pub const ELEVATION_STATUS: u32 = 0x503b;
/// A Windows program's ShellExecute of a document no Windows association
/// opens (or the "openas" verb), as OpenWith.exe takes it on Windows:
/// `[mode u8][native path UTF-8]`, mode `open_with::OPEN` (the file's
/// default app in File Explorer, or the picker when none is set) or
/// `open_with::CHOOSE` (always the picker). Windows programs only; File
/// Explorer shows it.
pub const OPEN_WITH: u32 = 0x5037;
/// Settings' firewall page (manifest WORKSPACE): a change Windows asks an
/// administrator to approve, as a `firewall::CHANGE` body. The window
/// server shows User Account Control and, on approval, has Init store it;
/// the caller only learns that the prompt opened.
pub const FIREWALL_CHANGE: u32 = 0x503a;
pub mod open_with {
    pub const OPEN: u8 = b'W';
    pub const CHOOSE: u8 = b'A';
}
/// A Windows program's notification-area icon (Shell_NotifyIcon), from a
/// Win32 window client: `[op u8][version u8][flags u16][window u64][id
/// u32][callback u32][tip length u8][tip UTF-8]`, with one read-only
/// section transfer of `notify::ICON_BYTES` (32x32 premultiplied BGRA)
/// when `flags` has `notify::ICON`. The icons live in the taskbar's
/// "Show hidden icons" flyout, as new programs' icons do on Windows 11;
/// clicks come back to the owner as `NOTIFY_ICON_EVENT`. An owner's icons
/// go with its connection.
pub const NOTIFY_ICON: u32 = 0x5038;
pub mod notify {
    pub const ADD: u8 = 0;
    pub const MODIFY: u8 = 1;
    pub const DELETE: u8 = 2;
    pub const SET_FOCUS: u8 = 3;
    pub const SET_VERSION: u8 = 4;
    /// Where the icon is on screen (Shell_NotifyIconGetRect): no tip or
    /// section; the reply is `[x i32][y i32][width u32][height u32]`, the
    /// icon's cell while the hidden-icons flyout is open, else the "Show
    /// hidden icons" chevron it sits behind.
    pub const RECT: u8 = 5;
    /// NIF_* bits the window server keeps.
    pub const MESSAGE: u16 = 0x1;
    pub const ICON: u16 = 0x2;
    pub const TIP: u16 = 0x4;
    pub const EDGE: u32 = 32;
    pub const ICON_BYTES: usize = (EDGE * EDGE * 4) as usize;
    pub const MAX_TIP: usize = 127;
    /// Icons one client may show.
    pub const MAX_ICONS: usize = 16;
}
pub mod elevation {
    /// A batch file (.bat/.cmd): Doors Command Processor runs it elevated.
    pub const SCRIPT: u32 = 1;
    /// Downloaded from the Internet (the file carries a mark of the web).
    pub const DOWNLOADED: u32 = 2;
    /// Windows Installer for a package: the path is `program NUL package
    /// NUL binding`, the program must be the system's msiexec.exe, and the
    /// prompt names the package's file, as Windows names an .msi rather
    /// than msiexec.exe. `binding` is the lower-case hex SHA-256 of the
    /// package's SHA-256 (lower-case hex), NUL and the command line, so the
    /// request fits one message whatever the command line's length. On
    /// approval the elevated msiexec.exe gets one argument,
    /// `CONSENT_SWITCH` and the hex SHA-256 of `CONSENT_TAG`, the package,
    /// NUL and the binding: it acts only on a hand-off whose values give
    /// the same digest, and only on package bytes with that hash, so
    /// nothing changed after the prompt is installed.
    pub const PACKAGE: u32 = 4;
    /// The program's arguments follow its path: `program NUL arguments`,
    /// each argument NUL-terminated so an empty one survives
    /// (ShellExecuteEx's lpParameters, split as CommandLineToArgvW splits
    /// them). Not with SCRIPT or PACKAGE.
    pub const ARGUMENTS: u32 = 8;
    /// ELEVATION_STATUS answers (scalars).
    pub const DECLINED: u64 = 1;
    pub const FAILED: u64 = 2;
    /// The longest path text: one REQUEST_ELEVATION message (its 10-byte
    /// header first).
    pub const MAX_PATH: usize = crate::wire::BODY_BYTES - 10;
    pub const CONSENT_TAG: &[u8] = b"DOORS-MSI-CONSENT\0";
    pub const CONSENT_SWITCH: &str = "/DoorsConsent:";
    /// Where msiexec.exe lives (native, and as Windows programs name it).
    pub const MSIEXEC: [&str; 4] = [
        "/compat/Windows/System32/msiexec.exe",
        "C:\\Windows\\System32\\msiexec.exe",
        "/compat/Windows/SysWOW64/msiexec.exe",
        "C:\\Windows\\SysWOW64\\msiexec.exe",
    ];
}
/// Init operation from the authenticated current window server (signed-in
/// session only), 8 bytes `[setting id u32][value u32]`: persist a setting
/// the shell changed in place for the session user, exactly as Settings
/// would write it. Accepted ids: `Setting::DesktopAppearance` (the
/// dark-mode schedule switched the mode) and the `preferences` words
/// (ids 10..=20, e.g. Win+V "Turn on" sets `CLIPBOARD`). Init validates
/// the value, storage validates it again; the server has already applied
/// it, so nothing is sent back.
pub const PERSIST_SETTING: u32 = 0x50e0;
pub mod search_history {
    //! Start search history entries (`SEARCH_HISTORY_SET`):
    //! `[count u8][index u8][text]`, text 1..=`MAX_TEXT` bytes of UTF-8
    //! without control characters, or just `[0][0]` for an empty list.
    use crate::Code;
    /// Windows keeps the ten newest searches.
    pub const MAX_ENTRIES: usize = 10;
    /// Each search is stored as a REG_SZ of at most 64 bytes (31 UTF-16
    /// units and the terminator) under `HKCU\Software\Doors\Search\History`.
    pub const MAX_UNITS: usize = 31;
    pub const MAX_TEXT: usize = MAX_UNITS * 3;
    pub const KEY: &str = r"Software\Doors\Search\History";
    /// A decoded SEARCH_HISTORY_SET body: (count, Some((index, text))).
    pub type Entry<'a> = (u8, Option<(u8, &'a str)>);
    /// `text` trimmed and cut at a character boundary to `MAX_UNITS`
    /// UTF-16 units (None when nothing is left).
    pub fn clip(text: &str) -> Option<&str> {
        let text = text.trim();
        let mut units = 0;
        let mut end = 0;
        for (i, ch) in text.char_indices() {
            units += ch.len_utf16();
            if units > MAX_UNITS {
                break;
            }
            end = i + ch.len_utf8();
        }
        let clipped = text[..end].trim_end();
        (!clipped.is_empty() && !clipped.chars().any(char::is_control)).then_some(clipped)
    }
    pub fn encode(count: u8, entry: Option<(u8, &str)>, out: &mut [u8]) -> Result<usize, Code> {
        if count as usize > MAX_ENTRIES || out.len() < 2 + MAX_TEXT {
            return Err(Code::InvalidRequest);
        }
        out[0] = count;
        let Some((index, text)) = entry else {
            out[1] = 0;
            return if count == 0 {
                Ok(2)
            } else {
                Err(Code::InvalidRequest)
            };
        };
        if index >= count
            || text.is_empty()
            || text.len() > MAX_TEXT
            || text.chars().any(char::is_control)
        {
            return Err(Code::InvalidRequest);
        }
        out[1] = index;
        out[2..2 + text.len()].copy_from_slice(text.as_bytes());
        Ok(2 + text.len())
    }
    /// (count, Some((index, text))) or (0, None).
    pub fn decode(b: &[u8]) -> Result<Entry<'_>, Code> {
        if b.len() < 2 || b[0] as usize > MAX_ENTRIES || b.len() > 2 + MAX_TEXT {
            return Err(Code::InvalidRequest);
        }
        if b[0] == 0 {
            return if b.len() == 2 && b[1] == 0 {
                Ok((0, None))
            } else {
                Err(Code::InvalidRequest)
            };
        }
        let text = core::str::from_utf8(&b[2..]).map_err(|_| Code::InvalidRequest)?;
        if b[1] >= b[0] || text.is_empty() || text.chars().any(char::is_control) {
            return Err(Code::InvalidRequest);
        }
        Ok((b[0], Some((b[1], text))))
    }
}
/// Init only, and only on the adversarial test image (`system/window-tests.conf`):
/// enable the window server's test hooks, today the DISPLAY-01 presentation
/// fault chord (Win+Ctrl+left Shift+F12). Empty body; production images never
/// send it.
pub const SET_TEST_HOOKS: u32 = 0x50f0;
pub mod search {
    //! Bounded Start search result payload shared by the shell and providers.
    //! One SEARCH_RESULTS call carries one result (RPC bodies are 240
    //! bytes): `[serial u32][state][count][index][kind][name_len][path_len]
    //! [name][path]`, or just `[serial][state][0]` for no results. A result
    //! set is at most `MAX_RESULTS` entries (under 2 KiB in all).
    use crate::{BODY_BYTES, Code};
    pub const MAX_RESULTS: usize = 8;
    /// Start > Recent shows up to six items (three rows of two).
    pub const MAX_RECENT: usize = 6;
    pub const MAX_QUERY: usize = 128;
    pub const MAX_NAME: usize = 128;
    const HEADER: usize = 10;
    /// Longest display path that fits beside a `MAX_NAME` name.
    pub const MAX_PATH: usize = BODY_BYTES - HEADER - MAX_NAME;
    pub const FILE: u8 = 0;
    pub const FOLDER: u8 = 1;
    /// Provider index state: still indexing, complete, or stopped at a bound.
    pub const BUILDING: u8 = 0;
    pub const COMPLETE: u8 = 1;
    pub const PARTIAL: u8 = 2;
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Entry<'a> {
        pub kind: u8,
        /// Display name (file or folder name).
        pub name: &'a str,
        /// Windows-style location shown under the name ("C:\Users\...").
        pub path: &'a str,
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Message<'a> {
        pub serial: u32,
        pub state: u8,
        pub count: u8,
        /// (index, entry); None for an empty result set.
        pub entry: Option<(u8, Entry<'a>)>,
    }
    /// Encode one message; `entry` must be None exactly when `count` is 0.
    pub fn encode(message: Message<'_>, out: &mut [u8]) -> Result<usize, Code> {
        let Message {
            serial,
            state,
            count,
            entry,
        } = message;
        if state > PARTIAL || count as usize > MAX_RESULTS || out.len() < 6 {
            return Err(Code::InvalidRequest);
        }
        out[..4].copy_from_slice(&serial.to_le_bytes());
        out[4] = state;
        out[5] = count;
        let Some((index, e)) = entry else {
            return if count == 0 {
                Ok(6)
            } else {
                Err(Code::InvalidRequest)
            };
        };
        let (n, p) = (e.name.as_bytes(), e.path.as_bytes());
        if index >= count
            || e.kind > FOLDER
            || n.is_empty()
            || n.len() > MAX_NAME
            || p.len() > MAX_PATH
        {
            return Err(Code::InvalidRequest);
        }
        let end = HEADER + n.len() + p.len();
        if end > out.len() || end > BODY_BYTES {
            return Err(Code::InvalidRequest);
        }
        out[6] = index;
        out[7] = e.kind;
        out[8] = n.len() as u8;
        out[9] = p.len() as u8;
        out[HEADER..HEADER + n.len()].copy_from_slice(n);
        out[HEADER + n.len()..end].copy_from_slice(p);
        Ok(end)
    }
    fn text(bytes: &[u8]) -> Result<&str, Code> {
        let s = core::str::from_utf8(bytes).map_err(|_| Code::InvalidRequest)?;
        if s.chars().any(char::is_control) {
            return Err(Code::InvalidRequest);
        }
        Ok(s)
    }
    /// Validate one message: exact length, known state/kind, index < count,
    /// no control characters, bounded fields.
    pub fn decode(b: &[u8]) -> Result<Message<'_>, Code> {
        if b.len() < 6 || b.len() > BODY_BYTES || b[4] > PARTIAL || b[5] as usize > MAX_RESULTS {
            return Err(Code::InvalidRequest);
        }
        let serial = u32::from_le_bytes(b[..4].try_into().unwrap());
        let (state, count) = (b[4], b[5]);
        if count == 0 {
            return if b.len() == 6 {
                Ok(Message {
                    serial,
                    state,
                    count,
                    entry: None,
                })
            } else {
                Err(Code::InvalidRequest)
            };
        }
        if b.len() < HEADER || b[6] >= count || b[7] > FOLDER {
            return Err(Code::InvalidRequest);
        }
        let (n, p) = (b[8] as usize, b[9] as usize);
        if n == 0 || n > MAX_NAME || p > MAX_PATH || b.len() != HEADER + n + p {
            return Err(Code::InvalidRequest);
        }
        Ok(Message {
            serial,
            state,
            count,
            entry: Some((
                b[6],
                Entry {
                    kind: b[7],
                    name: text(&b[HEADER..HEADER + n])?,
                    path: text(&b[HEADER + n..])?,
                },
            )),
        })
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl Rect {
    pub fn encode(self, out: &mut [u8]) {
        out[..4].copy_from_slice(&self.x.to_le_bytes());
        out[4..8].copy_from_slice(&self.y.to_le_bytes());
        out[8..12].copy_from_slice(&self.width.to_le_bytes());
        out[12..16].copy_from_slice(&self.height.to_le_bytes());
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() != 16 {
            return Err(Code::InvalidRequest);
        }
        Ok(Self {
            x: i32::from_le_bytes(b[..4].try_into().unwrap()),
            y: i32::from_le_bytes(b[4..8].try_into().unwrap()),
            width: u32::from_le_bytes(b[8..12].try_into().unwrap()),
            height: u32::from_le_bytes(b[12..].try_into().unwrap()),
        })
    }
}
/// Fixed little-endian event: kind/detail, window, epoch/serial/time, rect,
/// content width/height or key/button fields, and timestamp. Reserved bytes zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Event {
    pub kind: u32,
    pub detail: u32,
    pub window: u64,
    pub value: u64,
    pub rect: Rect,
    pub a: u32,
    pub b: u32,
    pub timestamp: u64,
}
impl Event {
    pub fn encode(self) -> [u8; EVENT_BYTES] {
        let mut o = [0; EVENT_BYTES];
        o[..4].copy_from_slice(&self.kind.to_le_bytes());
        o[4..8].copy_from_slice(&self.detail.to_le_bytes());
        o[8..16].copy_from_slice(&self.window.to_le_bytes());
        o[16..24].copy_from_slice(&self.value.to_le_bytes());
        self.rect.encode(&mut o[24..40]);
        o[40..44].copy_from_slice(&self.a.to_le_bytes());
        o[44..48].copy_from_slice(&self.b.to_le_bytes());
        o[48..56].copy_from_slice(&self.timestamp.to_le_bytes());
        o
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() != EVENT_BYTES || b[56..].iter().any(|&v| v != 0) {
            return Err(Code::InvalidRequest);
        }
        let kind = u32::from_le_bytes(b[..4].try_into().unwrap());
        if !(1..=LAST_EVENT).contains(&kind) {
            return Err(Code::InvalidRequest);
        }
        Ok(Self {
            kind,
            detail: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            window: u64::from_le_bytes(b[8..16].try_into().unwrap()),
            value: u64::from_le_bytes(b[16..24].try_into().unwrap()),
            rect: Rect::decode(&b[24..40])?,
            a: u32::from_le_bytes(b[40..44].try_into().unwrap()),
            b: u32::from_le_bytes(b[44..48].try_into().unwrap()),
            timestamp: u64::from_le_bytes(b[48..56].try_into().unwrap()),
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checked_event_roundtrip() {
        let e = Event {
            kind: INPUT,
            rect: Rect {
                x: -8,
                y: 6,
                width: 31,
                height: 51,
            },
            window: u64::MAX,
            timestamp: 42,
            ..Event::default()
        };
        assert_eq!(Event::decode(&e.encode()), Ok(e));
        for n in 0..EVENT_BYTES {
            assert!(Event::decode(&e.encode()[..n]).is_err())
        }
        let mut b = e.encode();
        b[63] = 1;
        assert!(Event::decode(&b).is_err());
    }
    #[test]
    fn search_results_roundtrip_and_reject_malformed_payloads() {
        use search::{Entry, Message};
        let entry = Entry {
            kind: search::FILE,
            name: "Report.txt",
            path: r"C:\Users\1001\Report.txt",
        };
        let message = Message {
            serial: 7,
            state: search::COMPLETE,
            count: 2,
            entry: Some((1, entry)),
        };
        let mut out = [0u8; crate::BODY_BYTES];
        let n = search::encode(message, &mut out).unwrap();
        assert_eq!(search::decode(&out[..n]), Ok(message));
        let none = Message {
            serial: 8,
            state: search::BUILDING,
            count: 0,
            entry: None,
        };
        let m = search::encode(none, &mut out).unwrap();
        assert_eq!((m, search::decode(&out[..m])), (6, Ok(none)));
        let n = search::encode(message, &mut out).unwrap();
        assert!(search::decode(&out[..n - 1]).is_err(), "truncated");
        let mut bad = out;
        bad[6] = 2;
        assert!(search::decode(&bad[..n]).is_err(), "index beyond count");
        let mut bad = out;
        bad[4] = 9;
        assert!(search::decode(&bad[..n]).is_err(), "unknown state");
        let control = Message {
            entry: Some((
                0,
                Entry {
                    name: "a\nb",
                    ..entry
                },
            )),
            ..message
        };
        let c = search::encode(control, &mut out).unwrap();
        assert!(search::decode(&out[..c]).is_err(), "control characters");
        assert!(
            search::encode(
                Message {
                    count: 9,
                    ..message
                },
                &mut out
            )
            .is_err()
        );
        assert!(
            search::encode(
                Message {
                    entry: Some((0, Entry { name: "", ..entry })),
                    ..message
                },
                &mut out
            )
            .is_err()
        );
        assert!(
            search::encode(
                Message {
                    count: 1,
                    entry: None,
                    ..message
                },
                &mut out
            )
            .is_err()
        );
        let long = [b'x'; search::MAX_PATH + 1];
        let long = core::str::from_utf8(&long).unwrap();
        assert!(
            search::encode(
                Message {
                    entry: Some((
                        0,
                        Entry {
                            path: long,
                            ..entry
                        }
                    )),
                    ..message
                },
                &mut out
            )
            .is_err()
        );
    }
}
#[cfg(test)]
mod win32_tests {
    use super::*;
    #[test]
    fn win32_opcodes_and_capability_are_disjoint() {
        let ops = [
            SET_OWNER,
            SET_ENABLED,
            MOVE_RESIZE,
            SET_SIZE_LIMITS,
            SET_FRAMELESS,
            SET_CAPTION_BUTTONS,
            SET_RESPONDING,
            SET_ACCENT,
            SET_RECYCLE_BIN,
        ];
        for op in ops {
            assert!((0x5000..0x5100).contains(&op));
            assert!(
                ![
                    SET_TIME_LOCALE,
                    SET_TEST_HOOKS,
                    DOCUMENT_GET,
                    CLOSE_DECLINED
                ]
                .contains(&op)
            );
        }
        let caps = [
            CAP_CPU_SNAPSHOT,
            CAP_TRANSIENT_DISMISS,
            CAP_EPOCH_COMMIT,
            CAP_CLIENT_CAPTION,
            CAP_APPEARANCE,
            CAP_DESKTOP_CONTROLS,
            CAP_TABBED_CAPTION,
            CAP_MULTI_WINDOW,
            CAP_BAND_COMMIT,
            CAP_WHEEL,
            CAP_ACRYLIC,
            CAP_SEARCH_PROVIDER,
            CAP_DISPLAY_SCALE,
            crate::clipboard::CAP_CLIPBOARD,
            crate::dnd::CAP_DRAG_DROP,
            crate::text_input::CAP_TEXT_INPUT,
            crate::accessibility::CAP_ACCESSIBILITY,
        ];
        assert!(caps.iter().all(|c| c & CAP_WIN32 == 0));
        assert_eq!(EDGE_LEFT | EDGE_TOP | EDGE_RIGHT | EDGE_BOTTOM, 15);
    }
}
#[cfg(test)]
mod display_scale_tests {
    extern crate std;
    use super::*;
    #[test]
    fn offered_scales_keep_a_usable_logical_desktop() {
        let offered = |w, h| -> std::vec::Vec<u32> {
            DISPLAY_SCALES
                .into_iter()
                .filter(|p| display_scale_offered(w, h, *p))
                .collect()
        };
        assert_eq!(offered(1280, 720), [100]);
        assert_eq!(offered(1920, 1080), [100, 125, 150, 175]);
        assert_eq!(offered(2560, 1600), [100, 125, 150, 175, 200, 225, 250]);
        assert_eq!(offered(3840, 2160), DISPLAY_SCALES);
        assert!(!display_scale_offered(2560, 1600, 130));
        assert_eq!(
            (logical_extent(2560, 150), logical_extent(1600, 150)),
            (1707, 1067)
        );
        let e = Event {
            kind: SCALE_CHANGED,
            a: 150,
            b: 144,
            value: 150u64 << 32 | 150,
            rect: Rect {
                x: 0,
                y: 0,
                width: 2560,
                height: 1600,
            },
            ..Event::default()
        };
        assert_eq!(Event::decode(&e.encode()), Ok(e));
    }
}
#[cfg(test)]
mod search_history_tests {
    use super::*;
    #[test]
    fn history_entries_roundtrip_clip_and_reject_malformed_bodies() {
        let mut out = [0u8; 2 + search_history::MAX_TEXT];
        let n = search_history::encode(3, Some((2, "notepad")), &mut out).unwrap();
        assert_eq!(
            search_history::decode(&out[..n]),
            Ok((3, Some((2, "notepad"))))
        );
        let n = search_history::encode(0, None, &mut out).unwrap();
        assert_eq!((n, search_history::decode(&out[..n])), (2, Ok((0, None))));
        assert!(search_history::encode(1, None, &mut out).is_err());
        assert!(search_history::encode(2, Some((2, "x")), &mut out).is_err());
        assert!(search_history::encode(11, Some((0, "x")), &mut out).is_err());
        assert!(search_history::encode(1, Some((0, "a\nb")), &mut out).is_err());
        assert!(search_history::decode(&[1, 0]).is_err(), "empty text");
        assert!(search_history::decode(&[0, 1]).is_err());
        assert!(search_history::decode(&[2, 2, b'x']).is_err());
        assert!(search_history::decode(&[1, 0, 0xff]).is_err(), "not UTF-8");
        // Clipping: trimmed, 31 UTF-16 units, never inside a character.
        assert_eq!(search_history::clip("  report  "), Some("report"));
        assert_eq!(search_history::clip("   "), None);
        let long = "abcdefghijklmnopqrstuvwxyz0123456789";
        assert_eq!(search_history::clip(long), Some(&long[..31]));
        let emoji = "\u{1F600}".repeat(16);
        let clipped = search_history::clip(&emoji).unwrap();
        assert_eq!(clipped.encode_utf16().count(), 30);
        assert!(clipped.encode_utf16().count() < 32, "fits a 64-byte REG_SZ");
        assert!(
            Event::decode(
                &Event {
                    kind: SEARCH_HISTORY,
                    ..Event::default()
                }
                .encode()
            )
            .is_ok()
        );
        assert!(
            Event::decode(
                &Event {
                    kind: LAST_EVENT + 1,
                    ..Event::default()
                }
                .encode()
            )
            .is_err()
        );
    }
}

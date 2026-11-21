//! Allocation-free window RPC. Section memory remains producer-owned; the server
//! takes a checked private snapshot before releasing a submitted buffer.
use crate::{Connection, Error, Result};
use core::sync::atomic::{AtomicU32, Ordering};
use door_abi::{Handle, Transfer, rights};
use door_service_protocol::desktop::Controls;
use door_service_protocol::{Code, window as w};
pub use w::{Event, Rect};
/// How a Run as administrator of a program ended (`elevation_status`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elevation {
    /// It runs: a handle to wait on and query.
    Started(Handle),
    Declined,
    Failed,
}
/// The session's desktop controls as last delivered to this process
/// (CONTROLS_CHANGED, sent on HELLO and on every change). Process-wide
/// presentation (contrast themes, text size) reads them, as Windows apps read
/// SPI_GETHIGHCONTRAST, without threading them through every painter.
static DESKTOP_CONTROLS: AtomicU32 = AtomicU32::new(Controls::DEFAULT.bits());
/// The last desktop controls this process received (the default until then).
pub fn desktop_controls() -> Controls {
    Controls::from_bits(DESKTOP_CONTROLS.load(Ordering::Relaxed)).unwrap_or_default()
}
/// Record delivered (or, in host reviews, simulated) desktop controls.
pub fn observe_desktop_controls(controls: Controls) {
    DESKTOP_CONTROLS.store(controls.bits(), Ordering::Relaxed);
}
pub struct WindowClient {
    pub(crate) connection: Connection,
    pending: [Event; 3],
    next: usize,
    count: usize,
    pub(crate) capabilities: u32,
    /// Optional capabilities this client also requests (e.g. search).
    extra: u32,
    /// Physical output extent and DPI from the last HELLO reply.
    output: (u32, u32, u32),
}
impl WindowClient {
    pub fn tab_target(&mut self, source: u64) -> Result<(u64, i32, i32)> {
        if self.capabilities & w::CAP_MULTI_WINDOW == 0 {
            return Err(Error::Service(Code::Unsupported));
        }
        let r = self
            .connection
            .call(w::TAB_TARGET, &source.to_le_bytes(), &[])?;
        let b = r.data()?;
        if b.len() != 16 {
            return Err(Error::Malformed);
        }
        Ok((
            u64::from_le_bytes(b[..8].try_into().unwrap()),
            i32::from_le_bytes(b[8..12].try_into().unwrap()),
            i32::from_le_bytes(b[12..].try_into().unwrap()),
        ))
    }
    pub fn open_settings_page(&mut self, page: u32) -> Result<()> {
        if self.capabilities & w::CAP_DESKTOP_CONTROLS == 0 {
            return Err(Error::Service(Code::Unsupported));
        }
        // Any Settings route, optionally with a section (`door_settings_map::
        // navigate_arg`); the window server validates it again.
        if door_settings_map::decode_navigate(page).is_none() {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let mut bytes = [0; 8];
        bytes[..4].copy_from_slice(&3u32.to_le_bytes());
        bytes[4..].copy_from_slice(&page.to_le_bytes());
        self.connection.call(w::LAUNCH_APP, &bytes, &[])?.empty()
    }
    pub fn launch_window(&mut self, app: u32) -> Result<()> {
        if self.capabilities & w::CAP_MULTI_WINDOW == 0 {
            return Err(Error::Service(Code::Unsupported));
        }
        self.connection
            .call(w::LAUNCH_WINDOW, &app.to_le_bytes(), &[])?
            .empty()
    }
    pub fn launch_app(&mut self, app: u32) -> Result<()> {
        self.connection
            .call(w::LAUNCH_APP, &app.to_le_bytes(), &[])?
            .empty()
    }
    pub fn set_theme(&mut self, dark: bool) -> Result<()> {
        self.connection
            .call(w::SET_THEME, &u32::from(dark).to_le_bytes(), &[])?
            .empty()
    }
    pub fn set_appearance(&mut self, value: u32) -> Result<()> {
        self.connection
            .call(w::SET_APPEARANCE, &value.to_le_bytes(), &[])?
            .empty()
    }
    pub fn set_desktop_controls(
        &mut self,
        value: door_service_protocol::desktop::Controls,
    ) -> Result<()> {
        if self.capabilities & w::CAP_DESKTOP_CONTROLS == 0 {
            return Err(Error::Service(Code::Unsupported));
        }
        self.connection
            .call(w::SET_DESKTOP_CONTROLS, &value.bits().to_le_bytes(), &[])?
            .empty()
    }
    pub fn connect(registry: Handle) -> Result<Self> {
        Self::connect_with(registry, 0)
    }
    /// Connect and also request optional capabilities (`w::CAP_SEARCH_PROVIDER`);
    /// a server without them still connects with the standard set.
    pub fn connect_with(registry: Handle, extra: u32) -> Result<Self> {
        let mut result = Self {
            connection: crate::connect(registry, door_abi::platform::manifest::WINDOW)?,
            pending: [Event::default(); 3],
            next: 0,
            count: 0,
            capabilities: 0,
            extra,
            output: (0, 0, 96),
        };
        let _ = result.hello()?;
        Ok(result)
    }
    pub fn is_search_provider(&self) -> bool {
        self.capabilities & w::CAP_SEARCH_PROVIDER != 0
    }
    /// Search provider: the query text for `serial` (UTF-8 into `out`).
    pub fn search_query_get(&mut self, serial: u32, out: &mut [u8]) -> Result<usize> {
        if !self.is_search_provider() {
            return Err(Error::Service(Code::Unsupported));
        }
        let r = self
            .connection
            .call(w::SEARCH_QUERY_GET, &serial.to_le_bytes(), &[])?;
        let data = r.data()?;
        if data.len() > out.len() || data.len() > w::search::MAX_QUERY {
            return Err(Error::Malformed);
        }
        out[..data.len()].copy_from_slice(data);
        Ok(data.len())
    }
    /// Search provider: one result message (`w::search::encode`).
    pub fn search_result(&mut self, message: w::search::Message<'_>) -> Result<()> {
        if !self.is_search_provider() {
            return Err(Error::Service(Code::Unsupported));
        }
        let mut body = [0u8; door_service_protocol::BODY_BYTES];
        let n = w::search::encode(message, &mut body).map_err(Error::Service)?;
        self.connection
            .call(w::SEARCH_RESULTS, &body[..n], &[])?
            .empty()
    }
    /// Search provider: open a document in File Explorer (1, an encoded
    /// location or search), Notes (2) or Preview (10, a Door path).
    pub fn open_document(&mut self, app: u32, payload: &[u8]) -> Result<()> {
        if !self.is_search_provider() {
            return Err(Error::Service(Code::Unsupported));
        }
        if payload.is_empty() || payload.len() > w::MAX_DOCUMENT {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let mut body = [0u8; 4 + w::MAX_DOCUMENT];
        body[..4].copy_from_slice(&app.to_le_bytes());
        body[4..4 + payload.len()].copy_from_slice(payload);
        self.connection
            .call(w::OPEN_DOCUMENT, &body[..4 + payload.len()], &[])?
            .empty()
    }
    /// Desktop-controls client (File Explorer's Open and Open with): open a
    /// document in File Explorer (1, an encoded location), Notes (2) or
    /// Preview (10, a Door path). The window server accepts OPEN_DOCUMENT
    /// from CAP_DESKTOP_CONTROLS clients as well as the search provider.
    pub fn open_document_in_app(&mut self, app: u32, payload: &[u8]) -> Result<()> {
        if self.capabilities & w::CAP_DESKTOP_CONTROLS == 0 {
            return Err(Error::Service(Code::Unsupported));
        }
        if payload.is_empty() || payload.len() > w::MAX_DOCUMENT {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let mut body = [0u8; 4 + w::MAX_DOCUMENT];
        body[..4].copy_from_slice(&app.to_le_bytes());
        body[4..4 + payload.len()].copy_from_slice(payload);
        self.connection
            .call(w::OPEN_DOCUMENT, &body[..4 + payload.len()], &[])?
            .empty()
    }
    /// After DOCUMENT_OPEN: the document payload for this app (consumed).
    /// Snipping Tool: open the shell's snip overlay (`w::SNIP_BEGIN`) in
    /// `mode` (`w::snip`) after `delay` seconds (0, 3, 5 or 10).
    pub fn snip_begin(&mut self, mode: u32, delay: u32) -> Result<()> {
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&mode.to_le_bytes());
        b[4..].copy_from_slice(&delay.to_le_bytes());
        self.connection.call(w::SNIP_BEGIN, &b, &[])?.empty()
    }
    /// Magnifier: the shell's magnification (`w::MAGNIFY`).
    pub fn magnify(&mut self, percent: u32, view: u32, flags: u32) -> Result<()> {
        let mut b = [0u8; 12];
        b[..4].copy_from_slice(&percent.to_le_bytes());
        b[4..8].copy_from_slice(&view.to_le_bytes());
        b[8..].copy_from_slice(&flags.to_le_bytes());
        self.connection.call(w::MAGNIFY, &b, &[])?.empty()
    }
    /// On-Screen Keyboard: a key to the focused window (`w::INJECT_KEY`).
    pub fn inject_key(&mut self, scan: u32, flags: u32) -> Result<()> {
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&scan.to_le_bytes());
        b[4..].copy_from_slice(&flags.to_le_bytes());
        self.connection.call(w::INJECT_KEY, &b, &[])?.empty()
    }
    pub fn document_get(&mut self, out: &mut [u8]) -> Result<usize> {
        let r = self.connection.call(w::DOCUMENT_GET, &[], &[])?;
        let data = r.data()?;
        if data.is_empty() || data.len() > out.len() || data.len() > w::MAX_DOCUMENT {
            return Err(Error::Malformed);
        }
        out[..data.len()].copy_from_slice(data);
        Ok(data.len())
    }
    pub fn pending_events(&self) -> bool {
        self.next < self.count
    }
    pub fn hello(&mut self) -> Result<[u8; w::HELLO_BYTES]> {
        let mut request = [0; 8];
        request[..4].copy_from_slice(&w::VERSION.to_le_bytes());
        let mut response = None;
        let extra = self.extra;
        for features in [
            w::CAP_TRANSIENT_DISMISS
                | w::CAP_EPOCH_COMMIT
                | w::CAP_CLIENT_CAPTION
                | w::CAP_APPEARANCE
                | w::CAP_DESKTOP_CONTROLS
                | w::CAP_TABBED_CAPTION
                | w::CAP_MULTI_WINDOW
                | w::CAP_WHEEL,
            w::CAP_TRANSIENT_DISMISS
                | w::CAP_EPOCH_COMMIT
                | w::CAP_CLIENT_CAPTION
                | w::CAP_APPEARANCE
                | w::CAP_DESKTOP_CONTROLS
                | w::CAP_TABBED_CAPTION
                | w::CAP_MULTI_WINDOW,
            w::CAP_TRANSIENT_DISMISS
                | w::CAP_EPOCH_COMMIT
                | w::CAP_CLIENT_CAPTION
                | w::CAP_APPEARANCE
                | w::CAP_DESKTOP_CONTROLS
                | w::CAP_TABBED_CAPTION,
            w::CAP_TRANSIENT_DISMISS
                | w::CAP_EPOCH_COMMIT
                | w::CAP_CLIENT_CAPTION
                | w::CAP_APPEARANCE
                | w::CAP_DESKTOP_CONTROLS,
            w::CAP_TRANSIENT_DISMISS
                | w::CAP_EPOCH_COMMIT
                | w::CAP_CLIENT_CAPTION
                | w::CAP_APPEARANCE,
            w::CAP_TRANSIENT_DISMISS | w::CAP_EPOCH_COMMIT | w::CAP_CLIENT_CAPTION,
            w::CAP_TRANSIENT_DISMISS | w::CAP_EPOCH_COMMIT,
            w::CAP_TRANSIENT_DISMISS,
            0,
        ]
        .into_iter()
        .flat_map(|f| {
            if extra != 0 {
                [Some(f | extra), Some(f)]
            } else {
                [Some(f), None]
            }
        })
        .flatten()
        {
            request[4..].copy_from_slice(&features.to_le_bytes());
            let body = if features == 0 {
                &request[..4]
            } else {
                &request[..]
            };
            match self.connection.call(w::HELLO, body, &[]) {
                Err(Error::Service(Code::InvalidRequest | Code::Unsupported)) => continue,
                other => {
                    response = Some(other?);
                    break;
                }
            }
        }
        let r = response.ok_or(Error::Service(Code::Unsupported))?;
        let data = r.data()?;
        if data.len() != w::HELLO_BYTES
            || u32::from_le_bytes(data[..4].try_into().unwrap()) != w::VERSION
        {
            return Err(Error::Malformed);
        }
        let mut out = [0; w::HELLO_BYTES];
        out.copy_from_slice(data);
        self.capabilities = u32::from_le_bytes(out[20..24].try_into().unwrap());
        let word = |i: usize| u32::from_le_bytes(out[i * 4..i * 4 + 4].try_into().unwrap());
        self.output = (word(1), word(2), word(4));
        Ok(out)
    }
    /// Physical output width, height and DPI reported by HELLO.
    pub fn output(&self) -> (u32, u32, u32) {
        self.output
    }
    pub fn supports_epoch_commit(&self) -> bool {
        self.capabilities & w::CAP_EPOCH_COMMIT != 0
    }
    /// The server granted CAP_WHEEL: INPUT events with detail `INPUT_WHEEL`
    /// are delivered (requested automatically by `hello`).
    pub fn supports_wheel(&self) -> bool {
        self.capabilities & w::CAP_WHEEL != 0
    }
    /// The server granted CAP_DISPLAY_SCALE (request it with
    /// `connect_with(registry, w::CAP_DISPLAY_SCALE)`): the HELLO DPI is the
    /// fractional display DPI and SCALE_CHANGED events arrive.
    pub fn supports_display_scale(&self) -> bool {
        self.capabilities & w::CAP_DISPLAY_SCALE != 0
    }
    /// Settings > Display > Scale: apply `percent` live (0 = automatic,
    /// "Recommended") and return the resulting effective percent. The
    /// caller persists the preference in storage (`Setting::ScalePercent`).
    pub fn set_display_scale(&mut self, percent: u32) -> Result<u32> {
        let effective = self
            .connection
            .call(w::SET_DISPLAY_SCALE, &percent.to_le_bytes(), &[])?
            .scalar()?;
        u32::try_from(effective).map_err(|_| Error::Malformed)
    }
    /// The preference, effective and recommended percents and the scales
    /// this output offers.
    pub fn display_scale_info(&mut self) -> Result<DisplayScaleInfo> {
        let r = self.connection.call(w::DISPLAY_SCALE_INFO, &[], &[])?;
        DisplayScaleInfo::decode(r.data()?).ok_or(Error::Malformed)
    }
    /// Settings: show the taskbar clock in this time zone and regional
    /// format (`window::SET_TIME_LOCALE`).
    pub fn set_time_locale(&mut self, zone: u32, region: u32) -> Result<()> {
        self.connection
            .call(w::SET_TIME_LOCALE, &time_locale(zone, region), &[])?
            .empty()
    }
    /// Init at session binding: the user's persisted time zone and format.
    pub fn bind_time_locale(port: Handle, zone: u32, region: u32) -> Result<()> {
        let mut c = Connection::connect(port, door_abi::platform::manifest::WINDOW)?;
        c.call(w::SET_TIME_LOCALE, &time_locale(zone, region), &[])?
            .empty()
    }
    /// Settings > Personalization > Colors > Accent color: apply the user's
    /// accent live (the caller persists `Setting::AccentColor`). Every
    /// CAP_APPEARANCE client then receives APPEARANCE_CHANGED with the
    /// resolved colour in `a` (`desktop::Accent::from_event_word`).
    pub fn set_accent(&mut self, accent: door_service_protocol::desktop::Accent) -> Result<()> {
        self.connection
            .call(w::SET_ACCENT, &accent.bits().to_le_bytes(), &[])?
            .empty()
    }
    /// File Explorer: whether the user's Recycle Bin holds items (the
    /// desktop icon shows the full or empty bin).
    pub fn set_recycle_bin(&mut self, full: bool) -> Result<()> {
        self.connection
            .call(w::SET_RECYCLE_BIN, &u32::from(full).to_le_bytes(), &[])?
            .empty()
    }
    /// Settings: apply one `preferences` word live (the caller persists it
    /// with Storage SET_SETTING first).
    pub fn set_preference(&mut self, id: u32, value: u32) -> Result<()> {
        let mut body = [0; 8];
        body[..4].copy_from_slice(&id.to_le_bytes());
        body[4..].copy_from_slice(&value.to_le_bytes());
        self.connection.call(w::SET_PREFERENCE, &body, &[])?.empty()
    }
    /// Init at session binding: one persisted `preferences` word.
    pub fn bind_preference(port: Handle, id: u32, value: u32) -> Result<()> {
        let mut c = Connection::connect(port, door_abi::platform::manifest::WINDOW)?;
        let mut body = [0; 8];
        body[..4].copy_from_slice(&id.to_le_bytes());
        body[4..].copy_from_slice(&value.to_le_bytes());
        c.call(w::SET_PREFERENCE, &body, &[])?.empty()
    }
    /// Init at session binding: the user's persisted accent colour.
    pub fn bind_accent(port: Handle, accent: door_service_protocol::desktop::Accent) -> Result<()> {
        let mut c = Connection::connect(port, door_abi::platform::manifest::WINDOW)?;
        c.call(w::SET_ACCENT, &accent.bits().to_le_bytes(), &[])?
            .empty()
    }
    /// Init at session binding: the user's persisted preference.
    pub fn bind_display_scale(port: Handle, percent: u32) -> Result<u32> {
        let mut c = Connection::connect(port, door_abi::platform::manifest::WINDOW)?;
        let effective = c
            .call(w::SET_DISPLAY_SCALE, &percent.to_le_bytes(), &[])?
            .scalar()?;
        u32::try_from(effective).map_err(|_| Error::Malformed)
    }
    /// The server granted CAP_WIN32 (request it with
    /// `connect_with(registry, w::CAP_WIN32)`): window relations below.
    pub fn supports_win32(&self) -> bool {
        self.capabilities & w::CAP_WIN32 != 0
    }
    fn win32(&self) -> Result<()> {
        if self.supports_win32() {
            Ok(())
        } else {
            Err(Error::Service(Code::Unsupported))
        }
    }
    /// SET_OWNER: `owner` 0 clears the relation.
    pub fn set_owner(&mut self, id: u64, owner: u64) -> Result<()> {
        self.win32()?;
        self.pair(w::SET_OWNER, id, owner)
    }
    /// SET_ENABLED: a disabled window takes no input (modal owner).
    pub fn set_enabled(&mut self, id: u64, enabled: bool) -> Result<()> {
        self.win32()?;
        let mut b = [0; 12];
        b[..8].copy_from_slice(&id.to_le_bytes());
        b[8..].copy_from_slice(&u32::from(enabled).to_le_bytes());
        self.connection.call(w::SET_ENABLED, &b, &[])?.empty()
    }
    /// REQUEST_ELEVATION (File Explorer): Run as administrator on `path`.
    /// A program sends its bytes in `image` (a section of `image_bytes`) and
    /// the working-folder handle; a batch file sends neither. The window
    /// server verifies, asks and runs; this only says the prompt opened.
    /// A Windows program's ShellExecute of a document no Windows association
    /// opens (`w::OPEN_WITH`): File Explorer opens it with its default app
    /// here or asks which app to use (`mode` `w::open_with`).
    pub fn open_with(&mut self, mode: u8, path: &str) -> Result<()> {
        if path.is_empty() || 1 + path.len() > w::MAX_DOCUMENT {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let mut b = [0; w::MAX_DOCUMENT];
        b[0] = mode;
        b[1..1 + path.len()].copy_from_slice(path.as_bytes());
        self.connection
            .call(w::OPEN_WITH, &b[..1 + path.len()], &[])?
            .empty()
    }
    /// Settings: a firewall change (`firewall::CHANGE` body) for User
    /// Account Control; `WouldBlock` while another prompt is open.
    pub fn firewall_change(&mut self, body: &[u8]) -> Result<()> {
        self.connection.call(w::FIREWALL_CHANGE, body, &[])?.empty()
    }
    pub fn request_elevation(
        &mut self,
        path: &str,
        flags: u32,
        image: Option<(Handle, u32)>,
        directory: Handle,
    ) -> Result<()> {
        let mut b = [0; 10 + w::elevation::MAX_PATH];
        if path.is_empty() || path.len() > w::elevation::MAX_PATH {
            return Err(Error::Service(Code::InvalidRequest));
        }
        b[..4].copy_from_slice(&flags.to_le_bytes());
        b[4..8].copy_from_slice(&image.map_or(0, |(_, n)| n).to_le_bytes());
        b[8..10].copy_from_slice(&(path.len() as u16).to_le_bytes());
        b[10..10 + path.len()].copy_from_slice(path.as_bytes());
        let body = &b[..10 + path.len()];
        match image {
            None => self
                .connection
                .call(w::REQUEST_ELEVATION, body, &[])?
                .empty(),
            Some((section, _)) => self
                .connection
                .call(
                    w::REQUEST_ELEVATION,
                    body,
                    &[
                        Transfer {
                            handle: directory,
                            rights: rights::READ
                                | rights::QUERY
                                | rights::DUPLICATE
                                | rights::TRANSFER,
                            mode: 0,
                        },
                        // No WRITE: the server snapshots (READ|MAP) what it verifies.
                        Transfer {
                            handle: section,
                            rights: rights::READ | rights::MAP | rights::QUERY,
                            mode: 0,
                        },
                    ],
                )?
                .empty(),
        }
    }
    /// ELEVATION_STATUS: how the caller's last Run as administrator of a
    /// program ended (`Ok(None)` while the prompt or the launch is pending).
    pub fn elevation_status(&mut self) -> Result<Option<Elevation>> {
        match self.connection.call(w::ELEVATION_STATUS, &[], &[]) {
            Err(Error::Service(Code::WouldBlock)) => Ok(None),
            Err(e) => Err(e),
            Ok(reply) => {
                if reply.handle_count() == 1 {
                    return reply.one_handle().map(|h| Some(Elevation::Started(h)));
                }
                Ok(Some(match reply.scalar()? {
                    w::elevation::DECLINED => Elevation::Declined,
                    _ => Elevation::Failed,
                }))
            }
        }
    }
    /// NOTIFY_ICON (Shell_NotifyIcon): add, change or remove a notification
    /// icon of `window`; `icon` is a section of `w::notify::ICON_BYTES`.
    #[allow(clippy::too_many_arguments)]
    pub fn notify_icon(
        &mut self,
        op: u8,
        version: u8,
        flags: u16,
        window: u64,
        id: u32,
        callback: u32,
        tip: &str,
        icon: Option<Handle>,
    ) -> Result<()> {
        self.win32()?;
        let tip = door_service_protocol::shell::clip(tip, w::notify::MAX_TIP);
        let mut b = [0; 21 + w::notify::MAX_TIP];
        b[0] = op;
        b[1] = version;
        b[2..4].copy_from_slice(&flags.to_le_bytes());
        b[4..12].copy_from_slice(&window.to_le_bytes());
        b[12..16].copy_from_slice(&id.to_le_bytes());
        b[16..20].copy_from_slice(&callback.to_le_bytes());
        b[20] = tip.len() as u8;
        b[21..21 + tip.len()].copy_from_slice(tip.as_bytes());
        let transfers: &[Transfer] = match icon {
            Some(section) => &[Transfer {
                handle: section,
                rights: rights::READ | rights::MAP | rights::QUERY,
                mode: 0,
            }],
            None => &[],
        };
        self.connection
            .call(w::NOTIFY_ICON, &b[..21 + tip.len()], transfers)?
            .empty()
    }
    /// NOTIFY_ICON's RECT: where notification icon `id` of `window` is on
    /// screen (left, top, right, bottom).
    pub fn notify_icon_rect(&mut self, window: u64, id: u32) -> Result<[i32; 4]> {
        self.win32()?;
        let mut b = [0; 21];
        b[0] = w::notify::RECT;
        b[4..12].copy_from_slice(&window.to_le_bytes());
        b[12..16].copy_from_slice(&id.to_le_bytes());
        let reply = self.connection.call(w::NOTIFY_ICON, &b, &[])?;
        let d = reply.data()?;
        if d.len() != 16 {
            return Err(Error::Malformed);
        }
        let at = |i: usize| i32::from_le_bytes([d[i], d[i + 1], d[i + 2], d[i + 3]]);
        let (x, y) = (at(0), at(4));
        Ok([x, y, x.saturating_add(at(8)), y.saturating_add(at(12))])
    }
    /// SET_FULLSCREEN: cover the whole output (F11), or restore.
    pub fn set_fullscreen(&mut self, id: u64, on: bool) -> Result<()> {
        let mut b = [0; 12];
        b[..8].copy_from_slice(&id.to_le_bytes());
        b[8..].copy_from_slice(&u32::from(on).to_le_bytes());
        self.connection.call(w::SET_FULLSCREEN, &b, &[])?.empty()
    }
    /// SET_FRAMELESS: no trusted caption buttons, border or title bar.
    pub fn set_frameless(&mut self, id: u64, frameless: bool) -> Result<()> {
        self.win32()?;
        let mut b = [0; 12];
        b[..8].copy_from_slice(&id.to_le_bytes());
        b[8..].copy_from_slice(&u32::from(frameless).to_le_bytes());
        self.connection.call(w::SET_FRAMELESS, &b, &[])?.empty()
    }
    /// SET_CAPTION_BUTTONS: which of Minimize (bit 0) and Maximize (bit 1)
    /// the caption offers.
    pub fn set_caption_buttons(&mut self, id: u64, buttons: u32) -> Result<()> {
        self.win32()?;
        let mut b = [0; 12];
        b[..8].copy_from_slice(&id.to_le_bytes());
        b[8..].copy_from_slice(&buttons.to_le_bytes());
        self.connection
            .call(w::SET_CAPTION_BUTTONS, &b, &[])?
            .empty()
    }
    /// SET_RESPONDING: the thread owning `id` stopped retrieving messages
    /// for five seconds with messages pending (`false`), or recovered.
    pub fn set_responding(&mut self, id: u64, responding: bool) -> Result<()> {
        self.win32()?;
        let mut b = [0; 12];
        b[..8].copy_from_slice(&id.to_le_bytes());
        b[8..].copy_from_slice(&u32::from(responding).to_le_bytes());
        self.connection.call(w::SET_RESPONDING, &b, &[])?.empty()
    }
    /// MOVE_RESIZE: continue the held press `serial` (an INPUT event
    /// timestamp) as a trusted move (`edges` 0) or border resize.
    pub fn move_resize(&mut self, id: u64, edges: u32, serial: u64) -> Result<()> {
        self.win32()?;
        let mut b = [0; 24];
        b[..8].copy_from_slice(&id.to_le_bytes());
        b[8..12].copy_from_slice(&edges.to_le_bytes());
        b[16..].copy_from_slice(&serial.to_le_bytes());
        self.connection.call(w::MOVE_RESIZE, &b, &[])?.empty()
    }
    /// SET_SIZE_LIMITS: minimum and maximum outer size (0 = unbounded max).
    pub fn set_size_limits(&mut self, id: u64, min: (u32, u32), max: (u32, u32)) -> Result<()> {
        self.win32()?;
        let mut b = [0; 24];
        b[..8].copy_from_slice(&id.to_le_bytes());
        for (i, v) in [min.0, min.1, max.0, max.1].into_iter().enumerate() {
            b[8 + 4 * i..12 + 4 * i].copy_from_slice(&v.to_le_bytes());
        }
        self.connection.call(w::SET_SIZE_LIMITS, &b, &[])?.empty()
    }
    pub fn supports_acrylic(&self) -> bool {
        self.capabilities & w::CAP_ACRYLIC != 0
    }
    /// Select the material behind this window's translucent pixels:
    /// `w::MATERIAL_DEFAULT` (wallpaper), `w::MATERIAL_ACRYLIC` (live blur
    /// of the windows behind) or `w::MATERIAL_MICA` (Windows 11 Mica: paint
    /// the page background transparent and cards translucent over it). The
    /// server renders a solid fallback when the user reduced transparency
    /// and, for Mica, while the window is inactive.
    pub fn set_material(&mut self, id: u64, material: u32) -> Result<()> {
        if !self.supports_acrylic() {
            return Err(Error::Service(Code::Unsupported));
        }
        let mut b = [0; 12];
        b[..8].copy_from_slice(&id.to_le_bytes());
        b[8..].copy_from_slice(&material.to_le_bytes());
        self.connection.call(w::SET_MATERIAL, &b, &[])?.empty()
    }
    pub fn set_title(&mut self, id: u64, title: &str) -> Result<()> {
        if title.len() > 128 {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let mut b = [0; 136];
        b[..8].copy_from_slice(&id.to_le_bytes());
        b[8..8 + title.len()].copy_from_slice(title.as_bytes());
        self.connection
            .call(w::SET_TITLE, &b[..8 + title.len()], &[])?
            .empty()
    }
    pub fn request_close(&mut self, id: u64) -> Result<()> {
        self.connection
            .call(w::REQUEST_CLOSE, &id.to_le_bytes(), &[])?
            .empty()
    }
    pub fn set_cursor(&mut self, id: u64, shape: u32) -> Result<()> {
        let mut b = [0; 12];
        b[..8].copy_from_slice(&id.to_le_bytes());
        b[8..].copy_from_slice(&shape.to_le_bytes());
        self.connection.call(w::SET_CURSOR, &b, &[])?.empty()
    }
    pub fn cancel_close(&mut self, id: u64) -> Result<()> {
        self.connection
            .call(w::CANCEL_CLOSE, &id.to_le_bytes(), &[])?
            .empty()
    }
    /// The user cancelled this window's close prompt (cancels a shutdown).
    pub fn close_declined(&mut self, id: u64) -> Result<()> {
        self.connection
            .call(w::CLOSE_DECLINED, &id.to_le_bytes(), &[])?
            .empty()
    }
    pub fn capture(&mut self, id: u64) -> Result<()> {
        self.connection
            .call(w::CAPTURE, &id.to_le_bytes(), &[])?
            .empty()
    }
    pub fn release_capture(&mut self, id: u64) -> Result<()> {
        self.connection
            .call(w::RELEASE_CAPTURE, &id.to_le_bytes(), &[])?
            .empty()
    }
    pub fn request_frame(&mut self, id: u64) -> Result<()> {
        self.connection
            .call(w::FRAME, &id.to_le_bytes(), &[])?
            .empty()
    }
    /// Initialization only: the service independently requires the INIT identity.
    pub fn bind(port: Handle, session: u64, epoch: u64, uid: u32) -> Result<()> {
        let mut c = Connection::connect(port, door_abi::platform::manifest::WINDOW)?;
        let mut b = [0; 24];
        b[..8].copy_from_slice(&session.to_le_bytes());
        b[8..16].copy_from_slice(&epoch.to_le_bytes());
        b[16..20].copy_from_slice(&uid.to_le_bytes());
        c.call(w::BIND_SESSION, &b, &[])?.empty()
    }
    /// Init only, adversarial test image only: enable the window server's
    /// test hooks (`window::SET_TEST_HOOKS`).
    pub fn enable_test_hooks(port: Handle) -> Result<()> {
        let mut c = Connection::connect(port, door_abi::platform::manifest::WINDOW)?;
        c.call(w::SET_TEST_HOOKS, &[], &[])?.empty()
    }
    pub fn bind_theme(port: Handle, session: u64, epoch: u64, uid: u32, dark: bool) -> Result<()> {
        let mut c = Connection::connect(port, door_abi::platform::manifest::WINDOW)?;
        let mut b = [0; 32];
        b[..8].copy_from_slice(&session.to_le_bytes());
        b[8..16].copy_from_slice(&epoch.to_le_bytes());
        b[16..20].copy_from_slice(&uid.to_le_bytes());
        b[24..28].copy_from_slice(&u32::from(dark).to_le_bytes());
        c.call(w::BIND_SESSION, &b, &[])?.empty()
    }
    pub fn bind_appearance(
        port: Handle,
        session: u64,
        epoch: u64,
        uid: u32,
        value: u32,
    ) -> Result<()> {
        let mut c = Connection::connect(port, door_abi::platform::manifest::WINDOW)?;
        let mut b = [0; 40];
        b[..8].copy_from_slice(&session.to_le_bytes());
        b[8..16].copy_from_slice(&epoch.to_le_bytes());
        b[16..20].copy_from_slice(&uid.to_le_bytes());
        b[24..28].copy_from_slice(&(value & 1).to_le_bytes());
        b[32..36].copy_from_slice(&(value >> 1).to_le_bytes());
        c.call(w::BIND_SESSION, &b, &[])?.empty()
    }
    /// Init only: deliver the session user's persisted shell layout.
    pub fn bind_shell_layout(
        port: Handle,
        layout: door_service_protocol::desktop::ShellLayout,
    ) -> Result<()> {
        let mut c = Connection::connect(port, door_abi::platform::manifest::WINDOW)?;
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&layout.flags().to_le_bytes());
        b[4..].copy_from_slice(&layout.order().to_le_bytes());
        c.call(w::SET_SHELL_LAYOUT, &b, &[])?.empty()
    }
    /// Init only: apply persisted desktop controls (clients get
    /// CONTROLS_CHANGED), without rebinding the session.
    pub fn apply_controls(
        port: Handle,
        controls: door_service_protocol::desktop::Controls,
    ) -> Result<()> {
        let mut c = Connection::connect(port, door_abi::platform::manifest::WINDOW)?;
        c.call(w::SET_DESKTOP_CONTROLS, &controls.bits().to_le_bytes(), &[])?
            .empty()
    }
    pub fn bind_controls(
        port: Handle,
        session: u64,
        epoch: u64,
        uid: u32,
        appearance: u32,
        controls: door_service_protocol::desktop::Controls,
    ) -> Result<()> {
        Self::bind_appearance(port, session, epoch, uid, appearance)?;
        let mut c = Connection::connect(port, door_abi::platform::manifest::WINDOW)?;
        c.call(w::SET_DESKTOP_CONTROLS, &controls.bits().to_le_bytes(), &[])?
            .empty()
    }
    pub fn create(&mut self, rect: Rect, title: &str) -> Result<u64> {
        self.create_role(0, 0, rect, title)
    }
    pub fn create_client_caption(
        &mut self,
        rect: Rect,
        title: &str,
        max_width: u32,
        side_margin: u32,
    ) -> Result<u64> {
        if self.capabilities & w::CAP_CLIENT_CAPTION == 0 {
            return Err(Error::Service(Code::Unsupported));
        }
        self.create_role(
            w::CLIENT_CAPTION_ROLE,
            u64::from(max_width) | (u64::from(side_margin) << 32),
            rect,
            title,
        )
    }
    pub fn create_tabbed_caption(&mut self, rect: Rect, title: &str, left: u32) -> Result<u64> {
        if self.capabilities & w::CAP_TABBED_CAPTION == 0 {
            return Err(Error::Service(Code::Unsupported));
        }
        self.create_tabbed_caption_with_height(rect, title, left, 32)
    }
    pub fn create_tabbed_caption_with_height(
        &mut self,
        rect: Rect,
        title: &str,
        left: u32,
        height: u32,
    ) -> Result<u64> {
        if self.capabilities & w::CAP_TABBED_CAPTION == 0 || !(32..=72).contains(&height) {
            return Err(Error::Service(Code::Unsupported));
        }
        self.create_role(
            w::TABBED_CAPTION_ROLE,
            2048 | (u64::from(height) << 16) | (u64::from(left) << 32),
            rect,
            title,
        )
    }
    pub fn set_tabbed_caption(&mut self, id: u64, left: u32, width: u32) -> Result<()> {
        let mut b = [0; 16];
        b[..8].copy_from_slice(&id.to_le_bytes());
        b[8..12].copy_from_slice(&left.to_le_bytes());
        b[12..].copy_from_slice(&width.to_le_bytes());
        self.connection
            .call(w::SET_TABBED_CAPTION, &b, &[])?
            .empty()
    }
    pub fn create_popup(&mut self, parent: u64, rect: Rect, title: &str) -> Result<u64> {
        self.create_role(3, parent, rect, title)
    }
    fn create_role(&mut self, role: u32, parent: u64, rect: Rect, title: &str) -> Result<u64> {
        if title.len() > 128 {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let mut b = [0; 160];
        b[..4].copy_from_slice(&role.to_le_bytes());
        rect.encode(&mut b[4..20]);
        b[20..24].copy_from_slice(&(title.len() as u32).to_le_bytes());
        b[24..32].copy_from_slice(&parent.to_le_bytes());
        b[32..32 + title.len()].copy_from_slice(title.as_bytes());
        self.connection
            .call(w::CREATE, &b[..32 + title.len()], &[])?
            .scalar()
    }
    pub fn register(
        &mut self,
        section: Handle,
        width: u32,
        height: u32,
        stride: u64,
        bytes: u64,
    ) -> Result<u64> {
        let mut b = [0; 24];
        b[..4].copy_from_slice(&width.to_le_bytes());
        b[4..8].copy_from_slice(&height.to_le_bytes());
        b[8..16].copy_from_slice(&stride.to_le_bytes());
        b[16..24].copy_from_slice(&bytes.to_le_bytes());
        self.connection
            .call(
                w::REGISTER,
                &b,
                &[Transfer {
                    handle: section,
                    rights: rights::QUERY
                        | rights::DUPLICATE
                        | rights::READ
                        | rights::MAP
                        | rights::TRANSFER,
                    mode: 0,
                }],
            )?
            .scalar()
    }
    fn pair(&mut self, op: u32, a: u64, b: u64) -> Result<()> {
        let mut body = [0; 16];
        body[..8].copy_from_slice(&a.to_le_bytes());
        body[8..].copy_from_slice(&b.to_le_bytes());
        self.connection.call(op, &body, &[])?.empty()
    }
    pub fn acknowledge(&mut self, id: u64, epoch: u64) -> Result<()> {
        self.pair(w::ACK, id, epoch)
    }
    pub fn attach(&mut self, id: u64, buffer: u64, epoch: u64) -> Result<()> {
        let mut b = [0; 24];
        b[..8].copy_from_slice(&id.to_le_bytes());
        b[8..16].copy_from_slice(&buffer.to_le_bytes());
        b[16..].copy_from_slice(&epoch.to_le_bytes());
        self.connection.call(w::ATTACH, &b, &[])?.empty()
    }
    pub fn commit(&mut self, id: u64) -> Result<()> {
        self.connection
            .call(w::COMMIT, &id.to_le_bytes(), &[])?
            .empty()
    }
    pub fn unregister(&mut self, id: u64) -> Result<()> {
        self.connection
            .call(w::UNREGISTER, &id.to_le_bytes(), &[])?
            .empty()
    }
    /// A concurrent configure must be distinguished from an unattached commit.
    pub fn commit_epoch(&mut self, id: u64, epoch: u64) -> Result<()> {
        if !self.supports_epoch_commit() {
            return Err(Error::Service(Code::Unsupported));
        }
        self.pair(w::COMMIT, id, epoch)
    }
    pub fn supports_band_commit(&self) -> bool {
        self.capabilities & w::CAP_BAND_COMMIT != 0 && self.supports_epoch_commit()
    }
    /// Epoch commit naming the only rows (`top..top + rows`) that changed
    /// since the previous commit of this same buffer. The server falls back
    /// to a full capture when it holds no snapshot of this buffer.
    pub fn commit_band(&mut self, id: u64, epoch: u64, top: u32, rows: u32) -> Result<()> {
        if !self.supports_band_commit() {
            return Err(Error::Service(Code::Unsupported));
        }
        let mut body = [0; 24];
        body[..8].copy_from_slice(&id.to_le_bytes());
        body[8..16].copy_from_slice(&epoch.to_le_bytes());
        body[16..20].copy_from_slice(&top.to_le_bytes());
        body[20..].copy_from_slice(&rows.to_le_bytes());
        self.connection.call(w::COMMIT, &body, &[])?.empty()
    }
    pub fn destroy(&mut self, id: u64) -> Result<()> {
        self.connection
            .call(w::DESTROY, &id.to_le_bytes(), &[])?
            .empty()
    }
    /// Raise an owned visible window without restoring/maximizing it.
    pub fn activate(&mut self, id: u64) -> Result<()> {
        if self.capabilities & w::CAP_MULTI_WINDOW == 0 {
            return Err(Error::Service(Code::Unsupported));
        }
        self.pair(w::STATE, id, 3)
    }
    pub fn state(&mut self, id: u64, state: u64) -> Result<()> {
        self.pair(w::STATE, id, state)
    }
    pub fn configure(&mut self, id: u64, rect: Rect) -> Result<u64> {
        let mut b = [0; 24];
        b[..8].copy_from_slice(&id.to_le_bytes());
        rect.encode(&mut b[8..]);
        self.connection.call(w::CONFIGURE, &b, &[])?.scalar()
    }
    pub fn poll(&mut self) -> Result<Option<Event>> {
        if self.next < self.count {
            let e = self.pending[self.next];
            self.next += 1;
            return Ok(Some(e));
        }
        match self.connection.call(w::POLL_BATCH, &[], &[]) {
            Ok(r) => {
                let data = r.data()?;
                if data.is_empty()
                    || data.len() > 3 * w::EVENT_BYTES
                    || !data.len().is_multiple_of(w::EVENT_BYTES)
                {
                    return Err(Error::Malformed);
                }
                self.count = data.len() / w::EVENT_BYTES;
                self.next = 1;
                let capabilities = self.capabilities;
                for (event, b) in self
                    .pending
                    .iter_mut()
                    .zip(data.chunks_exact(w::EVENT_BYTES))
                {
                    // Clipboard, drag/drop, text-input and accessibility
                    // events reach only clients that negotiated them.
                    *event = Event::decode(b)
                        .or_else(|e| crate::clipboard::decode_event(b, capabilities).ok_or(e))
                        .or_else(|e| crate::text_input::decode_event(b, capabilities).ok_or(e))
                        .or_else(|e| crate::accessibility::decode_event(b, capabilities).ok_or(e))
                        .map_err(Error::Service)?;
                    if event.kind == w::CONTROLS_CHANGED
                        && let Some(controls) = Controls::from_bits(event.a)
                    {
                        observe_desktop_controls(controls);
                    }
                }
                Ok(Some(self.pending[0]))
            }
            Err(Error::Service(Code::WouldBlock)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
/// `DISPLAY_SCALE_INFO` reply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayScaleInfo {
    /// Stored preference: 0 = automatic, else a percent.
    pub preference: u32,
    /// Scale in effect now.
    pub effective: u32,
    /// What automatic resolves to (host hint, else the integer heuristic):
    /// Settings labels it "(Recommended)".
    pub recommended: u32,
    /// Bit i set when `w::DISPLAY_SCALES[i]` is offered for this output.
    pub offered: u32,
}
impl DisplayScaleInfo {
    pub const BYTES: usize = 16;
    pub fn encode(self) -> [u8; Self::BYTES] {
        let mut b = [0; Self::BYTES];
        for (i, v) in [
            self.preference,
            self.effective,
            self.recommended,
            self.offered,
        ]
        .into_iter()
        .enumerate()
        {
            b[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != Self::BYTES {
            return None;
        }
        let word = |i: usize| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap());
        let info = Self {
            preference: word(0),
            effective: word(1),
            recommended: word(2),
            offered: word(3),
        };
        let known = |p: u32| w::DISPLAY_SCALES.contains(&p);
        (info.offered >> w::DISPLAY_SCALES.len() == 0
            && (info.preference == 0 || known(info.preference))
            && known(info.effective)
            && known(info.recommended))
        .then_some(info)
    }
    /// Offered percents, ascending.
    pub fn offered_percents(self) -> impl Iterator<Item = u32> {
        w::DISPLAY_SCALES
            .into_iter()
            .enumerate()
            .filter(move |(i, _)| self.offered & (1 << i) != 0)
            .map(|(_, p)| p)
    }
}

fn time_locale(zone: u32, region: u32) -> [u8; 8] {
    let mut b = [0; 8];
    b[..4].copy_from_slice(&zone.to_le_bytes());
    b[4..].copy_from_slice(&region.to_le_bytes());
    b
}
/// door.shell (`door_service_protocol::shell`) on the window connection: the
/// desktop shell role (granted to the session's shell host only) and the
/// SET_TASKBAR_STATE call any window owner may make for its own windows.
impl WindowClient {
    /// Request the desktop shell role; the reply carries the window-list
    /// generation. `WouldBlock` until Init has reported this process.
    pub fn shell_hello(&mut self) -> Result<door_service_protocol::shell::Hello> {
        use door_service_protocol::shell as s;
        let mut body = [0; 8];
        body[..4].copy_from_slice(&s::VERSION.to_le_bytes());
        let reply = self.connection.call(s::HELLO, &body, &[])?;
        s::Hello::decode(reply.data()?).map_err(|_| Error::Malformed)
    }
    /// The first authorized window with an id greater than `after`; the
    /// record bytes are copied to `out` (decode with `WindowPage::decode`).
    pub fn shell_windows(
        &mut self,
        after: u64,
        out: &mut [u8; door_service_protocol::shell::WindowPage::MAX_BYTES],
    ) -> Result<usize> {
        use door_service_protocol::shell as s;
        let reply = self
            .connection
            .call(s::WINDOWS, &after.to_le_bytes(), &[])?;
        let data = reply.data()?;
        s::WindowPage::decode(data).map_err(|_| Error::Malformed)?;
        out[..data.len()].copy_from_slice(data);
        Ok(data.len())
    }
    pub fn shell_poll(&mut self) -> Result<Option<door_service_protocol::shell::Event>> {
        use door_service_protocol::shell as s;
        let reply = self.connection.call(s::POLL, &[], &[])?;
        let data = reply.data()?;
        if data.is_empty() {
            return Ok(None);
        }
        s::Event::decode(data)
            .map(Some)
            .map_err(|_| Error::Malformed)
    }
    pub fn shell_publish(&mut self, entry: &door_service_protocol::shell::Entry<'_>) -> Result<()> {
        use door_service_protocol::shell as s;
        let mut body = [0; door_service_protocol::BODY_BYTES];
        let len = entry.encode(&mut body).map_err(Error::Service)?;
        self.connection.call(s::PUBLISH, &body[..len], &[])?.empty()
    }
    pub fn shell_notify_view(
        &mut self,
        view: &door_service_protocol::shell::NoticeView<'_>,
    ) -> Result<()> {
        use door_service_protocol::shell as s;
        let mut body = [0; door_service_protocol::BODY_BYTES];
        let len = view.encode(&mut body).map_err(Error::Service)?;
        self.connection
            .call(s::NOTIFY_VIEW, &body[..len], &[])?
            .empty()
    }
    pub fn shell_action(&mut self, window: u64, action: u32) -> Result<()> {
        use door_service_protocol::shell as s;
        self.connection
            .call(s::ACTION, &s::encode_action(window, action), &[])?
            .empty()
    }
    /// Taskbar attention (FlashWindowEx) and progress (ITaskbarList3) for
    /// one of this client's own top-level windows.
    pub fn set_taskbar_state(
        &mut self,
        state: door_service_protocol::shell::TaskbarState,
    ) -> Result<()> {
        use door_service_protocol::shell as s;
        self.connection
            .call(s::SET_TASKBAR_STATE, &state.encode(), &[])?
            .empty()
    }
}
/// Start search history (P14): the search provider's side.
impl WindowClient {
    /// Search provider: the query committed as `id` (SEARCH_HISTORY with
    /// detail `HISTORY_COMMITTED`), clipped to `search_history::MAX_UNITS`.
    pub fn search_history_get(&mut self, id: u32, out: &mut [u8]) -> Result<usize> {
        if !self.is_search_provider() {
            return Err(Error::Service(Code::Unsupported));
        }
        let r = self
            .connection
            .call(w::SEARCH_HISTORY_GET, &id.to_le_bytes(), &[])?;
        let data = r.data()?;
        if data.len() > out.len() || data.len() > w::search_history::MAX_TEXT {
            return Err(Error::Malformed);
        }
        out[..data.len()].copy_from_slice(data);
        Ok(data.len())
    }
    /// Search provider: send the user's recent searches, newest first (an
    /// empty list clears Start's).
    pub fn search_history_set(&mut self, searches: &[&str]) -> Result<()> {
        if !self.is_search_provider() {
            return Err(Error::Service(Code::Unsupported));
        }
        let count = searches.len().min(w::search_history::MAX_ENTRIES) as u8;
        let mut body = [0u8; 2 + w::search_history::MAX_TEXT];
        if count == 0 {
            let n = w::search_history::encode(0, None, &mut body).map_err(Error::Service)?;
            return self
                .connection
                .call(w::SEARCH_HISTORY_SET, &body[..n], &[])?
                .empty();
        }
        for (i, text) in searches.iter().take(usize::from(count)).enumerate() {
            let n = w::search_history::encode(count, Some((i as u8, text)), &mut body)
                .map_err(Error::Service)?;
            self.connection
                .call(w::SEARCH_HISTORY_SET, &body[..n], &[])?
                .empty()?;
        }
        Ok(())
    }
}
/// The window server's own setting writes go through Init (`PERSIST_SETTING`).
impl crate::Init {
    /// Window server: persist `setting` = `value` for the session user.
    /// Completion arrives through `poll_desktop_command`.
    pub fn begin_persist_setting(&mut self, setting: u32, value: u32, now: u64) -> Result<()> {
        let mut body = [0; 8];
        body[..4].copy_from_slice(&setting.to_le_bytes());
        body[4..].copy_from_slice(&value.to_le_bytes());
        self.connection.begin(w::PERSIST_SETTING, &body, now)
    }
}
#[cfg(test)]
mod display_scale_tests {
    extern crate std;
    use super::*;
    #[test]
    fn scale_info_roundtrips_and_rejects_unknown_percents() {
        let info = DisplayScaleInfo {
            preference: 0,
            effective: 150,
            recommended: 150,
            offered: 0b0111_1111,
        };
        assert_eq!(DisplayScaleInfo::decode(&info.encode()), Some(info));
        assert_eq!(
            info.offered_percents().collect::<std::vec::Vec<_>>(),
            [100, 125, 150, 175, 200, 225, 250]
        );
        for bad in [
            DisplayScaleInfo {
                effective: 130,
                ..info
            },
            DisplayScaleInfo {
                preference: 99,
                ..info
            },
            DisplayScaleInfo {
                offered: 1 << 8,
                ..info
            },
        ] {
            assert_eq!(DisplayScaleInfo::decode(&bad.encode()), None);
        }
        assert_eq!(DisplayScaleInfo::decode(&[0; 15]), None);
    }
}

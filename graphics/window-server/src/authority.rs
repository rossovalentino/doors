//! Authenticated window authority and captured surface lifetime. Never maps a
//! producer section into this process or retains a reference to mutable pixels.
use super::*;
use door_abi::{HandleKind, Transfer, rights};
use door_system_services::server;
fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn rect(b: &[u8]) -> Result<g::Rect, Code> {
    let r = w::Rect::decode(b)?;
    g::Rect::new(r.x, r.y, r.width, r.height).map_err(error)
}
/// A refused commit of non-premultiplied pixels names its first offending
/// pixel (row `top` onward, `stride` bytes per row), so the client's drawing
/// fault can be found from the serial log alone.
fn report_invalid_pixels(window: g::WindowId, top: u32, stride: usize, bytes: &[u8]) {
    if let Some(i) = bytes
        .chunks_exact(4)
        .position(|p| p[..3].iter().any(|c| *c > p[3]))
        && stride != 0
    {
        let at = i * 4;
        let p = &bytes[at..at + 4];
        let _ = door_sys::write(
            alloc::format!(
                "DOOR_P12 COMMIT_INVALID_PIXELS window={} x={} y={} bgra={},{},{},{}
",
                window.0,
                (at % stride) / 4,
                top as usize + at / stride,
                p[0],
                p[1],
                p[2],
                p[3]
            )
            .as_bytes(),
        );
    }
}
pub(super) fn error(e: g::Error) -> Code {
    match e {
        g::Error::WrongOwner => Code::Denied,
        g::Error::UnknownId => Code::InvalidHandle,
        g::Error::Quota => Code::Resources,
        g::Error::QueueFull | g::Error::Busy => Code::WouldBlock,
        g::Error::StaleEpoch | g::Error::NotAcknowledged => Code::StaleInstance,
        g::Error::UnsupportedRole => Code::Unsupported,
        g::Error::Closing => Code::Closing,
        _ => Code::InvalidRequest,
    }
}
impl Desktop {
    /// SET_TIME_LOCALE: 8 bytes, known zone and region ids; redraws the clock.
    /// SET_ACCENT: the user's accent setting (Init at binding, Settings live).
    fn accent_request(&mut self, b: &[u8], t: &[Transfer]) -> Result<Response, Code> {
        if b.len() != 4 || !t.is_empty() {
            return Err(Code::InvalidRequest);
        }
        self.accent = door_service_protocol::desktop::Accent::from_bits(u32_at(b, 0))
            .ok_or(Code::InvalidRequest)?;
        self.apply_accent(true);
        Ok(Response::empty())
    }
    /// SET_PREFERENCE: one `preferences` word (Init at binding, Settings
    /// live). The server applies the words it consumes at once.
    fn preference_request(&mut self, b: &[u8], t: &[Transfer]) -> Result<Response, Code> {
        use door_service_protocol::preferences as p;
        if b.len() != 8 || !t.is_empty() {
            return Err(Code::InvalidRequest);
        }
        let (id, value) = (u32_at(b, 0), u32_at(b, 4));
        let changed = self
            .shell
            .prefs
            .set(id, value)
            .ok_or(Code::InvalidRequest)?;
        if !changed {
            return Ok(Response::empty());
        }
        match id {
            p::ACCENT_SURFACES => {
                self.apply_title_accent();
                self.engine.invalidate();
            }
            p::THEME_SCHEDULE => self.shell.theme_schedule.reset(),
            p::REGIONAL_OVERRIDES => {
                self.tray.overrides = self.shell.prefs.regional_overrides();
                self.bar_dirty = true;
                self.engine.invalidate();
            }
            p::CLIPBOARD if !self.shell.prefs.clipboard_history() => {
                // Turning history off clears it, as Windows does.
                self.shell.clipboard.clear_history();
                self.shell.clipboard_panel.open = false;
                self.engine.invalidate();
            }
            p::SEARCH_HISTORY if !self.shell.prefs.search_history() => {
                self.shell.start.history.clear();
                self.engine.invalidate();
            }
            p::SIGN_IN_AWAY => self.shell.idle.input(door_sys::time().unwrap_or(0)),
            p::TASKBAR => {
                use door_service_protocol::preferences::taskbar as tb;
                self.tray.seconds = value & tb::SECONDS != 0;
                self.taskbar_material.set_look(tb::look(value));
                if value & tb::AUTO_HIDE == 0 {
                    self.shell.taskbar_revealed = false;
                }
                // Auto-hide changes the work area; maximized windows follow.
                let scale = self.engine.scale();
                let work = door_window::taskbar::physical(self.taskbar_layout().work_area(), scale);
                let _ = self.engine.reflow_work_area(work);
                self.bar_dirty = true;
                self.icons_dirty = true;
                self.engine.invalidate();
            }
            p::LOCK_SCREEN => self.engine.invalidate(),
            p::START_FOLDERS => {
                self.shell.start.folders = value;
                self.engine.invalidate();
            }
            p::VISION => self.apply_vision(value),
            p::WINDOWS => {
                use door_service_protocol::preferences::windows as wd;
                self.engine
                    .set_square_corners(value & wd::SQUARE_CORNERS != 0);
                self.engine.set_animation_speed(wd::speed(value));
                self.engine
                    .set_mica_everywhere(value & wd::MICA_EVERYWHERE != 0);
            }
            p::BACKGROUND => {
                self.shell.slide = self.background;
                self.shell.slide_due = 0;
                if self.controls.contrast().is_none() {
                    let _ = self.refresh_wallpaper(self.background);
                }
            }
            _ => (),
        }
        // The recent-pictures and recent-colours lists are bookkeeping that
        // every background or accent pick rewrites; Settings logs its own
        // request, so they spend no window-server diagnostic credit.
        if !matches!(id, p::BACKGROUND_RECENT | p::ACCENT_RECENT) {
            let _ =
                door_sys::write(format!("DOOR_P14 PREFERENCE id={id} value={value}\n").as_bytes());
        }
        Ok(Response::empty())
    }
    /// "Show accent colour on title bars and window borders": the active
    /// window's trusted frame takes the accent (never under contrast).
    pub(super) fn apply_title_accent(&mut self) {
        let on = self
            .shell
            .prefs
            .accent_on_title_bars(self.controls.contrast().is_some());
        self.engine.set_title_accent(
            on.then(|| door_window::preferences::title_accent(self.resolve_accent())),
        );
    }
    /// The Quick settings network glyph: door.network's status every 3 s.
    /// The service's port is resolved through Init without blocking (Init
    /// calls this server while it binds a session, so a blocking resolve
    /// would deadlock both); status calls then go straight to the service,
    /// which never calls back. A service not ready yet is asked again in
    /// 3 s; a failed call drops the link for 30 s.
    pub(super) fn network_tick(&mut self, now: u64) {
        if let Some(init) = self.tray.network_resolve.as_mut() {
            match init.poll_resolve(now) {
                Ok(None) => return,
                Ok(Some(port)) => {
                    self.tray.network_link = door_service_client::Network::connect_port(port).ok();
                    let _ = door_sys::close(port);
                }
                Err(_) => self.tray.network_at = now.saturating_add(300),
            }
            self.tray.network_resolve = None;
        }
        if now < self.tray.network_at {
            return;
        }
        let Some(link) = self.tray.network_link.as_mut() else {
            self.tray.network_at = now.saturating_add(300);
            self.tray.network_resolve = door_service_client::Init::connect_now(self.registry)
                .ok()
                .and_then(|mut init| {
                    init.begin_resolve(door_abi::platform::manifest::NETWORK, now)
                        .ok()
                        .map(|()| init)
                });
            return;
        };
        self.tray.network_at = now.saturating_add(300);
        let status = match link.status() {
            Ok(status) => Some(status),
            Err(_) => {
                self.tray.network_link = None;
                self.tray.network_at = now.saturating_add(3000);
                None
            }
        };
        let net = door_window::taskbar::Net::of(status.as_ref());
        if net != self.tray.network {
            self.tray.network = net;
            self.bar_dirty = true;
            let _ = door_sys::write(format!("DOOR_P15 TRAY NETWORK {}\n", net.name()).as_bytes());
        }
    }
    /// door.audio: resolved and polled like door.network (once a second, at
    /// once after a change); the session user's SOUND preference is pushed
    /// whenever it differs from what the service last got, and a dragged
    /// volume slider follows the pointer.
    pub(super) fn audio_tick(&mut self, now: u64) {
        use door_service_protocol::{audio::Output, preferences as p};
        if self.tray.volume_drag {
            let (x, _) = self.logical_pointer();
            let volume = door_window::tray::volume_at(self.taskbar_layout(), x);
            let output = self.sound_output();
            if output.volume != volume || output.muted {
                self.set_sound_output(Output {
                    volume,
                    muted: false,
                    ..output
                });
            }
        }
        if let Some(init) = self.tray.audio_resolve.as_mut() {
            match init.poll_resolve(now) {
                Ok(None) => return,
                Ok(Some(port)) => {
                    self.tray.audio_link =
                        door_service_client::audio::Audio::connect_port(port).ok();
                    self.tray.audio_pushed = None;
                    self.tray.capture_pushed = None;
                    self.tray.input_pushed = None;
                    let _ = door_sys::close(port);
                }
                Err(_) => self.tray.audio_at = now.saturating_add(300),
            }
            self.tray.audio_resolve = None;
        }
        let Some(link) = self.tray.audio_link.as_mut() else {
            if now >= self.tray.audio_at {
                self.tray.audio_at = now.saturating_add(300);
                self.tray.audio_resolve = door_service_client::Init::connect_now(self.registry)
                    .ok()
                    .and_then(|mut init| {
                        init.begin_resolve(door_abi::platform::manifest::AUDIO, now)
                            .ok()
                            .map(|()| init)
                    });
            }
            return;
        };
        let desired = Output::from_word(self.shell.prefs.get(p::SOUND)).unwrap_or_default();
        let mut failed = false;
        if self.tray.audio_pushed != Some(desired) {
            if link.set_output(desired).is_ok() {
                self.tray.audio_pushed = Some(desired);
                self.tray.audio_at = now;
            } else {
                failed = true;
            }
        }
        // Privacy & security > Microphone, restored when the session starts
        // and whenever Settings changes it.
        let microphone = self.shell.prefs.get(p::MICROPHONE);
        let policy = microphone & door_service_protocol::audio::capture::POLICY_ALL;
        if !failed && self.tray.capture_pushed != Some(policy) {
            if link.capture_policy(policy).is_ok() {
                self.tray.capture_pushed = Some(policy);
            } else {
                failed = true;
            }
        }
        // Settings > System > Sound > Input > Volume (the same word).
        let input = door_service_protocol::audio::capture::input_of(microphone);
        if !failed && self.tray.input_pushed != Some(input) {
            if link.set_input(input).is_ok() {
                self.tray.input_pushed = Some(input);
            } else {
                failed = true;
            }
        }
        let mut adopted = None;
        let mut adopted_input = None;
        if !failed && now >= self.tray.audio_at {
            self.tray.audio_at = now.saturating_add(100);
            match link.status() {
                Ok(status) => {
                    // A program changed the volume or mute (the mixer API,
                    // as Windows lets any program): it becomes the user's
                    // setting, persisted like a change in quick settings.
                    if status.present
                        && self.tray.audio_pushed == Some(desired)
                        && status.output != desired
                    {
                        self.tray.audio_pushed = Some(status.output);
                        adopted = Some(status.output);
                    }
                    // A program changed the microphone level (the mixer
                    // API, IAudioEndpointVolume): the user's setting too.
                    if status.present
                        && self.tray.input_pushed == Some(input)
                        && status.input != input
                    {
                        self.tray.input_pushed = Some(status.input);
                        adopted_input = Some(status.input);
                    }
                    // Who is recording, for the microphone glyph's tooltip.
                    let names: Vec<alloc::string::String> = if status.capturing > 0 {
                        link.capture_status()
                            .map(|c| {
                                c.recent[..usize::from(c.used)]
                                    .iter()
                                    .filter(|u| u.active)
                                    .map(|u| alloc::string::String::from(u.name()))
                                    .collect()
                            })
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    if self.tray.capture_names != names {
                        self.tray.capture_names = names;
                    }
                    if self.tray.audio != Some(status) {
                        let first = self.tray.audio.is_none();
                        self.tray.audio = Some(status);
                        self.bar_dirty = true;
                        if self.tray.open == Some(door_window::tray::Flyout::Devices) {
                            self.engine.invalidate();
                        }
                        if first {
                            let _ = door_sys::write(
                                format!(
                                    "DOOR_P15 TRAY AUDIO present={} volume={} muted={}\n",
                                    u8::from(status.present),
                                    status.output.volume,
                                    u8::from(status.output.muted)
                                )
                                .as_bytes(),
                            );
                        }
                    }
                }
                Err(_) => failed = true,
            }
        }
        if let Some(output) = adopted {
            let _ = door_sys::write(
                format!(
                    "DOOR_P15 TRAY AUDIO ADOPTED volume={} muted={}\n",
                    output.volume,
                    u8::from(output.muted)
                )
                .as_bytes(),
            );
            self.set_sound_output(output);
        }
        if let Some(level) = adopted_input {
            let _ = door_sys::write(
                format!(
                    "DOOR_P15 TRAY AUDIO INPUT ADOPTED volume={} muted={}\n",
                    level.volume,
                    u8::from(level.muted)
                )
                .as_bytes(),
            );
            let id = p::MICROPHONE;
            let word = door_service_protocol::audio::capture::with_input(microphone, level);
            if self.shell.prefs.set(id, word).is_some() {
                self.queue_setting(id, word);
            }
        }
        if failed {
            self.tray.audio_link = None;
            self.tray.audio = None;
            self.tray.audio_at = now.saturating_add(3000);
            self.bar_dirty = true;
        }
    }
    /// Queue a preference word the shell changed in place for Init to
    /// persist (`PERSIST_SETTING`); a newer value for the same word
    /// replaces a queued one.
    pub(super) fn queue_setting(&mut self, id: u32, word: u32) {
        let pending = &mut self.shell.setting_pending;
        if let Some(slot) = pending.iter_mut().find(|(i, _)| *i == id) {
            slot.1 = word;
        } else {
            pending.push((id, word));
        }
    }
    /// Apply a VISION word (night light, colour filters, brightness):
    /// night light and brightness fade in, colour filters switch at once.
    pub(super) fn apply_vision(&mut self, word: u32) {
        use door_service_protocol::preferences::vision as v;
        let filters = v::FILTER | v::KIND_MASK;
        let now = self.shell_now();
        let fade = if (self.tray.vision ^ word) & filters != 0 || self.controls.reduce_motion() {
            0
        } else {
            door_window::scanout::VISION_FADE_US
        };
        self.tray.vision = word;
        if self.presentation.fade_vision(word, now, fade) {
            self.engine.invalidate();
        }
    }
    /// Change the VISION word from quick settings: apply, persist and tell
    /// the preference cache.
    pub(super) fn set_vision(&mut self, word: u32) {
        let id = door_service_protocol::preferences::VISION;
        if self.shell.prefs.set(id, word) != Some(true) {
            return;
        }
        self.apply_vision(word);
        self.queue_setting(id, word);
        let _ = door_sys::write(format!("DOOR_P15 QUICK VISION value={word}\n").as_bytes());
    }
    /// The brightness slider follows the pointer while dragged.
    pub(super) fn brightness_tick(&mut self) {
        if !self.tray.brightness_drag {
            return;
        }
        use door_service_protocol::preferences::vision as v;
        let (x, _) = self.logical_pointer();
        let level = door_window::tray::level_at(self.taskbar_layout(), 0, x);
        let current = self
            .shell
            .prefs
            .get(door_service_protocol::preferences::VISION);
        let word = v::with_brightness(current, u32::from(level));
        if word != current {
            self.set_vision(word);
        }
    }
    /// Play a system sound through door.audio (nothing without a device or
    /// before the link is up, as Windows stays silent without a sound card).
    pub(super) fn play_sound(&mut self, event: u32) {
        if let Some(link) = self.tray.audio_link.as_mut()
            && self.tray.audio.is_some_and(|a| a.present)
        {
            let _ = link.play_sound(event);
        }
    }
    /// The output the session user chose (SOUND preference).
    pub(super) fn sound_output(&self) -> door_service_protocol::audio::Output {
        door_service_protocol::audio::Output::from_word(
            self.shell
                .prefs
                .get(door_service_protocol::preferences::SOUND),
        )
        .unwrap_or_default()
    }
    /// Change the master output from quick settings: the preference word
    /// (persisted through Init), door.audio at the next tick, and the glyphs
    /// at once.
    pub(super) fn set_sound_output(&mut self, output: door_service_protocol::audio::Output) {
        let word = output.word();
        let id = door_service_protocol::preferences::SOUND;
        if self.shell.prefs.set(id, word).is_none() {
            return;
        }
        self.queue_setting(id, word);
        if let Some(status) = self.tray.audio.as_mut() {
            status.output = output;
        }
        self.bar_dirty = true;
        self.engine.invalidate();
    }
    /// The Light/Dark schedule and the sign-in-away lock, once per tick.
    pub(super) fn preferences_tick(&mut self, now: u64) {
        self.slideshow_tick(now);
        let minute = self.tray.time.map(door_window::preferences::minute_of_day);
        let schedule = self.shell.prefs.theme_schedule();
        if let Some(dark) = self.shell.theme_schedule.tick(schedule, minute)
            && dark != self.dark
        {
            let _ = self.apply_appearance(
                door_window::policy::appearance_value(
                    w::SET_THEME,
                    u32::from(dark),
                    self.background,
                )
                .unwrap_or(u32::from(dark)),
            );
            self.engine.set_theme(if self.dark {
                g::Theme::Dark
            } else {
                g::Theme::Light
            });
            let _ = door_sys::write(
                format!("DOOR_P14 THEME_SCHEDULE dark={}\n", u8::from(dark)).as_bytes(),
            );
        }
        let away = self.shell.prefs.sign_in_away();
        if self
            .shell
            .idle
            .due(away, now, self.session.is_some(), self.shell.locked)
        {
            let _ = door_sys::write(format!("DOOR_P14 IDLE_LOCK minutes={away}\n").as_bytes());
            self.lock();
        }
    }
    /// The colour 0xRRGGBB the accent setting resolves to now: Automatic
    /// picks the dominant colour of the desktop background.
    pub(super) fn resolve_accent(&self) -> u32 {
        use door_service_protocol::desktop::AccentMode;
        match self.accent.mode() {
            AccentMode::Default => door_window::paint::DEFAULT_ACCENT,
            AccentMode::Manual(rgb) => rgb,
            AccentMode::Automatic => door_design_tokens::accent::from_background(
                self.wallpaper
                    .chunks_exact(4 * 61)
                    .map(|p| [p[0], p[1], p[2], 255]),
            ),
        }
    }
    /// Paint the shell with the resolved accent and tell every appearance
    /// client (APPEARANCE_CHANGED `a`). `report` logs the applied setting.
    fn apply_accent(&mut self, report: bool) {
        let rgb = self.resolve_accent();
        let reported = (self.accent.bits(), rgb);
        if report && self.accent_reported != Some(reported) {
            self.accent_reported = Some(reported);
            let _ = door_sys::write(
                format!(
                    "DOOR_P14 ACCENT applied={:#x} rgb={rgb:06x}\n",
                    self.accent.bits()
                )
                .as_bytes(),
            );
        }
        if rgb == self.accent_rgb {
            return;
        }
        self.accent_rgb = rgb;
        door_window::paint::set_accent(rgb);
        self.apply_title_accent();
        self.bar_dirty = true;
        self.engine.invalidate();
        let client = self.client_appearance();
        for c in &mut self.clients {
            if c.appearance_events {
                c.appearance_pending = Some(client);
            }
        }
    }
    fn time_locale_request(&mut self, b: &[u8], t: &[Transfer]) -> Result<Response, Code> {
        if b.len() != 8 || !t.is_empty() || !self.tray.set_locale(u32_at(b, 0), u32_at(b, 4)) {
            return Err(Code::InvalidRequest);
        }
        self.bar_dirty = true;
        self.engine.invalidate();
        let _ = door_sys::write(
            format!(
                "DOOR_P12 TIME_LOCALE zone={} region={}\n",
                u32_at(b, 0),
                u32_at(b, 4)
            )
            .as_bytes(),
        );
        Ok(Response::empty())
    }
    /// Win32 window relations (CAP_WIN32): SET_OWNER, SET_ENABLED,
    /// MOVE_RESIZE and SET_SIZE_LIMITS. Every named window must belong to
    /// the authenticated caller.
    fn win32_request(
        &mut self,
        owner: g::Owner,
        op: u32,
        b: &[u8],
        t: &[Transfer],
    ) -> Result<Response, Code> {
        use door_window::win32 as r;
        if !t.is_empty() || b.len() < 8 {
            return Err(Code::InvalidRequest);
        }
        let id = g::WindowId(u64_at(b, 0));
        let info = self.engine.window_info(id).map_err(error)?;
        if info.owner != owner {
            return Err(Code::Denied);
        }
        match op {
            w::SET_OWNER if b.len() == 16 => {
                let target = u64_at(b, 8);
                let target = if target == 0 {
                    None
                } else {
                    let t = self
                        .engine
                        .window_info(g::WindowId(target))
                        .map_err(error)?;
                    if t.owner != owner {
                        return Err(Code::Denied);
                    }
                    Some(target)
                };
                self.win32.set_owner(id.0, target)?;
            }
            w::SET_ENABLED if b.len() == 12 => {
                let enabled = match u32_at(b, 8) {
                    0 => false,
                    1 => true,
                    _ => return Err(Code::InvalidRequest),
                };
                self.win32.set_enabled(id.0, enabled)?;
                // A window disabled while it holds focus hands it to its modal.
                if !enabled
                    && self.engine.focused_window() == Some(id)
                    && let Some(target) = self.win32.modal_target(id.0)
                {
                    let _ = self.engine.activate(g::WindowId(target));
                }
            }
            w::MOVE_RESIZE if b.len() == 24 => {
                let edges = u32_at(b, 8);
                if u32_at(b, 12) != 0 {
                    return Err(Code::InvalidRequest);
                }
                r::admit_move_resize(
                    self.win32_press,
                    self.buttons & 1 != 0,
                    self.drag.is_some(),
                    id.0,
                    u64_at(b, 16),
                    edges,
                )?;
                let movable = info.state == g::WindowState::Normal
                    || (edges == 0 && info.state == g::WindowState::Maximized);
                if !movable {
                    return Err(Code::InvalidRequest);
                }
                self.drag = Some(Drag {
                    id,
                    owner,
                    rect: info.outer,
                    x: self.x,
                    y: self.y,
                    edges: r::server_edges(edges),
                    epoch: info.epoch,
                    restore_pending: edges == 0
                        && (info.snapped || info.state == g::WindowState::Maximized),
                    moved: false,
                });
                self.win32_press = None;
                // The trusted drag owns the pointer from here on.
                let _ = self.engine.release_capture(owner, id);
            }
            w::SET_SIZE_LIMITS if b.len() == 24 => {
                self.win32.set_limits(
                    id.0,
                    r::Limits {
                        min_width: u32_at(b, 8),
                        min_height: u32_at(b, 12),
                        max_width: u32_at(b, 16),
                        max_height: u32_at(b, 20),
                    },
                )?;
            }
            w::SET_FRAMELESS if b.len() == 12 => {
                let frameless = match u32_at(b, 8) {
                    0 => false,
                    1 => true,
                    _ => return Err(Code::InvalidRequest),
                };
                self.engine
                    .set_frameless(owner, id, frameless)
                    .map_err(error)?;
            }
            w::SET_CAPTION_BUTTONS if b.len() == 12 => {
                let buttons = u8::try_from(u32_at(b, 8)).map_err(|_| Code::InvalidRequest)?;
                self.engine
                    .set_caption_buttons(owner, id, buttons)
                    .map_err(error)?;
            }
            w::SET_RESPONDING if b.len() == 12 => {
                let responding = match u32_at(b, 8) {
                    0 => false,
                    1 => true,
                    _ => return Err(Code::InvalidRequest),
                };
                if self.shell.hang.report(owner, id.0, responding)? {
                    self.ghost_changed(owner, id.0, !responding);
                }
            }
            _ => return Err(Code::InvalidRequest),
        }
        Ok(Response::empty())
    }
    fn request_inner(
        &mut self,
        channel: Handle,
        peer: Identity,
        op: u32,
        b: &[u8],
        t: &[Transfer],
    ) -> Result<Response, Code> {
        if op == w::SET_DESKTOP_CONTROLS
            && peer.manifest == door_abi::platform::manifest::INIT
            && peer.session == 0
        {
            return self.desktop_controls(b, t);
        }
        // Init sends this only on the adversarial test image.
        if op == w::SET_TEST_HOOKS
            && peer.manifest == door_abi::platform::manifest::INIT
            && peer.session == 0
        {
            if !b.is_empty() || !t.is_empty() {
                return Err(Code::InvalidRequest);
            }
            self.presentation.enable_test_hooks();
            let _ = door_sys::write(b"DOOR_P12 TEST_HOOKS present-fault=1\n");
            return Ok(Response::empty());
        }
        // door.text-input: Init applies the user's input methods.
        if (door_service_protocol::text_input::FIRST..=door_service_protocol::text_input::LAST)
            .contains(&op)
            && peer.manifest == door_abi::platform::manifest::INIT
            && peer.session == 0
        {
            return self.text_input_init(peer, op, b, t);
        }
        // Shell: the session user's persisted pins/Start view (Init only).
        if op == w::SET_SHELL_LAYOUT
            && peer.manifest == door_abi::platform::manifest::INIT
            && peer.session == 0
        {
            return self.apply_shell_layout(b, t);
        }
        if matches!(op, w::SET_THEME | w::SET_APPEARANCE)
            && peer.manifest == door_abi::platform::manifest::INIT
            && peer.session == 0
        {
            return self.appearance(op, b, t);
        }
        // Accent colour (Personalization > Colors): Init binds the user's
        // stored setting.
        if op == w::SET_ACCENT
            && peer.manifest == door_abi::platform::manifest::INIT
            && peer.session == 0
        {
            return self.accent_request(b, t);
        }
        if op == w::SET_PREFERENCE
            && peer.manifest == door_abi::platform::manifest::INIT
            && peer.session == 0
        {
            return self.preference_request(b, t);
        }
        // Time zone and regional format (spec §109): Init binds the user's
        // stored settings.
        if op == w::SET_TIME_LOCALE
            && peer.manifest == door_abi::platform::manifest::INIT
            && peer.session == 0
        {
            return self.time_locale_request(b, t);
        }
        // Display scale (spec §57): Init binds the user's stored preference.
        if op == w::SET_DISPLAY_SCALE
            && peer.manifest == door_abi::platform::manifest::INIT
            && peer.session == 0
        {
            return self.display_scale_request(b, t);
        }
        if op == w::BIND_SESSION {
            if peer.manifest != door_abi::platform::manifest::INIT
                || peer.session != 0
                || !matches!(b.len(), 24 | 32 | 40)
                || !t.is_empty()
                || b[20..24].iter().any(|&v| v != 0)
                || b.len() >= 32 && (u32_at(b, 24) > 1 || b[28..32].iter().any(|&v| v != 0))
                || b.len() == 40 && (u32_at(b, 32) >= 4 || b[36..].iter().any(|&v| v != 0))
            {
                return Err(Code::Denied);
            }
            let binding = (u64_at(b, 0), u64_at(b, 8), u32_at(b, 16));
            if binding.0 == 0 || binding.1 == 0 || binding.2 < 1000 {
                return Err(Code::InvalidRequest);
            }
            if self.session.is_some_and(|old| old != binding) {
                return Err(Code::Denied);
            }
            self.session = Some(binding);
            self.shell_focus = None;
            self.login_pressed = false;
            if b.len() >= 32 {
                self.apply_appearance(
                    u32_at(b, 24)
                        | (if b.len() == 40 {
                            u32_at(b, 32)
                        } else {
                            self.background
                        } << 1),
                )?;
                self.engine.set_theme(if self.dark {
                    g::Theme::Dark
                } else {
                    g::Theme::Light
                });
            }
            // Every admitted protocol form replaces the login surface. In
            // particular legacy24 carries no theme to invalidate it indirectly.
            self.engine.invalidate();
            self.binding_presentation = Some((b.len() as u8, self.login_reported));
            return Ok(Response::empty());
        }
        let (session, epoch, uid) = self.session.ok_or(Code::WouldBlock)?;
        // The signed-in user's applications, and Windows services (the
        // service account's session): Doors has no separate
        // non-interactive desktop yet, so a service's windows are the
        // user's desktop's.
        let user = server::session_peer(peer)
            && peer.session == session
            && peer.epoch == epoch
            && peer.uid == uid;
        if !user && !server::service_peer(peer) {
            return Err(Code::Denied);
        }
        let owner = g::Owner {
            pid: peer.pid,
            session: peer.session,
            epoch: peer.instance,
        };
        if self.terminations.contains(owner) {
            return Err(Code::Closing);
        }
        if let Some(client) = self.clients.iter().find(|c| c.owner == owner) {
            if client.channel != channel {
                return Err(Code::Denied);
            }
            if client.revoked {
                return Err(Code::Closing);
            }
        } else {
            // Six apps, the search host and the IME fill eight; the rest is
            // headroom for the gallery and assistive client (kernel cap 13).
            if self.clients.len() >= 12 {
                return Err(Code::Resources);
            }
            self.clients.push(Client {
                manifest: peer.manifest,
                multiwindow: false,
                new_windows: 0,
                channel,
                owner,
                revoked: false,
                transient_events: false,
                appearance_events: false,
                appearance_pending: None,
                controls_events: false,
                controls_pending: None,
                wheel_events: false,
                scale_events: false,
                scale_pending: false,
                win32: false,
            });
        }
        // REGISTER and clipboard WRITE_SECTION carry one section handle;
        // REQUEST_ELEVATION a working folder and the program's image.
        if op != w::REGISTER
            && op != door_service_protocol::clipboard::WRITE_SECTION
            && op != w::REQUEST_ELEVATION
            && op != w::NOTIFY_ICON
            && !t.is_empty()
        {
            return Err(Code::InvalidRequest);
        }
        // door.shell: the desktop shell role and SET_TASKBAR_STATE.
        if (door_service_protocol::shell::FIRST..=door_service_protocol::shell::LAST).contains(&op)
        {
            return self.shell_request(owner, peer, op, b);
        }
        match op {
            w::LAUNCH_APP if b.len() == 4 || b.len() == 8 => {
                let destination = if b.len() == 8 {
                    // Settings (3) at any route the shared route table knows,
                    // optionally with a section (door_settings_map).
                    if u32_at(b, 0) != 3
                        || door_settings_map::decode_navigate(u32_at(b, 4)).is_none()
                    {
                        return Err(Code::InvalidRequest);
                    }
                    if !self
                        .clients
                        .iter()
                        .any(|c| c.owner == owner && c.controls_events)
                    {
                        return Err(Code::Unsupported);
                    }
                    // An elevated route (Registry Editor, a management
                    // snap-in) waits for administrator approval first.
                    if door_settings_map::elevation(u32_at(b, 4)).is_some() {
                        self.open_settings(u32_at(b, 4));
                        return Ok(Response::empty());
                    }
                    Some(u32_at(b, 4))
                } else {
                    None
                };
                // Search-only accessories (gallery 12, Captions 13), e.g. from
                // the desktop shell's registration database.
                if destination.is_none()
                    && let Some(tool) = door_window::apps::Tool::ALL
                        .into_iter()
                        .find(|t| u32::from(t.command()) == u32_at(b, 0))
                {
                    self.launch_tool(tool);
                    return Ok(Response::empty());
                }
                let app = match u32_at(b, 0) {
                    1 => desktop::App::Files,
                    2 => desktop::App::Notes,
                    3 => desktop::App::Settings,
                    9 => desktop::App::Terminal,
                    10 => desktop::App::Preview,
                    11 => desktop::App::Monitor,
                    _ => return Err(Code::InvalidRequest),
                };
                self.launch_app(app).map_err(|e| match e {
                    door_service_client::Error::Service(code) => code,
                    door_service_client::Error::Transport(status) => server::status(status),
                    door_service_client::Error::Malformed => Code::Io,
                })?;
                if let Some(page) = destination {
                    self.settings_navigation = Some(page);
                }
                Ok(Response::empty())
            }
            w::LAUNCH_WINDOW if b.len() == 4 => {
                if !self
                    .clients
                    .iter()
                    .any(|c| c.owner == owner && c.multiwindow)
                {
                    return Err(Code::Unsupported);
                }
                let app = desktop::App::all()
                    .into_iter()
                    .find(|a| a.command() as u32 == u32_at(b, 0))
                    .ok_or(Code::InvalidRequest)?;
                self.launch_window(app).map_err(|e| match e {
                    door_service_client::Error::Service(c) => c,
                    _ => Code::Io,
                })?;
                Ok(Response::empty())
            }
            w::SET_THEME | w::SET_APPEARANCE => self.appearance(op, b, t),
            w::SET_DESKTOP_CONTROLS => self.desktop_controls(b, t),
            // Settings > Display > Scale (workspace apps only).
            w::SET_DISPLAY_SCALE if peer.manifest == door_abi::platform::manifest::WORKSPACE => {
                self.display_scale_request(b, t)
            }
            w::DISPLAY_SCALE_INFO if b.is_empty() => {
                Ok(Response::bytes(&self.display_scale_info()))
            }
            // File Explorer: the Recycle Bin's full/empty state.
            w::SET_RECYCLE_BIN if peer.manifest == door_abi::platform::manifest::WORKSPACE => {
                if b.len() != 4 || !t.is_empty() || u32_at(b, 0) > 1 {
                    return Err(Code::InvalidRequest);
                }
                let full = u32_at(b, 0) == 1;
                if full != self.recycle_full {
                    self.recycle_full = full;
                    self.invalidate_icons();
                }
                Ok(Response::empty())
            }
            // Settings > Personalization > Colors (workspace apps only).
            w::SET_ACCENT if peer.manifest == door_abi::platform::manifest::WORKSPACE => {
                self.accent_request(b, t)
            }
            // Settings: a preference word the user changed (workspace apps only).
            w::SET_PREFERENCE if peer.manifest == door_abi::platform::manifest::WORKSPACE => {
                self.preference_request(b, t)
            }
            // Settings > Time & language (workspace apps only).
            w::SET_TIME_LOCALE if peer.manifest == door_abi::platform::manifest::WORKSPACE => {
                self.time_locale_request(b, t)
            }
            w::SET_CURSOR if b.len() == 12 => {
                let shape = match u32_at(b, 8) {
                    w::CURSOR_ARROW => g::CursorIcon::Arrow,
                    w::CURSOR_TEXT => g::CursorIcon::Text,
                    w::CURSOR_SIZE_WE => g::CursorIcon::EastWest,
                    _ => return Err(Code::InvalidRequest),
                };
                self.engine
                    .set_cursor(owner, g::WindowId(u64_at(b, 0)), shape)
                    .map_err(error)?;
                self.cursor_dirty = true;
                Ok(Response::empty())
            }
            w::CANCEL_CLOSE if b.len() == 8 => {
                let id = g::WindowId(u64_at(b, 0));
                // A Win32 client acknowledges every CLOSE the moment it
                // retrieves it, before WM_CLOSE runs. Outside a shutdown
                // its window never left its state, so there is nothing to
                // restore and no deadline to clear.
                if self.shell.hang.is_client(owner) {
                    let info = self.engine.window_info(id).map_err(error)?;
                    if info.owner != owner {
                        return Err(Code::Denied);
                    }
                    self.shell.hang.acknowledge(owner, id.0);
                    if info.state != g::WindowState::Closing {
                        return Ok(Response::empty());
                    }
                }
                self.engine.cancel_close(owner, id).map_err(error)?;
                self.closing.retain(|v| v.1 != id);
                // Shell: during a shutdown the app is asking about unsaved
                // work; the shutdown waits for the answer (Windows 11).
                if self
                    .shell
                    .shutdown_gate
                    .close_deferred(self.shutdown.is_some(), owner)
                    == door_window::policy::Deferred::WindowOnly
                {
                    self.unfreeze_pending = true;
                }
                Ok(Response::empty())
            }
            w::CLOSE_DECLINED if b.len() == 8 => {
                let id = g::WindowId(u64_at(b, 0));
                if self.engine.window_info(id).map_err(error)?.owner != owner {
                    return Err(Code::Denied);
                }
                if self
                    .shell
                    .shutdown_gate
                    .declined(self.shutdown.is_some(), owner)
                {
                    self.cancel_shutdown(owner);
                }
                Ok(Response::empty())
            }
            w::HELLO if b.len() == 4 || b.len() == 8 => {
                if u32_at(b, 0) != w::VERSION {
                    return Err(Code::Unsupported);
                }
                let features = if b.len() == 8 { u32_at(b, 4) } else { 0 };
                let transients = features & w::CAP_TRANSIENT_DISMISS != 0;
                if features
                    & !(w::CAP_TRANSIENT_DISMISS
                        | w::CAP_EPOCH_COMMIT
                        | w::CAP_CLIENT_CAPTION
                        | w::CAP_TABBED_CAPTION
                        | w::CAP_MULTI_WINDOW
                        | w::CAP_APPEARANCE
                        | w::CAP_DESKTOP_CONTROLS
                        | w::CAP_WHEEL
                        | w::CAP_SEARCH_PROVIDER
                        | w::CAP_DISPLAY_SCALE
                        | w::CAP_WIN32
                        | door_service_protocol::clipboard::CAP_CLIPBOARD
                        | door_service_protocol::dnd::CAP_DRAG_DROP
                        | door_service_protocol::text_input::CAP_TEXT_INPUT
                        | door_service_protocol::text_input::CAP_IME
                        | door_service_protocol::accessibility::CAP_ACCESSIBILITY
                        | door_service_protocol::accessibility::CAP_ASSISTIVE)
                    != 0
                {
                    return Err(Code::Unsupported);
                }
                // Wheel INPUT events are opt-in (legacy clients keep their
                // exact event set) and granted only with a wheel device.
                let features = if self.wheel_input {
                    features
                } else {
                    features & !w::CAP_WHEEL
                };
                // Shell: only workspace apps and the desktop shell (which hosts
                // Start file search) may answer it.
                let features = if matches!(
                    peer.manifest,
                    door_abi::platform::manifest::WORKSPACE
                        | door_abi::platform::manifest::DESKTOP_SHELL
                ) {
                    features
                } else {
                    features & !w::CAP_SEARCH_PROVIDER
                };
                let features = self.text_input_hello(owner, peer, features);
                // door.accessibility: CAP_ASSISTIVE only for the Captions
                // manifest, decided from the kernel identity.
                let features = self.accessibility_hello(owner, peer, features);
                // Win32 relations and close semantics: Windows-personality
                // manifests only (kernel identity), tracked for hangs.
                let mut features = door_window::win32::grant(peer.manifest, features);
                if features & w::CAP_WIN32 != 0 && self.shell.hang.register(owner).is_err() {
                    features &= !w::CAP_WIN32;
                }
                let appearance = self.client_appearance();
                let client = self
                    .clients
                    .iter_mut()
                    .find(|c| c.owner == owner)
                    .ok_or(Code::InvalidRequest)?;
                client.win32 = features & w::CAP_WIN32 != 0;
                client.wheel_events = features & w::CAP_WHEEL != 0;
                client.transient_events = transients;
                client.multiwindow = features & w::CAP_MULTI_WINDOW != 0;
                client.appearance_events = features & w::CAP_APPEARANCE != 0;
                client.controls_events = features & w::CAP_DESKTOP_CONTROLS != 0;
                if client.controls_events {
                    client.controls_pending = Some(self.controls.bits());
                }
                // Fractional DPI and SCALE_CHANGED (initially and on every
                // change) only for clients that render at any scale.
                client.scale_events = features & w::CAP_DISPLAY_SCALE != 0;
                client.scale_pending = client.scale_events;
                let dpi = if client.scale_events {
                    self.engine.scale().dpi()
                } else {
                    96 * self.engine.scale().legacy_factor()
                };
                if client.appearance_events {
                    client.appearance_pending = Some(appearance);
                }
                if features & w::CAP_SEARCH_PROVIDER != 0 {
                    self.search_provider_hello(owner);
                }
                // door.clipboard / door.dnd (native-ui-v1.md).
                self.native_ui_hello(owner, features);
                let mut out = [0; w::HELLO_BYTES];
                let r = self.engine.output_rect();
                let counts = self.engine.resource_counts();
                for (chunk, value) in out.chunks_exact_mut(4).zip([
                    w::VERSION,
                    r.width(),
                    r.height(),
                    0,
                    dpi,
                    // Legacy 4-byte HELLOs keep exactly the original word;
                    // band commits need epoch commits, which need 8 bytes.
                    w::CAP_CPU_SNAPSHOT
                        | if b.len() == 8 {
                            w::CAP_BAND_COMMIT | w::CAP_ACRYLIC
                        } else {
                            0
                        }
                        | features,
                    4,
                    8,
                    g::MAX_SURFACE_BYTES as u32,
                    8,
                    8,
                    counts.0 as u32,
                    counts.1 as u32,
                    self.attachments.iter().filter(|a| a.owner == owner).count() as u32,
                    0,
                    0,
                ]) {
                    chunk.copy_from_slice(&value.to_le_bytes());
                }
                Ok(Response::bytes(&out))
            }
            w::PING if b.is_empty() => Ok(Response::empty()),
            w::TAB_TARGET if b.len() == 8 => {
                if !self
                    .clients
                    .iter()
                    .any(|c| c.owner == owner && c.multiwindow)
                {
                    return Err(Code::Unsupported);
                }
                let source = g::WindowId(u64_at(b, 0));
                let target = self
                    .engine
                    .tab_drop_target(owner, source, self.x, self.y)
                    .map_err(error)?;
                let mut out = [0; 16];
                if let Some((id, x, y)) = target {
                    out[..8].copy_from_slice(&id.0.to_le_bytes());
                    out[8..12].copy_from_slice(&x.to_le_bytes());
                    out[12..].copy_from_slice(&y.to_le_bytes());
                }
                Ok(Response::bytes(&out))
            }
            w::CAPTURE if b.len() == 8 => {
                self.engine
                    .set_capture(owner, g::WindowId(u64_at(b, 0)))
                    .map_err(error)?;
                Ok(Response::empty())
            }
            w::RELEASE_CAPTURE if b.len() == 8 => {
                self.engine
                    .release_capture(owner, g::WindowId(u64_at(b, 0)))
                    .map_err(error)?;
                Ok(Response::empty())
            }
            w::POLL_BATCH if b.is_empty() => {
                // Retrieval is what the hung rule measures (Win32 clients).
                self.shell.hang.polled(owner);
                let mut out = [0; 3 * w::EVENT_BYTES];
                let mut count = 0;
                // A scale change precedes the CONFIGURED events it causes.
                if let Some(event) = self.take_scale(owner) {
                    out[..w::EVENT_BYTES].copy_from_slice(&event.encode());
                    count = 1;
                }
                // Controls first: a contrast change reaches the client before
                // the appearance it implies.
                if let Some(event) = self.take_controls(owner) {
                    out[count * w::EVENT_BYTES..(count + 1) * w::EVENT_BYTES]
                        .copy_from_slice(&event.encode());
                    count += 1;
                }
                if let Some(event) = self.take_appearance(owner) {
                    out[count * w::EVENT_BYTES..(count + 1) * w::EVENT_BYTES]
                        .copy_from_slice(&event.encode());
                    count += 1;
                }
                if count < 3
                    && let Some(event) = self.take_settings_navigation(owner)
                {
                    out[count * w::EVENT_BYTES..(count + 1) * w::EVENT_BYTES]
                        .copy_from_slice(&event.encode());
                    count += 1;
                }
                if count < 3
                    && let Some(event) = self.take_search_event(owner)
                {
                    out[count * w::EVENT_BYTES..(count + 1) * w::EVENT_BYTES]
                        .copy_from_slice(&event.encode());
                    count += 1;
                }
                if count < 3
                    && let Some(event) = self.take_snip_event(owner)
                {
                    out[count * w::EVENT_BYTES..(count + 1) * w::EVENT_BYTES]
                        .copy_from_slice(&event.encode());
                    count += 1;
                }
                if count < 3
                    && let Some(c) = self
                        .clients
                        .iter_mut()
                        .find(|c| c.owner == owner && c.multiwindow && c.new_windows > 0)
                {
                    c.new_windows -= 1;
                    let event = w::Event {
                        kind: w::NEW_WINDOW,
                        ..w::Event::default()
                    };
                    out[count * w::EVENT_BYTES..(count + 1) * w::EVENT_BYTES]
                        .copy_from_slice(&event.encode());
                    count += 1;
                }
                while count < 3 {
                    let Some(event) = self
                        .take_native_event(owner)
                        .or_else(|| self.shell.text_input.take_event(owner))
                    else {
                        break;
                    };
                    out[count * w::EVENT_BYTES..(count + 1) * w::EVENT_BYTES]
                        .copy_from_slice(&event.encode());
                    count += 1;
                }
                while count < 3 {
                    let Some(e) = self.engine.poll_event(owner) else {
                        break;
                    };
                    out[count * w::EVENT_BYTES..(count + 1) * w::EVENT_BYTES]
                        .copy_from_slice(&encode(e).encode());
                    count += 1;
                }
                // Win32 close requests follow the input queued before them.
                while count < 3 {
                    let Some(event) = self.take_win32_close(owner) else {
                        break;
                    };
                    out[count * w::EVENT_BYTES..(count + 1) * w::EVENT_BYTES]
                        .copy_from_slice(&event.encode());
                    count += 1;
                }
                if count == 0 {
                    return Err(Code::WouldBlock);
                }
                Ok(Response::bytes(&out[..count * w::EVENT_BYTES]))
            }
            w::SET_TITLE if (8..=136).contains(&b.len()) => {
                let id = g::WindowId(u64_at(b, 0));
                let title = core::str::from_utf8(&b[8..]).map_err(|_| Code::InvalidRequest)?;
                if !title.bytes().all(|c| (32..=126).contains(&c)) {
                    return Err(Code::InvalidRequest);
                }
                self.engine.set_title(owner, id, title).map_err(error)?;
                Ok(Response::empty())
            }
            w::REQUEST_CLOSE if b.len() == 8 => {
                self.close(owner, g::WindowId(u64_at(b, 0)))
                    .map_err(error)?;
                Ok(Response::empty())
            }
            w::CREATE => {
                if matches!(
                    peer.manifest,
                    door_abi::platform::manifest::WORKSPACE
                        | door_abi::platform::manifest::PREVIEW
                        | door_abi::platform::manifest::MONITOR
                ) && !self
                    .app_classes
                    .iter()
                    .any(|(identity, _)| *identity == owner)
                {
                    return Err(Code::WouldBlock);
                }
                if b.len() < 32 || u32_at(b, 20) as usize != b.len() - 32 || u32_at(b, 20) > 128 {
                    return Err(Code::InvalidRequest);
                }
                let role = match u32_at(b, 0) {
                    0 if u64_at(b, 24) == 0 => g::Role::TopLevel,
                    w::CLIENT_CAPTION_ROLE | w::TABBED_CAPTION_ROLE => g::Role::TopLevel,
                    1 => g::Role::ShellPanel,
                    2 => g::Role::LockScreen,
                    3 => g::Role::OwnedPopup(g::WindowId(u64_at(b, 24))),
                    _ => return Err(Code::Unsupported),
                };
                let title = core::str::from_utf8(&b[32..]).map_err(|_| Code::InvalidRequest)?;
                if !title.bytes().all(|c| (32..=126).contains(&c)) {
                    return Err(Code::InvalidRequest);
                }
                let outer = rect(&b[4..20])?;
                let id = if matches!(
                    u32_at(b, 0),
                    w::CLIENT_CAPTION_ROLE | w::TABBED_CAPTION_ROLE
                ) {
                    self.engine.create_client_caption(
                        owner,
                        outer,
                        g::CaptionLayout {
                            tabbed: u32_at(b, 0) == w::TABBED_CAPTION_ROLE,
                            max_width: if u32_at(b, 0) == w::TABBED_CAPTION_ROLE {
                                u32_at(b, 24) & 0xffff
                            } else {
                                u32_at(b, 24)
                            },
                            height: if u32_at(b, 0) == w::TABBED_CAPTION_ROLE {
                                u32_at(b, 24) >> 16
                            } else {
                                32
                            },
                            side_margin: u32_at(b, 28),
                        },
                    )
                } else {
                    self.engine.create_window(owner, role, outer)
                }
                .map_err(error)?;
                self.engine.set_title(owner, id, title).map_err(error)?;
                if role == g::Role::TopLevel
                    && let Some((_, app)) = self.app_classes.iter().find(|v| v.0 == owner)
                {
                    if let Some(workspace) = self.app_workspaces[app.index()]
                        .filter(|id| self.engine.workspace_exists(*id))
                    {
                        self.engine
                            .move_to_workspace(owner, id, workspace)
                            .map_err(error)?;
                    }
                    if self.engine.window_info(id).is_ok_and(|w| w.visible)
                        && self.foreground.allows_new(app.command())
                    {
                        self.engine.activate(id).map_err(error)?;
                    } else {
                        self.engine.lower_top_level(owner, id).map_err(error)?;
                    }
                } else if role == g::Role::TopLevel
                    && let Some(tool) = door_window::apps::Tool::from_manifest(peer.manifest)
                    && self.foreground.allows_new(tool.command())
                    && self.engine.window_info(id).is_ok_and(|w| w.visible)
                {
                    // A tool launched from Start search (Doors UI Gallery,
                    // Captions) takes keyboard focus as an app does; without
                    // this its window opened on top but unfocused.
                    self.engine.activate(id).map_err(error)?;
                    self.foreground.complete();
                }
                Ok(Response::scalar(id.0))
            }
            w::REQUEST_ELEVATION => self.elevation_request(owner.pid, peer.manifest, b, t),
            w::ELEVATION_STATUS if b.is_empty() => Ok(self.elevation_status(owner.pid)),
            w::FIREWALL_CHANGE => self.firewall_change_request(peer.manifest, b),
            w::NOTIFY_ICON => self.notify_icon_request(owner, peer.manifest, b, t),
            w::OPEN_WITH => {
                // A Windows program's ShellExecute of a file no Windows
                // association opens: File Explorer takes it, as OpenWith.exe
                // does on Windows (its own default app, or the picker).
                use door_abi::platform::manifest::{WINDOWS, WINDOWS_APPLICATION, WINDOWS_GUI};
                if !matches!(peer.manifest, WINDOWS | WINDOWS_APPLICATION | WINDOWS_GUI) {
                    return Err(Code::Denied);
                }
                let (&mode, path) = b.split_first().ok_or(Code::InvalidRequest)?;
                let path = core::str::from_utf8(path).map_err(|_| Code::InvalidRequest)?;
                if !matches!(mode, w::open_with::OPEN | w::open_with::CHOOSE)
                    || !path.starts_with('/')
                    || path.ends_with('/')
                    || path.contains('\0')
                    || b.len() > w::MAX_DOCUMENT
                {
                    return Err(Code::InvalidRequest);
                }
                let _ = door_sys::write(
                    format!("DOOR_P16 OPEN_WITH {} {path}\n", mode as char).as_bytes(),
                );
                self.deliver_document(door_window::apps::App::Files, b)
            }
            w::SET_FULLSCREEN => {
                if b.len() != 12 || !t.is_empty() {
                    return Err(Code::InvalidRequest);
                }
                let id = g::WindowId(u64_at(b, 0));
                let on = match u32_at(b, 8) {
                    0 => false,
                    1 => true,
                    _ => return Err(Code::InvalidRequest),
                };
                if on && self.engine.focused_window() != Some(id) {
                    return Err(Code::Denied);
                }
                self.engine.set_fullscreen(owner, id, on).map_err(error)?;
                // The taskbar hides under (or returns from) the window.
                self.bar_dirty = true;
                self.engine.invalidate();
                door_rt::diagnostic(format_args!(
                    "DOOR_P16 FULLSCREEN window={} on={}\n",
                    id.0,
                    u8::from(on)
                ));
                Ok(Response::empty())
            }
            w::SET_MATERIAL => {
                if b.len() != 12 || !t.is_empty() {
                    return Err(Code::InvalidRequest);
                }
                let material = match u32_at(b, 8) {
                    w::MATERIAL_DEFAULT => g::Material::Default,
                    w::MATERIAL_ACRYLIC => g::Material::Acrylic,
                    w::MATERIAL_MICA => g::Material::Mica,
                    _ => return Err(Code::InvalidRequest),
                };
                self.engine
                    .set_material(owner, g::WindowId(u64_at(b, 0)), material)
                    .map_err(error)?;
                Ok(Response::empty())
            }
            w::SET_TABBED_CAPTION => {
                if b.len() != 16 || !t.is_empty() {
                    return Err(Code::InvalidRequest);
                }
                self.engine
                    .set_tabbed_caption(
                        owner,
                        g::WindowId(u64_at(b, 0)),
                        u32_at(b, 8),
                        u32_at(b, 12),
                    )
                    .map_err(error)?;
                Ok(Response::empty())
            }
            w::REGISTER => {
                if b.len() != 24 || t.len() != 1 {
                    return Err(Code::InvalidRequest);
                }
                let descriptor = g::SurfaceDescriptor::new(
                    u32_at(b, 0),
                    u32_at(b, 4),
                    usize::try_from(u64_at(b, 8)).map_err(|_| Code::InvalidRequest)?,
                    usize::try_from(u64_at(b, 16)).map_err(|_| Code::InvalidRequest)?,
                )
                .map_err(error)?;
                let info = door_sys::inspect(t[0].handle).map_err(server::status)?;
                if info[0] != HandleKind::Section as u64 || info[1] & rights::WRITE != 0 {
                    return Err(Code::Denied);
                }
                // Verify the declared last byte against the real kernel-owned backing.
                let mut last = [0];
                door_sys::section_snapshot(
                    t[0].handle,
                    descriptor.backing_bytes as u64 - 1,
                    &mut last,
                )
                .map_err(server::status)?;
                let handle =
                    door_sys::duplicate(t[0].handle, rights::READ | rights::MAP | rights::QUERY)
                        .map_err(server::status)?;
                let id = match self.engine.register_buffer(owner, descriptor) {
                    Ok(id) => id,
                    Err(e) => {
                        let _ = door_sys::close(handle);
                        return Err(error(e));
                    }
                };
                self.buffers.push(Buffer {
                    owner,
                    id,
                    handle,
                    bytes: descriptor.backing_bytes,
                });
                Ok(Response::scalar(id.0))
            }
            w::ACK if b.len() == 16 => {
                self.engine
                    .acknowledge_configure(owner, g::WindowId(u64_at(b, 0)), u64_at(b, 8))
                    .map_err(error)?;
                Ok(Response::empty())
            }
            w::ATTACH if b.len() == 24 => {
                let window = g::WindowId(u64_at(b, 0));
                let buffer = g::BufferId(u64_at(b, 8));
                self.engine
                    .attach(owner, window, buffer, u64_at(b, 16))
                    .map_err(error)?;
                self.attachments.retain(|a| a.window != window);
                self.attachments.push(Attachment {
                    owner,
                    window,
                    buffer,
                });
                Ok(Response::empty())
            }
            w::COMMIT if matches!(b.len(), 8 | 16 | 24) => {
                let window = g::WindowId(u64_at(b, 0));
                let info = self.engine.window_info(window).map_err(error)?;
                if info.owner != owner {
                    return Err(Code::Denied);
                }
                if info.state == g::WindowState::Closing {
                    return Err(Code::Closing);
                }
                if b.len() >= 16 && u64_at(b, 8) != info.epoch {
                    return Err(Code::StaleInstance);
                }
                let a = self
                    .attachments
                    .iter()
                    .find(|a| a.window == window && a.owner == owner)
                    .ok_or(Code::InvalidRequest)?;
                let buffer = self
                    .buffers
                    .iter()
                    .find(|v| v.id == a.buffer && v.owner == owner)
                    .ok_or(Code::InvalidHandle)?;
                let (buffer_handle, buffer_bytes) = (buffer.handle, buffer.bytes);
                // Band commit: only the producer's changed rows are copied,
                // validated and diffed into the existing trusted snapshot.
                // Without a compatible snapshot it degrades to a full capture.
                if b.len() == 24
                    && let Some((stride, height)) = self.engine.band_geometry(owner, window)
                {
                    let (top, rows) = (u32_at(b, 16), u32_at(b, 20));
                    if rows == 0 || top.checked_add(rows).is_none_or(|end| end > height) {
                        return Err(Code::InvalidRequest);
                    }
                    let bytes = rows as usize * stride;
                    if bytes <= door_window::capture::LARGE_BYTES {
                        let epoch = info.epoch;
                        return self.commit_band(owner, window, buffer_handle, epoch, top, bytes);
                    }
                }
                let replacing =
                    self.engine.snapshot_capacity(window) == door_window::capture::CELL_BYTES;
                let Some(mut pixels) = self.captures.capture(buffer_bytes, replacing) else {
                    let stats = HEAP.stats();
                    let _ = door_sys::write(
                        format!(
                            "DOOR_P12 CAPTURE RESOURCES requested={} used={} largest={}\n",
                            buffer_bytes,
                            stats.used,
                            HEAP.largest_free()
                        )
                        .as_bytes(),
                    );
                    return Err(Code::Resources);
                };
                let limit = door_abi::graphics::SECTION_COPY_BYTES;
                let first_capture = self.engine.snapshot_epoch(window) != Some(info.epoch);
                let (epoch, width, height) =
                    (info.epoch, info.content.width(), info.content.height());
                let capture = (|| {
                    for (i, chunk) in pixels.chunks_mut(limit).enumerate() {
                        let n =
                            door_sys::section_snapshot(buffer_handle, (i * limit) as u64, chunk)
                                .map_err(server::status)?;
                        if n != chunk.len() {
                            return Err(Code::Io);
                        }
                        if i % 2 == 1 {
                            self.input().map_err(server::status)?;
                            if self.cursor_dirty {
                                self.present_cursor().map_err(server::status)?;
                            }
                            let latest = self.engine.window_info(window).map_err(error)?;
                            if latest.state == g::WindowState::Closing {
                                return Err(Code::Closing);
                            }
                            if latest.epoch != epoch {
                                return Err(Code::StaleInstance);
                            }
                        }
                    }
                    self.engine
                        .commit_recycling(owner, window, &mut pixels)
                        .map_err(|e| {
                            if e == g::Error::InvalidPixels {
                                report_invalid_pixels(window, 0, width as usize * 4, &pixels);
                            }
                            error(e)
                        })
                })();
                self.captures.recycle(pixels);
                capture?;
                if first_capture
                    && let Some((_, app)) = self.app_classes.iter().find(|(who, _)| *who == owner)
                {
                    let _ = door_sys::write(format!("DOOR_P12 CAPTURED app={} width={width} height={height} epoch={epoch}\n", app.command()).as_bytes());
                }
                self.attachments.retain(|a| a.window != window);
                Ok(Response::empty())
            }
            w::UNREGISTER if b.len() == 8 => {
                let id = g::BufferId(u64_at(b, 0));
                self.engine.unregister_buffer(owner, id).map_err(error)?;
                if let Some(i) = self
                    .buffers
                    .iter()
                    .position(|v| v.id == id && v.owner == owner)
                {
                    let v = self.buffers.remove(i);
                    let _ = door_sys::close(v.handle);
                }
                Ok(Response::empty())
            }
            w::SET_OWNER
            | w::SET_ENABLED
            | w::MOVE_RESIZE
            | w::SET_SIZE_LIMITS
            | w::SET_FRAMELESS
            | w::SET_CAPTION_BUTTONS
            | w::SET_RESPONDING => {
                // Granted only to Windows-personality clients (HELLO).
                if !self.clients.iter().any(|c| c.owner == owner && c.win32) {
                    return Err(Code::Unsupported);
                }
                self.win32_request(owner, op, b, t)
            }
            w::DESTROY if b.len() == 8 => {
                let id = g::WindowId(u64_at(b, 0));
                self.engine
                    .destroy_window_recycling(owner, id, &mut |pixels| {
                        self.captures.recycle(pixels)
                    })
                    .map_err(error)?;
                self.attachments
                    .retain(|a| self.engine.window_info(a.window).is_ok());
                self.shell.dnd.window_gone(id.0);
                self.accessibility_window_gone(id.0);
                self.win32.forget(id.0);
                if self.shell.hang.prompt().is_some_and(|p| p.window == id.0) {
                    self.engine.invalidate();
                }
                self.shell.hang.forget_window(owner, id.0);
                Ok(Response::empty())
            }
            w::STATE if b.len() == 16 => {
                let id = g::WindowId(u64_at(b, 0));
                let previous = self.engine.window_info(id).map_err(error)?.epoch;
                match u64_at(b, 8) {
                    0 => self.engine.restore_window(owner, id),
                    1 => self
                        .engine
                        .request_state(owner, id, g::WindowState::Minimized),
                    2 => self
                        .engine
                        .request_state(owner, id, g::WindowState::Maximized),
                    3 if self
                        .clients
                        .iter()
                        .any(|c| c.owner == owner && c.multiwindow) =>
                    {
                        self.engine.activate_owned(owner, id)
                    }
                    _ => return Err(Code::InvalidRequest),
                }
                .map_err(error)?;
                if self
                    .engine
                    .window_info(id)
                    .is_ok_and(|w| w.epoch != previous)
                {
                    self.attachments.retain(|a| a.window != id);
                }
                Ok(Response::empty())
            }
            w::CONFIGURE if b.len() == 24 => {
                let id = g::WindowId(u64_at(b, 0));
                let previous = self.engine.window_info(id).map_err(error)?.epoch;
                let requested = rect(&b[8..])?;
                // Win32 size limits (CAP_WIN32) also bound client requests.
                let (cw, ch) = self
                    .win32
                    .limits(id.0)
                    .clamp(requested.width(), requested.height());
                let requested =
                    g::Rect::new(requested.x(), requested.y(), cw, ch).map_err(error)?;
                let epoch = self.engine.configure(owner, id, requested).map_err(error)?;
                if epoch != previous {
                    self.attachments.retain(|a| a.window != id);
                }
                Ok(Response::scalar(epoch))
            }
            w::FRAME if b.len() == 8 => {
                self.engine
                    .request_frame(owner, g::WindowId(u64_at(b, 0)))
                    .map_err(error)?;
                Ok(Response::empty())
            }
            w::POLL if b.is_empty() => {
                self.shell.hang.polled(owner);
                if let Some(event) = self.take_scale(owner) {
                    return Ok(Response::bytes(&event.encode()));
                }
                if let Some(event) = self.take_settings_navigation(owner) {
                    return Ok(Response::bytes(&event.encode()));
                }
                if let Some(event) = self.take_controls(owner) {
                    return Ok(Response::bytes(&event.encode()));
                }
                if let Some(event) = self.take_appearance(owner) {
                    return Ok(Response::bytes(&event.encode()));
                }
                if let Some(event) = self.take_search_event(owner) {
                    return Ok(Response::bytes(&event.encode()));
                }
                if let Some(event) = self.take_snip_event(owner) {
                    return Ok(Response::bytes(&event.encode()));
                }
                if let Some(event) = self
                    .take_native_event(owner)
                    .or_else(|| self.shell.text_input.take_event(owner))
                {
                    return Ok(Response::bytes(&event.encode()));
                }
                if let Some(event) = self.engine.poll_event(owner) {
                    return Ok(Response::bytes(&encode(event).encode()));
                }
                let event = self.take_win32_close(owner).ok_or(Code::WouldBlock)?;
                Ok(Response::bytes(&event.encode()))
            }
            w::SEARCH_QUERY_GET => self.search_query_get(owner, b),
            w::SNIP_BEGIN => self.snip_request(owner, b),
            w::MAGNIFY => self.magnify_request(owner, b),
            w::INJECT_KEY => self.inject_key_request(owner, b),
            w::SEARCH_RESULTS => self.search_results(owner, b),
            w::OPEN_DOCUMENT => self.open_document(owner, b),
            w::DOCUMENT_GET => self.document_get(owner, b),
            door_service_protocol::clipboard::FIRST..=door_service_protocol::clipboard::LAST => {
                self.clipboard_request(peer, owner, op, b, t)
            }
            door_service_protocol::dnd::FIRST..=door_service_protocol::dnd::LAST => {
                self.dnd_request(peer, owner, op, b)
            }
            door_service_protocol::text_input::FIRST..=door_service_protocol::text_input::LAST => {
                self.text_input_request(peer, owner, op, b)
            }
            door_service_protocol::accessibility::FIRST
                ..=door_service_protocol::accessibility::LAST => {
                self.accessibility_request(peer, owner, op, b, t)
            }
            _ => Err(Code::InvalidRequest),
        }
    }
    /// Copies `bytes` (whole rows from `top`) of the attached buffer into a
    /// private band, then lets the engine validate, diff and patch the
    /// trusted snapshot. Input keeps flowing between copy chunks, and a
    /// configure or close during the copy rejects the stale band.
    fn commit_band(
        &mut self,
        owner: g::Owner,
        window: g::WindowId,
        handle: Handle,
        epoch: u64,
        top: u32,
        bytes: usize,
    ) -> Result<Response, Code> {
        let mut band = Vec::new();
        let mut band = if band.try_reserve_exact(bytes).is_ok() {
            band.resize(bytes, 0);
            band
        } else {
            // At 4K the heap left beside composition can lack one contiguous
            // band (up to `LARGE_BYTES`); the capture pool's spare cell
            // serves the copy and goes back to the pool below. Refusing
            // instead left the window's later frames unpresented.
            let mut cell = self.captures.scratch().ok_or(Code::Resources)?;
            cell.resize(bytes, 0);
            cell
        };
        let result = self.copy_band(owner, window, handle, epoch, top, &mut band);
        self.captures.recycle(band);
        result
    }
    /// Copy the producer's rows `top..` into `band` (servicing input between
    /// chunks) and publish them into the window's snapshot.
    fn copy_band(
        &mut self,
        owner: g::Owner,
        window: g::WindowId,
        handle: Handle,
        epoch: u64,
        top: u32,
        band: &mut [u8],
    ) -> Result<Response, Code> {
        let stride = self
            .engine
            .band_geometry(owner, window)
            .ok_or(Code::InvalidRequest)?
            .0;
        let limit = door_abi::graphics::SECTION_COPY_BYTES;
        let start = top as usize * stride;
        for (i, chunk) in band.chunks_mut(limit).enumerate() {
            let n = door_sys::section_snapshot(handle, (start + i * limit) as u64, chunk)
                .map_err(server::status)?;
            if n != chunk.len() {
                return Err(Code::Io);
            }
            if i % 2 == 1 {
                self.input().map_err(server::status)?;
                if self.cursor_dirty {
                    self.present_cursor().map_err(server::status)?;
                }
                let latest = self.engine.window_info(window).map_err(error)?;
                if latest.state == g::WindowState::Closing {
                    return Err(Code::Closing);
                }
                if latest.epoch != epoch {
                    return Err(Code::StaleInstance);
                }
            }
        }
        self.engine
            .commit_band(owner, window, top, band)
            .map_err(|e| {
                if e == g::Error::InvalidPixels {
                    report_invalid_pixels(window, top, stride, band);
                }
                error(e)
            })?;
        self.attachments.retain(|a| a.window != window);
        Ok(Response::empty())
    }
    fn appearance(&mut self, op: u32, b: &[u8], t: &[Transfer]) -> Result<Response, Code> {
        // `policy::appearance_value` bounds the value (two themes, or
        // APPEARANCE_VALUES desktop words for the 18 pictures).
        if b.len() != 4 || !t.is_empty() {
            return Err(Code::InvalidRequest);
        }
        self.apply_appearance(
            door_window::policy::appearance_value(op, u32_at(b, 0), self.background)
                .ok_or(Code::InvalidRequest)?,
        )?;
        self.engine.set_theme(if self.dark {
            g::Theme::Dark
        } else {
            g::Theme::Light
        });
        Ok(Response::empty())
    }
    fn take_settings_navigation(&mut self, owner: g::Owner) -> Option<w::Event> {
        if !self
            .clients
            .iter()
            .any(|c| c.owner == owner && c.controls_events)
            || !self
                .app_classes
                .iter()
                .any(|(o, app)| *o == owner && *app == desktop::App::Settings)
        {
            return None;
        }
        Some(w::Event {
            kind: w::SETTINGS_NAVIGATE,
            a: self.settings_navigation.take()?,
            ..w::Event::default()
        })
    }
    fn take_controls(&mut self, owner: g::Owner) -> Option<w::Event> {
        let client = self.clients.iter_mut().find(|c| c.owner == owner)?;
        Some(w::Event {
            kind: w::CONTROLS_CHANGED,
            a: client.controls_pending.take()?,
            ..w::Event::default()
        })
    }
    fn desktop_controls(&mut self, b: &[u8], t: &[Transfer]) -> Result<Response, Code> {
        if b.len() != 4 || !t.is_empty() {
            return Err(Code::InvalidRequest);
        }
        let controls = door_service_protocol::desktop::Controls::from_bits(u32_at(b, 0))
            .ok_or(Code::InvalidRequest)?;
        if controls == self.controls {
            return Ok(Response::empty());
        }
        let output = self.engine.output_rect();
        let scale = self.engine.scale();
        let layout = door_window::taskbar::Layout::with_controls(
            scale.logical_len(output.width()),
            scale.logical_len(output.height()),
            controls,
        );
        let cache = door_window::taskbar::physical(layout.cache(), scale);
        let old_layout = self.taskbar_layout();
        if layout.bar() != old_layout.bar() {
            // Retain the high-water cache allocation. Alternating edges at4K
            // must not require two large underlays to coexist on the heap.
            let bytes = cache.width() as usize * cache.height() as usize * 4;
            self.overlay_underlay
                .try_reserve_exact(bytes.saturating_sub(self.overlay_underlay.len()))
                .map_err(|_| Code::Resources)?;
            let material = door_window::material::Taskbar::for_region(
                output.width(),
                output.height(),
                door_window::taskbar::physical(layout.bar(), scale),
            )
            .ok_or(Code::Resources)?;
            let old_epochs: Vec<_> = self.engine.windows().map(|w| (w.id, w.epoch)).collect();
            self.engine
                .reflow_work_area(door_window::taskbar::physical(layout.work_area(), scale))
                .map_err(error)?;
            self.attachments.retain(|a| {
                self.engine.window_info(a.window).is_ok_and(|w| {
                    old_epochs
                        .iter()
                        .any(|(id, epoch)| *id == a.window && *epoch == w.epoch)
                })
            });
            self.overlay_underlay.resize(bytes, 0);
            self.taskbar_material = material;
        }
        let contrast_changed = self.controls.contrast() != controls.contrast();
        // Accessibility > Text size: a new shell atlas (display x text), so
        // taskbar labels, Start, flyouts and menus relayout with it live.
        if controls.text_percent() != self.fonts.text_percent() {
            match door_window::display_scale::shell_fonts_text(
                self.engine.scale(),
                controls.text_percent(),
            ) {
                Some(fonts) => {
                    self.fonts = fonts;
                    door_window::paint::set_text_percent(self.fonts.text_percent());
                    self.close_menu();
                }
                None => {
                    let _ = door_sys::write(b"DOOR_P12 TEXT_SIZE atlas unavailable\n");
                }
            }
        }
        self.controls = controls;
        self.apply_visual_preferences();
        if contrast_changed {
            if self.refresh_wallpaper(self.background).is_err() {
                let _ = door_sys::write(b"DOOR_P12 CONTRAST wallpaper failed\n");
            }
            let appearance = self.client_appearance();
            for client in &mut self.clients {
                if client.appearance_events {
                    client.appearance_pending = Some(appearance);
                }
            }
        }
        let overlay_owned_focus = self.launcher || self.tray.open.is_some();
        self.launcher = false;
        self.tray.open = None;
        self.drag = None;
        self.caption_press = None;
        self.desktop_press = None;
        self.desktop_selection.cancel_gesture();
        self.icon_hover = None;
        if overlay_owned_focus {
            self.restore_shell_focus();
        } else {
            self.shell_focus = None;
        }
        self.icons_dirty = true;
        self.bar_dirty = true;
        self.engine.invalidate();
        for client in &mut self.clients {
            if client.controls_events {
                client.controls_pending = Some(controls.bits());
            }
        }
        let _ = door_sys::write(
            format!(
                "DOOR_P12 CONTROLS bits={} edge={:?} compact={} aligned_start={}\n",
                controls.bits(),
                controls.edge(),
                controls.small(),
                controls.start_aligned()
            )
            .as_bytes(),
        );
        Ok(Response::empty())
    }
    /// The appearance clients paint with. While a contrast theme is on they
    /// get light: their palettes map light-theme colours onto the theme's
    /// system colours (the stored mode is unchanged).
    fn client_appearance(&self) -> u32 {
        let dark = self.dark && self.controls.contrast().is_none();
        (self.background << 1) | u32::from(dark)
    }
    fn take_appearance(&mut self, owner: g::Owner) -> Option<w::Event> {
        let client = self
            .clients
            .iter_mut()
            .find(|client| client.owner == owner)?;
        let value = client.appearance_pending.take()?;
        Some(w::Event {
            kind: w::APPEARANCE_CHANGED,
            value: u64::from(value),
            a: door_service_protocol::desktop::Accent::event_word(self.accent_rgb),
            ..w::Event::default()
        })
    }
    /// The desktop background: the chosen picture, or the contrast theme's
    /// solid Window colour while one is on (as Windows contrast themes do).
    /// Personalization > Lock screen: the lock screen's own picture or
    /// colour, rendered once per lock (the desktop background needs none).
    pub(super) fn prepare_lock_backdrop(&mut self) {
        use door_window::preferences::{LockBackdrop, lock_backdrop};
        self.lock_pixels = Vec::new();
        if self.controls.contrast().is_some() {
            return;
        }
        match lock_backdrop(self.shell.prefs.lock_screen(), self.background) {
            LockBackdrop::Desktop => (),
            LockBackdrop::Solid(color) => {
                let mut pixels = Vec::new();
                if pixels.try_reserve_exact(self.wallpaper.len()).is_ok() {
                    pixels.extend(core::iter::repeat_n(color, self.wallpaper.len() / 4).flatten());
                    self.lock_pixels = pixels;
                }
            }
            LockBackdrop::Picture(index) => {
                let output = self.engine.output_rect();
                let (Some(source), Some(mut scratch)) =
                    (self.assets.background(index), self.captures.scratch())
                else {
                    return;
                };
                let mut pixels = Vec::new();
                let result = door_window::wallpaper::load_with_scratch(
                    source,
                    output.width(),
                    output.height(),
                    &mut pixels,
                    &mut scratch,
                );
                self.captures.recycle(scratch);
                if result.is_ok() && pixels.len() == self.wallpaper.len() {
                    self.lock_pixels = pixels;
                }
            }
        }
    }
    fn refresh_wallpaper(&mut self, background: u32) -> Result<(), Code> {
        if let Some(theme) = self.controls.contrast() {
            let w = theme.colors().window;
            for p in self.wallpaper.chunks_exact_mut(4) {
                p.copy_from_slice(&[w[2], w[1], w[0], 255]);
            }
        } else {
            let output = self.engine.output_rect();
            // Personalization > Background: kind, fit and colour.
            let word = door_service_protocol::preferences::Background::from_bits(
                self.shell
                    .prefs
                    .get(door_service_protocol::preferences::BACKGROUND),
            )
            .unwrap_or(door_service_protocol::preferences::Background::DEFAULT);
            let rgb = if word.color == 0 {
                0x0063B1
            } else {
                word.color
            };
            let color = [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8];
            if word.kind == 1 {
                let length = output.width() as usize * output.height() as usize * 4;
                self.wallpaper
                    .try_reserve(length.saturating_sub(self.wallpaper.len()))
                    .map_err(|_| Code::Resources)?;
                self.wallpaper.resize(length, 0);
                for p in self.wallpaper.chunks_exact_mut(4) {
                    p.copy_from_slice(&[color[2], color[1], color[0], 255]);
                }
                return self.wallpaper_applied();
            }
            let shown = if word.kind == 2 {
                self.shell.slide
            } else {
                background
            };
            let mut scratch = self.captures.scratch().ok_or(Code::Resources)?;
            let result = door_window::wallpaper::load_fitted(
                self.assets.background(shown).ok_or(Code::InvalidRequest)?,
                output.width(),
                output.height(),
                &mut self.wallpaper,
                &mut scratch,
                door_window::wallpaper::Fit::from_word(word.fit),
                color,
            );
            self.captures.recycle(scratch);
            result.map_err(|_| Code::Resources)?;
        }
        self.wallpaper_applied()
    }
    /// The wallpaper pixels changed: material and an Automatic accent follow.
    fn wallpaper_applied(&mut self) -> Result<(), Code> {
        self.engine
            .set_wallpaper_material(&self.wallpaper)
            .map_err(error)?;
        // An Automatic accent follows the new background.
        if self.accent == door_service_protocol::desktop::Accent::AUTOMATIC {
            self.apply_accent(false);
        }
        self.icons_dirty = true;
        self.engine.invalidate();
        Ok(())
    }
    /// Background slideshow: the next picture when its interval is up.
    pub(super) fn slideshow_tick(&mut self, now: u64) {
        use door_service_protocol::preferences::Background;
        let word = Background::from_bits(
            self.shell
                .prefs
                .get(door_service_protocol::preferences::BACKGROUND),
        )
        .unwrap_or(Background::DEFAULT);
        if word.kind != 2 || self.controls.contrast().is_some() {
            self.shell.slide_due = 0;
            return;
        }
        let period = u64::from(Background::MINUTES[word.interval as usize & 3])
            * door_window::preferences::TICKS_PER_MINUTE;
        if self.shell.slide_due == 0 {
            self.shell.slide_due = now + period;
            return;
        }
        if now < self.shell.slide_due {
            return;
        }
        self.shell.slide_due = now + period;
        // The slideshow steps through the album (theme folder) of the
        // chosen picture, as a theme's slideshow does on Windows.
        use door_initfs::desktop_assets::{WALLPAPERS, next_in_album};
        let members = WALLPAPERS
            .get(self.shell.slide as usize)
            .map_or(&[0u32][..], |w| w.album.members());
        self.shell.slide = if word.shuffle && members.len() > 2 {
            // A cheap deterministic shuffle that never repeats the picture.
            let at = members
                .iter()
                .position(|m| *m == self.shell.slide)
                .unwrap_or(0);
            let step = 1 + now as usize % (members.len() - 1);
            members[(at + step) % members.len()]
        } else {
            next_in_album(self.shell.slide)
        };
        let _ = self.refresh_wallpaper(self.background);
        let _ = door_sys::write(
            format!("DOOR_P14 SLIDESHOW picture={}\n", self.shell.slide).as_bytes(),
        );
    }
    fn apply_appearance(&mut self, value: u32) -> Result<(), Code> {
        let background = value >> 1;
        if background != self.background {
            // A new picture (or theme) restarts the slideshow from it, as
            // choosing one does on Windows; otherwise a slideshow kept
            // showing the previous album's picture.
            self.shell.slide = background;
            self.shell.slide_due = 0;
            if self.controls.contrast().is_none() {
                self.refresh_wallpaper(background)?;
            }
            self.background = background;
        }
        self.dark = value & 1 != 0;
        let client = self.client_appearance();
        for c in &mut self.clients {
            if c.appearance_events {
                c.appearance_pending = Some(client);
            }
        }
        let _ = door_sys::write(format!("DOOR_P12 APPEARANCE applied={}\n", value).as_bytes());
        Ok(())
    }
    pub(super) fn revoke(&mut self, owner: g::Owner) {
        self.magnifier_revoked(owner);
        let win32 = self.shell.hang.is_client(owner);
        if win32 {
            self.shell.hang.forget(owner);
            self.engine.invalidate();
        }
        self.revoke_resources(owner);
        // Win32 owners (bounded diagnostics): what the release left behind,
        // all zero unless something leaked.
        if win32 {
            let windows: Vec<g::WindowId> = self
                .engine
                .windows()
                .filter(|w| w.owner == owner)
                .map(|w| w.id)
                .collect();
            let surfaces = windows
                .iter()
                .filter(|id| self.engine.has_snapshot(**id))
                .count();
            let buffers = self.buffers.iter().filter(|b| b.owner == owner);
            let bytes = buffers.clone().map(|b| b.bytes).sum::<usize>()
                + windows
                    .iter()
                    .map(|id| self.engine.snapshot_capacity(*id))
                    .sum::<usize>();
            let _ = door_sys::write(
                alloc::format!(
                    "DOOR_P13 OWNER_RELEASED pid={} windows={} surfaces={surfaces} buffers={} attachments={} bytes={bytes}\n",
                    owner.pid,
                    windows.len(),
                    buffers.count(),
                    self.attachments.iter().filter(|a| a.owner == owner).count(),
                )
                .as_bytes(),
            );
        }
    }
    fn revoke_resources(&mut self, owner: g::Owner) {
        self.shell_revoked(owner);
        self.elevation_revoked(owner.pid);
        if self
            .shell_focus
            .is_some_and(|saved| saved.belongs_to(owner))
        {
            self.shell_focus = None;
        }
        self.closing.retain(|v| v.0 != owner);
        if self.caption_press.is_some_and(|(id, _)| {
            self.engine
                .window_info(id)
                .is_ok_and(|info| info.owner == owner)
        }) {
            self.caption_press = None;
        }
        let gone: Vec<u64> = self
            .engine
            .windows()
            .filter(|w| w.owner == owner)
            .map(|w| w.id.0)
            .collect();
        for id in gone {
            self.win32.forget(id);
        }
        if self.win32_press.is_some_and(|p| {
            self.engine
                .window_info(g::WindowId(p.window))
                .is_ok_and(|w| w.owner == owner)
        }) {
            self.win32_press = None;
        }
        self.engine
            .disconnect_recycling(owner, &mut |pixels| self.captures.recycle(pixels));
        self.attachments.retain(|a| a.owner != owner);
        self.buffers.retain(|v| {
            if v.owner == owner {
                let _ = door_sys::close(v.handle);
                false
            } else {
                true
            }
        });
        self.clients.retain(|v| v.owner != owner);
        // Clipboard data stays (copy-at-set); its drags and drops end.
        self.native_ui_exited(owner);
        self.shell.text_input.revoke(owner);
        if self.drag.is_some_and(|d| d.owner == owner) {
            self.drag = None;
        }
    }
    pub(super) fn evict(&mut self, owner: g::Owner) {
        let channel = self
            .clients
            .iter()
            .find(|v| v.owner == owner)
            .map(|v| v.channel);
        self.revoke(owner);
        if let Some(channel) = channel {
            self.clients.push(Client {
                manifest: 0,
                multiwindow: false,
                new_windows: 0,
                channel,
                owner,
                revoked: true,
                transient_events: false,
                appearance_events: false,
                appearance_pending: None,
                controls_events: false,
                controls_pending: None,
                wheel_events: false,
                scale_events: false,
                scale_pending: false,
                win32: false,
            });
        }
    }
    /// A close the user (or the client itself) asked for.
    pub(super) fn close(&mut self, owner: g::Owner, id: g::WindowId) -> g::Result<()> {
        self.close_for(owner, id, self.shutdown.is_some())
    }
    /// `shutdown`: the shutdown/sign-out path, which keeps the native close
    /// (Closing state, bounded containment) for every client. Otherwise a
    /// Win32 window gets a queued CLOSE with no deadline, or the "not
    /// responding" prompt when it is a ghost.
    pub(super) fn close_for(
        &mut self,
        owner: g::Owner,
        id: g::WindowId,
        shutdown: bool,
    ) -> g::Result<()> {
        // A rejected foreign request must not mutate a trusted pointer gesture.
        let info = self.engine.window_info(id)?;
        if info.owner != owner {
            return Err(g::Error::WrongOwner);
        }
        // Input can wait behind composition. An old hardware timestamp must
        // never spend a client's close budget before we service the request.
        let now = door_sys::time().map_err(|_| g::Error::InvalidState)?;
        if !shutdown && self.shell.hang.is_client(owner) {
            let title = alloc::string::String::from(info.title);
            if self.drag.is_some_and(|drag| drag.id == id) {
                self.drag = None;
            }
            if self.caption_press.is_some_and(|press| press.0 == id) {
                self.caption_press = None;
            }
            match self.shell.hang.request_close(owner, id.0, &title) {
                door_window::not_responding::Close::Queued => {
                    let _ = door_sys::write(
                        alloc::format!("DOOR_P12 CLOSE REQUEST pid={} at={now}\n", owner.pid)
                            .as_bytes(),
                    );
                }
                door_window::not_responding::Close::Prompt => self.prompt_opened(),
            }
            return Ok(());
        }
        // An app answering its shutdown prompt with Save/Don't save closes
        // itself: that continues the shutdown, it is not a new request.
        if !self.closing.iter().any(|v| v.0 == owner && v.1 == id)
            && !self.shell.shutdown_gate.waiting_on(owner)
        {
            let _ = door_sys::write(
                alloc::format!("DOOR_P12 CLOSE REQUEST pid={} at={now}\n", owner.pid).as_bytes(),
            );
        }
        let output = self.engine.output_rect();
        let grace = if u64::from(output.width()) * u64::from(output.height()) > 1920 * 1200 {
            1000
        } else {
            300
        };
        if self.drag.is_some_and(|drag| drag.id == id) {
            self.drag = None;
        }
        if self.caption_press.is_some_and(|press| press.0 == id) {
            self.caption_press = None;
        }
        door_window::policy::request_close_with_grace(
            &mut self.engine,
            &mut self.closing,
            owner,
            id,
            now,
            grace,
        )
    }
    /// The next queued Win32 CLOSE event for `owner`'s POLL.
    fn take_win32_close(&mut self, owner: g::Owner) -> Option<w::Event> {
        let window = self.shell.hang.take_close(owner)?;
        Some(w::Event {
            kind: w::CLOSE,
            window,
            ..w::Event::default()
        })
    }
    /// The logical area the "not responding" prompt is centred in.
    pub(super) fn prompt_area(&self) -> g::Rect {
        self.taskbar_layout().work_area()
    }
    /// The prompt opened: record its physical buttons for runners.
    fn prompt_opened(&mut self) {
        let Some(p) = self.shell.hang.prompt() else {
            return;
        };
        let scale = self.engine.scale();
        let geometry = door_window::not_responding::geometry(self.prompt_area());
        let r = |r: g::Rect| {
            let r = door_window::paint::physical(r, scale);
            alloc::format!("{},{},{},{}", r.x(), r.y(), r.width(), r.height())
        };
        let _ = door_sys::write(
            alloc::format!(
                "DOOR_P13 WIN32 NOT_RESPONDING_PROMPT pid={} window={} end_task={} wait={}\n",
                p.owner.pid,
                p.window,
                r(geometry.end_task),
                r(geometry.wait),
            )
            .as_bytes(),
        );
        self.engine.invalidate();
    }
    /// A window became a ghost (`hung`) or responds again.
    pub(super) fn ghost_changed(&mut self, owner: g::Owner, window: u64, hung: bool) {
        let _ = door_sys::write(
            alloc::format!(
                "DOOR_P13 WIN32 {} pid={} window={window}\n",
                if hung { "NOT_RESPONDING" } else { "RESPONDING" },
                owner.pid
            )
            .as_bytes(),
        );
        self.engine.invalidate();
    }
    /// Once per iteration: the five-second hung rule for Win32 clients.
    pub(super) fn track_responsiveness(&mut self, now: u64) {
        let owners: Vec<g::Owner> = self.shell.hang.owners().collect();
        for owner in owners {
            let pending = self.engine.pending_events(owner) != 0;
            let Some(change) = self.shell.hang.tick(owner, pending, now) else {
                continue;
            };
            let hung = change == door_window::not_responding::Change::Hung;
            let windows: Vec<u64> = self
                .engine
                .windows()
                .filter(|w| w.owner == owner)
                .map(|w| w.id.0)
                .collect();
            for window in windows {
                self.ghost_changed(owner, window, hung);
            }
            self.engine.invalidate();
        }
    }
    /// Taskbar "End task" (System > Advanced): retire each owner through
    /// Init, as the not-responding prompt does.
    pub(super) fn end_task(&mut self, mut owners: Vec<g::Owner>) {
        owners.sort_by_key(|o| o.pid);
        owners.dedup_by_key(|o| o.pid);
        let now = door_sys::time().unwrap_or(0);
        for owner in owners {
            let _ =
                door_sys::write(alloc::format!("DOOR_P14 END_TASK pid={}\n", owner.pid).as_bytes());
            self.terminations.push(owner, now);
            self.evict(owner);
        }
        self.engine.invalidate();
    }
    /// End task or Wait in the "not responding" prompt.
    pub(super) fn not_responding_answer(
        &mut self,
        (owner, window, answer): (g::Owner, u64, door_window::not_responding::Answer),
    ) {
        let end = answer == door_window::not_responding::Answer::EndTask;
        let _ = door_sys::write(
            alloc::format!(
                "DOOR_P13 WIN32 {} pid={} window={window}\n",
                if end { "END_TASK" } else { "WAIT" },
                owner.pid
            )
            .as_bytes(),
        );
        if end {
            // The only path that terminates a not-responding Win32 client
            // outside sign-out and shutdown: Init's authenticated retirement.
            let now = door_sys::time().unwrap_or(0);
            self.terminations.push(owner, now);
            self.evict(owner);
        }
        self.engine.invalidate();
    }
    pub(super) fn expire_closed(&mut self, now: u64) {
        self.closing
            .retain(|v| self.engine.window_info(v.1).is_ok());
        door_window::policy::retry_closes(&mut self.engine, &self.closing);
        while let Some(owner) = self.closing.iter().find(|v| now >= v.2).map(|v| v.0) {
            self.terminations.push(owner, now);
            self.evict(owner);
            let _ = door_sys::write(b"DOOR_P11 CLOSE TIMEOUT revoked\n");
        }
        while let Some(owner) = self.terminations.due(now) {
            let result = door_service_client::Init::connect(self.registry)
                .and_then(|mut init| init.terminate_window_client(owner.pid));
            let accepted = matches!(
                result,
                Ok(()) | Err(door_service_client::Error::Service(Code::NotFound))
            );
            self.terminations.complete(owner, now, accepted);
        }
    }
}
impl Handler for Desktop {
    fn request(
        &mut self,
        ch: Handle,
        peer: Identity,
        op: u32,
        b: &[u8],
        t: &[Transfer],
    ) -> Response {
        self.request_inner(ch, peer, op, b, t)
            .unwrap_or_else(Response::error)
    }
    fn disconnected(&mut self, ch: Handle) {
        if let Some(owner) = self
            .clients
            .iter()
            .find(|v| v.channel == ch)
            .map(|v| v.owner)
        {
            self.revoke(owner)
        }
    }
    fn shutdown(&mut self) -> Result<(), Status> {
        self.report_motion(door_window::telemetry::Cutoff::ServiceStop);
        while let Some(owner) = self.clients.first().map(|v| v.owner) {
            self.revoke(owner)
        }
        door_sys::display_release(self.display)
    }
}
fn wr(r: g::Rect) -> w::Rect {
    w::Rect {
        x: r.x(),
        y: r.y(),
        width: r.width(),
        height: r.height(),
    }
}
fn encode(e: g::Event) -> w::Event {
    let mut out = w::Event::default();
    match e {
        g::Event::DismissTransients { window } => {
            out.kind = w::DISMISS_TRANSIENTS;
            out.window = window.0;
        }

        g::Event::Configure {
            window,
            epoch,
            outer,
            content_width,
            content_height,
        } => {
            out.kind = w::CONFIGURED;
            out.window = window.0;
            out.value = epoch;
            out.rect = wr(outer);
            out.a = content_width;
            out.b = content_height;
        }
        g::Event::BufferReleased { buffer } => {
            out.kind = w::RELEASED;
            out.value = buffer.0;
        }
        g::Event::Input {
            window,
            input,
            timestamp,
        } => {
            out.kind = w::INPUT;
            out.window = window.0;
            out.timestamp = timestamp;
            match input {
                g::Input::Key { code, down } => {
                    out.detail = 1;
                    out.a = code;
                    out.b = down as u32
                }
                g::Input::PointerMove { x, y } => {
                    out.detail = 2;
                    out.rect.x = x;
                    out.rect.y = y;
                }
                g::Input::PointerButton { button, down, x, y } => {
                    out.detail = 3;
                    out.rect.x = x;
                    out.rect.y = y;
                    out.a = button as u32;
                    out.b = down as u32;
                }
                g::Input::Wheel { x, y, delta } => {
                    out.detail = w::INPUT_WHEEL;
                    out.rect.x = x;
                    out.rect.y = y;
                    out.a = delta as u32;
                }
            }
        }
        g::Event::Focus { window, focused } => {
            out.kind = w::FOCUS;
            out.window = window.0;
            out.a = focused as u32;
        }
        g::Event::CaptureLost { window } => {
            out.kind = w::CAPTURE_LOST;
            out.window = window.0;
        }
        g::Event::CloseRequested { window } => {
            out.kind = w::CLOSE;
            out.window = window.0;
        }
        g::Event::Frame { window, serial } => {
            out.kind = w::FRAME_READY;
            out.window = window.0;
            out.value = serial;
        }
        g::Event::Presentation {
            window,
            serial,
            timestamp,
            status,
        } => {
            out.kind = w::PRESENTED;
            out.window = window.0;
            out.value = serial;
            out.timestamp = timestamp;
            out.a = matches!(status, g::PresentationStatus::CpuTransferred) as u32;
        }
    }
    out
}

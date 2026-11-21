//! `door-settings`: Settings (Workspace manifest, settings mode).
#![no_std]
#![no_main]
extern crate alloc;
mod native;
// Crate-root names the application modules import with `use super::*`.
#[allow(unused_imports)]
use alloc::{boxed::Box, format, string::String};
#[allow(unused_imports)]
use door_abi::platform::{StartupV2, inherited, mode};
#[allow(unused_imports)]
use door_graphics_text::FontAtlas;
#[allow(unused_imports)]
use door_service_client::{Error, Storage, window::WindowClient};
#[allow(unused_imports)]
use door_service_protocol::{Code, window as w};
use door_workspace::runtime::WindowOptions;
#[allow(unused_imports)]
use door_workspace::{icons, menu, ui};
use native::Hosted;
#[global_allocator]
static HEAP: door_rt::HeapAllocator = door_rt::HeapAllocator::new();
door_rt::entry_v2!(main);
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    door_rt::panic()
}
mod settings;
fn main(startup: StartupV2) -> i64 {
    native::run::<App>(&HEAP, startup)
}
/// Settings plus its close deferral: a theme or desktop-controls request
/// still in flight is applied before the window closes.
struct App {
    settings: Box<settings::Settings>,
    deferred_close: bool,
    /// Service registry (the notification service for its preference words).
    registry: u64,
    /// This window is the context-menu customizer (Personalization >
    /// Context menu > Customize), not the Settings page.
    customizer: Option<Box<settings::menu_custom::Customizer>>,
    uid: u32,
    /// door.network (Network & internet), connected on first use; the next
    /// status poll and reconnect attempt (scheduler ticks).
    network: Option<door_service_client::Network>,
    net_poll: u64,
    /// When Settings > VPN next reads the connections.
    vpn_poll: u64,
    net_retry: u64,
    /// door.audio while System > Sound shows (status once a second, the
    /// Test tone).
    audio: Option<door_service_client::audio::Audio>,
    audio_poll: u64,
    audio_retry: u64,
}
impl Hosted for App {
    fn make(startup: &StartupV2, storage: &mut Storage, _fresh: bool) -> Result<Self, Error> {
        Ok(Self {
            settings: Box::new(settings::Settings::new(storage, startup)?),
            deferred_close: false,
            registry: startup.base.inherited[door_abi::platform::inherited::REGISTRY].handle,
            customizer: None,
            uid: startup.identity.uid,
            network: None,
            net_poll: 0,
            vpn_poll: 0,
            net_retry: 0,
            audio: None,
            audio_poll: 0,
            audio_retry: 0,
        })
    }
    /// Settings commits through its own registry (`Settings::watch`).
    fn observes_settings(&self) -> bool {
        false
    }
    fn title(&self) -> &str {
        match &self.customizer {
            Some(c) => c.title(),
            None => "Settings",
        }
    }
    fn options(&self) -> WindowOptions {
        if self.customizer.is_some() {
            let (width, height) = settings::menu_custom::WINDOW;
            return WindowOptions {
                width,
                height,
                caption_height: 48,
                caption_left: 0,
                centered: true,
            };
        }
        WindowOptions {
            width: 1129,
            height: 787,
            caption_height: 48,
            caption_left: 0,
            centered: true,
        }
    }
    fn caption(&self) -> (u32, u32) {
        (0, 1)
    }
    fn mica(&self) -> bool {
        self.customizer.is_none()
    }
    fn configure(&mut self, width: u32, height: u32, _fonts: &FontAtlas) {
        match self.customizer.as_mut() {
            Some(c) => c.configure(width, height),
            None => self.settings.configure(width, height),
        }
    }
    fn draw(&self, view: &mut ui::View<'_>) {
        match &self.customizer {
            Some(c) => c.draw(view),
            None => self.settings.draw(view),
        }
    }
    fn semantics(&self, s: &mut door_workspace::semantics::Semantics, _fonts: &FontAtlas) {
        if self.customizer.is_none() {
            self.settings.semantics(s)
        }
    }
    fn target(&self, x: i32, y: i32, _fonts: &FontAtlas) -> Option<u32> {
        match &self.customizer {
            Some(c) => c.target(x, y),
            None => self.settings.target(x, y),
        }
    }
    fn click(
        &mut self,
        x: i32,
        y: i32,
        _size: (u32, u32),
        storage: &mut Storage,
        _fonts: &FontAtlas,
    ) -> Result<bool, Error> {
        match self.customizer.as_mut() {
            Some(c) => c.click(x, y, storage),
            None => self.settings.click(x, y, storage),
        }
    }
    fn key(&mut self, code: u32, storage: &mut Storage, _fonts: &FontAtlas) -> Result<bool, Error> {
        match self.customizer.as_mut() {
            Some(c) => c.key(code, storage),
            None => self.settings.key(code, storage),
        }
    }
    fn menu_open(&self) -> bool {
        self.customizer.is_none() && self.settings.menu_open()
    }
    fn dismiss_menu(&mut self) {
        self.settings.dismiss_menu()
    }
    fn hover_menu(&mut self, x: i32, y: i32) -> bool {
        match self.customizer.as_mut() {
            Some(c) => c.pointer_motion(x, y),
            None => self.settings.hover_menu(x, y),
        }
    }
    fn pointer_down(&mut self, x: i32, y: i32, _fonts: &FontAtlas) -> bool {
        match self.customizer.as_mut() {
            Some(c) => c.pointer_down(x, y),
            None => self.settings.pointer_down(x, y),
        }
    }
    fn pointer_motion(&mut self, x: i32, y: i32, _fonts: &FontAtlas) -> bool {
        match self.customizer.as_mut() {
            Some(c) => c.pointer_motion(x, y),
            None => self.settings.pointer_motion(x, y),
        }
    }
    fn pointer_up(&mut self) {
        match self.customizer.as_mut() {
            Some(c) => c.release_pending(),
            None => self.settings.cancel_drag(),
        }
    }
    fn wheel(&mut self, x: i32, y: i32, notches: i32, _fonts: &FontAtlas) -> bool {
        match self.customizer.as_mut() {
            Some(c) => c.wheel(x, y, notches),
            None => self.settings.wheel(x, y, notches),
        }
    }
    fn take_new_window(&mut self) -> bool {
        core::mem::take(&mut self.settings.open_customizer)
    }
    fn prepare_window(&self, window: &mut Self, storage: &mut Storage) -> Result<(), Error> {
        window.customizer = Some(Box::new(settings::menu_custom::Customizer::new(
            storage,
            self.uid,
            self.settings.controls(),
        )));
        Ok(())
    }
    fn closing(&mut self, _storage: &mut Storage) -> Result<bool, Error> {
        if self.customizer.is_some() {
            return Ok(true);
        }
        if self.settings.has_pending_request() {
            self.deferred_close = true;
            Ok(false)
        } else {
            Ok(true)
        }
    }
    fn wants_close(&mut self) -> bool {
        if let Some(c) = self.customizer.as_mut() {
            return core::mem::take(&mut c.close);
        }
        if self.deferred_close && !self.settings.has_pending_request() {
            self.deferred_close = false;
            true
        } else {
            false
        }
    }
    fn tick(&mut self, storage: &mut Storage, now: u64) -> bool {
        if let Some(c) = self.customizer.as_mut() {
            // Menu style: applied by `update` like the Settings page's controls.
            if let Some(controls) = c.controls_request.take() {
                self.settings.pending_controls = Some(controls);
            }
            return c.tick(storage);
        }
        // Smooth wheel scrolling advances on elapsed scheduler ticks.
        self.settings.tick(now)
    }
    fn display(&mut self, width: u32, height: u32, scale: u32) {
        self.settings.display(width, height, scale)
    }
    fn event(&mut self, e: w::Event) -> Result<bool, Error> {
        match e.kind {
            w::CONTROLS_CHANGED => {
                let c = door_service_protocol::desktop::Controls::from_bits(e.a)
                    .ok_or(Error::Malformed)?;
                if let Some(customizer) = self.customizer.as_mut() {
                    customizer.controls_changed(c);
                }
                self.settings.controls_changed(c)
            }
            w::SETTINGS_NAVIGATE => self.settings.navigate_requested(e.a)?,
            w::APPEARANCE_CHANGED => self.settings.appearance_changed(e.value as u32),
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn update(&mut self, storage: &mut Storage, c: &mut WindowClient) -> Result<bool, Error> {
        let registry = self.registry;
        let a = &mut self.settings;
        // Pages reached by a deep link read their providers here.
        let loaded = a.ensure_providers(storage);
        if let Some(command) = a.pending_launch.take() {
            c.launch_app(command as u32)?;
        }
        // A firewall change: the window server asks an administrator.
        if let Some(body) = a.pending_firewall.take() {
            let result = c.firewall_change(&body);
            let _ = door_sys::write(
                format!(
                    "DOOR_P15 SETTINGS FIREWALL CHANGE kind={} prompt={}\n",
                    body[0],
                    u8::from(result.is_ok())
                )
                .as_bytes(),
            );
            if matches!(result, Err(Error::Service(Code::WouldBlock))) {
                a.pending_firewall = Some(body);
            }
        }
        let programs = a.programs_tick(storage, c, door_sys::time().unwrap_or(0));
        if let Some(v) = a.pending_theme {
            c.set_appearance(v)?;
            a.pending_theme = None;
        }
        if let Some(v) = a.pending_controls {
            match c.set_desktop_controls(v) {
                Ok(()) => {
                    a.pending_controls = None;
                    // The window server applied the request before
                    // answering; guest scenarios wait for this marker.
                    let _ = door_sys::write(
                        format!("DOOR_P12 SETTINGS APPLIED controls={}\n", v.bits()).as_bytes(),
                    );
                }
                Err(Error::Service(Code::WouldBlock)) => (),
                Err(e) => return Err(e),
            }
        }
        // System > Display > Scale: the window server's report (recommended
        // and offered scales), then live apply of a chosen scale; only an
        // accepted scale is persisted. A server without live scale changes
        // persists it for the next sign-in ("Sign out to apply").
        let mut redraw = loaded || programs;
        // Network & internet: the network service's status while a page
        // shows it (once a second), and configuration changes.
        if a.wants_network() || a.pending_net.is_some() {
            let now = door_sys::time().unwrap_or(0);
            if self.network.is_none() && now >= self.net_retry {
                match door_service_client::Network::connect(registry) {
                    Ok(n) => self.network = Some(n),
                    Err(_) => self.net_retry = now + 200,
                }
            }
            if let Some(n) = self.network.as_mut() {
                if let Some(config) = a.pending_net.take() {
                    if n.configure(&config).is_ok() {
                        let _ = door_sys::write(
                            format!(
                                "DOOR_P15 SETTINGS NETWORK enabled={}
",
                                u8::from(config.enabled)
                            )
                            .as_bytes(),
                        );
                    }
                    self.net_poll = 0;
                }
                // VPN: the page's requests, in order, then (on the page)
                // the user's connections once a second.
                let requests = core::mem::take(&mut a.pending_vpn);
                let acted = !requests.is_empty();
                for op in requests {
                    use settings::VpnOp;
                    let (verb, name, result) = match &op {
                        VpnOp::Save(name, text) => ("SAVE", name, n.vpn_save(self.uid, name, text)),
                        VpnOp::Connect(name) => ("CONNECT", name, n.vpn_connect(self.uid, name)),
                        VpnOp::Disconnect(name) => {
                            ("DISCONNECT", name, n.vpn_disconnect(self.uid, name))
                        }
                        VpnOp::Remove(name) => ("REMOVE", name, n.vpn_remove(self.uid, name)),
                    };
                    let _ = door_sys::write(
                        format!(
                            "DOOR_P15 SETTINGS VPN {verb} {name} ok={}\n",
                            u8::from(result.is_ok())
                        )
                        .as_bytes(),
                    );
                    // The phonebook keeps saved connections across restarts.
                    let kept = match (&op, &result) {
                        (VpnOp::Save(name, text), Ok(())) => Some((name, Some(text.as_str()))),
                        (VpnOp::Remove(name), Ok(())) => Some((name, None)),
                        _ => None,
                    };
                    if let Some((name, text)) = kept {
                        let home = format!("/users/{}", self.uid);
                        let stored =
                            door_workspace::phonebook::store(storage, &home, name, text).is_ok();
                        let _ = door_sys::write(
                            format!(
                                "DOOR_P15 SETTINGS VPN PHONEBOOK {name} ok={}\n",
                                u8::from(stored)
                            )
                            .as_bytes(),
                        );
                    }
                }
                if a.wants_vpn() && (acted || now >= self.vpn_poll) {
                    self.vpn_poll = now + 100;
                    let mut rows = alloc::vec::Vec::new();
                    let mut entries = alloc::vec::Vec::new();
                    for index in 0..=u8::MAX {
                        let Ok(Some(e)) = n.vpn_entry(self.uid, index) else {
                            break;
                        };
                        rows.push(vpn_row(&e));
                        entries.push(e);
                    }
                    let shown: alloc::vec::Vec<String> =
                        rows.iter().map(|r| format!("{}: {}", r.0, r.1)).collect();
                    let changed_rows = a.set_vpns(entries, rows);
                    if changed_rows {
                        redraw = true;
                        let _ = door_sys::write(
                            format!("DOOR_P15 SETTINGS VPN LIST [{}]\n", shown.join("; "))
                                .as_bytes(),
                        );
                    }
                }
                if now >= self.net_poll {
                    self.net_poll = now + 100;
                    match n.status() {
                        Ok(status) => {
                            if a.net != Some(status) {
                                a.net = Some(status);
                                redraw = true;
                            }
                            let v6 = n.status6().ok();
                            if a.net6 != v6 {
                                a.net6 = v6;
                                redraw = true;
                            }
                            // Firewall & network protection: the policy
                            // door.network enforces.
                            if a.wants_firewall()
                                && let Ok((profiles, _, rules, _)) = n.firewall_state()
                                && a.firewall != profiles
                            {
                                a.firewall = profiles;
                                redraw = true;
                                let _ = door_sys::write(
                                    format!(
                                        "DOOR_P15 SETTINGS FIREWALL STATE {:#x},{:#x},{:#x} rules={rules}\n",
                                        profiles[0], profiles[1], profiles[2]
                                    )
                                    .as_bytes(),
                                );
                            }
                        }
                        Err(_) => {
                            self.network = None;
                            a.net = None;
                            a.net6 = None;
                            redraw = true;
                        }
                    }
                }
            } else if a.net.take().is_some() {
                redraw = true;
            }
        }
        // System > Sound: the sound service's status while the page shows
        // (once a second) and the Test tone; a released slider is stored.
        if let Some((id, value)) = a.unsaved_pref.take()
            && let Some(setting) = door_service_client::Setting::from_id(id)
            && storage.set_setting(setting, value).is_ok()
        {
            let _ = storage.flush();
        }
        if a.wants_audio() || a.pending_test {
            let now = door_sys::time().unwrap_or(0);
            if self.audio.is_none() && now >= self.audio_retry {
                match door_service_client::audio::Audio::connect(registry) {
                    Ok(link) => self.audio = Some(link),
                    Err(_) => self.audio_retry = now + 200,
                }
            }
            if let Some(link) = self.audio.as_mut() {
                if core::mem::take(&mut a.pending_test) {
                    let _ = link.test_tone();
                    self.audio_poll = 0;
                }
                if now >= self.audio_poll {
                    self.audio_poll = now + 100;
                    if a.wants_capture() {
                        let status = link.capture_status().ok();
                        if a.set_capture(status) {
                            redraw = true;
                        }
                    }
                    match link.status() {
                        Ok(status) => {
                            if a.audio != Some(status) {
                                a.audio = Some(status);
                                redraw = true;
                            }
                        }
                        Err(_) => {
                            self.audio = None;
                            a.audio = None;
                            redraw = true;
                        }
                    }
                }
            } else {
                a.pending_test = false;
                if a.audio.take().is_some() {
                    redraw = true;
                }
            }
        }
        // Time & language > Typing: the keyboard list applies live through
        // door.text-input (saved first, so a refusal still takes effect at
        // the next sign-in); the page reads the live state on each visit.
        // Date & time / Regional format: the taskbar clock follows at once
        // (the settings were saved first, so a refusal applies at sign-in).
        if let Some((zone, region)) = a.pending_locale {
            match c.set_time_locale(zone, region) {
                Ok(()) => a.locale_applied(true),
                Err(Error::Service(Code::WouldBlock)) => (),
                Err(Error::Service(_)) => a.locale_applied(false),
                Err(e) => return Err(e),
            }
            redraw = true;
        }
        // P14 preference words: saved first, then applied live by the
        // window server; the notification service follows its own words.
        while let Some(&(id, value)) = a.pending_prefs.first() {
            match c.set_preference(id, value) {
                Ok(()) | Err(Error::Service(Code::InvalidRequest)) => {
                    a.pending_prefs.remove(0);
                }
                Err(Error::Service(Code::WouldBlock)) => break,
                Err(e) => return Err(e),
            }
            use door_service_protocol::preferences as p;
            if matches!(id, p::NOTIFICATIONS | p::DND_SCHEDULE | p::FOCUS)
                && let Ok(mut n) = door_service_client::Notifications::connect(registry)
            {
                let _ = n.apply_preference(id, value);
            }
            redraw = true;
        }
        if let Some(bits) = a.pending_accent
            && let Some(accent) = door_service_protocol::desktop::Accent::from_bits(bits)
        {
            match c.set_accent(accent) {
                Ok(()) | Err(Error::Service(Code::InvalidRequest)) => a.pending_accent = None,
                Err(Error::Service(Code::WouldBlock)) => (),
                Err(e) => return Err(e),
            }
            redraw = true;
        }
        if let Some(bits) = a.pending_input {
            match c.set_input_methods(bits) {
                Ok(()) => a.input_applied(),
                Err(Error::Service(Code::WouldBlock)) => (),
                Err(Error::Service(_)) => a.input_refused(),
                Err(e) => return Err(e),
            }
            redraw = true;
        }
        if a.take_input_query() {
            if let Ok(state) = c.input_state() {
                a.input_reported(state.preferences, state.flags);
            }
            redraw = true;
        }
        if a.take_scale_query()
            && let Ok(info) = c.display_scale_info()
        {
            a.scale_info(info.effective, info.recommended, info.offered);
            redraw = true;
        }
        if let Some(percent) = a.pending_scale {
            match c.set_display_scale(percent) {
                Ok(effective) => a.scale_applied(effective, Some(storage))?,
                Err(Error::Service(Code::WouldBlock)) => return Ok(redraw),
                Err(Error::Service(Code::InvalidRequest)) => a.scale_refused(),
                Err(Error::Service(_)) => a.scale_deferred(Some(storage))?,
                Err(e) => return Err(e),
            }
            redraw = true;
        }
        Ok(redraw)
    }
}

/// A VPN connection as Settings lists it: name, status line, and whether
/// it is up or coming up (its button says Disconnect).
fn vpn_row(e: &door_service_protocol::net::vpn::Entry) -> (String, String, bool) {
    use door_service_protocol::net::vpn::{Failure, State};
    let status = match (e.state, e.failure) {
        (State::Connected, _) => String::from("Connected"),
        (State::Connecting, _) => String::from("Connecting…"),
        (State::Failed, Failure::Unresolved) => {
            String::from("Can't connect: the server name couldn't be found")
        }
        (State::Failed, Failure::NoAnswer) => {
            String::from("Can't connect: the server isn't responding")
        }
        (State::Failed, _) => String::from("Can't connect: no network connection"),
        (State::Disconnected, _) => String::from("Not connected"),
    };
    (
        String::from(e.name()),
        status,
        matches!(e.state, State::Connected | State::Connecting),
    )
}

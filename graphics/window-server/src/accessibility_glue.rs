//! Window-server glue for door.accessibility (`native-ui-v1.md`,
//! "Accessibility"): HELLO grants decided from the kernel-authenticated
//! identity (CAP_ASSISTIVE only for the Captions manifest, never from request
//! bytes), request dispatch into the host-tested broker
//! (`door_window::accessibility`), events, the shell surfaces' own trees
//! (`door_window::shell_semantics`), the caption buttons merged into every
//! top-level window's tree, focus and lock propagation, tray flyout keyboard
//! stops, and assistive invokes of server-owned nodes.
use super::*;
use door_abi::{HandleKind, Transfer, rights};
use door_service_protocol::accessibility as a;
use door_system_services::server;
use door_window::accessibility::{ClientKey, Context, Peer, Reply};
use door_window::shell_semantics as sem;
use door_window::taskbar::{self, Slot};
use door_window::{start, tray};

fn key(owner: g::Owner) -> ClientKey {
    (owner.pid, owner.session, owner.epoch)
}
fn peer(owner: g::Owner, identity: Identity) -> Peer {
    Peer {
        key: key(owner),
        assistive: identity.manifest == door_abi::platform::manifest::ASSIST,
    }
}
/// Trusted facts for the broker: window ownership and captions from the
/// engine, focus from the shell first (an open Start or flyout has it).
struct Cx<'a> {
    engine: &'a g::Engine,
    shell_focus: Option<u64>,
}
impl Context for Cx<'_> {
    fn owns(&self, client: ClientKey, window: u64) -> bool {
        self.engine
            .window_info(g::WindowId(window))
            .is_ok_and(|info| key(info.owner) == client && info.state != g::WindowState::Closing)
    }
    /// Sign-in, lock and elevation are painted by the server, not windows;
    /// a locked session already denies every read.
    fn secure(&self, _window: u64) -> bool {
        false
    }
    fn focused_window(&self) -> Option<u64> {
        self.shell_focus
            .or_else(|| self.engine.focused_window().map(|id| id.0))
    }
    fn snapshot(&mut self, section: u64, offset: u64, out: &mut [u8]) -> Result<(), Code> {
        let limit = door_abi::graphics::SECTION_COPY_BYTES;
        for (i, chunk) in out.chunks_mut(limit).enumerate() {
            let n = door_sys::section_snapshot(section, offset + (i * limit) as u64, chunk)
                .map_err(server::status)?;
            if n != chunk.len() {
                return Err(Code::Io);
            }
        }
        Ok(())
    }
    fn caption(&self, window: u64) -> Option<sem::Caption> {
        let info = self.engine.window_info(g::WindowId(window)).ok()?;
        if info.role != g::Role::TopLevel || info.state == g::WindowState::Closing {
            return None;
        }
        let s = self.engine.scale();
        let logical = |r: g::Rect| {
            g::Rect::new(
                s.logical(r.x()),
                s.logical(r.y()),
                s.logical_len(r.width()).max(1),
                s.logical_len(r.height()).max(1),
            )
            .ok()
        };
        let title_height = if info.caption_height == 0 {
            g::TITLE_HEIGHT
        } else {
            info.caption_height
        };
        Some(sem::caption(
            info.title,
            logical(info.outer)?,
            logical(info.content)?,
            title_height,
            info.state == g::WindowState::Maximized,
        ))
    }
}
/// FNV-1a over the shell state the published shell trees describe.
struct Signature(u64);
impl Signature {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
    fn add(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
    }
    fn text(&mut self, s: &str) {
        self.add(s.len() as u64);
        for b in s.bytes() {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
    }
}
fn flyout_code(f: Option<tray::Flyout>) -> u64 {
    match f {
        None => 0,
        Some(tray::Flyout::Calendar) => 1,
        Some(tray::Flyout::Devices) => 2,
        Some(tray::Flyout::Input(n)) => 3 + u64::from(n),
        Some(tray::Flyout::Hidden(n)) => 0x1000 + u64::from(n),
    }
}
impl Desktop {
    /// The shell surface holding keyboard focus, if any.
    fn a11y_shell_focus(&self) -> Option<u64> {
        if self.session.is_none() || self.shell.locked {
            return None;
        }
        if self.launcher {
            return Some(sem::START);
        }
        self.tray.open.map(sem::flyout_window)
    }
    /// HELLO: grant the accessibility capabilities out of `features`.
    pub(super) fn accessibility_hello(
        &mut self,
        owner: g::Owner,
        identity: Identity,
        features: u32,
    ) -> u32 {
        let mask = a::CAP_ACCESSIBILITY | a::CAP_ASSISTIVE;
        let requested = features & mask;
        let granted = if requested == 0 {
            0
        } else {
            self.shell.a11y.hello(&peer(owner, identity), requested)
        };
        if granted & a::CAP_ASSISTIVE != 0 {
            // A new reader gets the shell's trees on the next tick.
            self.shell.a11y_signature = 0;
            let _ =
                door_sys::write(format!("DOOR_P12 A11Y ASSISTIVE pid={}\n", owner.pid).as_bytes());
        }
        (features & !mask) | granted
    }
    /// door.accessibility request (opcodes `a::FIRST..=a::LAST`).
    pub(super) fn accessibility_request(
        &mut self,
        identity: Identity,
        owner: g::Owner,
        op: u32,
        b: &[u8],
        t: &[Transfer],
    ) -> Result<Response, Code> {
        let mut sections = [0u64; 1];
        let sections: &[u64] = match t {
            [] => &[],
            [one] if op == a::PUBLISH_SECTION => {
                let info = door_sys::inspect(one.handle).map_err(server::status)?;
                if info[0] != HandleKind::Section as u64 || info[1] & rights::READ == 0 {
                    return Err(Code::Denied);
                }
                sections[0] = one.handle;
                &sections
            }
            _ => return Err(Code::InvalidRequest),
        };
        let p = peer(owner, identity);
        let mut cx = Cx {
            engine: &self.engine,
            shell_focus: self.a11y_shell_focus(),
        };
        match self.shell.a11y.request(&p, op, b, sections, &mut cx)? {
            Reply::Empty => Ok(Response::empty()),
            Reply::Data(d) => Ok(Response::bytes(&d)),
            Reply::Server {
                window,
                node,
                action,
                serial,
            } => {
                let result = self.accessibility_server_action(window, node, action);
                self.shell.a11y.server_action_done(p.key, serial, result);
                Ok(Response::bytes(&serial.to_le_bytes()))
            }
        }
    }
    /// An assistive Invoke (or Select/Expand) of a node the server owns:
    /// performed exactly as the pointer or keyboard would.
    fn accessibility_server_action(&mut self, window: u64, node: u64, action: u32) -> u32 {
        if self.shell.locked {
            return a::RESULT_GONE;
        }
        if sem::is_caption_node(node) {
            let region = match node {
                sem::CAPTION_MINIMIZE => g::HitRegion::Minimize,
                sem::CAPTION_MAXIMIZE => g::HitRegion::Maximize,
                sem::CAPTION_CLOSE => g::HitRegion::Close,
                _ => return a::RESULT_REJECTED,
            };
            self.caption_action(g::WindowId(window), region);
            self.engine.invalidate();
            return a::RESULT_DONE;
        }
        if !matches!(
            action,
            a::ACTION_INVOKE
                | a::ACTION_SELECT
                | a::ACTION_EXPAND
                | a::ACTION_COLLAPSE
                | a::ACTION_FOCUS
        ) {
            return a::RESULT_REJECTED;
        }
        match window {
            sem::START if self.launcher => {
                let layout = self.taskbar_layout();
                let g = self.shell.start.geometry(layout);
                let Some(target) = sem::start_target(&self.shell.start, &g, node) else {
                    return a::RESULT_GONE;
                };
                if target == start::Target::Search {
                    self.shell.start.focus = Some(target);
                } else {
                    self.start_activate(target);
                }
            }
            sem::TASKBAR => {
                let buttons = self.taskbar_buttons();
                match sem::taskbar_target(node, &buttons) {
                    Some(sem::TaskbarTarget::Slot(Slot::Start)) => self.toggle_start(false),
                    Some(sem::TaskbarTarget::Slot(Slot::Search)) => self.toggle_start(true),
                    Some(sem::TaskbarTarget::Slot(Slot::TaskView)) => {
                        self.close_start();
                        self.system_command(door_window::policy::SystemCommand::TaskView);
                    }
                    Some(sem::TaskbarTarget::Slot(Slot::App(i))) => {
                        self.close_start();
                        self.tray.open = None;
                        if let Some(button) = buttons.get(i).copied() {
                            self.taskbar_app(button.app);
                        } else if let Some(extra) = self.taskbar_extra(i) {
                            self.extra_click(&extra);
                        } else {
                            return a::RESULT_GONE;
                        }
                    }
                    Some(sem::TaskbarTarget::Tray(target)) => {
                        let action = match target {
                            taskbar::TrayTarget::Language => tray::Action::Language,
                            taskbar::TrayTarget::QuickSettings => tray::Action::Devices,
                            taskbar::TrayTarget::Clock => tray::Action::Clock,
                            taskbar::TrayTarget::ShowDesktop => tray::Action::Desktop,
                        };
                        if matches!(
                            action,
                            tray::Action::Devices | tray::Action::Clock | tray::Action::Language
                        ) {
                            self.remember_shell_focus();
                        }
                        self.tray_action(action);
                    }
                    None => return a::RESULT_GONE,
                }
            }
            w if self.tray.open.is_some_and(|f| sem::flyout_window(f) == w) => {
                let layout = self.taskbar_layout();
                match sem::flyout_action(&self.tray, layout, node) {
                    Some(action) => self.tray_action(action),
                    None => return a::RESULT_REJECTED,
                }
            }
            _ => return a::RESULT_GONE,
        }
        self.bar_dirty = true;
        self.engine.invalidate();
        a::RESULT_DONE
    }
    pub(super) fn accessibility_event(&mut self, owner: g::Owner) -> Option<w::Event> {
        self.shell.a11y.take_event(key(owner))
    }
    pub(super) fn accessibility_exited(&mut self, owner: g::Owner) {
        let cx = Cx {
            engine: &self.engine,
            shell_focus: self.a11y_shell_focus(),
        };
        self.shell.a11y.client_gone(key(owner), &cx);
    }
    pub(super) fn accessibility_window_gone(&mut self, window: u64) {
        let cx = Cx {
            engine: &self.engine,
            shell_focus: self.a11y_shell_focus(),
        };
        self.shell.a11y.window_closed(window, &cx);
    }
    /// Tab/Shift+Tab and the arrows move through the open flyout's stops;
    /// Enter and Space activate the focused one. Returns true when consumed.
    pub(super) fn tray_key(&mut self, scan: u32, shift: bool) -> bool {
        let layout = self.taskbar_layout();
        let stops = sem::tray_stops(&self.tray, layout);
        if stops.is_empty() {
            return false;
        }
        let n = stops.len();
        let at = self.shell.tray_focus.filter(|i| *i < n);
        match scan {
            15 | 0x148 | 0x150 | 0x14b | 0x14d => {
                let back = (scan == 15 && shift) || matches!(scan, 0x148 | 0x14b);
                self.shell.tray_focus = Some(match at {
                    None => 0,
                    Some(i) if back => (i + n - 1) % n,
                    Some(i) => (i + 1) % n,
                });
                self.shell.tray_focus_for = self.tray.open;
                true
            }
            28 | 0x11c | 57 => {
                if let Some(stop) = at.and_then(|i| stops.get(i)) {
                    self.tray_action(stop.action);
                    self.bar_dirty = true;
                    self.engine.invalidate();
                }
                true
            }
            _ => false,
        }
    }
    /// Once per service loop: lock state, tray focus, the shell trees (only
    /// while an assistive client reads and only when the shell changed) and
    /// focus changes for readers.
    pub(super) fn accessibility_tick(&mut self) {
        // A flyout opens with its first stop focused (Windows moves focus
        // into Quick settings and the Notification center); closing drops it.
        if self.tray.open != self.shell.tray_focus_for {
            self.shell.tray_focus_for = self.tray.open;
            self.shell.tray_focus = self.tray.open.map(|_| 0);
        }
        let locked = self.shell.locked;
        if locked != self.shell.a11y_locked {
            self.shell.a11y_locked = locked;
            let cx = Cx {
                engine: &self.engine,
                shell_focus: self.a11y_shell_focus(),
            };
            self.shell.a11y.set_locked(locked, &cx);
        }
        if !self.shell.a11y.has_readers() || locked || self.session.is_none() {
            let cx = Cx {
                engine: &self.engine,
                shell_focus: self.a11y_shell_focus(),
            };
            self.shell.a11y.focus_changed(&cx);
            return;
        }
        let layout = self.taskbar_layout();
        let buttons = self.taskbar_buttons();
        let mut sig = Signature::new();
        sig.add(u64::from(layout.controls().bits()));
        let (w, h) = layout.size();
        sig.add(u64::from(w) << 32 | u64::from(h));
        for b in &buttons {
            sig.add(b.app.index() as u64 | u64::from(b.windows) << 8 | u64::from(b.active) << 16);
        }
        for e in self.taskbar_extras() {
            sig.add(u64::from(e.key) | (e.windows() as u64) << 32 | u64::from(e.active) << 40);
            sig.text(&e.label);
        }
        sig.add(u64::from(self.launcher));
        if self.launcher {
            let s = &self.shell.start;
            sig.text(&s.query);
            sig.add(s.focus.and_then(|t| sem::panel_node(s, t)).unwrap_or(0));
            sig.add(u64::from(s.search_panel()));
            sig.add(s.selected as u64);
            sig.add(s.scroll as u64);
            sig.add(u64::from(s.view.index()) | u64::from(s.expanded) << 8);
            sig.add(
                door_window::search::Filter::ALL
                    .iter()
                    .position(|f| *f == s.filter)
                    .unwrap_or(0) as u64,
            );
            for app in &s.pins {
                sig.add(app.index() as u64);
            }
            sig.add(s.files.len() as u64 | (s.recent.len() as u64) << 16);
            for f in s.files.iter().chain(&s.recent) {
                sig.text(&f.name);
            }
        }
        sig.add(flyout_code(self.tray.open));
        sig.add(self.shell.tray_focus.map_or(0, |i| i as u64 + 1));
        if let Some((y, m)) = self.tray.month {
            sig.add(u64::from(y) << 8 | u64::from(m));
        }
        if let Some(t) = self.tray.time {
            sig.add(
                u64::from(t.year) << 32
                    | u64::from(t.month) << 24
                    | u64::from(t.day) << 16
                    | u64::from(t.hour) << 8
                    | u64::from(t.minute),
            );
        }
        sig.add(u64::from(self.tray.region) << 16 | u64::from(self.tray.zone));
        if let Some(p) = self.tray.input {
            sig.add(u64::from(p.bits()) | (p.current_index() as u64) << 40);
        }
        let signature = sig.0 | 1;
        if signature != self.shell.a11y_signature {
            self.shell.a11y_signature = signature;
            let cx = Cx {
                engine: &self.engine,
                shell_focus: self.a11y_shell_focus(),
            };
            let extras = self.taskbar_extras();
            let bar = sem::taskbar_with(
                1,
                layout,
                &buttons,
                &extras,
                &self.tray,
                self.launcher.then_some(self.shell.start.mode),
            );
            self.shell.a11y.publish_server(bar, &cx);
            if self.launcher {
                let tree = sem::start(1, &self.shell.start, layout, &self.fonts);
                self.shell.a11y.publish_server(tree, &cx);
            } else {
                self.shell.a11y.withdraw_server(sem::START, &cx);
            }
            let open = self.tray.open.map(sem::flyout_window);
            for window in [
                sem::QUICK_SETTINGS,
                sem::NOTIFICATION_CENTER,
                sem::INPUT_SWITCHER,
            ] {
                if Some(window) != open {
                    self.shell.a11y.withdraw_server(window, &cx);
                }
            }
            if let Some(tree) = sem::flyout(1, &self.tray, layout, self.shell.tray_focus) {
                self.shell.a11y.publish_server(tree, &cx);
            }
        }
        let cx = Cx {
            engine: &self.engine,
            shell_focus: self.a11y_shell_focus(),
        };
        self.shell.a11y.focus_changed(&cx);
    }
}

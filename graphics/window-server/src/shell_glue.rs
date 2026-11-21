//! door.shell wiring (spec §60, GUI-F/GUI-H): the restartable desktop shell
//! process's role, authorized window enumeration and change notes, its
//! publications (taskbar groups, search catalog, notification view), the
//! actions it may request, and the taskbar's extra buttons. The server never
//! calls the shell: it queues events the shell polls, and while no shell is
//! connected it derives Windows apps' taskbar buttons from its own window
//! list so nothing disappears and no application restarts.
use super::*;
use crate::desktop::{App, items};
use door_service_protocol::shell as s;
use door_window::taskbar::Slot;
use door_window::{
    menus,
    shell_link::{Candidate, Extra, WindowState},
    shell_view::NoticeHit,
};

/// One authorized top-level window as door.shell reports it.
pub(super) struct ShellWindow {
    pub(super) id: u64,
    pub(super) owner: g::Owner,
    pub(super) app: u32,
    pub(super) flags: u32,
    pub(super) state: WindowState,
    pub(super) manifest: u32,
    pub(super) title: alloc::string::String,
}
impl ShellWindow {
    fn signature(&self) -> u64 {
        // FNV-1a over what the shell can observe.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |b: u8| {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x100_0000_01b3);
        };
        for b in self.flags.to_le_bytes() {
            eat(b);
        }
        eat(self.state.progress_state);
        for b in self.state.progress.to_le_bytes() {
            eat(b);
        }
        for b in self.title.bytes() {
            eat(b);
        }
        h
    }
}
impl Desktop {
    /// Authorized enumeration: ordinary top-level windows of the session's
    /// clients (never lock/sign-in or prompt surfaces), sorted by id.
    pub(super) fn shell_windows(&self) -> Vec<ShellWindow> {
        let active = self.engine.active_workspace();
        let mut out: Vec<ShellWindow> = self
            .engine
            .windows()
            .filter(|w| w.role == g::Role::TopLevel && w.state != g::WindowState::Closing)
            .filter_map(|w| {
                let client = self
                    .clients
                    .iter()
                    .find(|c| c.owner == w.owner && !c.revoked)?;
                let app = self
                    .app_classes
                    .iter()
                    .find(|(o, _)| *o == w.owner)
                    .map(|(_, a)| u32::from(a.command()))
                    .or_else(|| {
                        door_window::apps::Tool::from_manifest(client.manifest)
                            .map(|t| u32::from(t.command()))
                    })
                    .unwrap_or(0);
                let state = self.shell_link.window_state(w.id.0);
                let mut flags = 0;
                for (on, bit) in [
                    (w.active, s::flag::ACTIVE),
                    (w.state == g::WindowState::Minimized, s::flag::MINIMIZED),
                    (w.state == g::WindowState::Maximized, s::flag::MAXIMIZED),
                    (state.attention, s::flag::ATTENTION),
                    (self.shell.hang.window_hung(w.owner, w.id.0), s::flag::HUNG),
                    (client.win32, s::flag::WIN32),
                    (self.win32.owner(w.id.0).is_some(), s::flag::SKIP_TASKBAR),
                    (w.workspace != active, s::flag::OTHER_DESKTOP),
                ] {
                    if on {
                        flags |= bit;
                    }
                }
                Some(ShellWindow {
                    id: w.id.0,
                    owner: w.owner,
                    app,
                    flags,
                    state,
                    manifest: client.manifest as u32,
                    title: s::clip(w.title, s::TITLE_BYTES).into(),
                })
            })
            .collect();
        out.sort_by_key(|w| w.id);
        out
    }
    /// Per-iteration diff of the authorized window list into door.shell
    /// events (ADDED/CHANGED/REMOVED/FOCUS). Activation satisfies attention.
    pub(super) fn shell_tick(&mut self) {
        // Nothing observes window changes without a shell or per-window
        // taskbar state; a connected shell is served at most once per tick.
        if !self.shell_link.connected() && !self.shell_link.has_states() {
            self.shell_seen.clear();
            return;
        }
        let now = door_sys::time().unwrap_or(0);
        if now == self.shell_tick_at && !self.shell_link.has_states() {
            return;
        }
        self.shell_tick_at = now;
        let windows = self.shell_windows();
        let mut repaint = false;
        for w in &windows {
            if w.flags & s::flag::ACTIVE != 0 && w.state.attention {
                repaint |= self.shell_link.activated(w.id);
            }
        }
        let current: Vec<(u64, u64)> = windows.iter().map(|w| (w.id, w.signature())).collect();
        let focus = windows
            .iter()
            .find(|w| w.flags & s::flag::ACTIVE != 0)
            .map_or(0, |w| w.id);
        let previous = core::mem::take(&mut self.shell_seen);
        for (id, sig) in &current {
            match previous.iter().find(|(p, _)| p == id) {
                None => self.shell_link.note(s::event::ADDED, *id),
                Some((_, old)) if old != sig => self.shell_link.note(s::event::CHANGED, *id),
                _ => (),
            }
        }
        for (id, _) in &previous {
            if !current.iter().any(|(c, _)| c == id) {
                self.shell_link.note(s::event::REMOVED, *id);
                repaint = true;
            }
        }
        if focus != self.shell_focus_id {
            self.shell_focus_id = focus;
            self.shell_link.note(s::event::FOCUS, focus);
        }
        if previous.len() != current.len() {
            repaint = true;
        }
        self.shell_seen = current;
        if repaint {
            self.bar_dirty = true;
        }
    }
    /// Extra taskbar buttons after the catalog apps: the shell's published
    /// groups (Windows programs and bundled apps such as Clock), else (and for
    /// windows no group covers yet) Windows programs'
    /// unowned top-level windows grouped per process.
    pub(super) fn taskbar_extras(&self) -> Vec<Extra> {
        let published = self.shell_link.taskbar();
        // Fast path (every pointer move asks): nothing published and no
        // Windows-personality client means no extra button.
        if published.is_none_or(|groups| groups.is_empty()) && !self.clients.iter().any(|c| c.win32)
        {
            return Vec::new();
        }
        let listed =
            |id: u64| published.is_some_and(|groups| groups.iter().any(|g| g.ids.contains(&id)));
        let candidates: Vec<Candidate> = self
            .shell_windows()
            .into_iter()
            // Native windows join only when the shell lists them (bundled
            // apps such as Clock); catalog apps keep their own buttons.
            .filter(|w| {
                (w.app == 0 || listed(w.id))
                    && w.flags & (s::flag::SKIP_TASKBAR | s::flag::OTHER_DESKTOP) == 0
                    && (w.flags & s::flag::WIN32 != 0 || listed(w.id))
            })
            .map(|w| Candidate {
                id: g::WindowId(w.id),
                owner: w.owner,
                active: w.flags & s::flag::ACTIVE != 0,
                title: w.title,
                state: w.state,
            })
            .collect();
        door_window::shell_link::extras(published, &candidates)
    }
    /// Attention/progress marks of catalog apps' windows.
    pub(super) fn taskbar_marks(&self) -> Vec<(App, WindowState)> {
        if !self.shell_link.has_states() {
            return Vec::new();
        }
        self.engine
            .windows()
            .filter_map(|w| {
                let state = self.shell_link.window_state(w.id.0);
                if state == WindowState::default() {
                    return None;
                }
                self.app_classes
                    .iter()
                    .find(|(o, _)| *o == w.owner)
                    .map(|(_, a)| (*a, state))
            })
            .collect()
    }
    /// Taskbar slots: catalog app buttons plus extras.
    pub(super) fn taskbar_slot_count(&self) -> usize {
        self.taskbar_buttons().len() + self.taskbar_extras().len()
    }
    /// The extra button at taskbar app slot `slot`, if it is one.
    pub(super) fn taskbar_extra(&self, slot: usize) -> Option<Extra> {
        let natives = self.taskbar_buttons().len();
        slot.checked_sub(natives)
            .and_then(|i| self.taskbar_extras().into_iter().nth(i))
    }
    /// Primary click on an extra button: launch a pinned app with no window
    /// (through the shell), otherwise Windows' taskbar toggle/cycle.
    pub(super) fn extra_click(&mut self, extra: &Extra) {
        let shift = self.key_modifiers & door_abi::graphics::SHIFT != 0;
        if extra.ids.is_empty() || shift {
            let command = if shift {
                s::command::LAUNCH_NEW
            } else {
                s::command::LAUNCH
            };
            if self.shell_link.invoke(command, 0, extra.key, 0) {
                self.shell_note("invoke", command, extra.key);
            }
            return;
        }
        let ids: Vec<g::WindowId> = extra
            .ids
            .iter()
            .copied()
            .filter(|id| self.engine.window_info(*id).is_ok())
            .collect();
        let active = ids
            .iter()
            .position(|id| self.engine.window_info(*id).is_ok_and(|w| w.active));
        match active {
            Some(i) if ids.len() == 1 => {
                if let Ok(owner) = self.engine.window_info(ids[i]).map(|w| w.owner) {
                    let _ = self
                        .engine
                        .request_state(owner, ids[i], g::WindowState::Minimized);
                }
                self.manual_focus(None);
            }
            Some(i) => {
                let next = ids[(i + 1) % ids.len()];
                self.activate_desktop_window(next);
                self.manual_focus(Some(next));
            }
            None => {
                if let Some(top) = ids.last().copied() {
                    self.activate_desktop_window(top);
                    self.manual_focus(Some(top));
                }
            }
        }
        self.bar_dirty = true;
    }
    pub(super) fn extra_menu(&mut self, extra: &Extra, anchor: g::Rect) {
        let shell =
            self.shell_link.connected() && extra.key & door_window::shell_link::FALLBACK_KEY == 0;
        let menu = menus::extra_jump_list(
            &extra.label,
            extra.key,
            extra.pinned,
            extra.windows(),
            shell,
            self.shell.prefs.end_task(),
        );
        if !menu.entries.is_empty() {
            let edge = self.taskbar_layout().controls().edge();
            self.open_menu(
                menu,
                door_window::menu::Anchor::Bar(anchor, edge, door_window::menu::BarAlign::Center),
                None,
            );
        }
    }
    /// Close every window of the extra button `key` (server authority).
    pub(super) fn close_extra(&mut self, key: u32) {
        let Some(extra) = self.taskbar_extras().into_iter().find(|e| e.key == key) else {
            return;
        };
        for id in extra.ids {
            if let Ok(owner) = self.engine.window_info(id).map(|w| w.owner) {
                let _ = self.close(owner, id);
            }
        }
    }
    /// A Start/search catalog entry (registered app) was chosen.
    pub(super) fn launch_registered(&mut self, key: u32) {
        self.close_start();
        if self.shell_link.invoke(s::command::LAUNCH, 0, key, 0) {
            self.shell_note("invoke", s::command::LAUNCH, key);
        }
    }
    /// A notification toast or centre control was used.
    pub(super) fn notice_action(&mut self, hit: NoticeHit, toast: bool) {
        let (command, a, b) = match hit {
            NoticeHit::Body(id) => (s::command::NOTICE_ACTIVATE, id, 0),
            NoticeHit::Button(id, i) => (s::command::NOTICE_ACTIVATE, id, 1 + u32::from(i)),
            NoticeHit::Close(id) if toast => (s::command::NOTICE_DISMISS, id, 0),
            NoticeHit::Close(id) => (s::command::NOTICE_CLEAR, id, 0),
            NoticeHit::ClearAll => (s::command::NOTICE_CLEAR, 0, 0),
            NoticeHit::Dnd => (
                s::command::NOTICE_DND,
                0,
                u32::from(!self.shell_link.notices().dnd),
            ),
            NoticeHit::Inside => return,
        };
        if self.shell_link.invoke(command, 0, a, b) {
            self.shell_note("invoke", command, a);
        }
        if matches!(hit, NoticeHit::Body(_) | NoticeHit::Button(..)) {
            self.tray.open = None;
        }
        self.engine.invalidate();
    }
    /// The notification centre opened: unread notifications are seen.
    pub(super) fn notices_seen(&mut self) {
        if self.shell_link.notices().badge > 0 {
            self.shell_link.invoke(s::command::NOTICE_SEEN, 0, 0, 0);
        }
    }
    /// Copy the published notification view where the painters read it.
    fn sync_notices(&mut self) {
        let notices = self.shell_link.notices().clone();
        let live = self.shell_link.connected();
        if self.tray.notices != notices || self.tray.notices_live != live {
            self.tray.notices = notices;
            self.tray.notices_live = live;
            self.bar_dirty = true;
            self.engine.invalidate();
        }
    }
    fn shell_note(&self, what: &str, a: u32, b: u32) {
        let _ = door_sys::write(format!("DOOR_P14 SHELL {what} a={a} b={b}\n").as_bytes());
    }
    /// The role holder went away: published state is withdrawn and the
    /// taskbar falls back to the server's own window list.
    pub(super) fn shell_revoked(&mut self, owner: g::Owner) {
        if self.shell_link.revoke(owner) {
            self.shell.start.catalog.clear();
            self.sync_notices();
            self.bar_dirty = true;
            self.engine.invalidate();
            let _ = door_sys::write(
                format!("DOOR_P14 SHELL_LOST pid={} fallback=1\n", owner.pid).as_bytes(),
            );
        }
    }
    /// door.shell requests (window endpoint, `shell::FIRST..=LAST`).
    pub(super) fn shell_request(
        &mut self,
        owner: g::Owner,
        peer: Identity,
        op: u32,
        b: &[u8],
    ) -> Result<Response, Code> {
        if op == s::SET_TASKBAR_STATE {
            let state = s::TaskbarState::decode(b)?;
            let id = g::WindowId(state.window);
            let info = self.engine.window_info(id).map_err(|_| Code::NotFound)?;
            if info.owner != owner || info.role != g::Role::TopLevel {
                return Err(Code::Denied);
            }
            let active = info.active;
            if self.shell_link.set_window_state(state, active) {
                self.bar_dirty = true;
            }
            return Ok(Response::empty());
        }
        if op == s::HELLO {
            if b.len() != 8
                || u32::from_le_bytes(b[..4].try_into().unwrap()) != s::VERSION
                || b[4..] != [0; 4]
            {
                return Err(Code::InvalidRequest);
            }
            if peer.manifest != door_abi::platform::manifest::DESKTOP_SHELL {
                return Err(Code::Denied);
            }
            // Init's catalog names the session's shell host; until it does,
            // the shell retries.
            match self.shell.files.host {
                Some(host) if host == owner => (),
                Some(_) => return Err(Code::Denied),
                None => return Err(Code::WouldBlock),
            }
            let previous = self.shell_link.grant(owner);
            self.shell_seen = self
                .shell_windows()
                .iter()
                .map(|w| (w.id, w.signature()))
                .collect();
            let hello = s::Hello {
                version: s::VERSION,
                generation: self.shell_link.generation(),
                windows: self.shell_seen.len() as u32,
                locked: self.shell.locked,
            };
            let _ = door_sys::write(
                format!(
                    "DOOR_P14 SHELL_CONNECTED pid={} incarnation={} windows={} replaced={}\n",
                    owner.pid,
                    self.shell_link.grants,
                    hello.windows,
                    u8::from(previous.is_some())
                )
                .as_bytes(),
            );
            self.sync_notices();
            return Ok(Response::bytes(&hello.encode()));
        }
        if self.shell_link.holder() != Some(owner) {
            return Err(Code::Denied);
        }
        match op {
            s::WINDOWS if b.len() == 8 => {
                let after = u64::from_le_bytes(b.try_into().unwrap());
                let windows = self.shell_windows();
                let next = windows.iter().find(|w| w.id > after);
                let page = s::WindowPage {
                    generation: self.shell_link.generation(),
                    window: next.map(|w| s::Window {
                        id: w.id,
                        pid: w.owner.pid,
                        instance: w.owner.epoch,
                        app: w.app,
                        flags: w.flags,
                        progress_state: w.state.progress_state,
                        progress: w.state.progress,
                        manifest: w.manifest,
                        title: &w.title,
                    }),
                };
                let mut out = [0; s::WindowPage::MAX_BYTES];
                let n = page.encode(&mut out)?;
                Ok(Response::bytes(&out[..n]))
            }
            s::POLL if b.is_empty() => Ok(match self.shell_link.poll(owner)? {
                Some(e) => Response::bytes(&e.encode()),
                None => Response::empty(),
            }),
            s::PUBLISH => {
                let entry = s::Entry::decode(b)?;
                let live: Vec<u64> = self.shell_windows().iter().map(|w| w.id).collect();
                let before = self.taskbar_extras();
                if self
                    .shell_link
                    .publish(owner, &entry, |id| live.contains(&id))?
                {
                    if entry.kind == s::kind::CATALOG {
                        self.shell.start.catalog = self
                            .shell_link
                            .catalog()
                            .iter()
                            .map(|g| door_window::start::CatalogApp {
                                key: g.key,
                                icon: g.icon,
                                label: g.label.clone(),
                            })
                            .collect();
                        if self.launcher {
                            self.engine.invalidate();
                        }
                    } else if self.taskbar_extras() != before {
                        // Only a visible change repaints (the bar band).
                        self.bar_dirty = true;
                    }
                }
                Ok(Response::empty())
            }
            s::NOTIFY_VIEW => {
                let view = s::NoticeView::decode(b)?;
                let shown: Vec<u32> = self.shell_link.notices().toasts().map(|n| n.id).collect();
                if self.shell_link.publish_notice(owner, &view)? {
                    // A new banner plays the scheme's Notification sound
                    // unless sounds are off or Do not disturb holds it.
                    let fresh = self
                        .shell_link
                        .notices()
                        .toasts()
                        .any(|n| !shown.contains(&n.id));
                    let notify = self
                        .shell
                        .prefs
                        .get(door_service_protocol::preferences::NOTIFICATIONS);
                    if fresh
                        && !self.shell_link.notices().dnd
                        && notify & door_service_protocol::preferences::notify::SOUNDS != 0
                    {
                        self.play_sound(door_service_protocol::audio::sound::NOTIFICATION);
                    }
                    self.sync_notices();
                }
                Ok(Response::empty())
            }
            s::ACTION => {
                let (window, action) = s::decode_action(b)?;
                let id = g::WindowId(window);
                if !self.shell_windows().iter().any(|w| w.id == window) {
                    return Err(Code::NotFound);
                }
                let info = self.engine.window_info(id).map_err(|_| Code::NotFound)?;
                let (target, active, state) = (info.owner, info.active, info.state);
                match action {
                    s::action::ACTIVATE | s::action::RESTORE => {
                        if action == s::action::RESTORE && state == g::WindowState::Maximized {
                            let _ = self
                                .engine
                                .request_state(target, id, g::WindowState::Normal);
                        }
                        self.activate_desktop_window(id);
                        self.manual_focus(Some(id));
                    }
                    s::action::MINIMIZE => {
                        let _ = self
                            .engine
                            .request_state(target, id, g::WindowState::Minimized);
                    }
                    s::action::CLOSE => {
                        let _ = self.close(target, id);
                    }
                    _ if active => {
                        let _ = self
                            .engine
                            .request_state(target, id, g::WindowState::Minimized);
                        self.manual_focus(None);
                    }
                    _ => {
                        self.activate_desktop_window(id);
                        self.manual_focus(Some(id));
                    }
                }
                self.bar_dirty = true;
                Ok(Response::empty())
            }
            _ => Err(Code::InvalidRequest),
        }
    }
}
/// Hover dwell before the taskbar preview opens (Windows: about 400 ms).
const PREVIEW_DWELL_TICKS: u64 = 40;
impl Desktop {
    fn hide_preview(&mut self) {
        if self.shell.preview.take().is_some() {
            self.engine.invalidate();
        }
    }
    fn hide_tooltip(&mut self) {
        if self.shell.tooltip.take().is_some() {
            self.engine.invalidate();
        }
    }
    /// The tooltip a taskbar button shows: its name for Start, Search (as
    /// an icon), Task View and pinned apps that are not running; running
    /// apps show their thumbnails instead.
    fn slot_tooltip(
        &self,
        slot: Slot,
        layout: door_window::taskbar::Layout,
    ) -> Option<alloc::string::String> {
        match slot {
            Slot::Start => Some(alloc::string::String::from("Start")),
            Slot::Search => (!layout.search_box()).then(|| alloc::string::String::from("Search")),
            Slot::TaskView => Some(alloc::string::String::from("Task View")),
            Slot::App(i) => {
                if let Some(button) = self.taskbar_buttons().get(i) {
                    (button.windows == 0).then(|| alloc::string::String::from(button.app.label()))
                } else {
                    self.taskbar_extra(i)
                        .filter(|extra| extra.windows() == 0)
                        .map(|extra| extra.label.clone())
                }
            }
        }
    }
    /// Windows 11 taskbar tooltips: after the same dwell as thumbnails, the
    /// hovered button's name, off the bar on any edge; never while its
    /// thumbnails show, and not again until the pointer moves to another
    /// button.
    fn tooltip_tick(
        &mut self,
        now: u64,
        layout: door_window::taskbar::Layout,
        slots: usize,
        slot: Option<Slot>,
    ) {
        match (slot, self.tooltip_hover) {
            (None, _) => {
                self.tooltip_hover = None;
                self.hide_tooltip();
            }
            (Some(s), Some((t, since))) if s == t => {
                if self.shell.tooltip.is_none()
                    && self.shell.preview.is_none()
                    && now >= since + PREVIEW_DWELL_TICKS
                {
                    if let Some(text) = self.slot_tooltip(s, layout) {
                        self.shell.tooltip = Some(door_window::shell_view::Tooltip::new(
                            &self.fonts,
                            layout,
                            layout.slot(s, slots),
                            &text,
                        ));
                        self.engine.invalidate();
                    }
                    self.tooltip_hover = Some((s, u64::MAX / 2));
                }
                if self.shell.preview.is_some() {
                    self.hide_tooltip();
                }
            }
            (Some(s), _) => {
                self.tooltip_hover = Some((s, now));
                self.hide_tooltip();
            }
        }
    }
    /// A press on Start, Search or Task View hides its tooltip until the
    /// pointer moves (`preview_hold`).
    pub(super) fn suppress_tooltip(&mut self) {
        self.tooltip_hover = self.tooltip_hover.map(|(s, _)| (s, u64::MAX / 2));
        self.preview_hold = Some(self.logical_pointer());
        self.hide_tooltip();
    }
    /// A press on an app button hides its thumbnails and tooltip until the
    /// pointer moves, as Windows does.
    pub(super) fn suppress_preview(&mut self, slot: usize) {
        self.preview_hover = Some((slot, u64::MAX / 2));
        self.hide_preview();
        self.tooltip_hover = self.tooltip_hover.map(|(s, _)| (s, u64::MAX / 2));
        self.hide_tooltip();
        self.preview_hold = Some(self.logical_pointer());
    }
    /// Windows 11 taskbar hover preview: after a dwell on a button with
    /// windows, live thumbnails of its windows open above it; they stay
    /// while the pointer is over them.
    pub(super) fn preview_tick(&mut self, now: u64) {
        if self.session.is_none()
            || self.shell.locked
            || self.shell.menu.is_some()
            || self.launcher
            || self.drag.is_some()
            || self.overview.is_some()
            || self.tray.open.is_some()
        {
            self.preview_hover = None;
            self.hide_preview();
            self.tooltip_hover = None;
            self.hide_tooltip();
            return;
        }
        let (x, y) = self.logical_pointer();
        // A press-dismissed thumbnail or tooltip may come back once the
        // pointer moves: the dwell starts again from here.
        if self.preview_hold.is_some_and(|p| p != (x, y)) {
            self.preview_hold = None;
            self.preview_hover = self.preview_hover.map(|(s, _)| (s, now));
            self.tooltip_hover = self.tooltip_hover.map(|(s, _)| (s, now));
        }
        if self
            .shell
            .preview
            .as_ref()
            .is_some_and(|p| p.rect.contains(x, y))
        {
            return;
        }
        let layout = self.taskbar_layout();
        // Cheap test first: the pointer is on the bar at all.
        if !layout.bar().contains(x, y) {
            self.preview_hover = None;
            self.hide_preview();
            self.tooltip_hover = None;
            self.tray.mic_hover = None;
            self.hide_tooltip();
            return;
        }
        let slots = self.taskbar_slot_count();
        let hit = layout.hit_slot(x, y, slots);
        // The microphone glyph names who is recording after the dwell.
        let mic = layout.microphone();
        if self.tray.audio.is_some_and(|a| a.capturing > 0) && mic.contains(x, y) {
            self.tooltip_hover = None;
            match self.tray.mic_hover {
                Some(since)
                    if self.shell.tooltip.is_none() && now >= since + PREVIEW_DWELL_TICKS =>
                {
                    let text = door_window::tray::microphone_tooltip(&self.tray.capture_names);
                    self.shell.tooltip = Some(door_window::shell_view::Tooltip::new(
                        &self.fonts,
                        layout,
                        mic,
                        &text,
                    ));
                    self.tray.mic_hover = Some(u64::MAX / 2);
                    self.engine.invalidate();
                }
                Some(_) => (),
                None => self.tray.mic_hover = Some(now),
            }
        } else {
            if self.tray.mic_hover.take().is_some() {
                self.hide_tooltip();
            }
            self.tooltip_tick(now, layout, slots, hit);
        }
        let slot = match hit {
            Some(Slot::App(i)) => Some(i),
            _ => None,
        };
        match (slot, self.preview_hover) {
            (None, _) => {
                self.preview_hover = None;
                self.hide_preview();
            }
            (Some(i), Some((j, since))) if i == j => {
                if self.shell.preview.is_none() && now >= since + PREVIEW_DWELL_TICKS {
                    self.show_preview(i, layout, slots);
                    // Shown once per dwell (no reopening while it stays).
                    self.preview_hover = Some((i, u64::MAX / 2));
                }
            }
            (Some(i), _) => {
                self.preview_hover = Some((i, now));
                self.hide_preview();
            }
        }
    }
    fn show_preview(&mut self, slot: usize, layout: door_window::taskbar::Layout, slots: usize) {
        let ws = self.engine.active_workspace();
        let windows: Vec<(g::WindowId, u16)> =
            if let Some(button) = self.taskbar_buttons().get(slot).copied() {
                let icon = door_window::shell_view::app_icon(button.app);
                items(&self.engine, &self.app_classes)
                    .into_iter()
                    .filter(|i| i.app == button.app)
                    .map(|i| (i.id, icon))
                    .collect()
            } else if let Some(extra) = self.taskbar_extra(slot) {
                extra.ids.iter().map(|id| (*id, extra.icon)).collect()
            } else {
                Vec::new()
            };
        let scale = self.engine.scale();
        let (tw, th) = (
            scale.len(door_window::shell_view::THUMB_W) as usize,
            scale.len(door_window::shell_view::THUMB_H) as usize,
        );
        let mut preview_items = Vec::new();
        for (id, icon) in windows
            .into_iter()
            .take(door_window::shell_view::MAX_PREVIEW)
        {
            let Ok(info) = self.engine.window_info(id) else {
                continue;
            };
            if info.workspace != ws || info.state == g::WindowState::Closing {
                continue;
            }
            let title = alloc::string::String::from(info.title);
            let (size, pixels) = match self.engine.snapshot(id) {
                Some(snap) => door_window::shell_view::thumbnail(
                    snap.bytes,
                    snap.descriptor.stride_bytes,
                    snap.descriptor.width as usize,
                    snap.descriptor.height as usize,
                    tw,
                    th,
                ),
                None => ((0, 0), Vec::new()),
            };
            preview_items.push(door_window::shell_view::PreviewItem {
                id: id.0,
                title,
                icon,
                size,
                pixels,
            });
        }
        if preview_items.is_empty() {
            return;
        }
        let anchor = layout.slot(Slot::App(slot), slots);
        let rect = door_window::shell_view::preview_rect(layout, anchor, preview_items.len());
        if self.preview_notes > 0 {
            self.preview_notes -= 1;
            let _ = door_sys::write(
                format!(
                    "DOOR_P14 SHELL PREVIEW slot={slot} windows={}\n",
                    preview_items.len()
                )
                .as_bytes(),
            );
        }
        self.shell.preview = Some(door_window::shell_view::Preview {
            rect,
            items: preview_items,
        });
        self.engine.invalidate();
    }
    pub(super) fn preview_action(&mut self, hit: door_window::shell_view::PreviewHit) {
        use door_window::shell_view::PreviewHit;
        match hit {
            PreviewHit::Window(id) => {
                let id = g::WindowId(id);
                self.hide_preview();
                if self.activate_desktop_window(id) {
                    self.manual_focus(Some(id));
                }
            }
            PreviewHit::Close(id) => {
                let id = g::WindowId(id);
                self.hide_preview();
                if let Ok(owner) = self.engine.window_info(id).map(|w| w.owner) {
                    let _ = self.close(owner, id);
                }
            }
            PreviewHit::Inside => (),
        }
        self.bar_dirty = true;
    }
}

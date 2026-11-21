//! Window-server glue for door.clipboard and door.dnd (`native-ui-v1.md`):
//! request dispatch with authenticated environment, events, the trusted
//! pointer during drags, and the Win+V clipboard history panel. Policy lives
//! in the host-tested `door_window::{clipboard, dnd, clipboard_history}`.
use super::*;
use door_abi::{HandleKind, Transfer, rights};
use door_service_protocol::{clipboard as cb, dnd as d};
use door_system_services::server;
use door_window::clipboard_history::{Key, Target};
/// Requests see the server's trusted state, never wire claims.
struct Env<'a> {
    engine: &'a g::Engine,
    locked: bool,
    /// The caller is a workspace app (the Settings app lives there).
    workspace: bool,
    external: usize,
}
impl door_window::clipboard::Env for Env<'_> {
    fn locked(&self) -> bool {
        self.locked
    }
    fn focused(&self, owner: g::Owner) -> bool {
        self.engine
            .focused_window()
            .and_then(|id| self.engine.window_info(id).ok())
            .is_some_and(|info| info.owner == owner)
    }
    fn owns(&self, owner: g::Owner, window: u64) -> bool {
        self.engine
            .window_info(g::WindowId(window))
            .is_ok_and(|info| info.owner == owner && info.state != g::WindowState::Closing)
    }
    fn settings_client(&self, _: g::Owner) -> bool {
        self.workspace
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
    fn external_bytes(&self) -> usize {
        self.external
    }
    fn now(&self) -> u64 {
        door_sys::time().unwrap_or(0)
    }
}
struct Start<'a> {
    env: &'a Env<'a>,
    /// Primary button held, and the window under (or capturing) the pointer.
    held: bool,
    under: Option<g::WindowId>,
}
impl door_window::dnd::StartEnv for Start<'_> {
    fn locked(&self) -> bool {
        self.env.locked
    }
    fn owns(&self, owner: g::Owner, window: u64) -> bool {
        door_window::clipboard::Env::owns(self.env, owner, window)
    }
    fn gesture(&self, _: g::Owner, window: u64) -> bool {
        self.held && self.under == Some(g::WindowId(window))
    }
}
fn reply(r: door_window::clipboard::Reply) -> Response {
    match r {
        door_window::clipboard::Reply::Empty => Response::empty(),
        door_window::clipboard::Reply::Bytes(b) => Response::bytes(&b),
    }
}
impl Desktop {
    /// HELLO granted these native-ui capabilities.
    pub(super) fn native_ui_hello(&mut self, owner: g::Owner, features: u32) {
        if features & cb::CAP_CLIPBOARD != 0 {
            self.shell.clipboard.listen(owner);
        }
        if features & d::CAP_DRAG_DROP != 0 {
            self.shell.dnd.grant(owner);
        }
    }
    /// door.clipboard request (opcodes `cb::FIRST..=cb::LAST`).
    pub(super) fn clipboard_request(
        &mut self,
        peer: Identity,
        owner: g::Owner,
        op: u32,
        b: &[u8],
        t: &[Transfer],
    ) -> Result<Response, Code> {
        let section = match t {
            [] => None,
            [one] if op == cb::WRITE_SECTION => {
                let info = door_sys::inspect(one.handle).map_err(server::status)?;
                if info[0] != HandleKind::Section as u64 || info[1] & rights::READ == 0 {
                    return Err(Code::Denied);
                }
                Some(one.handle)
            }
            _ => return Err(Code::InvalidRequest),
        };
        let mut env = Env {
            engine: &self.engine,
            locked: self.shell.locked,
            workspace: peer.manifest == door_abi::platform::manifest::WORKSPACE,
            external: self.shell.dnd.held_bytes(),
        };
        let result = self
            .shell
            .clipboard
            .request(&mut env, owner, op, b, section)
            .map(reply);
        // A background copy reorders an open Win+V panel: repaint it so
        // what the user aims at is what the next hit test sees.
        if op == cb::COMMIT && result.is_ok() && self.shell.clipboard_panel.open {
            self.bar_dirty = true;
            self.engine.invalidate();
        }
        result
    }
    /// Position of a Win+V item (hover keys, which must not grow with ids).
    pub(super) fn clipboard_index(&self, id: u32) -> usize {
        let items = self.shell.clipboard.history();
        items.iter().position(|h| h.id == id).unwrap_or(items.len())
    }
    /// door.dnd request (opcodes `d::FIRST..=d::LAST`).
    pub(super) fn dnd_request(
        &mut self,
        peer: Identity,
        owner: g::Owner,
        op: u32,
        b: &[u8],
    ) -> Result<Response, Code> {
        let held = self.buttons & 1 != 0;
        let under = self
            .engine
            .captured_window()
            .or_else(|| self.engine.hit_test(self.x, self.y).map(|h| h.0));
        let env = Env {
            engine: &self.engine,
            locked: self.shell.locked,
            workspace: peer.manifest == door_abi::platform::manifest::WORKSPACE,
            external: self.shell.dnd.held_bytes(),
        };
        let start = Start {
            env: &env,
            held,
            under,
        };
        let result =
            self.shell
                .dnd
                .request(&start, &mut self.shell.clipboard, &env, owner, op, b)?;
        if op == d::START {
            // The server owns the pointer from here: the source's capture
            // ends (its START reply is the release notification).
            if let Some(id) = self.engine.captured_window()
                && self.engine.window_info(id).is_ok_and(|w| w.owner == owner)
            {
                let _ = self.engine.release_capture(owner, id);
            }
            self.dnd_motion(door_sys::time().unwrap_or(0));
        }
        Ok(reply(result))
    }
    /// Pending clipboard/drag events for `owner` (before window events).
    pub(super) fn take_native_event(&mut self, owner: g::Owner) -> Option<w::Event> {
        self.shell
            .dnd
            .take_event(owner)
            .or_else(|| self.shell.clipboard.take_event(owner))
            .or_else(|| self.accessibility_event(owner))
            .or_else(|| self.tray.notify.take_event(owner))
    }
    pub(super) fn native_ui_exited(&mut self, owner: g::Owner) {
        self.shell.clipboard.owner_exited(owner);
        self.shell.dnd.owner_exited(owner);
        self.accessibility_exited(owner);
        if !self.tray.notify.is_empty() {
            self.tray.notify.exited(owner);
            if self.tray.notify.is_empty()
                && matches!(self.tray.open, Some(door_window::tray::Flyout::Hidden(_)))
            {
                self.tray.open = None;
            }
            self.engine.invalidate();
        }
    }
    /// What a drag is over: a window's content, a taskbar app button, or
    /// shell UI/desktop (nothing).
    fn dnd_hover(&self) -> door_window::dnd::Hover {
        use door_window::dnd::Hover;
        if self.overlay_contains() {
            let (x, y) = self.logical_pointer();
            let layout = self.taskbar_layout();
            return match layout.hit_slot(x, y, self.taskbar_slot_count()) {
                Some(door_window::taskbar::Slot::App(i)) => Hover::Taskbar(i as u32),
                _ => Hover::Nothing,
            };
        }
        match self.engine.hit_test(self.x, self.y) {
            Some((id, g::HitRegion::Client)) => match self.engine.window_info(id) {
                Ok(info) => Hover::Window {
                    owner: info.owner,
                    window: id.0,
                    x: self.x - info.content.x(),
                    y: self.y - info.content.y(),
                },
                Err(_) => Hover::Nothing,
            },
            _ => Hover::Nothing,
        }
    }
    /// Pointer motion or a modifier change during a drag.
    pub(super) fn dnd_motion(&mut self, now: u64) {
        let hover = self.dnd_hover();
        self.shell.dnd.motion(hover, self.key_modifiers, now);
        self.dnd_feedback_moved();
    }
    /// Repaint the effect label at the pointer.
    fn dnd_feedback_moved(&mut self) {
        let feedback = self.shell.dnd.feedback();
        let badge = feedback.and_then(|f| {
            door_window::clipboard_view::feedback_rect(
                &self.fonts,
                &f,
                self.logical_pointer(),
                self.taskbar_layout().output(),
            )
        });
        if badge != self.shell.dnd_badge || feedback != self.shell.dnd_feedback {
            self.shell.dnd_badge = badge;
            self.shell.dnd_feedback = feedback;
            self.bar_dirty = true;
            self.engine.invalidate();
        }
    }
    /// Release of the primary button during a drag.
    pub(super) fn dnd_release(&mut self, now: u64) {
        self.shell.dnd.release(now);
        self.shell.dnd_badge = None;
        self.shell.dnd_feedback = None;
        self.bar_dirty = true;
        self.engine.invalidate();
    }
    /// Esc, a lock or an input reset.
    pub(super) fn dnd_cancel(&mut self) {
        if self.shell.dnd.active() {
            self.shell.dnd.cancel();
            self.shell.dnd_badge = None;
            self.shell.dnd_feedback = None;
            self.bar_dirty = true;
            self.engine.invalidate();
        }
    }
    /// A key during a drag: Esc cancels, modifiers change the effect,
    /// everything else is swallowed (the OLE modal loop owns the keyboard).
    pub(super) fn dnd_key(&mut self, scan: u32, down: bool) {
        if down && scan == 1 {
            self.dnd_cancel();
        } else if matches!(scan, 29 | 0x11d | 42 | 54 | 56 | 0x138) {
            self.dnd_motion(door_sys::time().unwrap_or(0));
        }
    }
    /// Per-loop drag timing: taskbar hover activation and drop expiry.
    pub(super) fn dnd_tick(&mut self) {
        let now = door_sys::time().unwrap_or(0);
        if let Some(slot) = self.shell.dnd.tick(now)
            && let Some(button) = self.taskbar_buttons().get(slot as usize).copied()
        {
            self.bring_app_forward(button.app);
        }
    }
    /// Windows: a drag resting on a taskbar button brings that app's window
    /// forward (never minimizes it, never launches the app).
    fn bring_app_forward(&mut self, app: desktop::App) {
        let ws = self.engine.active_workspace();
        let top = self
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
            .map(|w| w.id)
            .last();
        if let Some(id) = top {
            self.activate_desktop_window(id);
            self.manual_focus(Some(id));
            self.engine.invalidate();
        }
    }
    /// Win+V.
    pub(super) fn toggle_clipboard_panel(&mut self) {
        if self.shell.clipboard_panel.open {
            self.close_clipboard_panel(true);
            return;
        }
        if self.shell.locked {
            return;
        }
        // Windows opens the panel at the focused app's text caret.
        let anchor = self.text_input_caret();
        self.close_menu();
        self.close_start();
        self.tray.open = None;
        self.overview = None;
        self.shell.run.open = false;
        self.remember_shell_focus();
        self.engine.clear_focus();
        self.manual_focus(None);
        self.shell.clipboard_panel.show_at(anchor);
        self.bar_dirty = true;
        self.engine.invalidate();
    }
    pub(super) fn close_clipboard_panel(&mut self, restore: bool) {
        self.shell.clipboard_panel.open = false;
        self.shell.clipboard_panel.menu = None;
        if restore {
            self.restore_shell_focus();
        }
        self.bar_dirty = true;
        self.engine.invalidate();
    }
    pub(super) fn clipboard_panel_contains(&self, x: i32, y: i32) -> bool {
        self.shell.clipboard_panel.open
            && door_window::clipboard_view::geometry(
                self.taskbar_layout(),
                &self.shell.clipboard_panel,
            )
            .panel
            .contains(x, y)
    }
    pub(super) fn clipboard_panel_hit(&self, x: i32, y: i32) -> Option<Target> {
        let c = &self.shell.clipboard;
        self.shell.clipboard_panel.hit(
            &door_window::clipboard_view::geometry(
                self.taskbar_layout(),
                &self.shell.clipboard_panel,
            ),
            c.history_enabled(),
            c.history(),
            x,
            y,
        )
    }
    pub(super) fn clipboard_panel_key(&mut self, scan: u32) -> bool {
        let g = door_window::clipboard_view::geometry(
            self.taskbar_layout(),
            &self.shell.clipboard_panel,
        );
        let on = self.shell.clipboard.history_enabled();
        let items = self.shell.clipboard.history();
        match self.shell.clipboard_panel.key(&g, scan, on, items) {
            Key::Close => {
                self.close_clipboard_panel(true);
                self.shortcut_keys |= 4;
            }
            // By id: a missing item (deleted, or evicted by a background
            // copy) is a no-op.
            Key::Paste(id) => self.paste_history(id),
            Key::Delete(id) => {
                self.shell.clipboard.delete(id);
            }
            Key::TurnOn => {
                let _ = self
                    .shell
                    .clipboard
                    .apply_settings(self.shell.clipboard.settings() | cb::SETTING_HISTORY);
            }
            Key::Moved | Key::Ignored => (),
        }
        self.bar_dirty = true;
        self.engine.invalidate();
        true
    }
    pub(super) fn clipboard_panel_wheel(&mut self, notches: i32) -> bool {
        let (x, y) = self.logical_pointer();
        if !self.clipboard_panel_contains(x, y) {
            return false;
        }
        let g = door_window::clipboard_view::geometry(
            self.taskbar_layout(),
            &self.shell.clipboard_panel,
        );
        let items = self.shell.clipboard.history().len();
        if self.shell.clipboard_panel.wheel(&g, items, notches) {
            self.bar_dirty = true;
            self.engine.invalidate();
        }
        true
    }
    pub(super) fn clipboard_target(&mut self, target: Target) {
        match target {
            Target::TurnOn => {
                let _ = self
                    .shell
                    .clipboard
                    .apply_settings(self.shell.clipboard.settings() | cb::SETTING_HISTORY);
            }
            Target::ClearAll => self.shell.clipboard.clear_history(),
            Target::Item(id) => {
                self.paste_history(id);
                return;
            }
            Target::More(id) => {
                let menu = &mut self.shell.clipboard_panel.menu;
                *menu = if *menu == Some(id) { None } else { Some(id) };
            }
            Target::Pin | Target::Delete => {
                if let Some(id) = self.shell.clipboard_panel.menu.take() {
                    if target == Target::Pin {
                        self.shell.clipboard.toggle_pin(id);
                    } else {
                        self.shell.clipboard.delete(id);
                    }
                }
            }
            Target::Panel => self.shell.clipboard_panel.menu = None,
        }
        self.bar_dirty = true;
        self.engine.invalidate();
    }
    /// Win+V choice: the item becomes the clipboard, the panel closes, focus
    /// returns to the app that had it and that app receives Ctrl+V.
    fn paste_history(&mut self, id: u32) {
        if !self.shell.clipboard.paste_history(id) {
            return;
        }
        self.close_clipboard_panel(true);
        if self.engine.focused_window().is_none() {
            return;
        }
        let now = door_sys::time().unwrap_or(0);
        let control = door_abi::graphics::CONTROL << 9;
        for (code, down) in [
            (29 | control, true),
            (47 | control, true),
            (47 | control, false),
            (29, false),
        ] {
            let _ = self.engine.route_input(g::Input::Key { code, down }, now);
        }
    }
}

//! Trusted input policy. Client messages cannot synthesize activation or grant
//! capture to another owner. A saturated consumer loses its complete window state.
use super::*;
use door_abi::graphics as d;
impl Desktop {
    pub(super) fn route(&mut self, event: g::Input, time: u64) {
        match self.engine.route_input(event, time) {
            Ok(_) => (),
            Err(g::Error::QueueFull) => {
                let target = match event {
                    g::Input::Key { .. } => self.engine.focused_window(),
                    g::Input::PointerMove { x, y }
                    | g::Input::PointerButton { x, y, .. }
                    | g::Input::Wheel { x, y, .. } => self
                        .engine
                        .captured_window()
                        .or_else(|| self.engine.hit_test(x, y).map(|h| h.0)),
                };
                if let Some(owner) =
                    target.and_then(|id| self.engine.window_info(id).ok().map(|w| w.owner))
                {
                    // A Win32 client that stopped retrieving is a ghost, not
                    // a fault: Windows drops input beyond a full queue and
                    // terminates only on End task.
                    if self.shell.hang.is_client(owner) {
                        return;
                    }
                    self.terminations.push(owner, time);
                    self.evict(owner);
                    let _ = door_sys::write(b"DOOR_P11 INPUT_REVOKED queue-full\n");
                }
            }
            Err(_) => (),
        }
    }
    /// Wheel detents at the pointer: the shell first (Start, flyouts, Task
    /// View, taskbar), then the window under the pointer (or the capture
    /// owner) without activating it. A window drag swallows the wheel.
    fn wheel(&mut self, notches: i32, time: u64) {
        if notches == 0 || self.session.is_none() || self.shell_wheel(notches) {
            return;
        }
        // Only owners that negotiated CAP_WHEEL receive wheel events.
        let target = self
            .engine
            .captured_window()
            .or_else(|| self.engine.hit_test(self.x, self.y).map(|hit| hit.0));
        // "Scroll inactive windows when hovering over them" off: only the
        // focused window scrolls.
        if self
            .shell
            .prefs
            .get(door_service_protocol::preferences::MOUSE)
            & door_service_protocol::preferences::mouse::NO_INACTIVE_SCROLL
            != 0
            && target.is_some()
            && target != self.engine.focused_window()
        {
            return;
        }
        let negotiated = target
            .and_then(|id| self.engine.window_info(id).ok().map(|w| w.owner))
            .is_some_and(|owner| {
                self.clients
                    .iter()
                    .any(|c| c.owner == owner && c.wheel_events)
            });
        if negotiated && self.drag.is_none() {
            self.route(
                g::Input::Wheel {
                    x: self.x,
                    y: self.y,
                    delta: notches,
                },
                time,
            );
        }
    }
    fn motion(&mut self, time: u64) {
        if self.session.is_none() {
            return;
        }
        self.auto_hide_motion();
        // A drag and drop owns the pointer (door.dnd): no window sees it.
        if self.shell.dnd.active() {
            self.dnd_motion(time);
            return;
        }
        if self.drag.is_none() && self.engine.captured_window().is_none() && self.overlay_contains()
        {
            // Shell UI took the pointer: the window under it was left.
            self.engine.pointer_leave(time);
            return;
        }
        if let Some(mut drag) = self.drag {
            if !self.engine.window_info(drag.id).is_ok_and(|info| {
                info.owner == drag.owner
                    && info.epoch == drag.epoch
                    && info.visible
                    && matches!(
                        info.state,
                        g::WindowState::Normal | g::WindowState::Maximized
                    )
            }) {
                self.drag = None;
                return;
            }
            let scale = self.engine.scale();
            let len = |logical: u32| scale.len(logical) as i32;
            let dx = self.x - drag.x;
            let dy = self.y - drag.y;
            if drag.edges == 0 && !drag.moved && dx.abs() < len(4) && dy.abs() < len(4) {
                return;
            }
            if drag.restore_pending {
                match self.engine.restore_title_drag(
                    drag.owner,
                    drag.id,
                    (drag.x, drag.y),
                    (self.x, self.y),
                ) {
                    Ok(rect) => {
                        drag.rect = rect;
                        drag.x = self.x;
                        drag.y = self.y;
                        drag.epoch = self.engine.window_info(drag.id).unwrap().epoch;
                        drag.restore_pending = false;
                        drag.moved = true;
                        self.drag = Some(drag);
                        self.attachments.retain(|a| a.window != drag.id);
                    }
                    Err(g::Error::QueueFull) => (),
                    Err(_) => self.drag = None,
                }
                return;
            }
            drag.moved = true;
            self.drag = Some(drag);
            if drag.edges != 0 {
                let (mut left, mut top) = (drag.rect.x(), drag.rect.y());
                let (mut right, mut bottom) = (
                    left + drag.rect.width() as i32,
                    top + drag.rect.height() as i32,
                );
                if drag.edges & 1 != 0 {
                    left = (left + dx).clamp(
                        right - self.engine.output_rect().width() as i32,
                        right - len(240),
                    );
                }
                if drag.edges & 2 != 0 {
                    right = (right + dx).clamp(
                        left + len(240),
                        left + self.engine.output_rect().width() as i32,
                    );
                }
                if drag.edges & 4 != 0 {
                    top = (top + dy).clamp(
                        bottom - self.engine.output_rect().height() as i32,
                        bottom - len(120),
                    );
                }
                if drag.edges & 8 != 0 {
                    bottom = (bottom + dy).clamp(
                        top + len(120),
                        top + self.engine.output_rect().height() as i32,
                    );
                }
                // Only the newest size is configured, once per frame
                // (compositor::apply_resize), never once per motion packet.
                if let Ok(r) = g::Rect::new(left, top, (right - left) as u32, (bottom - top) as u32)
                {
                    self.resize_pending = Some((drag.owner, drag.id, r));
                }
            } else {
                let out = self.engine.work_area();
                let x = (drag.rect.x() + dx).clamp(
                    out.x() + len(80) - drag.rect.width() as i32,
                    out.right() - len(80),
                );
                let y = (drag.rect.y() + dy).clamp(out.y(), out.bottom() - len(32));
                let _ = self.engine.move_window(drag.owner, drag.id, x, y);
                self.snap_drag_motion(drag.id, drag.owner, drag.rect);
                self.shake_motion(drag.id, self.x, time);
            }
        } else {
            self.route(
                g::Input::PointerMove {
                    x: self.x,
                    y: self.y,
                },
                time,
            );
        }
    }
    /// System > Multitasking > "Title bar window shake": four quick direction
    /// changes of a dragged window (each at least 24 px, within a second)
    /// minimize every other window of the desktop.
    fn shake_motion(&mut self, id: g::WindowId, x: i32, time: u64) {
        if self
            .shell
            .prefs
            .get(door_service_protocol::preferences::MULTITASK)
            & door_service_protocol::preferences::multitask::SHAKE
            == 0
        {
            return;
        }
        let now = door_sys::time().unwrap_or(time);
        let s = &mut self.shell.shake;
        if s.window != Some(id) || now.saturating_sub(s.started) > 100 {
            *s = desktop::Shake {
                window: Some(id),
                anchor: x,
                last: x,
                direction: 0,
                turns: 0,
                started: now,
            };
            return;
        }
        let step = x - s.last;
        s.last = x;
        let direction = step.signum();
        if direction == 0 {
            return;
        }
        let far = (x - s.anchor).abs() >= self.engine.scale().len(24) as i32;
        if direction != s.direction && (far || s.direction == 0) {
            s.turns += u8::from(s.direction != 0);
            s.direction = direction;
            s.anchor = x;
        }
        if s.turns >= 4 {
            self.shell.shake = desktop::Shake::default();
            let ws = self.engine.active_workspace();
            let others: Vec<(g::Owner, g::WindowId)> = self
                .engine
                .windows()
                .filter(|w| {
                    w.id != id
                        && w.role == g::Role::TopLevel
                        && w.workspace == ws
                        && w.state != g::WindowState::Minimized
                        && w.state != g::WindowState::Closing
                })
                .map(|w| (w.owner, w.id))
                .collect();
            for (owner, other) in others {
                let _ = self
                    .engine
                    .request_state(owner, other, g::WindowState::Minimized);
            }
            let _ = door_sys::write(b"DOOR_P14 SHAKE minimized-others=1\n");
        }
    }
    /// "Automatically hide the taskbar": the pointer at the taskbar's edge
    /// reveals it; leaving it (with no Start, flyout or menu open) hides it.
    fn auto_hide_motion(&mut self) {
        let layout = self.taskbar_layout();
        if !layout.auto_hide() {
            return;
        }
        let (x, y) = self.logical_pointer();
        let revealed = self.shell.taskbar_revealed;
        let keep = self.shell.start_motion.open
            || self.tray.open.is_some()
            || self.shell.menu.is_some()
            || self.overview.is_some();
        let want = if revealed {
            keep || layout.bar().contains(x, y)
        } else {
            layout.reveal_edge().contains(x, y)
        };
        if want != revealed {
            self.shell.taskbar_revealed = want;
            self.bar_dirty = true;
            self.engine.invalidate();
        }
    }
    fn button(&mut self, button: u8, down: bool, time: u64) {
        // Signed out or locked: only the trusted credential gate sees input.
        if self.session.is_none() || self.shell.locked {
            self.gate_button(button, down);
            return;
        }
        // User Account Control's secure desktop: every press is its own.
        if self.shell.uac.is_some() {
            if button == 0 {
                let (x, y) = self.logical_pointer();
                let area = self.taskbar_layout().output();
                let answer = self
                    .shell
                    .uac
                    .as_mut()
                    .and_then(|c| c.button(area, x, y, down));
                if let Some(answer) = answer {
                    self.uac_answer(answer);
                }
            }
            self.engine.invalidate();
            return;
        }
        // Drag and drop: the primary release drops, other buttons are
        // swallowed until it ends.
        if self.shell.dnd.active() {
            if button == 0 && !down {
                self.dnd_release(time);
            }
            return;
        }
        // Shell context menus (right button) follow; a lock never gets here.
        // The "not responding" prompt takes presses inside it; others pass.
        if self.shell.hang.prompt().is_some() && self.drag.is_none() {
            let (x, y) = self.logical_pointer();
            let area = self.prompt_area();
            let (consumed, answer) = if button == 0 {
                self.shell.hang.button(area, x, y, down)
            } else {
                (door_window::not_responding::hit(area, x, y).is_some(), None)
            };
            if let Some(answer) = answer {
                self.not_responding_answer(answer);
            }
            if consumed {
                self.engine.invalidate();
                return;
            }
        }
        // The Security Alert takes presses inside it (and the keyboard with
        // them); a press elsewhere goes on and leaves it open.
        if self.shell.firewall_alert.is_some() && self.drag.is_none() {
            let (x, y) = self.logical_pointer();
            let area = self.prompt_area();
            let (consumed, answer) = match self.shell.firewall_alert.as_mut() {
                Some(alert) if button == 0 => alert.button(area, x, y, down),
                _ => (door_window::firewall_alert::hit(area, x, y).is_some(), None),
            };
            if down {
                self.shell.firewall_alert_focused = consumed;
            }
            if let Some(answer) = answer {
                self.firewall_answer(answer);
            }
            if consumed {
                self.engine.invalidate();
                return;
            }
        }
        // Doors extra (Windhawk "Middle click to close"): the middle button
        // on a taskbar app closes its windows.
        if button == 2
            && down
            && self.drag.is_none()
            && self
                .shell
                .prefs
                .get(door_service_protocol::preferences::TASKBAR)
                & door_service_protocol::preferences::taskbar::MIDDLE_CLOSE
                != 0
        {
            let layout = self.taskbar_layout();
            let (x, y) = self.logical_pointer();
            if let Some(door_window::taskbar::Slot::App(i)) =
                layout.hit_slot(x, y, self.taskbar_slot_count())
            {
                if let Some(extra) = self.taskbar_extra(i) {
                    self.close_extra(extra.key);
                } else if let Some(button) = self.taskbar_buttons().get(i).copied() {
                    let windows: Vec<_> = desktop::items(&self.engine, &self.app_classes)
                        .into_iter()
                        .filter(|w| w.app == button.app)
                        .map(|w| (w.owner, w.id))
                        .collect();
                    for (owner, id) in windows {
                        let _ = self.close(owner, id);
                    }
                }
                return;
            }
        }
        if button == 1 && self.drag.is_none() && self.desktop_secondary(down) {
            return;
        }
        if let Some(action) = self.overview_hit() {
            if button == 0 {
                if down {
                    if let Some(view) = self.overview.as_mut() {
                        view.pressed = Some(action);
                        view.press_origin = Some((self.x, self.y));
                    }
                } else {
                    let (pressed, origin) = self
                        .overview
                        .as_mut()
                        .map(|v| (v.pressed.take(), v.press_origin.take()))
                        .unwrap_or((None, None));
                    if pressed == Some(action) {
                        self.overview_action(action);
                    } else if let (
                        Some(door_window::overview::Action::Window(window)),
                        door_window::overview::Action::Desktop(workspace),
                        Some((x, y)),
                    ) = (pressed, action, origin)
                        && (self.x.abs_diff(x) > self.engine.scale().len(4)
                            || self.y.abs_diff(y) > self.engine.scale().len(4))
                        && let Ok(info) = self.engine.window_info(window)
                    {
                        let _ = self.engine.move_to_workspace(info.owner, window, workspace);
                        self.engine.invalidate();
                    }
                }
            } else if down {
                self.overview_action(door_window::overview::Action::Dismiss);
            }
            return;
        }
        if button == 0
            && !down
            && let Some((id, region)) = self.caption_press.take()
        {
            if !self.overlay_contains()
                && self.engine.hit_test(self.x, self.y) == Some((id, region))
            {
                self.caption_action(id, region);
            }
            return;
        }
        // Win32 modal owners (CAP_WIN32): a press on a disabled window
        // activates its modal window and reaches no client.
        if down
            && self.drag.is_none()
            && self.engine.captured_window().is_none()
            && !self.overlay_contains()
            && let Some((id, _)) = self.engine.hit_test(self.x, self.y)
            && !self.win32.enabled(id.0)
        {
            if let Some(target) = self.win32.modal_target(id.0) {
                let _ = self.engine.activate(g::WindowId(target));
            }
            return;
        }
        if down
            && button != 0
            && (self.launcher || self.tray.open.is_some())
            && !self.overlay_contains()
        {
            self.launcher = false;
            self.tray.open = None;
            self.shell_focus = None;
            self.engine.invalidate();
        }
        // The On-Screen Keyboard's windows never take focus.
        let keyboard_press = down
            && !self.overlay_contains()
            && self
                .engine
                .hit_test(self.x, self.y)
                .is_some_and(|hit| self.keyboard_window(hit.0));
        if down && !keyboard_press {
            let focus = (!self.overlay_contains())
                .then(|| self.engine.hit_test(self.x, self.y).map(|hit| hit.0))
                .flatten();
            self.manual_focus(focus);
        }
        if button != 0 && self.drag.is_none() && self.engine.captured_window().is_none() {
            let bit = 1u32 << button;
            if !down && self.overlay_buttons & bit != 0 {
                self.overlay_buttons &= !bit;
                return;
            }
            if self.overlay_contains() || down && self.launcher {
                if down {
                    self.overlay_buttons |= bit;
                }
                return;
            }
        }
        if button == 0 && self.desktop_button(down, time) {
            return;
        }
        if down && self.engine.captured_window().is_none() {
            match self.engine.hit_test(self.x, self.y) {
                None => self.engine.clear_focus(),
                Some((id, region)) if region != g::HitRegion::Client => {
                    if self.engine.window_info(id).is_ok_and(|w| {
                        self.clients
                            .iter()
                            .any(|c| c.owner == w.owner && c.transient_events)
                    }) {
                        self.engine.dismiss_transients(id);
                    }
                }
                _ => (),
            }
        }
        if button == 0 {
            if !down && self.drag.is_some() {
                // The release lands on the final interactive size.
                self.apply_resize();
            }
            if !down && let Some(drag) = self.drag.take() {
                if drag.edges == 0 && drag.moved && self.snap_drag_drop() {
                    return;
                }
                if drag.edges == 0
                    && drag.moved
                    && !drag.restore_pending
                    && !self.snap_off()
                    && self
                        .engine
                        .window_info(drag.id)
                        .is_ok_and(|w| w.owner == drag.owner && w.epoch == drag.epoch && w.visible)
                    && let Some(snap) = door_window::policy::edge_snap(
                        self.engine.work_area(),
                        self.engine.scale(),
                        self.x,
                        self.y,
                    )
                    && self
                        .engine
                        .snap_window(drag.owner, drag.id, snap, Some(drag.rect))
                        .is_ok()
                {
                    self.attachments.retain(|a| a.window != drag.id);
                    // Shell: Snap Assist fills the rest of the screen.
                    self.assist_after_snap(snap, drag.id);
                    return;
                }
                // A Win32 window tracks its own position (GetWindowRect,
                // WM_MOVE, WM_EXITSIZEMOVE geometry): a trusted move ends
                // with a CONFIGURE carrying the final rectangle.
                if drag.edges == 0
                    && drag.moved
                    && self.clients.iter().any(|c| {
                        c.owner == drag.owner
                            && c.manifest == door_abi::platform::manifest::WINDOWS_GUI
                    })
                    && let Ok(info) = self.engine.window_info(drag.id)
                    && info.owner == drag.owner
                    && info.outer != drag.rect
                {
                    let outer = info.outer;
                    let _ = self.engine.configure(drag.owner, drag.id, outer);
                }
                return;
            }
            if down && let Some((id, region)) = self.engine.hit_test(self.x, self.y) {
                if !self.keyboard_window(id) {
                    let _ = self.engine.activate(id);
                }
                if let Ok(info) = self.engine.window_info(id) {
                    let owner = info.owner;
                    let rect = info.outer;
                    let state = info.state;
                    let epoch = info.epoch;
                    let snapped = info.snapped;
                    match region {
                        g::HitRegion::TitleBar | g::HitRegion::Resize => {
                            if state == g::WindowState::Normal
                                || region == g::HitRegion::TitleBar
                                    && state == g::WindowState::Maximized
                            {
                                self.drag = Some(Drag {
                                    id,
                                    owner,
                                    rect,
                                    x: self.x,
                                    y: self.y,
                                    edges: if region == g::HitRegion::Resize {
                                        g::CursorIcon::resize_edges_scaled(
                                            rect,
                                            self.x,
                                            self.y,
                                            self.engine.scale(),
                                        )
                                    } else {
                                        0
                                    },
                                    epoch,
                                    restore_pending: region == g::HitRegion::TitleBar
                                        && (snapped || state == g::WindowState::Maximized),
                                    moved: false,
                                });
                            }
                            return;
                        }
                        g::HitRegion::Close | g::HitRegion::Minimize | g::HitRegion::Maximize => {
                            self.caption_press = Some((id, region));
                            return;
                        }
                        g::HitRegion::Client => (),
                    }
                }
            }
        }
        // The held primary press a Win32 client may continue as MOVE_RESIZE.
        if button == 0 {
            self.win32_press = if down {
                self.engine
                    .hit_test(self.x, self.y)
                    .filter(|(_, region)| *region == g::HitRegion::Client)
                    .map(|(id, _)| door_window::win32::Press {
                        window: id.0,
                        serial: time,
                    })
            } else {
                None
            };
        }
        self.route(
            g::Input::PointerButton {
                button,
                down,
                x: self.x,
                y: self.y,
            },
            time,
        );
    }
    pub(super) fn caption_action(&mut self, id: g::WindowId, region: g::HitRegion) {
        if !self.win32.enabled(id.0) {
            if let Some(target) = self.win32.modal_target(id.0) {
                let _ = self.engine.activate(g::WindowId(target));
            }
            return;
        }
        // A greyed Minimize or Maximize does nothing.
        if !self.engine.caption_button_enabled(id, region) {
            return;
        }
        let Ok(info) = self.engine.window_info(id) else {
            return;
        };
        let (owner, state, epoch) = (info.owner, info.state, info.epoch);
        match region {
            g::HitRegion::Close => {
                let _ = self.close(owner, id);
            }
            g::HitRegion::Minimize => {
                let _ = self
                    .engine
                    .request_state(owner, id, g::WindowState::Minimized);
            }
            g::HitRegion::Maximize => {
                let changed = if state == g::WindowState::Maximized {
                    self.engine.restore_window(owner, id)
                } else {
                    self.engine
                        .request_state(owner, id, g::WindowState::Maximized)
                };
                if changed.is_ok()
                    && self
                        .engine
                        .window_info(id)
                        .is_ok_and(|window| window.epoch != epoch)
                {
                    self.attachments
                        .retain(|attachment| attachment.window != id);
                }
            }
            _ => (),
        }
    }
    pub(super) fn input(&mut self) -> Result<(), Status> {
        // Coalesce motion bursts before hover/hit-test/drag work; button,
        // key and reset events keep their order (door_window::input_batch).
        // Starting from the current buttons keeps a release (or press) at its
        // own position when motion follows it in the same batch.
        let mut batch = if self.buttons_unknown {
            door_window::input_batch::Batch::new()
        } else {
            door_window::input_batch::Batch::with_buttons(self.buttons)
        };
        for _ in 0..door_window::input_batch::READS {
            if batch.is_full() {
                break;
            }
            match door_sys::display_input(self.display) {
                Ok(v) => {
                    batch.push(v);
                }
                Err(Status::WouldBlock) => break,
                Err(e) => return Err(e),
            }
        }
        if !batch.events().is_empty() {
            // Any input restarts "If you've been away" (sign-in options).
            self.shell.idle.input(door_sys::time().unwrap_or(0));
        }
        for event in batch.events().iter().copied() {
            if event.kind == d::RESET {
                self.key_modifiers = 0;
                if let Some(id) = self.engine.focused_window() {
                    self.engine.dismiss_transients(id);
                }
                self.engine.clear_focus();
                self.drag = None;
                self.dnd_cancel();
                self.resize_pending = None;
                self.caption_press = None;
                self.login_pressed = false;
                self.system_keys.reset();
                self.shell.text_input.reset();
                self.overview = None;
                self.shell_focus = None;
                self.desktop_press = None;
                self.desktop_selection.cancel_gesture();
                self.manual_focus(None);
                self.overlay_buttons = 0;
                self.shortcut_keys = 0;
                self.buttons_unknown = true;
                self.launcher = false;
                self.tray.open = None;
                self.engine.invalidate();
                let _ = door_sys::write(b"DOOR_P12 INPUT RESET state-cancelled=1\n");
                continue;
            }
            if event.kind == d::MOTION {
                // Bluetooth & devices > Mouse: speed, precision, primary
                // button and lines per notch.
                let mouse = self
                    .shell
                    .prefs
                    .get(door_service_protocol::preferences::MOUSE);
                let mut event = event;
                event.flags = door_window::preferences::mouse_buttons(mouse, event.flags);
                let (dx, dy) = door_window::preferences::pointer_motion(
                    mouse,
                    event.x,
                    event.y,
                    &mut self.shell.mouse_rest,
                );
                let notches = door_window::preferences::wheel_notches(
                    mouse,
                    event.wheel(),
                    &mut self.shell.wheel_rest,
                );
                let old_hover = self.hover_target();
                let output = self.engine.output_rect();
                self.x = (self.x + dx).clamp(0, output.width() as i32 - 1);
                self.y = (self.y + dy).clamp(0, output.height() as i32 - 1);
                // Magnifier's view follows the pointer.
                if self.shell.magnifier.is_some() {
                    self.engine.invalidate();
                }
                // The snip overlay owns the pointer while open.
                if self.shell.snip.is_some() && self.session.is_some() {
                    self.snip_pointer(event.flags);
                    self.buttons = event.flags;
                    self.cursor_dirty = true;
                    continue;
                }
                self.wheel(notches, event.timestamp);
                if self.session.is_none() {
                    self.cursor_dirty = true;
                    for button in 0..3 {
                        let bit = 1 << button;
                        if (self.buttons ^ event.flags) & bit != 0 {
                            self.button(button, event.flags & bit != 0, event.timestamp);
                        }
                    }
                    self.buttons = event.flags;
                    continue;
                }
                if let Some(action) = self.overview_hit() {
                    if self
                        .overview
                        .as_mut()
                        .is_some_and(|view| view.hover(action))
                    {
                        self.engine.invalidate();
                    }
                    // Task View covers every window.
                    self.engine.pointer_leave(event.timestamp);
                    self.cursor_dirty = true;
                    for button in 0..3 {
                        let bit = 1 << button;
                        if (self.buttons ^ event.flags) & bit != 0 {
                            self.button(button, event.flags & bit != 0, event.timestamp);
                        }
                    }
                    self.buttons = event.flags;
                    continue;
                }

                self.desktop_selection.motion(self.x, self.y);
                self.shell_motion();
                let hover = self.desktop_icon_hover();
                if hover != self.icon_hover {
                    self.icon_hover = hover;
                    self.invalidate_icons();
                }
                if old_hover != self.hover_target() {
                    self.bar_dirty = true;
                }
                self.cursor_dirty = true;
                self.motion(event.timestamp);
                if self.buttons_unknown {
                    self.buttons = event.flags;
                    self.buttons_unknown = false;
                }
                for button in 0..3 {
                    let bit = 1 << button;
                    if (self.buttons ^ event.flags) & bit != 0 {
                        self.frame_clock
                            .input(door_window::telemetry::cycles(), event.timestamp);
                        self.button(button, event.flags & bit != 0, event.timestamp)
                    }
                }
                self.buttons = event.flags;
            } else {
                self.key_modifiers = event.flags;
                let down = event.flags & d::DOWN != 0;
                if down {
                    self.frame_clock
                        .input(door_window::telemetry::cycles(), event.timestamp);
                }
                if self.session.is_none() || self.shell.locked {
                    self.gate_key(event.code, event.flags);
                    continue;
                }
                // The consent prompt has every key, the Doors key included
                // (releases too: its password box tracks Shift).
                if self.shell.uac.is_some() {
                    let answer = self
                        .shell
                        .uac
                        .as_mut()
                        .and_then(|c| c.key_event(event.code, event.flags));
                    if let Some(answer) = answer {
                        self.uac_answer(answer);
                    }
                    if down {
                        self.engine.invalidate();
                    }
                    continue;
                }
                if self.shell.dnd.active() {
                    self.dnd_key(event.code, down);
                    continue;
                }
                if self.shell.snip.is_some() {
                    self.snip_key(event.code, down);
                    continue;
                }
                // door.text-input: Win+Space and the language hotkeys.
                if self.text_input_hotkey(&event) {
                    continue;
                }
                match self
                    .system_keys
                    .event(event.code, down, event.flags & d::CONTROL != 0)
                {
                    door_window::policy::SystemEvent::Consumed => continue,
                    door_window::policy::SystemEvent::Command(command) => {
                        self.system_command(command);
                        continue;
                    }
                    door_window::policy::SystemEvent::Pass => (),
                }
                if down && event.flags & d::ALT != 0 && event.code == 15 {
                    self.shortcut_keys |= 1;
                    self.task_switch(event.flags & d::SHIFT != 0);
                    continue;
                }
                if !down
                    && matches!(event.code, 56 | 0x138)
                    && event.flags & d::ALT == 0
                    && self.overview.as_ref().is_some_and(|v| v.is_switcher())
                {
                    if let Some(id) = self.overview.as_ref().and_then(|v| v.selected_window()) {
                        self.overview_action(door_window::overview::Action::Window(id));
                    } else {
                        self.overview_action(door_window::overview::Action::Dismiss);
                    }
                    continue;
                }
                if self.overview.is_some() {
                    if !down && event.code == 15 {
                        self.shortcut_keys &= !1;
                    }
                    if down {
                        match event.code {
                            1 => self.overview_action(door_window::overview::Action::Dismiss),
                            28 | 0x11c => {
                                if let Some(id) =
                                    self.overview.as_ref().and_then(|v| v.selected_window())
                                {
                                    self.overview_action(door_window::overview::Action::Window(id));
                                }
                            }
                            15 | 0x14b | 0x14d => {
                                if let Some(view) = self.overview.as_mut() {
                                    view.step(event.code == 0x14b || event.flags & d::SHIFT != 0);
                                }
                                self.engine.invalidate();
                            }
                            _ => (),
                        }
                    }
                    continue;
                }
                // The "not responding" prompt has the keyboard: Esc is Wait,
                // Enter/Space the focused button, Tab/arrows move the focus.
                if down
                    && self.shell.hang.prompt().is_some()
                    && !matches!(event.code, 0x15b | 0x15c)
                {
                    if let Some(answer) = self.shell.hang.key(event.code) {
                        self.not_responding_answer(answer);
                    }
                    self.engine.invalidate();
                    continue;
                }
                // The Security Alert, while it has the keyboard.
                if down
                    && self.shell.firewall_alert_focused
                    && !matches!(event.code, 0x15b | 0x15c)
                    && let Some(alert) = self.shell.firewall_alert.as_mut()
                {
                    let alt = event.flags & door_abi::graphics::ALT != 0;
                    if let Some(answer) = alert.key(event.code, alt) {
                        self.firewall_answer(answer);
                    }
                    self.engine.invalidate();
                    continue;
                }
                // Start search and Run take the session layout's Unicode
                // text and IME commits through their own text-input context
                // (text_input_glue.rs); the Windows key keeps its Start role.
                if self.shell_text_focus() && !matches!(event.code, 0x15b | 0x15c) {
                    self.shell_text_key(&event);
                    continue;
                }
                if down && self.desktop_key(event.code) {
                    continue;
                }
                if !down && (self.launcher || matches!(event.code, 0x15b | 0x15c)) {
                    continue;
                }
                let shortcut = match event.code {
                    1 => 4,
                    15 => 1,
                    62 => 2,
                    _ => 0,
                };
                if !down && self.shortcut_keys & shortcut != 0 {
                    self.shortcut_keys &= !shortcut;
                    continue;
                }
                if down && event.flags & d::ALT != 0 && event.code == 62 {
                    self.shortcut_keys |= 2;
                    if let Some((id, owner)) = self
                        .engine
                        .focused_window()
                        .and_then(|id| self.engine.window_info(id).ok().map(|w| (id, w.owner)))
                    {
                        let _ = self.close(owner, id);
                    }
                    continue;
                }
                // The session layout translates the key; a window with a
                // text-input context gets it as records (text_input_glue.rs),
                // others the legacy code (scan | flags << 9 | ASCII << 16).
                if let Some(code) = self.text_input_key(&event) {
                    // A press aimed at a disabled Win32 owner activates its
                    // modal first; every key, releases included, then reaches
                    // the focused client (`Relations::key_activation`).
                    if let Some(target) = self
                        .engine
                        .focused_window()
                        .and_then(|focus| self.win32.key_activation(focus.0, down))
                    {
                        let _ = self.engine.activate(g::WindowId(target));
                    }
                    self.route(g::Input::Key { code, down }, event.timestamp);
                }
            }
        }
        if self.session.is_some() {
            let covered = self.overlay_contains();
            let (x, y) = if covered { (-1, -1) } else { (self.x, self.y) };
            if self.engine.set_caption_pointer(x, y, self.buttons & 1 != 0)
                && (self.launcher || self.tray.open.is_some())
            {
                self.engine.invalidate();
            }
        }
        Ok(())
    }
}

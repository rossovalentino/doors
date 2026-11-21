//! Compositor scheduling owned by the window server: the transition clock,
//! taskbar anchors for minimize/restore, per-frame interactive resize, the
//! visual preferences (reduce motion/transparency) and the shell's wheel hook.
use super::*;

impl Desktop {
    /// Engine preferences derived from the session's desktop controls.
    /// Contrast themes paint the shell with system colours and replace
    /// acrylic and blur with solid colour, as Windows does.
    pub(super) fn apply_visual_preferences(&mut self) {
        self.engine.set_animations(!self.controls.reduce_motion());
        self.presentation_log
            .animation
            .preference(self.controls.reduce_motion());
        let contrast = self.controls.contrast();
        door_window::paint::set_contrast(contrast);
        self.engine
            .set_reduce_transparency(self.controls.reduce_transparency() || contrast.is_some());
        self.apply_title_accent();
    }
    /// Advance window transitions to the current time. Idle scenes (no
    /// transition running) queue no damage, so they never repaint.
    pub(super) fn advance_transitions(&mut self) -> Result<(), Status> {
        let tick = door_sys::time()?;
        let now = self.transition_clock.now_ms(
            door_window::telemetry::cycles(),
            tick,
            door_abi::platform::MONOTONIC_TICK_MILLIS,
        );
        if self.engine.animations_enabled() {
            self.update_taskbar_anchors();
        }
        self.engine.advance(now);
        Ok(())
    }
    /// Minimize/restore animate toward the window's taskbar button.
    fn update_taskbar_anchors(&mut self) {
        let scale = self.engine.scale();
        // Pinned/unpinned apps reorder taskbar buttons; anchor to the live slot.
        let slots: Vec<_> = door_window::apps::App::ALL
            .iter()
            .map(|app| self.taskbar_anchor(*app))
            .collect();
        let anchors: Vec<_> = self
            .engine
            .windows()
            .filter(|w| w.role == g::Role::TopLevel)
            .map(|w| {
                let anchor = self
                    .app_classes
                    .iter()
                    .find(|(owner, _)| *owner == w.owner)
                    .and_then(|(_, app)| slots[app.index()])
                    .map(|slot| door_window::taskbar::physical(slot, scale));
                (w.id, anchor)
            })
            .collect();
        for (id, anchor) in anchors {
            self.engine.set_taskbar_anchor(id, anchor);
        }
    }
    /// Apply only the newest interactive-resize geometry for this frame: one
    /// configure (and one client repaint) per frame instead of per packet.
    pub(super) fn apply_resize(&mut self) {
        let Some((owner, id, mut rect)) = self.resize_pending.take() else {
            return;
        };
        // Win32 size limits (CAP_WIN32), anchoring the edges not dragged.
        let limits = self.win32.limits(id.0);
        if limits != door_window::win32::Limits::default() {
            use door_service_protocol::window::{EDGE_BOTTOM, EDGE_RIGHT};
            let edges = self
                .drag
                .filter(|d| d.id == id)
                .map_or(EDGE_RIGHT | EDGE_BOTTOM, |d| {
                    door_window::win32::protocol_edges(d.edges)
                });
            let (x, y, w, h) = door_window::win32::clamp_resize(
                limits,
                edges,
                (rect.x(), rect.y(), rect.width(), rect.height()),
            );
            if let Ok(r) = g::Rect::new(x, y, w, h) {
                rect = r;
            }
        }
        let previous = self.engine.window_info(id).ok().map(|w| w.epoch);
        if let Ok(epoch) = self.engine.configure(owner, id, rect)
            && Some(epoch) != previous
        {
            self.attachments.retain(|a| a.window != id);
            if let Some(drag) = self.drag.as_mut().filter(|d| d.id == id) {
                drag.epoch = epoch;
            }
        }
    }
    /// Foreground scene identity for presentation diagnostics, captured
    /// before an overlay-only (shell) frame is rendered.
    pub(super) fn capture_overlay_scene(&self) -> door_window::telemetry::Scene {
        let app = self
            .engine
            .focused_window()
            .and_then(|id| self.engine.window_info(id).ok())
            .and_then(|info| {
                self.app_classes
                    .iter()
                    .find(|(owner, _)| *owner == info.owner)
            })
            .map_or(0, |(_, app)| app.command());
        let overview = self.overview.as_ref().map(|v| v.is_switcher());
        self.presentation_log.capture(app, false, overview)
    }
    /// After a transferred overlay-only frame: the windows beneath had no
    /// pending damage, so the foreground scene is complete. A shell surface
    /// that closes through overlay frames (Start's dismissal animation) must
    /// still report `SCENE_PRESENTED ... overlay=0`, exactly like a scene frame.
    pub(super) fn overlay_scene_presented(&mut self, captured: door_window::telemetry::Scene) {
        let overlay = self.shell_overlay_visible();
        if let Some(scene) = self.presentation_log.overlay_presented(captured, overlay) {
            let _ = door_sys::write(
                format!(
                    "DOOR_P12 SCENE_PRESENTED app={} overlay={}\n",
                    scene.app,
                    u8::from(scene.overlay)
                )
                .as_bytes(),
            );
        }
    }
    /// Shell hook for the mouse wheel, called before window routing. Returns
    /// true when the shell consumed it: Start, flyouts, Task View and the
    /// taskbar own the wheel while open or under the pointer, so it never
    /// scrolls a window beneath them. Shell surfaces that scroll implement
    /// their scrolling here (`notches` > 0 = away from the user).
    pub(super) fn shell_wheel(&mut self, notches: i32) -> bool {
        self.start_wheel(notches);
        self.launcher
            || self.shell.menu.is_some()
            || self.shell.locked
            || self.tray.open.is_some()
            || self.overview.is_some()
            || self.overlay_contains()
    }
}

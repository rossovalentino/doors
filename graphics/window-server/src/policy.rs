use door_graphics_core::{Limits, Scale};
mod shortcuts;
pub use shortcuts::{SystemCommand, SystemEvent, SystemKeys};
extern crate alloc;
use alloc::vec::Vec;
use door_graphics_core::{Engine, Error, Owner, WindowId, WindowState};
pub type PendingClose = (Owner, WindowId, u64);
/// Integer density heuristic for outputs without a preference or host
/// hint: QHD and larger outputs need native 2x glyph/icon frames, not a tiny
/// 96-DPI UI; UHD gets 3x.
pub fn output_scale(width: u32) -> Scale {
    Scale::from_factor(if width >= 3840 {
        3
    } else if width >= 2560 {
        2
    } else {
        1
    })
    .unwrap_or_default()
}
/// The automatic ("Recommended") display scale of an output: the host's
/// scale hint when it is offered for this output, else `output_scale`.
pub fn recommended_scale(hint: u32, width: u32, height: u32) -> Scale {
    offered_scale(hint, width, height).unwrap_or_else(|| output_scale(width))
}
/// Effective display scale: an offered `preference` (Settings ScalePercent;
/// 0 = automatic), else the recommended scale.
pub fn resolve_scale(preference: u32, hint: u32, width: u32, height: u32) -> Scale {
    offered_scale(preference, width, height)
        .unwrap_or_else(|| recommended_scale(hint, width, height))
}
fn offered_scale(percent: u32, width: u32, height: u32) -> Option<Scale> {
    Scale::from_percent(percent).filter(|_| crate::display_scale::offered(percent, width, height))
}
/// Physical rows from the top of the 48-DIP bottom taskbar band to the
/// bottom of a `height`-row output (the band's snapped rows plus any
/// sub-logical-pixel remainder below the logical output).
pub fn bar_rows(height: u32, scale: Scale) -> u32 {
    let top = scale.px(scale.logical_len(height) as i32 - 48).max(0) as u32;
    height.saturating_sub(top)
}
/// A trusted overlay may return focus only to the exact still-live origin.
#[derive(Clone, Copy)]
pub struct ReturnFocus {
    id: WindowId,
    owner: Owner,
    workspace: door_graphics_core::WorkspaceId,
    session: (u64, u64, u32),
}
impl ReturnFocus {
    pub fn belongs_to(self, owner: Owner) -> bool {
        self.owner == owner
    }
    pub fn capture(engine: &Engine, session: Option<(u64, u64, u32)>) -> Option<Self> {
        let info = engine.window_info(engine.focused_window()?).ok()?;
        Some(Self {
            id: info.id,
            owner: info.owner,
            workspace: engine.active_workspace(),
            session: session?,
        })
    }
    pub fn resolve(self, engine: &Engine, session: Option<(u64, u64, u32)>) -> Option<WindowId> {
        let info = engine.window_info(self.id).ok()?;
        (session == Some(self.session)
            && engine.active_workspace() == self.workspace
            && info.workspace == self.workspace
            && info.owner == self.owner
            && info.visible
            && matches!(info.state, WindowState::Normal | WindowState::Maximized))
        .then_some(self.id)
    }
}
pub fn appearance_value(operation: u32, value: u32, background: u32) -> Option<u32> {
    use door_service_protocol::window::{SET_APPEARANCE, SET_THEME};
    match operation {
        SET_THEME if value <= 1 && background < door_service_protocol::window::WALLPAPERS => {
            Some((background << 1) | value)
        }
        SET_APPEARANCE if value < door_service_protocol::window::APPEARANCE_VALUES => Some(value),
        _ => None,
    }
}
/// Rows of the retained shell underlay. Open shell surfaces (Start, tray
/// flyouts, menus, the snap bar) can cover any part of the output, so the
/// cache spans the whole output (`taskbar::Layout::cache`); a closed shell
/// needs only the taskbar band.
pub fn overlay_rows(height: u32, launcher: bool) -> u32 {
    if launcher { height } else { height.min(48) }
}
pub fn overlay_rows_scaled(height: u32, launcher: bool, scale: Scale) -> u32 {
    if launcher {
        height
    } else {
        bar_rows(height, scale)
    }
}
pub fn overlay_transfer_rows(height: u32, scale: Scale, open: bool, previously_open: bool) -> u32 {
    overlay_rows_scaled(height, open || previously_open, scale)
}
pub fn edge_snap(
    area: door_graphics_core::Rect,
    scale: Scale,
    x: i32,
    y: i32,
) -> Option<door_graphics_core::Snap> {
    use door_graphics_core::Snap;
    let threshold = scale.len(2) as i32;
    let corner = scale.len(48) as i32;
    if x <= area.x() + threshold {
        return Some(if y < area.y() + corner {
            Snap::TopLeft
        } else if y >= area.bottom() - corner {
            Snap::BottomLeft
        } else {
            Snap::Left
        });
    }
    if x >= area.right() - 1 - threshold {
        return Some(if y < area.y() + corner {
            Snap::TopRight
        } else if y >= area.bottom() - corner {
            Snap::BottomRight
        } else {
            Snap::Right
        });
    }
    (y <= area.y() + threshold).then_some(Snap::Maximize)
}
/// What a desktop icon release did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    /// Second release of a double click: open the item.
    Open,
    /// First click of a possible pair.
    First,
    /// The press was cancelled before this release (or never seen).
    NoPress,
    /// The pointer left the press (or first click) position.
    Moved,
    OtherIcon,
    /// Ticks since the first click (over the 500 ms double-click time).
    TooSlow(u64),
}
impl Release {
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::First => "first",
            Self::NoPress => "no-press",
            Self::Moved => "moved",
            Self::OtherIcon => "other-icon",
            Self::TooSlow(_) => "too-slow",
        }
    }
}
/// Single selection and a real double-click (500 ms, four-pixel tolerance).
/// Input timestamps are monotonic 10 ms ticks. Cancelled drags/epoch resets
/// cannot combine with an unrelated later press to open an application.
#[derive(Default)]
pub struct DesktopSelection {
    selected: Option<usize>,
    pressed: Option<(usize, i32, i32)>,
    clicked: Option<(usize, u64, i32, i32)>,
}
impl DesktopSelection {
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn cancel_gesture(&mut self) {
        self.pressed = None;
        self.clicked = None;
    }
    pub fn select(&mut self, index: usize) {
        self.cancel_gesture();
        self.selected = (index < door_service_protocol::application::DESKTOP_APPS).then_some(index);
    }
    pub fn press(&mut self, index: usize, x: i32, y: i32) {
        if self.selected != Some(index) {
            self.clicked = None;
        }
        self.selected = (index < door_service_protocol::application::DESKTOP_APPS).then_some(index);
        self.pressed = self.selected.map(|i| (i, x, y));
    }
    pub fn motion(&mut self, x: i32, y: i32) {
        if self
            .pressed
            .is_some_and(|(_, px, py)| !near((px, py), (x, y)))
        {
            self.cancel_gesture();
        }
    }
    pub fn release(&mut self, index: usize, now: u64, x: i32, y: i32) -> bool {
        self.release_outcome(index, now, x, y) == Release::Open
    }
    /// The release and why it did or did not complete a double click.
    pub fn release_outcome(&mut self, index: usize, now: u64, x: i32, y: i32) -> Release {
        let Some((i, px, py)) = self.pressed.take() else {
            self.clicked = None;
            return Release::NoPress;
        };
        if i != index || !near((px, py), (x, y)) {
            self.clicked = None;
            return Release::Moved;
        }
        let outcome = match self.clicked.take() {
            None => Release::First,
            Some((i, ..)) if i != index => Release::OtherIcon,
            Some((_, _, px, py)) if !near((px, py), (x, y)) => Release::Moved,
            Some((_, time, ..)) if now < time || now - time > 50 => {
                Release::TooSlow(now.saturating_sub(time))
            }
            Some(_) => return Release::Open,
        };
        self.clicked = Some((index, now, x, y));
        outcome
    }
    /// A press or first click is in progress (diagnostics).
    pub fn active(&self) -> bool {
        self.pressed.is_some() || self.clicked.is_some()
    }
}
fn near(a: (i32, i32), b: (i32, i32)) -> bool {
    a.0.abs_diff(b.0) <= 4 && a.1.abs_diff(b.1) <= 4
}
/// Last explicit user focus decision wins over slow application startup.
#[derive(Default)]
pub struct Foreground {
    managed: bool,
    desired: Option<u8>,
    pending_until: Option<u64>,
}
impl Foreground {
    pub fn request(&mut self, app: u8, now: u64) {
        self.manual(Some(app));
        self.pending_until = Some(now.saturating_add(2000));
    }
    pub fn manual(&mut self, app: Option<u8>) {
        self.managed = true;
        self.desired = app;
        self.pending_until = None;
    }
    pub fn allows_new(&self, app: u8) -> bool {
        !self.managed || self.desired == Some(app)
    }
    pub fn pending(&mut self, now: u64) -> Option<u8> {
        if self.pending_until.is_some_and(|deadline| now < deadline) {
            self.desired
        } else {
            self.pending_until = None;
            None
        }
    }
    pub fn complete(&mut self) {
        self.pending_until = None;
    }
}
/// Shutdown and document prompts (Windows 11). A CANCEL_CLOSE during a
/// shutdown means the app is asking the user about unsaved work: the
/// shutdown waits for it, the app is neither closed again nor terminated,
/// and its own close after Save or Don't save continues the shutdown by
/// itself. Cancel in that prompt (CLOSE_DECLINED) cancels the shutdown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShutdownGate {
    waiting: alloc::vec::Vec<Owner>,
}
/// What a CANCEL_CLOSE does to the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Deferred {
    /// A shutdown is in progress: it now waits on this app's prompt.
    Waiting,
    /// No shutdown: only that window's close was cancelled.
    WindowOnly,
}
impl ShutdownGate {
    pub fn close_deferred(&mut self, shutting_down: bool, owner: Owner) -> Deferred {
        if !shutting_down {
            return Deferred::WindowOnly;
        }
        if !self.waiting.contains(&owner) {
            self.waiting.push(owner);
        }
        Deferred::Waiting
    }
    /// CLOSE_DECLINED: true when it cancels a shutdown (the gate clears).
    pub fn declined(&mut self, shutting_down: bool, owner: Owner) -> bool {
        let cancels = shutting_down && self.waiting.contains(&owner);
        if cancels {
            self.waiting.clear();
        }
        cancels
    }
    /// The shutdown is asking `owner`'s user: never close it again or
    /// terminate it, and do not log its own close as a new request.
    pub fn waiting_on(&self, owner: Owner) -> bool {
        self.waiting.contains(&owner)
    }
    pub fn is_waiting(&self) -> bool {
        !self.waiting.is_empty()
    }
    /// Drop apps that exited (their prompt was answered Save/Don't save).
    pub fn retain_live(&mut self, live: impl Fn(Owner) -> bool) {
        self.waiting.retain(|o| live(*o));
    }
    pub fn clear(&mut self) {
        self.waiting.clear();
    }
}
#[derive(Default)]
pub struct RetryGate(u64);
impl RetryGate {
    pub fn claim(&mut self, now: u64) -> bool {
        if now < self.0 {
            return false;
        }
        self.0 = now.saturating_add(10);
        true
    }
}
/// Scene revocation is immediate; trusted process termination is retried until
/// acknowledged. Incarnation identity also blocks a quarantined reconnection.
#[derive(Default)]
pub struct Terminations(Vec<(Owner, u64)>);
impl Terminations {
    pub fn contains(&self, owner: Owner) -> bool {
        self.0.iter().any(|v| v.0 == owner)
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn push(&mut self, owner: Owner, now: u64) {
        if !self.contains(owner) {
            self.0.push((owner, now));
        }
    }
    pub fn due(&self, now: u64) -> Option<Owner> {
        self.0.iter().find(|v| now >= v.1).map(|v| v.0)
    }
    pub fn complete(&mut self, owner: Owner, now: u64, accepted: bool) {
        if accepted {
            self.0.retain(|v| v.0 != owner);
        } else if let Some(v) = self.0.iter_mut().find(|v| v.0 == owner) {
            v.1 = now.saturating_add(10);
        }
    }
}
/// A saturated client still gets a fixed containment deadline. Its queued input
/// remains intact, and notification is retried when the consumer makes room.
pub fn request_close(
    engine: &mut Engine,
    pending: &mut Vec<PendingClose>,
    owner: Owner,
    id: WindowId,
    now: u64,
) -> door_graphics_core::Result<()> {
    request_close_with_grace(engine, pending, owner, id, now, 300)
}
/// The 4K CPU profile can be inside a large surface configure when Close arrives.
/// A fixed longer grace contains hung clients without revoking healthy resizing apps.
pub fn request_close_with_grace(
    engine: &mut Engine,
    pending: &mut Vec<PendingClose>,
    owner: Owner,
    id: WindowId,
    now: u64,
    grace: u64,
) -> door_graphics_core::Result<()> {
    if pending
        .iter()
        .any(|entry| entry.0 == owner && entry.1 == id)
    {
        return Ok(());
    }
    match engine.close(owner, id) {
        Ok(()) | Err(Error::QueueFull) => (),
        Err(error) => return Err(error),
    }
    pending.push((owner, id, now.saturating_add(grace.clamp(300, 1000))));
    Ok(())
}
pub fn retry_closes(engine: &mut Engine, pending: &[PendingClose]) {
    for &(owner, id, _) in pending {
        if engine
            .window_info(id)
            .is_ok_and(|w| w.state != WindowState::Closing)
        {
            let _ = engine.close(owner, id);
        }
    }
}
pub const HEAP_BYTES: usize = 256 * 1024 * 1024;
pub const fn limits() -> Limits {
    Limits {
        windows_per_owner: 4,
        buffers_per_owner: 8,
        bytes_per_owner: 128 * 1024 * 1024,
        total_bytes: 192 * 1024 * 1024,
        total_windows: 24,
        total_buffers: 32,
        total_owners: 8,
        // A bounded normal typing burst must survive a costly native 4K frame.
        // Non-polling owners still exhaust this quota and are revoked.
        events_per_owner: 256,
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn transfer_band_keeps_dismissal_but_avoids_closed_flyout_rows() {
        for factor in [1, 2] {
            let scale = door_graphics_core::Scale::from_factor(factor).unwrap();
            assert_eq!(
                super::overlay_transfer_rows(2160, scale, false, false),
                48 * factor
            );
            for (open, old) in [(true, false), (true, true), (false, true)] {
                assert_eq!(super::overlay_transfer_rows(2160, scale, open, old), 2160);
            }
            // The allocated underlay always covers the shell cache.
            for (w, h) in [(1280, 720), (1920, 1080), (800, 600)] {
                let layout = crate::taskbar::Layout::new(w, h);
                assert_eq!(layout.cache().height(), super::overlay_rows(h, true));
            }
        }
        let two = door_graphics_core::Scale::from_factor(2).unwrap();
        assert_eq!(super::overlay_transfer_rows(720, two, false, true), 720);
    }
    extern crate alloc;
    use super::*;
    use alloc::vec;
    use door_graphics_core::*;
    #[test]
    fn native_typing_burst_is_ordered_bounded_and_isolated_between_owners() {
        let mut e = Engine::new(640, 480, Theme::Light, limits()).unwrap();
        let owner = Owner {
            pid: 1,
            session: 1,
            epoch: 1,
        };
        let other = Owner { pid: 2, ..owner };
        let id = e
            .create_window(owner, Role::TopLevel, Rect::new(0, 0, 300, 200).unwrap())
            .unwrap();
        let sibling = e
            .create_window(other, Role::TopLevel, Rect::new(320, 0, 300, 200).unwrap())
            .unwrap();
        e.activate(id).unwrap();
        while e.poll_event(owner).is_some() {}
        let mut expected = Vec::new();
        for i in 0..256 {
            let input = if i % 2 == 0 {
                Input::Key {
                    code: 30 + i % 20,
                    down: i % 4 == 0,
                }
            } else {
                Input::PointerButton {
                    button: 1,
                    down: false,
                    x: 100,
                    y: 100,
                }
            };
            assert_eq!(e.route_input(input, i as u64), Ok(Some(id)));
            expected.push((input, i as u64));
        }
        assert_eq!(
            e.route_input(
                Input::Key {
                    code: 31,
                    down: true
                },
                256
            ),
            Err(Error::QueueFull)
        );
        assert_eq!(e.focused_window(), Some(id));
        assert!(
            matches!(e.poll_event(other), Some(Event::Configure { window, .. }) if window == sibling)
        );
        let content = e.window_info(id).unwrap().content;
        for (input, timestamp) in expected {
            let input = match input {
                Input::PointerButton { button, down, x, y } => Input::PointerButton {
                    button,
                    down,
                    x: x - content.x(),
                    y: y - content.y(),
                },
                key => key,
            };
            assert_eq!(
                e.poll_event(owner),
                Some(Event::Input {
                    window: id,
                    input,
                    timestamp
                })
            );
        }
        assert_eq!(e.poll_event(owner), None);
        e.activate(sibling).unwrap();
        assert_eq!(
            e.route_input(
                Input::Key {
                    code: 32,
                    down: true
                },
                257
            ),
            Ok(Some(sibling))
        );
    }
    #[test]
    fn output_density_uses_native_two_x_before_controls_become_tiny() {
        for (width, scale) in [(1024, 1), (1280, 1), (1920, 1), (2560, 2), (3840, 3)] {
            assert_eq!(output_scale(width), Scale::from_factor(scale).unwrap());
        }
    }
    #[test]
    fn display_scale_resolves_preference_then_host_hint_then_heuristic() {
        let p = |percent| Scale::from_percent(percent).unwrap();
        // Automatic: the host hint when offered, else the integer heuristic.
        assert_eq!(resolve_scale(0, 150, 2560, 1600), p(150));
        assert_eq!(resolve_scale(0, 0, 2560, 1600), p(200));
        assert_eq!(resolve_scale(0, 300, 1920, 1080), p(100));
        assert_eq!(recommended_scale(175, 1920, 1080), p(175));
        // An offered preference wins; an invalid or unoffered one is automatic.
        assert_eq!(resolve_scale(125, 150, 2560, 1600), p(125));
        assert_eq!(resolve_scale(300, 150, 2560, 1600), p(150));
        assert_eq!(resolve_scale(130, 0, 1920, 1080), p(100));
        // The bottom band is the 48-DIP bar's snapped rows at every scale.
        for percent in Scale::PERCENTS {
            let s = p(percent);
            for height in [600u32, 720, 768, 1080, 1200, 1440, 1600, 2160] {
                let rows = bar_rows(height, s);
                let logical = s.logical_len(height) as i32;
                assert_eq!(
                    rows,
                    height - s.px(logical - 48) as u32,
                    "{percent}% {height}"
                );
                assert!(
                    rows >= s.len(48) && rows < s.len(48) + s.len(1) + 1,
                    "{percent}% {height}: {rows}"
                );
            }
        }
        assert_eq!(bar_rows(1600, p(150)), 72);
    }
    #[test]
    fn overlay_focus_never_restores_a_stale_hidden_foreign_or_other_session_origin() {
        let mut e = Engine::new(640, 480, Theme::Light, limits()).unwrap();
        let owner = Owner {
            pid: 10,
            session: 1,
            epoch: 3,
        };
        let id = e
            .create_window(owner, Role::TopLevel, Rect::new(0, 0, 300, 200).unwrap())
            .unwrap();
        e.activate(id).unwrap();
        let session = Some((1, 3, 1001));
        let saved = ReturnFocus::capture(&e, session).unwrap();
        e.clear_focus();
        assert_eq!(saved.resolve(&e, session), Some(id));
        assert_eq!(saved.resolve(&e, None), None);
        assert_eq!(saved.resolve(&e, Some((1, 4, 1001))), None);
        assert_eq!(
            ReturnFocus {
                owner: Owner { epoch: 4, ..owner },
                ..saved
            }
            .resolve(&e, session),
            None
        );
        let workspace = e.create_workspace().unwrap();
        e.switch_workspace(workspace).unwrap();
        assert_eq!(saved.resolve(&e, session), None);
        e.switch_workspace(saved.workspace).unwrap();
        e.request_state(owner, id, WindowState::Minimized).unwrap();
        assert_eq!(saved.resolve(&e, session), None);
        e.restore_window(owner, id).unwrap();
        e.close(owner, id).unwrap();
        assert_eq!(saved.resolve(&e, session), None);
        e.disconnect(owner);
        let replacement = e
            .create_window(owner, Role::TopLevel, Rect::new(0, 0, 300, 200).unwrap())
            .unwrap();
        assert_ne!(replacement, id);
        assert_eq!(saved.resolve(&e, session), None);
    }
    #[test]
    fn late_app_cannot_override_newer_manual_focus_or_raise_above_it() {
        let mut intent = Foreground::default();
        assert!(intent.allows_new(1) && intent.allows_new(3));
        intent.request(3, 100);
        assert_eq!(intent.pending(101), Some(3));
        intent.manual(Some(1));
        assert_eq!(intent.pending(102), None);
        assert!(intent.allows_new(1) && !intent.allows_new(3));
        intent.manual(None);
        assert!(!intent.allows_new(1) && !intent.allows_new(3));
        intent.request(2, 200);
        assert_eq!(intent.pending(2199), Some(2));
        assert_eq!(intent.pending(2200), None);
        let mut engine = Engine::new(320, 240, Theme::Light, limits()).unwrap();
        let a = Owner {
            pid: 1,
            session: 1,
            epoch: 1,
        };
        let b = Owner { pid: 2, ..a };
        let rect = Rect::new(0, 0, 300, 200).unwrap();
        let first = engine.create_window(a, Role::TopLevel, rect).unwrap();
        engine.activate(first).unwrap();
        let late = engine.create_window(b, Role::TopLevel, rect).unwrap();
        engine.lower_top_level(b, late).unwrap();
        assert_eq!(engine.focused_window(), Some(first));
        assert_eq!(engine.hit_test(100, 100).map(|h| h.0), Some(first));
        engine.activate(late).unwrap();
        assert_eq!(engine.hit_test(100, 100).map(|h| h.0), Some(late));
    }
    #[test]
    fn input_damage_during_transfer_preserves_current_pixels_and_next_scene() {
        let mut engine = Engine::new(320, 240, Theme::Light, limits()).unwrap();
        let serial = engine.render().unwrap().serial;
        assert!(matches!(engine.output_view(), Err(Error::Busy)));
        let before = engine.transfer_view(serial).unwrap().pixels.to_vec();
        engine.set_theme(Theme::Dark);
        assert_eq!(engine.transfer_view(serial).unwrap().pixels, before);
        assert!(matches!(engine.render(), Err(Error::Busy)));
        assert!(matches!(
            engine.transfer_view(serial + 1),
            Err(Error::UnknownFrame)
        ));
        engine.complete_transfer(serial, true, 1).unwrap();
        assert!(engine.has_damage());
        assert!(matches!(
            engine.transfer_view(serial),
            Err(Error::UnknownFrame)
        ));
        let next = engine.render().unwrap().serial;
        assert_ne!(engine.transfer_view(next).unwrap().pixels, before);
    }
    /// Windows 11 shutdown with a document prompt: Save/Don't save lets the
    /// shutdown finish by itself; Cancel cancels it; a prompting app is never
    /// closed again or terminated while it asks.
    #[test]
    fn shutdown_waits_on_document_prompts_and_cancel_keeps_the_session() {
        let app = |pid| Owner {
            pid,
            session: 1,
            epoch: 1,
        };
        let (files, notes) = (app(16), app(18));
        // Save path: Notes defers its close to a prompt; Files closes.
        let mut gate = ShutdownGate::default();
        assert_eq!(gate.close_deferred(true, notes), Deferred::Waiting);
        assert!(gate.waiting_on(notes) && !gate.waiting_on(files) && gate.is_waiting());
        assert_eq!(
            gate.close_deferred(true, notes),
            Deferred::Waiting,
            "repeat"
        );
        // Save: Notes closes its window and exits; nobody is asked any more,
        // so the shutdown (still in progress) completes without a new request.
        let alive = [files];
        gate.retain_live(|o| alive.contains(&o));
        assert!(!gate.is_waiting());
        // Cancel path: the shutdown is cancelled, the gate is clear and Notes
        // keeps its window.
        let mut gate = ShutdownGate::default();
        gate.close_deferred(true, notes);
        assert!(!gate.declined(true, files), "only the asking app cancels");
        assert!(gate.declined(true, notes));
        assert!(!gate.is_waiting());
        // Outside a shutdown a cancelled close or prompt touches nothing else.
        assert_eq!(gate.close_deferred(false, notes), Deferred::WindowOnly);
        assert!(!gate.is_waiting());
        assert!(!gate.declined(false, notes));
    }
    #[test]
    fn desktop_selection_requires_a_bounded_pair_and_cancels_drag_or_reset() {
        let mut s = DesktopSelection::default();
        s.press(0, 40, 32);
        assert_eq!(s.selected(), Some(0));
        assert!(!s.release(0, 100, 40, 32));
        s.press(0, 42, 34);
        assert!(s.release(0, 150, 42, 34));
        // A third click starts a new pair, not another activation.
        s.press(0, 40, 32);
        assert!(!s.release(0, 151, 40, 32));
        s.press(0, 40, 32);
        assert!(!s.release(0, 202, 40, 32));
        s.press(0, 40, 32);
        s.motion(50, 32);
        s.motion(40, 32);
        assert!(!s.release(0, 210, 40, 32));
        s.press(1, 40, 120);
        assert!(!s.release(1, 220, 40, 120));
        s.cancel_gesture();
        s.press(1, 40, 120);
        assert!(!s.release(1, 225, 40, 120));
        s.clear();
        assert_eq!(s.selected(), None);
        s.press(3, 40, 120);
        assert!(!s.release(3, 226, 40, 120));
        s.press(0, i32::MIN, 32);
        assert!(!s.release(0, 230, i32::MAX, 32));
    }
    /// The window server's desktop double-click path from device packets:
    /// input batching, button transitions from the flags, the desktop grid
    /// hit band and `DesktopSelection` on 10 ms tick timestamps (as in
    /// `input.rs` / `desktop_button`). Returns the launched icons.
    fn replay(packets: &[(u64, i32, i32, u32)], start: (i32, i32)) -> alloc::vec::Vec<usize> {
        use door_abi::graphics as d;
        let mut batch = crate::input_batch::Batch::with_buttons(0);
        for &(timestamp, x, y, flags) in packets {
            assert!(batch.push(d::Input {
                kind: d::MOTION,
                x,
                y,
                flags,
                timestamp,
                ..d::Input::default()
            }));
        }
        dispatch(batch.events(), start)
    }
    /// `input.rs` order per event: motion first, then button transitions.
    fn dispatch(events: &[door_abi::graphics::Input], start: (i32, i32)) -> alloc::vec::Vec<usize> {
        use crate::apps::desktop_grid;
        let (mut px, mut py, mut buttons) = (start.0, start.1, 0u32);
        let mut selection = DesktopSelection::default();
        let mut pressed = None;
        let mut launched = alloc::vec::Vec::new();
        let icon = |x: i32, y: i32| {
            desktop_grid::HIT_X
                .contains(&x)
                .then(|| (0..6).find(|i| desktop_grid::band(*i).contains(&y)))
                .flatten()
        };
        for e in events {
            px += e.x;
            py += e.y;
            selection.motion(px, py);
            if (buttons ^ e.flags) & 1 != 0 {
                let down = e.flags & 1 != 0;
                let target = icon(px, py);
                if down {
                    match target {
                        Some(i) => selection.press(i, px, py),
                        None => selection.cancel_gesture(),
                    }
                    pressed = target;
                } else if let Some(i) = pressed.take().filter(|i| target == Some(*i))
                    && selection.release(i, e.timestamp, px, py)
                {
                    launched.push(i);
                }
            }
            buttons = e.flags;
        }
        launched
    }
    #[test]
    fn desktop_double_click_from_device_packets_launches_within_the_double_click_time() {
        use crate::apps::desktop_grid;
        for index in 0..3 {
            let (x, y, w, h) = desktop_grid::plate(index);
            let (cx, cy) = (x + w as i32 / 2, y + h as i32 / 2);
            assert!(desktop_grid::HIT_X.contains(&cx) && desktop_grid::band(index).contains(&cy));
            // Move onto the icon, then down/up/down/up 50 ms apart (the
            // scenario helper) with up to 200 ms of extra QMP delay per command.
            for overhead in [0u64, 10, 20] {
                let t = |n: u64| 100 + n * (5 + overhead);
                let packets = [
                    (90, cx - 200, cy - 300, 0),
                    (t(0), 0, 0, 1),
                    (t(1), 0, 0, 0),
                    (t(2), 0, 0, 1),
                    (t(3), 0, 0, 0),
                ];
                assert_eq!(
                    replay(&packets, (200, 300)),
                    [index],
                    "icon {index} overhead {overhead}"
                );
            }
        }
        // The old scenario timing (200 ms after each command, 100 ms before
        // the second release) fails once QMP adds over 100 ms per command.
        let old = |overhead: u64| {
            let r1 = 100 + 20 + overhead;
            [
                (100, 0, 0, 1),
                (r1, 0, 0, 0),
                (r1 + 20 + overhead, 0, 0, 1),
                (r1 + 30 + 2 * overhead, 0, 0, 0),
            ]
        };
        assert_eq!(replay(&old(0), (42, 46)), [0]);
        assert_eq!(replay(&old(11), (42, 46)), alloc::vec::Vec::<usize>::new());
        // Zero-delta packets between the clicks and a merged release burst
        // keep the pair; a 5 px slip between presses does not.
        let jitter = [
            (100, 0, 0, 1),
            (105, 0, 0, 0),
            (106, 0, 0, 0),
            (110, 0, 0, 1),
            (115, 0, 0, 0),
            (116, 3, 0, 0),
        ];
        assert_eq!(replay(&jitter, (42, 46)), [0]);
        let slip = [
            (100, 0, 0, 1),
            (105, 0, 0, 0),
            (107, 5, 0, 0),
            (110, 0, 0, 1),
            (115, 0, 0, 0),
        ];
        assert_eq!(replay(&slip, (42, 46)), alloc::vec::Vec::<usize>::new());
        // The guest failure: the scenario moves the pointer 12 px right after
        // the second release, and both packets arrive in one input batch.
        // The release must be handled where it happened, not after the move
        // (which cancels the pressed gesture first).
        let away = [
            (100, 0, 0, 1),
            (105, 0, 0, 0),
            (110, 0, 0, 1),
            (115, 0, 0, 0),
            (116, -12, 0, 0),
        ];
        assert_eq!(replay(&away, (42, 46)), [0]);
        // What the previous batching produced: the move merged into the
        // release event, so the release was seen after the pointer left.
        let merged: alloc::vec::Vec<_> = [(100, 0, 1), (105, 0, 0), (110, 0, 1), (116, -12, 0)]
            .iter()
            .map(|&(timestamp, x, flags)| door_abi::graphics::Input {
                kind: door_abi::graphics::MOTION,
                x,
                flags,
                timestamp,
                ..door_abi::graphics::Input::default()
            })
            .collect();
        assert_eq!(dispatch(&merged, (42, 46)), alloc::vec::Vec::<usize>::new());
        // Re-applying identical desktop controls between the clicks leaves
        // input state alone (SET_DESKTOP_CONTROLS returns before touching it).
        let mut s = DesktopSelection::default();
        s.press(0, 42, 46);
        assert_eq!(s.release_outcome(0, 100, 42, 46), Release::First);
        s.press(0, 42, 46);
        assert_eq!(s.release_outcome(0, 105, 42, 46), Release::Open);
        // Outcomes explain a missed pair.
        s.press(0, 42, 46);
        assert_eq!(s.release_outcome(0, 200, 42, 46), Release::First);
        s.press(0, 42, 46);
        assert_eq!(s.release_outcome(0, 251, 42, 46), Release::TooSlow(51));
        s.press(0, 42, 46);
        s.motion(54, 46);
        assert_eq!(s.release_outcome(0, 252, 54, 46), Release::NoPress);
        assert!(!s.active());
    }
    #[test]
    fn legacy_theme_changes_preserve_selected_wallpaper_and_reject_new_values() {
        use door_service_protocol::window::{SET_APPEARANCE, SET_THEME};
        let appearance = appearance_value(SET_APPEARANCE, 7, 0).unwrap();
        assert_eq!(appearance_value(SET_THEME, 0, appearance >> 1), Some(6));
        assert_eq!(appearance_value(SET_THEME, 1, appearance >> 1), Some(7));
        assert_eq!(appearance_value(SET_THEME, 2, 3), None);
        // Dark Flow 4 (picture 17) is the last valid word.
        assert_eq!(appearance_value(SET_APPEARANCE, 35, 0), Some(35));
        assert_eq!(appearance_value(SET_THEME, 1, 17), Some(35));
        assert_eq!(appearance_value(SET_THEME, 1, 18), None);
        assert_eq!(appearance_value(SET_APPEARANCE, 36, 0), None);
        assert_eq!(appearance_value(0, 0, 0), None);
    }
    #[test]
    fn overlay_cache_and_repaint_fit_every_accepted_small_output() {
        for height in [96, 120, 240, 279, 280, 720, 2160] {
            let mut e = Engine::new(320, height, Theme::Light, limits()).unwrap();
            let frame = e.render().unwrap();
            let serial = frame.serial;
            e.complete_transfer(serial, true, 1).unwrap();
            let cache = vec![0u8; 320 * overlay_rows(height, true) as usize * 4];
            for launcher in [false, true] {
                let rows = overlay_rows(height, launcher);
                let bytes = 320 * rows as usize * 4;
                let region = Rect::new(0, (height - rows) as i32, 320, rows).unwrap();
                let frame = e
                    .repaint_overlay(region, |canvas| {
                        let len = canvas.pixels_mut().len();
                        canvas.pixels_mut()[len - bytes..]
                            .copy_from_slice(&cache[cache.len() - bytes..]);
                    })
                    .unwrap();
                let serial = frame.serial;
                e.complete_transfer(serial, true, 2).unwrap();
            }
        }
    }
    #[test]
    fn failed_control_delivery_retries_on_time_despite_every_tick_polling() {
        let mut retry = RetryGate::default();
        let mut attempts = Vec::new();
        for now in 0..31 {
            if retry.claim(now) {
                attempts.push(now);
            }
        }
        assert_eq!(attempts, [0, 10, 20, 30]);
    }
    #[test]
    fn quarantine_survives_failed_delivery_and_tracks_incarnations() {
        let owner = Owner {
            pid: 7,
            session: 1,
            epoch: 7,
        };
        let mut pending = Terminations::default();
        pending.push(owner, 100);
        for now in [100, 110] {
            assert_eq!(pending.due(now), Some(owner));
            pending.complete(owner, now, false);
            assert!(pending.contains(owner));
            assert_eq!(pending.due(now + 1), None);
        }
        assert!(!pending.contains(Owner { epoch: 8, ..owner }));
        pending.complete(owner, 120, true);
        assert!(pending.is_empty());
    }
    #[test]
    fn saturated_close_retries_without_dropping_input_or_extending_deadline() {
        let mut engine = Engine::new(320, 240, Theme::Light, limits()).unwrap();
        let owner = Owner {
            pid: 1,
            session: 1,
            epoch: 1,
        };
        let window = engine
            .create_window(owner, Role::TopLevel, Rect::new(0, 0, 300, 200).unwrap())
            .unwrap();
        engine.activate(window).unwrap();
        while engine
            .route_input(
                Input::Key {
                    code: 30,
                    down: true,
                },
                0,
            )
            .is_ok()
        {}
        assert_eq!(engine.close(owner, window), Err(Error::QueueFull));
        let mut pending = Vec::new();
        request_close(&mut engine, &mut pending, owner, window, 100).unwrap();
        request_close(&mut engine, &mut pending, owner, window, 200).unwrap();
        assert_eq!(pending, [(owner, window, 400)]);
        let mut keys = 0;
        while let Some(event) = engine.poll_event(owner) {
            if matches!(
                event,
                Event::Input {
                    input: Input::Key { .. },
                    ..
                }
            ) {
                keys += 1;
            }
        }
        assert!(keys > 0);
        retry_closes(&mut engine, &pending);
        assert_eq!(
            engine.window_info(window).unwrap().state,
            WindowState::Closing
        );
        assert!(
            matches!(engine.poll_event(owner),Some(Event::CloseRequested{window:id}) if id==window)
        );
    }
    #[test]
    fn three_wide_clients_can_commit_and_replace_maximized_surfaces() {
        for (width, height) in [(1280, 720), (3840, 2160)] {
            let mut engine = Engine::new(width, height, Theme::Light, limits()).unwrap();
            let descriptor = SurfaceDescriptor::new(
                width,
                height - 80,
                width as usize * 4,
                width as usize * (height as usize - 80) * 4,
            )
            .unwrap();
            for pid in 1..=3 {
                let owner = Owner {
                    pid,
                    session: 1,
                    epoch: pid,
                };
                let window = engine
                    .create_window(
                        owner,
                        Role::TopLevel,
                        Rect::new(0, 0, width, height - 48).unwrap(),
                    )
                    .unwrap();
                let epoch = engine.window_info(window).unwrap().epoch;
                engine.acknowledge_configure(owner, window, epoch).unwrap();
                let buffer = engine.register_buffer(owner, descriptor).unwrap();
                for _ in 0..2 {
                    engine.attach(owner, window, buffer, epoch).unwrap();
                    engine
                        .commit(owner, window, vec![255; descriptor.backing_bytes])
                        .unwrap();
                }
            }
            assert_eq!(
                door_abi::platform::manifest_policy(door_abi::platform::manifest::WINDOW)
                    .unwrap()
                    .max_heap_bytes,
                HEAP_BYTES as u64
            );
            // Account for all snapshots, one replacement capture, the output and
            // generous text/metadata headroom; registered sections live elsewhere.
            assert!(
                4 * descriptor.backing_bytes
                    + width as usize * height as usize * 8
                    + 2 * 1024 * 1024
                    < HEAP_BYTES
            );
        }
    }
}

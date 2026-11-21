//! Time-based window transitions composed only from retained pixels.
//!
//! Open/restore animate the live window (it keeps its final geometry for
//! input; only its pixels are drawn through a scaled, faded layer). Close and
//! minimize keep a "ghost": a private image of the window built when it
//! leaves the scene, drawn at its stacking position until the transition ends
//! and then freed. Ghosts take no input, hold no client resources or quota,
//! and are bounded in count and bytes. Nothing here asks a client to repaint.
//! Time is supplied by the trusted caller in milliseconds (`Engine::advance`);
//! the engine never reads a clock, so composition stays deterministic.
use super::*;

// Durations and curves follow the Fluent motion tokens (learn.microsoft.com
// /windows/apps/design/motion/timing-and-easing): entrances decelerate with
// (0, 0, 0, 1), exits accelerate with (1, 0, 1, 1) and are shorter, and a
// surface moving between two resting places uses the point-to-point curve
// (0.55, 0.55, 0, 1). The values are the `door_design_tokens` motion tracks
// (window open, close, minimize, restore, maximize/snap).
use door_design_tokens::{Easing as Curve, MotionToken, motion};
const fn track(token: MotionToken) -> u64 {
    motion(token).duration_ms as u64
}
const fn spline(token: MotionToken) -> [i64; 4] {
    match motion(token).easing {
        Curve::CubicBezier { x1, y1, x2, y2 } => [x1 as i64, y1 as i64, x2 as i64, y2 as i64],
        Curve::Linear => [0, 0, 1000, 1000],
    }
}
pub const OPEN_MS: u64 = track(MotionToken::WindowOpen);
pub const CLOSE_MS: u64 = track(MotionToken::WindowClose);
pub const MINIMIZE_MS: u64 = track(MotionToken::WindowMinimize);
pub const RESTORE_MS: u64 = track(MotionToken::WindowRestore);
pub const SNAP_MS: u64 = track(MotionToken::WindowMaximize);
/// Animation speed (Doors extra): durations in percent of the Fluent
/// tracks (60 fast, 100 normal, 160 slow).
static DURATION_PERCENT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(100);
fn scaled(ms: u64) -> u64 {
    (ms * u64::from(DURATION_PERCENT.load(core::sync::atomic::Ordering::Relaxed)) / 100).max(1)
}
const DECELERATE: [i64; 4] = spline(MotionToken::WindowOpen);
const ACCELERATE: [i64; 4] = spline(MotionToken::WindowClose);
const POINT_TO_POINT: [i64; 4] = spline(MotionToken::WindowMaximize);
/// After a transition, the stretched last frame stays up at most this long
/// while the client repaints at its new size.
pub(crate) const STRETCH_HOLD_MS: u64 = 2_000;
/// A new window that has not committed a frame by then appears unanimated.
pub(crate) const FIRST_FRAME_MS: u64 = 400;
/// Animated frames are paced to at most one per this interval.
pub(crate) const FRAME_MS: u64 = 8;
const MAX_GHOSTS: usize = 6;
const MAX_LAYER_BYTES: usize = 64 * 1024 * 1024;
/// `Engine::transition_activity` kinds: a window opening (or reopening
/// after a cancelled close), closing, minimizing to or restoring from its
/// taskbar button, and gliding to new bounds (snap and Win+Up maximize).
/// The low three bits are left to the shell.
pub const TRANSITION_OPEN: u8 = 1 << 3;
pub const TRANSITION_CLOSE: u8 = 1 << 4;
pub const TRANSITION_MINIMIZE: u8 = 1 << 5;
pub const TRANSITION_RESTORE: u8 = 1 << 6;
pub const TRANSITION_RELOCATE: u8 = 1 << 7;
/// Transitions in progress at the engine's last `advance`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransitionActivity {
    /// `TRANSITION_*` bits of transitions not yet at their final pose.
    pub kinds: u8,
    /// Earliest start among them (ms, the trusted transition clock).
    pub start: u64,
    /// The clock value of the last `advance` (the time poses are drawn for).
    pub now: u64,
}
/// Fixed-point unit of scale and easing progress.
pub(crate) const ONE: i64 = 1024;

/// Placement of a layer relative to its resting bounds: uniform scale about
/// the center, a center offset, and opacity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Pose {
    pub scale: i64,
    pub dx: i64,
    pub dy: i64,
    pub alpha: i64,
}
impl Pose {
    pub(crate) const REST: Self = Self {
        scale: ONE,
        dx: 0,
        dy: 0,
        alpha: 255,
    };
    /// Windows 11 open/close: 95% scale, transparent.
    pub(crate) const FADED: Self = Self {
        scale: ONE * 95 / 100,
        dx: 0,
        dy: 0,
        alpha: 0,
    };
    pub(crate) fn apply(self, bounds: Rect) -> Option<Rect> {
        let width =
            (i64::from(bounds.width()) * self.scale / ONE).clamp(1, i64::from(MAX_WIDTH) * 2);
        let height =
            (i64::from(bounds.height()) * self.scale / ONE).clamp(1, i64::from(MAX_HEIGHT) * 2);
        let cx2 = 2 * i64::from(bounds.x()) + i64::from(bounds.width()) + 2 * self.dx;
        let cy2 = 2 * i64::from(bounds.y()) + i64::from(bounds.height()) + 2 * self.dy;
        let x = (cx2 - width).div_euclid(2);
        let y = (cy2 - height).div_euclid(2);
        Rect::new(
            i32::try_from(x).ok()?,
            i32::try_from(y).ok()?,
            width as u32,
            height as u32,
        )
        .ok()
    }
    /// Uniform pose shrinking `bounds` toward `target` (a taskbar button).
    pub(crate) fn toward(bounds: Rect, target: Rect, alpha: i64) -> Self {
        let wanted = i64::from(target.width()) * 3;
        let scale = (wanted * ONE / i64::from(bounds.width()).max(1)).clamp(ONE / 12, ONE * 6 / 10);
        let center = |r: Rect| {
            (
                2 * i64::from(r.x()) + i64::from(r.width()),
                2 * i64::from(r.y()) + i64::from(r.height()),
            )
        };
        let (a, b) = (center(bounds), center(target));
        Self {
            scale,
            dx: (b.0 - a.0) / 2,
            dy: (b.1 - a.1) / 2,
            alpha,
        }
    }
    /// Uniform pose placing a window resting at `rest` over its previous
    /// bounds `from` (snap glide): centers coincide, widths match.
    pub(crate) fn from_rect(rest: Rect, from: Rect) -> Self {
        let scale = (i64::from(from.width()) * ONE / i64::from(rest.width()).max(1))
            .clamp(ONE / 8, ONE * 4);
        let center = |r: Rect| {
            (
                2 * i64::from(r.x()) + i64::from(r.width()),
                2 * i64::from(r.y()) + i64::from(r.height()),
            )
        };
        let (a, b) = (center(rest), center(from));
        Self {
            scale,
            dx: (b.0 - a.0) / 2,
            dy: (b.1 - a.1) / 2,
            alpha: 255,
        }
    }
    fn lerp(a: Self, b: Self, t: i64) -> Self {
        let mix = |a: i64, b: i64| a + (b - a) * t / ONE;
        Self {
            scale: mix(a.scale, b.scale),
            dx: mix(a.dx, b.dx),
            dy: mix(a.dy, b.dy),
            alpha: mix(a.alpha, b.alpha).clamp(0, 255),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Easing {
    /// Fast start, gentle landing (open, restore).
    Decelerate,
    /// Gentle start, fast exit (close, minimize).
    Accelerate,
    /// Window moving between two resting places (snap, maximize).
    PointToPoint,
}
impl Easing {
    /// Cubic-bezier control points (x1, y1, x2, y2) in thousandths.
    pub(crate) const fn spline(self) -> [i64; 4] {
        match self {
            Self::Decelerate => DECELERATE,
            Self::Accelerate => ACCELERATE,
            Self::PointToPoint => POINT_TO_POINT,
        }
    }
}
fn ease(easing: Easing, t: i64) -> i64 {
    bezier(easing.spline(), t)
}
/// CSS-style cubic bezier from (0, 0) to (1, 1) with control points in
/// thousandths, for progress `t` in 0..=ONE (integer only: the guest has no
/// floating point). The curve parameter is found by bisection on x(s).
pub(crate) fn bezier([x1, y1, x2, y2]: [i64; 4], t: i64) -> i64 {
    let t = t.clamp(0, ONE);
    if t == 0 || t == ONE {
        return t;
    }
    const S: i64 = 1 << 16;
    // Coordinate (thousandths x S) of curve parameter s/S for control points p1, p2.
    let at = |s: i64, p1: i64, p2: i64| -> i64 {
        let u = S - s;
        (3 * p1 * s * u / S * u + 3 * p2 * s * s / S * u + 1000 * s * s / S * s) / S
    };
    let target = t * 1000 * S / ONE;
    let (mut lo, mut hi) = (0, S);
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if at(mid, x1, x2) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let s = if target - at(lo, x1, x2) <= at(hi, x1, x2) - target {
        lo
    } else {
        hi
    };
    let scale = 1000 * S;
    ((at(s, y1, y2) * ONE + scale / 2) / scale).clamp(0, ONE)
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Transition {
    pub from: Pose,
    pub to: Pose,
    pub start: u64,
    pub duration: u64,
    pub easing: Easing,
    /// One `TRANSITION_*` bit (diagnostics only; never affects poses).
    pub kind: u8,
}
impl Transition {
    pub(crate) fn pose(&self, now: u64) -> Pose {
        let elapsed = now.saturating_sub(self.start).min(self.duration);
        let t = (elapsed as i64 * ONE) / self.duration.max(1) as i64;
        Pose::lerp(self.from, self.to, ease(self.easing, t))
    }
    pub(crate) fn done(&self, now: u64) -> bool {
        now >= self.start.saturating_add(self.duration)
    }
}
/// Private premultiplied image of one window including its shadow.
pub(crate) struct Layer {
    /// Resting output position of the image (window outer plus shadow).
    pub bounds: Rect,
    /// Image dimensions. They differ from `bounds` when the image shows a
    /// frame drawn for another size (the window's last frame before it
    /// repainted at its new size); drawing then stretches it to `bounds`.
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub key: Option<LayerKey>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LayerKey {
    pub revision: u64,
    pub outer: Rect,
    pub focused: bool,
    pub state: WindowState,
    pub theme: Theme,
    pub reduce_transparency: bool,
    pub caption: [Option<bool>; 3],
}
/// A live window in an open or restore transition. `transition` is None
/// while a new window waits for its first frame (drawn invisible).
pub(crate) struct Live {
    pub id: WindowId,
    pub created: u64,
    pub transition: Option<Transition>,
    pub layer: Option<Layer>,
    pub drawn: Option<Rect>,
}
pub(crate) struct Ghost {
    pub source: WindowId,
    /// Drawn directly above this window (None: below every window).
    pub below: Option<WindowId>,
    pub layer: Layer,
    pub transition: Transition,
    pub drawn: Option<Rect>,
}
#[derive(Default)]
pub(crate) struct Motion {
    pub enabled: bool,
    pub now: u64,
    pub live: Vec<Live>,
    pub ghosts: Vec<Ghost>,
    pub anchors: Vec<(WindowId, Rect)>,
    last_frame: u64,
}
impl Motion {
    fn live_index(&self, id: WindowId) -> Option<usize> {
        self.live.iter().position(|l| l.id == id)
    }
    pub(crate) fn active(&self) -> bool {
        !self.ghosts.is_empty() || !self.live.is_empty()
    }
}
/// Resting bounds of a window's layer: the outer rect plus its shadow.
pub(crate) fn layer_bounds(window: &Window) -> Option<Rect> {
    if window.state == WindowState::Maximized {
        Some(window.outer)
    } else {
        crate::shadow::window_shadow_bounds(window.outer, window.scale)
    }
}
impl Engine {
    /// Enable or disable transitions (off honors "reduce motion"). Disabling
    /// completes every running transition at its final state immediately.
    pub fn set_animations(&mut self, enabled: bool) {
        if !enabled {
            self.finish_motion();
        }
        self.motion.enabled = enabled;
    }
    pub fn animations_enabled(&self) -> bool {
        self.motion.enabled
    }
    /// Animation speed (Doors extra): 0 normal, 1 fast, 2 slow.
    pub fn set_animation_speed(&mut self, speed: u32) {
        let percent = match speed {
            1 => 60,
            2 => 160,
            _ => 100,
        };
        DURATION_PERCENT.store(percent, core::sync::atomic::Ordering::Relaxed);
    }
    /// Trusted shell geometry used as the minimize/restore target (a taskbar
    /// button) for a top-level window, in physical output pixels.
    pub fn set_taskbar_anchor(&mut self, id: WindowId, anchor: Option<Rect>) {
        self.motion.anchors.retain(|(window, _)| *window != id);
        if let Some(anchor) = anchor
            && self.window(id).is_some()
        {
            self.motion.anchors.push((id, anchor));
        }
    }
    /// True while a transition needs further frames (not merely waiting for
    /// a client's first frame).
    pub fn animating(&self) -> bool {
        let now = self.motion.now;
        !self.motion.ghosts.is_empty()
            || self
                .motion
                .live
                .iter()
                .any(|l| l.transition.is_some_and(|t| !t.done(now)))
    }
    /// Kinds and earliest start of the transitions still short of their
    /// final pose, for presentation diagnostics (MOTION-01). Read-only.
    pub fn transition_activity(&self) -> TransitionActivity {
        let now = self.motion.now;
        let running = self
            .motion
            .live
            .iter()
            .filter_map(|l| l.transition)
            .chain(self.motion.ghosts.iter().map(|g| g.transition))
            .filter(|t| !t.done(now));
        let mut activity = TransitionActivity {
            now,
            start: u64::MAX,
            kinds: 0,
        };
        for transition in running {
            activity.kinds |= transition.kind;
            activity.start = activity.start.min(transition.start);
        }
        if activity.kinds == 0 {
            activity.start = 0;
        }
        activity
    }
    /// Ghost images still composed (for diagnostics and leak checks).
    pub fn transition_resources(&self) -> (usize, usize) {
        (
            self.motion.ghosts.len(),
            self.motion
                .ghosts
                .iter()
                .map(|g| g.layer.pixels.len())
                .chain(
                    self.motion
                        .live
                        .iter()
                        .filter_map(|l| l.layer.as_ref().map(|l| l.pixels.len())),
                )
                .sum(),
        )
    }
    /// Advance the trusted transition clock (milliseconds, monotonic) and
    /// queue damage for every pose that changed. Idle scenes queue nothing.
    /// Returns whether more animation frames are needed.
    pub fn advance(&mut self, now: u64) -> bool {
        let now = now.max(self.motion.now);
        if !self.motion.active() {
            self.motion.now = now;
            return false;
        }
        let paced = now >= self.motion.last_frame.saturating_add(FRAME_MS);
        let mut damage: Vec<Rect> = Vec::new();
        let mut index = 0;
        while index < self.motion.live.len() {
            let live = &self.motion.live[index];
            let id = live.id;
            let Some(window) = self.window(id) else {
                self.motion.live.remove(index);
                continue;
            };
            let bounds = layer_bounds(window);
            match live.transition {
                None if now.saturating_sub(live.created) >= FIRST_FRAME_MS => {
                    damage.extend(bounds);
                    self.motion.live.remove(index);
                    continue;
                }
                None => (),
                // Finished, but the client has not repainted at its new size
                // yet: keep showing the stretched last frame at rest (one
                // repaint, then idle) until it does, for a bounded time.
                Some(transition)
                    if transition.done(now)
                        && Self::snapshot_stale(window)
                        && now < transition.start + transition.duration + STRETCH_HOLD_MS =>
                {
                    if live.drawn != bounds {
                        damage.extend(bounds);
                        damage.extend(live.drawn);
                        self.motion.live[index].drawn = bounds;
                    }
                }
                Some(transition) if transition.done(now) => {
                    damage.extend(bounds);
                    damage.extend(live.drawn);
                    self.motion.live.remove(index);
                    continue;
                }
                Some(transition) if paced => {
                    let pose = bounds.and_then(|b| transition.pose(now).apply(b));
                    damage.extend(live.drawn);
                    damage.extend(pose);
                    self.motion.live[index].drawn = pose;
                }
                Some(_) => (),
            }
            index += 1;
        }
        let mut index = 0;
        while index < self.motion.ghosts.len() {
            let ghost = &self.motion.ghosts[index];
            if ghost.transition.done(now) {
                damage.extend(ghost.drawn);
                self.motion.ghosts.remove(index);
                continue;
            }
            if paced {
                let pose = ghost.transition.pose(now).apply(ghost.layer.bounds);
                damage.extend(ghost.drawn);
                damage.extend(pose);
                self.motion.ghosts[index].drawn = pose;
            }
            index += 1;
        }
        if paced {
            self.motion.last_frame = now;
        }
        self.motion.now = now;
        for rect in damage {
            self.dirty(rect);
        }
        self.animating()
    }
    pub(crate) fn finish_motion(&mut self) {
        let mut damage: Vec<Rect> = Vec::new();
        for live in core::mem::take(&mut self.motion.live) {
            damage.extend(live.drawn);
            damage.extend(self.window(live.id).and_then(layer_bounds));
        }
        for ghost in core::mem::take(&mut self.motion.ghosts) {
            damage.extend(ghost.drawn);
        }
        for rect in damage {
            self.dirty(rect);
        }
    }
    pub(crate) fn motion_created(&mut self, id: WindowId) {
        if self.motion.enabled {
            self.motion.live.retain(|l| l.id != id);
            self.motion.live.push(Live {
                id,
                created: self.motion.now,
                transition: None,
                layer: None,
                drawn: None,
            });
        }
    }
    pub(crate) fn motion_first_frame(&mut self, id: WindowId) {
        let now = self.motion.now;
        if let Some(live) = self
            .motion
            .live
            .iter_mut()
            .find(|l| l.id == id && l.transition.is_none())
        {
            live.transition = Some(Transition {
                from: Pose::FADED,
                to: Pose::REST,
                start: now,
                duration: scaled(OPEN_MS),
                easing: Easing::Decelerate,
                kind: TRANSITION_OPEN,
            });
            if let Some(bounds) = self.window(id).and_then(layer_bounds) {
                self.dirty(bounds);
            }
        }
    }
    /// Current pose of a window still showing a transition, then forget it.
    fn take_live_pose(&mut self, id: WindowId) -> (Option<Pose>, Option<Layer>, Option<Rect>) {
        let Some(index) = self.motion.live_index(id) else {
            return (None, None, None);
        };
        let live = self.motion.live.remove(index);
        let pose = live
            .transition
            .map(|t| t.pose(self.motion.now))
            .or(Some(Pose {
                alpha: 0,
                ..Pose::FADED
            }));
        (pose, live.layer, live.drawn)
    }
    /// Retain the window's current image as a ghost transitioning to `to`.
    /// Called while the window is still visible, before its state changes.
    pub(crate) fn motion_leave(&mut self, id: WindowId, minimize: bool) {
        let (pose, cached, drawn) = self.take_live_pose(id);
        if !self.motion.enabled || !self.visible(id) {
            if let Some(drawn) = drawn {
                self.dirty(drawn);
            }
            return;
        }
        let Some(window) = self.window(id) else {
            return;
        };
        if window.snapshot.is_none() || pose.is_some_and(|p| p.alpha == 0) {
            // Never-painted (or still invisible) windows simply vanish.
            if let Some(drawn) = drawn {
                self.dirty(drawn);
            }
            return;
        }
        let from = pose.unwrap_or(Pose::REST);
        let key = self.layer_key(window);
        let layer = match cached {
            Some(layer) if layer.key.is_some() && layer.key == Some(key) => Some(layer),
            _ => {
                let base = self.static_base(window);
                self.build_layer(window, &base, None, Some(key))
            }
        };
        let Some(layer) = layer else {
            return;
        };
        let anchor = self
            .motion
            .anchors
            .iter()
            .find(|(window, _)| *window == id)
            .map(|(_, r)| *r);
        let to = match (minimize, anchor) {
            (true, Some(anchor)) => Pose::toward(layer.bounds, anchor, 0),
            _ => Pose::FADED,
        };
        let below = {
            let position = self.z_order.iter().position(|w| *w == id);
            position.and_then(|i| {
                self.z_order[..i]
                    .iter()
                    .rev()
                    .copied()
                    .find(|w| self.visible(*w))
            })
        };
        let transition = Transition {
            from,
            to,
            start: self.motion.now,
            duration: scaled(if minimize { MINIMIZE_MS } else { CLOSE_MS }),
            easing: Easing::Accelerate,
            kind: if minimize {
                TRANSITION_MINIMIZE
            } else {
                TRANSITION_CLOSE
            },
        };
        let drawn = drawn.or_else(|| from.apply(layer.bounds));
        // Bounded retention: the oldest ghosts finish early.
        let bytes = layer.pixels.len();
        while !self.motion.ghosts.is_empty()
            && (self.motion.ghosts.len() >= MAX_GHOSTS
                || self.transition_resources().1 + bytes > MAX_LAYER_BYTES)
        {
            let old = self.motion.ghosts.remove(0);
            if let Some(drawn) = old.drawn {
                self.dirty(drawn);
            }
        }
        if let Some(drawn) = drawn {
            self.dirty(drawn);
        }
        self.motion.ghosts.retain(|g| g.source != id);
        self.motion.ghosts.push(Ghost {
            source: id,
            below,
            layer,
            transition,
            drawn,
        });
    }
    /// A window becomes visible again (restore, cancelled close): animate it
    /// in from its ghost's current pose, the taskbar anchor, or faded.
    pub(crate) fn motion_return(&mut self, id: WindowId, from_anchor: bool) {
        let now = self.motion.now;
        let ghost = self
            .motion
            .ghosts
            .iter()
            .position(|g| g.source == id)
            .map(|i| self.motion.ghosts.remove(i));
        if let Some(drawn) = ghost.as_ref().and_then(|g| g.drawn) {
            self.dirty(drawn);
        }
        if !self.motion.enabled {
            return;
        }
        let Some(bounds) = self.window(id).and_then(layer_bounds) else {
            return;
        };
        let anchor = self
            .motion
            .anchors
            .iter()
            .find(|(window, _)| *window == id)
            .map(|(_, r)| *r);
        let from = match (&ghost, anchor) {
            (Some(ghost), _) if ghost.layer.bounds == bounds => ghost.transition.pose(now),
            (_, Some(anchor)) if from_anchor => Pose::toward(bounds, anchor, 0),
            _ => Pose::FADED,
        };
        self.motion.live.retain(|l| l.id != id);
        self.motion.live.push(Live {
            id,
            created: now,
            transition: Some(Transition {
                from,
                to: Pose::REST,
                start: now,
                duration: scaled(if from_anchor { RESTORE_MS } else { OPEN_MS }),
                easing: Easing::Decelerate,
                kind: if from_anchor {
                    TRANSITION_RESTORE
                } else {
                    TRANSITION_OPEN
                },
            }),
            layer: None,
            drawn: from.apply(bounds),
        });
        self.dirty(bounds);
    }
    /// A window just placed elsewhere (snap) glides from its previous outer
    /// bounds into place. Input already uses the new geometry; only retained
    /// pixels move, and the client repaints at the new size as usual.
    pub(crate) fn motion_relocate(&mut self, id: WindowId, from: Rect) {
        if !self.motion.enabled
            || self
                .motion
                .live
                .iter()
                .any(|l| l.id == id && l.transition.is_none())
        {
            return;
        }
        let Some(window) = self.window(id) else {
            return;
        };
        let (Some(bounds), outer) = (layer_bounds(window), window.outer) else {
            return;
        };
        let pose = Pose::from_rect(outer, from);
        let previous = self
            .motion
            .live_index(id)
            .and_then(|i| self.motion.live.remove(i).drawn);
        let drawn = pose.apply(bounds);
        for rect in [previous, drawn, Some(bounds)].into_iter().flatten() {
            self.dirty(rect);
        }
        self.motion.live.push(Live {
            id,
            created: self.motion.now,
            transition: Some(Transition {
                from: pose,
                to: Pose::REST,
                start: self.motion.now,
                duration: scaled(SNAP_MS),
                easing: Easing::PointToPoint,
                kind: TRANSITION_RELOCATE,
            }),
            layer: None,
            drawn,
        });
    }
    pub(crate) fn motion_destroyed(&mut self, id: WindowId) {
        if let Some(index) = self.motion.live_index(id) {
            let live = self.motion.live.remove(index);
            if let Some(drawn) = live.drawn {
                self.dirty(drawn);
            }
        }
        // A ghost anchored above this window keeps the (now absent) anchor
        // and finishes above every window instead of dropping to the bottom.
        self.motion.anchors.retain(|(window, _)| *window != id);
    }
    pub(crate) fn layer_key(&self, window: &Window) -> LayerKey {
        LayerKey {
            revision: window.snapshot.as_ref().map_or(0, |s| s.revision),
            outer: window.outer,
            focused: self.focused == Some(window.id),
            state: window.state,
            theme: self.theme,
            reduce_transparency: self.reduce_transparency,
            caption: [HitRegion::Minimize, HitRegion::Maximize, HitRegion::Close]
                .map(|region| self.caption_visual(window.id, region)),
        }
    }
    /// Paint the window, its shadow and caption controls into a private
    /// premultiplied layer at its resting bounds. Optional trusted chrome
    /// (title text) receives window information translated to the layer.
    pub(crate) fn build_layer(
        &self,
        window: &Window,
        base: &render::Base<'_>,
        chrome: Option<&mut render::ChromeCallback<'_>>,
        key: Option<LayerKey>,
    ) -> Option<Layer> {
        let bounds = layer_bounds(window)?;
        // A snapshot from before the latest configure is painted at its own
        // geometry (a frame of the size it was drawn for) and stretched to
        // the new bounds when composited, instead of exposing empty tails.
        let (outer, content) = stale_geometry(window).unwrap_or((window.outer, window.content()));
        let image = if window.state == WindowState::Maximized {
            outer
        } else {
            crate::shadow::window_shadow_bounds(outer, window.scale)?
        };
        let bytes = (image.width() as usize)
            .checked_mul(image.height() as usize)?
            .checked_mul(4)?;
        if bytes > MAX_LAYER_BYTES {
            return None;
        }
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(bytes).ok()?;
        pixels.resize(bytes, 0);
        {
            let mut canvas = Canvas::new(
                &mut pixels,
                image.width(),
                image.height(),
                image.width() as usize * 4,
            )
            .ok()?;
            let placed =
                render::Placed::with_geometry(window, outer, content, (image.x(), image.y()))?;
            self.paint_window_layer(placed, base, &mut canvas, chrome);
        }
        Some(Layer {
            bounds,
            width: image.width(),
            height: image.height(),
            pixels,
            key,
        })
    }
    /// The live window still shows a frame drawn for another size.
    pub(crate) fn snapshot_stale(window: &Window) -> bool {
        stale_geometry(window).is_some()
    }
}
/// (outer, content) the window had when its current snapshot was drawn,
/// when that differs from its current geometry. Client-caption windows draw
/// the whole outer rectangle; others keep their trusted title bar.
fn stale_geometry(window: &Window) -> Option<(Rect, Rect)> {
    let snapshot = window.snapshot.as_ref()?;
    let content = window.content();
    let (width, height) = (snapshot.descriptor.width, snapshot.descriptor.height);
    if (width, height) == (content.width(), content.height()) {
        return None;
    }
    let chrome = window.outer.height() - content.height();
    let outer = Rect::new(window.outer.x(), window.outer.y(), width, height + chrome).ok()?;
    let content = Rect::new(content.x(), content.y(), width, height).ok()?;
    Some((outer, content))
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Floating-point reference of the same cubic bezier (host only).
    fn reference([x1, y1, x2, y2]: [i64; 4], t: f64) -> f64 {
        let p = |s: f64, a: f64, b: f64| {
            3.0 * a * s * (1.0 - s) * (1.0 - s) + 3.0 * b * s * s * (1.0 - s) + s * s * s
        };
        let (x1, y1, x2, y2) = (
            x1 as f64 / 1000.0,
            y1 as f64 / 1000.0,
            x2 as f64 / 1000.0,
            y2 as f64 / 1000.0,
        );
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..60 {
            let mid = (lo + hi) / 2.0;
            if p(mid, x1, x2) < t {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        p((lo + hi) / 2.0, y1, y2)
    }
    #[test]
    fn integer_bezier_matches_floating_reference_and_is_monotonic() {
        for easing in [Easing::Decelerate, Easing::Accelerate, Easing::PointToPoint] {
            assert_eq!(ease(easing, 0), 0);
            assert_eq!(ease(easing, ONE), ONE);
            let mut last = 0;
            for t in 0..=ONE {
                let v = ease(easing, t);
                assert!(v >= last, "{easing:?} not monotonic at {t}");
                last = v;
                let expected = reference(easing.spline(), t as f64 / ONE as f64) * ONE as f64;
                assert!(
                    (v as f64 - expected).abs() <= 1.5,
                    "{easing:?} t={t}: {v} vs {expected}"
                );
            }
        }
        // Entrances cover most of the distance early; exits start gently.
        assert!(ease(Easing::Decelerate, ONE / 4) > ONE * 6 / 10);
        assert!(ease(Easing::Accelerate, ONE / 4) < ONE / 10);
    }
}

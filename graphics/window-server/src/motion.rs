//! Time-based shell motion. Times are microseconds from the compositor's
//! cycle-calibrated `telemetry::TransitionClock`, so transitions advance by
//! real elapsed time rather than per frame.
/// Windows 11 motion curves (Fluent "fast out, slow in" deceleration for
/// entrances, acceleration for exits) as fixed-point cubic beziers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    Decelerate,
    /// WinUI ControlFastOutSlowInKeySpline (0,0,0,1): Start's measured rise.
    FastOutSlowIn,
    Accelerate,
    Linear,
}
/// Fluent key splines sampled at 33 points (no floating point in the guest).
/// Decelerate: cubic-bezier(0.1, 0.9, 0.2, 1); accelerate: (0.7, 0, 1, 0.5).
const DECELERATE: [u16; 33] = [
    0, 251, 435, 564, 657, 725, 777, 817, 849, 875, 896, 914, 928, 940, 950, 959, 966, 972, 977,
    982, 985, 989, 991, 993, 995, 996, 997, 998, 999, 999, 1000, 1000, 1000,
];
/// cubic-bezier(0, 0, 0, 1).
const FAST_OUT_SLOW_IN: [u16; 33] = [
    0, 235, 347, 432, 500, 558, 608, 652, 691, 725, 757, 785, 810, 833, 854, 873, 890, 905, 919,
    932, 943, 953, 962, 970, 976, 982, 987, 991, 994, 997, 999, 1000, 1000,
];
const ACCELERATE: [u16; 33] = [
    0, 0, 1, 3, 6, 9, 13, 18, 24, 30, 38, 47, 57, 68, 80, 94, 109, 125, 143, 163, 185, 209, 236,
    266, 298, 335, 376, 423, 476, 540, 619, 726, 1000,
];
fn sample(table: &[u16; 33], t: u32) -> u32 {
    let t = t.min(1000);
    let position = t * 32;
    let i = (position / 1000) as usize;
    if i >= 32 {
        return table[32] as u32;
    }
    let frac = position % 1000;
    let a = table[i] as u32;
    let b = table[i + 1] as u32;
    (a * (1000 - frac) + b * frac + 500) / 1000
}
impl Curve {
    /// Progress (0..=1000) for linear time progress (0..=1000).
    pub fn ease(self, t: u32) -> u32 {
        match self {
            Self::Decelerate => sample(&DECELERATE, t),
            Self::FastOutSlowIn => sample(&FAST_OUT_SLOW_IN, t),
            Self::Accelerate => sample(&ACCELERATE, t),
            Self::Linear => t.min(1000),
        }
    }
}
/// One open/close transition of a shell surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transition {
    /// Target state.
    pub open: bool,
    started: u64,
    duration: u64,
    /// Progress (0..=1000) at the moment the transition started, so a reversal
    /// mid-way continues from the current position instead of jumping.
    from: u32,
    /// An animated leg ran and its exact final frame is still owed
    /// (`take_settled`).
    settle: bool,
    /// Curve of the opening leg (the closing leg always accelerates).
    entrance: Curve,
}
impl Default for Transition {
    fn default() -> Self {
        Self {
            open: false,
            started: 0,
            duration: 0,
            from: 0,
            settle: false,
            entrance: Curve::Decelerate,
        }
    }
}
/// Open/close timings (µs): Windows 11 Start rises from the screen edge in
/// 167 ms on the fast-out-slow-in spline (measured, VIS-01 `start.open`);
/// the tray flyouts enter in as long with a short decelerating slide and fade
/// (`FlyoutOpen`, unmeasured); both exit faster with acceleration.
pub const OPEN_US: u64 = door_design_tokens::motion(door_design_tokens::MotionToken::StartOpen)
    .duration_ms as u64
    * 1000;
pub const CLOSE_US: u64 = door_design_tokens::motion(door_design_tokens::MotionToken::StartClose)
    .duration_ms as u64
    * 1000;
impl Transition {
    /// Start's transition: the measured fast-out-slow-in rise.
    pub const fn start() -> Self {
        Self {
            open: false,
            started: 0,
            duration: 0,
            from: 0,
            settle: false,
            entrance: Curve::FastOutSlowIn,
        }
    }
    pub fn set(&mut self, open: bool, now: u64, reduce_motion: bool) -> bool {
        if open == self.open && (self.duration != 0 || self.from == if open { 1000 } else { 0 }) {
            return false;
        }
        // Continue from the current visual position; a reversed leg takes
        // time in proportion to the distance still to travel.
        let current = self.progress(now);
        self.open = open;
        self.started = now;
        self.from = current;
        let distance = u64::from(if open { 1000 - current } else { current });
        let full = if open { OPEN_US } else { CLOSE_US };
        self.duration = if reduce_motion || distance == 0 {
            0
        } else {
            (full * distance / 1000).max(60_000)
        };
        if self.duration == 0 {
            self.from = if open { 1000 } else { 0 };
        }
        self.settle = self.duration != 0;
        true
    }
    /// Snap to a state without animating.
    pub fn jump(&mut self, open: bool) {
        *self = Self {
            open,
            started: 0,
            duration: 0,
            from: if open { 1000 } else { 0 },
            settle: false,
            entrance: self.entrance,
        };
    }
    /// Linear position 0 (closed) ..= 1000 (open).
    fn raw(&self, now: u64) -> u32 {
        let target = if self.open { 1000 } else { 0 };
        if self.duration == 0 {
            return target;
        }
        let elapsed = now.saturating_sub(self.started).min(self.duration);
        let span = (target as i64 - self.from as i64) * elapsed as i64 / self.duration as i64;
        (self.from as i64 + span).clamp(0, 1000) as u32
    }
    /// Eased visual progress 0 (hidden) ..= 1000 (fully shown).
    pub fn progress(&self, now: u64) -> u32 {
        let raw = self.raw(now);
        if self.duration == 0 {
            return raw;
        }
        // Map the remaining fraction of this leg through the leg's curve.
        let (lo, hi) = if self.open {
            (self.from, 1000)
        } else {
            (0, self.from)
        };
        if hi <= lo {
            return raw;
        }
        let local = ((raw - lo) as u64 * 1000 / (hi - lo) as u64) as u32;
        let eased = if self.open {
            self.entrance.ease(local)
        } else {
            1000 - Curve::Accelerate.ease(1000 - local)
        };
        lo + ((hi - lo) as u64 * eased as u64 / 1000) as u32
    }
    pub fn animating(&self, now: u64) -> bool {
        self.duration != 0 && now < self.started + self.duration
    }
    /// Start (µs) of the leg in progress, while it animates.
    pub fn animating_since(&self, now: u64) -> Option<u64> {
        self.animating(now).then_some(self.started)
    }
    /// True once after an animated leg ends: the caller presents one more
    /// frame so the exact final pose reaches the screen even when the last
    /// animated frame was drawn just before the end.
    pub fn take_settled(&mut self, now: u64) -> bool {
        if self.settle && !self.animating(now) {
            self.settle = false;
            return true;
        }
        false
    }
    /// Shown at all (open or still fading out).
    pub fn visible(&self, now: u64) -> bool {
        self.open || self.progress(now) > 0
    }
}
/// Hover and press feedback (WinUI ControlFasterAnimationDuration, 83 ms)
/// and small state changes such as toggles (ControlFastAnimationDuration,
/// 167 ms), in µs.
pub const FASTER_US: u64 =
    door_design_tokens::motion(door_design_tokens::MotionToken::Faster).duration_ms as u64 * 1000;
pub const FAST_US: u64 =
    door_design_tokens::motion(door_design_tokens::MotionToken::Fast).duration_ms as u64 * 1000;
/// One final frame is owed this long after a tween's nominal end, so the
/// exact resting value is always painted.
const SETTLE_US: u64 = 16_000;
/// A value (0..=1000) easing toward a target over real elapsed time: hover
/// plates, press scales, toggle fills, rotations. A new target starts from
/// wherever the value is, so a reversal mid-way never jumps, and a shorter
/// distance takes proportionally less time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tween {
    from: u32,
    to: u32,
    started: u64,
    duration: u64,
    curve: Curve,
}
impl Default for Tween {
    fn default() -> Self {
        Self::at(0)
    }
}
impl Tween {
    /// At rest at `value`.
    pub const fn at(value: u32) -> Self {
        Self {
            from: value,
            to: value,
            started: 0,
            duration: 0,
            curve: Curve::Linear,
        }
    }
    /// The value now.
    pub fn value(&self, now: u64) -> u32 {
        if self.duration == 0 || now >= self.started + self.duration {
            return self.to;
        }
        let t = ((now.saturating_sub(self.started)) * 1000 / self.duration) as u32;
        let e = self.curve.ease(t) as i64;
        (self.from as i64 + (self.to as i64 - self.from as i64) * e / 1000) as u32
    }
    pub fn target(&self) -> u32 {
        self.to
    }
    /// Head for `target` over at most `full` µs (0 jumps: reduced motion).
    /// False when it is already heading there.
    pub fn set(&mut self, target: u32, now: u64, full: u64, curve: Curve) -> bool {
        let target = target.min(1000);
        if target == self.to {
            return false;
        }
        let current = self.value(now);
        let distance = u64::from(current.abs_diff(target));
        *self = Self {
            from: current,
            to: target,
            started: now,
            duration: full * distance / 1000,
            curve,
        };
        true
    }
    /// Still moving, or owing its final frame.
    pub fn animating(&self, now: u64) -> bool {
        self.duration != 0 && now < self.started + self.duration + SETTLE_US
    }
}
/// Vertical slide offset (logical px) and opacity (0..=255) for `progress`.
/// Offset (logical px) of a surface that rises `distance` px into place
/// without fading (Start, measured).
pub fn rise(progress: u32, distance: i32) -> i32 {
    distance * (1000 - progress.min(1000) as i32) / 1000
}
pub fn slide(progress: u32, distance: i32) -> (i32, u8) {
    let p = progress.min(1000) as i32;
    let offset = distance * (1000 - p) / 1000;
    // Opacity reaches full a little before the slide settles, as in Windows.
    let opacity = ((p * 255 * 10 / 7) / 1000).min(255) as u8;
    (offset, opacity)
}
#[cfg(test)]
mod tests {
    use super::*;
    /// The sampled tables are the design-token splines (Start/flyout open
    /// and close tracks), within one table unit.
    #[test]
    fn sampled_tables_match_the_motion_token_splines() {
        use door_design_tokens::{Easing, MotionToken, motion};
        fn bezier(easing: Easing, t: f64) -> f64 {
            let Easing::CubicBezier { x1, y1, x2, y2 } = easing else {
                return t;
            };
            let p = |s: f64, a: u16, b: u16| {
                let (a, b) = (f64::from(a) / 1000.0, f64::from(b) / 1000.0);
                3.0 * a * s * (1.0 - s) * (1.0 - s) + 3.0 * b * s * s * (1.0 - s) + s * s * s
            };
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
        for (table, token) in [
            (&FAST_OUT_SLOW_IN, MotionToken::StartOpen),
            (&DECELERATE, MotionToken::FlyoutOpen),
            (&ACCELERATE, MotionToken::StartClose),
        ] {
            for (i, v) in table.iter().enumerate() {
                let expected = bezier(motion(token).easing, i as f64 / 32.0) * 1000.0;
                assert!(
                    (f64::from(*v) - expected).abs() <= 1.5,
                    "{token:?} sample {i}: {v} vs {expected}"
                );
            }
        }
        // One opening duration (OPEN_US) serves Start and the flyouts.
        assert_eq!(
            motion(MotionToken::FlyoutOpen).duration_ms,
            motion(MotionToken::StartOpen).duration_ms
        );
    }
    #[test]
    fn curves_are_monotonic_and_anchored() {
        for curve in [
            Curve::Decelerate,
            Curve::FastOutSlowIn,
            Curve::Accelerate,
            Curve::Linear,
        ] {
            assert_eq!(curve.ease(0), 0);
            assert_eq!(curve.ease(1000), 1000);
            let mut last = 0;
            for t in (0..=1000).step_by(25) {
                let v = curve.ease(t);
                assert!(v >= last, "{curve:?} at {t}");
                last = v;
            }
        }
        // Deceleration covers most of the distance early.
        assert!(Curve::Decelerate.ease(250) > 600);
        assert!(Curve::Accelerate.ease(250) < 200);
    }
    #[test]
    fn transition_runs_on_time_reverses_smoothly_and_honors_reduce_motion() {
        let mut t = Transition::default();
        assert!(!t.visible(0));
        assert!(t.set(true, 1_000, false));
        assert!(t.animating(1_000));
        assert_eq!(t.progress(1_000), 0);
        let mid = t.progress(1_000 + OPEN_US / 2);
        assert!(mid > 500 && mid < 1000);
        assert_eq!(t.progress(1_000 + OPEN_US), 1000);
        assert!(!t.animating(1_000 + OPEN_US));
        assert_eq!(t.animating_since(1_000 + OPEN_US / 2), Some(1_000));
        assert_eq!(t.animating_since(1_000 + OPEN_US), None);
        assert!(!t.take_settled(1_000 + OPEN_US - 1), "still animating");
        assert!(t.take_settled(1_000 + OPEN_US), "one final frame");
        assert!(!t.take_settled(1_000 + OPEN_US + 1));
        assert!(!t.set(true, 1_000 + OPEN_US, false));
        // Close half-way through, then reopen: no jump.
        let at = 1_000 + OPEN_US + 10;
        assert!(t.set(false, at, false));
        let before = t.progress(at + CLOSE_US / 3);
        assert!(before < 1000 && before > 0);
        let reopen = at + CLOSE_US / 3;
        t.set(true, reopen, false);
        assert!(t.progress(reopen).abs_diff(before) <= 30);
        assert_eq!(t.progress(reopen + OPEN_US), 1000);
        let mut r = Transition::default();
        r.set(true, 0, true);
        assert_eq!(r.progress(0), 1000);
        assert!(!r.animating(0));
        r.set(false, 5, true);
        assert!(!r.visible(5));
        assert!(!r.take_settled(5), "reduced motion owes no settle frame");
    }
    #[test]
    fn tweens_ease_reverse_from_where_they_are_and_settle() {
        let mut t = Tween::default();
        assert_eq!(t.value(0), 0);
        assert!(!t.animating(0));
        assert!(t.set(1000, 100, FASTER_US, Curve::Decelerate));
        assert!(
            !t.set(1000, 120, FASTER_US, Curve::Decelerate),
            "already heading there"
        );
        assert_eq!(t.value(100), 0);
        let mid = t.value(100 + FASTER_US / 2);
        assert!(mid > 500 && mid < 1000, "{mid}");
        assert_eq!(t.value(100 + FASTER_US), 1000);
        assert!(t.animating(100 + FASTER_US), "owes its final frame");
        assert!(!t.animating(100 + FASTER_US + SETTLE_US));
        // Release half-way: back from the current value, in proportion.
        let mut p = Tween::default();
        p.set(1000, 0, FAST_US, Curve::Linear);
        let at = FAST_US / 2;
        let half = p.value(at);
        p.set(0, at, FAST_US, Curve::Linear);
        assert_eq!(p.value(at), half);
        assert_eq!(
            p.value(at + FAST_US / 2),
            0,
            "half the distance, half the time"
        );
        // Reduced motion: a zero duration jumps.
        let mut r = Tween::at(1000);
        r.set(0, 5, 0, Curve::Decelerate);
        assert_eq!(r.value(5), 0);
        assert!(!r.animating(5));
    }
    #[test]
    fn slide_offsets_and_opacity() {
        assert_eq!(slide(0, 40), (40, 0));
        assert_eq!(slide(1000, 40), (0, 255));
        let (o, a) = slide(500, 40);
        assert_eq!(o, 20);
        assert!(a > 127);
    }
}

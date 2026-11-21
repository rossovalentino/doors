//! Presentation diagnostics must not spend the process budget on unchanged frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scene {
    pub app: u8,
    pub overlay: bool,
    pub overview: Option<bool>,
    pub activation: u64,
}
/// FRAME lines per process (about 90 bytes each): the first frame and the
/// first seven slow ones. Software rendering makes most frames slow, so a
/// larger count only spends the 16 KiB diagnostic budget the long desktop
/// workflow needs for its scene markers.
pub const FRAME_REPORTS: u8 = 8;
#[derive(Default)]
pub struct PresentationLog {
    activation: u64,
    last_scene: Option<Scene>,
    slow_samples: u8,
    /// MOTION-01 animation episodes (`AnimationTrace`).
    pub animation: AnimationTrace,
}
impl PresentationLog {
    pub fn activated(&mut self) {
        // Diagnostic sequence only; neither authority nor a resource identity.
        self.activation = self.activation.wrapping_add(1);
    }
    /// Capture before rendering; input during scanout belongs to the next frame.
    pub fn capture(&self, app: u8, overlay: bool, overview: Option<bool>) -> Scene {
        Scene {
            app,
            overlay,
            overview,
            activation: self.activation,
        }
    }
    /// Call only after successful transfer, preserving retry and input ordering.
    pub fn presented(&mut self, scene: Scene) -> bool {
        if self.superseded(scene) || self.last_scene == Some(scene) {
            return false;
        }
        self.last_scene = Some(scene);
        true
    }
    pub fn superseded(&self, scene: Scene) -> bool {
        scene.activation != self.activation
    }
    /// An overlay-only frame (shell surfaces opening, animating or closing)
    /// over a window scene with no pending damage completes a foreground
    /// scene too. `captured` comes from `capture` before rendering; `overlay`
    /// is whether shell surfaces remain on screen after this frame. Returns
    /// the scene to report, under the same rules as `presented`.
    pub fn overlay_presented(&mut self, captured: Scene, overlay: bool) -> Option<Scene> {
        let scene = Scene {
            overlay,
            ..captured
        };
        self.presented(scene).then_some(scene)
    }
    /// First frame plus slow frames, at most `FRAME_REPORTS` samples per
    /// process, leaving credit for activation/close/failures. Never a
    /// per-frame lifetime stream.
    pub fn frame_sample(&mut self, serial: u64, elapsed_ticks: u64) -> bool {
        if (serial != 1 && elapsed_ticks < 8) || self.slow_samples >= FRAME_REPORTS {
            return false;
        }
        self.slow_samples += 1;
        true
    }
}
/// Shell bits of `AnimationTrace` kinds (the engine's `TRANSITION_*` bits
/// use 3..=7): Start, a taskbar flyout, and the snap bar or Snap Assist.
pub const ANIMATION_START: u8 = 1;
pub const ANIMATION_FLYOUT: u8 = 2;
pub const ANIMATION_SNAP_UI: u8 = 4;
/// At most this many records per process (about 1.3 KiB of the 16 KiB
/// diagnostic budget); later episodes are only counted in `n`. Records are
/// armed only once Animation effects were turned off and on again in the
/// session (MOTION-01's first step), so long workflows that never touch the
/// preference spend no diagnostic credit on them.
pub const ANIMATION_RECORDS: u8 = 24;
/// Guest-side animation timing for MOTION-01. Each successfully presented
/// frame reports which transitions were still short of their final pose
/// when it was composed. An episode runs from the earliest transition start
/// through its in-progress frames to the first frame composed with nothing
/// animating (the rest frame), and yields one `AnimationRecord`.
#[derive(Clone, Copy, Debug, Default)]
pub struct AnimationTrace {
    frames: u64,
    episode: Option<Episode>,
    emitted: u8,
    seen_reduced: bool,
    armed: bool,
}
#[derive(Clone, Copy, Debug)]
struct Episode {
    kinds: u8,
    start: u64,
    last: u64,
    moving: u32,
}
/// `DOOR_P12 ANIM k=<kinds hex> t=<start ms> l=<last in-progress frame>
/// e=<rest frame> p=<rest frame presented> f=<in-progress frames> n=<frame
/// count>`: l, e and p are ms after t on the compositor's transition clock
/// (l and e at composition, p after transfer); n counts every presented
/// frame, so gaps between records show frames presented while idle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnimationRecord {
    pub kinds: u8,
    pub start: u64,
    pub last: u64,
    pub rest: u64,
    pub presented: u64,
    pub moving: u32,
    pub frame: u64,
}
impl AnimationTrace {
    /// The session's reduce-motion preference as applied: off after having
    /// been on arms the records.
    pub fn preference(&mut self, reduce_motion: bool) {
        if reduce_motion {
            self.seen_reduced = true;
        } else if self.seen_reduced {
            self.armed = true;
        }
    }
    /// One presented frame: `composed` is the clock value its poses were
    /// drawn for, `active` the kinds still animating then (earliest start
    /// `start`), and `presented` the clock after its transfer.
    pub fn frame(
        &mut self,
        composed: u64,
        active: u8,
        start: u64,
        presented: u64,
    ) -> Option<AnimationRecord> {
        self.frames = self.frames.saturating_add(1);
        if active != 0 {
            let episode = self.episode.get_or_insert(Episode {
                kinds: 0,
                start,
                last: composed,
                moving: 0,
            });
            episode.kinds |= active;
            episode.start = episode.start.min(start);
            episode.last = composed;
            episode.moving = episode.moving.saturating_add(1);
            return None;
        }
        let episode = self.episode.take()?;
        if !self.armed || self.emitted >= ANIMATION_RECORDS {
            return None;
        }
        self.emitted += 1;
        Some(AnimationRecord {
            kinds: episode.kinds,
            start: episode.start,
            last: episode.last.saturating_sub(episode.start),
            rest: composed.saturating_sub(episode.start),
            presented: presented.max(composed).saturating_sub(episode.start),
            moving: episode.moving,
            frame: self.frames,
        })
    }
}
impl core::fmt::Display for AnimationRecord {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        writeln!(
            f,
            "DOOR_P12 ANIM k={:x} t={} l={} e={} p={} f={} n={}",
            self.kinds, self.start, self.last, self.rest, self.presented, self.moving, self.frame
        )
    }
}
/// Every completed profiling scope, including errors, is counted. The gesture
/// mode is captured before composition, never read again after input/scanout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameKind {
    Scene,
    Caption,
    Icons,
    Bar,
}
pub struct FrameWork {
    kind: FrameKind,
    dragging: bool,
    pub started: u64,
    pub composed: Option<u64>,
    /// TSC when composition finished (FrameClock), alongside the tick.
    pub composed_cycles: Option<u64>,
    pub completed: Option<u64>,
    pub bytes: u64,
    pub calls: u64,
}
impl FrameWork {
    pub fn new(kind: FrameKind, dragging: bool, started: u64) -> Self {
        Self {
            kind,
            dragging,
            started,
            composed: None,
            composed_cycles: None,
            completed: None,
            bytes: 0,
            calls: 0,
        }
    }
    pub fn dragging(&self) -> bool {
        self.dragging
    }
    pub fn mark_composed(&mut self, tick: u64) {
        self.composed = Some(tick);
        self.composed_cycles = Some(cycles());
    }
    pub fn finish(self, at: u64, failed: bool) -> Observation {
        let at = at.max(self.started);
        let composed = self.composed.unwrap_or(at).clamp(self.started, at);
        Observation {
            kind: self.kind,
            dragging: self.dragging,
            compose_ticks: composed - self.started,
            transfer_ticks: at - composed,
            bytes: self.bytes,
            calls: self.calls,
            failed,
        }
    }
}
pub struct Observation {
    pub kind: FrameKind,
    pub dragging: bool,
    pub compose_ticks: u64,
    pub transfer_ticks: u64,
    pub bytes: u64,
    pub calls: u64,
    pub failed: bool,
}
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct FrameTotals {
    frames: u32,
    compose: u64,
    transfer: u64,
    bytes: u64,
    calls: u64,
    maximum: u32,
    failed: u32,
    histogram: [u32; 7],
    paths: [u32; 4],
}
#[derive(Default)]
pub struct Motion {
    groups: [FrameTotals; 2],
    reported: bool,
}
impl Motion {
    pub fn record(&mut self, v: Observation) {
        if self.reported {
            return;
        }
        let g = &mut self.groups[usize::from(v.dragging)];
        let elapsed = v.compose_ticks.saturating_add(v.transfer_ticks);
        let bucket = match elapsed {
            0 => 0,
            1 => 1,
            2 => 2,
            3 => 3,
            4..=7 => 4,
            8..=15 => 5,
            _ => 6,
        };
        g.frames = g.frames.saturating_add(1);
        g.compose = g.compose.saturating_add(v.compose_ticks);
        g.transfer = g.transfer.saturating_add(v.transfer_ticks);
        g.bytes = g.bytes.saturating_add(v.bytes);
        g.calls = g.calls.saturating_add(v.calls);
        g.maximum = g.maximum.max(elapsed.min(u64::from(u32::MAX)) as u32);
        g.failed = g.failed.saturating_add(u32::from(v.failed));
        g.histogram[bucket] = g.histogram[bucket].saturating_add(1);
        g.paths[v.kind as usize] = g.paths[v.kind as usize].saturating_add(1);
    }
    /// At most two records per service incarnation. No per-frame text or heap.
    pub fn take_summary(&mut self) -> Option<[FrameTotals; 2]> {
        if core::mem::replace(&mut self.reported, true) {
            None
        } else {
            Some(self.groups)
        }
    }
}
#[derive(Clone, Copy)]
pub enum Cutoff {
    PowerAttempt,
    ServiceStop,
    ScopeExit,
}
impl Cutoff {
    fn tag(self) -> char {
        match self {
            Self::PowerAttempt => 'p',
            Self::ServiceStop => 's',
            Self::ScopeExit => 'e',
        }
    }
}
pub struct TotalsLine {
    pub dragging: bool,
    pub totals: FrameTotals,
    pub cutoff: Cutoff,
}
impl core::fmt::Display for TotalsLine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let g = self.totals;
        // Hex keeps even saturated counters inside the runtime's256B record.
        // h:0,1,2,3,4..7,8..15,16+ ticks; p:scene,caption,icons,bar.
        writeln!(
            f,
            "DOOR_P12 MOTION v=1 ms={} x=1 cut={} d={} n={:x} c={:x} t={:x} b={:x} k={:x} m={:x} f={:x} h={:x},{:x},{:x},{:x},{:x},{:x},{:x} p={:x},{:x},{:x},{:x}",
            door_abi::platform::MONOTONIC_TICK_MILLIS,
            self.cutoff.tag(),
            u8::from(self.dragging),
            g.frames,
            g.compose,
            g.transfer,
            g.bytes,
            g.calls,
            g.maximum,
            g.failed,
            g.histogram[0],
            g.histogram[1],
            g.histogram[2],
            g.histogram[3],
            g.histogram[4],
            g.histogram[5],
            g.histogram[6],
            g.paths[0],
            g.paths[1],
            g.paths[2],
            g.paths[3]
        )
    }
}

/// Frames kept for percentile reporting (newest win; fixed, allocated once).
pub const FRAME_SAMPLES: usize = 1024;
/// A key/button followed by no frame within this many ticks changed nothing
/// visible; its latency sample is dropped rather than charged to a later frame.
const LATENCY_HORIZON_TICKS: u64 = 50;
/// CPU timestamp counter. Calibrated against the monotonic tick at report
/// time; never used for scheduling.
pub fn cycles() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: RDTSC has no memory effects; user mode may execute it.
        unsafe { core::arch::x86_64::_rdtsc() }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}
/// Sub-tick frame timing: total CPU work and compose time per presented
/// frame, plus key/button arrival to transfer completion (submission to the
/// framebuffer, not photons).
pub struct FrameClock {
    origin: Option<(u64, u64)>,
    work: alloc::vec::Vec<u64>,
    compose: alloc::vec::Vec<u64>,
    latency: alloc::vec::Vec<u64>,
    frames: usize,
    latencies: usize,
    pending_input: Option<(u64, u64)>,
}
extern crate alloc;
impl FrameClock {
    pub fn new() -> Self {
        Self {
            origin: None,
            work: alloc::vec![0; FRAME_SAMPLES],
            compose: alloc::vec![0; FRAME_SAMPLES],
            latency: alloc::vec![0; FRAME_SAMPLES],
            frames: 0,
            latencies: 0,
            pending_input: None,
        }
    }
    pub fn start(&mut self, cycles: u64, tick: u64) {
        self.origin.get_or_insert((cycles, tick));
    }
    /// A discrete key or button event; the first since the last frame wins.
    pub fn input(&mut self, cycles: u64, tick: u64) {
        self.pending_input.get_or_insert((cycles, tick));
    }
    pub fn frame(&mut self, started: u64, composed: u64, ended: u64, tick: u64) {
        let at = self.frames % FRAME_SAMPLES;
        self.work[at] = ended.saturating_sub(started);
        self.compose[at] = composed.clamp(started, ended) - started;
        self.frames += 1;
        if let Some((input, input_tick)) = self.pending_input.take()
            && tick.saturating_sub(input_tick) <= LATENCY_HORIZON_TICKS
        {
            self.latency[self.latencies % FRAME_SAMPLES] = ended.saturating_sub(input);
            self.latencies += 1;
        }
    }
    /// One report, or None before a usable calibration window (≥1 s).
    pub fn summary(&self, cycles: u64, tick: u64, tick_millis: u64) -> Option<FrameTimeLine> {
        let (c0, t0) = self.origin?;
        let micros = tick.checked_sub(t0)?.checked_mul(tick_millis * 1000)?;
        if micros < 1_000_000 || cycles <= c0 {
            return None;
        }
        let per_us = (cycles - c0) / micros;
        if per_us == 0 {
            return None;
        }
        let stats = |samples: &[u64], count: usize| {
            let mut v: alloc::vec::Vec<u64> = samples[..count.min(FRAME_SAMPLES)]
                .iter()
                .map(|c| c / per_us)
                .collect();
            v.sort_unstable();
            percentiles(&v)
        };
        let work = stats(&self.work, self.frames);
        let n = self.frames.min(FRAME_SAMPLES);
        let over = |limit_us: u64| {
            self.work[..n]
                .iter()
                .filter(|c| **c / per_us > limit_us)
                .count()
        };
        Some(FrameTimeLine {
            frames: self.frames,
            work,
            compose: stats(&self.compose, self.frames),
            latencies: self.latencies,
            latency: stats(&self.latency, self.latencies),
            over_16_7ms: over(16_667),
            over_33_3ms: over(33_333),
            cycles_per_us: per_us,
        })
    }
}
impl Default for FrameClock {
    fn default() -> Self {
        Self::new()
    }
}
/// Millisecond clock for compositor transitions: the monotonic tick, refined
/// between ticks by the cycle counter once its rate is calibrated over at
/// least ten ticks. It never leads the tick by more than one tick and never
/// runs backwards.
#[derive(Clone, Copy, Debug, Default)]
pub struct TransitionClock {
    origin: Option<(u64, u64)>,
    /// First observation (cycles, tick) of the newest tick.
    seen: (u64, u64),
    per_ms: u64,
    last: u64,
}
impl TransitionClock {
    pub fn now_ms(&mut self, cycles: u64, tick: u64, tick_millis: u64) -> u64 {
        let (c0, t0) = *self.origin.get_or_insert((cycles, tick));
        if self.seen == (0, 0) || tick > self.seen.1 {
            self.seen = (cycles, tick);
            let elapsed = tick.saturating_sub(t0);
            if elapsed >= 10 && cycles > c0 {
                let rate = (cycles - c0) / (elapsed * tick_millis).max(1);
                if rate > 0 {
                    self.per_ms = rate;
                }
            }
        }
        let tick_time = tick.saturating_mul(tick_millis);
        let time = if self.per_ms > 0 && cycles >= self.seen.0 {
            (tick_time + (cycles - self.seen.0) / self.per_ms).min(tick_time + tick_millis)
        } else {
            tick_time
        };
        self.last = self.last.max(time);
        self.last
    }
}
/// (p50, p95, p99, max) in microseconds, nearest-rank.
fn percentiles(sorted: &[u64]) -> [u64; 4] {
    if sorted.is_empty() {
        return [0; 4];
    }
    let rank = |p: usize| sorted[((sorted.len() * p).div_ceil(100)).clamp(1, sorted.len()) - 1];
    [rank(50), rank(95), rank(99), sorted[sorted.len() - 1]]
}
pub struct FrameTimeLine {
    pub frames: usize,
    pub work: [u64; 4],
    pub compose: [u64; 4],
    pub latencies: usize,
    pub latency: [u64; 4],
    pub over_16_7ms: usize,
    pub over_33_3ms: usize,
    pub cycles_per_us: u64,
}
impl core::fmt::Display for FrameTimeLine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let [w50, w95, w99, wmax] = self.work;
        let [c50, c95, c99, cmax] = self.compose;
        let [l50, l95, l99, lmax] = self.latency;
        writeln!(
            f,
            "DOOR_P12 FRAMETIME v=1 unit=us n={} work={w50}/{w95}/{w99}/{wmax} compose={c50}/{c95}/{c99}/{cmax} over16={} over33={} input_n={} input={l50}/{l95}/{l99}/{lmax} cpu_mhz={}",
            self.frames, self.over_16_7ms, self.over_33_3ms, self.latencies, self.cycles_per_us
        )
    }
}
#[cfg(test)]
mod frame_clock_tests {
    use super::*;
    #[test]
    fn percentiles_calibration_and_latency_horizon() {
        let mut clock = FrameClock::new();
        // 1000 cycles per microsecond; 10 ms ticks.
        clock.start(0, 0);
        for i in 0..100u64 {
            let start = i * 50_000_000;
            // Work grows 1..=100 ms; compose is the first half.
            clock.frame(
                start,
                start + i * 500_000 + 500_000,
                start + (i + 1) * 1_000_000,
                i,
            );
        }
        clock.input(10, 0);
        clock.frame(20, 30, 2_010, 1); // 2 µs after the key
        clock.input(50, 0);
        clock.frame(60, 70, 80, 200); // too late: no visible response sample
        let s = clock.summary(10_000_000_000, 1000, 10).unwrap();
        assert_eq!(s.cycles_per_us, 1000);
        assert_eq!(s.frames, 102);
        assert_eq!(s.latencies, 1);
        assert_eq!(s.latency[0], 2);
        assert!(s.work[1] >= 95_000 && s.work[3] == 100_000);
        assert_eq!(s.over_16_7ms, 84);
        assert!(
            clock.summary(1, 5, 10).is_none(),
            "under one second is uncalibrated"
        );
    }
    #[test]
    fn transition_clock_interpolates_ticks_and_never_runs_backwards() {
        let mut clock = TransitionClock::default();
        // Uncalibrated: tick resolution only.
        assert_eq!(clock.now_ms(0, 100, 10), 1000);
        assert_eq!(clock.now_ms(5_000, 100, 10), 1000);
        // 1000 cycles per millisecond, calibrated after ten ticks.
        assert_eq!(clock.now_ms(100_000, 110, 10), 1100);
        assert_eq!(clock.now_ms(103_000, 110, 10), 1103);
        // Cycles far beyond the tick are clamped to one tick ahead.
        assert_eq!(clock.now_ms(200_000, 110, 10), 1110);
        // A late tick observation never moves time backwards.
        assert_eq!(clock.now_ms(150_000, 110, 10), 1110);
        assert_eq!(clock.now_ms(151_000, 111, 10), 1110);
        assert_eq!(clock.now_ms(152_000, 115, 10), 1150);
        assert_eq!(clock.now_ms(154_000, 115, 10), 1151);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlay_frames_complete_the_foreground_scene_after_a_shell_dismissal() {
        let mut log = PresentationLog::default();
        // Start open over the desktop, then an existing app is activated
        // from Start while Start still fades out (scene frame, overlay=1).
        assert!(log.presented(log.capture(0, true, None)));
        log.activated();
        let fading = log.capture(1, true, None);
        assert!(log.presented(fading));
        // Further fade frames are overlay-only and change nothing.
        let captured = log.capture(1, false, None);
        assert_eq!(log.overlay_presented(captured, true), None);
        // The dismissal frame removes the last surface: app=1 overlay=0.
        assert_eq!(
            log.overlay_presented(captured, false),
            Some(Scene {
                app: 1,
                overlay: false,
                overview: None,
                activation: captured.activation,
            })
        );
        assert_eq!(
            log.overlay_presented(captured, false),
            None,
            "reported once"
        );
        // An activation during the overlay frame's scanout is not claimed.
        let stale = log.capture(1, false, None);
        log.activated();
        assert_eq!(log.overlay_presented(stale, true), None);
    }
    #[test]
    fn long_unchanged_frames_preserve_credit_and_reactivation_reports_once() {
        let mut log = PresentationLog::default();
        assert!(log.presented(log.capture(2, false, None)));
        for _ in 0..100_000 {
            assert!(!log.presented(log.capture(2, false, None)));
            assert!(!log.frame_sample(2, 7));
        }
        log.activated();
        assert!(log.presented(log.capture(2, false, None)));
        assert!(!log.presented(log.capture(2, false, None)));
        assert!(log.presented(log.capture(0, true, None)));
        assert!(log.presented(log.capture(0, true, Some(false))));
        assert!(log.presented(log.capture(0, true, Some(true))));
        assert_eq!(
            (0..100_000).filter(|_| log.frame_sample(2, 8)).count(),
            usize::from(FRAME_REPORTS)
        );
    }
    #[test]
    fn first_fast_frame_is_measured_within_the_same_bounded_credit() {
        let mut log = PresentationLog::default();
        assert!(log.frame_sample(1, 0));
        assert!(!log.frame_sample(2, 0));
        assert!(!log.frame_sample(3, 7));
        assert_eq!(
            (0..100).filter(|_| log.frame_sample(4, 8)).count(),
            usize::from(FRAME_REPORTS) - 1
        );
        assert!(!log.frame_sample(101, u64::MAX));
    }
    #[test]
    fn activation_during_transfer_cannot_label_the_older_frame_as_ready() {
        let mut log = PresentationLog::default();
        let old = log.capture(1, false, None);
        assert!(log.presented(old));
        // A changed scene must also be rejected, not just an already logged one.
        let in_flight = log.capture(2, false, None);
        log.activated();
        assert!(log.superseded(in_flight));
        assert!(!log.presented(in_flight));
        let fresh = log.capture(2, false, None);
        assert_ne!(old.activation, fresh.activation);
        assert!(!log.superseded(fresh));
        assert!(log.presented(fresh));
        assert!(!log.presented(fresh));
    }
    #[test]
    fn totals_cover_all_paths_and_bins_with_the_original_gesture_mode() {
        let mut motion = Motion::default();
        let durations = [0, 1, 2, 3, 4, 7, 8, 15, 16, 40];
        for dragging in [false, true] {
            for kind in [
                FrameKind::Scene,
                FrameKind::Caption,
                FrameKind::Icons,
                FrameKind::Bar,
            ] {
                for duration in durations {
                    let mut work = FrameWork::new(kind, dragging, 10);
                    work.composed = Some(10 + duration / 2);
                    work.bytes = 16384;
                    work.calls = 1;
                    assert_eq!(work.dragging(), dragging);
                    motion.record(work.finish(10 + duration, duration == 40));
                }
            }
        }
        let summary = motion.take_summary().unwrap();
        assert_eq!(summary[0], summary[1]);
        assert_eq!(summary[0].frames, 40);
        assert_eq!(summary[0].paths, [10; 4]);
        assert_eq!(summary[0].histogram, [4, 4, 4, 4, 8, 8, 8]);
        assert_eq!(summary[0].failed, 4);
        assert_eq!(summary[0].maximum, 40);
        assert_eq!(summary[0].bytes, 40 * 16384);
        assert_eq!(summary[0].calls, 40);
        assert!(motion.take_summary().is_none());
    }
    #[test]
    fn aggregate_records_stay_inside_two256byte_records_even_when_saturated() {
        use core::fmt::Write;
        struct Bound {
            len: usize,
        }
        impl core::fmt::Write for Bound {
            fn write_str(&mut self, s: &str) -> core::fmt::Result {
                self.len += s.len();
                if self.len > 256 {
                    Err(core::fmt::Error)
                } else {
                    Ok(())
                }
            }
        }
        let full = FrameTotals {
            frames: u32::MAX,
            compose: u64::MAX,
            transfer: u64::MAX,
            bytes: u64::MAX,
            calls: u64::MAX,
            maximum: u32::MAX,
            failed: u32::MAX,
            histogram: [u32::MAX; 7],
            paths: [u32::MAX; 4],
        };
        let mut motion = Motion {
            groups: [full; 2],
            reported: false,
        };
        motion.record(Observation {
            kind: FrameKind::Bar,
            dragging: true,
            compose_ticks: u64::MAX,
            transfer_ticks: u64::MAX,
            bytes: u64::MAX,
            calls: u64::MAX,
            failed: true,
        });
        assert_eq!(motion.groups, [full; 2]);
        for (d, g) in motion.take_summary().unwrap().into_iter().enumerate() {
            let mut output = Bound { len: 0 };
            write!(
                &mut output,
                "{}",
                TotalsLine {
                    dragging: d == 1,
                    totals: g,
                    cutoff: Cutoff::PowerAttempt,
                }
            )
            .unwrap();
            assert!(output.len <= 256);
        }
    }
    #[test]
    fn early_failure_and_clock_regression_cannot_wrap_timing() {
        let work = FrameWork::new(FrameKind::Scene, true, 20);
        let v = work.finish(12, true);
        assert_eq!((v.compose_ticks, v.transfer_ticks, v.failed), (0, 0, true));
        let mut work = FrameWork::new(FrameKind::Caption, false, 20);
        work.composed = Some(u64::MAX);
        let v = work.finish(24, false);
        assert_eq!((v.compose_ticks, v.transfer_ticks), (4, 0));
    }
}
#[cfg(test)]
mod animation_tests {
    use super::*;
    #[test]
    fn animation_trace_reports_one_bounded_record_per_settled_episode() {
        let mut trace = AnimationTrace::default();
        // Unarmed: episodes settle silently; turning reduce motion off alone
        // (the boot default) does not arm.
        trace.preference(false);
        trace.frame(10, ANIMATION_START, 5, 12);
        assert_eq!(trace.frame(400, 0, 0, 410), None);
        trace.preference(true);
        trace.preference(false);
        // Idle frames open nothing.
        assert_eq!(trace.frame(900, 0, 0, 905), None);
        // Start opens at 1000: in-progress frames, then the rest frame.
        assert_eq!(trace.frame(1_010, ANIMATION_START, 1_000, 1_020), None);
        assert_eq!(trace.frame(1_200, ANIMATION_START, 1_000, 1_215), None);
        let record = trace.frame(1_260, 0, 0, 1_270).unwrap();
        assert_eq!(
            record,
            AnimationRecord {
                kinds: 1,
                start: 1_000,
                last: 200,
                rest: 260,
                presented: 270,
                moving: 2,
                frame: 6
            }
        );
        assert_eq!(
            alloc::format!("{record}"),
            "DOOR_P12 ANIM k=1 t=1000 l=200 e=260 p=270 f=2 n=6\n"
        );
        // A minimize interrupted by a restore is one episode with both kinds
        // and the earliest start.
        trace.frame(2_020, 0x20, 2_000, 2_030);
        trace.frame(2_080, 0x40, 2_070, 2_090);
        let both = trace.frame(2_400, 0, 0, 2_410).unwrap();
        assert_eq!(
            (both.kinds, both.start, both.rest, both.frame),
            (0x60, 2_000, 400, 9)
        );
        // The record budget is bounded; frames keep counting.
        for i in 0..u64::from(ANIMATION_RECORDS) {
            let t = 10_000 + i * 100;
            trace.frame(t, 8, t, t);
            let _ = trace.frame(t + 50, 0, 0, t + 50);
        }
        trace.frame(90_000, 8, 90_000, 90_000);
        assert_eq!(trace.frame(90_100, 0, 0, 90_100), None, "budget spent");
        assert!(trace.frames > 7 + 2 * u64::from(ANIMATION_RECORDS));
    }
}

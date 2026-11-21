//! Allocation-free checked GOP span planning. Adjacent full rows share calls;
//! partial rows never overwrite the gap between damaged columns.
use core::ops::Range;
use door_graphics_core::Rect;

pub struct Spans {
    width: usize,
    right: usize,
    bottom: usize,
    at: usize,
    left: usize,
    contiguous: bool,
}
impl Spans {
    pub fn new(width: u32, height: u32, region: Rect) -> Option<Self> {
        let output = Rect::new(0, 0, width, height).ok()?;
        if output.intersection(region) != Some(region) {
            return None;
        }
        Some(Self {
            width: width as usize,
            right: region.right() as usize,
            bottom: region.bottom() as usize,
            at: (region.y() as usize * width as usize + region.x() as usize) * 4,
            left: region.x() as usize,
            contiguous: region.x() == 0 && region.width() == width,
        })
    }
}
impl Iterator for Spans {
    type Item = Range<usize>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.at >= self.bottom * self.width * 4 {
            return None;
        }
        let row = self.at / (self.width * 4);
        let end = if self.contiguous {
            self.bottom * self.width * 4
        } else {
            (row * self.width + self.right) * 4
        };
        let range = self.at..(self.at + door_abi::graphics::MAX_COPY_BYTES).min(end);
        self.at = range.end;
        if !self.contiguous && self.at == end {
            self.at = ((row + 1) * self.width + self.left) * 4;
        }
        Some(range)
    }
}
/// Longest wait between presentation attempts while copies fail (1 s).
pub const MAX_RETRY_TICKS: u64 = 100;
/// Checked copies the test-image hook fails per trigger: three consecutive
/// presentation attempts fail before the fourth recovers.
pub const INJECTED_COPIES: u32 = 3;
/// DISPLAY-01 presentation failure policy. A failed checked copy (or one
/// failed on purpose by the test-image hook) does not end the session: the
/// attempt is abandoned, the whole output is repainted on the next attempt
/// after a bounded exponential backoff (1, 2, 4 ... `MAX_RETRY_TICKS`), and
/// input and IPC are serviced meanwhile. Buffers of the failed frame are
/// released by the engine with `TransferFailed`.
#[derive(Debug, Default)]
pub struct Presentation {
    test_hooks: bool,
    injected: u32,
    copy_failed: bool,
    failures: u32,
    total: u64,
    retry_at: u64,
    /// Night light and colour filters: an affine RGB transform applied to
    /// every copied span (`set_vision`), and its scratch span.
    vision: Option<[i32; 12]>,
    /// A night light / brightness change fading from one transform to the
    /// next: (from, to, start µs, duration µs).
    fade: Option<([i32; 12], [i32; 12], u64, u64)>,
    /// The shell clock at the last `vision_tick` (the fade position).
    now: u64,
    scratch: alloc::vec::Vec<u8>,
}
/// The identity transform (no vision word applies).
const IDENTITY: [i32; 12] = [256, 0, 0, 0, 0, 256, 0, 0, 0, 0, 256, 0];
/// Night light and brightness changes fade in rather than snapping.
pub const VISION_FADE_US: u64 = 600_000;
impl Presentation {
    /// Display > Night light and Accessibility > Color filters
    /// (`preferences::VISION`): the transform scanout applies, or none.
    pub fn set_vision(&mut self, word: u32) -> bool {
        let next = vision_matrix(word);
        let changed = next != self.vision;
        self.vision = next;
        self.fade = None;
        changed
    }
    /// `set_vision`, fading from the current transform over `duration` µs
    /// (0 jumps, as reduced motion does). Colour filters always jump, as
    /// Windows applies them at once.
    pub fn fade_vision(&mut self, word: u32, now: u64, duration: u64) -> bool {
        let from = self.current(now).unwrap_or(IDENTITY);
        let next = vision_matrix(word);
        let to = next.unwrap_or(IDENTITY);
        if from == to {
            return self.set_vision(word);
        }
        self.vision = next;
        self.fade = (duration > 0).then_some((from, to, now, duration));
        true
    }
    /// The transform at `now` (mid-fade: interpolated).
    fn current(&self, now: u64) -> Option<[i32; 12]> {
        let Some((from, to, start, duration)) = self.fade else {
            return self.vision;
        };
        let t = (now.saturating_sub(start).min(duration) * 1000 / duration.max(1)) as i32;
        let t = crate::motion::Curve::Decelerate.ease(t as u32) as i32;
        let mut m = [0; 12];
        for i in 0..12 {
            m[i] = from[i] + (to[i] - from[i]) * t / 1000;
        }
        Some(m)
    }
    /// Advance a fade to `now`: true while frames are still owed.
    pub fn vision_tick(&mut self, now: u64) -> bool {
        self.now = now;
        let Some((_, _, start, duration)) = self.fade else {
            return false;
        };
        if now >= start + duration {
            self.fade = None;
        }
        true
    }
    /// `bytes` (BGRA) through the vision transform; the bytes themselves
    /// when there is none (or no memory for the scratch span).
    pub fn transformed<'a>(&'a mut self, bytes: &'a [u8]) -> &'a [u8] {
        let Some(m) = self.current(self.now) else {
            return bytes;
        };
        self.scratch.clear();
        if self.scratch.try_reserve(bytes.len()).is_err() {
            return bytes;
        }
        for p in bytes.chunks_exact(4) {
            let (b, g, r) = (i32::from(p[0]), i32::from(p[1]), i32::from(p[2]));
            let channel = |row: usize| {
                ((m[row * 4] * r + m[row * 4 + 1] * g + m[row * 4 + 2] * b + m[row * 4 + 3]) >> 8)
                    .clamp(0, 255) as u8
            };
            self.scratch
                .extend_from_slice(&[channel(2), channel(1), channel(0), p[3]]);
        }
        &self.scratch
    }
    /// Init enables the hook only on the adversarial test image.
    pub fn enable_test_hooks(&mut self) {
        self.test_hooks = true;
    }
    pub fn test_hooks(&self) -> bool {
        self.test_hooks
    }
    /// Test image only: fail the next `copies` checked copies.
    pub fn arm(&mut self, copies: u32) -> bool {
        if self.test_hooks {
            self.injected = copies;
        }
        self.test_hooks
    }
    /// Before each checked copy: fail it on purpose (consumes one).
    pub fn inject(&mut self) -> bool {
        let inject = self.injected != 0;
        self.injected = self.injected.saturating_sub(1);
        inject
    }
    pub fn copy_failed(&mut self) {
        self.copy_failed = true;
    }
    /// Whether the attempt that just ended had a failed checked copy (as
    /// opposed to an internal error, which is still fatal).
    pub fn take_copy_failure(&mut self) -> bool {
        core::mem::take(&mut self.copy_failed)
    }
    /// A presentation attempt failed at `now`: consecutive failures and the
    /// ticks until the next attempt.
    pub fn attempt_failed(&mut self, now: u64) -> (u32, u64) {
        self.failures = self.failures.saturating_add(1);
        self.total = self.total.saturating_add(1);
        let delay = (1u64 << (self.failures - 1).min(7)).min(MAX_RETRY_TICKS);
        self.retry_at = now.saturating_add(delay);
        (self.failures, delay)
    }
    /// Presenting may be attempted (not backing off after a failure).
    pub fn ready(&self, now: u64) -> bool {
        self.failures == 0 || now >= self.retry_at
    }
    pub fn failing(&self) -> bool {
        self.failures != 0
    }
    /// A scene presentation succeeded: the consecutive failures it ends.
    pub fn recovered(&mut self) -> Option<u32> {
        (self.failures != 0).then(|| core::mem::take(&mut self.failures))
    }
    pub fn total_failures(&self) -> u64 {
        self.total
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presentation_failures_back_off_and_recover_and_the_hook_is_test_only() {
        extern crate alloc;
        let mut p = Presentation::default();
        assert!(
            !p.arm(INJECTED_COPIES) && !p.inject(),
            "no hook outside tests"
        );
        p.enable_test_hooks();
        assert!(p.arm(INJECTED_COPIES));
        let mut now = 50;
        let mut delays = alloc::vec::Vec::new();
        while p.inject() {
            p.copy_failed();
            assert!(p.take_copy_failure() && !p.take_copy_failure());
            let (n, delay) = p.attempt_failed(now);
            delays.push((n, delay));
            assert!(!p.ready(now + delay - 1) && p.ready(now + delay));
            now += delay;
        }
        assert_eq!(delays, [(1, 1), (2, 2), (3, 4)]);
        assert!(p.failing());
        assert_eq!(p.recovered(), Some(3));
        assert!(!p.failing() && p.ready(0) && p.recovered().is_none());
        assert_eq!(p.total_failures(), 3);
        // A persistent failure settles at one attempt per MAX_RETRY_TICKS.
        for _ in 0..20 {
            p.attempt_failed(now);
        }
        assert_eq!(p.attempt_failed(now).1, MAX_RETRY_TICKS);
    }
    #[test]
    fn spans_cover_exact_damage_once_with_bounded_calls() {
        for (width, height) in [(1, 3), (1280, 720), (3840, 2160)] {
            for r in [
                Rect::new(0, 0, width, height).unwrap(),
                Rect::new(
                    (width / 3) as i32,
                    1,
                    (width - width / 3).min(71),
                    height - 1,
                )
                .unwrap(),
            ] {
                let mut previous = 0;
                let mut count = 0;
                for range in Spans::new(width, height, r).unwrap() {
                    assert!(range.start >= previous && range.end > range.start);
                    assert!(range.len() <= door_abi::graphics::MAX_COPY_BYTES);
                    assert!(range.start.is_multiple_of(4) && range.end.is_multiple_of(4));
                    for pixel in (range.start / 4)..(range.end / 4) {
                        assert!(r.contains(
                            (pixel % width as usize) as i32,
                            (pixel / width as usize) as i32
                        ));
                    }
                    count += range.len() / 4;
                    previous = range.end;
                }
                assert_eq!(count, r.width() as usize * r.height() as usize);
            }
        }
        assert!(Spans::new(100, 80, Rect::new(-1, 0, 20, 20).unwrap()).is_none());
        let r = Rect::new(0, 0, 1280, 720).unwrap();
        assert_eq!(Spans::new(1280, 720, r).unwrap().count(), 225);
    }
    #[test]
    fn native3x_caption_matches_old_copies_with_fewer_calls_and_no_gap_writes() {
        extern crate alloc;
        use alloc::vec;
        let (width, height) = (3840, 128);
        let source: alloc::vec::Vec<_> = (0..width * height * 4).map(|v| (v % 251) as u8).collect();
        for region in [
            Rect::new(168, 8, 2226, 96).unwrap(),
            Rect::new(0, 8, 3840, 96).unwrap(),
        ] {
            let mut old = vec![253; source.len()];
            let mut next = old.clone();
            let mut old_calls = 0;
            for y in region.y() as usize..region.bottom() as usize {
                let first = (y * width + region.x() as usize) * 4;
                let end = first + region.width() as usize * 4;
                for at in (first..end).step_by(4096) {
                    let right = (at + 4096).min(end);
                    old[at..right].copy_from_slice(&source[at..right]);
                    old_calls += 1;
                }
            }
            let spans: alloc::vec::Vec<_> = Spans::new(width as u32, height as u32, region)
                .unwrap()
                .collect();
            for span in &spans {
                next[span.clone()].copy_from_slice(&source[span.clone()]);
            }
            assert_eq!(old, next);
            assert!(spans.len() * 2 <= old_calls);
            if region.width() == 2226 {
                assert_eq!((old_calls, spans.len()), (288, 96));
            } else {
                assert_eq!((old_calls, spans.len()), (384, 90));
            }
        }
    }
}
/// The affine RGB transform (rows R, G, B: three weights and an offset, in
/// 1/256) of a `preferences::VISION` word, None when nothing applies.
pub fn vision_matrix(word: u32) -> Option<[i32; 12]> {
    use door_service_protocol::preferences::vision as v;
    let mut m = IDENTITY;
    let mut any = false;
    if word & v::FILTER != 0 {
        any = true;
        // Luminance weights (BT.709) and the colour-vision matrices.
        const GRAY: [i32; 3] = [54, 183, 19];
        m = match v::kind(word) {
            0 => [
                GRAY[0], GRAY[1], GRAY[2], 0, GRAY[0], GRAY[1], GRAY[2], 0, GRAY[0], GRAY[1],
                GRAY[2], 0,
            ],
            1 => [-256, 0, 0, 65280, 0, -256, 0, 65280, 0, 0, -256, 65280],
            2 => [
                -GRAY[0], -GRAY[1], -GRAY[2], 65280, -GRAY[0], -GRAY[1], -GRAY[2], 65280, -GRAY[0],
                -GRAY[1], -GRAY[2], 65280,
            ],
            3 => [160, 96, 0, 0, 179, 77, 0, 0, 0, 77, 179, 0],
            4 => [145, 111, 0, 0, 143, 113, 0, 0, 0, 62, 194, 0],
            _ => [243, 13, 0, 0, 0, 111, 145, 0, 0, 122, 134, 0],
        };
    }
    if word & v::NIGHT_LIGHT != 0 && v::strength(word) > 0 {
        any = true;
        // Warmer: green down to 69% and blue down to 33% at full strength.
        let k = v::strength(word) as i32;
        let gains = [256, 256 - 79 * k / 100, 256 - 171 * k / 100];
        for (row, gain) in gains.iter().enumerate() {
            for col in 0..4 {
                m[row * 4 + col] = m[row * 4 + col] * gain / 256;
            }
        }
    }
    if v::brightness(word) < 100 {
        any = true;
        // Software brightness: 25% of full output at 0, never black.
        let gain = 64 + 192 * v::brightness(word) as i32 / 100;
        for value in m.iter_mut() {
            *value = *value * gain / 256;
        }
    }
    any.then_some(m)
}
#[cfg(test)]
mod vision_tests {
    use super::*;
    use door_service_protocol::preferences::vision as v;
    #[test]
    fn vision_words_transform_spans_and_none_leaves_them_alone() {
        assert_eq!(vision_matrix(v::DEFAULT), None);
        let mut p = Presentation::default();
        let px = [10u8, 120, 240, 255]; // B, G, R, A
        assert!(!p.set_vision(v::DEFAULT));
        assert_eq!(p.transformed(&px), &px);
        // Inverted.
        assert!(p.set_vision(v::FILTER | 1 << v::KIND_SHIFT));
        assert_eq!(p.transformed(&px), &[245, 135, 15, 255]);
        // Grayscale: every channel the same luminance.
        p.set_vision(v::FILTER);
        let g = p.transformed(&px).to_vec();
        assert!(g[0] == g[1] && g[1] == g[2]);
        // Night light keeps red and cuts blue more than green.
        p.set_vision(v::NIGHT_LIGHT | 100 << v::STRENGTH_SHIFT);
        let n = p.transformed(&[200, 200, 200, 255]).to_vec();
        assert_eq!(n[2], 200);
        assert!(n[0] < n[1] && n[1] < 200);
    }
}

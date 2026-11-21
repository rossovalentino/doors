//! QEMU MC146818 register decoding, independently testable without port access.
use door_abi::{Status, clock::WallClock};
/// Stable [second,minute,hour,day,month,year,century,registerB] sample.
pub fn decode(raw: [u8; 8]) -> Option<WallClock> {
    let binary = raw[7] & 4 != 0;
    let value = |byte: u8| -> Option<u8> {
        if binary {
            Some(byte)
        } else if byte & 15 <= 9 && byte >> 4 <= 9 {
            Some((byte >> 4) * 10 + (byte & 15))
        } else {
            None
        }
    };
    let mut hour = value(raw[2] & 0x7f)?;
    if raw[7] & 2 == 0 {
        if !(1..=12).contains(&hour) {
            return None;
        }
        hour = hour % 12 + if raw[2] & 0x80 != 0 { 12 } else { 0 };
    } else if raw[2] & 0x80 != 0 {
        return None;
    }
    let time = WallClock {
        year: u16::from(value(raw[6])?) * 100 + u16::from(value(raw[5])?),
        month: value(raw[4])?,
        day: value(raw[3])?,
        hour,
        minute: value(raw[1])?,
        second: value(raw[0])?,
    };
    time.valid().then_some(time)
}
/// FILETIME units per 100 Hz scheduler tick.
pub const TICK_FILETIME: u64 = 100_000;
/// Re-read the RTC at most once a minute of ticks.
pub const RESAMPLE_TICKS: u64 = 60 * 100;
/// Larger RTC/projection disagreements re-anchor on the RTC (host or
/// firmware adjustment) instead of keeping the projection monotonic.
const STEP_LIMIT: u64 = 2 * door_abi::clock::FILETIME_PER_SECOND;
/// Wall time anchor: a validated RTC reading (FILETIME, whole seconds) and
/// the scheduler tick at which it was taken. Wall time is the anchor plus
/// the monotonic tick offset, so it advances between RTC seconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Anchor {
    pub filetime: u64,
    pub tick: u64,
}
impl Anchor {
    pub fn project(self, tick: u64) -> Option<u64> {
        tick.checked_sub(self.tick)?
            .checked_mul(TICK_FILETIME)?
            .checked_add(self.filetime)
    }
    pub fn stale(self, tick: u64) -> bool {
        tick.checked_sub(self.tick)
            .is_none_or(|age| age >= RESAMPLE_TICKS)
    }
    /// Re-anchor on a fresh RTC sample. Within the RTC's one-second
    /// granularity the later of projection and sample wins, so successive
    /// readings never step back; a larger disagreement follows the RTC.
    pub fn rebase(self, sample: u64, tick: u64) -> Self {
        let filetime = match self.project(tick) {
            Some(projected) if projected.abs_diff(sample) < STEP_LIMIT => projected.max(sample),
            _ => sample,
        };
        Self { filetime, tick }
    }
}
/// Ticks between RTC sampling attempts after an invalid or unsupported
/// reading (1 s). An update in progress is retried on the next tick.
pub const RETRY_TICKS: u64 = 100;
/// `WALL_TIME` state: the anchor plus a retry gate. Sampling runs with
/// interrupts off, so a missing, invalid or mid-update RTC costs at most one
/// bounded sample per tick (update in progress) or per `RETRY_TICKS`
/// (invalid), however often callers ask; between attempts the last outcome
/// is reported without port I/O.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Wall {
    anchor: Option<Anchor>,
    /// First tick at which a new sample may be taken.
    retry: u64,
    unsupported: bool,
}
impl Wall {
    pub const fn new() -> Self {
        Self {
            anchor: None,
            retry: 0,
            unsupported: false,
        }
    }
    /// Wall time at `tick`. `sample` reads the RTC as FILETIME: WouldBlock
    /// while an update is in progress, any other error when the clock is
    /// invalid or unsupported (which drops the anchor; no time is invented).
    pub fn read(
        &mut self,
        tick: u64,
        sample: impl FnOnce() -> Result<u64, Status>,
    ) -> Result<u64, Status> {
        let project = |anchor: Anchor| anchor.project(tick).ok_or(Status::Unsupported);
        if let Some(anchor) = self.anchor
            && !anchor.stale(tick)
        {
            return project(anchor);
        }
        if tick < self.retry {
            return match self.anchor {
                Some(anchor) => project(anchor),
                None if self.unsupported => Err(Status::Unsupported),
                None => Err(Status::WouldBlock),
            };
        }
        match sample() {
            Ok(sample) => {
                let anchor = self.anchor.map_or(
                    Anchor {
                        filetime: sample,
                        tick,
                    },
                    |old| old.rebase(sample, tick),
                );
                *self = Self {
                    anchor: Some(anchor),
                    retry: 0,
                    unsupported: false,
                };
                project(anchor)
            }
            Err(Status::WouldBlock) => {
                self.retry = tick.saturating_add(1);
                self.unsupported = false;
                match self.anchor {
                    Some(anchor) => project(anchor),
                    None => Err(Status::WouldBlock),
                }
            }
            Err(_) => {
                *self = Self {
                    anchor: None,
                    retry: tick.saturating_add(RETRY_TICKS),
                    unsupported: true,
                };
                Err(Status::Unsupported)
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wall_reads_rate_limit_rtc_sampling_and_never_invent_time() {
        let second = door_abi::clock::FILETIME_PER_SECOND;
        let mut wall = Wall::new();
        let mut samples = 0;
        // No valid RTC: one sample, then none until RETRY_TICKS have passed.
        for tick in 0..RETRY_TICKS {
            assert_eq!(
                wall.read(tick, || {
                    samples += 1;
                    Err(Status::Unsupported)
                }),
                Err(Status::Unsupported)
            );
        }
        assert_eq!(samples, 1);
        // An update in progress is retried on the next tick, not spun on.
        let busy = |samples: &mut u32| {
            *samples += 1;
            Err(Status::WouldBlock)
        };
        assert_eq!(
            wall.read(RETRY_TICKS, || busy(&mut samples)),
            Err(Status::WouldBlock)
        );
        assert_eq!(
            wall.read(RETRY_TICKS, || busy(&mut samples)),
            Err(Status::WouldBlock)
        );
        assert_eq!(samples, 2);
        let at = RETRY_TICKS + 1;
        assert_eq!(wall.read(at, || Ok(7 * second)), Ok(7 * second));
        // Anchored: projected without sampling until stale; a busy resample
        // keeps projecting, an invalid one drops the anchor.
        assert_eq!(wall.read(at + 100, || unreachable!()), Ok(8 * second));
        let stale = at + RESAMPLE_TICKS;
        assert_eq!(
            wall.read(stale, || busy(&mut samples)),
            Ok(7 * second + RESAMPLE_TICKS * TICK_FILETIME)
        );
        assert_eq!(samples, 3);
        assert!(wall.read(stale, || unreachable!()).is_ok());
        assert_eq!(
            wall.read(stale + 1, || Err(Status::IoFailure)),
            Err(Status::Unsupported)
        );
        assert_eq!(
            wall.read(stale + 2, || unreachable!()),
            Err(Status::Unsupported)
        );
    }
    #[test]
    fn wall_anchor_projects_ticks_and_rebases_monotonically() {
        let second = door_abi::clock::FILETIME_PER_SECOND;
        let a = Anchor {
            filetime: 100_000 * second,
            tick: 50,
        };
        assert_eq!(a.project(50), Some(100_000 * second));
        assert_eq!(a.project(150), Some(100_001 * second));
        assert_eq!(a.project(49), None);
        assert!(!a.stale(50 + RESAMPLE_TICKS - 1) && a.stale(50 + RESAMPLE_TICKS) && a.stale(0));
        // The RTC second ticked 0.4 s after the projection's floor: keep the later.
        let tick = 50 + RESAMPLE_TICKS;
        let projected = a.project(tick).unwrap();
        assert_eq!(a.rebase(projected - second / 2, tick).filetime, projected);
        assert_eq!(
            a.rebase(projected + second / 2, tick).filetime,
            projected + second / 2
        );
        // A host adjustment (an hour back) follows the RTC.
        assert_eq!(
            a.rebase(projected - 3600 * second, tick).filetime,
            projected - 3600 * second
        );
        assert_eq!(
            a.rebase(5, 10),
            Anchor {
                filetime: 5,
                tick: 10
            }
        );
    }
    #[test]
    fn bcd_binary_noon_midnight_and_invalid_samples() {
        let raw = [0x59, 0x58, 0x23, 0x19, 0x10, 0x26, 0x20, 2];
        let time = decode(raw).unwrap();
        assert_eq!((time.year, time.hour, time.minute), (2026, 23, 58));
        assert_eq!(decode([59, 58, 23, 19, 10, 26, 20, 6]), Some(time));
        assert_eq!(decode([0, 0, 0x12, 1, 1, 0x26, 0x20, 0]).unwrap().hour, 0);
        assert_eq!(decode([0, 0, 0x92, 1, 1, 0x26, 0x20, 0]).unwrap().hour, 12);
        for (index, value) in [(0, 0x6a), (3, 0), (4, 0x13), (2, 0x24)] {
            let mut bad = raw;
            bad[index] = value;
            assert!(decode(bad).is_none());
        }
    }
}

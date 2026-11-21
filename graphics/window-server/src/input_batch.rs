//! Bounded device-input batching. Consecutive relative-motion packets with
//! identical button state merge (motion and wheel deltas add, the newest
//! timestamp wins), so hover, hit testing and drag work run once per burst
//! instead of once per PS/2 packet. A button change, key or reset is never
//! merged and keeps its exact position in the sequence; later motion never
//! merges *into* a button change either, because the consumer applies an
//! event's motion before its button transition: a release followed by a
//! quick move must still be released where it happened (a double click
//! must not see the second release after the pointer left the icon).
use door_abi::graphics as d;

/// Distinct (unmerged) events delivered per batch.
pub const CAPACITY: usize = 32;
/// Raw device events read per batch; merged packets cost no slot.
pub const READS: usize = 4 * CAPACITY;

pub struct Batch {
    events: [d::Input; CAPACITY],
    len: usize,
    /// Button state after the pushed events (None: unknown, e.g. after a
    /// reset or before the first packet of a `new()` batch).
    buttons: Option<u32>,
    /// The newest event is motion with no button change: later motion with
    /// the same buttons may merge into it.
    open: bool,
}
impl Default for Batch {
    fn default() -> Self {
        Self::new()
    }
}
impl Batch {
    /// A batch whose starting button state is unknown: its first motion
    /// packet is treated as a possible button change.
    pub const fn new() -> Self {
        Self::start(None)
    }
    /// A batch continuing from the consumer's current button state.
    pub const fn with_buttons(buttons: u32) -> Self {
        Self::start(Some(buttons))
    }
    const fn start(buttons: Option<u32>) -> Self {
        Self {
            events: [d::Input {
                kind: 0,
                flags: 0,
                x: 0,
                y: 0,
                code: 0,
                text: 0,
                timestamp: 0,
            }; CAPACITY],
            len: 0,
            buttons,
            open: false,
        }
    }
    pub fn is_full(&self) -> bool {
        self.len == CAPACITY
    }
    /// Append one device event, merging it into a preceding motion packet
    /// with the same buttons (never into a button change). Returns false
    /// (event not taken) when full.
    pub fn push(&mut self, event: d::Input) -> bool {
        if self.open
            && event.kind == d::MOTION
            && let Some(last) = self.len.checked_sub(1).map(|i| &mut self.events[i])
            && last.kind == d::MOTION
            && last.flags == event.flags
        {
            last.x = last.x.saturating_add(event.x);
            last.y = last.y.saturating_add(event.y);
            last.code = (last.wheel() + event.wheel()).clamp(-d::MAX_WHEEL, d::MAX_WHEEL) as u32;
            last.timestamp = event.timestamp;
            return true;
        }
        if self.is_full() {
            return false;
        }
        self.events[self.len] = event;
        self.len += 1;
        match event.kind {
            d::MOTION => {
                self.open = self.buttons == Some(event.flags);
                self.buttons = Some(event.flags);
            }
            d::RESET => {
                self.open = false;
                self.buttons = None;
            }
            _ => self.open = false,
        }
        true
    }
    pub fn events(&self) -> &[d::Input] {
        &self.events[..self.len]
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn motion(x: i32, y: i32, flags: u32, wheel: i32, timestamp: u64) -> d::Input {
        d::Input {
            kind: d::MOTION,
            x,
            y,
            flags,
            code: wheel as u32,
            timestamp,
            ..d::Input::default()
        }
    }
    #[test]
    fn motion_bursts_merge_but_buttons_keys_and_resets_keep_their_order() {
        let mut batch = Batch::with_buttons(0);
        let key = d::Input {
            kind: d::KEY,
            code: 30,
            flags: d::DOWN,
            timestamp: 5,
            ..d::Input::default()
        };
        for event in [
            motion(1, 2, 0, 0, 1),
            motion(3, -1, 0, 1, 2),
            motion(-2, 0, 0, 2, 3),
            // Press: a new button state is never merged into the hover burst,
            // and the drag motion after it never merges into the press.
            motion(0, 0, 1, 0, 4),
            motion(5, 5, 1, 0, 5),
            key,
            motion(1, 1, 1, 0, 6),
            // Release in its own packet.
            motion(0, 0, 0, -1, 7),
            d::Input {
                kind: d::RESET,
                ..d::Input::default()
            },
            motion(4, 4, 0, 0, 8),
        ] {
            assert!(batch.push(event));
        }
        let events = batch.events();
        assert_eq!(events.len(), 8);
        assert_eq!(
            (
                events[0].x,
                events[0].y,
                events[0].wheel(),
                events[0].timestamp
            ),
            (2, 1, 3, 3)
        );
        assert_eq!(
            (events[1].flags, events[1].x, events[1].timestamp),
            (1, 0, 4)
        );
        assert_eq!((events[2].flags, events[2].x, events[2].y), (1, 5, 5));
        assert_eq!(events[3], key);
        assert_eq!((events[4].flags, events[4].x), (1, 1));
        assert_eq!((events[5].flags, events[5].wheel()), (0, -1));
        assert_eq!(events[6].kind, d::RESET);
        assert_eq!((events[7].x, events[7].y), (4, 4));
    }
    #[test]
    fn a_release_keeps_its_position_when_motion_follows_in_the_same_batch() {
        // Double click on a desktop icon, then the pointer moves away at once.
        let mut batch = Batch::with_buttons(0);
        for event in [
            motion(0, 0, 1, 0, 10),
            motion(0, 0, 0, 0, 15),
            motion(0, 0, 1, 0, 20),
            motion(0, 0, 0, 0, 25),
            motion(-12, 0, 0, 0, 26),
            motion(-3, 0, 0, 0, 27),
        ] {
            assert!(batch.push(event));
        }
        let e = batch.events();
        assert_eq!(e.len(), 5);
        assert_eq!(
            (e[3].flags, e[3].x, e[3].timestamp),
            (0, 0, 25),
            "release at its own position"
        );
        assert_eq!(
            (e[4].flags, e[4].x, e[4].timestamp),
            (0, -15, 27),
            "later motion merges after it"
        );
        // Unknown starting buttons: the first packet may be a change.
        let mut unknown = Batch::new();
        assert!(unknown.push(motion(0, 0, 0, 0, 1)));
        assert!(unknown.push(motion(7, 0, 0, 0, 2)));
        assert!(unknown.push(motion(1, 0, 0, 0, 3)));
        let e = unknown.events();
        assert_eq!((e.len(), e[0].x, e[1].x), (2, 0, 8));
    }
    #[test]
    fn capacity_bounds_distinct_events_and_wheel_saturates() {
        let mut batch = Batch::with_buttons(1);
        for i in 0..CAPACITY - 1 {
            assert!(batch.push(motion(0, 0, (i % 2) as u32, 0, i as u64)));
        }
        let last = ((CAPACITY - 2) % 2) as u32;
        // Same buttons after a change: a new (mergeable) slot fills the batch.
        assert!(batch.push(motion(0, 0, last, 0, 50)));
        assert!(batch.is_full());
        assert!(batch.push(motion(1, 0, last, 100, 99)));
        assert!(batch.push(motion(1, 0, last, 100, 100)));
        assert!(!batch.push(motion(0, 0, 1 - last, 0, 101)));
        let tail = batch.events()[CAPACITY - 1];
        assert_eq!(
            (tail.x, tail.wheel(), tail.timestamp),
            (2, d::MAX_WHEEL, 100)
        );
    }
}

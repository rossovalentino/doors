//! The receive counter window (whitepaper 5.4.6, RFC 6479 style): a ring of
//! 128 64-bit blocks, so 8128 counters behind the newest one are still
//! accepted once each.
const BLOCK_BITS: u64 = 64;
const RING_BLOCKS: u64 = 128;
pub const WINDOW: u64 = (RING_BLOCKS - 1) * BLOCK_BITS;

#[derive(Clone)]
pub struct Window {
    last: u64,
    ring: [u64; RING_BLOCKS as usize],
}
impl Default for Window {
    fn default() -> Self {
        Self {
            last: 0,
            ring: [0; RING_BLOCKS as usize],
        }
    }
}
impl Window {
    /// Whether `counter` would be accepted now (before decrypting).
    pub fn fresh(&self, counter: u64, limit: u64) -> bool {
        if counter >= limit {
            return false;
        }
        if counter > self.last {
            return true;
        }
        if self.last - counter > WINDOW {
            return false;
        }
        let block = ((counter / BLOCK_BITS) % RING_BLOCKS) as usize;
        self.ring[block] & (1 << (counter % BLOCK_BITS)) == 0
    }
    /// Record an authenticated `counter`; false for a replay.
    pub fn accept(&mut self, counter: u64, limit: u64) -> bool {
        if !self.fresh(counter, limit) {
            return false;
        }
        let index = counter / BLOCK_BITS;
        if counter > self.last {
            let current = self.last / BLOCK_BITS;
            let diff = (index - current).min(RING_BLOCKS);
            for i in current + 1..=current + diff {
                self.ring[(i % RING_BLOCKS) as usize] = 0;
            }
            self.last = counter;
        }
        self.ring[(index % RING_BLOCKS) as usize] |= 1 << (counter % BLOCK_BITS);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const LIMIT: u64 = u64::MAX - (1 << 13);

    #[test]
    fn accepts_each_counter_once_within_the_window() {
        let mut w = Window::default();
        for (counter, expected) in [
            (0, true),
            (1, true),
            (1, false),
            (9, true),
            (8, true),
            (7, true),
            (7, false),
            (WINDOW + 1, true),
            (WINDOW, true),
            (WINDOW, false),
            (2, true),
            (1, false),
            (WINDOW + 20, true),
            (20, true),
            (19, false),
            (WINDOW * 4, true),
            (WINDOW * 3, true),
            (WINDOW * 3 - 1, false),
            (WINDOW * 3, false),
        ] {
            assert_eq!(w.accept(counter, LIMIT), expected, "counter {counter}");
        }
    }

    #[test]
    fn a_jump_past_the_ring_clears_it() {
        let mut w = Window::default();
        assert!(w.accept(5, LIMIT));
        assert!(w.accept(1_000_000, LIMIT));
        // Old bits do not alias into the new window.
        assert!(w.accept(1_000_000 - 64 * 128 + 5 + 64, LIMIT));
        assert!(!w.accept(5, LIMIT));
    }

    #[test]
    fn the_limit_is_exclusive() {
        let mut w = Window::default();
        assert!(!w.accept(LIMIT, LIMIT));
        assert!(w.accept(LIMIT - 1, LIMIT));
        assert!(!w.accept(LIMIT - 1, LIMIT));
        assert!(w.fresh(LIMIT - 2, LIMIT));
    }
}

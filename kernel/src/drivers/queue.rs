//! Bounded descriptor ownership, independent of register access and block class.
use super::{Error, Result};

pub const SLOTS: usize = 4;
pub const QUEUE_SIZE: u16 = 16;

pub fn completion_count(previous: u16, current: u16, outstanding: usize) -> Result<usize> {
    let count = current.wrapping_sub(previous) as usize;
    if count > outstanding || count > SLOTS {
        Err(Error::Malformed)
    } else {
        Ok(count)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Token {
    pub slot: usize,
    generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Free,
    Pending,
    Published,
    Complete(Result<()>),
}

#[derive(Clone, Copy)]
struct Slot {
    generation: u64,
    state: State,
    expected: u32,
}
const EMPTY: Slot = Slot {
    generation: 0,
    state: State::Free,
    expected: 0,
};

pub struct Queue {
    slots: [Slot; SLOTS],
    generation: u64,
    accepting: bool,
}
impl Queue {
    pub const fn new() -> Self {
        Self {
            slots: [EMPTY; SLOTS],
            generation: 0,
            accepting: true,
        }
    }
    pub fn reserve(&mut self, expected: u32) -> Result<Token> {
        if !self.accepting {
            return Err(Error::Removed);
        }
        let slot = self
            .slots
            .iter()
            .position(|s| s.state == State::Free)
            .ok_or(Error::QueueFull)?;
        self.generation = self.generation.checked_add(1).ok_or(Error::Stale)?;
        self.slots[slot] = Slot {
            generation: self.generation,
            state: State::Pending,
            expected,
        };
        Ok(Token {
            slot,
            generation: self.generation,
        })
    }
    fn get(&mut self, token: Token) -> Result<&mut Slot> {
        let slot = self.slots.get_mut(token.slot).ok_or(Error::Stale)?;
        if slot.generation != token.generation || slot.state == State::Free {
            return Err(Error::Stale);
        }
        Ok(slot)
    }
    pub fn publish(&mut self, token: Token) -> Result<()> {
        let slot = self.get(token)?;
        if slot.state != State::Pending {
            return Err(Error::InFlight);
        }
        slot.state = State::Published;
        Ok(())
    }
    pub fn cancel(&mut self, token: Token) -> Result<()> {
        let slot = self.get(token)?;
        if slot.state != State::Pending {
            return Err(Error::InFlight);
        }
        slot.state = State::Complete(Err(Error::Canceled));
        Ok(())
    }
    pub fn complete(&mut self, head: u32, bytes: u32, status: u8) -> Result<()> {
        if !head.is_multiple_of(3) || head / 3 >= SLOTS as u32 {
            return Err(Error::Malformed);
        }
        let slot = &mut self.slots[head as usize / 3];
        if slot.state != State::Published || bytes > slot.expected || bytes == 0 || status > 2 {
            return Err(Error::Malformed);
        }
        let result = match status {
            0 if bytes != slot.expected => Err(Error::ShortIo),
            0 => Ok(()),
            1 => Err(Error::Io),
            _ => Err(Error::Unsupported),
        };
        slot.state = State::Complete(result);
        Ok(())
    }
    pub fn result(&mut self, token: Token) -> Result<Option<Result<()>>> {
        Ok(match self.get(token)?.state {
            State::Complete(result) => Some(result),
            _ => None,
        })
    }
    pub fn reap(&mut self, token: Token) -> Result<()> {
        let slot = self.get(token)?;
        let State::Complete(result) = slot.state else {
            return Err(Error::InFlight);
        };
        slot.state = State::Free;
        result
    }
    pub fn published(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| s.state == State::Published)
            .count()
    }
    pub fn stop(&mut self) {
        self.accepting = false;
    }
    /// Called only after the transport has positively acknowledged device reset.
    pub fn reset_acknowledged(&mut self) {
        for slot in &mut self.slots {
            if slot.state != State::Free {
                slot.state = State::Complete(Err(Error::Reset));
            }
        }
    }
    pub fn restart(&mut self) {
        self.accepting = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_cancel_reuse_and_stale_tokens() {
        let mut q = Queue::new();
        let first = q.reserve(513).unwrap();
        for _ in 1..SLOTS {
            q.reserve(1).unwrap();
        }
        assert_eq!(q.reserve(1), Err(Error::QueueFull));
        q.cancel(first).unwrap();
        assert_eq!(q.reap(first), Err(Error::Canceled));
        let next = q.reserve(1).unwrap();
        assert_eq!(q.publish(first), Err(Error::Stale));
        q.publish(next).unwrap();
        assert_eq!(q.cancel(next), Err(Error::InFlight));
    }
    #[test]
    fn completion_validates_head_length_status_and_duplicate() {
        let mut q = Queue::new();
        let t = q.reserve(513).unwrap();
        q.publish(t).unwrap();
        for head in [1, 12, u32::MAX] {
            assert_eq!(q.complete(head, 513, 0), Err(Error::Malformed));
        }
        assert_eq!(q.complete(0, 514, 0), Err(Error::Malformed));
        assert_eq!(q.complete(0, 513, 3), Err(Error::Malformed));
        q.complete(0, 512, 0).unwrap();
        assert_eq!(q.complete(0, 513, 0), Err(Error::Malformed));
        assert_eq!(q.reap(t), Err(Error::ShortIo));
    }
    #[test]
    fn reset_cannot_reuse_outstanding_until_acknowledged() {
        let mut q = Queue::new();
        let t = q.reserve(1).unwrap();
        q.publish(t).unwrap();
        q.stop();
        assert_eq!(q.reserve(1), Err(Error::Removed));
        assert_eq!(q.reap(t), Err(Error::InFlight));
        q.reset_acknowledged();
        assert_eq!(q.reap(t), Err(Error::Reset));
        assert_eq!(q.complete(0, 1, 0), Err(Error::Malformed));
        q.restart();
        let next = q.reserve(1).unwrap();
        assert_ne!(t, next);
        assert_eq!(q.result(t), Err(Error::Stale));
    }
    #[test]
    fn used_index_wrap_and_overproduction_are_distinct() {
        assert_eq!(completion_count(65534, 1, 3), Ok(3));
        assert_eq!(completion_count(65534, 1, 2), Err(Error::Malformed));
        assert_eq!(completion_count(0, 65535, 4), Err(Error::Malformed));
        assert_eq!(completion_count(20, 25, 5), Err(Error::Malformed));
    }
    #[test]
    fn errors_preserve_descriptor_ownership_until_reaped() {
        let mut q = Queue::new();
        let a = q.reserve(513).unwrap();
        q.publish(a).unwrap();
        q.complete(0, 1, 1).unwrap();
        assert_eq!(q.result(a), Ok(Some(Err(Error::Io))));
        assert_eq!(q.cancel(a), Err(Error::InFlight));
        assert_eq!(q.reap(a), Err(Error::Io));
        let b = q.reserve(1).unwrap();
        q.publish(b).unwrap();
        q.complete(0, 1, 2).unwrap();
        assert_eq!(q.reap(b), Err(Error::Unsupported));
        assert_eq!(q.reap(b), Err(Error::Stale));
    }
    #[test]
    fn failed_reset_retains_every_published_slot_and_blocks_new_work() {
        let mut q = Queue::new();
        let tokens = core::array::from_fn::<_, SLOTS, _>(|_| {
            let token = q.reserve(513).unwrap();
            q.publish(token).unwrap();
            token
        });
        q.stop();
        // A timeout/removal produces no acknowledgement. Repeated cancellation,
        // reap and reservation attempts must not surrender even one DMA slot.
        for _ in 0..8 {
            for token in tokens {
                assert_eq!(q.cancel(token), Err(Error::InFlight));
                assert_eq!(q.reap(token), Err(Error::InFlight));
            }
            assert_eq!(q.reserve(1), Err(Error::Removed));
            assert_eq!(q.published(), SLOTS);
        }
        q.reset_acknowledged();
        for token in tokens {
            assert_eq!(q.reap(token), Err(Error::Reset));
        }
        assert_eq!(q.published(), 0);
    }
}

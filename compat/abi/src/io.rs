//! Bounded async request identity. Cancellation and completion are serialized by
//! the adapter's owner; terminal publication can occur exactly once per epoch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    Idle,
    Queued,
    Running,
    Terminal,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Request {
    pub generation: u64,
    pub state: State,
    pub cancelled: bool,
}
impl Default for Request {
    fn default() -> Self {
        Self::new()
    }
}
impl Request {
    pub const fn new() -> Self {
        Self {
            generation: 0,
            state: State::Idle,
            cancelled: false,
        }
    }
    pub fn submit(&mut self) -> Option<u64> {
        if self.state != State::Idle {
            return None;
        }
        self.generation = self.generation.checked_add(1)?;
        self.state = State::Queued;
        self.cancelled = false;
        Some(self.generation)
    }
    pub fn start(&mut self, generation: u64) -> bool {
        if generation != self.generation || self.state != State::Queued {
            return false;
        }
        self.state = State::Running;
        true
    }
    pub fn cancel(&mut self) -> bool {
        if !matches!(self.state, State::Queued | State::Running) {
            return false;
        }
        self.cancelled = true;
        true
    }
    pub fn finish(&mut self, generation: u64) -> bool {
        if generation != self.generation || !matches!(self.state, State::Queued | State::Running) {
            return false;
        }
        self.state = State::Terminal;
        true
    }
    /// Only after worker retirement; terminal state alone does not release code,
    /// file, event or owned transfer-buffer lifetime.
    pub fn retire(&mut self) -> bool {
        if self.state != State::Terminal {
            return false;
        }
        self.state = State::Idle;
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_cannot_complete_twice_or_recycle_worker_identity() {
        let mut request = Request::new();
        let first = request.submit().unwrap();
        assert_eq!(request.submit(), None);
        assert!(request.cancel());
        assert!(request.start(first));
        assert!(request.cancelled);
        assert!(request.finish(first));
        assert!(!request.finish(first));
        assert!(!request.cancel());
        assert!(request.retire());
        let second = request.submit().unwrap();
        assert_ne!(first, second);
        assert!(!request.start(first));
        assert!(!request.finish(first));
        assert!(!request.cancelled);
        assert!(!request.retire());
        assert!(request.start(second));
        assert!(request.finish(second));
    }
    #[test]
    fn exhausted_generation_fails_without_state_change() {
        let mut request = Request {
            generation: u64::MAX,
            state: State::Idle,
            cancelled: false,
        };
        assert_eq!(request.submit(), None);
        assert_eq!(request.state, State::Idle);
        assert!(!request.start(0));
    }
}

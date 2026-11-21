//! User-mode APC FIFO. Ownership/target authority belongs to the adapter's
//! registered native thread; callback execution occurs outside its metadata lock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Callback {
    pub function: u64,
    pub argument: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Queue {
    entries: [Option<Callback>; 8],
}
impl Default for Queue {
    fn default() -> Self {
        Self::new()
    }
}
impl Queue {
    pub const fn new() -> Self {
        Self { entries: [None; 8] }
    }
    pub fn push(&mut self, callback: Callback) -> bool {
        if callback.function == 0 {
            return false;
        }
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.is_none()) else {
            return false;
        };
        *entry = Some(callback);
        true
    }
    pub fn take(&mut self) -> [Option<Callback>; 8] {
        let snapshot = self.entries;
        self.entries.fill(None);
        snapshot
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_fifo_snapshot_frees_slots_before_foreign_reentry() {
        let mut queue = Queue::new();
        assert!(!queue.push(Callback {
            function: 0,
            argument: 9
        }));
        for argument in 0..8 {
            assert!(queue.push(Callback {
                function: 0x10000,
                argument
            }));
        }
        assert!(!queue.push(Callback {
            function: 0x10000,
            argument: 8
        }));
        let snapshot = queue.take();
        assert!(queue.push(Callback {
            function: 0x20000,
            argument: 9
        }));
        for (index, callback) in snapshot.into_iter().enumerate() {
            assert_eq!(callback.unwrap().argument, index as u64);
        }
        assert_eq!(
            queue.take()[0],
            Some(Callback {
                function: 0x20000,
                argument: 9
            })
        );
        assert_eq!(queue.take(), [None; 8]);
    }
}

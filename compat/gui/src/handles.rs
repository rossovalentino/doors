//! Generational handle tables for `HWND`, `HMENU`, `HACCEL`, `HDC` and
//! `HGDIOBJ`. A handle value is `kind << 24 | generation << 16 | index`:
//! index 0 is never issued, the generation (1..=255) changes every time a
//! slot is reused, and the kind nibble keeps handles of different tables
//! disjoint, so a stale, foreign or forged value is rejected instead of
//! addressing an unrelated object. Values stay below 2^28, so they survive
//! the sign extension Win32 applies to 32-bit handle values.
use crate::{Error, Result};
use alloc::collections::VecDeque;
use alloc::vec::Vec;

/// A freed slot is reused only after this many other slots were freed
/// (first in, first out): with 8-bit generations a stale handle can match
/// again only after about 255 x 64 reuses of the table, not 255 of one slot.
pub const QUARANTINE: usize = 64;

/// Handle kinds; each table uses exactly one.
pub mod kind {
    pub const WINDOW: u8 = 1;
    pub const MENU: u8 = 2;
    pub const ACCEL: u8 = 3;
    pub const DC: u8 = 4;
    pub const GDI: u8 = 5;
    pub const CURSOR: u8 = 6;
    /// Icons and cursors made from images (system cursors keep CURSOR).
    pub const ICON: u8 = 7;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Handle(u32);
impl Handle {
    pub const fn raw(self) -> u32 {
        self.0
    }
    pub const fn value(self) -> u64 {
        self.0 as u64
    }
    pub const fn kind(self) -> u8 {
        (self.0 >> 24) as u8
    }
    pub const fn index(self) -> usize {
        (self.0 & 0xffff) as usize
    }
    pub const fn generation(self) -> u8 {
        (self.0 >> 16) as u8
    }
    /// Decode an application value. Only the exact canonical form is accepted
    /// (upper 32 bits zero, nonzero index and generation).
    pub const fn from_value(value: u64) -> Option<Self> {
        if value >> 28 != 0 || value & 0xffff == 0 || (value >> 16) & 0xff == 0 {
            return None;
        }
        Some(Self(value as u32))
    }
}

struct Slot<T> {
    generation: u8,
    value: Option<T>,
}

pub struct Table<T> {
    kind: u8,
    limit: usize,
    slots: Vec<Slot<T>>,
    /// Freed indices, oldest first.
    free: VecDeque<u16>,
    live: usize,
}

impl<T> Table<T> {
    /// `limit` is at most 65535 objects.
    pub const fn new(kind: u8, limit: usize) -> Self {
        Self {
            kind,
            limit: if limit > 0xffff { 0xffff } else { limit },
            slots: Vec::new(),
            free: VecDeque::new(),
            live: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.live
    }
    pub fn is_empty(&self) -> bool {
        self.live == 0
    }
    fn handle(&self, index: usize) -> Handle {
        Handle(
            (u32::from(self.kind) << 24)
                | (u32::from(self.slots[index].generation) << 16)
                | index as u32,
        )
    }
    pub fn insert(&mut self, value: T) -> Result<Handle> {
        if self.live >= self.limit {
            return Err(Error::Quota);
        }
        if self.slots.is_empty() {
            // Index 0 is reserved so that no handle is ever zero.
            self.slots.try_reserve(1).map_err(|_| Error::Quota)?;
            self.slots.push(Slot {
                generation: 0,
                value: None,
            });
        }
        // A fresh slot while fewer than QUARANTINE wait, else the oldest
        // freed one (FIFO): one slot is not cycled through its generations.
        let fresh = self.free.len() <= QUARANTINE
            && self.slots.len() <= 0xffff
            && self.slots.try_reserve(1).is_ok();
        let index = if fresh {
            self.slots.push(Slot {
                generation: 0,
                value: None,
            });
            self.slots.len() - 1
        } else if let Some(index) = self.free.pop_front() {
            index as usize
        } else {
            return Err(Error::Quota);
        };
        let slot = &mut self.slots[index];
        slot.generation = if slot.generation == 255 {
            1
        } else {
            slot.generation + 1
        };
        slot.value = Some(value);
        self.live += 1;
        Ok(self.handle(index))
    }
    fn slot(&self, handle: Handle) -> Option<&Slot<T>> {
        if handle.kind() != self.kind {
            return None;
        }
        self.slots
            .get(handle.index())
            .filter(|s| s.generation == handle.generation() && s.value.is_some())
    }
    pub fn contains(&self, handle: Handle) -> bool {
        self.slot(handle).is_some()
    }
    pub fn get(&self, handle: Handle) -> Option<&T> {
        self.slot(handle).and_then(|s| s.value.as_ref())
    }
    pub fn get_mut(&mut self, handle: Handle) -> Option<&mut T> {
        self.slot(handle)?;
        self.slots[handle.index()].value.as_mut()
    }
    /// Look up a raw application value.
    pub fn resolve(&self, value: u64) -> Option<Handle> {
        Handle::from_value(value).filter(|h| self.contains(*h))
    }
    pub fn remove(&mut self, handle: Handle) -> Option<T> {
        self.slot(handle)?;
        let value = self.slots[handle.index()].value.take();
        // A free-list push cannot fail: its capacity equals the slot count.
        if self.free.try_reserve(1).is_ok() {
            self.free.push_back(handle.index() as u16);
        }
        self.live -= 1;
        value
    }
    pub fn iter(&self) -> impl Iterator<Item = (Handle, &T)> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter_map(move |(i, s)| s.value.as_ref().map(|v| (self.handle(i), v)))
    }
    pub fn handles(&self) -> Vec<Handle> {
        self.iter().map(|(h, _)| h).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_foreign_and_forged_handles_are_rejected() {
        let mut t = Table::new(kind::WINDOW, 3);
        let a = t.insert("a").unwrap();
        let b = t.insert("b").unwrap();
        assert_ne!(a, b);
        assert_eq!(a.kind(), kind::WINDOW);
        assert!(a.value() < 1 << 28 && a.index() != 0);
        assert_eq!(t.get(a), Some(&"a"));
        assert_eq!(t.remove(a), Some("a"));
        assert_eq!(t.get(a), None, "stale after removal");
        let c = t.insert("c").unwrap();
        assert_ne!(c.index(), a.index(), "a freed slot is quarantined");
        assert_ne!(c, a, "with a new generation");
        assert_eq!(t.get(a), None);
        let mut m: Table<u8> = Table::new(kind::MENU, 4);
        m.insert(1).unwrap();
        assert_eq!(m.resolve(b.value()), None, "kinds are disjoint");
        assert_eq!(t.resolve(b.value()), Some(b));
        assert_eq!(t.resolve(b.value() | 1 << 32), None, "non-canonical");
        assert_eq!(t.resolve(0), None);
        assert_eq!(Handle::from_value(0x0100_0000), None, "generation zero");
        t.insert("d").unwrap();
        assert_eq!(t.insert("e"), Err(Error::Quota));
        assert_eq!(t.len(), 3);
        assert_eq!(t.handles().len(), 3);
    }
    #[test]
    fn generations_wrap_without_zero() {
        let mut t = Table::new(kind::GDI, 2);
        let mut last = t.insert(0u32).unwrap();
        for _ in 0..600 {
            t.remove(last).unwrap();
            last = t.insert(0).unwrap();
            assert_ne!(last.generation(), 0);
        }
    }
    #[test]
    fn a_stale_handle_survives_thousands_of_reuses() {
        let mut t = Table::new(kind::WINDOW, 8);
        let stale = t.insert(0u32).unwrap();
        t.remove(stale).unwrap();
        // CreateWindow/DestroyWindow in a loop: the old value must not come
        // back for far more than 255 cycles (8-bit generations).
        for i in 0..10_000u32 {
            let h = t.insert(i).unwrap();
            assert_ne!(h, stale, "cycle {i}");
            assert_eq!(t.get(stale), None);
            t.remove(h).unwrap();
        }
        // The quarantine bounds the slot count.
        assert!(t.slots.len() <= QUARANTINE + 3);
    }
}

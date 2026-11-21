//! Generational widget identity. An id names one widget instance for its
//! whole life; removing a widget bumps its slot's generation, so delayed
//! callbacks and async results addressed to a recycled slot are rejected.
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WidgetId {
    index: u32,
    generation: u32,
}
impl WidgetId {
    pub const fn index(self) -> u32 {
        self.index
    }
    pub const fn generation(self) -> u32 {
        self.generation
    }
    /// Nonzero 64-bit form (generation in the high half, from 1).
    pub const fn to_bits(self) -> u64 {
        ((self.generation as u64) << 32) | self.index as u64
    }
    pub const fn from_bits(bits: u64) -> Option<Self> {
        let generation = (bits >> 32) as u32;
        if generation == 0 {
            None
        } else {
            Some(Self {
                index: bits as u32,
                generation,
            })
        }
    }
}

struct Slot<T> {
    generation: u32,
    value: Option<T>,
}
/// Slab of values addressed by `WidgetId`.
pub(crate) struct Arena<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    len: usize,
}
impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            len: 0,
        }
    }
}
impl<T> Arena<T> {
    pub fn insert(&mut self, value: T) -> WidgetId {
        self.len += 1;
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            slot.value = Some(value);
            return WidgetId {
                index,
                generation: slot.generation,
            };
        }
        self.slots.push(Slot {
            generation: 1,
            value: Some(value),
        });
        WidgetId {
            index: self.slots.len() as u32 - 1,
            generation: 1,
        }
    }
    pub fn get(&self, id: WidgetId) -> Option<&T> {
        self.slots
            .get(id.index as usize)
            .filter(|s| s.generation == id.generation)
            .and_then(|s| s.value.as_ref())
    }
    pub fn get_mut(&mut self, id: WidgetId) -> Option<&mut T> {
        self.slots
            .get_mut(id.index as usize)
            .filter(|s| s.generation == id.generation)
            .and_then(|s| s.value.as_mut())
    }
    pub fn contains(&self, id: WidgetId) -> bool {
        self.get(id).is_some()
    }
    pub fn remove(&mut self, id: WidgetId) -> Option<T> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        let value = slot.value.take()?;
        // Generations wrap only after 4 billion reuses of one slot; skip 0
        // so `to_bits` stays nonzero.
        slot.generation = slot.generation.wrapping_add(1).max(1);
        self.free.push(id.index);
        self.len -= 1;
        Some(value)
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn ids(&self) -> impl Iterator<Item = WidgetId> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.value.is_some())
            .map(|(i, s)| WidgetId {
                index: i as u32,
                generation: s.generation,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recycled_slots_reject_stale_ids() {
        let mut a = Arena::default();
        let x = a.insert(1);
        assert_eq!(a.remove(x), Some(1));
        let y = a.insert(2);
        assert_eq!(x.index(), y.index());
        assert_ne!(x, y);
        assert!(a.get(x).is_none());
        assert_eq!(a.remove(x), None);
        assert_eq!(a.get(y), Some(&2));
        assert_eq!(WidgetId::from_bits(y.to_bits()), Some(y));
        assert!(WidgetId::from_bits(5).is_none());
        assert_eq!(a.len(), 1);
    }
}

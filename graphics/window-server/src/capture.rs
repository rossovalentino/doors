//! Fixed large allocations shared by retained captures and wallpaper decoding.
//! Three retained 4K clients plus one spare fit the 224 MiB manifest budget.
extern crate alloc;
use alloc::vec::Vec;
pub const CELL_BYTES: usize = 3840 * 2400 * 4;
pub const LARGE_BYTES: usize = 8 * 1024 * 1024;
/// Cells reserved at start. One always stays spare (wallpaper decoding and
/// replacements), so at most `RETAINED_CELLS` first captures larger than
/// `LARGE_BYTES` are held at once; the next is refused.
pub const CELLS: usize = 4;
pub const RETAINED_CELLS: usize = CELLS - 1;
pub struct Pool {
    free: Vec<Vec<u8>>,
}
impl Pool {
    pub fn new() -> Option<Self> {
        let mut free = Vec::new();
        free.try_reserve_exact(CELLS).ok()?;
        for _ in 0..CELLS {
            let mut cell = Vec::new();
            cell.try_reserve_exact(CELL_BYTES).ok()?;
            // Reserve stable large extents without clearing 140 MiB before
            // sign-in. Each capture/decoder initializes only its actual length.
            free.push(cell);
        }
        Some(Self { free })
    }
    /// Keep one spare unless replacing a snapshot that returns another cell.
    /// A window whose snapshot already holds a cell keeps capturing into one
    /// while it shrinks through small sizes (an interactive resize at 4K):
    /// each step would otherwise take a fresh general-heap allocation beside
    /// the retained cell, and the heap left by 4K composition runs out.
    pub fn capture(&mut self, bytes: usize, replacing: bool) -> Option<Vec<u8>> {
        if bytes <= LARGE_BYTES && (!replacing || self.free.is_empty()) {
            let mut pixels = Vec::new();
            pixels.try_reserve_exact(bytes).ok()?;
            pixels.resize(bytes, 0);
            return Some(pixels);
        }
        if bytes > CELL_BYTES || self.free.len() <= usize::from(!replacing) {
            return None;
        }
        let mut pixels = self.free.pop()?;
        pixels.resize(bytes, 0);
        Some(pixels)
    }
    pub fn scratch(&mut self) -> Option<Vec<u8>> {
        self.free.pop()
    }
    pub fn recycle(&mut self, pixels: Vec<u8>) {
        if pixels.capacity() == CELL_BYTES {
            // No allocation: new() reserved all four metadata slots.
            debug_assert!(self.free.len() < CELLS);
            self.free.push(pixels);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_shrinking_cell_snapshot_keeps_capturing_into_cells() {
        // desktop-files/notes accelerated: a 4K window snapshot (a cell) is
        // resized down through sizes below LARGE_BYTES. Each replacement
        // takes a cell and returns the old one; the heap is not touched.
        let mut pool = Pool::new().unwrap();
        let mut retained = pool.capture(16_901_376, false).unwrap();
        for bytes in [7_516_144, 4_527_360, 3_888_000] {
            let next = pool.capture(bytes, true).unwrap();
            assert_eq!((next.len(), next.capacity()), (bytes, CELL_BYTES));
            pool.recycle(core::mem::replace(&mut retained, next));
            assert_eq!(pool.free.len(), 3, "one cell held, the rest free");
        }
        // A first or heap-held small capture still uses the heap.
        assert_ne!(
            pool.capture(4_527_360, false).unwrap().capacity(),
            CELL_BYTES
        );
        // With every cell held, a replacement falls back to the heap.
        let held: Vec<_> = (0..3).map(|_| pool.scratch().unwrap()).collect();
        assert!(pool.capture(4_527_360, true).is_some());
        for cell in held {
            pool.recycle(cell);
        }
        pool.recycle(retained);
        assert_eq!(pool.free.len(), CELLS);
    }
    #[test]
    fn replacements_recycle_and_always_leave_wallpaper_scratch() {
        let mut pool = Pool::new().unwrap();
        let mut retained: Vec<_> = (0..RETAINED_CELLS)
            .map(|_| pool.capture(32_440_320, false).unwrap())
            .collect();
        assert!(pool.capture(32_440_320, false).is_none());
        for index in (0..RETAINED_CELLS).cycle().take(12) {
            let next = pool.capture(32_440_320, true).unwrap();
            pool.recycle(core::mem::replace(&mut retained[index], next));
            let scratch = pool.scratch().unwrap();
            assert_eq!(scratch.capacity(), CELL_BYTES);
            pool.recycle(scratch);
        }
        for pixels in retained {
            pool.recycle(pixels);
        }
        assert_eq!(pool.free.len(), CELLS);
    }
}

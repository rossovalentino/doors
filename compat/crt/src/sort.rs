//! qsort and bsearch over raw elements: an in-place heap sort (no
//! allocation, O(n log n) in every case) with insertion sort for short runs,
//! comparing through the caller's comparator as C's qsort does.

/// Sort `count` elements of `size` bytes at `base` with `compare(a, b)`
/// (negative, zero, positive).
///
/// # Safety
/// `base` points at `count * size` writable bytes.
pub unsafe fn qsort(
    base: *mut u8,
    count: usize,
    size: usize,
    compare: &mut dyn FnMut(*const u8, *const u8) -> i32,
) {
    if count < 2 || size == 0 {
        return;
    }
    let at = |i: usize| unsafe { base.add(i * size) };
    let swap = |a: usize, b: usize| unsafe {
        if a != b {
            core::ptr::swap_nonoverlapping(at(a), at(b), size);
        }
    };
    if count <= 16 {
        for i in 1..count {
            let mut j = i;
            while j > 0 && compare(at(j - 1), at(j)) > 0 {
                swap(j - 1, j);
                j -= 1;
            }
        }
        return;
    }
    // Heap sort.
    let sift =
        |mut root: usize, end: usize, compare: &mut dyn FnMut(*const u8, *const u8) -> i32| loop {
            let mut child = 2 * root + 1;
            if child >= end {
                break;
            }
            if child + 1 < end && compare(at(child), at(child + 1)) < 0 {
                child += 1;
            }
            if compare(at(root), at(child)) >= 0 {
                break;
            }
            swap(root, child);
            root = child;
        };
    for start in (0..count / 2).rev() {
        sift(start, count, compare);
    }
    for end in (1..count).rev() {
        swap(0, end);
        sift(0, end, compare);
    }
}

/// bsearch: the element equal to `key`, or null.
///
/// # Safety
/// `base` points at `count * size` readable bytes, sorted by `compare`.
pub unsafe fn bsearch(
    key: *const u8,
    base: *const u8,
    count: usize,
    size: usize,
    compare: &mut dyn FnMut(*const u8, *const u8) -> i32,
) -> *const u8 {
    let (mut low, mut high) = (0usize, count);
    while low < high {
        let mid = low + (high - low) / 2;
        let element = unsafe { base.add(mid * size) };
        match compare(key, element) {
            0 => return element,
            c if c < 0 => high = mid,
            _ => low = mid + 1,
        }
    }
    core::ptr::null()
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;
    #[test]
    fn sorts_and_searches() {
        for n in [0usize, 1, 2, 15, 16, 17, 100, 1000] {
            let mut v: Vec<u32> = (0..n as u32)
                .map(|i| i.wrapping_mul(2_654_435_761) % 997)
                .collect();
            let mut expected = v.clone();
            expected.sort();
            let mut cmp = |a: *const u8, b: *const u8| unsafe {
                (*(a as *const u32)).cmp(&*(b as *const u32)) as i32
            };
            unsafe { qsort(v.as_mut_ptr() as *mut u8, n, 4, &mut cmp) };
            assert_eq!(v, expected);
            if let Some(&probe) = expected.get(n / 2) {
                let found = unsafe {
                    bsearch(
                        &probe as *const u32 as *const u8,
                        v.as_ptr() as *const u8,
                        n,
                        4,
                        &mut cmp,
                    )
                };
                assert!(!found.is_null());
                assert_eq!(unsafe { *(found as *const u32) }, probe);
            }
        }
    }
}

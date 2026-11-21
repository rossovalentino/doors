use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
struct Tracking;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static LIMIT: AtomicUsize = AtomicUsize::new(usize::MAX);
fn charge(n: usize) -> bool {
    if let Ok(old) = LIVE.fetch_update(SeqCst, SeqCst, |old| {
        old.checked_add(n).filter(|v| *v <= LIMIT.load(SeqCst))
    }) {
        PEAK.fetch_max(old + n, SeqCst);
        true
    } else {
        false
    }
}
unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if !charge(l.size()) {
            return core::ptr::null_mut();
        }
        let p = unsafe { System.alloc(l) };
        if p.is_null() {
            LIVE.fetch_sub(l.size(), SeqCst);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        LIVE.fetch_sub(l.size(), SeqCst);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let growth = n.saturating_sub(l.size());
        if !charge(growth) {
            return core::ptr::null_mut();
        }
        let q = unsafe { System.realloc(p, l, n) };
        if q.is_null() {
            LIVE.fetch_sub(growth, SeqCst);
        } else if n < l.size() {
            LIVE.fetch_sub(l.size() - n, SeqCst);
        }
        q
    }
}
#[global_allocator]
static ALLOC: Tracking = Tracking;
#[test]
fn compressed_one_megapixel_decoders_fit_the_documented_4k_heap_budget() {
    let fonts = door_graphics_text::FontAtlas::windows_reference_scaled(3).unwrap();
    let mut surface = vec![0; 3840 * 2112 * 4];
    let encoded_reserve = vec![0u8; 12 * 1024 * 1024];
    let retained = vec![0u8; 16 * 1024 * 1024];
    // Reserve the entire shared icon-cache allowance, even though this viewer
    // uses far fewer icons. The test includes real font and decode allocations.
    let cache_budget = vec![0u8; 8 * 1024 * 1024];
    for (name, input) in [
        (
            "RGBA PNG",
            include_bytes!("../../../tests/fixtures/preview/rgba-1mp.png").as_slice(),
        ),
        (
            "16-bit PNG",
            include_bytes!("../../../tests/fixtures/preview/gray16-1mp.png").as_slice(),
        ),
        (
            "JPEG",
            include_bytes!("../../../tests/fixtures/preview/rgb-1mp.jpg").as_slice(),
        ),
    ] {
        let captured = input.to_vec();
        let mut image = door_workspace::explorer_preview::decode_full(&captured).unwrap();
        assert_eq!((image.width, image.height), (1024, 1024));
        image.rotate_clockwise();
        image.flip_horizontal();
        let png = door_preview::export_png(&image).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
        drop(png);
        let exported = image.export_qoi().unwrap();
        assert!(!exported.is_empty());
        drop(exported);
        let mut v = door_app_ui::View::new_scaled(&mut surface, 3840, 2112, &fonts, 3);
        v.image_fit((220, 108, 756, 550), image.width, image.height, &image.bgra);
        println!(
            "{name}: live={} peak={} limit={}",
            LIVE.load(SeqCst),
            PEAK.load(SeqCst),
            96 * 1024 * 1024
        );
        assert!(
            PEAK.load(SeqCst) < 88 * 1024 * 1024,
            "keep at least8MiB for allocator/stack/runtime variation"
        );
    }
    assert_eq!(
        retained.len() + cache_budget.len() + encoded_reserve.len(),
        36 * 1024 * 1024
    );
    drop((surface, encoded_reserve, retained, cache_budget, fonts));
    LIMIT.store(128 * 1024 * 1024, SeqCst);
    PEAK.store(LIVE.load(SeqCst), SeqCst);
    let fonts = door_graphics_text::FontAtlas::windows_reference_scaled(3).unwrap();
    let mut surfaces = vec![vec![0u8; 3840 * 2112 * 4]];
    for _ in 0..3 {
        surfaces.push(vec![0u8; 2880 * 1800 * 4]);
    }
    let pending_encoded = vec![0u8; 6 * 1024 * 1024];
    let undo = vec![0u8; 4 * 1024 * 1024];
    let cache = vec![0u8; 8 * 1024 * 1024];
    assert!(
        !door_workspace::runtime::working_memory(32 * 1024 * 1024),
        "shared process working set must reject before an infallible decoder"
    );
    assert_eq!(surfaces.len(), 4);
    assert_eq!(undo.len(), 4 * 1024 * 1024);
    assert_eq!(pending_encoded.len(), 6 * 1024 * 1024);
    surfaces.pop();
    surfaces.pop();
    assert!(door_workspace::runtime::working_memory(32 * 1024 * 1024));
    let image = door_workspace::explorer_preview::decode_full(include_bytes!(
        "../../../tests/fixtures/preview/rgba-1mp.png"
    ))
    .unwrap();
    assert_eq!((image.width, image.height), (1024, 1024));
    assert_eq!(cache.len(), 8 * 1024 * 1024);
    assert!(PEAK.load(SeqCst) <= 128 * 1024 * 1024);
    drop((fonts, surfaces, pending_encoded, undo, cache, image));
    LIMIT.store(usize::MAX, SeqCst);
}

use super::*;
use alloc::{vec, vec::Vec};

/// Darkening (%) of a white backdrop at 3-px steps from the window edge
/// (d = 0 is the first pixel outside), measured on the owner's PC at 150 %:
/// `reports/artifacts/reference-26h2/explorer-shadow-active-white.png` (active, the
/// same profile as `terminal-shadow-white.png`) and `photos-shadow-white.png`
/// (inactive), sampled at the middle of each edge.
const ACTIVE_BOTTOM: [f64; 40] = [
    59.2, 57.6, 55.7, 53.7, 51.4, 49.4, 47.1, 44.7, 42.0, 40.0, 37.6, 35.3, 32.9, 31.0, 29.0, 26.3,
    24.3, 22.7, 20.4, 18.4, 16.9, 15.3, 13.7, 12.2, 11.0, 9.4, 8.2, 7.1, 6.3, 5.1, 4.3, 3.5, 3.1,
    2.4, 2.0, 1.6, 1.2, 0.8, 0.8, 0.4,
];
const ACTIVE_TOP: [f64; 20] = [
    25.5, 22.4, 19.2, 16.1, 13.7, 11.4, 9.0, 7.1, 5.5, 3.9, 2.7, 2.0, 1.2, 0.8, 0.4, 0.4, 0.0, 0.0,
    0.0, 0.0,
];
const ACTIVE_SIDE: [f64; 30] = [
    36.1, 32.9, 29.4, 25.9, 22.7, 19.6, 16.9, 14.5, 11.8, 9.8, 8.2, 6.7, 5.1, 3.9, 3.1, 2.4, 2.0,
    1.2, 0.8, 0.8, 0.4, 0.4, 0.4, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
];
const INACTIVE_BOTTOM: [f64; 40] = [
    22.4, 20.4, 18.8, 17.6, 16.5, 15.3, 14.5, 13.3, 12.9, 12.2, 11.8, 11.0, 10.6, 9.8, 9.4, 8.6,
    8.2, 7.5, 7.1, 6.3, 5.9, 5.1, 4.7, 4.3, 3.5, 3.1, 2.7, 2.4, 2.0, 1.6, 1.6, 1.2, 1.2, 0.8, 0.8,
    0.4, 0.4, 0.4, 0.4, 0.0,
];
const INACTIVE_TOP: [f64; 20] = [
    6.3, 4.7, 3.5, 2.4, 1.6, 1.2, 0.8, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
    0.0,
];
const INACTIVE_SIDE: [f64; 30] = [
    11.4, 9.4, 7.8, 6.3, 5.1, 3.9, 3.5, 2.7, 2.0, 2.0, 1.6, 1.2, 1.2, 0.8, 0.8, 0.8, 0.4, 0.4, 0.4,
    0.4, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
];

fn white(width: u32, height: u32) -> Vec<u8> {
    vec![255; width as usize * height as usize * 4]
}
fn darkening(pixels: &[u8], width: u32, x: i32, y: i32) -> f64 {
    let at = (y as usize * width as usize + x as usize) * 4;
    let sum = u32::from(pixels[at]) + u32::from(pixels[at + 1]) + u32::from(pixels[at + 2]);
    (255.0 - f64::from(sum) / 3.0) / 255.0 * 100.0
}

/// The Explorer window of the reference (1782 x 1141 px at 150 %) over
/// white: every profile sample within 1.5 % of the capture (8-bit captures
/// quantize in 0.4 % steps; the fitted model's RMS residual is 0.3 %).
#[test]
fn profiles_match_the_windows_26h2_reference_captures() {
    let scale = Scale::from_percent(150).unwrap();
    let (width, height) = (2022u32, 1381u32);
    let window = Rect::new(120, 120, 1782, 1141).unwrap();
    for (active, bottom, top, side) in [
        (true, &ACTIVE_BOTTOM[..], &ACTIVE_TOP[..], &ACTIVE_SIDE[..]),
        (
            false,
            &INACTIVE_BOTTOM[..],
            &INACTIVE_TOP[..],
            &INACTIVE_SIDE[..],
        ),
    ] {
        let mut pixels = white(width, height);
        {
            let mut canvas = Canvas::new(&mut pixels, width, height, width as usize * 4).unwrap();
            let mask = ShadowMask::build(ShadowMask::key(window, scale), scale, active).unwrap();
            canvas.window_shadow(window, scale.len(crate::WINDOW_RADIUS), &mask);
        }
        let (cx, cy) = (window.x() + window.width() as i32 / 2, window.y() + 570);
        let check = |expected: &[f64], at: &dyn Fn(i32) -> (i32, i32)| {
            for (i, e) in expected.iter().enumerate() {
                let (x, y) = at(3 * i as i32);
                if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                    continue;
                }
                let d = darkening(&pixels, width, x, y);
                assert!(
                    (d - e).abs() <= 1.5,
                    "active={active} sample {i} at ({x},{y}): {d:.1} % vs {e} %"
                );
            }
        };
        check(bottom, &|d| (cx, window.bottom() + d));
        check(top, &|d| (cx, window.y() - 1 - d));
        check(side, &|d| (window.x() - 1 - d, cy));
        check(side, &|d| (window.right() + d, cy));
    }
}

/// Composition reads cached masks (corner blocks, edge-row constants, one
/// side row; sizes past the layers' reach share one mask). It must equal a
/// direct per-pixel evaluation of both layers for the window's own size, at
/// every scale, small and large, clipped, in both states.
#[test]
fn cached_masks_equal_direct_per_pixel_evaluation() {
    for percent in [100, 125, 150, 200, 300] {
        let scale = Scale::from_percent(percent).unwrap();
        let margins = WINDOW_SHADOW.scaled(scale);
        let radius = scale.len(crate::WINDOW_RADIUS);
        for active in [true, false] {
            let layers = ShadowMask::layers(active);
            // The last size exceeds the shared-mask extent at every scale while
            // the canvas stays within MAX_WIDTH x MAX_HEIGHT.
            let large = (3300 * 100 / percent, 1560 * 100 / percent);
            for (w, h) in [(48, 40), (300, 180), (900, 560), large] {
                let (pw, ph) = (scale.len(w), scale.len(h));
                // Shifted so the clip and the canvas cut the shadow.
                let window =
                    Rect::new(margins.left as i32 - 13, margins.top as i32 + 7, pw, ph).unwrap();
                let bounds = margins.around(window).unwrap();
                let (cw, ch) = (bounds.right() as u32 - 40, bounds.bottom() as u32 - 20);
                let mut actual: Vec<u8> = (0..cw as usize * ch as usize * 4)
                    .map(|i| if i % 4 == 3 { 255 } else { (i % 241) as u8 })
                    .collect();
                let mut expected = actual.clone();
                let clip = Rect::new(3, 0, cw - 9, ch).unwrap();
                {
                    let mut canvas = Canvas::new(&mut actual, cw, ch, cw as usize * 4).unwrap();
                    canvas.set_clip(clip);
                    let mask =
                        ShadowMask::build(ShadowMask::key(window, scale), scale, active).unwrap();
                    canvas.window_shadow(window, radius, &mask);
                }
                let profiles = Profiles::new(layers, margins, (pw, ph), percent).unwrap();
                for y in 0..ch as i32 {
                    for x in 0..cw as i32 {
                        if !clip.contains(x, y)
                            || !bounds.contains(x, y)
                            || window.contains(x, y)
                                && crate::render::rounded_coverage(window, radius, x, y) == 255
                        {
                            continue;
                        }
                        let (bx, by) = ((x - bounds.x()) as usize, (y - bounds.y()) as usize);
                        let c = |i: usize| product(profiles.x[i][bx], profiles.y[i][by]);
                        darken(
                            &mut expected,
                            (y as usize * cw as usize + x as usize) * 4,
                            combine(layers, [c(0), c(1)]),
                        );
                    }
                }
                if let Some(i) = (0..actual.len()).find(|i| actual[*i] != expected[*i]) {
                    let p = i / 4;
                    panic!(
                        "percent={percent} active={active} size={w}x{h} at ({}, {}) bounds={bounds:?} window={window:?}: {} vs {}",
                        p % cw as usize,
                        p / cw as usize,
                        actual[i],
                        expected[i]
                    );
                }
            }
        }
    }
}

/// Every pixel past `WINDOW_SHADOW` rounds to zero alpha, and the shadow is
/// offset down: darkest below the window, then the sides, then the top.
#[test]
fn margins_hold_the_whole_shadow_and_follow_its_offset() {
    const EXTRA: u32 = 24;
    for percent in Scale::PERCENTS {
        let scale = Scale::from_percent(percent).unwrap();
        for active in [true, false] {
            let layers = ShadowMask::layers(active);
            let m = WINDOW_SHADOW.scaled(scale);
            let wide = ShadowMargins {
                left: m.left + EXTRA,
                top: m.top + EXTRA,
                right: m.right + EXTRA,
                bottom: m.bottom + EXTRA,
            };
            let size = (scale.len(1000), scale.len(700));
            let p = Profiles::new(layers, wide, size, percent).unwrap();
            let alpha = |x: usize, y: usize| {
                let c = |i: usize| product(p.x[i][x], p.y[i][y]);
                combine(layers, [c(0), c(1)])
            };
            let (bw, bh) = (p.x[0].len(), p.y[0].len());
            let e = EXTRA as usize;
            for y in 0..bh {
                for x in 0..bw {
                    if x < e || x >= bw - e || y < e || y >= bh - e {
                        assert_eq!(
                            alpha(x, y),
                            0,
                            "percent={percent} active={active} ({x},{y})"
                        );
                    }
                }
            }
            let (cx, cy) = (bw / 2, bh / 2);
            let (left, top) = (wide.left as usize, wide.top as usize);
            let bottom = top + size.1 as usize;
            let (b, s, t) = (alpha(cx, bottom), alpha(left - 1, cy), alpha(cx, top - 1));
            assert!(
                b > s && s > t,
                "percent={percent} active={active}: {b} {s} {t}"
            );
        }
    }
}

/// Larger windows share one mask per scale and state; the active shadow is
/// the stronger one; the cache stays bounded.
#[test]
fn cache_shares_large_sizes_and_bounds_its_entries() {
    let scale = Scale::from_percent(150).unwrap();
    let mut cache = ShadowCache::default();
    let a = cache
        .get(Rect::new(0, 0, 1500, 1000).unwrap(), scale, true)
        .unwrap();
    let b = cache
        .get(Rect::new(40, 9, 2400, 1500).unwrap(), scale, true)
        .unwrap();
    assert!(Rc::ptr_eq(&a, &b));
    let inactive = cache
        .get(Rect::new(0, 0, 1500, 1000).unwrap(), scale, false)
        .unwrap();
    assert!(!Rc::ptr_eq(&a, &inactive));
    assert!(a.center > inactive.center);
    for w in 0..20 {
        cache.get(Rect::new(0, 0, 100 + w, 90).unwrap(), scale, true);
    }
    assert_eq!(cache.masks.len(), CACHED_MASKS);
}

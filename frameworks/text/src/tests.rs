use super::*;

#[test]
fn physical_text_clips_match_full_native_raster_at_unaligned_window_origins() {
    for scale in 2..=3 {
        let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
        let mut full = vec![255; 200 * 100 * 4];
        let mut partial = full.clone();
        let run = TextRun {
            text: "Explorer",
            x: 5,
            top: 7,
            style: TextStyle::Regular14,
            color: [20, 20, 20, 255],
        };
        fonts
            .draw_physical(
                &mut Target::new(&mut full, 200, 100, 800).unwrap(),
                Clip::new(0, 0, 200, 100),
                run,
            )
            .unwrap();
        let clip = Clip::new(8, 13, 47, 29);
        fonts
            .draw_physical(
                &mut Target::new(&mut partial, 200, 100, 800).unwrap(),
                clip,
                run,
            )
            .unwrap();
        for y in 0..100 {
            for x in 0..200 {
                let at = (y * 200 + x) * 4;
                assert_eq!(
                    &partial[at..at + 4],
                    if clip.contains(x as i64, y as i64) {
                        &full[at..at + 4]
                    } else {
                        &[255; 4]
                    }
                );
            }
        }
    }
}

#[test]
fn lcd_text_has_rgb_coverage_at_every_density_and_preserves_alpha_and_clip() {
    for scale in 1..=3 {
        let atlas = FontAtlas::windows_reference_scaled(scale).unwrap();
        let (width, height) = (160 * scale, 40 * scale);
        let clip = Clip::new(4, 2, 130, 28);
        let mut opaque = vec![255; (width * height * 4) as usize];
        let mut transparent = vec![0; opaque.len()];
        for pixels in [&mut opaque, &mut transparent] {
            let mut target = Target::new(pixels, width, height, width as usize * 4)
                .unwrap()
                .cleartype();
            atlas
                .draw(
                    &mut target,
                    clip,
                    TextRun {
                        text: "Settings Explorer",
                        x: 4,
                        top: 2,
                        style: TextStyle::Regular14,
                        color: [0, 0, 0, 255],
                    },
                )
                .unwrap();
        }
        assert!(opaque.chunks_exact(4).any(|p| p[0] != p[1] || p[1] != p[2]));
        assert!(opaque.chunks_exact(4).all(|p| p[3] == 255));
        assert!(
            transparent
                .chunks_exact(4)
                .all(|p| p[..3].iter().all(|c| *c <= p[3]))
        );
        for y in 0..height {
            for x in 0..width {
                if x < 4 * scale || y < 2 * scale || x >= 134 * scale || y >= 30 * scale {
                    let at = ((y * width + x) * 4) as usize;
                    assert_eq!(&opaque[at..at + 4], &[255; 4]);
                    assert_eq!(&transparent[at..at + 4], &[0; 4]);
                }
            }
        }
        assert!(atlas.cache_bytes() <= MAX_ATLAS_BYTES * (scale * scale) as usize * 3);
    }
}
use alloc::{string::String, vec};
use unicode_segmentation::UnicodeSegmentation;
fn run(text: &str) -> TextRun<'_> {
    TextRun {
        text,
        x: 2,
        top: 2,
        style: TextStyle::Regular14,
        color: [0, 0, 0, 255],
    }
}
#[test]
fn text_targets_use_validated_backing_extent_at_widescreen_and_4k_widths() {
    let atlas = FontAtlas::windows_reference().unwrap();
    for width in [1280, 1920, 3840] {
        let mut pixels = vec![255; width * 32 * 4];
        let mut target = Target::new(&mut pixels, width as u32, 32, width * 4).unwrap();
        atlas
            .draw(
                &mut target,
                Clip::new(0, 0, width as u32, 32),
                run("File Explorer"),
            )
            .unwrap();
        assert!(pixels.iter().any(|&v| v != 255));
        assert!(Target::new(&mut pixels, width as u32, 33, width * 4).is_err());
    }
}
#[test]
fn builtin_fonts_have_valid_distinct_ascii_metrics_and_bounded_cache() {
    let atlas = FontAtlas::new().unwrap();
    assert!(atlas.cache_bytes() > 1000);
    assert!(atlas.cache_bytes() <= MAX_ATLAS_BYTES);
    assert!(atlas.cache_capacity() <= MAX_ATLAS_BYTES);
    for style in [
        TextStyle::Regular14,
        TextStyle::Regular20,
        TextStyle::Semibold14,
        TextStyle::Regular12,
        TextStyle::Semibold28,
    ] {
        let m = atlas.metrics(style);
        assert!(m.baseline > 5);
        assert!(m.baseline < m.line_height);
        assert!(m.descent <= 0);
        assert!(
            atlas.measure("iiii", style).unwrap().advance_width
                < atlas.measure("WWWW", style).unwrap().advance_width
        );
    }
    assert!(
        atlas
            .measure("Door OS", TextStyle::Regular20)
            .unwrap()
            .advance_width
            > atlas
                .measure("Door OS", TextStyle::Regular14)
                .unwrap()
                .advance_width
    );
}

#[test]
fn native_density_row_ink_and_caret_share_the_control_center() {
    for scale in 1..=3 {
        let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
        for style in [
            TextStyle::Regular12,
            TextStyle::Regular14,
            TextStyle::Semibold14,
        ] {
            let (width, height) = (128 * scale, 32 * scale);
            let mut pixels = vec![255; width as usize * height as usize * 4];
            let text_top = fonts.centered_top(style, 0, 32);
            let mut target = Target::new(&mut pixels, width, height, width as usize * 4).unwrap();
            fonts
                .draw(
                    &mut target,
                    Clip::new(0, 0, width, height),
                    TextRun {
                        text: "Hg",
                        x: 4,
                        top: text_top,
                        style,
                        color: [0, 0, 0, 255],
                    },
                )
                .unwrap();
            let mut first = height;
            let mut last = 0;
            for (i, p) in pixels.chunks_exact(4).enumerate() {
                if p[..3] != [255, 255, 255] {
                    let y = i as u32 / width;
                    first = first.min(y);
                    last = last.max(y);
                }
            }
            assert!(first <= last);
            assert!(
                (i64::from(first + last) - i64::from(height - 1)).abs() <= i64::from(scale),
                "{scale} {style:?} {first}..{last}"
            );
            let caret = fonts.caret_metrics(style);
            let top = (text_top + caret.top) * scale as i32;
            let bottom = top + caret.height as i32 * scale as i32 - 1;
            assert!(top <= first as i32 && bottom >= last as i32);
            assert!(
                (i64::from(top + bottom) - i64::from(first + last)).abs() <= 2 * i64::from(scale)
            );
        }
    }
}
#[test]
fn local_windows_font_cache_is_bounded_and_measures_reference_caption_size() {
    let atlas = FontAtlas::windows_reference().unwrap();
    assert!(atlas.cache_bytes() <= MAX_ATLAS_BYTES * 3);
    assert_eq!(atlas.metrics(TextStyle::Regular12).pixel_size, 12);
    // Independent FreeType/Pillow reference: Segoe UI12px Documents=60.375px.
    assert!(
        (60..=62).contains(
            &atlas
                .measure("Documents", TextStyle::Regular12)
                .unwrap()
                .advance_width
        )
    );
    assert!(
        atlas
            .measure("This PC", TextStyle::Regular12)
            .unwrap()
            .advance_width
            < atlas
                .measure("This PC", TextStyle::Regular14)
                .unwrap()
                .advance_width
    );
}
#[test]
fn glyph_coverage_has_antialiasing_and_is_repeatable() {
    let atlas = FontAtlas::new().unwrap();
    let mut first = vec![255; 128 * 32 * 4];
    let mut target = Target::new(&mut first, 128, 32, 512).unwrap();
    let clip = target.bounds();
    let metrics = atlas.draw(&mut target, clip, run("Door OS")).unwrap();
    assert!(metrics.painted_pixels > 100);
    assert!(first.chunks_exact(4).any(|p| p[0] > 0 && p[0] < 255));
    assert!(
        first
            .chunks_exact(4)
            .all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 255)
    );
    let mut second = vec![255; 128 * 32 * 4];
    let mut target = Target::new(&mut second, 128, 32, 512).unwrap();
    atlas.draw(&mut target, clip, run("Door OS")).unwrap();
    assert_eq!(first, second);
}
#[test]
fn clipping_preserves_stride_padding_and_guard_bytes() {
    let atlas = FontAtlas::new().unwrap();
    let stride = 48;
    let mut pixels = vec![211; stride * 24 + 16];
    let mut target = Target::new(&mut pixels, 10, 24, stride).unwrap();
    let clip = Clip::new(3, 4, 4, 8);
    atlas
        .draw(
            &mut target,
            clip,
            TextRun {
                x: -2,
                top: -1,
                ..run("WWWWW")
            },
        )
        .unwrap();
    let mut changed = 0;
    for y in 0..24 {
        for x in 0..10 {
            let p = &pixels[y * stride + x * 4..y * stride + x * 4 + 4];
            if !(3..7).contains(&x) || !(4..12).contains(&y) {
                assert_eq!(p, [211; 4]);
            } else if p != [211; 4] {
                changed += 1;
            }
        }
        assert!(
            pixels[y * stride + 40..(y + 1) * stride]
                .iter()
                .all(|b| *b == 211)
        );
    }
    assert!(changed > 0);
    assert_eq!(&pixels[stride * 24..], &[211; 16]);
}
#[test]
fn extreme_offscreen_positions_and_empty_clips_are_safe() {
    let atlas = FontAtlas::new().unwrap();
    let mut pixels = vec![0; 16 * 16 * 4];
    let mut target = Target::new(&mut pixels, 16, 16, 64).unwrap();
    for (x, top) in [(i32::MIN, i32::MIN), (i32::MAX, i32::MAX), (-1000, 2)] {
        let metrics = atlas
            .draw(
                &mut target,
                Clip::new(i32::MIN, i32::MIN, u32::MAX, u32::MAX),
                TextRun {
                    x,
                    top,
                    ..run("Door")
                },
            )
            .unwrap();
        assert_eq!(metrics.painted_pixels, 0);
    }
    let metrics = atlas
        .draw(&mut target, Clip::new(0, 0, 0, 0), run("Door"))
        .unwrap();
    assert_eq!(metrics.painted_pixels, 0);
    assert!(pixels.iter().all(|b| *b == 0));
}
#[test]
fn target_validation_rejects_stride_overflow_and_short_backing() {
    let mut bytes = [0; 64];
    assert!(matches!(
        Target::new(&mut bytes, 5, 4, 16),
        Err(Error::InvalidTarget)
    ));
    assert!(matches!(
        Target::new(&mut bytes, 4, 4, 17),
        Err(Error::InvalidTarget)
    ));
    assert!(matches!(
        Target::new(&mut bytes, 4, 4, usize::MAX - 3),
        Err(Error::InvalidTarget)
    ));
    assert!(matches!(
        Target::new(&mut bytes, 4, 5, 16),
        Err(Error::InvalidTarget)
    ));
    assert!(matches!(
        Target::new(&mut bytes, 0, 1, 4),
        Err(Error::InvalidTarget)
    ));
}
#[test]
fn source_over_coverage_is_premultiplied() {
    let mut bytes = [20, 40, 60, 128];
    let mut target = Target::new(&mut bytes, 1, 1, 4).unwrap();
    assert!(target.blend(Clip::new(0, 0, 1, 1), 0, 0, [255, 255, 255, 255], 128));
    assert_eq!(bytes, [138, 148, 158, 192]);
    let atlas = FontAtlas::new().unwrap();
    let mut bytes = vec![0; 64 * 32 * 4];
    let mut target = Target::new(&mut bytes, 64, 32, 256).unwrap();
    let clip = target.bounds();
    atlas
        .draw(
            &mut target,
            clip,
            TextRun {
                color: [30, 60, 90, 128],
                ..run("Text")
            },
        )
        .unwrap();
    assert!(
        bytes
            .chunks_exact(4)
            .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3])
    );
}
#[test]
fn text_validation_is_bounded_and_atomic_before_paint() {
    let atlas = FontAtlas::new().unwrap();
    let mut bytes = vec![255; 64 * 32 * 4];
    let mut target = Target::new(&mut bytes, 64, 32, 256).unwrap();
    let clip = target.bounds();
    // Without a Unicode engine: controls are zero width and each other
    // grapheme cluster outside ASCII is one replacement glyph.
    let style = TextStyle::Regular14;
    assert_eq!(
        atlas.measure("ab\u{1}\u{200d}", style),
        atlas.measure("ab", style)
    );
    for text in [
        "Door \u{4e2d}\u{6587}",
        "Door e\u{301}\u{302}",
        "Door \u{1f468}\u{200d}\u{1f469}",
    ] {
        let tail = text.split_once(' ').unwrap().1;
        let replaced: String = tail.graphemes(true).map(|_| "?").collect();
        assert_eq!(
            atlas.measure(text, style),
            atlas.measure(&alloc::format!("Door {replaced}"), style)
        );
    }
    assert_eq!(atlas.measure("A\r\nB", style).unwrap().lines, 2);
    let long: String = core::iter::repeat_n('A', MAX_TEXT_BYTES + 1).collect();
    assert_eq!(
        atlas.draw(&mut target, clip, run(&long)),
        Err(Error::TextTooLong)
    );
    assert_eq!(
        atlas.draw(
            &mut target,
            clip,
            TextRun {
                color: [255, 0, 0, 0],
                ..run("Door")
            }
        ),
        Err(Error::InvalidColor)
    );
    assert!(bytes.iter().all(|b| *b == 255));
}
#[test]
fn measure_matches_layout_newlines_tabs_and_draw() {
    let atlas = FontAtlas::new().unwrap();
    let style = TextStyle::Regular14;
    assert_eq!(atlas.measure("", style).unwrap().height, 0);
    let measured = atlas.measure("Door\nOS\tNotes", style).unwrap();
    assert_eq!(measured.lines, 2);
    assert_eq!(measured.height, 40);
    let mut pixels = vec![255; 200 * 60 * 4];
    let mut target = Target::new(&mut pixels, 200, 60, 800).unwrap();
    let clip = target.bounds();
    let drawn = atlas
        .draw(&mut target, clip, run("Door\nOS\tNotes"))
        .unwrap();
    assert_eq!(drawn.advance_width, measured.advance_width);
    assert_eq!(drawn.height, measured.height);
    assert!(drawn.painted_pixels > 0);
    assert_eq!(
        atlas.measure("Door\r\nOS", style),
        atlas.measure("Door\nOS", style)
    );
}
#[test]
fn repeated_draws_do_not_grow_or_rasterize_the_atlas() {
    let atlas = FontAtlas::new().unwrap();
    let size = atlas.cache_bytes();
    let capacity = atlas.cache_capacity();
    let mut pixels = vec![0; 100 * 24 * 4];
    let mut target = Target::new(&mut pixels, 100, 24, 400).unwrap();
    let clip = target.bounds();
    for _ in 0..1000 {
        atlas.draw(&mut target, clip, run("Door OS")).unwrap();
        atlas.measure("Door OS", TextStyle::Semibold14).unwrap();
    }
    assert_eq!(atlas.cache_bytes(), size);
    assert_eq!(atlas.cache_capacity(), capacity);
}
#[test]
fn terminal_monospace_ascii_fits_its_native_cells_at_all_densities() {
    for scale in 1..=3 {
        let atlas = FontAtlas::monospace_reference_scaled(scale).unwrap();
        for (style, cell) in [
            (TextStyle::Regular12, 8),
            (TextStyle::Regular14, 9),
            (TextStyle::Regular20, 12),
        ] {
            for ch in ' '..='~' {
                assert!(
                    atlas
                        .measure(&alloc::format!("{ch}"), style)
                        .unwrap()
                        .advance_width
                        <= cell
                );
            }
        }
    }
}
/// Grayscale mode (shell text on acrylic) draws one coverage per pixel even on
/// opaque targets: no colour fringes, the ink mass of LCD coverage, and the
/// LCD raster's centre (green) subpixel coverage.
#[test]
fn grayscale_target_has_neutral_coverage_and_the_lcd_ink_mass() {
    for scale in 1..=3 {
        let atlas = FontAtlas::windows_reference_scaled(scale).unwrap();
        let (width, height) = (120 * scale, 32 * scale);
        let draw = |gray: bool| {
            let mut pixels = vec![255u8; (width * height * 4) as usize];
            let target = Target::new(&mut pixels, width, height, width as usize * 4).unwrap();
            let mut target = if gray {
                target.grayscale()
            } else {
                target.cleartype()
            };
            let clip = target.bounds();
            atlas
                .draw_physical(
                    &mut target,
                    clip,
                    TextRun {
                        text: "Search",
                        x: 4,
                        top: 4,
                        style: TextStyle::Regular14,
                        color: [26, 26, 26, 255],
                    },
                )
                .unwrap();
            pixels
        };
        let (gray, lcd) = (draw(true), draw(false));
        let ink = |p: &[u8]| -> u32 {
            p.chunks_exact(4)
                .map(|p| p[..3].iter().map(|c| 255 - u32::from(*c)).sum::<u32>())
                .sum()
        };
        assert!(
            gray.chunks_exact(4)
                .all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 255)
        );
        assert!(lcd.chunks_exact(4).any(|p| p[0].abs_diff(p[2]) > 50));
        let (g, l) = (ink(&gray), ink(&lcd));
        assert!(g.abs_diff(l) * 50 <= l, "{scale}x ink mass {g} vs {l}");
        // Green (centre subpixel) coverage is shared by both modes (to the
        // blend rounding).
        assert!(
            gray.chunks_exact(4)
                .zip(lcd.chunks_exact(4))
                .all(|(g, l)| g[1].abs_diff(l[1]) <= 1)
        );
    }
}

/// Windows 11 draws shell and modern-app text with grayscale coverage
/// (`reports/artifacts/reference-26h2`: no colour-fringed pixels in Settings, Start or
/// the Explorer command bar): that is the default; ClearType is opt-in.
#[test]
fn default_target_is_grayscale_and_cleartype_is_opt_in() {
    let atlas = FontAtlas::windows_reference_scaled(1).unwrap();
    let (width, height) = (160, 40);
    let render = |cleartype: bool| {
        let mut pixels = vec![255u8; (width * height * 4) as usize];
        let target = Target::new(&mut pixels, width, height, width as usize * 4).unwrap();
        let mut target = if cleartype {
            target.cleartype()
        } else {
            target
        };
        let clip = target.bounds();
        atlas
            .draw(
                &mut target,
                clip,
                TextRun {
                    text: "Display resolution",
                    x: 4,
                    top: 4,
                    style: TextStyle::Regular14,
                    color: [0, 0, 0, 255],
                },
            )
            .unwrap();
        pixels
    };
    let fringed = |p: &[u8]| {
        p.chunks_exact(4)
            .filter(|p| p[0] != p[1] || p[1] != p[2])
            .count()
    };
    let gray = render(false);
    assert_eq!(fringed(&gray), 0);
    assert!(gray.chunks_exact(4).any(|p| p[0] < 128), "ink drawn");
    assert!(fringed(&render(true)) > 20);
}

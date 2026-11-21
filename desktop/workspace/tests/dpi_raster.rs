//! Exercise the native application raster at its negotiated logical/physical
//! boundary. These are the actual View and resource selectors used by guest apps.
extern crate alloc;
#[allow(dead_code)]
#[path = "../src/icons.rs"]
mod icons;
#[path = "../src/tokens.rs"]
mod tokens;
#[allow(dead_code)]
#[path = "../src/ui.rs"]
mod ui;
use door_graphics_core::Rect;
use door_graphics_text::{FontAtlas, TextStyle};
use icons::Icon;

#[test]
fn modal_warning_labels_focus_and_utf8_fallback_survive_native_minimum_and_density() {
    use door_workspace::dialog::{Dialog, Layout};
    for scale in 1..=3 {
        let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
        for (width, height) in [(240, 178), (490, 358)] {
            let l = Layout::new(width, height, 3);
            let compact = l.panel.width < 336;
            let title = "Save changes to desktop.txt?";
            let body = "Your changes will be lost from this window if you don't save.";
            let title_style = if compact {
                TextStyle::Semibold14
            } else {
                TextStyle::Semibold20
            };
            let body_style = if compact {
                TextStyle::Regular12
            } else {
                TextStyle::Regular14
            };
            let measure = |s: &str, style| fonts.measure(s, style).unwrap().advance_width;
            let lines = |s: &str, style| {
                door_workspace::dialog::lines(s, l.text.width, |s| measure(s, style)).len() as u32
            };
            assert!(
                lines(title, title_style) * title_style.line_height()
                    + if compact { 8 } else { 12 }
                    + lines(body, body_style) * body_style.line_height()
                    <= l.text.height
            );
            let label_style = if compact {
                TextStyle::Regular12
            } else {
                TextStyle::Regular14
            };
            for (b, label) in l.buttons.iter().zip(["Save", "Don't save", "Cancel"]) {
                assert!(measure(label, label_style) <= b.width - 8);
            }
            for dark in [false, true] {
                let pw = width * scale;
                let ph = height * scale;
                let mut pixels = vec![255; (pw * ph * 4) as usize];
                let mut d = Dialog::default();
                d.show(3);
                d.key(15, false);
                {
                    let mut view = ui::View::new_scaled(&mut pixels, pw, ph, &fonts, scale);
                    view.palette.dark = dark;
                    view.dialog(&d, title, body, &["Save", "Don't save", "Cancel"]);
                }
                let b = l.buttons[1];
                assert_eq!(
                    pixel(
                        &pixels,
                        pw as usize,
                        b.x as usize * scale as usize,
                        (b.y as usize + 16) * scale as usize
                    ),
                    if dark {
                        [240, 240, 240, 255]
                    } else {
                        [32, 32, 32, 255]
                    }
                );
                let mut ink = 0;
                for y in l.text.y as u32 * scale..(l.text.y as u32 + l.text.height) * scale {
                    for x in l.text.x as u32 * scale..(l.text.x as u32 + l.text.width) * scale {
                        let p = pixel(&pixels, pw as usize, x as usize, y as usize);
                        if if dark {
                            p[..3].iter().all(|v| *v > 150)
                        } else {
                            p[..3].iter().all(|v| *v < 120)
                        } {
                            ink += 1;
                        }
                    }
                }
                assert!(ink > 50 * scale);
                let mut view = ui::View::new_scaled(&mut pixels, pw, ph, &fonts, scale);
                view.dialog(&d, "é🪟", "好\nSaved", &["Save", "Don't save", "Cancel"]);
            }
        }
    }
}

#[test]
fn native_ascii_metrics_support_monotone_caret_hit_search() {
    for scale in 1..=3 {
        let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
        for before in 32..=126 {
            for after in 32..=126 {
                let pair = [before, after];
                let first = core::str::from_utf8(&pair[..1]).unwrap();
                let both = core::str::from_utf8(&pair).unwrap();
                let advance = |s| {
                    fonts
                        .measure(s, TextStyle::Regular14)
                        .unwrap()
                        .advance_width
                };
                assert!(
                    advance(both) > advance(first),
                    "scale={scale} bytes={pair:?}"
                );
            }
        }
    }
}

#[test]
fn every_color_icon_is_valid_at_all_native_densities_including_explorer() {
    for scale in 1..=3 {
        for icon in [
            Icon::Explorer,
            Icon::Notepad,
            Icon::Settings,
            Icon::Folder,
            Icon::Computer,
            Icon::Desktop,
            Icon::Documents,
            Icon::Downloads,
            Icon::Music,
            Icon::Pictures,
            Icon::Videos,
            Icon::Drive,
            Icon::Network,
            Icon::Pdf,
            Icon::Sheet,
            Icon::Word,
            Icon::CodeFile,
            Icon::VideoFile,
            Icon::AudioFile,
            Icon::ArchiveFile,
            Icon::ModelFile,
            Icon::ModelFileDark,
            Icon::ImageFile,
            Icon::PresentationFile,
        ] {
            for size in [16, 20, 40] {
                let source = icon.color_scaled(size, scale).unwrap();
                let decoded;
                let source = if source.starts_with(b"qoif") {
                    let (header, rgba) = qoi::decode_to_vec(source).unwrap();
                    assert_eq!(header.width, size * scale);
                    assert_eq!(header.height, size * scale);
                    decoded = rgba
                        .chunks_exact(4)
                        .flat_map(|p| [p[2], p[1], p[0], p[3]])
                        .collect::<Vec<_>>();
                    &decoded[..]
                } else {
                    source
                };
                assert_eq!(source.len(), (size * scale * size * scale * 4) as usize);
                assert!(
                    source
                        .chunks_exact(4)
                        .all(|p| p[..3].iter().all(|c| *c <= p[3]))
                );
                assert!(source.chunks_exact(4).any(|p| p[3] > 128));
            }
        }
    }
}

#[test]
fn compressed_previews_decode_once_and_match_native_pixels_with_clipping() {
    for scale in 1..=3 {
        let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
        let cache = ui::ImageCache::default();
        let width = 120 * scale;
        let mut pixels = vec![255; (width * 100 * scale * 4) as usize];
        let (compressed, raw): (&[u8], &[u8]) = match scale {
            3 => (
                include_bytes!("../../../assets/wallpapers/windows/bloom-330x258.qoi"),
                include_bytes!("../../../assets/wallpapers/windows/bloom-330x258.bgr"),
            ),
            2 => (
                include_bytes!("../../../assets/wallpapers/windows/bloom-220x172.qoi"),
                include_bytes!("../../../assets/wallpapers/windows/bloom-220x172.bgr"),
            ),
            _ => (
                include_bytes!("../../../assets/wallpapers/windows/bloom-110x86.qoi"),
                include_bytes!("../../../assets/wallpapers/windows/bloom-110x86.bgr"),
            ),
        };
        let mut expected = pixels.clone();
        ui::View::new_scaled(&mut expected, width, 100 * scale, &fonts, scale)
            .bitmap_bgr_rounded(2, 3, 110, raw, 4);
        for _ in 0..2 {
            pixels.fill(255);
            ui::View::new_scaled(&mut pixels, width, 100 * scale, &fonts, scale)
                .with_images(&cache)
                .bitmap_bgr_rounded(2, 3, 110, compressed, 4);
            assert_eq!(pixels, expected);
        }
    }
}

fn pixel(bytes: &[u8], width: usize, x: usize, y: usize) -> &[u8] {
    &bytes[(y * width + x) * 4..(y * width + x + 1) * 4]
}
#[test]
fn physical_backing_and_scrolled_clip_do_not_leave_odd_edge_pixels() {
    let fonts = FontAtlas::windows_reference_scaled(2).unwrap();
    let mut pixels = vec![0xee; 41 * 31 * 4];
    let mut view = ui::View::new_scaled(&mut pixels, 41, 31, &fonts, 2);
    assert_eq!((view.width, view.height), (20, 15));
    view.rect(0, 0, view.width, view.height, [255; 4]);
    view.viewport(Rect::new(4, 5, 7, 6).unwrap(), 3);
    view.rect(0, 0, 40, 40, [90, 80, 70, 255]);
    for y in 0..31 {
        for x in 0..41 {
            let expected = if (8..22).contains(&x) && (10..22).contains(&y) {
                [90, 80, 70, 255]
            } else {
                [255; 4]
            };
            assert_eq!(pixel(&pixels, 41, x, y), expected);
        }
    }
}
#[test]
fn icon_is_blended_from_native_density_export_without_frame_upscaling() {
    let fonts = FontAtlas::windows_reference_scaled(2).unwrap();
    let mut pixels = vec![255; 100 * 100 * 4];
    let mut view = ui::View::new_scaled(&mut pixels, 100, 100, &fonts, 2);
    view.icon(5, 7, Icon::Settings, [32, 32, 32, 255]);
    let source = include_bytes!("../../../assets/icons/windows/settings.40.bgra");
    for y in 0..40 {
        for x in 0..40 {
            let source = &source[(y * 40 + x) * 4..(y * 40 + x + 1) * 4];
            let mut expected = [255; 4];
            for c in 0..3 {
                expected[c] = (u32::from(source[c]) + u32::from(255 - source[3])).min(255) as u8;
            }
            assert_eq!(pixel(&pixels, 100, x + 10, y + 14), expected);
        }
    }
    assert_eq!(pixel(&pixels, 100, 9, 20), [255; 4]);
    assert_eq!(pixel(&pixels, 100, 50, 20), [255; 4]);
}
#[test]
fn glyphs_use_native_coverage_and_keep_logical_measurement() {
    let fonts = FontAtlas::windows_reference_scaled(2).unwrap();
    let original = FontAtlas::windows_reference().unwrap();
    let capacity = fonts.cache_capacity();
    let mut pixels = vec![255; 240 * 80 * 4];
    let mut view = ui::View::new_scaled(&mut pixels, 240, 80, &fonts, 2);
    let width = view.measure("Settings");
    let unscaled = original
        .measure("Settings", TextStyle::Regular14)
        .unwrap()
        .advance_width;
    assert!(width.abs_diff(unscaled) <= 1);
    view.label(4, 3, "Settings");
    // A stretched1x frame would repeat every source pixel over a2x2 block.
    // Rasterizing the actual outline at2x preserves finer coverage boundaries.
    let detailed_blocks = (0..40)
        .step_by(2)
        .flat_map(|y| (0..160).step_by(2).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let p = pixel(&pixels, 240, x, y);
            p != pixel(&pixels, 240, x + 1, y)
                || p != pixel(&pixels, 240, x, y + 1)
                || p != pixel(&pixels, 240, x + 1, y + 1)
        })
        .count();
    assert!(detailed_blocks > 50);
    assert_eq!(fonts.cache_capacity(), capacity);
}
#[test]
fn rounded_bitmap_uses_full_density_and_respects_physical_viewport() {
    let fonts = FontAtlas::windows_reference_scaled(2).unwrap();
    let mut pixels = vec![255; 24 * 24 * 4];
    let mut source = vec![0; 16 * 16 * 3];
    for (i, p) in source.chunks_exact_mut(3).enumerate() {
        p.copy_from_slice(&[(i % 16) as u8 * 10, (i / 16) as u8 * 10, 17]);
    }
    let mut view = ui::View::new_scaled(&mut pixels, 24, 24, &fonts, 2);
    view.viewport(Rect::new(3, 3, 6, 6).unwrap(), 0);
    view.bitmap_bgr_rounded(2, 2, 8, &source, 2);
    // Physical(12,12) is source(8,8), not a duplicated logical source pixel.
    assert_eq!(pixel(&pixels, 24, 12, 12), [80, 80, 17, 255]);
    assert_eq!(pixel(&pixels, 24, 5, 12), [255; 4]);
    assert_eq!(pixel(&pixels, 24, 18, 12), [255; 4]);
}

#[test]
fn target_theme_swatches_keep_their_own_colors_inside_the_current_palette() {
    for scale in [1, 2] {
        let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
        let width = 40 * scale as usize;
        for dark in [false, true] {
            let mut pixels = vec![0; width * 20 * scale as usize * 4];
            let mut view =
                ui::View::new_scaled(&mut pixels, width as u32, 20 * scale, &fonts, scale);
            view.palette.dark = dark;
            view.viewport(Rect::new(3, 3, 31, 10).unwrap(), 1);
            view.rounded_asset(2, 3, 8, 8, 2, [249, 249, 249, 255]);
            view.rounded_asset(14, 3, 8, 8, 2, [40, 40, 40, 255]);
            view.rounded(26, 3, 8, 8, 2, [249, 249, 249, 255]);
            let s = scale as usize;
            assert_eq!(pixel(&pixels, width, 6 * s, 6 * s), [249, 249, 249, 255]);
            assert_eq!(pixel(&pixels, width, 18 * s, 6 * s), [40, 40, 40, 255]);
            assert_eq!(
                pixel(&pixels, width, 30 * s, 6 * s),
                if dark {
                    [40, 40, 40, 255]
                } else {
                    [249, 249, 249, 255]
                }
            );
            assert_eq!(pixel(&pixels, width, 2 * s, 6 * s), [0; 4]);
            assert_eq!(pixel(&pixels, width, 6 * s, 2 * s), [0; 4]);
        }
    }
}

#[test]
fn selected_glyph_edges_match_white_on_clean_accent_at_native_density() {
    for scale in [1, 2, 3] {
        let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
        let width = 180 * scale;
        let height = 48 * scale;
        for dark in [false, true] {
            let mut actual = vec![0; (width * height * 4) as usize];
            let mut expected = actual.clone();
            let selected = Rect::new(8, 12, 150, 22).unwrap();
            for (pixels, candidate) in [(&mut actual, true), (&mut expected, false)] {
                let mut view = ui::View::new_scaled(pixels, width, height, &fonts, scale);
                view.palette.dark = dark;
                view.rect(0, 0, 180, 48, [255; 4]);
                view.viewport(Rect::new(6, 6, 156, 32).unwrap(), 3);
                if candidate {
                    view.selected_text(
                        10,
                        13,
                        "Selected edges",
                        TextStyle::Regular14,
                        [32, 32, 32, 255],
                        Some(selected),
                    );
                } else {
                    // Independent backdrop-first reference: no dark glyph has
                    // ever touched the selected pixels before white coverage.
                    view.rect(8, 12, 150, 22, [196, 103, 0, 255]);
                    view.text(10, 13, "Selected edges", TextStyle::Regular14, [255; 4]);
                }
                // Selection painting must restore the existing viewport and
                // scroll offset, including pixels outside the selection.
                view.rect(6, 36, 156, 2, [17, 29, 41, 255]);
            }
            assert_eq!(actual, expected, "density={scale}, dark={dark}");
            let edges = actual
                .chunks_exact(4)
                .filter(|p| p[0] > 196 && p[0] < 255 && p[1] > 103 && p[2] > 0)
                .count();
            assert!(
                edges > 20,
                "comparison must include antialiased glyph edges"
            );
        }
    }
}

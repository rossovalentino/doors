//! Exercise the actual app painters with the pinned native-density fonts/assets.
//! Storage is not mocked: these tests perform no I/O or service operations.
#![allow(dead_code, unused_imports)] // Private app modules expose more than paint.
extern crate alloc;
use alloc::{format, string::String};
use door_graphics_text::FontAtlas;
use door_service_client::{Error, Storage};
use door_service_protocol::Code;
use door_workspace::menu;
#[path = "../src/files.rs"]
mod files;
#[path = "../src/icons.rs"]
mod icons;
#[path = "../src/notes.rs"]
mod notes;
#[path = "../src/tokens.rs"]
mod tokens;
#[path = "../src/ui.rs"]
mod ui;

fn paint(scale: u32, dark: bool, draw: impl FnOnce(&mut ui::View<'_>)) -> Vec<u8> {
    let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
    let mut pixels = vec![255; (742 * 498 * scale * scale * 4) as usize];
    let images = ui::ImageCache::default();
    let mut view = ui::View::new_scaled(&mut pixels, 742 * scale, 498 * scale, &fonts, scale)
        .with_images(&images);
    view.palette.dark = dark;
    view.rect(0, 0, 742, 498, [255; 4]);
    draw(&mut view);
    pixels
}
fn ink_rows(pixels: &[u8], scale: u32, dark: bool, rect: (u32, u32, u32, u32)) -> (u32, u32) {
    let (x, y, w, h) = rect;
    let rows: Vec<_> = (y * scale..(y + h) * scale)
        .filter(|py| {
            (x * scale..(x + w) * scale).any(|px| {
                let at = ((py * 742 * scale + px) * 4) as usize;
                let p = &pixels[at..at + 3];
                if dark {
                    p.iter().any(|v| *v > 130)
                } else {
                    p.iter().any(|v| *v < 160)
                }
            })
        })
        .collect();
    (
        *rows
            .first()
            .unwrap_or_else(|| panic!("no ink: scale{scale} dark{dark} rect{rect:?}")),
        *rows.last().unwrap() + 1,
    )
}
fn assert_centered(rows: (u32, u32), scale: u32, top: u32, height: u32) {
    assert!(
        (i64::from(rows.0 + rows.1) - i64::from((top * 2 + height) * scale)).abs()
            <= i64::from(scale * 2),
        "ink {rows:?}, scale {scale}, row {top}/{height}"
    );
}
#[test]
fn actual_command_popup_and_dialog_labels_center_at_each_density_and_theme() {
    for scale in 1..=3 {
        for dark in [false, true] {
            let pixels = paint(scale, dark, |view| {
                // Caps-only labels: the cap band itself must be centred.
                view.button(10, 10, 110, "HH", icons::Icon::Folder, false);
                let mut menu = menu::Menu::default();
                menu.show(10, 70, 742, 498, 1);
                view.popup(&menu, &["HH   HH"]);
            });
            assert_centered(
                ink_rows(&pixels, scale, dark, (64, 10, 40, 30)),
                scale,
                10,
                30,
            );
            assert_centered(
                ink_rows(&pixels, scale, dark, (50, 74, 25, 32)),
                scale,
                74,
                32,
            );
            assert_centered(
                ink_rows(&pixels, scale, dark, (198, 74, 24, 32)),
                scale,
                74,
                32,
            );
            let pixels = paint(scale, dark, |view| {
                let mut dialog = door_workspace::dialog::Dialog::default();
                dialog.show(2);
                view.dialog(&dialog, "Title", "Body", &["HH", "HH"]);
            });
            let layout = door_workspace::dialog::Layout::new(742, 498, 2);
            let b = layout.buttons[1];
            assert_centered(
                ink_rows(
                    &pixels,
                    scale,
                    dark,
                    (b.x as u32 + 20, b.y as u32 + 4, b.width - 40, b.height - 8),
                ),
                scale,
                b.y as u32,
                b.height,
            );
        }
    }
}
#[test]
fn registered_txt_and_home_artwork_exist_at_all_consumed_native_sizes() {
    for name in ["notes.txt", "NOTES.TXT", "multi.part.TxT"] {
        assert!(matches!(
            icons::Icon::for_file_name(name),
            icons::Icon::TextFile
        ));
    }
    for name in ["notes", "notes.txt.bak"] {
        assert!(matches!(
            icons::Icon::for_file_name(name),
            icons::Icon::Document
        ));
    }
    for icon in [icons::Icon::TextFile, icons::Icon::Home] {
        for scale in 1..=3 {
            for size in [16, 20, 40] {
                let pixels = icon.color_scaled(size, scale).expect("native icon export");
                let decoded;
                let pixels = if pixels.starts_with(b"qoif") {
                    let (header, rgba) = qoi::decode_to_vec(pixels).unwrap();
                    assert_eq!(header.width, size * scale);
                    assert_eq!(header.height, size * scale);
                    decoded = rgba
                        .chunks_exact(4)
                        .flat_map(|p| [p[2], p[1], p[0], p[3]])
                        .collect::<Vec<_>>();
                    &decoded[..]
                } else {
                    pixels
                };
                assert_eq!(pixels.len(), (size * size * scale * scale * 4) as usize);
                assert!(pixels.chunks_exact(4).any(|p| p[3] > 0));
                assert!(
                    pixels
                        .chunks_exact(4)
                        .all(|p| p[..3].iter().all(|v| *v <= p[3]))
                );
            }
        }
    }
}

#[test]
fn source_files_use_the_new_format_artwork_without_claiming_an_editor() {
    assert!(matches!(
        icons::Icon::for_file_name("notes.rs"),
        icons::Icon::CodeFile
    ));
}

#[test]
fn lossless_device_assets_preserve_native_alpha_pixels_and_cached_redraws() {
    for scale in 1..=3 {
        let (raw, packed): (&[u8], &[u8]) = match scale {
            1 => (
                include_bytes!("../../../assets/icons/advanced/device-laptop.1.bgra"),
                include_bytes!("../../../assets/icons/advanced/device-laptop.1.qoi"),
            ),
            2 => (
                include_bytes!("../../../assets/icons/advanced/device-laptop.2.bgra"),
                include_bytes!("../../../assets/icons/advanced/device-laptop.2.qoi"),
            ),
            _ => (
                include_bytes!("../../../assets/icons/advanced/device-laptop.3.bgra"),
                include_bytes!("../../../assets/icons/advanced/device-laptop.3.qoi"),
            ),
        };
        let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
        let cache = ui::ImageCache::default();
        let render = |source: &[u8]| {
            let mut pixels = vec![235; (260 * 180 * scale * scale * 4) as usize];
            let mut v = ui::View::new_scaled(&mut pixels, 260 * scale, 180 * scale, &fonts, scale)
                .with_images(&cache);
            v.viewport(door_graphics_core::Rect::new(15, 18, 220, 140).unwrap(), 8);
            v.color_image(20, 28, 200, 136, source);
            pixels
        };
        let expected = render(raw);
        assert_eq!(render(packed), expected);
        assert_eq!(render(packed), expected);
    }
}

#[test]
fn compressed_high_density_hero_icons_match_original_premultiplied_exports() {
    for scale in [2, 3] {
        let raw: &[u8] = if scale == 2 {
            include_bytes!("../../../assets/icons/windows/explorer-home.160.bgra")
        } else {
            include_bytes!("../../../assets/icons/windows/explorer-home.240.bgra")
        };
        let packed = icons::Icon::Home.color_scaled(80, scale).unwrap();
        assert!(packed.starts_with(b"qoif"));
        let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
        let cache = ui::ImageCache::default();
        let render = |source: &[u8]| {
            let mut pixels = vec![235; (100 * 100 * scale * scale * 4) as usize];
            let mut v = ui::View::new_scaled(&mut pixels, 100 * scale, 100 * scale, &fonts, scale)
                .with_images(&cache);
            v.image_fit(
                (10, 10, 64, 64),
                80 * scale as usize,
                80 * scale as usize,
                source,
            );
            pixels
        };
        let expected = render(raw);
        assert_eq!(render(packed), expected);
        assert_eq!(render(packed), expected);
    }
}

#[test]
fn themed_bloom_screens_are_distinct_visible_and_lossless_for_every_device_density() {
    macro_rules! pair {
        ($device:literal,$scale:literal) => {
            (
                include_bytes!(concat!(
                    "../../../assets/icons/advanced/device-",
                    $device,
                    "-light.",
                    $scale,
                    ".qoi"
                )) as &[u8],
                include_bytes!(concat!(
                    "../../../assets/icons/advanced/device-",
                    $device,
                    "-dark.",
                    $scale,
                    ".qoi"
                )) as &[u8],
                include_bytes!(concat!(
                    "../../../assets/icons/advanced/device-",
                    $device,
                    "-light.",
                    $scale,
                    ".bgra"
                )) as &[u8],
                include_bytes!(concat!(
                    "../../../assets/icons/advanced/device-",
                    $device,
                    "-dark.",
                    $scale,
                    ".bgra"
                )) as &[u8],
                $scale,
            )
        };
    }
    for (light, dark, raw_light, raw_dark, scale) in [
        pair!("laptop", 1),
        pair!("laptop", 2),
        pair!("laptop", 3),
        pair!("monitor", 1),
        pair!("monitor", 2),
        pair!("monitor", 3),
        pair!("tablet", 1),
        pair!("tablet", 2),
        pair!("tablet", 3),
    ] {
        assert_ne!(light, dark);
        assert_ne!(raw_light, raw_dark);
        for (packed, expected) in [(light, raw_light), (dark, raw_dark)] {
            let (header, rgba) = qoi::decode_to_vec(packed).unwrap();
            assert_eq!((header.width, header.height), (200 * scale, 136 * scale));
            let bgra: Vec<u8> = rgba
                .chunks_exact(4)
                .flat_map(|p| [p[2], p[1], p[0], p[3]])
                .collect();
            assert_eq!(bgra, expected);
            assert!(
                bgra.chunks_exact(4)
                    .filter(|p| p[0] > 140 && p[0].saturating_sub(p[2]) > 60)
                    .count()
                    > (100 * scale * scale) as usize,
                "Bloom screen must contain visible blue coverage"
            );
            assert!(
                bgra.chunks_exact(4)
                    .all(|p| p[..3].iter().all(|v| *v <= p[3]))
            );
        }
    }
}

#[test]
fn one_lossless_source_keeps_bgr_and_bgra_cache_entries_distinct() {
    let packed = include_bytes!("../../../assets/icons/advanced/device-laptop-dark.1.qoi");
    let bgra = include_bytes!("../../../assets/icons/advanced/device-laptop-dark.1.bgra");
    let bgr: Vec<u8> = bgra
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
    let cache = ui::ImageCache::default();
    let render = |compressed: bool| {
        let mut pixels = vec![235; 440 * 180 * 4];
        let mut v = ui::View::new_scaled(&mut pixels, 440, 180, &fonts, 1).with_images(&cache);
        v.color_image(10, 10, 200, 136, if compressed { packed } else { bgra });
        v.bitmap_bgr_rounded(225, 10, 200, if compressed { packed } else { &bgr }, 6);
        v.image_fit(
            (10, 150, 40, 27),
            200,
            136,
            if compressed { packed } else { bgra },
        );
        pixels
    };
    let expected = render(false);
    assert_eq!(render(true), expected);
    assert_eq!(render(true), expected);
}
/// Files and Notes at 150% through the same View the runtime uses: snapped
/// 1 px lines (no half-alpha line pairs), text from the 150% atlases.
fn fractional_frame(size: (u32, u32), dark: bool, draw: impl FnOnce(&mut ui::View<'_>)) {
    use door_workspace::raster_check::{blurred_lines, crisp_lines};
    let zoom = door_graphics_core::Scale::from_percent(150).unwrap();
    let atlas = door_workspace::assets::get("fonts/segoe-ui/hinted-150.dfa");
    let fonts = FontAtlas::from_packed_percent(atlas, 150).unwrap();
    let mono = FontAtlas::monospace_reference_percent(150).unwrap();
    let images = ui::ImageCache::default();
    let (w, h) = (zoom.len(size.0), zoom.len(size.1));
    let mut pixels = vec![0; (w * h * 4) as usize];
    let mut v = ui::View::new_zoomed(&mut pixels, w, h, &fonts, zoom)
        .with_images(&images)
        .with_monospace(Some(&mono));
    assert_eq!((v.width, v.height), size);
    v.palette.dark = dark;
    draw(&mut v);
    let (w, h) = (w as usize, h as usize);
    assert_eq!(blurred_lines(&pixels, w, h), 0, "half-alpha line pair");
    assert!(crisp_lines(&pixels, w, h) > 0, "no crisp lines");
}
/// Writes File Explorer at 150 % (dark, NoneDrive and Linux on) to
/// DUMP_DIR/explorer-150.bmp for the side-by-side reference check.
#[test]
#[ignore]
fn render_explorer_150() {
    let dir = std::env::var("DUMP_DIR").unwrap();
    let zoom = door_graphics_core::Scale::from_percent(150).unwrap();
    let atlas = door_workspace::assets::get("fonts/segoe-ui/hinted-150.dfa");
    let fonts = FontAtlas::from_packed_percent(atlas, 150).unwrap();
    let images = ui::ImageCache::default();
    let mut f = files::Files::empty(
        "/users/1001".into(),
        door_service_protocol::desktop::Controls::default(),
    );
    f.configure(1000, 700);
    f.show_features_for_test(Some("/users/1001/NoneDrive".into()), true);
    let (w, h) = (zoom.len(1000), zoom.len(700));
    let mut pixels = vec![0; (w * h * 4) as usize];
    let mut v = ui::View::new_zoomed(&mut pixels, w, h, &fonts, zoom).with_images(&images);
    v.palette.dark = true;
    f.draw(&mut v);
    let mut out = Vec::new();
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + w * h * 4).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(-(h as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    out.extend_from_slice(&pixels);
    std::fs::write(format!("{dir}/explorer-150.bmp"), out).unwrap();
}
#[test]
fn files_and_notes_paint_snapped_lines_at_150_percent() {
    let mono = FontAtlas::monospace_reference_percent(150).unwrap();
    for dark in [false, true] {
        let mut f = files::Files::empty(
            "/users/owner".into(),
            door_service_protocol::desktop::Controls::default(),
        );
        f.configure(1000, 640);
        fractional_frame((1000, 640), dark, |v| f.draw(v));
        let mut n = notes::Notes::empty("/users/owner/notes/desktop.txt".into());
        n.configure(900, 640, &mono);
        fractional_frame((900, 640), dark, |v| n.draw(v));
    }
}

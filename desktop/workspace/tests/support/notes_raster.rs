use super::*;
use crate::{assert_centered, ink_rows, paint};
#[test]
fn actual_notes_empty_caret_text_selection_and_footer_share_rows() {
    // The text origin and first row (`document_format`), and the status
    // bar position column of the 742 x 498 client.
    let (left, top) = (fmt::TEXT_LEFT as u32, fmt::TEXT_TOP as u32);
    let chrome = fmt::Chrome {
        width: 742,
        height: 498,
        tabs: 1,
        active: 0,
    };
    let (status_x, status_y) = (
        chrome.status_columns().position as u32,
        chrome.status_top() as u32,
    );
    for scale in 1..=3 {
        for dark in [false, true] {
            let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
            let mut notes = Notes::empty("/users/owner/notes/desktop.txt".into());
            notes.configure(742, 498, &fonts);
            let pixels = paint(scale, dark, |view| notes.draw(view));
            let caret_rows: Vec<_> = (76 * scale..120 * scale)
                .filter(|y| {
                    let at = ((y * 742 * scale + left * scale) * 4) as usize;
                    pixels[at..at + 4] == [196, 103, 0, 255]
                })
                .collect();
            assert!(
                !caret_rows.is_empty(),
                "empty document caret must remain visible"
            );
            let caret = (
                *caret_rows.first().unwrap(),
                *caret_rows.last().unwrap() + 1,
            );
            for y in caret.0..caret.1 {
                for x in left * scale..(left + 1) * scale {
                    let at = ((y * 742 * scale + x) * 4) as usize;
                    assert_eq!(
                        &pixels[at..at + 4],
                        &[196, 103, 0, 255],
                        "empty caret must be a continuous strip"
                    );
                }
            }
            notes.current.document.insert("Hg").unwrap();
            notes.current.message = "Hg".into();
            notes.configure(742, 498, &fonts);
            let pixels = paint(scale, dark, |view| notes.draw(view));
            let width = fonts
                .measure("Hg", TextStyle::Regular14)
                .unwrap()
                .advance_width;
            let ink = ink_rows(&pixels, scale, dark, (left, top, width, 20));
            assert_centered(ink, scale, top, 20);
            assert!(caret.0 <= ink.0 && caret.1 >= ink.1);
            assert!(
                (i64::from(caret.0 + caret.1) - i64::from(ink.0 + ink.1)).abs()
                    <= i64::from(2 * scale),
                "caret{caret:?} ink{ink:?} scale{scale} dark{dark}"
            );
            assert_centered(
                // Status text centres cap height: measure the "L" only.
                ink_rows(
                    &pixels,
                    scale,
                    dark,
                    (
                        status_x,
                        status_y,
                        fonts
                            .measure("L", TextStyle::Regular12)
                            .unwrap()
                            .advance_width,
                        32,
                    ),
                ),
                scale,
                status_y,
                32,
            );
            notes.current.document.select_all();
            notes.configure(742, 498, &fonts);
            let selected = paint(scale, dark, |view| notes.draw(view));
            let rows: Vec<_> = (top * scale..(top + 20) * scale)
                .filter(|y| {
                    (left * scale..(left + width) * scale).any(|x| {
                        let at = ((y * 742 * scale + x) * 4) as usize;
                        selected[at..at + 3]
                            .iter()
                            .zip([196u8, 103, 0])
                            .any(|(value, bg)| value.saturating_sub(bg) > 50)
                    })
                })
                .collect();
            let selected_ink = (*rows.first().unwrap(), *rows.last().unwrap() + 1);
            assert!(
                (i64::from(selected_ink.0) - i64::from(ink.0)).abs() <= i64::from(scale)
                    && (i64::from(selected_ink.1) - i64::from(ink.1)).abs() <= i64::from(scale),
                "selection {selected_ink:?} normal {ink:?} must preserve baseline"
            );
        }
    }
}

#[test]
fn independent_tabs_keep_undo_save_guard_and_cancelled_close_choices() {
    let mut n = Notes::empty("/users/1001/notes/a.txt".into());
    n.current.document.insert("first").unwrap();
    n.new_tab();
    n.current.document.insert("second").unwrap();
    n.current.document.select_all();
    n.apply_tool(Tool::Bold);
    assert_eq!(n.current.document.to_text(), "**second**");
    n.current.document.undo();
    assert_eq!(n.current.document.to_text(), "second");
    n.switch_raw(0);
    assert_eq!(n.current.document.to_text(), "first");
    assert!(n.current.document.dirty());
    assert!(!n.close_requested());
    assert!(n.modal());
    n.current.prompt = Prompt::default();
    n.closing_all = false;
    n.switch_raw(1);
    assert_eq!(n.current.document.to_text(), "second");
    assert!(n.current.document.dirty());
    n.current.document.undo();
    assert_eq!(n.current.document.to_text(), "");
    n.switch_raw(0);
    assert_eq!(n.current.document.to_text(), "first");
}

/// Notepad menu bar: plates are the measured label plus 10 px per side, the
/// label ink is centred in its plate (±0.5 px) with caps centred vertically,
/// and the hover plate is exactly the hit rectangle.
#[test]
fn menu_bar_plates_fit_their_labels_and_centre_them() {
    for scale in [1u32, 2] {
        for (width, height) in [(742u32, 498u32), (240, 178)] {
            let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
            let mut notes = Notes::empty("/users/owner/notes/desktop.txt".into());
            notes.configure(width, height, &fonts);
            let bar = [250u8, 247, 243, 255];
            let paint_at = |notes: &Notes, hover: Option<(i32, i32)>| {
                let mut pixels = vec![0; (width * height * scale * scale * 4) as usize];
                {
                    let mut v = ui::View::new_scaled(
                        &mut pixels,
                        width * scale,
                        height * scale,
                        &fonts,
                        scale,
                    );
                    v.hover = hover;
                    notes.draw(&mut v);
                }
                pixels
            };
            let pixels = paint_at(&notes, None);
            let plates = notes.menu_bar();
            let at = |p: &[u8], x: u32, y: u32| {
                let i = ((y * width * scale + x) * 4) as usize;
                [p[i], p[i + 1], p[i + 2], p[i + 3]]
            };
            for (i, (r, label)) in plates.iter().zip(MENU_LABELS).enumerate() {
                let advance = fonts
                    .measure(label, TextStyle::Regular14)
                    .unwrap()
                    .advance_width;
                assert_eq!(r.width, advance + 20, "{label} plate width");
                if i > 0 {
                    let gap = fmt::MENU_GAP;
                    assert_eq!(r.x, plates[i - 1].x + plates[i - 1].width as i32 + gap);
                }
                // Hit testing uses exactly the painted plates.
                let id = [1, 40, 41][i];
                assert_eq!(notes.target(r.x + 1, r.y + 1), Some(id));
                assert_eq!(notes.target(r.x + r.width as i32 - 1, r.y + 16), Some(id));
                assert_ne!(notes.target(r.x - 1, r.y + 16), Some(id));
                // Ink bounding box of the label.
                let s = scale;
                let (mut x0, mut x1, mut y0, mut y1) = (u32::MAX, 0, u32::MAX, 0);
                for y in r.y as u32 * s..(r.y as u32 + r.height) * s {
                    for x in r.x as u32 * s..(r.x as u32 + r.width) * s {
                        if at(&pixels, x, y) != bar {
                            x0 = x0.min(x);
                            x1 = x1.max(x + 1);
                            y0 = y0.min(y);
                            y1 = y1.max(y + 1);
                        }
                    }
                }
                let twice_box = (2 * r.x as i64 + r.width as i64) * s as i64;
                assert!(
                    (i64::from(x0 + x1) - twice_box).abs() <= i64::from(s),
                    "{label} ink {x0}..{x1} not centred in {r:?} at {s}x"
                );
                // File and Edit start with a full-height capital stem: the
                // rows of their first ink column are the cap band.
                if label != "View" {
                    let rows: Vec<u32> = (y0..y1).filter(|&y| at(&pixels, x0, y) != bar).collect();
                    let (c0, c1) = (rows[0], rows[rows.len() - 1] + 1);
                    let twice_row = (2 * r.y as i64 + r.height as i64) * s as i64;
                    assert!(
                        (i64::from(c0 + c1) - twice_row).abs() <= i64::from(s),
                        "{label} caps {c0}..{c1} not centred in {r:?} at {s}x"
                    );
                }
                // The hover plate covers exactly the plate's columns.
                let hovered = paint_at(&notes, Some((r.x + 2, r.y + 2)));
                let mid = (r.y as u32 + r.height / 2) * s;
                let plate = at(&hovered, (r.x as u32 + 1) * s, mid);
                assert_ne!(plate, bar, "{label} hover plate missing");
                assert_eq!(at(&hovered, r.x as u32 * s - 1, mid), bar);
                assert_eq!(at(&hovered, (r.x as u32 + r.width) * s, mid), bar);
                assert_eq!(at(&hovered, (r.x as u32 + r.width) * s - 1, mid), plate);
            }
        }
    }
}

/// New windows open at 70% x 78% of the work area (`initial_window`): Notes'
/// 900x640 request becomes 896x524 at 1280x720, 717x562 at 1024x768 and
/// 896x587 at 1280x800. The menu bar, the formatting bar, the body and the
/// status bar keep their places without overlapping.
#[test]
fn notes_lays_out_at_the_default_window_sizes() {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/notes-review");
    std::fs::create_dir_all(&directory).unwrap();
    let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
    let mono = FontAtlas::monospace_reference_scaled(1).unwrap();
    for output in [(1280, 720), (1024, 768), (1280, 800), (1920, 1080)] {
        let (_, _, width, height) = door_graphics_core::initial_window(output, (900, 640), 0, None);
        let mut notes = Notes::empty("/users/owner/notes/desktop.txt".into());
        notes
            .current
            .document
            .insert("Doors notes\r\nA second line of text at the default window size.")
            .unwrap();
        notes.configure(width, height, &mono);
        let mut pixels = vec![0; (width * height * 4) as usize];
        {
            let mut v = ui::View::new_scaled(&mut pixels, width, height, &fonts, 1)
                .with_monospace(Some(&mono));
            notes.draw(&mut v);
        }
        let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
        ppm.extend(pixels.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]));
        std::fs::write(directory.join(format!("default-{width}x{height}.ppm")), ppm).unwrap();
        // Formatting buttons sit right of the menu plates and left of What's
        // new and Settings (80 px), or are hidden.
        let menus = notes.menu_bar();
        let last = menus[2].x + menus[2].width as i32;
        let chrome = notes.chrome();
        for tool in Tool::ALL {
            if let Some(b) = chrome.tool(tool) {
                assert!(
                    b.x > last && b.x + b.width as i32 <= chrome.whats_new().x,
                    "{width}x{height}"
                );
            }
        }
        // The body keeps rows above the 32 px status bar.
        assert!(notes.current.view.rows() >= 10, "{width}x{height}");
        assert!(
            fmt::TEXT_TOP + notes.current.view.rows() as i32 * 20 <= chrome.status_top(),
            "{width}x{height}"
        );
    }
}

/// Review renders against the owner's dark 26H2 Notepad screenshot (150 %,
/// 1268 x 737 client): the reference sample with the caret on line 2,
/// then Formatted Markdown with a hovered, unsaved tab. Written to
/// `target/notes-review` for inspection; asserts the shared chrome colours.
#[test]
fn notes_review_renders_match_the_dark_reference_chrome() {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/notes-review");
    std::fs::create_dir_all(&directory).unwrap();
    let zoom = door_graphics_core::Scale::from_percent(150).unwrap();
    let atlas = door_workspace::assets::get("fonts/segoe-ui/hinted-150.dfa");
    let fonts = FontAtlas::from_packed_percent(atlas, 150).unwrap();
    let mono = FontAtlas::monospace_reference_percent(150).unwrap();
    let samples = [
        ("reference", "Door OS reference sample\r\n", None),
        (
            "formatted",
            "# Plan **now**\r\n- first *item* and ~~old~~\r\n2. [Door](https://door)\r\n| a | b |\r\n| --- | --- |\r\nplain text",
            Some((120, 20)),
        ),
    ];
    for (name, text, hover) in samples {
        let mut notes = Notes::empty("/users/owner/notes/Notes.txt".into());
        notes.current.document.insert(text).unwrap();
        notes.configure(1268, 737, &mono);
        let (w, h) = (zoom.len(1268), zoom.len(737));
        let mut pixels = vec![0; (w * h * 4) as usize];
        {
            let images = ui::ImageCache::default();
            let mut v = ui::View::new_zoomed(&mut pixels, w, h, &fonts, zoom)
                .with_images(&images)
                .with_monospace(Some(&mono));
            v.palette.dark = true;
            v.hover = hover;
            notes.draw(&mut v);
        }
        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
        ppm.extend(pixels.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]));
        std::fs::write(directory.join(format!("dark-150-{name}.ppm")), ppm).unwrap();
        let at = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            [pixels[i + 2], pixels[i + 1], pixels[i]]
        };
        // Editor #272727, band and status on the layer, Mica caption.
        assert_eq!(at(w / 2, zoom.len(400)), [39; 3]);
        assert_eq!(at(zoom.len(100), zoom.len(60)), [45; 3]);
        assert_eq!(at(zoom.len(600), zoom.len(722)), [45; 3]);
        assert_eq!(at(zoom.len(600), zoom.len(5)), [32; 3]);
    }
}

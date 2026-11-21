//! Points the guest `desktop-notes` scenario (tools/xtask/src/desktop/notes.rs)
//! uses inside Notes' client, checked against the real document viewport,
//! dialog layout and dialog painter. Notes opens at (192, 74, 896, 524) on the
//! 1280x720 logical desktop of both profiles (the xtask host tests assert the
//! placement) and draws its own caption, so client = window. Its body
//! (`document_format::Chrome::body`, used by notes.rs `configure`) is at
//! (16, 89), client width - 32 wide, down to the 32 px status bar, with 20 px
//! rows, laid out with the monospace editor font at the output density. The
//! scenario's literal points are derived here from those layout functions.
use door_graphics_text::{FontAtlas, TextStyle};
use door_workspace::View;
use door_workspace::dialog::{Dialog, Layout};
use door_workspace::document::Document;
use door_workspace::document_format::{self as fmt, Chrome, TEXT_LEFT, TEXT_TOP};
use door_workspace::document_layout::Engine;
use door_workspace::document_view::{DocumentView, Geometry};
use door_workspace::explorer_layout::Bounds;

const CLIENT: (u32, u32) = (896, 524);
const MINIMUM: (u32, u32) = (240, 178);
const LABELS: [&str; 3] = ["Save", "Don't save", "Cancel"];
/// The scenario's client points (tools/xtask/src/desktop/notes.rs):
/// `TEXT_X`, `CARET_Y` and `ROW_Y` offsets, the select-all accent probe
/// row, `WRAP_WIDTH` and the body crop's right edge.
const TEXT_X: i32 = 16;
const CARET_Y: i32 = 91;
const ROW_Y: i32 = 96;
const SELECT_Y: i32 = 92;
const WRAP_WIDTH: u32 = 864;
const BODY_RIGHT: i32 = 885;
fn chrome(size: (u32, u32)) -> Chrome {
    Chrome {
        width: size.0,
        height: size.1,
        tabs: 1,
        active: 0,
    }
}

fn rect(b: Bounds) -> (i32, i32, u32, u32) {
    (b.x, b.y, b.width, b.height)
}
fn geometry() -> Geometry {
    let body = chrome(CLIENT).body(true);
    Geometry {
        x: body.x,
        y: body.y,
        width: body.width,
        height: body.height,
        line_height: 20,
    }
}
#[test]
fn notes_scenario_constants_are_the_layout() {
    let body = chrome(CLIENT).body(true);
    assert_eq!((TEXT_X, body.x, body.y), (TEXT_LEFT, TEXT_LEFT, TEXT_TOP));
    assert_eq!(body.width, WRAP_WIDTH);
    // The crop keeps 5 px past the wrap edge for trailing wrap carets.
    assert_eq!(BODY_RIGHT, body.x + WRAP_WIDTH as i32 + 5);
    // The pointer row and select-all probe are inside the first row.
    let first = TEXT_TOP..TEXT_TOP + 20;
    assert!(first.contains(&ROW_Y) && first.contains(&SELECT_Y) && first.contains(&93));
    assert!(chrome(CLIENT).status_top() == 492);
}
fn editor(scale: u32) -> FontAtlas {
    FontAtlas::monospace_reference_scaled(scale).unwrap()
}
fn width(fonts: &FontAtlas, text: &str) -> u32 {
    fonts
        .measure(text, TextStyle::Regular14)
        .map_or(0, |m| m.advance_width)
}

#[test]
fn notes_scenario_dialog_geometry_matches_the_app() {
    let full = Layout::new(CLIENT.0, CLIENT.1, 3);
    assert_eq!(rect(full.panel), (224, 146, 448, 232));
    assert_eq!(
        full.buttons.map(rect),
        [
            (248, 322, 128, 32),
            (384, 322, 128, 32),
            (520, 322, 128, 32)
        ]
    );
    let narrow = Layout::new(MINIMUM.0, MINIMUM.1, 3);
    assert_eq!(rect(narrow.panel), (8, 8, 224, 162));
    assert_eq!(
        narrow.buttons.map(rect),
        [(16, 130, 66, 32), (86, 130, 66, 32), (156, 130, 66, 32)]
    );
    // The text-cursor probe keeps the earlier fixture's panel-relative point:
    // (400, 198) in its 490 x 358 client.
    let earlier = Layout::new(490, 358, 3);
    assert_eq!(
        (603 - full.panel.x, 281 - full.panel.y),
        (400 - earlier.panel.x, 198 - earlier.panel.y)
    );
    // The outside click misses the panel but lands on the editor body.
    assert!(!full.panel.contains(20, 308));
    // Minimum client: focus/press probes and the held point are on Save; the
    // border probe is Save's left column; the File-overlap point is panel only.
    for (x, y) in [(22, 140), (40, 146), (16, 146)] {
        assert!(narrow.buttons[0].contains(x, y), "({x}, {y})");
    }
    assert_eq!(narrow.buttons[0].x, 16);
    assert!(narrow.panel.contains(20, 44));
    assert!(narrow.buttons.iter().all(|b| !b.contains(20, 44)));
    // The File menu plate lies under that point: its measured label plus
    // 10 px each side from x 4 (4..45), y 41..73 (`Chrome::menu`, which
    // notes.rs `menu_bar` paints and hit-tests).
    for scale in [1, 3] {
        let ui = FontAtlas::windows_reference_scaled(scale).unwrap();
        let labels = ["File", "Edit", "View"].map(|l| width(&ui, l));
        let file = chrome(CLIENT).menu(labels)[0];
        assert_eq!((file.x, file.width), (4, 41), "scale {scale}");
        assert!(file.contains(20, 44));
        assert_eq!((file.y, file.height), (fmt::BAND_TOP, fmt::BAND_HEIGHT));
    }
}

/// Paint the Notes save prompt at `client` and return RGB at a logical point.
fn dialog_pixels(
    client: (u32, u32),
    scale: u32,
    dark: bool,
    dialog: &Dialog,
    pointer: Option<(i32, i32)>,
    pressed: bool,
) -> impl Fn(i32, i32) -> [u8; 3] {
    let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
    let (pw, ph) = (client.0 * scale, client.1 * scale);
    let mut pixels = vec![255u8; (pw * ph * 4) as usize];
    {
        let mut view = View::new_scaled(&mut pixels, pw, ph, &fonts, scale);
        view.palette.dark = dark;
        view.hover = pointer;
        view.pressed = pointer.filter(|_| pressed);
        view.dialog(
            dialog,
            "Save changes to desktop.txt?",
            "Your changes will be lost from this window if you don't save.",
            &LABELS,
        );
    }
    move |x, y| {
        let at = ((y as u32 * scale * pw + x as u32 * scale) * 4) as usize;
        [pixels[at + 2], pixels[at + 1], pixels[at]]
    }
}

#[test]
fn notes_scenario_dialog_probes_match_the_painter() {
    for scale in [1, 3] {
        let full = Layout::new(CLIENT.0, CLIENT.1, 3);
        let save = full.buttons[0];
        for dark in [false, true] {
            let mut dialog = Dialog::default();
            dialog.show(3);
            let pixel = dialog_pixels(CLIENT, scale, dark, &dialog, None, false);
            // Keyboard focus on Save: accent fill and the white focus ring.
            assert_eq!(
                pixel(save.x + 5, save.y + 9),
                [0, 103, 196],
                "{scale} {dark}"
            );
            assert_eq!(pixel(save.x + 2, save.y + 15), [255; 3], "{scale} {dark}");
        }
        let narrow = Layout::new(MINIMUM.0, MINIMUM.1, 3);
        let mut dialog = Dialog::default();
        dialog.show(3);
        let pixel = dialog_pixels(MINIMUM, scale, false, &dialog, None, false);
        assert_eq!(pixel(22, 140), [0, 103, 196]);
        // Save held: the pressed accent.
        let mut held = Dialog::default();
        held.show(3);
        held.pointer(held.target(&narrow, 40, 146));
        let pixel = dialog_pixels(MINIMUM, scale, false, &held, Some((40, 146)), true);
        assert_eq!(pixel(22, 140), [0, 80, 160]);
        // A non-button press clears keyboard focus: Save's neutral border.
        let mut file = Dialog::default();
        file.show(3);
        file.pointer(file.target(&narrow, 20, 44));
        let pixel = dialog_pixels(MINIMUM, scale, false, &file, Some((20, 44)), true);
        assert_eq!(pixel(16, 146), [190; 3]);
    }
}

#[test]
fn notes_scenario_body_points_hit_the_intended_text() {
    for scale in [1, 3] {
        let fonts = editor(scale);
        let engine = Engine::new(&fonts, 100);
        let measure = |s: &str| width(&fonts, s);
        // Pointer insert: the boundary after "Original " on the first row.
        let mut document = Document::new("Original text\r\n").unwrap();
        let mut view = DocumentView::default();
        view.configure(&document, geometry(), &engine);
        assert_eq!(view.rows(), 20);
        let x = TEXT_X + measure("Original ") as i32;
        assert!(view.pointer_down(&mut document, x, ROW_Y, &engine));
        view.cancel_drag();
        document.insert("x").unwrap();
        assert_eq!(document.to_text(), "Original xtext\r\n", "scale {scale}");
        // Drag from the text start past the line end selects the visible line,
        // exactly the keyboard Home, Shift+End selection.
        let mut document = Document::new("Original text\r\n").unwrap();
        let mut view = DocumentView::default();
        view.configure(&document, geometry(), &engine);
        assert!(view.pointer_down(&mut document, TEXT_X + 1, ROW_Y, &engine));
        assert!(view.pointer_motion(&mut document, 460, 93, &engine));
        view.cancel_drag();
        assert_eq!(document.selection(), 0..13);
        // Select-all highlights the first row to the body edge (client x 880),
        // so the accent probe (400, SELECT_Y) is covered; row 3 (y 158) is empty.
        document.select_all();
        view.refresh(&document, &engine);
        let mut rows = Vec::new();
        view.visible(&document, &engine, |block, _, range| {
            for r in range {
                rows.push(block.selection(&engine, r, document.selection()).1);
            }
        });
        assert_eq!(rows, [true, false]);
        assert!(TEXT_X + measure("Original text") as i32 <= 165);
        assert!(CLIENT.0 as i32 - TEXT_LEFT > 400);
        assert!((TEXT_TOP..TEXT_TOP + 20).contains(&SELECT_Y));
        assert!((TEXT_TOP + 3 * 20..TEXT_TOP + 4 * 20).contains(&158));
        // Hover, text-cursor and outside-click points are inside the body.
        for (x, y) in [(200, 113), (603, 281), (20, 308), (470, 158)] {
            assert!(view.body_contains(x, y), "({x}, {y})");
        }
        // 160 W wrap once at the scenario's rule: the longest prefix that fits
        // the 864 px body, with the end caret left of the body crop (885).
        let wrapped = "W".repeat(160);
        let document = Document::new(&format!("{wrapped}\r\n")).unwrap();
        let mut view = DocumentView::default();
        view.configure(&document, geometry(), &engine);
        let count = (1..=160)
            .take_while(|n| measure(&wrapped[..*n]) <= WRAP_WIDTH)
            .last()
            .unwrap();
        assert!(count < 160);
        view.with_block(&document, &engine, 0, |block| {
            assert_eq!(block.row_range(0), 0..count, "scale {scale}");
            assert_eq!(block.row_range(1).end, 160);
        });
        assert!(TEXT_X + (measure(&wrapped[..count]) as i32) < BODY_RIGHT);
    }
}

#[test]
fn notes_scenario_caret_probe_row_is_caret_ink() {
    // CARET_Y is client 91: inside the editor caret on the first body row
    // (89..109) and, one row lower, on the second.
    for scale in [1, 3] {
        let fonts = editor(scale);
        let top = fonts.centered_top(TextStyle::Regular14, TEXT_TOP, 20);
        let caret = fonts.caret_metrics(TextStyle::Regular14);
        let rows = top + caret.top..top + caret.top + caret.height as i32;
        assert!(
            rows.contains(&CARET_Y) && rows.contains(&(CARET_Y + 1)),
            "{rows:?} scale {scale}"
        );
        assert!(
            rows.start >= TEXT_TOP && rows.end <= TEXT_TOP + 20,
            "{rows:?} scale {scale}"
        );
    }
}

#[test]
fn notes_scenario_minimum_dialog_bands_keep_readable_glyphs() {
    // notes.rs `modal_light`: at the 240x178 minimum client, the question
    // (client rows 16..36) and item + warning (44..88), x 17..223, each hold
    // at least 30 dark pixels per density step.
    for scale in [1u32, 3] {
        let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
        let (pw, ph) = (MINIMUM.0 * scale, MINIMUM.1 * scale);
        let mut pixels = vec![255u8; (pw * ph * 4) as usize];
        {
            let mut view = View::new_scaled(&mut pixels, pw, ph, &fonts, scale);
            let mut dialog = Dialog::default();
            dialog.show(3);
            view.dialog(
                &dialog,
                "Save changes to desktop.txt?",
                "Your changes will be lost from this window if you don't save.",
                &LABELS,
            );
        }
        for (name, top, bottom) in [("question", 16, 36), ("warning", 44, 88)] {
            let ink = (top * scale..bottom * scale)
                .flat_map(|y| (17 * scale..223 * scale).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    let at = ((y * pw + x) * 4) as usize;
                    pixels[at..at + 3].iter().all(|v| *v < 120)
                })
                .count();
            assert!(ink >= 30 * scale as usize, "{name} scale {scale}: {ink}");
        }
    }
}

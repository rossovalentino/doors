use super::*;
use crate::{assert_centered, ink_rows, paint};
#[test]
fn partial_editor_selection_matches_one_full_native_glyph_run_on_clean_backdrop() {
    use door_graphics_text::{Clip, Target, TextRun};
    for scale in 1..=3 {
        for dark in [false, true] {
            let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
            let mut files = Files::empty("/users/owner".into(), Controls::default());
            let text = "AVATAR.txt";
            files.edit.set(text.into(), false);
            files.edit.place(1, false);
            files.edit.place(4, true);
            let actual = paint(scale, dark, |v| files.draw_editor(v, 30, 35, 150));
            let mut expected = paint(scale, dark, |_| {});
            // Single-line fields centre cap height, exactly as `View::centered_text_top`.
            let top = {
                let mut one = vec![0; (scale * scale * 4) as usize];
                ui::View::new_scaled(&mut one, scale, scale, &fonts, scale).centered_text_top(
                    35,
                    20,
                    TextStyle::Regular14,
                )
            };
            let full = Clip::new(30, 35, 150, 20);
            let run = TextRun {
                text,
                x: 30,
                top,
                style: TextStyle::Regular14,
                color: door_workspace::palette::Palette::with_dark(dark)
                    .foreground([24, 24, 24, 255]),
            };
            fonts
                .draw(
                    &mut Target::new(
                        &mut expected,
                        742 * scale,
                        498 * scale,
                        (742 * scale * 4) as usize,
                    )
                    .unwrap(),
                    full,
                    run,
                )
                .unwrap();
            let left = fonts
                .measure(&text[..1], TextStyle::Regular14)
                .unwrap()
                .advance_width;
            let right = fonts
                .measure(&text[..4], TextStyle::Regular14)
                .unwrap()
                .advance_width;
            for y in 35 * scale..55 * scale {
                for x in (30 + left) * scale..(30 + right) * scale {
                    let at = ((y * 742 * scale + x) * 4) as usize;
                    expected[at..at + 4].copy_from_slice(&[196, 103, 0, 255]);
                }
            }
            fonts
                .draw(
                    &mut Target::new(
                        &mut expected,
                        742 * scale,
                        498 * scale,
                        (742 * scale * 4) as usize,
                    )
                    .unwrap(),
                    Clip::new((30 + left) as i32, 35, right - left, 20),
                    TextRun {
                        color: [255; 4],
                        ..run
                    },
                )
                .unwrap();
            let differences = actual.iter().zip(&expected).filter(|(a, b)| a != b).count();
            assert_eq!(
                differences, 0,
                "partial selection must preserve context kerning and single coverage; scale{scale} dark{dark}"
            );
        }
    }
}
#[test]
fn actual_explorer_normal_and_rename_rows_keep_the_same_baseline_in_both_views() {
    for scale in 1..=3 {
        for dark in [false, true] {
            for icons in [false, true] {
                let mut files = Files::empty("/users/owner".into(), Controls::default());
                files.configure(742, 498);
                files.overview = false;
                files.view = if icons {
                    ViewMode::Large
                } else {
                    ViewMode::Details
                };
                files.message = "Hg".into();
                let mut name = [0; door_service_protocol::files::NAME_UNITS];
                let text = "Hg.txt";
                for (unit, byte) in name.iter_mut().zip(text.bytes()) {
                    *unit = u16::from(byte);
                }
                files.entries.push(DirEntry {
                    times: Default::default(),
                    cookie: 2,
                    name,
                    name_len: text.len(),
                    info: door_service_client::FileInfo {
                        attributes: 0,
                        bytes: 2,
                        object: 2,
                        version: 1,
                        directory: false,
                    },
                });
                files.selected = Some(0);
                let row = files.item_cell(0).unwrap();
                let label = files.content_layout().label_bounds(row, files.view);
                let text_width = FontAtlas::windows_reference_scaled(scale)
                    .unwrap()
                    .measure(text, TextStyle::Regular14)
                    .unwrap()
                    .advance_width;
                let x = (label.x
                    + if icons {
                        (label.width - text_width) as i32 / 2
                    } else {
                        0
                    }) as u32;
                let text_y = if icons {
                    label.y as u32
                } else {
                    row.y as u32 + (row.height - 20) / 2
                };
                let text_height = 20;
                let pixels = paint(scale, dark, |view| files.draw(view));
                // Cap band only (the "H"): labels centre cap height, not descenders.
                let cap = FontAtlas::windows_reference_scaled(scale)
                    .unwrap()
                    .measure("H", TextStyle::Regular14)
                    .unwrap()
                    .advance_width;
                let normal = ink_rows(&pixels, scale, dark, (x, text_y, cap, text_height));
                assert_centered(normal, scale, text_y, text_height);
                assert_centered(
                    ink_rows(&pixels, scale, dark, (14, 474, cap, 24)),
                    scale,
                    474,
                    24,
                );
                files.rename = Some(ObservedRename {
                    parent: 1,
                    object: 2,
                    version: 1,
                });
                files.editor = 3;
                files.edit.set(text.into(), false);
                let pixels = paint(scale, dark, |view| files.draw(view));
                let editor = files.rename_bounds().unwrap();
                let renamed = ink_rows(
                    &pixels,
                    scale,
                    dark,
                    (editor.x as u32, text_y, cap, text_height),
                );
                assert_eq!(normal, renamed, "F2 must preserve the text baseline");
            }
        }
    }
}

#[test]
fn creating_a_tag_reveals_the_editor_in_short_and_unselected_panes() {
    let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
    for (width, height) in [(700, 470), (900, 568), (1100, 600)] {
        let mut f = Files::empty("/users/1001".into(), Controls::default());
        f.configure(width, height);
        for i in 0..8 {
            f.tags.create(&format!("Tag {i}")).unwrap();
        }
        f.detail_scroll = 3;
        f.begin_tag();
        assert_eq!(f.editor, 4);
        let pane = f.pane().unwrap();
        let create = door_workspace::explorer_tags::TagGrid::new(pane.width, 8).create();
        let y = pane.y + 224 + create.y - (f.detail_scroll * 24) as i32;
        assert!(y >= pane.y + 224 && y + 34 <= height as i32 - 24);
        assert_eq!(f.target(pane.x + create.x + 10, y + 10, &fonts), Some(931));
    }
    let mut f = Files::empty("/users/1001".into(), Controls::default());
    f.configure(1100, 600);
    for i in 0..8 {
        f.tags.create(&format!("Tag {i}")).unwrap();
    }
    f.begin_tag();
    f.edit.set("Draft".into(), false);
    f.configure(700, 470);
    assert_eq!(f.editor, 4);
    assert_eq!(f.edit.text(), "Draft");
    let pane = f.pane().unwrap();
    let create = door_workspace::explorer_tags::TagGrid::new(pane.width, 8).create();
    let y = pane.y + 224 + create.y - (f.detail_scroll * 24) as i32;
    assert!(y >= pane.y + 224 && y + 34 <= 470 - 24);
    assert_eq!(f.target(pane.x + create.x + 10, y + 10, &fonts), Some(931));
    f.configure(490, 358);
    assert_ne!(f.editor, 4);
    f.begin_tag();
    assert_ne!(f.editor, 4);
    assert!(f.message.contains("wider"));
}

#[test]
fn tag_reveal_refuses_scroll_steps_that_cannot_show_the_whole_editor() {
    for count in 0..=8 {
        for height in 436..=460 {
            let mut f = Files::empty("/users/1001".into(), Controls::default());
            f.configure(900, height);
            for i in 0..count {
                f.tags.create(&format!("Tag {i}")).unwrap();
            }
            f.begin_tag();
            if f.editor == 4 {
                let pane = f.pane().unwrap();
                let create =
                    door_workspace::explorer_tags::TagGrid::new(pane.width, count).create();
                let y = pane.y + 224 + create.y - (f.detail_scroll * 24) as i32;
                assert!(
                    y >= pane.y + 224 && y + 34 <= height as i32 - 24,
                    "height={height} count={count} y={y}"
                );
            }
        }
    }
}

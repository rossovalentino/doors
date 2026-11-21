//! Text that does not fit at the default window sizes (896x524 and 717x562
//! logical, with and without the details pane), through the real painter:
//! Details cells end with "..." inside their column, the columns fit the
//! list, and the breadcrumb hides leading crumbs only when they do not fit.
extern crate std;
use super::modern::paint::{MUTED, TEXT};
use super::*;
use door_service_client::FileInfo;
use door_workspace::explorer_layout::SIDEBAR;
use door_workspace::explorer_path::crumb;
use door_workspace::explorer_sort::SortKey;

const SIZES: [(u32, u32); 2] = [(896, 524), (717, 562)];
const WHITE: [u8; 4] = [255; 4];

fn entry(name: &str, object: u64, directory: bool, bytes: u64) -> DirEntry {
    let mut units = [0u16; door_service_protocol::files::NAME_UNITS];
    for (o, u) in units.iter_mut().zip(name.encode_utf16()) {
        *o = u;
    }
    DirEntry {
        times: Default::default(),
        cookie: object,
        name: units,
        name_len: name.encode_utf16().count().min(48),
        info: FileInfo {
            attributes: 0,
            object,
            version: 1,
            bytes,
            directory,
        },
    }
}
fn folder(size: (u32, u32), pane: bool, path: &str) -> Files {
    let mut f = Files::empty("/users/1001".into(), Controls::default());
    f.configure(size.0, size.1);
    let listing = alloc::vec![
        entry("Final folder", 60, true, 0),
        entry("Renamed.txt", 61, false, 1234),
        entry(
            "Quarterly planning notes for review.txt",
            62,
            false,
            52_480_000
        ),
        entry("archive.zip", 63, false, 900),
    ];
    f.navigate_entries(path.into(), listing);
    f.view = ViewMode::Details;
    f.details_visible = pane;
    f
}
fn fonts() -> FontAtlas {
    FontAtlas::windows_reference_scaled(1).unwrap()
}
/// Paints `draw` on a white light-theme surface of the window size.
fn canvas(size: (u32, u32), draw: impl FnOnce(&mut ui::View<'_>)) -> Vec<u8> {
    let fonts = fonts();
    let mut pixels = alloc::vec![0; (size.0 * size.1 * 4) as usize];
    let images = ui::ImageCache::default();
    let mut view =
        ui::View::new_scaled(&mut pixels, size.0, size.1, &fonts, 1).with_images(&images);
    view.hover = Some((-1, -1));
    view.rect(0, 0, size.0, size.1, WHITE);
    draw(&mut view);
    pixels
}
fn region(p: &[u8], width: u32, r: Bounds) -> Vec<[u8; 4]> {
    let mut out = Vec::new();
    for y in r.y..r.y + r.height as i32 {
        for x in r.x..r.x + r.width as i32 {
            let at = ((y as u32 * width + x as u32) * 4) as usize;
            out.push([p[at], p[at + 1], p[at + 2], p[at + 3]]);
        }
    }
    out
}
/// A Details cell as the painter lays it out: text box, full text, color,
/// right-aligned.
struct Cell {
    text: String,
    x: i32,
    width: u32,
    color: [u8; 4],
}
fn cells(f: &Files, entry: &DirEntry, cell: Bounds, measure: &dyn Fn(&str) -> u32) -> [Cell; 4] {
    let layout = f.content_layout();
    let columns = f.detail_columns(layout);
    let name = ui::display_ascii(&Files::name(entry));
    let text = layout.label_bounds(cell, ViewMode::Details);
    let name_end = columns[0].1.x + columns[0].1.width as i32 - 8;
    let kind = previewing::type_name(&name, entry.info.directory);
    let size = if entry.info.directory {
        String::new()
    } else {
        previewing::size(entry.info.bytes)
    };
    let (date, ty, sz) = (columns[1].1, columns[2].1, columns[3].1);
    let size_w = measure(&size).min(sz.width.saturating_sub(16));
    [
        Cell {
            text: name,
            x: text.x,
            width: (name_end - text.x).max(0) as u32,
            color: TEXT,
        },
        Cell {
            text: f.date_text(entry.times.modified),
            x: date.x + 8,
            width: date.width.saturating_sub(16),
            color: MUTED,
        },
        Cell {
            text: kind,
            x: ty.x + 8,
            width: ty.width.saturating_sub(16),
            color: MUTED,
        },
        Cell {
            text: size,
            x: sz.x + sz.width as i32 - 12 - size_w as i32,
            width: size_w,
            color: MUTED,
        },
    ]
}

#[test]
fn details_cells_end_with_an_ellipsis_inside_their_columns() {
    let fonts = fonts();
    let measure = |s: &str| {
        fonts
            .measure(s, TextStyle::Regular14)
            .map_or(0, |m| m.advance_width)
    };
    let mut cut = 0;
    for size in SIZES {
        for pane in [true, false] {
            let f = folder(size, pane, "/users/1001/notes");
            let layout = f.content_layout();
            let columns = f.detail_columns(layout);
            let available = layout.width - 12 - (SIDEBAR as u32 + 12);
            let total: u32 = columns.iter().map(|(_, r)| r.width).sum();
            assert!(
                total <= available,
                "{size:?} pane {pane}: columns {total} > list {available}"
            );
            assert!(
                columns.iter().all(|(_, r)| r.width >= 48),
                "{size:?} pane {pane}: {columns:?}"
            );
            // Name gives way first: the other columns keep their widths
            // while Name can still shrink.
            if available >= 96 + DEFAULT_COLUMNS[1] + DEFAULT_COLUMNS[2] + DEFAULT_COLUMNS[3] {
                assert_eq!(
                    [columns[1].1.width, columns[2].1.width, columns[3].1.width],
                    [DEFAULT_COLUMNS[1], DEFAULT_COLUMNS[2], DEFAULT_COLUMNS[3]]
                );
            }
            let painted = paint_files(&f);
            save(
                &std::format!(
                    "fit-{}x{}-{}",
                    size.0,
                    size.1,
                    if pane { "pane" } else { "list" }
                ),
                &painted,
                size,
            );
            for (row, (index, entry)) in f.shown().enumerate() {
                let cell = f.item_cell(row).unwrap();
                for (column, c) in cells(&f, entry, cell, &measure).into_iter().enumerate() {
                    let shown = door_workspace::explorer_path::ellipsize(&c.text, c.width, measure);
                    assert!(measure(&shown) <= c.width, "{size:?} {index}/{column}");
                    if measure(&c.text) > c.width {
                        assert!(shown.ends_with("..."), "{size:?} pane {pane}: {shown:?}");
                        cut += 1;
                    } else {
                        assert_eq!(shown, c.text);
                    }
                    // The painter drew exactly that text in that box.
                    let b = Bounds {
                        x: c.x,
                        y: cell.y,
                        width: c.width,
                        height: cell.height,
                    };
                    if b.width == 0 {
                        continue;
                    }
                    let expected = canvas(size, |v| {
                        v.text_clip_centered(
                            c.x,
                            (cell.y, cell.height),
                            c.width,
                            &shown,
                            TextStyle::Regular14,
                            c.color,
                        )
                    });
                    assert!(
                        region(&painted, size.0, b) == region(&expected, size.0, b),
                        "{size:?} pane {pane}: row {row} column {column} {shown:?}"
                    );
                    if !shown.is_empty() {
                        assert!(
                            region(&painted, size.0, b).iter().any(|p| *p != WHITE),
                            "{size:?}: no ink for {shown:?}"
                        );
                    }
                }
                // No ink between a cell and the next column's text.
                let cs = cells(&f, entry, cell, &measure);
                let gaps = [
                    (cs[0].x + cs[0].width as i32, columns[1].1.x + 8),
                    (cs[1].x + cs[1].width as i32, columns[2].1.x + 8),
                    (cs[2].x + cs[2].width as i32, columns[3].1.x + 4),
                ];
                for (from, to) in gaps {
                    if to > from {
                        let b = Bounds {
                            x: from,
                            y: cell.y + 4,
                            width: (to - from) as u32,
                            height: cell.height - 8,
                        };
                        assert!(
                            region(&painted, size.0, b).iter().all(|p| *p == WHITE),
                            "{size:?} pane {pane}: ink in {b:?}"
                        );
                    }
                }
            }
            // Column headers fit too.
            for (key, r) in columns {
                let shown = door_workspace::explorer_path::ellipsize(
                    key.label(),
                    r.width
                        .saturating_sub(if key == SortKey::Name { 38 } else { 16 }),
                    measure,
                );
                assert!(
                    !shown.is_empty() || r.width < 40,
                    "{size:?}: {key:?} header vanished"
                );
            }
        }
    }
    assert!(cut > 0, "the long name is cut at every size");
    // The reported case: at 896x524 with the pane open "Text document" is
    // readable in full.
    let f = folder((896, 524), true, "/users/1001/notes");
    let ty = f.detail_columns(f.content_layout())[2].1;
    assert!(measure("Text document") + 16 <= ty.width, "{ty:?}");
}
fn paint_files(f: &Files) -> Vec<u8> {
    let fonts = fonts();
    let (w, h) = (f.layout.width, f.layout.height);
    let mut pixels = alloc::vec![0; (w * h * 4) as usize];
    let images = ui::ImageCache::default();
    let mut view = ui::View::new_scaled(&mut pixels, w, h, &fonts, 1).with_images(&images);
    view.hover = Some((-1, -1));
    f.draw(&mut view);
    pixels
}

#[test]
fn narrow_lists_shrink_name_first_then_type_and_size_proportionally() {
    let f = folder((896, 524), false, "/users/1001");
    let [n, d, t, s] = DEFAULT_COLUMNS;
    assert_eq!(f.fitted_columns(n + d + t + s + 50), DEFAULT_COLUMNS);
    assert_eq!(f.fitted_columns(d + t + s + 150), [150, d, t, s]);
    assert_eq!(f.fitted_columns(96 + d + t + s), [96, d, t, s]);
    // Past Name's minimum, the others give in proportion to their spare.
    assert_eq!(f.fitted_columns(80 + d + t + s), [80, d, t, s]);
    let w = f.fitted_columns(80 + d + t + s - 60);
    assert_eq!(w[0], 80);
    assert_eq!(w.iter().sum::<u32>(), 80 + d + t + s - 60);
    assert!(t - w[2] > s - w[3], "Type has more to give: {w:?}");
    // Narrower than every minimum: the minimums scale.
    let w = f.fitted_columns(116);
    assert_eq!(w.iter().sum::<u32>(), 116);
    assert!(w[0] > w[2] && w[2] >= w[3] - 1, "{w:?}");
}

/// Crumb cost: label plate and the chevron after it.
fn cost(w: u32) -> u32 {
    w.min(crumb::MAX_TEXT) + crumb::PAD + crumb::PAD_END + crumb::CHEVRON
}
#[test]
fn breadcrumbs_hide_leading_crumbs_only_when_they_do_not_fit() {
    let fonts = fonts();
    let measure = |s: &str| {
        fonts
            .measure(s, TextStyle::Regular14)
            .map_or(0, |m| m.advance_width)
    };
    for size in SIZES {
        for pane in [true, false] {
            for path in [
                "/users/1001/notes",
                "/users/1001",
                "/users",
                "/users/1001/notes/Final folder/drafts/2024",
            ] {
                let f = folder(size, pane, path);
                let address = f.layout.address();
                let (chain, l) = f.crumb_geometry(&measure);
                let widths: Vec<u32> = chain
                    .iter()
                    .map(|(s, _)| measure(&ui::display_ascii(s)))
                    .collect();
                let total = |from: usize| widths[from..].iter().map(|w| cost(*w)).sum::<u32>();
                // Room after the location glyph and its root chevron.
                let room = (address.x + address.width as i32
                    - crumb::INSET
                    - (address.x + crumb::INSET + (crumb::ICON + crumb::CHEVRON) as i32))
                    as u32;
                let overflow = cost(measure("..."));
                let what = std::format!(
                    "{size:?} pane {pane} {path}: room {room}, widths {widths:?}, hidden {}",
                    l.hidden
                );
                if total(0) <= room {
                    assert_eq!(l.hidden, 0, "{what}");
                    assert!(l.overflow.is_none(), "{what}");
                } else {
                    assert!(l.hidden > 0 && l.overflow.is_some(), "{what}");
                    // Minimal: showing one more leading crumb would not fit.
                    assert!(overflow + total(l.hidden - 1) > room, "{what}");
                }
                assert_eq!(l.items.len(), chain.len() - l.hidden, "{what}");
                for item in &l.items {
                    assert!(
                        item.chevron.x + item.chevron.width as i32
                            <= address.x + address.width as i32,
                        "{what}"
                    );
                }
            }
        }
    }
    // At 896 wide C:\Users\1001 fits whole: This PC and Local Disk (C:) show.
    let f = folder((896, 524), true, "/users/1001");
    let (chain, l) = f.crumb_geometry(&measure);
    let labels: Vec<&str> = l.items.iter().map(|i| chain[i.index].0.as_str()).collect();
    assert_eq!(labels, ["This PC", "Local Disk (C:)", "Users", "1001"]);
    // The painter shows the first crumb's text where the layout put it.
    let painted = paint_files(&f);
    let first = l.items[0].text;
    let inner = Bounds {
        x: first.x + crumb::PAD as i32,
        y: first.y + 6,
        width: l.items[0].label_width,
        height: first.height - 12,
    };
    assert!(
        region(&painted, 896, inner).iter().any(|p| p[0] < 128),
        "This PC ink"
    );
}
/// Review capture under target/explorer-review.
fn save(name: &str, pixels: &[u8], size: (u32, u32)) {
    let mut ppm = std::format!("P6\n{} {}\n255\n", size.0, size.1).into_bytes();
    for p in pixels.chunks_exact(4) {
        ppm.extend_from_slice(&[p[2], p[1], p[0]]);
    }
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/explorer-review");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(std::format!("{name}.ppm")), ppm).unwrap();
}

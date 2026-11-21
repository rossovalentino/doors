//! Guest scenario points for File Explorer, derived from the real layout and
//! painter and kept as constants in tools/xtask (which cannot link the app):
//! `desktop/lifecycle.rs` and `desktop/explorer.rs` (wide profile 1280x720 at
//! 1x, 48 px bottom taskbar; Explorer is maximized with Win+Up, so its client
//! is 1280x672 at the screen origin and window points are screen points) and
//! `desktop/multitask.rs` (a normal Explorer window, 896x524 at (192, 74),
//! 70% x 78% of the work area: `graphics::initial_window`).
//! Every new window opens to Home, as File Explorer does. Change a point here
//! and in the scenario together.
extern crate std;
use super::home::{HEADER, QUICK, RECENT};
use super::*;
use address::CRUMB_TEXT;
use door_service_client::FileInfo;
use door_workspace::context_menu as cm;
use door_workspace::explorer_tree::Node as TreeNode;

const WHITE: [u8; 3] = [255; 3];
/// Selection fill (`paint::SELECTED`, BGRA [250, 233, 213]).
const SELECTED: [u8; 3] = [213, 233, 250];
/// Light menu panel (opaque fallback of the acrylic).
const MENU: [u8; 3] = [250; 3];
/// Light navigation pane background.
const RAIL: [u8; 3] = [233, 240, 246];

// lifecycle.rs, maximized.
const QUICK_HEADER: (i32, i32) = (240, 160);
const QUICK_TILE_PROBE: (i32, i32) = (205, 220);
const HOME_ROW_PROBE: (i32, i32) = (130, 159);
const THIS_PC_ROW: (i32, i32) = (100, 447);
const THIS_PC_ROW_PROBE: (i32, i32) = (130, 447);
const DRIVES_HEADER: (i32, i32) = (262, 152);
const DRIVE_ICON_PROBE: (i32, i32) = (212, 194);
/// Flat top face of the drive icon.
const DRIVE_ICON: [u8; 3] = [225, 227, 230];
const BACK: (i32, i32) = (32, 64);
const HISTORY_HOME_ROW: (i32, i32) = (148, 203);
const HISTORY_MENU_PROBE: (i32, i32) = (272, 203);
// explorer.rs, maximized.
const USER_CRUMB: (i32, i32) = (611, 64);
const FOOTER_ICONS: (i32, i32) = (1263, 659);
const FOOTER_DETAILS: (i32, i32) = (1237, 659);
const VIEW_PROBE: (i32, i32) = (800, 169);
/// The Pictures known-folder icon (sixth item) at the probe: its back
/// plate's tab (the 26H2 art is a gradient with no flat 5x5 area).
const PICTURES_ICON: [u8; 3] = [19, 116, 207];
// multitask.rs: a window point of the normal window (896x524 at its initial
// placement; the scenario adds the origin from `window_points`).
const MULTITASK_PROBE: (i32, i32) = (630, 436);

fn entry(name: &str, object: u64, directory: bool) -> DirEntry {
    let mut units = [0u16; door_service_protocol::files::NAME_UNITS];
    for (o, u) in units.iter_mut().zip(name.encode_utf16()) {
        *o = u;
    }
    DirEntry {
        times: Default::default(),
        cookie: object,
        name: units,
        name_len: name.encode_utf16().count(),
        info: FileInfo {
            attributes: 0,
            object,
            version: 1,
            bytes: if directory { 0 } else { 2048 },
            directory,
        },
    }
}
/// Home of a fresh profile as `show_home` builds it: the six known folders,
/// no favorites, and `recent` files opened elsewhere.
fn home(width: u32, height: u32, recent: &[&str]) -> Files {
    let mut f = Files::empty("/users/1001".into(), Controls::default());
    f.configure(width, height);
    f.history = alloc::vec![HOME_PAGE.into()];
    f.install("/users/1001".into(), Vec::new());
    f.home_view = true;
    for (i, (path, name, _)) in f.quick_access().into_iter().enumerate() {
        f.entries.push(entry(&name, 10 + i as u64, true));
        f.result_paths.push(path);
        f.home_sections.push(QUICK);
    }
    for (i, path) in recent.iter().enumerate() {
        f.recent.record(path, door_workspace::recent::Kind::File);
        f.entries.push(entry(
            path.rsplit('/').next().unwrap(),
            30 + i as u64,
            false,
        ));
        f.result_paths.push((*path).into());
        f.home_sections.push(RECENT);
    }
    f.view = ViewMode::Details;
    f
}
fn paint(f: &Files, hover: (i32, i32)) -> Vec<u8> {
    let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
    let (w, h) = (f.layout.width, f.layout.height);
    let mut pixels = alloc::vec![0; (w * h * 4) as usize];
    let images = ui::ImageCache::default();
    let mut view = ui::View::new_scaled(&mut pixels, w, h, &fonts, 1).with_images(&images);
    view.hover = Some(hover);
    f.draw(&mut view);
    pixels
}
fn rgb(p: &[u8], f: &Files, (x, y): (i32, i32)) -> [u8; 3] {
    let at = ((y as u32 * f.layout.width + x as u32) * 4) as usize;
    [p[at + 2], p[at + 1], p[at]]
}
/// The color of a 5x5 block around a probe, which tolerates a pixel of drift.
fn flat(p: &[u8], f: &Files, (x, y): (i32, i32)) -> Option<[u8; 3]> {
    let c = rgb(p, f, (x, y));
    (-2..=2)
        .all(|dy| (-2..=2).all(|dx| rgb(p, f, (x + dx, y + dy)) == c))
        .then_some(c)
}

#[test]
fn lifecycle_home_this_pc_and_history_points_follow_the_explorer_layout() {
    let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
    let target = |f: &Files, (x, y): (i32, i32)| f.modern_target(x, y, &fonts);
    // Home, maximized: the Quick access header collapses its section; the
    // first tile's folder icon disappears and Home stays selected.
    let mut f = home(1280, 672, &[]);
    assert_eq!(target(&f, QUICK_HEADER), Some(HEADER + u32::from(QUICK)));
    // Derived from the layout: the header band, and the first tile's 48 px
    // folder icon (8 px into the cell, centred) between the Favorites
    // header and its empty state once Quick access collapses.
    let l = f.home_layout();
    let tile = l.cells[0].unwrap();
    assert!(l.headers[usize::from(QUICK)].contains(QUICK_HEADER.0, QUICK_HEADER.1));
    assert_eq!(QUICK_TILE_PROBE.0, tile.x + 8 + 24);
    assert!((tile.y + 4..tile.y + 52).contains(&QUICK_TILE_PROBE.1));
    let before = paint(&f, QUICK_HEADER);
    assert_ne!(
        rgb(&before, &f, QUICK_TILE_PROBE),
        WHITE,
        "first Quick access tile icon"
    );
    f.toggle_home_section(QUICK);
    let after = paint(&f, QUICK_HEADER);
    assert_eq!(rgb(&after, &f, QUICK_TILE_PROBE), WHITE);
    assert_eq!(rgb(&after, &f, HOME_ROW_PROBE), SELECTED, "Home row");
    assert_eq!(
        target(&f, QUICK_HEADER),
        Some(HEADER + u32::from(QUICK)),
        "same header expands it"
    );
    f.toggle_home_section(QUICK);
    assert!(!f.home_collapsed(QUICK));
    // Navigation pane: This PC.
    let rows = f.rail_rows();
    let pc = rows
        .iter()
        .position(|r| r.node == TreeNode::ThisPc)
        .unwrap();
    let bounds = f.rail_row_bounds(&rows);
    let home_row = rows.iter().position(|r| r.node == TreeNode::Home).unwrap();
    assert!(bounds[home_row].contains(HOME_ROW_PROBE.0, HOME_ROW_PROBE.1));
    for (x, y) in [THIS_PC_ROW, THIS_PC_ROW_PROBE] {
        assert!(bounds[pc].contains(x, y));
        assert_eq!(y, bounds[pc].y + bounds[pc].height as i32 / 2);
    }
    assert_eq!(target(&f, THIS_PC_ROW), Some(1100 + pc as u32));
    f.rail_key = Some(rows[pc].key.clone());
    f.show_this_pc(Vec::new());
    let shown = paint(&f, THIS_PC_ROW);
    assert_eq!(rgb(&shown, &f, THIS_PC_ROW_PROBE), SELECTED, "This PC row");
    assert_eq!(
        rgb(&shown, &f, HOME_ROW_PROBE),
        RAIL,
        "Home row no longer selected"
    );
    assert_eq!(
        flat(&shown, &f, DRIVE_ICON_PROBE),
        Some(DRIVE_ICON),
        "drive tile icon"
    );
    // "Devices and drives" collapses and expands again.
    assert!(f.drives_header().contains(DRIVES_HEADER.0, DRIVES_HEADER.1));
    assert_eq!(target(&f, DRIVES_HEADER), Some(502));
    f.drives_expanded = false;
    assert_eq!(rgb(&paint(&f, DRIVES_HEADER), &f, DRIVE_ICON_PROBE), WHITE);
    assert_eq!(target(&f, DRIVES_HEADER), Some(502));
    f.drives_expanded = true;
    // Ctrl+L Documents, Ctrl+L Downloads, then right-click Back: the history
    // flyout lists newest first; its last row returns to Home (index 0).
    f.navigate_entries("/users/1001/Documents".into(), Vec::new());
    f.navigate_entries("/users/1001/Downloads".into(), Vec::new());
    assert_eq!(
        f.history,
        [
            HOME_PAGE,
            THIS_PC,
            "/users/1001/Documents",
            "/users/1001/Downloads"
        ]
    );
    // Back and the breadcrumbs share the address row's centre line.
    let middle = door_workspace::explorer_layout::ADDRESS_TOP
        + door_workspace::explorer_layout::address_height() as i32 / 2;
    assert_eq!((BACK.1, USER_CRUMB.1), (middle, middle));
    assert_eq!(target(&f, BACK), Some(20), "Back button");
    let closed = paint(&f, BACK);
    f.context(BACK.0, BACK.1, 1280, 672);
    assert!(matches!(f.menu_for, MenuFor::History));
    let open = paint(&f, BACK);
    assert_eq!(flat(&open, &f, HISTORY_MENU_PROBE), Some(MENU));
    assert_ne!(rgb(&closed, &f, HISTORY_MENU_PROBE), MENU);
    let menu = f.xmenu.as_mut().unwrap();
    let labels: Vec<_> = menu.menu.entries.iter().map(|e| e.label.as_str()).collect();
    assert_eq!(labels[2..], ["This PC", "Home"]);
    assert_eq!(
        menu.click(HISTORY_HOME_ROW.0, HISTORY_HOME_ROW.1),
        cm::Outcome::Invoke(cm::Action::Pick(0), cm::Source::Root, 3)
    );
}

#[test]
fn explorer_scenario_breadcrumb_footer_and_view_points_follow_the_layout() {
    let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
    let mut f = home(1280, 672, &[]);
    // "C:\Users\1001\notes": This PC > Local Disk (C:) > Users > 1001 > notes.
    f.navigate_entries("/users/1001/notes".into(), Vec::new());
    let measure = |s: &str| {
        fonts
            .measure(s, TextStyle::Regular14)
            .map_or(0, |m| m.advance_width)
    };
    let (chain, l) = f.crumb_geometry(&measure);
    let user = l
        .items
        .iter()
        .position(|i| chain[i.index].1 == "/users/1001")
        .unwrap();
    assert!(l.items[user].text.contains(USER_CRUMB.0, USER_CRUMB.1));
    assert_eq!(
        f.modern_target(USER_CRUMB.0, USER_CRUMB.1, &fonts),
        Some(CRUMB_TEXT + user as u32)
    );
    // The profile folder of a fresh guest disk (Explorer's own files hidden).
    let names = [
        "Desktop",
        "Documents",
        "Downloads",
        "Music",
        "notes",
        "Pictures",
        "Videos",
    ];
    let listing = names
        .iter()
        .enumerate()
        .map(|(i, n)| entry(n, 40 + i as u64, true))
        .collect();
    f.navigate_entries("/users/1001".into(), listing);
    let centre = |b: Bounds| (b.x + b.width as i32 / 2, b.y + b.height as i32 / 2);
    assert_eq!(FOOTER_ICONS, centre(f.layout.status_view(true)));
    assert_eq!(FOOTER_DETAILS, centre(f.layout.status_view(false)));
    assert_eq!(
        f.modern_target(FOOTER_ICONS.0, FOOTER_ICONS.1, &fonts),
        Some(41)
    );
    assert_eq!(
        f.modern_target(FOOTER_DETAILS.0, FOOTER_DETAILS.1, &fonts),
        Some(40)
    );
    // Large icons: the Pictures icon covers the probe (the block around it
    // is exactly that icon's raster); in Details the same point is blank
    // space in the first row's empty Size column.
    f.set_view(ViewMode::Large);
    let icon = f
        .content_layout()
        .icon_bounds(f.item_cell(5).unwrap(), ViewMode::Large);
    assert!(icon.contains(VIEW_PROBE.0, VIEW_PROBE.1));
    let large = paint(&f, FOOTER_ICONS);
    let mut alone = alloc::vec![255; large.len()];
    let images = ui::ImageCache::default();
    let mut view = ui::View::new_scaled(&mut alone, f.layout.width, f.layout.height, &fonts, 1)
        .with_images(&images);
    view.sized_icon(icon.x, icon.y, icon.width, Files::folder_icon("Pictures"));
    assert_eq!(rgb(&large, &f, VIEW_PROBE), PICTURES_ICON);
    for (dx, dy) in (-2..=2).flat_map(|dy| (-2..=2).map(move |dx| (dx, dy))) {
        let p = (VIEW_PROBE.0 + dx, VIEW_PROBE.1 + dy);
        assert_eq!(rgb(&large, &f, p), rgb(&alone, &f, p), "{p:?}");
    }
    f.set_view(ViewMode::Details);
    assert_eq!(rgb(&paint(&f, FOOTER_DETAILS), &f, VIEW_PROBE), WHITE);
}

#[test]
fn multitask_probe_is_blank_home_content_with_or_without_recent_files() {
    // Shell saves Notes before Win+E, so Home may already list it.
    for recent in [&[][..], &["/users/1001/notes/desktop.txt"][..]] {
        let f = home(896, 524, recent);
        assert_eq!(
            f.home_layout().columns.len(),
            usize::from(!recent.is_empty())
        );
        // The pointer is still at Task View's desktop thumbnail (200, 598),
        // window (8, 524): below the window.
        assert_eq!(
            flat(&paint(&f, (8, 524)), &f, MULTITASK_PROBE),
            Some(WHITE),
            "{recent:?}"
        );
    }
}

// file_mutations.rs and file_deletion.rs (wide 1280x720 at 1x; accelerated
// 3840x2160 is the same logical desktop at density 3): Explorer stays at its
// initial placement, 896x524 at (192, 74), so a screen point is a window
// point plus that origin (`window_points::origin`). Selecting an item with the keyboard inspects it,
// which opens the details pane.
const FULL: (u32, u32) = (896, 524);
/// Details still shows one row (the revealed editor); Large shows none.
const SMALL: (u32, u32) = (400, 240);
/// One Large icon slot.
const TALL: (u32, u32) = (400, 340);
/// The resize minimum width: rows remain but the name editor cannot fit.
const NARROW: (u32, u32) = (240, 524);
/// Light inline-rename border (top edge, left part) in Details row 0.
const EDITOR_TOP: (i32, i32) = (240, 170);
/// The same edge for the first Large icon slot.
const LARGE_EDITOR_TOP: (i32, i32) = (214, 246);
const EDITOR_BORDER: [u8; 3] = [160; 3];
/// Command bar Rename.
const RENAME_COMMAND: (i32, i32) = (274, 111);
/// Rename in the inline icon row of the item's context menu (Shift+F10).
const RENAME_ICON: (i32, i32) = (329, 180);
/// Dark editor: the caret at the start of the name and the editor fill.
const CARET: (i32, i32) = (199, 182);
const CARET_LEFT: (i32, i32) = (198, 182);
// file_deletion.rs: the removal dialog over the whole client.
const CANCEL_EDGE: (i32, i32) = (452, 335);
const DELETE_PRESS: (i32, i32) = (346, 338);
const DELETE_FILL: (i32, i32) = (256, 325);
const DIALOG_BLANK: (i32, i32) = (621, 275);
/// Dialog client at the minimum: 240 wide, 178 high (the client size the
/// scenario always used); ink bands of the question and of item + warning.
const MINIMUM: (u32, u32) = (240, 178);
const INK_BANDS: [(i32, i32); 2] = [(20, 44), (48, 106)];

fn notes(size: (u32, u32), items: &[(&str, bool)]) -> Files {
    let mut f = Files::empty("/users/1001".into(), Controls::default());
    f.configure(size.0, size.1);
    let listing = items
        .iter()
        .enumerate()
        .map(|(i, (n, d))| entry(n, 60 + i as u64, *d))
        .collect();
    f.navigate_entries("/users/1001/notes".into(), listing);
    f
}
/// Keyboard selection (Home/End) inspects the item, which opens the pane.
fn select(f: &mut Files, index: usize) {
    f.selected = Some(index);
    f.details_visible = true;
}
/// `begin_rename` without Storage; false is "RENAME REJECT not_visible".
fn rename(f: &mut Files) -> bool {
    let entry = f.entries[f.selected.unwrap()];
    if !f.reveal_for_rename(entry.info.object) {
        return false;
    }
    let name = door_workspace::file_name::editable_name(&entry.name[..entry.name_len]).unwrap();
    f.start_rename(entry, name, 7);
    true
}
fn paint_as(f: &Files, hover: (i32, i32), pressed: Option<(i32, i32)>, dark: bool) -> Vec<u8> {
    let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
    let (w, h) = (f.layout.width, f.layout.height);
    let mut pixels = alloc::vec![0; (w * h * 4) as usize];
    let images = ui::ImageCache::default();
    let mut view = ui::View::new_scaled(&mut pixels, w, h, &fonts, 1).with_images(&images);
    view.hover = Some(hover);
    view.pressed = pressed;
    view.palette.dark = dark;
    f.draw(&mut view);
    pixels
}
/// Horizontal run of 9 pixels centred on a probe (a 1 px border line).
fn border(p: &[u8], f: &Files, (x, y): (i32, i32)) -> Option<[u8; 3]> {
    let c = rgb(p, f, (x, y));
    (-4..=4).all(|dx| rgb(p, f, (x + dx, y)) == c).then_some(c)
}

#[test]
fn file_mutations_rename_points_follow_the_explorer_layout() {
    let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
    // Notes saved desktop.txt; Home selects it; F2.
    let mut f = notes(FULL, &[("desktop.txt", false)]);
    select(&mut f, 0);
    assert!(rename(&mut f));
    assert!(
        f.pane().is_some(),
        "the pane narrows the list, not the editor's left edge"
    );
    // The editor's 1 px border runs 2 px above its line box.
    assert_eq!(EDITOR_TOP.1, f.rename_bounds().unwrap().y - 2);
    assert_eq!(
        border(&paint(&f, (0, 0)), &f, EDITOR_TOP),
        Some(EDITOR_BORDER)
    );
    // New folder 1 selected (Home): the command bar's Rename.
    let mut f = notes(FULL, &[("New folder 1", true), ("Renamed.txt", false)]);
    select(&mut f, 0);
    let (top, height) = door_workspace::explorer_layout::command_button();
    assert_eq!(RENAME_COMMAND.1, top + height as i32 / 2);
    assert_eq!(
        f.modern_target(RENAME_COMMAND.0, RENAME_COMMAND.1, &fonts),
        Some(33)
    );
    // Archives selected: Shift+F10, then the inline Rename icon.
    let mut f = notes(FULL, &[("Archives", true), ("Renamed.txt", false)]);
    select(&mut f, 0);
    f.keyboard_context(FULL.0, FULL.1);
    let menu = f.xmenu.as_mut().unwrap();
    assert!(
        menu.root()
            .icons
            .iter()
            .any(|b| (b.x + b.width as i32 / 2, b.y + b.height as i32 / 2) == RENAME_ICON)
    );
    assert!(matches!(
        menu.click(RENAME_ICON.0, RENAME_ICON.1),
        cm::Outcome::Invoke(cm::Action::Rename, cm::Source::Icon, _)
    ));
    // End selects Renamed.txt; F2; the corner drag to SMALL keeps the editor
    // and reveals its row (one Details row), with the pane gone.
    let items = [("Final folder", true), ("Renamed.txt", false)];
    let mut f = notes(FULL, &items);
    select(&mut f, 1);
    assert!(rename(&mut f));
    f.configure(SMALL.0, SMALL.1);
    assert!(f.rename.is_some() && f.offset == 1 && f.pane().is_none());
    assert_eq!(f.content_layout().capacity(ViewMode::Details), 1);
    assert_eq!(
        border(&paint(&f, (0, 0)), &f, EDITOR_TOP),
        Some(EDITOR_BORDER)
    );
    // Esc; TALL; Ctrl+Shift+2: one Large slot shows the second item; F2.
    f.editor = 0;
    f.rename = None;
    f.configure(TALL.0, TALL.1);
    f.set_view(ViewMode::Large);
    assert_eq!(f.content_layout().capacity(ViewMode::Large), 1);
    assert!(rename(&mut f));
    assert_eq!(f.offset, 1);
    assert_eq!(LARGE_EDITOR_TOP.1, f.rename_bounds().unwrap().y - 2);
    assert_eq!(
        border(&paint(&f, (0, 0)), &f, LARGE_EDITOR_TOP),
        Some(EDITOR_BORDER)
    );
    // Back to SMALL: no Large slot, so the rename is cancelled and F2 refused.
    f.configure(SMALL.0, SMALL.1);
    assert!(f.rename.is_none() && f.editor == 0, "RENAME CANCEL layout");
    assert!(!rename(&mut f), "RENAME REJECT not_visible");
    // FULL, Details, Home, F2, then NARROW: rows remain but no editor fits.
    f.configure(FULL.0, FULL.1);
    f.set_view(ViewMode::Details);
    select(&mut f, 0);
    assert!(rename(&mut f));
    f.configure(NARROW.0, NARROW.1);
    assert!(f.content_layout().capacity(ViewMode::Details) > 0);
    assert!(f.rename.is_none(), "RENAME CANCEL layout");
    assert!(!rename(&mut f), "RENAME REJECT not_visible");
    // Dark: F2 on Final folder at FULL, Home puts the caret first.
    f.configure(FULL.0, FULL.1);
    assert!(rename(&mut f));
    f.edit.place(0, false);
    let r = f.rename_bounds().unwrap();
    assert_eq!((CARET, CARET_LEFT), ((r.x, r.y + 10), (r.x - 1, r.y + 10)));
    let dark = paint_as(&f, (0, 0), None, true);
    assert_eq!(rgb(&dark, &f, CARET), [170; 3]);
    assert_eq!(rgb(&dark, &f, CARET_LEFT), [25; 3]);
}

#[test]
fn file_deletion_dialog_points_follow_the_explorer_layout() {
    let open = |size: (u32, u32)| {
        let mut f = notes(size, &[("desktop.txt", false)]);
        select(&mut f, 0);
        assert!(f.removal.begin(Observed {
            parent: 7,
            object: 60,
            version: 1
        }));
        f.removal_name = "desktop.txt".into();
        f
    };
    let f = open(FULL);
    let l = f.dialog_layout();
    assert!(l.buttons[0].contains(DELETE_PRESS.0, DELETE_PRESS.1));
    assert_eq!(l.buttons[1].x, CANCEL_EDGE.0);
    // Light: Cancel holds the initial keyboard focus (dark focus border).
    let light = paint_as(&f, (0, 0), None, false);
    assert_eq!(rgb(&light, &f, CANCEL_EDGE), [32; 3]);
    // A held press on Delete paints its pressed accent.
    let mut held = open(FULL);
    assert!(held.pointer_down(DELETE_PRESS.0, DELETE_PRESS.1));
    let pressed = paint_as(&held, DELETE_PRESS, Some(DELETE_PRESS), false);
    assert_eq!(flat(&pressed, &held, DELETE_FILL), Some([0, 80, 160]));
    // Dark: blank panel fill and the light focus border.
    let dark = paint_as(&f, (0, 0), None, true);
    assert_eq!(
        flat(&dark, &f, DIALOG_BLANK),
        Some([25; 3]),
        "Explorer dark surface"
    );
    assert_eq!(rgb(&dark, &f, CANCEL_EDGE), [240; 3]);
    // At the minimum client the question, item and warning keep their ink.
    let small = open(MINIMUM);
    let p = paint_as(&small, (-1, -1), None, false);
    for (top, bottom) in INK_BANDS {
        let ink = (top..bottom)
            .flat_map(|y| (16..224).map(move |x| (x, y)))
            .filter(|&(x, y)| rgb(&p, &small, (x, y)).iter().all(|v| *v < 120))
            .count();
        assert!(ink >= 30, "band {top}..{bottom}: {ink}");
    }
}

/// file_mutations.rs `dark_and_classic`: with "Arrange primary actions inline"
/// off (controls 32), Shift+F10 on the selected folder and Up from the first
/// row wraps to the tail; the scenario's Up count must land on Rename.
#[test]
fn inline_off_menu_reaches_rename_with_the_scenario_keys() {
    let controls = Controls::from_bits(32).unwrap();
    let mut f = Files::empty("/users/1001".into(), controls);
    f.configure(FULL.0, FULL.1);
    let listing = [("Final folder", true), ("Renamed.txt", false)]
        .iter()
        .enumerate()
        .map(|(i, (n, d))| entry(n, 60 + i as u64, *d))
        .collect();
    f.navigate_entries("/users/1001/notes".into(), listing);
    select(&mut f, 0);
    f.keyboard_context(FULL.0, FULL.1);
    let menu = f.xmenu.as_mut().unwrap();
    let labels: Vec<_> = menu
        .menu
        .entries
        .iter()
        .map(|e| alloc::string::String::from(e.label.as_str()))
        .collect();
    for _ in 0..RENAME_UPS {
        menu.key(0x148);
    }
    let cm::Focus::Row(i) = menu.focus else {
        panic!("{labels:?}")
    };
    assert_eq!(
        menu.menu.entries[i].action,
        cm::Action::Rename,
        "{labels:?}"
    );
    assert!(matches!(
        menu.key(28),
        cm::Outcome::Invoke(cm::Action::Rename, _, _)
    ));
}
/// Up presses from Shift+F10 to Rename in the inline-off folder menu.
const RENAME_UPS: usize = 4;

/// New windows open at 70% x 78% of the work area (`initial_window`):
/// Explorer's 1000x640 request becomes 896x524 at 1280x720 (and the 2x/3x
/// modes of the same logical desktop), 717x562 at 1024x768 and 896x587 at
/// 1280x800 (1920x1080 keeps 1000x640). Home,
/// This PC and a folder in Details with the details pane open all lay out:
/// the pane keeps a usable list beside it and every view keeps visible rows.
#[test]
fn explorer_lays_out_at_the_default_window_sizes() {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/explorer-review");
    std::fs::create_dir_all(&directory).unwrap();
    for (output, size) in [
        ((1280, 720), (896, 524)),
        ((1024, 768), (717, 562)),
        ((1280, 800), (896, 587)),
        ((1920, 1080), (1000, 640)),
    ] {
        let (_, _, w, h) = door_graphics_core::initial_window(output, (1000, 640), 0, None);
        assert_eq!((w, h), size, "{output:?}");
        let save = |name: &str, f: &Files, p: &[u8]| {
            let mut ppm =
                alloc::format!("P6\n{} {}\n255\n", f.layout.width, f.layout.height).into_bytes();
            ppm.extend(p.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]));
            std::fs::write(
                directory.join(alloc::format!("default-{w}x{h}-{name}.ppm")),
                ppm,
            )
            .unwrap();
        };
        let f = home(w, h, &["/users/1001/notes/desktop.txt"]);
        save("home", &f, &paint(&f, (0, 0)));
        let mut f = home(w, h, &[]);
        f.show_this_pc(Vec::new());
        save("this-pc", &f, &paint(&f, (0, 0)));
        let names = [
            "Desktop",
            "Documents",
            "Downloads",
            "Music",
            "notes",
            "Pictures",
            "Videos",
        ];
        let items: Vec<_> = names
            .iter()
            .map(|n| (*n, true))
            .chain([("desktop.txt", false)])
            .collect();
        let mut f = notes((w, h), &items);
        select(&mut f, 7);
        let pane = f.pane().expect("details pane at the default size");
        let list = f.content_layout();
        assert!(
            list.width >= 320,
            "{w}x{h}: list {} beside the pane",
            list.width
        );
        assert!(pane.x as u32 + pane.width <= w);
        for view in [ViewMode::Details, ViewMode::Large, ViewMode::Medium] {
            f.set_view(view);
            assert!(f.content_layout().capacity(view) > 0, "{w}x{h} {view:?}");
        }
        f.set_view(ViewMode::Details);
        save("details-pane", &f, &paint(&f, (0, 0)));
    }
}

// file_drag.rs (accelerated 3840x2160, the 1280x720 logical desktop at
// density 3): Explorer at its initial placement shows /users/1001/notes
// with "New folder 1" and Notes' desktop.txt. The scenario presses on the
// file, drags it onto the folder and releases there.
const DRAG_FILE: (i32, i32) = (217, 211);
const DRAG_FOLDER: (i32, i32) = (217, 182);

#[test]
fn file_drag_points_hit_the_file_and_the_folder_rows() {
    let mut f = Files::empty("/users/1001".into(), Controls::default());
    f.configure(FULL.0, FULL.1);
    f.history = alloc::vec!["/users/1001/notes".into()];
    f.install(
        "/users/1001/notes".into(),
        alloc::vec![
            entry("desktop.txt", 21, false),
            entry("New folder 1", 22, true)
        ],
    );
    let name = |p: (i32, i32)| {
        f.entry_at(p.0, p.1, false)
            .map(|i| Files::name(&f.entries[i]))
    };
    // Both points sit 48 px into their rows, at mid height.
    for (row, point) in [DRAG_FOLDER, DRAG_FILE].into_iter().enumerate() {
        let r = f.item_cell(row).unwrap();
        assert_eq!(point, (r.x + 48, r.y + r.height as i32 / 2));
    }
    assert_eq!(name(DRAG_FOLDER).as_deref(), Some("New folder 1"));
    assert_eq!(name(DRAG_FILE).as_deref(), Some("desktop.txt"));
    // A drop on the folder row lands in it; on blank list space, here.
    let scale = 100;
    f.dnd_display(scale);
    assert_eq!(
        f.drop_folder(DRAG_FOLDER.0, DRAG_FOLDER.1).as_deref(),
        Some("/users/1001/notes/New folder 1")
    );
    assert_eq!(
        f.drop_folder(DRAG_FILE.0, DRAG_FILE.1).as_deref(),
        Some("/users/1001/notes")
    );
}
/// explorer_modern.rs (1920x1080, Explorer maximized: client 1920x1032 at
/// the screen origin): the Inspector's Tags tab and its Create tag field.
const MODERN_TAGS_TAB: (i32, i32) = (1857, 333);
const MODERN_CREATE_TAG: (i32, i32) = (1739, 392);
#[test]
fn modern_inspector_tag_points_follow_the_layout() {
    let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
    let mut f = notes((1920, 1032), &[("desktop.txt", false)]);
    select(&mut f, 0);
    let entry = f.entries[0];
    f.detail = Some(modern::Detail {
        path: "/users/1001/notes/desktop.txt".into(),
        name: "desktop.txt".into(),
        info: entry.info,
        times: entry.times,
        kind: door_workspace::explorer_preview::Kind::Text,
        tags: 0,
        children: Vec::new(),
        cookie: 0,
        children_done: true,
        truncated: false,
        status: String::new(),
        current: false,
    });
    assert_eq!(
        f.modern_target(MODERN_TAGS_TAB.0, MODERN_TAGS_TAB.1, &fonts),
        Some(914),
        "Tags tab"
    );
    f.detail_tab = 2;
    assert_eq!(
        f.modern_target(MODERN_CREATE_TAG.0, MODERN_CREATE_TAG.1, &fonts),
        Some(930),
        "Create tag field"
    );
}
#[test]
fn folder_sizes_fill_details_rows_only_when_shown() {
    use door_workspace::explorer_size::{State, Totals};
    let mut f = notes((1280, 720), &[("Projects", true), ("desktop.txt", false)]);
    f.view = ViewMode::Details;
    let folder = *f.entries.iter().find(|e| e.info.directory).unwrap();
    // Switched off (View > Show > Folder sizes): no scan, a blank cell.
    f.prefs.hide_folder_sizes = true;
    assert_eq!(f.unsized_folder(), None);
    assert_eq!(f.folder_size_text(&folder), None);
    // The default: the visible folder row is scanned.
    f.prefs.hide_folder_sizes = false;
    let (path, info) = f.unsized_folder().unwrap();
    assert_eq!(
        (path.as_str(), info),
        ("/users/1001/notes/Projects", folder.info)
    );
    let totals = |state| Totals {
        bytes: 2048,
        files: 2,
        folders: 0,
        unreadable: 0,
        state,
    };
    f.size_cache.push((folder.info, totals(State::Complete)));
    assert_eq!(f.unsized_folder(), None);
    let complete = f.folder_size_text(&folder).unwrap();
    assert_eq!(complete, previewing::size(2048));
    f.size_cache[0].1 = totals(State::Partial);
    assert_eq!(
        f.folder_size_text(&folder).unwrap(),
        format!("> {complete}")
    );
    // Other layouts keep the Size cell to Details.
    f.view = ViewMode::Tiles;
    f.size_cache.clear();
    assert_eq!(f.unsized_folder(), None);
}
#[test]
fn the_navigation_pane_splitter_drags_reflows_and_persists() {
    use door_workspace::explorer_layout::{SIDEBAR, SIDEBAR_MIN, content_top};
    let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
    let mut f = notes((1280, 720), &[("desktop.txt", false)]);
    let (x, y) = (SIDEBAR, content_top() + 200);
    // Over the divider: the east-west pointer, and no click target.
    assert!(f.splitter_cursor(x, y));
    assert!(!f.splitter_cursor(x + 40, y));
    assert_eq!(f.modern_target(x, y, &fonts), None);
    let rows = f.rail_rows();
    let before = f.rail_row_bounds(&rows)[0].width;
    // Drag 60 px right: the pane, its rows and the item view follow live.
    assert!(f.pointer_down(x, y));
    assert!(f.pointer_motion(x + 60, y));
    assert_eq!(f.layout.sidebar, SIDEBAR + 60);
    assert_eq!(f.rail_row_bounds(&rows)[0].width, before + 60);
    assert!(
        f.splitter_cursor(x + 200, y + 100),
        "keeps the pointer while dragging"
    );
    f.pointer_up();
    assert_eq!(f.prefs.sidebar, Some(SIDEBAR + 60));
    assert!(f.prefs_dirty);
    // Clamped: never under the minimum, never leaving under 320 px of view.
    let at = f.layout.sidebar;
    assert!(f.pointer_down(at, y));
    f.pointer_motion(-500, y);
    assert_eq!(f.layout.sidebar, SIDEBAR_MIN);
    f.pointer_motion(5000, y);
    // 600 px cap (the 1280 px window would allow 960).
    assert_eq!(f.layout.sidebar, 600);
    f.pointer_up();
    // A narrower window keeps the width; the saved width round-trips.
    f.configure(900, 600);
    assert_eq!(f.layout.sidebar, 600);
    let saved = super::prefs::Prefs::decode(&f.prefs.encode()).unwrap();
    assert_eq!(saved.sidebar, Some(600));
}
/// Review renders for the owner (`DUMP_DIR=<dir> cargo test -p door-workspace
/// render_explorer_review -- --ignored`): Explorer with a file's context menu
/// in both themes, a widened navigation pane and folder sizes.
#[test]
#[ignore]
fn render_explorer_review() {
    fn bmp(path: &str, w: u32, h: u32, bgra: &[u8]) {
        let mut out = std::vec::Vec::new();
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
        out.extend_from_slice(bgra);
        std::fs::write(path, out).unwrap();
    }
    let dir = std::env::var("DUMP_DIR").unwrap();
    let size = (1280, 720);
    let mut f = notes(size, &[("Projects", true), ("Report.txt", false)]);
    f.view = ViewMode::Details;
    select(&mut f, 1);
    f.keyboard_context(size.0, size.1);
    for (dark, name) in [(false, "explorer-menu-light"), (true, "explorer-menu-dark")] {
        bmp(
            &format!("{dir}/{name}.bmp"),
            size.0,
            size.1,
            &paint_as(&f, (0, 0), None, dark),
        );
    }
    f.dismiss_menu();
    // The classic-menu setting: Windows 10's menu, "Send to" open and a
    // disabled row hot under the pointer.
    f.controls = f.controls.with(Controls::CLASSIC_CONTEXT_MENU, true);
    f.keyboard_context(size.0, size.1);
    let menu = f.xmenu.as_mut().unwrap();
    let send = menu
        .menu
        .entries
        .iter()
        .position(|e| e.label == "Send to")
        .unwrap();
    menu.sub = Some((send, None, 0));
    menu.focus = cm::Focus::Row(send);
    let sub = menu.submenu().unwrap();
    let hot = sub.rows[0].unwrap();
    let pointer = (hot.x + 20, hot.y + 8);
    for (dark, name) in [
        (false, "explorer-win10-light"),
        (true, "explorer-win10-dark"),
    ] {
        bmp(
            &format!("{dir}/{name}.bmp"),
            size.0,
            size.1,
            &paint_as(&f, pointer, None, dark),
        );
    }
    f.dismiss_menu();
    f.controls = f.controls.with(Controls::CLASSIC_CONTEXT_MENU, false);
    f.layout.sidebar = 230;
    f.prefs.hide_folder_sizes = false;
    let folder = *f.entries.iter().find(|e| e.info.directory).unwrap();
    f.size_cache.push((
        folder.info,
        door_workspace::explorer_size::Totals {
            bytes: 48_213,
            files: 6,
            folders: 1,
            unreadable: 0,
            state: door_workspace::explorer_size::State::Complete,
        },
    ));
    bmp(
        &format!("{dir}/explorer-pane-sizes.bmp"),
        size.0,
        size.1,
        &paint_as(&f, (f.layout.sidebar, 400), None, false),
    );
}

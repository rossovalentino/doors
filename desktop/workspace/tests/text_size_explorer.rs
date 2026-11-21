//! Accessibility > Text size (P12-TEXT-SIZE) and contrast themes on the
//! actual Explorer layout, painter and semantic tree.
//!
//! The applied text size is process-global (`palette::text_percent`) and
//! this binary also compiles the Explorer modules' own unit tests (they come
//! with `files.rs` and assume 100%). Each check therefore re-runs this
//! binary filtered to itself, single-threaded, in a child process
//! (`isolated`), holds `LOCK` there and restores 100%.
#![allow(dead_code, unused_imports)] // Private app modules expose more than these checks use.
extern crate alloc;
use alloc::{format, string::String};
use door_graphics_text::{FontAtlas, TextStyle};
use door_service_client::{Error, Storage};
use door_service_protocol::Code;
use door_service_protocol::desktop::ContrastTheme;
use door_workspace::explorer_layout::{self as xl, ViewMode};
use door_workspace::menu;
use door_workspace::palette::{Palette, set_text_percent};
use door_workspace::semantics::{Role, Semantics};
use std::sync::Mutex;
#[path = "../src/files.rs"]
mod files;
#[path = "../src/icons.rs"]
mod icons;
#[path = "../src/tokens.rs"]
mod tokens;
#[path = "../src/ui.rs"]
mod ui;

static LOCK: Mutex<()> = Mutex::new(());

// `files.rs` brings its raster unit tests (`tests/support/files_raster.rs`),
// which paint through these crate-root helpers, as in `raster_alignment`.
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
const CHILD: &str = "DOOR_TEXT_SIZE_CHILD";

/// Runs `body` in a child process that runs only the test `name`.
fn isolated(name: &str, body: impl FnOnce()) {
    if std::env::var_os(CHILD).is_some() {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        body();
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([name, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD, "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "{name} failed in its child process:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn at_text<T>(text: u32, f: impl FnOnce() -> T) -> T {
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            set_text_percent(100);
        }
    }
    let _restore = Restore;
    set_text_percent(text);
    f()
}

#[derive(Clone, Debug)]
struct Node {
    target: Option<u32>,
    role: Role,
    name: String,
    r: (i32, i32, u32, u32),
}

fn collect(s: Semantics, title: &str) -> Vec<Node> {
    let built = s.finish(1, 1, title);
    built
        .tree
        .nodes()
        .map(|n| {
            let (x, y, w, h) = n.bounds.logical();
            Node {
                target: built
                    .targets
                    .iter()
                    .find(|(id, _)| *id == n.id)
                    .map(|(_, t)| *t),
                role: n.role,
                name: String::from(n.name.as_str()),
                r: (x, y, w as u32, h as u32),
            }
        })
        .collect()
}

fn width(fonts: &FontAtlas, text: &str, style: TextStyle) -> u32 {
    fonts.measure(text, style).map_or(0, |m| m.advance_width)
}

fn find<'a>(nodes: &'a [Node], role: Role, name: &str) -> &'a Node {
    nodes
        .iter()
        .find(|n| n.role == role && n.name == name)
        .unwrap_or_else(|| panic!("no {role:?} {name:?}"))
}

/// The node's centre and left corners answer its own target: semantics
/// bounds are the hit-test regions.
fn assert_hits(
    nodes: &[Node],
    what: &str,
    hit: impl Fn(i32, i32) -> Option<u32>,
    only: impl Fn(&Node) -> bool,
) {
    let mut checked = 0;
    for n in nodes.iter().filter(|n| only(n)) {
        let Some(target) = n.target else { continue };
        let (x, y, w, h) = n.r;
        if w < 4 || h < 4 {
            continue;
        }
        let (cx, cy, b) = (x + w as i32 / 2, y + h as i32 / 2, y + h as i32 - 1);
        for (px, py) in [(cx, cy), (x, y), (x, b)] {
            assert_eq!(
                hit(px, py),
                Some(target),
                "{what}: {:?} {:?} ({target}) at {px},{py} of {:?}",
                n.role,
                n.name,
                n.r
            );
        }
        checked += 1;
    }
    assert!(checked > 0, "{what}: nothing checked");
}

// ---------------------------------------------------------------- Explorer

const EW: u32 = 1460;
const EH: u32 = 900;

fn explorer_nodes(f: &files::Files, fonts: &FontAtlas) -> Vec<Node> {
    let mut s = Semantics::new(EW, EH);
    f.semantics(&mut s, fonts);
    collect(s, "Explorer")
}

/// Item and tree rows fully inside the scrolled views.
fn on_screen(n: &Node) -> bool {
    let (_, y, _, h) = n.r;
    y >= xl::content_top() && y + h as i32 <= EH as i32 - xl::status_height() as i32
}

fn first(nodes: &[Node], role: Role) -> Node {
    nodes
        .iter()
        .find(|n| n.role == role && on_screen(n))
        .cloned()
        .unwrap_or_else(|| panic!("no {role:?}"))
}

struct ExplorerSample {
    row: u32,
    list_row: u32,
    tree: u32,
    command: u32,
    address: u32,
    menu: u32,
    home_header: u32,
    home_tile: u32,
}

fn explorer_sample(fonts: &FontAtlas, what: &str, check: bool) -> ExplorerSample {
    let line14 = fonts.line_height(TextStyle::Regular14);
    let semibold = fonts.line_height(TextStyle::Semibold14);
    // Details view: rows, column header, tree, command bar, address/search.
    let details = files::Files::review_fixture(ViewMode::Details, EW, EH);
    let all = explorer_nodes(&details, fonts);
    let row = first(&all, Role::ListItem);
    let tree = first(&all, Role::TreeItem);
    let address = find(&all, Role::Edit, "Address").clone();
    let search = all
        .iter()
        .find(|n| n.role == Role::Edit && n.name.starts_with("Search"))
        .unwrap()
        .clone();
    let commands: Vec<Node> = all
        .iter()
        .filter(|n| n.role == Role::Button && matches!(n.target, Some(8 | 5 | 6 | 910)))
        .cloned()
        .collect();
    assert_eq!(commands.len(), 4, "{what}: labelled commands {commands:?}");
    // List view rows.
    let list = files::Files::review_fixture(ViewMode::List, EW, EH);
    let list_row = first(&explorer_nodes(&list, fonts), Role::ListItem);
    // Context menu on the first row.
    let mut menu = files::Files::review_fixture(ViewMode::Details, EW, EH);
    menu.context(row.r.0 + 40, row.r.1 + row.r.3 as i32 / 2, EW, EH);
    let menu_nodes = explorer_nodes(&menu, fonts);
    // Text rows; the icon row's 32 px buttons keep their size.
    let items: Vec<Node> = menu_nodes
        .iter()
        .filter(|n| n.role == Role::MenuItem && n.r.2 > 40)
        .cloned()
        .collect();
    assert!(items.len() >= 4, "{what}: menu items {}", items.len());
    // Home sections.
    let home = files::Files::review_home(EW, EH);
    let home_nodes = explorer_nodes(&home, fonts);
    let header = home_nodes
        .iter()
        .find(|n| n.role == Role::Expander)
        .unwrap()
        .clone();
    let tile = home_nodes
        .iter()
        .filter(|n| n.role == Role::ListItem)
        .max_by_key(|n| n.r.3)
        .unwrap()
        .clone();
    if check {
        // Rows hold their label's (grown) line box with padding.
        for (name, n, pad) in [
            ("details row", &row, 6),
            ("list row", &list_row, 2),
            // Windows' 30.7 px tree plate holds a 20 px line (26H2 E4).
            ("tree row", &tree, 10),
        ] {
            assert!(
                n.r.3 >= line14 + pad,
                "{what}: {name} {:?} vs line {line14}",
                n.r
            );
        }
        for n in [&address, &search] {
            assert!(n.r.3 >= line14 + 8, "{what}: {} {:?}", n.name, n.r);
        }
        // Labelled commands: icon, label and chevron fit with 8 px padding.
        for c in &commands {
            let label = match c.target {
                Some(8) => "New",
                Some(5) => "Sort",
                Some(6) => "View",
                _ => "Details",
            };
            let chevron = if c.target == Some(910) { 0 } else { 20 };
            assert!(c.r.3 >= line14 + 16, "{what}: command {label} {:?}", c.r);
            assert!(
                26 + width(fonts, label, TextStyle::Regular14) + chevron + 16 <= c.r.2,
                "{what}: command {label} {:?}",
                c.r
            );
        }
        // Command buttons never overlap.
        let mut sorted = commands.clone();
        sorted.sort_by_key(|c| c.r.0);
        for pair in sorted.windows(2) {
            assert!(
                pair[0].r.0 + pair[0].r.2 as i32 <= pair[1].r.0,
                "{what}: commands overlap {pair:?}"
            );
        }
        // Column header (E8): Windows' 27 px band at the content top holds
        // a 14 px line with its 7 px of padding at every text size.
        let top = xl::content_top();
        let header_h = xl::details_header();
        assert!(header_h >= line14 + 7, "{what}: header {header_h}");
        let name_x = row.r.0 + 40;
        assert_eq!(
            details.target(name_x, top, fonts),
            Some(960),
            "{what}: header top"
        );
        assert_eq!(
            details.target(name_x, top + header_h as i32 - 1, fonts),
            Some(960),
            "{what}: header bottom"
        );
        assert!(
            row.r.1 >= top + header_h as i32,
            "{what}: first row {:?} under the header",
            row.r
        );
        // Status bar holds its line; its view buttons stay inside it.
        assert!(
            xl::status_height() >= line14 + 4,
            "{what}: status {}",
            xl::status_height()
        );
        let status = find(&all, Role::Button, "Details");
        assert!(
            status.r.1 >= EH as i32 - xl::status_height() as i32,
            "{what}: status button {:?}",
            status.r
        );
        for n in &items {
            assert!(
                n.r.3 >= line14 + 4,
                "{what}: menu item {} {:?}",
                n.name,
                n.r
            );
        }
        assert!(
            header.r.3 >= semibold + 8,
            "{what}: home header {:?}",
            header.r
        );
        assert!(
            tile.r.3 >= 2 * line14 + 16,
            "{what}: home tile {:?}",
            tile.r
        );
        // Semantics bounds are the hit-test layout.
        assert_hits(&all, what, hit(&details, fonts), |n| {
            matches!(n.role, Role::Button | Role::TreeItem | Role::ListItem) && on_screen_or_bar(n)
        });
        assert_hits(&menu_nodes, what, hit(&menu, fonts), |n| {
            n.role == Role::MenuItem
        });
        assert_hits(&home_nodes, what, hit(&home, fonts), |n| {
            matches!(n.role, Role::Expander | Role::ListItem) && on_screen(n)
        });
    }
    ExplorerSample {
        row: row.r.3,
        list_row: list_row.r.3,
        tree: tree.r.3,
        command: commands[0].r.3,
        address: address.r.3,
        menu: items[0].r.3,
        home_header: header.r.3,
        home_tile: tile.r.3,
    }
}

fn hit<'a>(f: &'a files::Files, fonts: &'a FontAtlas) -> impl Fn(i32, i32) -> Option<u32> + 'a {
    move |x, y| f.target(x, y, fonts)
}
/// Buttons in the caption, command and status bars, plus visible rows.
fn on_screen_or_bar(n: &Node) -> bool {
    n.role == Role::Button
        && (n.r.1 < xl::content_top() + 4 || n.r.1 >= EH as i32 - xl::status_height() as i32)
        || on_screen(n)
}

fn explorer_check(display: u32, text: u32) {
    let base = explorer_sample(
        &FontAtlas::windows_reference_text(display, 100).unwrap(),
        "base",
        false,
    );
    let fonts = FontAtlas::windows_reference_text(display, text).unwrap();
    at_text(text, || {
        let what = format!("Explorer display {display}% text {text}%");
        let grown = explorer_sample(&fonts, &what, true);
        for (name, before, after) in [
            ("details row", base.row, grown.row),
            ("list row", base.list_row, grown.list_row),
            ("tree row", base.tree, grown.tree),
            ("command", base.command, grown.command),
            ("address", base.address, grown.address),
            ("menu item", base.menu, grown.menu),
            ("home header", base.home_header, grown.home_header),
            ("home tile", base.home_tile, grown.home_tile),
        ] {
            assert!(after > before, "{what}: {name} {before} -> {after}");
        }
    });
}

#[test]
fn explorer_text_holding_layout_grows_and_fits_at_225() {
    isolated("explorer_text_holding_layout_grows_and_fits_at_225", || {
        explorer_check(100, 225)
    });
}

#[test]
fn explorer_text_size_multiplies_with_display_150() {
    isolated("explorer_text_size_multiplies_with_display_150", || {
        explorer_check(150, 150)
    });
}

#[test]
fn explorer_layout_is_unchanged_at_100() {
    isolated("explorer_layout_is_unchanged_at_100", || {
        // The reference geometry other tests and scenarios pin: the
        // Windows 11 26H2 bands (fidelity-26h2-diff E1, E4, E8).
        assert_eq!(
            (
                xl::grow(),
                xl::content_top(),
                xl::status_height(),
                xl::address_height()
            ),
            (0, 136, 26, 32)
        );
        assert_eq!((xl::command_top(), xl::command_height()), (89, 47));
        assert_eq!(xl::details_header(), 27);
        let l = xl::ExplorerLayout::new(742, 498);
        // A 26 px plate at the 29 px pitch (Windows 24.7 at 28.7).
        assert_eq!(l.cell_bounds(0, ViewMode::Details).height, 26);
        assert_eq!(
            xl::ExplorerLayout::navigation(0).unwrap().y,
            xl::ADDRESS_TOP
        );
        assert_eq!(
            door_workspace::explorer_tree::row_pitch(),
            door_workspace::explorer_tree::ROW_PITCH
        );
    });
}

fn paint_theme(
    w: u32,
    h: u32,
    contrast: Option<ContrastTheme>,
    fonts: &FontAtlas,
    draw: impl FnOnce(&mut ui::View<'_>),
) -> Vec<u8> {
    let mut pixels = vec![0; (w * h * 4) as usize];
    let images = ui::ImageCache::default();
    let mut v = ui::View::new_scaled(&mut pixels, w, h, fonts, 1).with_images(&images);
    v.palette = Palette {
        dark: false,
        contrast,
    };
    draw(&mut v);
    pixels
}

fn pixel(p: &[u8], w: u32, x: i32, y: i32) -> [u8; 3] {
    let at = ((y as u32 * w + x as u32) * 4) as usize;
    [p[at + 2], p[at + 1], p[at]]
}

/// The share of `rect`'s pixels that are exactly `rgb`.
fn share(p: &[u8], w: u32, rect: (i32, i32, u32, u32), rgb: [u8; 3]) -> f32 {
    let (x, y, rw, rh) = rect;
    let mut hit = 0;
    for py in y..y + rh as i32 {
        for px in x..x + rw as i32 {
            hit += u32::from(pixel(p, w, px, py) == rgb);
        }
    }
    hit as f32 / (rw * rh).max(1) as f32
}

#[test]
fn explorer_contrast_themes_paint_system_colours() {
    isolated("explorer_contrast_themes_paint_system_colours", || {
        let fonts = FontAtlas::windows_reference_text(100, 100).unwrap();
        for theme in ContrastTheme::ALL {
            let s = theme.colors();
            let mut f = files::Files::review_fixture(ViewMode::Details, EW, EH);
            let all = explorer_nodes(&f, &fonts);
            let row = first(&all, Role::ListItem);
            f.context(row.r.0 + 40, row.r.1 + row.r.3 as i32 / 2, EW, EH);
            let menu = explorer_nodes(&f, &fonts);
            let panel = menu.iter().find(|n| n.role == Role::Menu).unwrap().r;
            let p = paint_theme(EW, EH, Some(theme), &fonts, |v| f.draw(v));
            // Command bar background and the menu panel are Window; the
            // menu border is WindowText; menu text is WindowText.
            assert_eq!(
                pixel(&p, EW, EW as i32 / 2, xl::command_top() + 2),
                s.window,
                "{theme:?} command bar"
            );
            assert_eq!(
                pixel(&p, EW, panel.0, panel.1 + panel.3 as i32 / 2),
                s.window_text,
                "{theme:?} menu border"
            );
            let inside = (panel.0 + 2, panel.1 + 2, panel.2 - 4, panel.3 - 4);
            assert!(
                share(&p, EW, inside, s.window) > 0.5,
                "{theme:?} menu panel"
            );
            assert!(
                share(&p, EW, inside, s.window_text) > 0.002,
                "{theme:?} menu text"
            );
            // Details pane background (a literal fill) is Window too.
            assert_eq!(
                pixel(&p, EW, EW as i32 - 30, EH as i32 - 60),
                s.window,
                "{theme:?} details pane"
            );
        }
    });
}

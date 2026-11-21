//! Accessibility > Text size (P12-TEXT-SIZE) and contrast themes on the
//! actual Terminal layout, painter and semantic tree.
//!
//! The applied text size is process-global (`palette::text_percent`) and
//! this binary also compiles the Terminal module's own unit tests (they come
//! with `terminal.rs` and assume 100%). Each check therefore re-runs this
//! binary filtered to itself, single-threaded, in a child process
//! (`isolated`), holds `LOCK` there and restores 100%.
#![allow(dead_code, unused_imports)] // Private app modules expose more than these checks use.
extern crate alloc;
use alloc::{format, string::String};
use door_graphics_text::{FontAtlas, TextStyle};
use door_service_client::{Error, Storage};
use door_service_protocol::Code;
use door_service_protocol::desktop::ContrastTheme;
use door_workspace::palette::{Palette, set_text_percent};
use door_workspace::semantics::{Role, Semantics};
use door_workspace::terminal_model::Page;
use std::sync::Mutex;
#[path = "../src/terminal.rs"]
mod terminal;
// The library's own View, so Terminal shares the caption-tab painter with
// the workspace binary exactly as it runs natively (as `terminal_raster`).
use door_workspace::ui;

static LOCK: Mutex<()> = Mutex::new(());
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

fn paint(
    w: u32,
    h: u32,
    contrast: Option<ContrastTheme>,
    fonts: &FontAtlas,
    draw: impl FnOnce(&mut ui::View<'_>),
) -> Vec<u8> {
    let mut pixels = vec![0; (w * h * 4) as usize];
    let images = ui::ImageCache::default();
    let mono = FontAtlas::monospace_reference_scaled(1).unwrap();
    let mut v = ui::View::new_scaled(&mut pixels, w, h, fonts, 1)
        .with_images(&images)
        .with_monospace(Some(&mono));
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

// ---------------------------------------------------------------- Terminal

const TW: u32 = 1280;
const TH: u32 = 800;

fn terminal(page: Page) -> terminal::Terminal {
    let mut t = terminal::Terminal::for_home("/users/owner".into());
    t.configure(TW, TH);
    t.state.page = page;
    t
}

fn terminal_nodes(t: &terminal::Terminal) -> Vec<Node> {
    let mut s = Semantics::new(TW, TH);
    t.semantics(&mut s);
    collect(s, "Terminal")
}

struct TerminalSample {
    rail: u32,
    card: u32,
    menu_pitch: i32,
}

fn terminal_sample(fonts: &FontAtlas, what: &str, check: bool) -> TerminalSample {
    let line14 = fonts.line_height(TextStyle::Regular14);
    let line12 = fonts.line_height(TextStyle::Regular12);
    let settings = terminal(Page::Settings);
    let nodes = terminal_nodes(&settings);
    let mut rail: Vec<Node> = nodes
        .iter()
        .filter(|n| n.role == Role::ListItem)
        .cloned()
        .collect();
    rail.sort_by_key(|n| n.r.1);
    assert_eq!(rail.len(), 8, "{what}: rail rows");
    let mut cards: Vec<Node> = nodes
        .iter()
        .filter(|n| matches!(n.target, Some(40..=42 | 44)))
        .cloned()
        .collect();
    cards.sort_by_key(|n| n.r.1);
    assert_eq!(cards.len(), 4, "{what}: settings cards");
    let save = nodes.iter().find(|n| n.target == Some(43)).unwrap().clone();
    // Actions menu rows (hit testing only; the menu has no semantics).
    let mut home = terminal(Page::Console);
    home.open_actions_for_review();
    let rows: Vec<i32> = (104..400)
        .filter(|y| {
            home.target(TW as i32 - 100, *y)
                .is_some_and(|t| (31..=33).contains(&t))
        })
        .collect();
    let pitch = rows
        .iter()
        .filter(|y| home.target(TW as i32 - 100, **y) == Some(31))
        .count() as i32;
    if check {
        for n in &rail {
            assert!(n.r.3 >= line14 + 8, "{what}: rail {} {:?}", n.name, n.r);
        }
        for pair in rail.windows(2) {
            assert!(
                pair[0].r.1 + pair[0].r.3 as i32 <= pair[1].r.1,
                "{what}: rail rows overlap {pair:?}"
            );
        }
        // Cards hold a 14 px title over a 12 px description; they stack.
        for c in &cards {
            assert!(
                c.r.3 >= line14 + line12 + 24,
                "{what}: card {} {:?}",
                c.name,
                c.r
            );
        }
        for pair in cards.windows(2) {
            assert!(
                pair[0].r.1 + pair[0].r.3 as i32 <= pair[1].r.1,
                "{what}: cards overlap"
            );
        }
        assert!(
            save.r.3 >= fonts.line_height(TextStyle::Semibold14) + 8,
            "{what}: Save {:?}",
            save.r
        );
        assert!(
            save.r.1 > cards[3].r.1 + cards[3].r.3 as i32,
            "{what}: Save under the cards"
        );
        // Menu rows hold their label, and the menu is wide enough for it.
        assert!(pitch >= line14 as i32 + 8, "{what}: menu row {pitch}");
        let menu_left = (0..TW as i32)
            .find(|x| home.target(*x, 104 + pitch / 2) == Some(31))
            .unwrap();
        let menu_w = TW as i32 - 12 - menu_left;
        assert!(
            28 + width(fonts, "Scroll to bottom", TextStyle::Regular14) as i32 <= menu_w,
            "{what}: menu width {menu_w}"
        );
        // Semantics bounds are the hit-test layout.
        assert_hits(
            &nodes,
            what,
            |x, y| settings.target(x, y),
            |n| {
                matches!(n.role, Role::ListItem | Role::Button) && n.target.is_some_and(|t| t < 100)
            },
        );
    }
    let home = rail.iter().find(|n| n.target == Some(1)).unwrap();
    let card = cards.iter().find(|n| n.target == Some(40)).unwrap();
    TerminalSample {
        rail: home.r.3,
        card: card.r.3,
        menu_pitch: pitch,
    }
}

fn terminal_check(display: u32, text: u32) {
    let base = terminal_sample(
        &FontAtlas::windows_reference_text(display, 100).unwrap(),
        "base",
        false,
    );
    let fonts = FontAtlas::windows_reference_text(display, text).unwrap();
    at_text(text, || {
        let what = format!("Terminal display {display}% text {text}%");
        let grown = terminal_sample(&fonts, &what, true);
        assert!(
            grown.rail > base.rail,
            "{what}: rail {} -> {}",
            base.rail,
            grown.rail
        );
        assert!(
            grown.card > base.card,
            "{what}: card {} -> {}",
            base.card,
            grown.card
        );
        assert!(
            grown.menu_pitch > base.menu_pitch,
            "{what}: menu {} -> {}",
            base.menu_pitch,
            grown.menu_pitch
        );
        // The console grid follows its own font size, not the text size.
        assert_eq!(
            terminal(Page::Console).state.preferences.cell(),
            (9, 21),
            "{what}"
        );
    });
}

#[test]
fn terminal_text_holding_layout_grows_and_fits_at_225() {
    isolated("terminal_text_holding_layout_grows_and_fits_at_225", || {
        terminal_check(100, 225)
    });
}

#[test]
fn terminal_text_size_multiplies_with_display_150() {
    isolated("terminal_text_size_multiplies_with_display_150", || {
        terminal_check(150, 150)
    });
}

#[test]
fn terminal_follows_the_contrast_theme_like_windows_terminal() {
    isolated(
        "terminal_follows_the_contrast_theme_like_windows_terminal",
        || {
            let fonts = FontAtlas::windows_reference_text(100, 100).unwrap();
            for theme in ContrastTheme::ALL {
                let s = theme.colors();
                let mut t = terminal(Page::Console);
                t.state.tab().output("hello contrast");
                let p = paint(TW, TH, Some(theme), &fonts, |v| t.draw(v));
                // Console: Window background, WindowText text, opaque.
                let console = (240, 120, 400, 200);
                assert!(
                    share(&p, TW, console, s.window) > 0.8,
                    "{theme:?} console background"
                );
                assert!(
                    share(&p, TW, (220, 70, 600, 400), s.window_text) > 0.0005,
                    "{theme:?} console text"
                );
                // The selected rail row (the active Doorsh profile) is a
                // Highlight plate.
                let home = terminal_nodes(&t)
                    .into_iter()
                    .find(|n| n.target == Some(10))
                    .unwrap();
                assert!(
                    share(&p, TW, (14, home.r.1 + 4, 60, home.r.3 - 8), s.highlight) > 0.3,
                    "{theme:?} selected row"
                );
                // Settings: cards are Window, the Save button Highlight.
                t.state.page = Page::Settings;
                let p = paint(TW, TH, Some(theme), &fonts, |v| t.draw(v));
                let nodes = terminal_nodes(&t);
                let save = nodes.iter().find(|n| n.target == Some(43)).unwrap().r;
                let card = nodes.iter().find(|n| n.target == Some(40)).unwrap().r;
                assert!(
                    share(&p, TW, (save.0 + 2, save.1 + 2, 8, save.3 - 4), s.highlight) > 0.9,
                    "{theme:?} Save"
                );
                assert!(
                    share(&p, TW, (card.0 + 2, card.1 + 4, 12, card.3 - 8), s.window) > 0.9,
                    "{theme:?} card"
                );
            }
            // A contrast theme also turns acrylic off, as on Windows.
            let mut t = terminal(Page::Console);
            assert!(t.acrylic());
            t.controls_changed(
                door_service_protocol::desktop::Controls::default()
                    .with_contrast(Some(ContrastTheme::Dusk)),
            );
            assert!(!t.acrylic());
            // Without a contrast theme the console keeps its scheme.
            let mut t = terminal(Page::Console);
            t.state.tab().output("x");
            let p = paint(TW, TH, None, &fonts, |v| t.draw(v));
            assert_ne!(
                pixel(&p, TW, 500, 300),
                ContrastTheme::NightSky.colors().window
            );
        },
    );
}

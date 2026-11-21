//! Semantic trees of the shell surfaces the window server paints itself:
//! the taskbar, Start, Quick settings, the Notification center (with the
//! calendar) and the input switcher, plus the keyboard stops of the tray
//! flyouts. Every node is built from the same geometry that painting and hit
//! testing use (`taskbar::Layout`, `start::Start::rect`/`targets`, the
//! `tray` rectangles), so what an assistive client reads is what a pointer
//! would hit. Bounds are output-relative logical pixels.
//!
//! The trees are published into the door.accessibility broker under the
//! reserved window ids below (never engine window ids), owned by the server.
extern crate alloc;
use crate::apps;
use crate::start::{self, Start, Target};
use crate::taskbar::{Button, Layout, Slot, TrayTarget};
use crate::tray::{self, Flyout};
use alloc::{format, string::String, vec::Vec};
use door_accessibility::{Actions, Bounds, Node, NodeId, Role, States, Tree, TreeBuilder};
use door_graphics_core::Rect;
use door_graphics_text::FontAtlas;

/// Reserved semantic window ids of shell surfaces (outside engine ids).
pub const TASKBAR: u64 = 0x7D00_0000_0000_0001;
pub const START: u64 = 0x7D00_0000_0000_0002;
pub const QUICK_SETTINGS: u64 = 0x7D00_0000_0000_0003;
pub const NOTIFICATION_CENTER: u64 = 0x7D00_0000_0000_0004;
pub const INPUT_SWITCHER: u64 = 0x7D00_0000_0000_0005;
pub const HIDDEN_ICONS: u64 = 0x7D00_0000_0000_0006;
pub const SURFACES: [u64; 6] = [
    TASKBAR,
    START,
    QUICK_SETTINGS,
    NOTIFICATION_CENTER,
    INPUT_SWITCHER,
    HIDDEN_ICONS,
];
pub fn is_shell(window: u64) -> bool {
    SURFACES.contains(&window)
}
/// The semantic window of an open tray flyout.
pub const fn flyout_window(kind: Flyout) -> u64 {
    match kind {
        Flyout::Devices => QUICK_SETTINGS,
        Flyout::Calendar => NOTIFICATION_CENTER,
        Flyout::Input(_) => INPUT_SWITCHER,
        Flyout::Hidden(_) => HIDDEN_ICONS,
    }
}

fn bounds(r: Rect) -> Bounds {
    Bounds::from_logical(r.x(), r.y(), r.width() as i32, r.height() as i32)
}
fn node(id: u64, role: Role, name: &str, r: Rect) -> Node {
    let mut n = Node::new(NodeId(id), role).named(name);
    n.bounds = bounds(r);
    n
}
/// A keyboard-focusable, invocable control.
fn control(id: u64, role: Role, name: &str, r: Rect) -> Node {
    let mut n = node(id, role, name, r);
    n.states = States::FOCUSABLE;
    n.actions = Actions::INVOKE;
    n
}
fn push(b: &mut TreeBuilder, n: Node) -> NodeId {
    // Ids are unique by construction; a duplicate would be a builder bug,
    // and the broker validates every tree again.
    let id = n.id;
    let _ = b.push(n);
    id
}

/// Taskbar node ids.
pub mod taskbar_node {
    pub const ROOT: u64 = 1;
    pub const START: u64 = 2;
    pub const SEARCH: u64 = 3;
    pub const TASK_VIEW: u64 = 4;
    pub const LANGUAGE: u64 = 10;
    pub const QUICK_SETTINGS: u64 = 11;
    pub const CLOCK: u64 = 12;
    pub const SHOW_DESKTOP: u64 = 13;
    /// App button of `App::index()`.
    pub const APP: u64 = 100;
    /// Extra (door.shell) button `n` after the catalog apps.
    pub const EXTRA: u64 = 200;
}
/// What an invoked taskbar node stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskbarTarget {
    Slot(Slot),
    Tray(TrayTarget),
}
/// The taskbar node a target maps to (`invoke` routing), by node id.
pub fn taskbar_target(node: u64, buttons: &[Button]) -> Option<TaskbarTarget> {
    use taskbar_node as n;
    Some(match node {
        n::START => TaskbarTarget::Slot(Slot::Start),
        n::SEARCH => TaskbarTarget::Slot(Slot::Search),
        n::TASK_VIEW => TaskbarTarget::Slot(Slot::TaskView),
        n::LANGUAGE => TaskbarTarget::Tray(TrayTarget::Language),
        n::QUICK_SETTINGS => TaskbarTarget::Tray(TrayTarget::QuickSettings),
        n::CLOCK => TaskbarTarget::Tray(TrayTarget::Clock),
        n::SHOW_DESKTOP => TaskbarTarget::Tray(TrayTarget::ShowDesktop),
        id if (n::APP..n::APP + apps::COUNT as u64).contains(&id) => {
            let slot = buttons
                .iter()
                .position(|b| b.app.index() as u64 == id - n::APP)?;
            TaskbarTarget::Slot(Slot::App(slot))
        }
        id if (n::EXTRA..n::EXTRA + 64).contains(&id) => {
            TaskbarTarget::Slot(Slot::App(buttons.len() + (id - n::EXTRA) as usize))
        }
        _ => return None,
    })
}
/// Windows names a running taskbar button "<app> - n running window(s)".
pub fn taskbar_button_name(button: &Button) -> String {
    match button.windows {
        0 => String::from(button.app.label()),
        1 => format!("{} - 1 running window", button.app.label()),
        n => format!("{} - {n} running windows", button.app.label()),
    }
}

/// Name of an extra (door.shell) taskbar button, Windows style.
pub fn extra_button_name(e: &crate::shell_link::Extra) -> String {
    match e.windows() {
        0 => e.label.clone(),
        1 => format!("{} - 1 running window", e.label),
        n => format!("{} - {n} running windows", e.label),
    }
}
/// The taskbar: Start, Search, Task View, the app buttons (pinned, then
/// running) and the system tray buttons. It has no keyboard focus of its
/// own here (no Win+T traversal yet), so the tree carries none.
pub fn taskbar(
    version: u32,
    layout: Layout,
    buttons: &[Button],
    tray: &tray::State,
    open: Option<start::Mode>,
) -> Tree {
    taskbar_with(version, layout, buttons, &[], tray, open)
}
/// `taskbar` with the desktop shell's extra buttons after the apps.
pub fn taskbar_with(
    version: u32,
    layout: Layout,
    buttons: &[Button],
    extras: &[crate::shell_link::Extra],
    tray: &tray::State,
    open: Option<start::Mode>,
) -> Tree {
    use taskbar_node as n;
    let mut b = TreeBuilder::new(
        TASKBAR,
        version,
        node(n::ROOT, Role::Pane, "Taskbar", layout.bar()),
    );
    let apps = buttons.len() + extras.len();
    for (slot, r) in layout.slots(apps) {
        let mut c = match slot {
            Slot::Start => {
                let mut c = control(n::START, Role::Button, "Start", r);
                c.states |= States::EXPANDABLE | States::HAS_POPUP;
                c.states
                    .set(States::EXPANDED, open == Some(start::Mode::Start));
                c
            }
            // 26H2: Search opens its own panel.
            Slot::Search => {
                let mut c = control(n::SEARCH, Role::Button, "Search", r);
                c.states |= States::EXPANDABLE | States::HAS_POPUP;
                c.states
                    .set(States::EXPANDED, open == Some(start::Mode::Search));
                c
            }
            Slot::TaskView => control(n::TASK_VIEW, Role::Button, "Task View", r),
            Slot::App(i) => {
                if let Some(button) = buttons.get(i) {
                    let name = taskbar_button_name(button);
                    control(n::APP + button.app.index() as u64, Role::Button, &name, r)
                } else if let Some(e) = extras
                    .get(i - buttons.len())
                    .filter(|_| i < 64 + buttons.len())
                {
                    control(
                        n::EXTRA + (i - buttons.len()) as u64,
                        Role::Button,
                        &extra_button_name(e),
                        r,
                    )
                } else {
                    continue;
                }
            }
        };
        if let Slot::App(i) = slot {
            c.position = Some((i as u32 + 1, apps as u32));
        }
        push(&mut b, c);
    }
    let flyout = |kind: fn(Flyout) -> bool| tray.open.is_some_and(kind);
    for target in [
        TrayTarget::Language,
        TrayTarget::QuickSettings,
        TrayTarget::Clock,
        TrayTarget::ShowDesktop,
    ] {
        let r = layout.tray(target);
        let c = match target {
            // The indicator exists only with several methods or an IME.
            TrayTarget::Language if tray.input.is_none() => continue,
            TrayTarget::Language => {
                let mut c = control(n::LANGUAGE, Role::Button, "Input Indicator", r);
                c.states |= States::EXPANDABLE | States::HAS_POPUP;
                c.states
                    .set(States::EXPANDED, flyout(|f| matches!(f, Flyout::Input(_))));
                if let Some(prefs) = tray.input
                    && let Some(m) = prefs.methods().nth(prefs.current_index())
                {
                    c.value = format!("{} {}", m.language, tray::keyboard_label(m));
                }
                c
            }
            TrayTarget::QuickSettings => {
                let mut c = control(n::QUICK_SETTINGS, Role::Button, "Quick settings", r);
                c.states |= States::EXPANDABLE | States::HAS_POPUP;
                c.states
                    .set(States::EXPANDED, flyout(|f| f == Flyout::Devices));
                c
            }
            TrayTarget::Clock => {
                let name = match tray.time {
                    Some(time) => {
                        let (t, d) = crate::taskbar::clock_text_in(time, &tray.region_format());
                        format!("Clock {t} {d}")
                    }
                    None => String::from("Clock"),
                };
                let mut c = control(n::CLOCK, Role::Button, &name, r);
                c.states |= States::EXPANDABLE | States::HAS_POPUP;
                c.states
                    .set(States::EXPANDED, flyout(|f| f == Flyout::Calendar));
                c
            }
            TrayTarget::ShowDesktop => control(n::SHOW_DESKTOP, Role::Button, "Show desktop", r),
        };
        push(&mut b, c);
    }
    b.finish(None)
}

/// Start node ids (targets map one to one, `start_node`).
pub mod start_node {
    pub const ROOT: u64 = 1;
    pub const SEARCH: u64 = 2;
    pub const PINNED: u64 = 3;
    pub const SHOW_ALL: u64 = 4;
    pub const VIEW: u64 = 5;
    pub const ALL: u64 = 6;
    pub const RECENT: u64 = 7;
    pub const USER: u64 = 8;
    pub const POWER: u64 = 9;
    pub const FILTERS: u64 = 10;
    pub const RESULTS: u64 = 15;
    pub const CATEGORY: u64 = 2900;
    /// Search panel home lists.
    pub const TOP_APPS: u64 = 19;
    /// The Search panel's search box: a control of its own (Start's is
    /// `SEARCH`), so assistive clients announce it when the panel opens
    /// after Start, or Start switches to it.
    pub const SEARCH_PANEL: u64 = 20;
    /// Search panel home: the Quick searches card.
    pub const QUICK_SEARCHES: u64 = 21;
}
/// Node id of a Start target (stable while the pins and view are).
pub fn start_node(target: Target) -> Option<u64> {
    use start_node as n;
    Some(match target {
        Target::Search => n::SEARCH,
        Target::Pinned(i) => 1000 + i as u64,
        Target::ShowAll => n::SHOW_ALL,
        Target::View => n::VIEW,
        Target::CategoryApp(card, slot) => 2000 + card as u64 * 8 + slot as u64,
        Target::GridApp(i) => 3000 + i as u64,
        Target::ListApp(i) => 4000 + i as u64,
        Target::Recent(i) => 5000 + i as u64,
        Target::User => n::USER,
        Target::Power => n::POWER,
        // The original four pills keep 11..=14; Web, Folders and Photos
        // take 16..=18 (15 is the results list).
        Target::Chip(f) => match f.id() {
            id @ 0..=3 => 11 + id as u64,
            id => 12 + id as u64,
        },
        Target::Result(i) => 6000 + i as u64,
        Target::Action(i) => 7000 + i as u64,
        Target::TopApp(i) => 8000 + i as u64,
        Target::QuickSearch(i) => 8100 + i as u64,
        Target::History(i) => 8200 + i as u64,
        Target::Folder(i) => 8300 + i as u64,
        Target::Panel => return None,
    })
}
/// The Start target behind a node id (assistive invoke routing).
pub fn start_target(s: &Start, g: &start::Geometry, node: u64) -> Option<Target> {
    s.targets(g)
        .into_iter()
        .find(|t| panel_node(s, *t) == Some(node))
}
/// Node id of a target on the surface as it is now: the Search panel's
/// search box is `SEARCH_PANEL`, everything else as `start_node`.
pub fn panel_node(s: &Start, target: Target) -> Option<u64> {
    if target == Target::Search && s.search_panel() {
        return Some(start_node::SEARCH_PANEL);
    }
    start_node(target)
}

/// Start: the search box, then either the Pinned/Recent/All sections or the
/// search filters, results and the selected result's actions, and the
/// account and power buttons. Focus follows `Start::focus` (Tab order is
/// `Start::targets`).
pub fn start(version: u32, s: &Start, layout: Layout, fonts: &FontAtlas) -> Tree {
    use start_node as n;
    let g = s.geometry(layout);
    let body = g.body();
    // The 26H2 Search panel shares Start's surface and tree slot; it is
    // announced as "Search" and has no account/Power footer.
    let title = if g.search { "Search" } else { "Start" };
    let mut b = TreeBuilder::new(START, version, node(n::ROOT, Role::Window, title, g.panel));
    let rect = |t: Target| s.rect(&g, fonts, t);
    let targets = s.targets(&g);
    let shown = |t: &Target| targets.contains(t);
    // Body targets scrolled out of the body are realized but OFFSCREEN.
    let place = |mut c: Node, r: Rect, in_body: bool| {
        if in_body && r.intersection(body).is_none() {
            c.states |= States::OFFSCREEN;
        }
        c
    };
    let search_id = panel_node(s, Target::Search).unwrap_or(n::SEARCH);
    let mut search = control(search_id, Role::Edit, "Search box", g.search_box());
    // Assistive technology types into the box as the keyboard does; the
    // server does not implement SET_VALUE for it, so it is not advertised.
    search.actions = Actions::FOCUS;
    search.states |= States::EDITABLE;
    search.value = s.query.clone();
    search.description = String::from("Search for apps, settings, and documents");
    push(&mut b, search);
    // The 26H2 Search home shows no filter pills before typing.
    if s.searching() {
        let filters = node(
            n::FILTERS,
            Role::TabList,
            "Search filters",
            g.chip(fonts, crate::search::Filter::All),
        );
        let id = push(&mut b, filters);
        b.enter(id);
        for f in crate::search::Filter::ALL {
            let t = Target::Chip(f);
            let Some(r) = rect(t) else { continue };
            let mut c = control(start_node(t).unwrap_or(0), Role::Tab, f.label(), r);
            c.states |= States::SELECTABLE;
            c.states.set(States::SELECTED, s.filter == f);
            c.actions |= Actions::SELECT;
            if !f.available() {
                // Web and Photos: shown, honestly disabled.
                c.states = States::DISABLED | States::SELECTABLE;
                c.description = String::from(f.reason());
            }
            push(&mut b, c);
        }
        b.leave();
    }
    if !s.searching() && g.search {
        let recent = s.home_recent_count(&g);
        let searches = s.home_history_count(&g);
        let rows = (searches + recent) as u32;
        let id = push(
            &mut b,
            node(n::RECENT, Role::List, "Recent", g.home_recent_header()),
        );
        b.enter(id);
        // Recent searches lead the list, as the painter draws them.
        for (i, query) in s.history.iter().take(searches).enumerate() {
            let t = Target::History(i);
            let Some(r) = rect(t) else { continue };
            let mut c = control(start_node(t).unwrap_or(0), Role::ListItem, query, r);
            c.description = "Recent search".into();
            c.position = Some((i as u32 + 1, rows));
            push(&mut b, c);
        }
        for (i, item) in s.recent.iter().take(recent).enumerate() {
            let t = Target::Recent(i);
            let Some(r) = rect(t) else { continue };
            let mut c = control(start_node(t).unwrap_or(0), Role::ListItem, &item.name, r);
            c.description = item.path.clone();
            c.position = Some(((searches + i) as u32 + 1, rows));
            push(&mut b, c);
        }
        b.leave();
        let id = push(
            &mut b,
            node(
                n::QUICK_SEARCHES,
                Role::List,
                "Quick searches",
                g.quick_card(),
            ),
        );
        b.enter(id);
        for (i, (label, _)) in start::QUICK_SEARCHES.iter().enumerate() {
            let t = Target::QuickSearch(i);
            let Some(r) = rect(t) else { continue };
            push(
                &mut b,
                control(start_node(t).unwrap_or(0), Role::Button, label, r),
            );
        }
        b.leave();
        let id = push(
            &mut b,
            node(n::TOP_APPS, Role::List, "Top apps", g.top_apps_header()),
        );
        b.enter(id);
        let top: Vec<Target> = targets
            .iter()
            .copied()
            .filter(|t| matches!(t, Target::TopApp(_)))
            .collect();
        for t in &top {
            let (Some(r), Some(app)) = (rect(*t), s.app(*t)) else {
                continue;
            };
            let Target::TopApp(i) = t else { continue };
            let mut c = control(start_node(*t).unwrap_or(0), Role::ListItem, app.label(), r);
            c.position = Some((*i as u32 + 1, top.len() as u32));
            push(&mut b, c);
        }
        b.leave();
    } else if s.searching() {
        let results = s.results();
        let (x, y, w) = g.results_column();
        let list = node(
            n::RESULTS,
            Role::List,
            "Results",
            Rect::new(x, y, w.max(1), body.height()).unwrap_or(body),
        );
        let id = push(&mut b, list);
        b.enter(id);
        let total = results.len() as u32;
        for (i, hit) in results.iter().enumerate() {
            let t = Target::Result(i);
            let Some(r) = rect(t) else { continue };
            let mut c = control(start_node(t).unwrap_or(0), Role::ListItem, s.title(*hit), r);
            c.description = String::from(s.kind(*hit));
            c.states |= States::SELECTABLE;
            c.states
                .set(States::SELECTED, i == s.selected && s.chosen());
            c.actions |= Actions::SELECT;
            c.position = Some((i as u32 + 1, total));
            push(&mut b, place(c, r, true));
        }
        b.leave();
        for (i, label) in s.actions().iter().enumerate() {
            let t = Target::Action(i);
            if !shown(&t) {
                continue;
            }
            if let Some(r) = rect(t) {
                push(
                    &mut b,
                    control(start_node(t).unwrap_or(0), Role::Button, label, r),
                );
            }
        }
    } else {
        if g.show_pinned {
            let pinned = node(n::PINNED, Role::List, "Pinned", g.pinned_header(s.scroll));
            let id = push(&mut b, pinned);
            b.enter(id);
            let total = s.pins.len() as u32;
            for (i, app) in s.pins.iter().enumerate() {
                let t = Target::Pinned(i);
                // Rows past "Show all" are not realized.
                let Some(r) = rect(t) else { continue };
                let mut c = control(start_node(t).unwrap_or(0), Role::ListItem, app.label(), r);
                c.position = Some((i as u32 + 1, total));
                push(&mut b, place(c, r, true));
            }
            b.leave();
        }
        if shown(&Target::ShowAll)
            && let Some(r) = rect(Target::ShowAll)
        {
            let label = if s.expanded { "Show less" } else { "Show all" };
            let mut c = control(n::SHOW_ALL, Role::Button, label, r);
            c.states |= States::EXPANDABLE;
            c.states.set(States::EXPANDED, s.expanded);
            push(&mut b, place(c, r, true));
        }
        let recent: Vec<Target> = targets
            .iter()
            .copied()
            .filter(|t| matches!(t, Target::Recent(_)))
            .collect();
        if !recent.is_empty() {
            let header = g.recent_header(s.scroll).unwrap_or(body);
            let id = push(&mut b, node(n::RECENT, Role::List, "Recent", header));
            b.enter(id);
            let total = recent.len() as u32;
            for t in recent {
                let Target::Recent(i) = t else { continue };
                let (Some(r), Some(item)) = (rect(t), s.recent.get(i)) else {
                    continue;
                };
                let mut c = control(start_node(t).unwrap_or(0), Role::ListItem, &item.name, r);
                c.description = item.path.clone();
                c.position = Some((i as u32 + 1, total));
                push(&mut b, place(c, r, true));
            }
            b.leave();
        }
        if g.show_all {
            if let Some(r) = rect(Target::View) {
                let mut c = control(n::VIEW, Role::Button, &start::view_label(s.view), r);
                c.states |= States::HAS_POPUP;
                push(&mut b, place(c, r, true));
            }
            let all = s.all_apps();
            let id = push(
                &mut b,
                node(n::ALL, Role::List, "All", g.all_header(s.scroll)),
            );
            b.enter(id);
            let total = all.len() as u32;
            let categories = apps::categories();
            let mut card_open: Option<usize> = None;
            for (k, (t, app)) in all.iter().enumerate() {
                if let Target::CategoryApp(card, _) = t
                    && card_open != Some(*card)
                {
                    if card_open.is_some() {
                        b.leave();
                    }
                    let label = categories
                        .get(*card)
                        .map_or("", |c| start::category_label(c.0));
                    let group = node(
                        n::CATEGORY + *card as u64,
                        Role::Group,
                        label,
                        g.card(*card, s.scroll),
                    );
                    let gid = push(&mut b, place(group, g.card(*card, s.scroll), true));
                    b.enter(gid);
                    card_open = Some(*card);
                }
                let Some(r) = rect(*t) else { continue };
                let mut c = control(start_node(*t).unwrap_or(0), Role::ListItem, app.label(), r);
                c.position = Some((k as u32 + 1, total));
                push(&mut b, place(c, r, true));
            }
            if card_open.is_some() {
                b.leave();
            }
            b.leave();
        }
    }
    if g.show_account
        && let Some(r) = rect(Target::User)
    {
        let mut c = control(n::USER, Role::Button, start::USER_NAME, r);
        c.states |= States::HAS_POPUP;
        push(&mut b, c);
    }
    for i in s.shown_folders() {
        let t = Target::Folder(i);
        if shown(&t)
            && let Some(r) = rect(t)
        {
            let name = door_service_protocol::preferences::start_folders::NAMES[i];
            push(
                &mut b,
                control(start_node(t).unwrap_or(0), Role::Button, name, r),
            );
        }
    }
    if shown(&Target::Power)
        && let Some(r) = rect(Target::Power)
    {
        let mut c = control(n::POWER, Role::Button, "Power", r);
        c.states |= States::HAS_POPUP;
        push(&mut b, c);
    }
    let focus = s.focus.and_then(|t| panel_node(s, t)).map(NodeId);
    b.finish(focus)
}

/// One keyboard stop of an open tray flyout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stop {
    pub action: tray::Action,
    pub rect: Rect,
}
/// Keyboard stops of the open flyout in Tab order (enabled controls only;
/// unavailable tiles and sliders are announced but not focusable, as WinUI
/// skips disabled controls).
pub fn tray_stops(state: &tray::State, layout: Layout) -> Vec<Stop> {
    let mut out = Vec::new();
    match state.open {
        Some(Flyout::Devices) => {
            for (i, (_, _, reason)) in tray::TILES.iter().enumerate() {
                if reason.is_none() && i == 3 {
                    out.push(Stop {
                        action: tray::Action::Accessibility,
                        rect: tray::tile(layout, i),
                    });
                }
            }
            out.push(Stop {
                action: tray::Action::Settings,
                rect: tray::settings_button(layout),
            });
        }
        Some(Flyout::Calendar) if state.time.is_some() => {
            let (header, _, prev, next) = tray::calendar_controls(layout);
            out.push(Stop {
                action: tray::Action::Today,
                rect: header,
            });
            out.push(Stop {
                action: tray::Action::Previous,
                rect: prev,
            });
            out.push(Stop {
                action: tray::Action::Next,
                rect: next,
            });
        }
        Some(Flyout::Input(n)) => {
            for i in 0..usize::from(n) {
                out.push(Stop {
                    action: tray::Action::InputMethod(i),
                    rect: tray::input_row(layout, n, i),
                });
            }
            out.push(Stop {
                action: tray::Action::KeyboardSettings,
                rect: tray::input_footer(layout, n),
            });
        }
        _ => (),
    }
    out
}
/// Node id of a flyout stop's action.
fn stop_node(action: tray::Action) -> u64 {
    match action {
        tray::Action::Accessibility => 100 + 3,
        tray::Action::Settings => 20,
        tray::Action::Today => 30,
        tray::Action::Previous => 31,
        tray::Action::Next => 32,
        tray::Action::InputMethod(i) => 200 + i as u64,
        tray::Action::KeyboardSettings => 40,
        _ => 0,
    }
}
/// The flyout action behind a node id (assistive invoke routing).
pub fn flyout_action(state: &tray::State, layout: Layout, node: u64) -> Option<tray::Action> {
    tray_stops(state, layout)
        .into_iter()
        .map(|s| s.action)
        .find(|a| stop_node(*a) == node)
}

/// The open tray flyout. `focus` indexes `tray_stops`.
pub fn flyout(
    version: u32,
    state: &tray::State,
    layout: Layout,
    focus: Option<usize>,
) -> Option<Tree> {
    let kind = state.open?;
    let stops = tray_stops(state, layout);
    let focused = focus
        .and_then(|i| stops.get(i))
        .map(|s| NodeId(stop_node(s.action)));
    let tree = match kind {
        Flyout::Devices => {
            let r = tray::quick_rect(layout);
            let mut b = TreeBuilder::new(
                QUICK_SETTINGS,
                version,
                node(1, Role::Window, "Quick settings", r),
            );
            for (i, (label, _, reason)) in tray::TILES.iter().enumerate() {
                let mut c = control(100 + i as u64, Role::Button, label, tray::tile(layout, i));
                match reason {
                    Some(why) => {
                        c.states = States::DISABLED;
                        c.actions = Actions::NONE;
                        c.description = String::from(*why);
                    }
                    // The Accessibility tile opens Settings > Accessibility.
                    None => c.states |= States::HAS_POPUP,
                }
                push(&mut b, c);
            }
            for (i, (name, why)) in [
                ("Brightness", "Brightness control unavailable"),
                ("Volume", "No audio output device"),
            ]
            .into_iter()
            .enumerate()
            {
                let row = tray::slider_row(layout, i);
                let mut c = node(150 + i as u64, Role::Slider, name, row);
                c.states = States::DISABLED | States::HORIZONTAL;
                c.description = String::from(why);
                push(&mut b, c);
            }
            push(
                &mut b,
                control(
                    20,
                    Role::Button,
                    "All settings",
                    tray::settings_button(layout),
                ),
            );
            b.finish(focused)
        }
        Flyout::Calendar => {
            let c = tray::calendar_rect(layout);
            let area = tray::notifications_rect(layout).map_or(c, |n| crate::paint::union(n, c));
            let mut b = TreeBuilder::new(
                NOTIFICATION_CENTER,
                version,
                node(1, Role::Window, "Notification center", area),
            );
            if let Some(nr) = tray::notifications_rect(layout) {
                let id = push(&mut b, node(10, Role::Group, "Notifications", nr));
                b.enter(id);
                push(&mut b, node(11, Role::Text, "No new notifications", nr));
                b.leave();
            }
            let id = push(&mut b, node(12, Role::Group, "Calendar", c));
            b.enter(id);
            match state.time {
                Some(time) => {
                    let region = state.region_format();
                    let mut title = String::new();
                    let _ = region.format(region.month_day, &time, &mut title);
                    let (year, month) = state.month.unwrap_or((time.year, time.month));
                    let mut heading = String::new();
                    let _ = region.format(
                        region.year_month,
                        &door_abi::clock::WallClock {
                            year,
                            month,
                            day: 1,
                            ..time
                        },
                        &mut heading,
                    );
                    let (header, month_row, prev, next) = tray::calendar_controls(layout);
                    push(&mut b, control(30, Role::Button, &title, header));
                    push(&mut b, node(33, Role::Heading, &heading, month_row));
                    push(&mut b, control(31, Role::Button, "Previous", prev));
                    push(&mut b, control(32, Role::Button, "Next", next));
                }
                None => {
                    push(&mut b, node(34, Role::Text, "Clock unavailable", c));
                }
            }
            b.leave();
            b.finish(focused)
        }
        Flyout::Hidden(n) => {
            use crate::notify_icons as ni;
            let r = ni::flyout(layout, state.recording(), usize::from(n));
            let mut b = TreeBuilder::new(
                HIDDEN_ICONS,
                version,
                node(1, Role::Window, "Hidden icons", r),
            );
            for (i, icon) in state.notify.icons.iter().take(usize::from(n)).enumerate() {
                let name = if icon.tip.is_empty() {
                    "Notification icon"
                } else {
                    icon.tip.as_str()
                };
                push(
                    &mut b,
                    control(300 + i as u64, Role::Button, name, ni::cell(r, i)),
                );
            }
            b.finish(focused)
        }
        Flyout::Input(n) => {
            let mut b = TreeBuilder::new(
                INPUT_SWITCHER,
                version,
                node(
                    1,
                    Role::Window,
                    "Input switcher",
                    tray::input_rect(layout, n),
                ),
            );
            let id = push(
                &mut b,
                node(
                    2,
                    Role::List,
                    "Keyboard layouts",
                    tray::input_rect(layout, n),
                ),
            );
            b.enter(id);
            if let Some(prefs) = state.input {
                let current = prefs.current_index();
                for (i, m) in prefs.methods().enumerate().take(usize::from(n)) {
                    let name = format!("{} {}", m.language, tray::keyboard_label(m));
                    let mut c = control(
                        200 + i as u64,
                        Role::ListItem,
                        &name,
                        tray::input_row(layout, n, i),
                    );
                    c.states |= States::SELECTABLE;
                    c.states.set(States::SELECTED, i == current);
                    c.actions |= Actions::SELECT;
                    c.position = Some((i as u32 + 1, u32::from(n)));
                    push(&mut b, c);
                }
            }
            b.leave();
            push(
                &mut b,
                control(
                    40,
                    Role::Hyperlink,
                    "More keyboard settings",
                    tray::input_footer(layout, n),
                ),
            );
            b.finish(focused)
        }
    };
    Some(tree)
}

/// The caption buttons the server draws for one top-level window, in
/// window-content logical coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Caption {
    pub title: String,
    pub bar: Bounds,
    /// Minimize, Maximize (or Restore), Close.
    pub buttons: [(&'static str, Bounds); 3],
}
/// Reserved node ids of the merged caption (title bar and its buttons).
pub const CAPTION_BAR: u64 = 0xFFFF_FFFF_FFFF_FF00;
pub const CAPTION_MINIMIZE: u64 = CAPTION_BAR + 1;
pub const CAPTION_MAXIMIZE: u64 = CAPTION_BAR + 2;
pub const CAPTION_CLOSE: u64 = CAPTION_BAR + 3;
pub fn is_caption_node(id: u64) -> bool {
    (CAPTION_BAR..=CAPTION_CLOSE).contains(&id)
}
/// Caption geometry from the window's outer and content rectangles (logical
/// pixels): 46 px buttons flush right over the title strip, as
/// `Engine::caption_button_rect` places and hit-tests them.
pub fn caption(
    title: &str,
    outer: Rect,
    content: Rect,
    title_height: u32,
    maximized: bool,
) -> Caption {
    let at = |offset: i32| {
        Bounds::from_logical(
            outer.right() - offset - content.x(),
            outer.y() - content.y(),
            46,
            title_height as i32,
        )
    };
    Caption {
        title: String::from(title),
        bar: Bounds::from_logical(
            outer.x() - content.x(),
            outer.y() - content.y(),
            outer.width() as i32,
            title_height as i32,
        ),
        buttons: [
            ("Minimize", at(138)),
            (if maximized { "Restore" } else { "Maximize" }, at(92)),
            ("Close", at(46)),
        ],
    }
}
/// Insert the server-drawn title bar as the first child of `tree`'s root
/// (UI Automation exposes it as a TitleBar with its three buttons).
pub fn merge_caption(tree: &mut Tree, caption: &Caption) {
    let root = tree.root;
    if tree.get(NodeId(CAPTION_BAR)).is_some() {
        return;
    }
    let mut bar = Node::new(NodeId(CAPTION_BAR), Role::ToolBar).named("Title bar");
    bar.value = caption.title.clone();
    bar.bounds = caption.bar;
    if tree.push(root, bar).is_err() {
        return;
    }
    for (i, (name, b)) in caption.buttons.iter().enumerate() {
        let mut n = Node::new(NodeId(CAPTION_MINIMIZE + i as u64), Role::Button).named(name);
        n.bounds = *b;
        n.actions = Actions::INVOKE;
        let _ = tree.push(NodeId(CAPTION_BAR), n);
    }
    if let Some(r) = tree.get_mut(root) {
        let children = &mut r.children;
        if let Some(i) = children.iter().position(|c| *c == NodeId(CAPTION_BAR)) {
            let bar = children.remove(i);
            children.insert(0, bar);
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::apps::App;
    use door_accessibility::outline;
    fn fonts() -> FontAtlas {
        FontAtlas::windows_reference_scaled(1).unwrap()
    }
    #[test]
    fn taskbar_tree_names_buttons_like_windows_and_validates() {
        let layout = Layout::new(1920, 1080);
        let buttons = crate::taskbar::buttons(&App::ALL, &[(App::Files, true)]);
        let tray = tray::State::default();
        let t = taskbar(1, layout, &buttons, &tray, None);
        t.validate().unwrap();
        let o = outline(&t);
        assert!(o.starts_with("pane \"Taskbar\"\n"), "{o}");
        assert!(
            o.contains("button \"Start\" [FOCUSABLE EXPANDABLE HAS_POPUP] {INVOKE}"),
            "{o}"
        );
        assert!(
            o.contains("button \"File Explorer - 1 running window\""),
            "{o}"
        );
        assert!(o.contains("button \"Notes\""), "{o}");
        assert!(o.contains("button \"Show desktop\""), "{o}");
        assert!(!o.contains("Input Indicator"), "one keyboard: no indicator");
        assert_eq!(
            taskbar_target(taskbar_node::APP + App::Notes.index() as u64, &buttons),
            Some(TaskbarTarget::Slot(Slot::App(1)))
        );
    }
    #[test]
    fn start_tree_follows_keyboard_focus_through_the_tab_order() {
        let fonts = fonts();
        let layout = Layout::new(1920, 1080);
        let mut s = Start::default();
        s.open();
        let t = start(1, &s, layout, &fonts);
        t.validate().unwrap();
        assert_eq!(t.focused().map(|n| n.name.as_str()), Some("Search box"));
        // M7: only what the server performs is advertised.
        let search = t.focused().unwrap();
        assert!(!search.actions.contains(Actions::SET_VALUE));
        assert!(search.actions.contains(Actions::FOCUS));
        let g = s.geometry(layout);
        assert!(s.navigate(&g, &fonts, 4));
        let t = start(2, &s, layout, &fonts);
        let f = t.focused().unwrap();
        assert_eq!(
            (f.role, f.name.as_str()),
            (Role::ListItem, App::ALL[0].label())
        );
        assert_eq!(
            t.ancestors(f.id)
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            ["Pinned", "Start"]
        );
        // Every target in Tab order has a node.
        for target in s.targets(&g) {
            if s.rect(&g, &fonts, target).is_none() {
                continue;
            }
            let id = start_node(target).unwrap();
            assert!(t.get(NodeId(id)).is_some(), "{target:?}");
            assert_eq!(start_target(&s, &g, id), Some(target));
        }
        // Searching: filters, results and actions.
        s.type_text("notes");
        let t = start(3, &s, layout, &fonts);
        t.validate().unwrap();
        let o = outline(&t);
        assert!(o.contains("tab \"Search filters\""), "{o}");
        assert!(
            o.contains(
                "list item \"Notes\" [FOCUSABLE SELECTED SELECTABLE] {INVOKE SELECT} pos=1/"
            ),
            "{o}"
        );
    }
    #[test]
    fn tools_are_found_by_search_only() {
        let fonts = fonts();
        let layout = Layout::new(1920, 1080);
        let mut s = Start::default();
        s.open();
        s.type_text("Live captions");
        assert_eq!(
            s.results()[0],
            crate::search::Hit::Tool(apps::Tool::Captions)
        );
        assert_eq!(
            s.activate(Target::Result(0)),
            start::Activation::LaunchTool(apps::Tool::Captions)
        );
        let t = start(1, &s, layout, &fonts);
        assert!(outline(&t).contains("list item \"Live captions\""));
        assert!(!s.pins.iter().any(|a| a.label() == "Live captions"));
    }
    #[test]
    fn flyouts_expose_unavailable_tiles_and_keyboard_stops() {
        let layout = Layout::new(1920, 1080);
        let mut state = tray::State {
            open: Some(Flyout::Devices),
            ..Default::default()
        };
        let stops = tray_stops(&state, layout);
        assert_eq!(
            stops.iter().map(|s| s.action).collect::<Vec<_>>(),
            [tray::Action::Accessibility, tray::Action::Settings]
        );
        let t = flyout(1, &state, layout, Some(0)).unwrap();
        t.validate().unwrap();
        let o = outline(&t);
        assert!(o.contains("button \"Wi-Fi\" [DISABLED]"), "{o}");
        assert_eq!(t.focused().map(|n| n.name.as_str()), Some("Accessibility"));
        assert_eq!(
            flyout_action(&state, layout, 20),
            Some(tray::Action::Settings)
        );
        state.open = Some(Flyout::Calendar);
        let t = flyout(2, &state, layout, None).unwrap();
        t.validate().unwrap();
        assert!(outline(&t).contains("text \"No new notifications\""));
    }
    #[test]
    fn captions_merge_first_with_reserved_ids() {
        let mut b = TreeBuilder::new(9, 1, Node::new(NodeId(1), Role::Window).named("Notes"));
        b.push(Node::new(NodeId(2), Role::Button).named("Save"))
            .unwrap();
        let mut t = b.finish(None);
        let outer = Rect::new(100, 100, 800, 600).unwrap();
        let c = caption("Notes", outer, outer, 32, false);
        merge_caption(&mut t, &c);
        t.validate().unwrap();
        let o = outline(&t);
        let lines: Vec<&str> = o.lines().collect();
        assert_eq!(lines[1], "  tool bar \"Title bar\" value=\"Notes\"");
        assert_eq!(lines[2], "    button \"Minimize\" {INVOKE}");
        assert_eq!(lines[4], "    button \"Close\" {INVOKE}");
        assert_eq!(lines[5], "  button \"Save\"");
        assert_eq!(c.buttons[2].1.logical(), (754, 0, 46, 32));
    }
}

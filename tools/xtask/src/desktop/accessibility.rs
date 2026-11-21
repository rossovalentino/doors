//! `a11y-01` acceptance test suite (A11Y-01; spec sections 58 and 113):
//! keyboard-only navigation of the shell, Settings and File Explorer read
//! back through the Captions assistive client (`door-assist`) over the
//! door.accessibility broker, never from screenshots.
//!
//! Evidence, all from authenticated `DOOR_USER pid=` serial records of the
//! assistive client: the taskbar tree it read while unfocused; the captions
//! Narrator-style announcing of each Tab stop in Start, the 26H2 Search
//! panel (Win+S), Quick settings,
//! Settings (Personalization > Colors) and File Explorer (items view and
//! navigation pane), each naming the control and its role; and focus
//! restoration after a popup closes (Settings' combo list, Explorer's
//! context menu) and after a window is reactivated (Start opened and
//! dismissed over Explorer).
//!
//! Process budget (kernel `MAX_PROCESSES` 13): five services, the search
//! host, the IME, Captions, Settings and File Explorer (ten).
use super::gallery::{TreeReport, launch, payloads, tree_reports};
use super::*;

const WAIT: Duration = Duration::from_secs(20);
/// Roles a caption names (UI Automation control types, Narrator wording).
const ROLES: [&str; 14] = [
    "button",
    "edit",
    "list item",
    "tree item",
    "menu item",
    "combo box",
    "toggle switch",
    "check box",
    "tab item",
    "link",
    "slider",
    "expander",
    "list",
    "tree",
];

pub(super) fn suite(run: &mut Run, image: &str, _test_image: &str) -> Result<Value, String> {
    let disk = run.directory.join("a11y-private.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    // Captions writes its serial trees and captions only on the build's
    // desktop-diagnostics image (production initfs plus its flag file).
    let image = session::diagnostics_image(&run.root, image)?;
    let mut record = session::launch_desktop_accelerated(run, &image, &disk, false, "1920x1080")?;
    let result = drive(run, &record);
    if result.is_err()
        && let Ok(mut c) = connection(&record)
    {
        let _ = scene::capture_settled(run, &mut c, "a11y-failure", &[]);
        let _ = c.execute("quit", json!({}));
    }
    let mut evidence = result?;
    seal_serial(&run.root, &mut record)?;
    evidence["session"] = record;
    Ok(evidence)
}

/// The assistive client's records, authenticated by the kernel's pid prefix.
struct Reader {
    root: std::path::PathBuf,
    record: Value,
    pid: u64,
}
impl Reader {
    fn offset(&self) -> Result<usize, String> {
        Ok(serial(&self.root, &self.record)?.len())
    }
    fn poll<T>(&self, what: &str, mut f: impl FnMut(&str) -> Option<T>) -> Result<T, String> {
        let deadline = Instant::now() + WAIT;
        loop {
            let text = serial(&self.root, &self.record)?;
            if let Some(v) = f(&text) {
                return Ok(v);
            }
            if text.contains("KERNEL_PANIC")
                || text.contains("DOOR_ASSIST FAILED")
                || text.contains("APP FAILED")
            {
                return Err(format!(
                    "guest failed while waiting for {what}; inspect serial"
                ));
            }
            if Instant::now() > deadline {
                let said = payloads(&text, 0, self.pid, "DOOR_ASSIST CAPTION ");
                let tail: Vec<&String> = said.iter().rev().take(6).collect();
                return Err(format!("missing {what}; last captions {tail:?}"));
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
    /// The first caption after `from` that satisfies `want`.
    fn caption(
        &self,
        from: usize,
        what: &str,
        want: impl Fn(&str) -> bool,
    ) -> Result<String, String> {
        self.poll(&format!("caption: {what}"), |t| {
            payloads(t, from, self.pid, "DOOR_ASSIST CAPTION ")
                .into_iter()
                .find(|c| want(c))
        })
    }
    /// The first complete tree report after `from` that satisfies `want`.
    fn tree(
        &self,
        from: usize,
        what: &str,
        want: impl Fn(&TreeReport) -> bool,
    ) -> Result<TreeReport, String> {
        let mut error = None;
        let found = self.poll(&format!("tree: {what}"), |t| {
            match tree_reports(t, from, self.pid) {
                Ok(r) => r.into_iter().find(|r| want(r)),
                Err(e) => {
                    error = Some(e);
                    Some(TreeReport::default())
                }
            }
        })?;
        error.map_or(Ok(found), Err)
    }
}
/// Press `keys`, then wait for the caption announcing the new focus.
fn step(
    c: &mut qmp::Client,
    r: &Reader,
    keys: &[&str],
    what: &str,
    want: impl Fn(&str) -> bool,
) -> Result<String, String> {
    let from = r.offset()?;
    key(c, keys)?;
    r.caption(from, what, want)
}
/// A caption names its control's role (after the name, as Narrator reads).
fn has_role(caption: &str) -> bool {
    caption
        .split(", ")
        .skip(1)
        .any(|part| ROLES.contains(&part))
}
/// The control a caption names: the segment before its role.
fn named(caption: &str, role: &str) -> Option<String> {
    let at = caption.find(&format!(", {role}"))?;
    caption[..at].rsplit(", ").next().map(String::from)
}
/// The same control announced again (possibly with or without the
/// container it sits in).
fn same(a: &str, b: &str) -> bool {
    a == b || a.ends_with(b) || b.ends_with(a)
}
fn lines_contain(tree: &TreeReport, needle: &str) -> bool {
    tree.lines
        .iter()
        .any(|l| l.trim_start().starts_with(needle))
}

fn drive(run: &mut Run, record: &Value) -> Result<Value, String> {
    let root = run.root.clone();
    let mut c = connection(record)?;
    thread::sleep(Duration::from_secs(2));
    let assist = launch(&root, record, &mut c, "Live captions", "DOOR_ASSIST READY")?;
    let granted = payloads(&serial(&root, record)?, 0, assist, "DOOR_ASSIST READY ");
    if granted.last().map(String::as_str) != Some("assistive=1") {
        return Err("door-assist runs without the assistive capability (CAP_ASSISTIVE)".into());
    }
    let r = Reader {
        root: root.clone(),
        record: record.clone(),
        pid: assist,
    };

    // The taskbar, read while unfocused (the server publishes it).
    let bar = r.tree(0, "taskbar", |t| {
        t.lines.first().is_some_and(|l| l == "pane \"Taskbar\"")
    })?;
    for needle in [
        "button \"Start\"",
        "button \"Search\"",
        "button \"Task View\"",
        "button \"File Explorer",
        "button \"Quick settings\"",
        "button \"Show desktop\"",
    ] {
        if !lines_contain(&bar, needle) {
            return Err(format!("taskbar tree lacks {needle}: {:?}", bar.lines));
        }
    }

    // Start: the search box has focus, Tab walks the pinned grid and the
    // sections after it, Shift+Tab walks back, Escape closes Start.
    let mut start = vec![step(&mut c, &r, &["meta_l"], "Start search box", |s| {
        s.starts_with("Search box, edit")
    })?];
    thread::sleep(Duration::from_millis(300));
    for i in 0..4 {
        start.push(step(
            &mut c,
            &r,
            &["tab"],
            &format!("Start Tab stop {}", i + 1),
            has_role,
        )?);
    }
    if !start[1].contains("File Explorer, list item, 1 of") || !start[1].starts_with("Pinned, list")
    {
        return Err(format!(
            "first Tab stop in Start is not the first pinned app: {:?}",
            start[1]
        ));
    }
    if start.windows(2).any(|w| same(&w[0], &w[1])) {
        return Err(format!("a Tab in Start announced no new focus: {start:?}"));
    }
    // Shift+Tab walks back to the search box, announcing every stop.
    let mut back = Vec::new();
    for _ in 0..8 {
        let said = step(&mut c, &r, &["shift", "tab"], "Start Shift+Tab stop", |s| {
            has_role(s)
        })?;
        let home = said.starts_with("Search box, edit");
        back.push(said);
        if home {
            break;
        }
    }
    if !back
        .last()
        .is_some_and(|s| s.starts_with("Search box, edit"))
    {
        return Err(format!(
            "Shift+Tab never returned to the Start search box: {back:?}"
        ));
    }
    key(&mut c, &["esc"])?;
    thread::sleep(Duration::from_millis(400));

    // 26H2 Search panel (Win+S, as the taskbar Search button): its search
    // box has focus; with no query there are no filter pills, so Tab
    // reaches the first Recent item or, with none, the first Quick search
    // (`door_window::start::QUICK_SEARCHES`); Escape closes it.
    let search = vec![
        step(
            &mut c,
            &r,
            &["meta_l", "s"],
            "Search panel search box",
            |s| s.starts_with("Search box, edit"),
        )?,
        {
            thread::sleep(Duration::from_millis(300));
            // Entering a group announces it first, as Narrator does
            // ("Quick searches, list, Focus settings, button").
            let quick = format!("{}, button", door_window::start::QUICK_SEARCHES[0].0);
            step(&mut c, &r, &["tab"], "Search panel first home stop", |s| {
                s.contains(", list item") || s.contains(&quick)
            })?
        },
    ];
    key(&mut c, &["esc"])?;
    thread::sleep(Duration::from_millis(400));

    // Quick settings (Win+A): focus moves to its first available control.
    let quick = step(
        &mut c,
        &r,
        &["meta_l", "a"],
        "Quick settings first stop",
        |s| s.starts_with("Accessibility, button"),
    )?;
    let quick_next = step(&mut c, &r, &["tab"], "Quick settings Tab", |s| {
        s.starts_with("All settings, button")
    })?;
    key(&mut c, &["esc"])?;
    thread::sleep(Duration::from_millis(400));

    // Settings > Personalization > Colors: Tab through the cards, open the
    // mode combo list, close it, focus returns to the combo.
    let from = r.offset()?;
    let settings = launch(
        &root,
        record,
        &mut c,
        "Settings",
        "DOOR_P12 WINDOW_COMMITTED",
    )?;
    r.tree(from, "Settings window", |t| {
        t.lines.first().is_some_and(|l| l == "window \"Settings\"")
    })?;
    thread::sleep(Duration::from_millis(500));
    let from = r.offset()?;
    key(&mut c, &["ctrl", "f"])?;
    type_line(&mut c, "colors")?;
    let colors = r.tree(from, "Colors page", |t| {
        t.lines.first().is_some_and(|l| l == "window \"Settings\"")
            && lines_contain(t, "heading \"Colors\"")
    })?;
    if !lines_contain(&colors, "combo box \"Choose your mode\"")
        || !lines_contain(&colors, "toggle switch \"Transparency effects\"")
    {
        return Err(format!("Colors tree lacks its cards: {:?}", colors.lines));
    }
    let combo = step(&mut c, &r, &["tab"], "Choose your mode", |s| {
        s.starts_with("Choose your mode, combo box")
    })?;
    let toggle = step(&mut c, &r, &["tab"], "Transparency effects", |s| {
        s.starts_with("Transparency effects, toggle switch")
    })?;
    let combo_back = step(
        &mut c,
        &r,
        &["shift", "tab"],
        "Choose your mode again",
        |s| s.starts_with("Choose your mode, combo box"),
    )?;
    let option = step(&mut c, &r, &["ret"], "combo list option", |s| {
        s.contains(", list item,")
            && (s.contains("Light, list item") || s.contains("Dark, list item"))
    })?;
    let combo_restored = step(&mut c, &r, &["esc"], "focus restored to the combo", |s| {
        s.starts_with("Choose your mode, combo box")
    })?;

    // File Explorer: the items view and the navigation pane by Tab, its
    // context menu, and focus restoration after the menu and after Start.
    let from = r.offset()?;
    let files = launch(
        &root,
        record,
        &mut c,
        "File Explorer",
        "DOOR_P12 WINDOW_COMMITTED",
    )?;
    let explorer = r.tree(from, "File Explorer window", |t| {
        lines_contain(t, "tree \"Navigation Pane\"")
    })?;
    for needle in [
        "list \"Items View\"",
        "edit \"Address\"",
        "tool bar \"Command bar\"",
        "tool bar \"Title bar\"",
    ] {
        if !lines_contain(&explorer, needle) {
            return Err(format!(
                "File Explorer tree lacks {needle}: {:?}",
                explorer.lines
            ));
        }
    }
    thread::sleep(Duration::from_millis(500));
    let pane = step(&mut c, &r, &["tab"], "navigation pane", |s| {
        s.contains(", tree item")
    })?;
    if !pane.starts_with("Navigation Pane, tree") {
        return Err(format!("Tab did not enter the navigation pane: {pane:?}"));
    }
    let item = step(&mut c, &r, &["tab"], "items view", |s| {
        s.contains(", list item")
    })?;
    let name = named(&item, "list item").ok_or("items view caption names no item")?;
    let menu = step(&mut c, &r, &["shift", "f10"], "context menu", |s| {
        s.contains(", menu item")
    })?;
    let restored = step(&mut c, &r, &["esc"], "focus restored after the menu", |s| {
        s.contains(&format!("{name}, list item"))
    })?;
    let over = step(&mut c, &r, &["meta_l"], "Start over File Explorer", |s| {
        s.starts_with("Search box, edit")
    })?;
    let reactivated = step(
        &mut c,
        &r,
        &["esc"],
        "focus restored on reactivation",
        |s| s.contains(&format!("{name}, list item")),
    )?;

    thread::sleep(Duration::from_secs(1));
    start_shut_down(&mut c)?;
    wait_serial(
        &root,
        record,
        "DOOR_P08 CLEANUP PASS",
        Duration::from_secs(30),
    )?;
    let text = serial(&root, record)?;
    if !text.contains("leaked=0") {
        return Err("desktop resource cleanup reported leaks".into());
    }
    let captions = payloads(&text, 0, assist, "DOOR_ASSIST CAPTION ");
    Ok(json!({
        "pids": {"assist": assist, "settings": settings, "files": files},
        "taskbar_tree": bar.lines,
        "shell": {"start_tab_order": start, "start_shift_tab": back, "search_panel": search, "quick_settings": [quick, quick_next]},
        "settings": {"tab_order": [combo, toggle], "shift_tab": combo_back, "popup_option": option, "restored_after_popup": combo_restored},
        "explorer": {"navigation_pane": pane, "items_view": item, "context_menu": menu, "restored_after_menu": restored, "start": over, "restored_on_reactivation": reactivated},
        "assistive_captions": captions.len(),
        "scope": "Guest keyboard-only run at 1920x1080 100%: captions and trees read through the door.accessibility broker by the Captions client (CAP_ASSISTIVE from its kernel manifest identity); no screenshot recognition. Speech output (P15), Windows UI Automation bridging and the Settings administration pages' tables are not covered"
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captions_are_parsed_for_roles_names_and_restoration() {
        assert!(has_role("Pinned, list, File Explorer, list item, 1 of 6"));
        assert!(has_role(
            "Choose your mode, combo box, collapsed, Light, 1 of 8"
        ));
        assert!(!has_role("Light"));
        assert_eq!(
            named(
                "Items View, list, Desktop, list item, selected, 1 of 9",
                "list item"
            )
            .as_deref(),
            Some("Desktop")
        );
        assert!(same(
            "Items View, list, Desktop, list item",
            "Desktop, list item"
        ));
        assert!(!same("Music, list item", "Desktop, list item"));
    }
}

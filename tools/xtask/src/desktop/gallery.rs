//! `component-gallery` acceptance suite (spec sections 55, 86 and 113): the
//! Door UI gallery on the guest desktop, driven only by real keyboard input,
//! with its semantic tree read back through the `door-assist` assistive
//! client over the door.accessibility broker.
//!
//! Evidence: every gallery page's tree as the assistive client received it,
//! compared with the stored host fixture (`desktop/component-gallery/expected/semantics`,
//! in `outline` form: geometry, focus and visibility removed); focus captions
//! that follow keyboard moves through the broker; a toggle, the password
//! box (never disclosed), the modal dialog (focus restored to its invoker);
//! and captures of every page and of the state matrix in each theme.
//!
//! Requirements outside this module: both binaries staged and launchable
//! from Start search as "Doors UI Gallery" and "Captions", built with the
//! `accessibility` feature, and the window server granting the assistive
//! capability to the Captions identity.
use super::*;

/// Navigation order, slug (`DOOR_GALLERY PAGE`) and page title.
const PAGES: [(&str, &str); 9] = [
    ("basic", "Basic input"),
    ("states", "Control states"),
    ("text", "Text and selection"),
    ("collections", "Collections"),
    ("menus", "Menus and toolbars"),
    ("status", "Status and info"),
    ("layout", "Layout"),
    ("dialogs", "Dialogs"),
    ("accessibility", "Accessibility"),
];
/// The Accessibility page shows live captions, so its tree is not a fixture.
const LIVE_PAGE: &str = "accessibility";
/// Status and info shows an indeterminate ProgressBar and ProgressRing,
/// which animate for as long as the page is shown (Animation effects on), so
/// its capture is still everywhere except inside a bounded moving region.
const ANIMATED_PAGE: &str = "status";
/// Theme picker entries after "Use system setting" (`desktop/component-gallery` THEMES).
const THEMES: [&str; 6] = [
    "Light",
    "Dark",
    "Night sky (contrast)",
    "Aquatic (contrast)",
    "Desert (contrast)",
    "Dusk (contrast)",
];
const WAIT: Duration = Duration::from_secs(20);

pub(super) fn suite(run: &mut Run, image: &str, _test_image: &str) -> Result<Value, String> {
    let disk = run.directory.join("gallery-private.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    // Captions writes its serial trees and captions only on the build's
    // desktop-diagnostics image (production initfs plus its flag file).
    let image = session::diagnostics_image(&run.root, image)?;
    let mut record = session::launch_desktop_accelerated(run, &image, &disk, false, "1920x1080")?;
    let result = drive(run, &record);
    if result.is_err()
        && let Ok(mut c) = connection(&record)
    {
        let _ = scene::capture_settled(run, &mut c, "gallery-failure", &[]);
        let _ = c.execute("quit", json!({}));
    }
    let mut evidence = result?;
    seal_serial(&run.root, &mut record)?;
    let text = serial(&run.root, &record)?;
    if text.contains("secret") {
        return Err("protected PasswordBox text reached the serial log".into());
    }
    evidence["session"] = record;
    Ok(evidence)
}

fn drive(run: &mut Run, record: &Value) -> Result<Value, String> {
    let root = run.root.clone();
    let mut c = connection(record)?;
    thread::sleep(Duration::from_secs(2));
    // The assistive client first (it keeps following focus), then the gallery.
    let assist = launch(&root, record, &mut c, "Live captions", "DOOR_ASSIST READY")?;
    let granted = payloads(&serial(&root, record)?, 0, assist, "DOOR_ASSIST READY ");
    if granted.last().map(String::as_str) != Some("assistive=1") {
        return Err("door-assist runs without the assistive capability (CAP_ASSISTIVE)".into());
    }
    let gallery = launch(
        &root,
        record,
        &mut c,
        "Doors UI Gallery",
        "DOOR_GALLERY READY",
    )?;
    let probe = Probe {
        root: root.clone(),
        record: record.clone(),
        assist,
        gallery,
    };
    // Keyboard only from here. Tab enters the navigation view.
    let start = probe.offset()?;
    key(&mut c, &["tab"])?;
    probe.caption(start, "Basic input, list item")?;
    let mut pages = Vec::new();
    let mut checks = serde_json::Map::new();
    for (i, (slug, title)) in PAGES.iter().enumerate() {
        let before = if i == 0 { 0 } else { probe.offset()? };
        if i > 0 {
            key(&mut c, &["down"])?;
            key(&mut c, &["ret"])?;
        }
        probe.marker(before, &format!("PAGE {slug}"))?;
        let heading = format!("heading \"{title}\" level=1");
        let tree = probe.tree(before, |t| t.lines.iter().any(|l| l.trim() == heading))?;
        let compared = if *slug == LIVE_PAGE {
            false
        } else {
            let path = run.root.join(format!(
                "desktop/component-gallery/expected/semantics/{slug}.txt"
            ));
            let stored =
                fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let expected: Vec<String> = stored.lines().map(outline_line).collect();
            let app_lines = without_title_bar(slug, &tree.lines)?;
            if let Some((n, (want, got))) = expected
                .iter()
                .zip(&app_lines)
                .enumerate()
                .find(|(_, (a, b))| a != b)
            {
                return Err(format!(
                    "{slug}: assistive tree differs from the fixture at line {}: expected {want:?}, read {got:?}",
                    n + 1
                ));
            }
            if expected.len() != app_lines.len() {
                return Err(format!(
                    "{slug}: assistive tree has {} app lines (after the title bar), fixture {}",
                    app_lines.len(),
                    expected.len()
                ));
            }
            true
        };
        if *slug == ANIMATED_PAGE {
            scene::capture_settled_moving(run, &mut c, &format!("gallery-{slug}"), 80)?;
        } else {
            scene::capture_settled(run, &mut c, &format!("gallery-{slug}"), &[])?;
        }
        match *slug {
            "basic" => checks.insert("toggle".into(), toggle(&probe, &mut c)?),
            "states" => checks.insert("themes".into(), themes(run, &probe, &mut c)?),
            "text" => checks.insert("password".into(), password(&probe, &mut c)?),
            "dialogs" => checks.insert("dialog".into(), dialog(&probe, &mut c)?),
            _ => None,
        };
        pages.push(json!({"page":slug,"window":tree.window,"version":tree.version,"lines":tree.lines.len(),"fixture_equal":compared}));
    }
    key(&mut c, &["alt", "f4"])?;
    thread::sleep(Duration::from_secs(1));
    start_shut_down(&mut c)?;
    wait_serial(
        &root,
        record,
        "DOOR_P08 CLEANUP PASS",
        Duration::from_secs(25),
    )?;
    let text = serial(&root, record)?;
    if !text.contains("leaked=0") {
        return Err("desktop resource cleanup reported leaks".into());
    }
    let captions = payloads(&text, 0, assist, "DOOR_ASSIST CAPTION ");
    Ok(json!({
        "pids":{"gallery":gallery,"assist":assist},
        "pages":pages,
        "checks":checks,
        "assistive_captions":captions.len(),
        "scope":"Guest keyboard-only gallery run at 1920x1080 100%; semantic trees read through the door.accessibility broker and compared with host fixtures without geometry, focus or visibility; display scales 125-200% and pixel-level state distinctness are host-measured (desktop/component-gallery tests), not guest evidence"
    }))
}

/// Serial markers of the two processes, authenticated by the kernel's
/// `DOOR_USER pid=` prefix.
struct Probe {
    root: std::path::PathBuf,
    record: Value,
    assist: u64,
    gallery: u64,
}
impl Probe {
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
            if text.contains("APP FAILED")
                || text.contains("KERNEL_PANIC")
                || text.contains("DOOR_GALLERY FAILED")
                || text.contains("DOOR_ASSIST FAILED")
            {
                return Err(format!(
                    "guest failed while waiting for {what}; inspect serial"
                ));
            }
            if Instant::now() > deadline {
                return Err(format!("missing {what}"));
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
    /// A gallery marker (`DOOR_GALLERY <payload>`) after `from`.
    fn marker(&self, from: usize, payload: &str) -> Result<(), String> {
        self.poll(&format!("DOOR_GALLERY {payload}"), |t| {
            payloads(t, from, self.gallery, "DOOR_GALLERY ")
                .iter()
                .any(|p| p == payload)
                .then_some(())
        })
    }
    /// An assistive caption starting with `prefix` after `from`.
    fn caption(&self, from: usize, prefix: &str) -> Result<String, String> {
        self.poll(&format!("assistive caption {prefix:?}"), |t| {
            payloads(t, from, self.assist, "DOOR_ASSIST CAPTION ")
                .into_iter()
                .find(|p| p.starts_with(prefix))
        })
    }
    /// The first gallery tree report after `from` that satisfies `f`.
    fn tree(&self, from: usize, f: impl Fn(&TreeReport) -> bool) -> Result<TreeReport, String> {
        let mut error = None;
        let found = self.poll("assistive tree report", |t| {
            match tree_reports(t, from, self.assist) {
                Ok(r) => r
                    .into_iter()
                    .filter(|r| {
                        r.lines
                            .first()
                            .is_some_and(|l| l == "window \"Doors UI Gallery\"")
                    })
                    .find(|r| f(r)),
                Err(e) => {
                    error = Some(e);
                    Some(TreeReport::default())
                }
            }
        })?;
        error.map_or(Ok(found), Err)
    }
    /// Press `step` until a caption starting with `prefix` arrives.
    fn seek(
        &self,
        c: &mut qmp::Client,
        step: &[&str],
        prefix: &str,
        limit: usize,
    ) -> Result<String, String> {
        for _ in 0..limit {
            let from = self.offset()?;
            key(c, step)?;
            if let Ok(said) = self.poll_short(from, prefix) {
                return Ok(said);
            }
        }
        Err(format!("keyboard focus never reached {prefix:?}"))
    }
    fn poll_short(&self, from: usize, prefix: &str) -> Result<String, String> {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let text = serial(&self.root, &self.record)?;
            let said = payloads(&text, from, self.assist, "DOOR_ASSIST CAPTION ");
            if let Some(s) = said.iter().find(|s| s.starts_with(prefix)) {
                return Ok(s.clone());
            }
            if !said.is_empty() && Instant::now() > deadline {
                return Err(said.join(" | "));
            }
            if Instant::now() > deadline + Duration::from_secs(2) {
                return Err("no caption".into());
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
    /// Back to the selected navigation item (Shift+Tab).
    fn back_to_navigation(&self, c: &mut qmp::Client, title: &str) -> Result<(), String> {
        self.seek(c, &["shift", "tab"], &format!("{title}, list item"), 30)
            .map(drop)
    }
}

/// Basic input: Space on the Wi-Fi switch toggles it; both processes see it.
fn toggle(p: &Probe, c: &mut qmp::Client) -> Result<Value, String> {
    let reached = p.seek(c, &["tab"], "Wi-Fi, toggle switch", 30)?;
    let from = p.offset()?;
    key(c, &["spc"])?;
    p.marker(from, "NOTIFY ToggleSwitch Toggled(true)")?;
    let said = p.caption(from, "On")?;
    p.back_to_navigation(c, "Basic input")?;
    Ok(json!({"focused":reached,"after_space":said}))
}

/// Control states: the Theme picker walks every theme; the matrix is
/// captured in each, then the session theme is restored.
fn themes(run: &mut Run, p: &Probe, c: &mut qmp::Client) -> Result<Value, String> {
    p.seek(c, &["tab"], "Theme, combo box", 5)?;
    let mut shots = Vec::new();
    for name in THEMES {
        let from = p.offset()?;
        key(c, &["down"])?;
        p.marker(from, &format!("THEME {name}"))?;
        let shot = format!(
            "gallery-states-{}",
            name.split(' ').next().unwrap_or(name).to_ascii_lowercase()
        );
        scene::capture_settled(run, c, &shot, &[])?;
        shots.push(shot);
    }
    let from = p.offset()?;
    for _ in THEMES {
        key(c, &["up"])?;
    }
    p.marker(from, "THEME Use system setting")?;
    p.back_to_navigation(c, "Control states")?;
    Ok(json!({"captures":shots}))
}

/// Text: the PasswordBox is read as protected and its text never published.
fn password(p: &Probe, c: &mut qmp::Client) -> Result<Value, String> {
    let said = p.seek(c, &["tab"], "Password, ", 15)?;
    if !said.contains("protected") {
        return Err(format!(
            "PasswordBox caption lacks the protected state: {said:?}"
        ));
    }
    // Every tree the assistive client received: the node is protected and
    // carries no value.
    let reports = tree_reports(&serial(&p.root, &p.record)?, 0, p.assist)?;
    let nodes: Vec<&String> = reports
        .iter()
        .flat_map(|r| &r.lines)
        .filter(|l| l.trim_start().starts_with("edit \"Password\""))
        .collect();
    if nodes.is_empty()
        || nodes
            .iter()
            .any(|l| l.contains("value=") || !l.contains("PROTECTED"))
    {
        return Err(format!(
            "PasswordBox node missing, disclosing content or not PROTECTED: {nodes:?}"
        ));
    }
    p.back_to_navigation(c, "Text and selection")?;
    Ok(json!({"caption":said,"node":nodes[0],"reports_checked":reports.len()}))
}

/// Dialogs: Enter opens the modal dialog (focus moves inside, the dialog is
/// announced), Escape closes it and focus returns to the invoking button.
fn dialog(p: &Probe, c: &mut qmp::Client) -> Result<Value, String> {
    p.seek(c, &["tab"], "Show dialog, button", 5)?;
    let from = p.offset()?;
    key(c, &["ret"])?;
    let opened = p.caption(from, "Save your work?, dialog")?;
    let tree = p.tree(from, |t| {
        t.lines
            .iter()
            .any(|l| l.trim_start().starts_with("dialog \"Save your work?\""))
    })?;
    let modal = tree.lines.iter().any(|l| {
        l.trim_start()
            .starts_with("dialog \"Save your work?\" [MODAL")
    });
    if !modal {
        return Err("dialog node lacks the MODAL state".into());
    }
    let from = p.offset()?;
    key(c, &["esc"])?;
    p.marker(from, "NOTIFY ContentDialog Closed(0)")?;
    let restored = p.caption(from, "Show dialog, button")?;
    p.back_to_navigation(c, "Dialogs")?;
    Ok(json!({"opened":opened,"restored":restored,"modal":modal}))
}

/// Launch from Start search and wait for the process's readiness marker;
/// returns its kernel pid.
pub(super) fn launch(
    root: &std::path::Path,
    record: &Value,
    c: &mut qmp::Client,
    query: &str,
    ready: &str,
) -> Result<u64, String> {
    let before = serial(root, record)?.len();
    key(c, &["meta_l"])?;
    thread::sleep(Duration::from_millis(400));
    type_line(c, query)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let text = serial(root, record)?;
        if let Some(pid) = text
            .get(before..)
            .unwrap_or("")
            .lines()
            .filter_map(user)
            .find(|(_, p)| p.starts_with(ready))
            .map(|(pid, _)| pid)
        {
            return Ok(pid);
        }
        if Instant::now() > deadline {
            return Err(format!(
                "{query} did not start ({ready} missing); is it staged and listed in Start search?"
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

pub(super) fn user(line: &str) -> Option<(u64, &str)> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let (pid, payload) = line.strip_prefix("DOOR_USER pid=")?.split_once(' ')?;
    let number: u64 = pid.parse().ok()?;
    (number > 0 && number.to_string() == pid).then_some((number, payload))
}
/// Payloads of `pid`'s records starting with `prefix`, after byte `from`.
pub(super) fn payloads(text: &str, from: usize, pid: u64, prefix: &str) -> Vec<String> {
    text.get(from..)
        .unwrap_or("")
        .lines()
        .filter_map(user)
        .filter(|(p, _)| *p == pid)
        .filter_map(|(_, l)| l.strip_prefix(prefix))
        .map(String::from)
        .collect()
}

#[derive(Debug, Default, PartialEq)]
pub(super) struct TreeReport {
    pub(super) window: u64,
    pub(super) version: u32,
    pub(super) lines: Vec<String>,
}
/// Complete `DOOR_ASSIST TREE` blocks of `pid` after byte `from`
/// (`TREE window= version= lines=`, `|` lines with `+` continuations, `END`).
pub(super) fn tree_reports(text: &str, from: usize, pid: u64) -> Result<Vec<TreeReport>, String> {
    let mut out = Vec::new();
    let mut open: Option<(TreeReport, usize)> = None;
    for line in payloads(text, from, pid, "DOOR_ASSIST ") {
        if let Some(head) = line.strip_prefix("TREE ") {
            let mut f = head
                .split(' ')
                .map(|t| t.split_once('=').map(|(_, v)| v.parse::<u64>()));
            let (Some(Some(Ok(window))), Some(Some(Ok(version))), Some(Some(Ok(count))), None) =
                (f.next(), f.next(), f.next(), f.next())
            else {
                return Err(format!("malformed tree report header {line:?}"));
            };
            let version = u32::try_from(version).map_err(|_| "tree report version")?;
            open = Some((
                TreeReport {
                    window,
                    version,
                    lines: Vec::new(),
                },
                count as usize,
            ));
        } else if let Some(body) = line
            .strip_prefix("| ")
            .or_else(|| (line == "|").then_some(""))
        {
            if let Some((r, _)) = open.as_mut() {
                r.lines.push(body.into());
            }
        } else if let Some(more) = line.strip_prefix("+ ") {
            if let Some(last) = open.as_mut().and_then(|(r, _)| r.lines.last_mut()) {
                last.push_str(more);
            }
        } else if let Some(end) = line.strip_prefix("END window=") {
            let Some((r, count)) = open.take() else {
                continue;
            };
            if end != r.window.to_string() || r.lines.len() != count {
                return Err(format!("incomplete tree report for window {end}"));
            }
            out.push(r);
        }
    }
    Ok(out)
}
/// `door_accessibility::outline_line`: a stored `dump` line without
/// geometry, the focus marker and the FOCUSED/OFFSCREEN states.
/// The broker inserts the server-drawn title bar (a UI Automation TitleBar
/// with its three buttons) as the window's first child
/// (`door_window::shell_semantics::merge_caption`); the host fixtures hold
/// the gallery's own tree. Checks that block exactly, then returns the tree
/// without it for the fixture comparison.
fn without_title_bar(slug: &str, lines: &[String]) -> Result<Vec<String>, String> {
    const BAR: [&str; 4] = [
        "  tool bar \"Title bar\" value=\"Doors UI Gallery\"",
        "    button \"Minimize\" {INVOKE}",
        "    button \"Maximize\" {INVOKE}",
        "    button \"Close\" {INVOKE}",
    ];
    if lines.len() < 1 + BAR.len() || lines[1..=BAR.len()] != BAR {
        return Err(format!(
            "{slug}: the window's first child is not the server title bar: {:?}",
            lines.iter().take(1 + BAR.len()).collect::<Vec<_>>()
        ));
    }
    Ok(lines[..1]
        .iter()
        .chain(&lines[1 + BAR.len()..])
        .cloned()
        .collect())
}

fn outline_line(line: &str) -> String {
    let line = line.rsplit_once(" @").map_or(line, |(l, _)| l);
    let line = line.replace(" *focus*", "");
    let Some((head, rest)) = line.split_once(" [") else {
        return line;
    };
    let Some((states, tail)) = rest.split_once(']') else {
        return line;
    };
    let kept: Vec<&str> = states
        .split(' ')
        .filter(|s| !matches!(*s, "FOCUSED" | "OFFSCREEN"))
        .collect();
    if kept.is_empty() {
        format!("{head}{tail}")
    } else {
        format!("{head} [{}]{tail}", kept.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tree_reports_need_complete_authenticated_blocks() {
        let text = "DOOR_USER pid=7 DOOR_ASSIST TREE window=3 version=9 lines=2\nDOOR_USER pid=8 DOOR_ASSIST | forged\nDOOR_USER pid=7 DOOR_ASSIST | window \"Doors UI Gallery\"\nDOOR_USER pid=7 DOOR_ASSIST |   text \"long\nDOOR_USER pid=7 DOOR_ASSIST + er\"\nDOOR_USER pid=7 DOOR_ASSIST END window=3\nDOOR_USER pid=7 DOOR_ASSIST TREE window=3 version=10 lines=5\n";
        let r = tree_reports(text, 0, 7).unwrap();
        assert_eq!(
            r,
            [TreeReport {
                window: 3,
                version: 9,
                lines: vec![
                    "window \"Doors UI Gallery\"".into(),
                    "  text \"longer\"".into()
                ]
            }]
        );
        assert!(
            tree_reports(&text.replace("lines=2", "lines=3"), 0, 7).is_err(),
            "count mismatch"
        );
        assert!(tree_reports(&text.replace("version=9", "version=x"), 0, 7).is_err());
        assert!(tree_reports(text, 0, 8).unwrap().is_empty());
        assert_eq!(
            payloads(
                "DOOR_USER pid=07 DOOR_ASSIST READY assistive=1\r\n",
                0,
                7,
                "DOOR_ASSIST READY "
            ),
            Vec::<String>::new()
        );
    }
    #[test]
    fn fixture_lines_normalize_like_the_assistive_outline() {
        assert_eq!(
            outline_line(
                "    button \"Save\" [FOCUSABLE FOCUSED OFFSCREEN] {FOCUS INVOKE} *focus* @10,10 120x32"
            ),
            "    button \"Save\" [FOCUSABLE] {FOCUS INVOKE}"
        );
        assert_eq!(
            outline_line("  text \"Focused\" [OFFSCREEN] @1,2 3x4"),
            "  text \"Focused\""
        );
        assert_eq!(
            outline_line(
                "  slider \"Volume\" value=\"40\" [FOCUSABLE HORIZONTAL HAS_RANGE] range=0..100000@40000 @5,6 7x8"
            ),
            "  slider \"Volume\" value=\"40\" [FOCUSABLE HORIZONTAL HAS_RANGE] range=0..100000@40000"
        );
        assert_eq!(
            outline_line("window \"Doors UI Gallery\""),
            "window \"Doors UI Gallery\""
        );
    }
}

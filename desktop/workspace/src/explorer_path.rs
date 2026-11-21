//! Explorer's display namespace over existing authenticated Storage directories.
//! C: is the existing `/compat` view; its Users alias names `/users`. This is
//! pathname presentation, not a new volume, known-folder API or access grant.
extern crate alloc;
use alloc::{format, string::String, vec::Vec};

#[derive(Debug, PartialEq, Eq)]
pub enum AddressError {
    Invalid,
    Unsupported,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Address {
    Overview,
    Directory {
        start: String,
        parts: Vec<String>,
        windows: bool,
    },
}

pub fn display(path: &str) -> String {
    if path == "/compat" {
        "C:\\".into()
    } else if let Some(tail) = path.strip_prefix("/compat/") {
        format!("C:\\{}", tail.replace('/', "\\"))
    } else if path == "/users" {
        "C:\\Users".into()
    } else if let Some(tail) = path.strip_prefix("/users/") {
        format!("C:\\Users\\{}", tail.replace('/', "\\"))
    } else {
        path.into()
    }
}
pub fn parent(path: &str, windows: bool) -> String {
    if windows && matches!(path, "/users" | "/compat") {
        return "/compat".into();
    }
    path[..path.rfind('/').unwrap_or(0).max(1)].into()
}
/// Windows environment variables a typed location may start with, as the
/// profile-relative (`~`) or C: paths they name for the signed-in user.
const VARIABLES: [(&str, &str); 16] = [
    ("USERPROFILE", "~"),
    ("HOMEPATH", "~"),
    ("HOMEDRIVE", "C:"),
    ("SYSTEMDRIVE", "C:"),
    ("APPDATA", "~\\AppData\\Roaming"),
    ("LOCALAPPDATA", "~\\AppData\\Local"),
    ("TEMP", "~\\AppData\\Local\\Temp"),
    ("TMP", "~\\AppData\\Local\\Temp"),
    ("PUBLIC", "C:\\Users\\Public"),
    ("WINDIR", "C:\\Windows"),
    ("SYSTEMROOT", "C:\\Windows"),
    ("PROGRAMFILES", "C:\\Program Files"),
    ("PROGRAMFILES(X86)", "C:\\Program Files (x86)"),
    ("PROGRAMDATA", "C:\\ProgramData"),
    ("ALLUSERSPROFILE", "C:\\ProgramData"),
    ("COMMONPROGRAMFILES", "C:\\Program Files\\Common Files"),
];
/// `shell:` known folders and the paths they name (the profile folders and
/// This PC are handled directly by `parse`).
const SHELL_FOLDERS: [(&str, &str); 30] = [
    ("appdata", "~\\AppData\\Roaming"),
    ("roamingappdata", "~\\AppData\\Roaming"),
    ("local appdata", "~\\AppData\\Local"),
    ("localappdata", "~\\AppData\\Local"),
    (
        "startup",
        "~\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\Startup",
    ),
    (
        "programs",
        "~\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs",
    ),
    (
        "start menu",
        "~\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu",
    ),
    ("sendto", "~\\AppData\\Roaming\\Microsoft\\Windows\\SendTo"),
    ("recent", "~\\AppData\\Roaming\\Microsoft\\Windows\\Recent"),
    (
        "templates",
        "~\\AppData\\Roaming\\Microsoft\\Windows\\Templates",
    ),
    ("favorites", "~\\Favorites"),
    ("links", "~\\Links"),
    ("contacts", "~\\Contacts"),
    ("searches", "~\\Searches"),
    ("savedgames", "~\\Saved Games"),
    ("saved games", "~\\Saved Games"),
    ("3d objects", "~\\3D Objects"),
    ("screenshots", "~\\Pictures\\Screenshots"),
    ("camera roll", "~\\Pictures\\Camera Roll"),
    ("usersfilesfolder", "~"),
    ("userprofiles", "C:\\Users"),
    ("public", "C:\\Users\\Public"),
    ("common desktop", "C:\\Users\\Public\\Desktop"),
    ("common documents", "C:\\Users\\Public\\Documents"),
    ("windows", "C:\\Windows"),
    ("system", "C:\\Windows\\System32"),
    ("systemx86", "C:\\Windows\\SysWOW64"),
    ("programfiles", "C:\\Program Files"),
    ("programfilesx86", "C:\\Program Files (x86)"),
    ("common appdata", "C:\\ProgramData"),
];
/// A typed location starting with an environment variable (`%TEMP%\x`) or
/// a `shell:` known folder (`shell:startup`), spelled as the `~` or C:
/// path it names; None when it starts with neither.
pub fn expand(value: &str) -> Option<String> {
    if let Some(rest) = value.strip_prefix('%')
        && let Some((name, tail)) = rest.split_once('%')
    {
        let (_, path) = VARIABLES
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))?;
        return Some(format!("{path}{tail}"));
    }
    let rest = value
        .get(..6)
        .filter(|s| s.eq_ignore_ascii_case("shell:"))
        .map(|_| &value[6..])?;
    let (name, tail) = match rest.find(['\\', '/']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let (_, path) = SHELL_FOLDERS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))?;
    Some(format!("{path}{tail}"))
}
pub fn parse(input: &str, cwd: &str, home: &str) -> Result<Address, AddressError> {
    let value = input.trim();
    let value = if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        &value[1..value.len() - 1]
    } else {
        value
    };
    let expanded = expand(value);
    let value = expanded.as_deref().unwrap_or(value);
    if value.is_empty()
        || value.len() > 192
        || !cwd.starts_with('/')
        || value.chars().any(|c| c < ' ' || c == '"')
    {
        return Err(AddressError::Invalid);
    }
    if value.eq_ignore_ascii_case("This PC") {
        return Ok(Address::Overview);
    }
    if value.eq_ignore_ascii_case("Home") {
        return directory(home, "", false);
    }
    // 26H2: "~" is the user profile folder, alone or as "~/sub/path"
    // (either separator). "~name" (another user's profile) is not supported.
    if let Some(tail) = value.strip_prefix('~') {
        if !tail.is_empty() && !tail.starts_with(['/', '\\']) {
            return Err(AddressError::Unsupported);
        }
        return directory(home, tail, true);
    }
    if value
        .get(..13)
        .is_some_and(|s| s.eq_ignore_ascii_case("%USERPROFILE%"))
    {
        let tail = &value[13..];
        if !tail.is_empty() && !tail.starts_with(['/', '\\']) {
            return Err(AddressError::Invalid);
        }
        return directory(home, tail, true);
    }
    if value
        .get(..6)
        .is_some_and(|s| s.eq_ignore_ascii_case("shell:"))
    {
        let folder = match value[6..].to_ascii_lowercase().as_str() {
            "profile" => "",
            "desktop" => "Desktop",
            "personal" | "documents" => "Documents",
            "downloads" => "Downloads",
            "mymusic" | "music" => "Music",
            "mypictures" | "pictures" => "Pictures",
            "myvideo" | "videos" => "Videos",
            "mycomputerfolder" => return Ok(Address::Overview),
            _ => return Err(AddressError::Unsupported),
        };
        return directory(home, folder, true);
    }
    if ["\\\\", "//", "\\??\\"]
        .iter()
        .any(|prefix| value.starts_with(prefix))
    {
        return Err(AddressError::Unsupported);
    }
    if value.as_bytes().get(1) == Some(&b':') {
        if !value.as_bytes()[0].eq_ignore_ascii_case(&b'c') {
            return Err(AddressError::Unsupported);
        }
        let tail = &value[2..];
        if tail.starts_with(['/', '\\']) {
            return directory("/compat", tail, true);
        }
        if display(cwd).starts_with("C:") {
            return directory(cwd, tail, true);
        }
        return Err(AddressError::Unsupported);
    }
    if value.starts_with('\\') {
        directory("/compat", value, true)
    } else if value.starts_with('/') {
        directory("/", value, false)
    } else {
        directory(cwd, value, display(cwd).starts_with("C:"))
    }
}
fn directory(start: &str, tail: &str, windows: bool) -> Result<Address, AddressError> {
    let mut parts = Vec::new();
    for raw in tail
        .split(|c| c == '/' || windows && c == '\\')
        .filter(|s| !s.is_empty())
    {
        let part = if windows && !matches!(raw, "." | "..") {
            raw.trim_end_matches([' ', '.'])
        } else {
            raw
        };
        if part.is_empty()
            || part.encode_utf16().count() > 48
            || part.chars().any(|c| c < ' ' || "\\:\"*<>?|".contains(c))
            || windows && device(part)
        {
            return Err(AddressError::Invalid);
        }
        // Keep dot/parent components in order. The caller checks each directory
        // through Storage before processing the next component, including '..'.
        parts.push(part.into());
    }
    if parts.len() > 16 {
        return Err(AddressError::Invalid);
    }
    Ok(Address::Directory {
        start: start.into(),
        parts,
        windows,
    })
}
fn device(part: &str) -> bool {
    let name = part.split('.').next().unwrap_or("");
    ["con", "prn", "aux", "nul"]
        .iter()
        .any(|s| name.eq_ignore_ascii_case(s))
        || name
            .get(..3)
            .is_some_and(|s| s.eq_ignore_ascii_case("com") || s.eq_ignore_ascii_case("lpt"))
            && matches!(
                &name[3..],
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
}

/// Windows 11 25H2 address-bar breadcrumb metrics at 96 DPI, measured from a
/// 150% capture ("monitor > Desktop > Doors OS >"): the 16 px location glyph
/// starts 10 px into the box; every chevron center sits 22 px after the text
/// (or icon) before it and 22 px before the next text, so the spacing is
/// constant whatever the segment widths.
pub mod crumb {
    /// Location-glyph plate (glyph at +8).
    pub const ICON: u32 = 32;
    /// Text inset before the label inside its hover plate.
    pub const PAD: u32 = 8;
    /// Plate end after the label advance. Segoe UI advances end 2-3 px past
    /// the ink, so the ink-to-chevron gap matches the reference (about 22 px
    /// from the last ink to the chevron center, 23 px to the next ink).
    pub const PAD_END: u32 = 5;
    /// Chevron plate; the glyph is centered in it.
    pub const CHEVRON: u32 = 28;
    /// Longest segment text before it is shortened with an ellipsis.
    pub const MAX_TEXT: u32 = 240;
    /// Box inset of the first plate and vertical plate inset.
    pub const INSET: i32 = 2;
}
/// One visible breadcrumb segment: text plate and the chevron plate after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CrumbItem {
    /// Index into the caller's full crumb list.
    pub index: usize,
    pub text: crate::explorer_layout::Bounds,
    pub chevron: crate::explorer_layout::Bounds,
    /// Width available to the label (it is shortened to fit).
    pub label_width: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrumbLayout {
    pub icon: crate::explorer_layout::Bounds,
    /// Chevron right after the location glyph (top-level locations).
    pub root: crate::explorer_layout::Bounds,
    /// "..." plate listing the leading segments that do not fit.
    pub overflow: Option<crate::explorer_layout::Bounds>,
    /// Number of leading crumbs hidden behind the overflow plate.
    pub hidden: usize,
    pub items: Vec<CrumbItem>,
}
/// Lays out crumbs of the given text widths in `address`. Leading segments
/// move into the overflow list until the rest fits; the current location is
/// always shown, shortened if it alone is too wide.
pub fn crumb_layout(
    address: crate::explorer_layout::Bounds,
    widths: &[u32],
    overflow_width: u32,
) -> CrumbLayout {
    use crate::explorer_layout::Bounds;
    use crumb::*;
    let y = address.y + INSET;
    let height = address.height.saturating_sub(2 * INSET as u32);
    let plate = |x: i32, width: u32| Bounds {
        x,
        y,
        width,
        height,
    };
    let left = address.x + INSET;
    let icon = plate(left, ICON);
    let root = plate(left + ICON as i32, CHEVRON);
    let start = root.x + CHEVRON as i32;
    let right = address.x + address.width as i32 - INSET;
    // The segment text limit widens with the applied text size.
    let max_text = crate::palette::text_len(MAX_TEXT);
    let cost = |w: u32| w.min(max_text) + PAD + PAD_END + CHEVRON;
    let total = |from: usize| widths[from..].iter().map(|w| cost(*w)).sum::<u32>();
    let room = (right - start).max(0) as u32;
    let mut hidden = 0;
    if total(0) > room {
        hidden = 1;
        while hidden + 1 < widths.len() && cost(overflow_width) + total(hidden) > room {
            hidden += 1;
        }
        hidden = hidden.min(widths.len().saturating_sub(1));
    }
    let mut x = start;
    let overflow = (hidden > 0).then(|| {
        let r = plate(x, overflow_width + PAD + PAD_END);
        x += (overflow_width + PAD + PAD_END + CHEVRON) as i32;
        r
    });
    let mut items = Vec::new();
    for (index, w) in widths.iter().enumerate().skip(hidden) {
        let space = (right - x).max(0) as u32;
        let fits = space.saturating_sub(PAD + PAD_END + CHEVRON);
        let label_width = (*w).min(max_text).min(fits);
        if label_width == 0 && index + 1 < widths.len() {
            break;
        }
        let text = plate(x, label_width + PAD + PAD_END);
        let chevron = plate(text.x + text.width as i32, CHEVRON);
        x = chevron.x + CHEVRON as i32;
        items.push(CrumbItem {
            index,
            text,
            chevron,
            label_width,
        });
    }
    CrumbLayout {
        icon,
        root,
        overflow,
        hidden,
        items,
    }
}
/// Longest prefix of `text` that fits `width` with a trailing ellipsis.
pub fn ellipsize(text: &str, width: u32, measure: impl Fn(&str) -> u32) -> String {
    if measure(text) <= width {
        return text.into();
    }
    let mut end = text.len();
    while end > 0 {
        end -= 1;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let candidate = format!("{}...", text[..end].trim_end());
        if measure(&candidate) <= width {
            return candidate;
        }
    }
    String::new()
}

#[cfg(test)]
mod crumb_tests {
    use super::*;
    use crate::explorer_layout::Bounds;
    const BOX: Bounds = Bounds {
        x: 242,
        y: 53,
        width: 600,
        height: 32,
    };
    #[test]
    fn chevrons_keep_constant_spacing_from_the_reference() {
        let l = crumb_layout(BOX, &[52, 60, 7], 12);
        assert_eq!(l.hidden, 0);
        // Icon glyph at box + 10, root chevron center 22 px after it.
        assert_eq!(l.icon.x + 8, BOX.x + 10);
        assert_eq!(l.root.x + crumb::CHEVRON as i32 / 2, BOX.x + 10 + 16 + 22);
        for item in &l.items {
            let text_end = item.text.x + (crumb::PAD + item.label_width) as i32;
            let center = item.chevron.x + crumb::CHEVRON as i32 / 2;
            assert_eq!(
                center - text_end,
                (crumb::PAD_END + crumb::CHEVRON / 2) as i32
            );
        }
        for pair in l.items.windows(2) {
            let center = pair[0].chevron.x + crumb::CHEVRON as i32 / 2;
            assert_eq!(pair[1].text.x + crumb::PAD as i32 - center, 22);
        }
    }
    #[test]
    fn narrow_bars_hide_leading_segments_but_keep_the_location() {
        for width in [40u32, 120, 200, 320, 480] {
            let b = Bounds { width, ..BOX };
            let widths = [52, 90, 30, 140, 70];
            let l = crumb_layout(b, &widths, 12);
            let right = b.x + b.width as i32;
            assert_eq!(l.items.last().unwrap().index, widths.len() - 1, "{width}");
            for item in &l.items {
                assert!(
                    item.chevron.x + item.chevron.width as i32 <= right.max(item.chevron.x + 28)
                );
            }
            assert_eq!(l.overflow.is_some(), l.hidden > 0);
            if l.hidden > 0 {
                assert_eq!(l.items[0].index, l.hidden);
            }
        }
        assert_eq!(
            ellipsize("Documents", 1000, |s| s.len() as u32 * 7),
            "Documents"
        );
        let short = ellipsize("A very long folder name", 70, |s| s.len() as u32 * 7);
        assert!(short.ends_with("...") && short.len() * 7 <= 70);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plan(value: &str) -> (String, Vec<String>, bool) {
        match parse(value, "/users/1001", "/users/1001").unwrap() {
            Address::Directory {
                start,
                parts,
                windows,
            } => (start, parts, windows),
            Address::Overview => panic!("directory expected"),
        }
    }
    #[test]
    fn display_and_input_share_existing_drive_and_users_aliases() {
        assert_eq!(
            display("/users/1001/Documents"),
            "C:\\Users\\1001\\Documents"
        );
        assert_eq!(display("/compat/Windows/System32"), "C:\\Windows\\System32");
        assert_eq!(display("/users-other/a"), "/users-other/a");
        assert_eq!(
            plan("c:\\uSeRs\\1001\\documents"),
            (
                "/compat".into(),
                alloc::vec!["uSeRs".into(), "1001".into(), "documents".into()],
                true
            )
        );
        assert_eq!(plan("C:"), ("/users/1001".into(), Vec::new(), true));
        assert_eq!(parent("/users", true), "/compat");
        assert_eq!(parent("/compat", true), "/compat");
        assert_eq!(parent("/users", false), "/");
    }
    #[test]
    fn relative_paths_keep_traversal_and_native_case_semantics() {
        assert_eq!(
            plan("locked\\..\\Documents").1,
            ["locked", "..", "Documents"]
        );
        assert_eq!(plan("Documents. ").1, ["Documents"]);
        assert_eq!(
            plan("/users/1001/Docs/../Documents").1,
            ["users", "1001", "Docs", "..", "Documents"]
        );
        assert!(!plan("/users/1001").2);
        assert_eq!(
            plan("\"C:\\Users\\1001\\My folder\"").1,
            ["Users", "1001", "My folder"]
        );
    }
    #[test]
    fn tilde_is_the_profile_folder_alone_or_with_a_path() {
        assert_eq!(plan("~"), ("/users/1001".into(), Vec::new(), true));
        assert_eq!(
            plan("~/Documents/notes"),
            (
                "/users/1001".into(),
                alloc::vec!["Documents".into(), "notes".into()],
                true
            )
        );
        assert_eq!(plan("~\\Desktop").1, ["Desktop"]);
        assert_eq!(plan("~/../1001").1, ["..", "1001"]);
        assert_eq!(
            parse("~guest", "/users/1001", "/users/1001"),
            Err(AddressError::Unsupported)
        );
        // A folder literally named "~" stays reachable by a relative path.
        assert_eq!(plan("./~").1, [".", "~"]);
    }
    #[test]
    fn known_folder_shortcuts_are_explicit_and_bounded_to_existing_profile() {
        assert_eq!(
            plan("%userprofile%\\Documents"),
            ("/users/1001".into(), alloc::vec!["Documents".into()], true)
        );
        assert_eq!(
            plan("shell:Downloads"),
            ("/users/1001".into(), alloc::vec!["Downloads".into()], true)
        );
        // Environment variables and shell: known folders name their paths.
        assert_eq!(
            plan("%TEMP%"),
            (
                "/users/1001".into(),
                alloc::vec!["AppData".into(), "Local".into(), "Temp".into()],
                true
            )
        );
        assert_eq!(
            plan("shell:startup").1.last().map(String::as_str),
            Some("Startup")
        );
        assert_eq!(plan("%windir%\\System32").0, "/compat");
        assert_eq!(parse("This PC", "/", "/users/1001"), Ok(Address::Overview));
        assert_eq!(
            parse("shell:MyComputerFolder", "/", "/users/1001"),
            Ok(Address::Overview)
        );
        for bad in [
            "%USERPROFILE%extra",
            "shell:NoSuchFolder",
            "D:\\",
            "\\\\server\\share",
            "//server/share",
            "\\\\?\\C:\\",
            "C:\\Users\\1001\\CON.txt",
            "C:\\x:stream",
            "",
            "x\0y",
        ] {
            assert!(parse(bad, "/users/1001", "/users/1001").is_err(), "{bad:?}");
        }
    }
}

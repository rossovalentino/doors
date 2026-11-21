#![no_std]
//! The Settings route table and the legacy redirect map (P14 WS-D).
//!
//! Every Windows legacy entry point a user can still type — Run commands,
//! control panel applets and canonical names, MMC snap-ins, shell folders and
//! `ms-settings:` URIs — resolves to a Settings page and, where the page has
//! several topics, a section of it. The shell's Run box, Start search and
//! launcher call [`resolve`]; Settings decodes [`navigate_arg`] values from
//! SETTINGS_NAVIGATE and highlights the section the way Windows highlights a
//! search target. Allocation-free and shared by the shell and Settings.
mod page;
mod table;
pub use page::{Page, contains};
pub use table::{Kind, REDIRECTS, Redirect, SECTIONS, Section};

/// A page number as stored and sent over SETTINGS_NAVIGATE.
pub type PageId = Page;

/// Resolve a legacy command, applet, snap-in, shell folder or Settings URI
/// to the page (and section anchor) that replaces it. `None` means the text
/// is not a known legacy Settings entry point (an app such as `notepad`, or
/// a Doors command), so the caller should treat it as something else.
pub fn resolve(command: &str) -> Option<(Page, Option<&'static str>)> {
    lookup(command).map(|r| (r.page, r.section))
}
/// The redirect row [`resolve`] uses for `command`.
pub fn lookup(command: &str) -> Option<&'static Redirect> {
    let mut buffer = [0u8; 160];
    let mut alternate = [0u8; 160];
    let (key, fallback) = normalize(command, &mut buffer, &mut alternate)?;
    find(key).or_else(|| fallback.and_then(find)).or_else(|| {
        // Windows opens Settings at Home for any ms-settings: URI it
        // does not know, rather than failing.
        key.starts_with("ms-settings:")
            .then(|| find("ms-settings:"))
            .flatten()
    })
}
fn find(key: &str) -> Option<&'static Redirect> {
    REDIRECTS.iter().find(|r| r.key == key)
}
/// The section with `anchor` on `page`.
pub fn section(page: Page, anchor: &str) -> Option<&'static Section> {
    SECTIONS
        .iter()
        .find(|s| s.page == page && s.anchor == anchor)
}
/// SETTINGS_NAVIGATE argument: the page number in the low 16 bits and the
/// section (its index in [`SECTIONS`] plus one, 0 for none) in the high 16.
/// A plain page number stays a valid argument.
pub fn navigate_arg(page: Page, section: Option<&str>) -> u32 {
    let index = section
        .and_then(|anchor| {
            SECTIONS
                .iter()
                .position(|s| s.page == page && s.anchor == anchor)
        })
        .map_or(0, |i| i as u32 + 1);
    page.index() | index << 16
}
/// Decode [`navigate_arg`]. A section of another page, or an unknown page or
/// section, is rejected (`None`): deep links never land somewhere else.
pub fn decode_navigate(argument: u32) -> Option<(Page, Option<&'static Section>)> {
    let page = Page::from_index((argument & 0xffff) as usize)?;
    match argument >> 16 {
        0 => Some((page, None)),
        n => {
            let section = SECTIONS.get(n as usize - 1)?;
            (section.page == page).then_some((page, Some(section)))
        }
    }
}
/// The program Windows elevates for a route (its consent prompt's name), or
/// None when the route opens without administrator approval. Registry
/// Editor and System Configuration elevate themselves; the management
/// snap-ins run in the Management Console, which asks for the highest
/// available rights; the firewall's advanced console likewise.
pub fn elevation(argument: u32) -> Option<&'static str> {
    let (page, section) = decode_navigate(argument)?;
    match page {
        Page::Registry => Some("Registry Editor"),
        Page::SystemConfiguration => Some("System Configuration"),
        Page::GroupPolicy
        | Page::DiskManagement
        | Page::UsersGroups
        | Page::Services
        | Page::TaskScheduler
        | Page::PermissionsSharing => Some("Macrohard Management Console"),
        Page::NetworkAdministration if section.is_some_and(|s| s.anchor == "firewall") => {
            Some("Macrohard Management Console")
        }
        _ => None,
    }
}
/// What a search query found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// A page by its own name.
    Page(Page),
    /// A section by its title.
    Section(&'static Section),
    /// A page or section by one of its plain-language keywords.
    Keyword(Page),
    /// A legacy tool, window or command ("Device Manager", "sysdm.cpl").
    Legacy(&'static Redirect),
}
impl Hit {
    pub fn page(self) -> Page {
        match self {
            Self::Page(p) | Self::Keyword(p) => p,
            Self::Section(s) => s.page,
            Self::Legacy(r) => r.page,
        }
    }
    pub fn anchor(self) -> Option<&'static str> {
        match self {
            Self::Page(_) | Self::Keyword(_) => None,
            Self::Section(s) => Some(s.anchor),
            Self::Legacy(r) => r.section,
        }
    }
    /// Destination identity: two hits that open the same place are one result.
    pub fn target(self) -> (Page, Option<&'static str>) {
        (self.page(), self.anchor())
    }
}
/// Search every page name, section title, keyword and legacy name, ranked
/// as Windows Settings ranks: page names (those starting with the query
/// first), then section titles, then keywords, then legacy names and
/// commands. Each destination appears once.
pub fn search(query: &str) -> impl Iterator<Item = Hit> + '_ {
    let query = query.trim();
    let live = !query.is_empty();
    let starts = move |text: &str| {
        text.len() >= query.len()
            && text.as_bytes()[..query.len()].eq_ignore_ascii_case(query.as_bytes())
    };
    let prefix = Page::ALL
        .into_iter()
        .filter(move |p| live && starts(p.nav_title()))
        .map(Hit::Page);
    let names = Page::ALL
        .into_iter()
        .filter(move |p| live && !starts(p.nav_title()) && p.name_matches(query))
        .map(Hit::Page);
    let sections = SECTIONS
        .iter()
        .filter(move |s| live && contains(s.title, query))
        .map(Hit::Section);
    let keywords = Page::ALL
        .into_iter()
        .filter(move |p| live && p.matches_keywords(query))
        .map(Hit::Keyword);
    let legacy = REDIRECTS
        .iter()
        .filter(move |r| {
            live && query.len() >= 3 && (contains(r.name, query) || contains(r.key, query))
        })
        .map(Hit::Legacy);
    let mut seen: [(u16, u16); 512] = [(u16::MAX, 0); 512];
    let mut count = 0;
    prefix
        .chain(names)
        .chain(sections)
        .chain(keywords)
        .chain(legacy)
        .filter(move |hit| {
            let (page, anchor) = hit.target();
            let key = (
                page as u16,
                anchor.map_or(0, |a| {
                    SECTIONS
                        .iter()
                        .position(|s| s.anchor == a && s.page == page)
                        .map_or(0, |i| i as u16 + 1)
                }),
            );
            if seen[..count].contains(&key) {
                return false;
            }
            if count < seen.len() {
                seen[count] = key;
                count += 1;
            }
            true
        })
}
/// Normalized lookup key(s) of a typed command: lower case, quotes and
/// paths removed, `.exe` dropped, `control /name X` reduced to `x`,
/// `rundll32 shell32.dll,Control_RunDLL x.cpl` and `mmc x.msc` to their
/// target, applet tabs written `x.cpl,N`. The second key, when present, is
/// a broader form to try when the first is unknown (an applet without its
/// tab, a command without its arguments).
pub fn normalize<'a>(
    command: &str,
    out: &'a mut [u8; 160],
    alternate: &'a mut [u8; 160],
) -> Option<(&'a str, Option<&'a str>)> {
    let mut lower = [0u8; 160];
    let mut n = 0;
    let mut space = false;
    for b in command.trim().bytes() {
        if !b.is_ascii() || n >= lower.len() {
            return None;
        }
        let b = if b == b'"' {
            continue;
        } else if b.is_ascii_whitespace() {
            if n == 0 || space {
                continue;
            }
            space = true;
            b' '
        } else {
            space = false;
            b.to_ascii_lowercase()
        };
        lower[n] = b;
        n += 1;
    }
    while n > 0 && lower[n - 1] == b' ' {
        n -= 1;
    }
    let text = core::str::from_utf8(&lower[..n]).ok()?;
    if text.is_empty() {
        return None;
    }
    if text.starts_with("ms-settings:") {
        let end = text.find(['?', '#']).unwrap_or(text.len());
        return Some((copy(out, &[&text[..end]])?, None));
    }
    let (first, rest) = match text.split_once(' ') {
        Some((a, b)) => (a, b.trim()),
        None => (text, ""),
    };
    // Program name without directories or extension.
    let program = first.rsplit(['\\', '/']).next().unwrap_or(first);
    let program = program.strip_suffix(".exe").unwrap_or(program);
    if program.starts_with("shell:") || program.starts_with("::{") {
        return shell(text, out);
    }
    match program {
        "explorer" if rest.starts_with("shell:") || rest.starts_with("::{") => shell(rest, out),
        "mmc" if !rest.is_empty() => target(rest, out, alternate),
        "rundll32" if !rest.is_empty() => {
            // rundll32 shell32.dll,Control_RunDLL desk.cpl,,1
            let rest = match rest.find("control_rundll") {
                Some(i) => rest[i + "control_rundll".len()..].trim(),
                None => rest,
            };
            target(rest, out, alternate)
        }
        "control" => {
            if rest.is_empty() {
                return Some((copy(out, &["control"])?, None));
            }
            if let Some(name) = rest
                .strip_prefix("/name ")
                .or_else(|| rest.strip_prefix("-name "))
            {
                let name = name.split(' ').next().unwrap_or(name);
                return Some((copy(out, &[name])?, None));
            }
            let target_text = rest.split(' ').next().unwrap_or(rest);
            if target_text.ends_with(".cpl") || target_text.contains(".cpl,") {
                return target(rest, out, alternate);
            }
            Some((copy(out, &["control ", rest])?, None))
        }
        _ => {
            if first.ends_with(".cpl") || first.contains(".cpl,") || first.ends_with(".msc") {
                return target(text, out, alternate);
            }
            if rest.is_empty() {
                Some((copy(out, &[program])?, None))
            } else {
                Some((
                    copy(out, &[program, " ", rest])?,
                    Some(copy(alternate, &[program])?),
                ))
            }
        }
    }
}
/// `x.cpl[,@0][,N]` or `x.msc` (possibly followed by arguments).
fn target<'a>(
    text: &str,
    out: &'a mut [u8; 160],
    alternate: &'a mut [u8; 160],
) -> Option<(&'a str, Option<&'a str>)> {
    let text = text.trim();
    let (head, tail) = text.split_once(' ').unwrap_or((text, ""));
    // Only the program or applet token carries a path.
    let file = head.rsplit(['\\', '/']).next().unwrap_or(head);
    if let Some(at) = file.find(".cpl") {
        let base = &file[..at + 4];
        // Arguments after the applet: ",,N", ",@0,N", " ,N" or a word
        // (rundll32 sysdm.cpl,EditEnvironmentVariables).
        let word = file[at + 4..]
            .split(',')
            .chain(tail.split([',', ' ']))
            .map(str::trim)
            .rfind(|p| !p.is_empty() && !p.starts_with('@'));
        return match word {
            Some(word) => Some((
                copy(out, &[base, ",", word])?,
                Some(copy(alternate, &[base])?),
            )),
            None => Some((copy(out, &[base])?, None)),
        };
    }
    if tail.is_empty() {
        Some((copy(out, &[file])?, None))
    } else {
        Some((
            copy(out, &[file, " ", tail])?,
            Some(copy(alternate, &[file])?),
        ))
    }
}
fn shell<'a>(text: &str, out: &'a mut [u8; 160]) -> Option<(&'a str, Option<&'a str>)> {
    let text = text.trim();
    let text = if text.starts_with("::{") {
        return Some((copy(out, &["shell:", text])?, None));
    } else {
        text
    };
    Some((copy(out, &[text])?, None))
}
fn copy<'a>(out: &'a mut [u8; 160], parts: &[&str]) -> Option<&'a str> {
    let mut n = 0;
    for part in parts {
        let bytes = part.as_bytes();
        out.get_mut(n..n + bytes.len())?.copy_from_slice(bytes);
        n += bytes.len();
    }
    core::str::from_utf8(&out[..n]).ok()
}
#[cfg(test)]
mod tests;

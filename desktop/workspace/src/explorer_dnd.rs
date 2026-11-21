//! File Explorer drag and drop (Windows 11), the decisions only: when a
//! press becomes a drag, what a FILES offer carries, what a folder offers
//! as a drop target ("Move to X" by default on the same volume, Ctrl copies,
//! Shift moves; the server applies the keys with `dnd::resolve`), and where
//! each dropped item lands. `files/dnd.rs` does the storage work.
use alloc::{format, string::String, vec::Vec};
use door_service_protocol::{Code, clipboard as cb, dnd};

/// SM_CXDRAG/SM_CYDRAG: the drag rectangle (logical px) centred on the
/// press; leaving it with the button held starts a drag (DragDetect).
pub const DRAG_RECT: i32 = 4;
/// A press at `press` held to `at` has become a drag.
pub fn drag_started(press: (i32, i32), at: (i32, i32)) -> bool {
    (at.0 - press.0).abs() > DRAG_RECT / 2 || (at.1 - press.1).abs() > DRAG_RECT / 2
}
/// The FILES payload offering `paths`.
pub fn payload(paths: &[String]) -> Result<Vec<u8>, Code> {
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let mut out = alloc::vec![0u8; cb::MAX_FILES_BYTES];
    let n = cb::encode_files(&refs, &mut out)?;
    out.truncate(n);
    Ok(out)
}
/// The paths of a dropped FILES payload.
pub fn paths(payload: &[u8]) -> Result<Vec<String>, Code> {
    cb::validate(cb::FILES, payload)?;
    cb::files(payload).map(|p| p.map(String::from)).collect()
}
pub fn parent(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) => "/",
        Some(i) => &path[..i],
        None => "",
    }
}
pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
pub fn join(folder: &str, name: &str) -> String {
    if folder.ends_with('/') {
        format!("{folder}{name}")
    } else {
        format!("{folder}/{name}")
    }
}
/// `path` is `folder` or lies inside it.
fn within(path: &str, folder: &str) -> bool {
    path == folder
        || path
            .strip_prefix(folder)
            .is_some_and(|rest| rest.starts_with('/') || folder.ends_with('/'))
}
/// Volume mount roots, longest first. The storage service mounts one
/// persistent volume (presented as C:), so every Door path shares it; a
/// second mount adds its root here and drops onto it copy by default.
const VOLUMES: [&str; 1] = ["/"];
pub fn volume(path: &str) -> &'static str {
    VOLUMES
        .into_iter()
        .find(|root| within(path, root))
        .unwrap_or("/")
}
/// What a folder offers for `sources` dropped into `dest`: (effects,
/// default). Nothing for a folder dropped onto itself or into its own
/// subtree, or for items already in `dest` (Windows does nothing there);
/// otherwise copy or move, moving by default on the same volume.
pub fn status(sources: &[String], dest: &str) -> (u32, u32) {
    if sources.is_empty()
        || dest.is_empty()
        || sources.iter().any(|s| within(dest, s))
        || sources.iter().all(|s| parent(s) == dest)
    {
        return (dnd::NONE, dnd::NONE);
    }
    let same = sources.iter().all(|s| volume(s) == volume(dest));
    (
        dnd::COPY | dnd::MOVE,
        if same { dnd::MOVE } else { dnd::COPY },
    )
}
/// The drop description insert naming `dest` ("Move to Documents"):
/// its last component (C: for the compatibility root), cut to the
/// protocol bound on a character boundary.
pub fn insert(dest: &str) -> String {
    let name = match dest {
        "/compat" => "Local Disk (C:)",
        "/" => "Door",
        _ => file_name(dest),
    };
    let mut cut = name.len().min(dnd::MAX_DESCRIPTION);
    while !name.is_char_boundary(cut) {
        cut -= 1;
    }
    name[..cut].chars().filter(|c| !c.is_control()).collect()
}
/// Where `source` lands in `dest`: its own name, or for a copy whose name
/// is taken "name - Copy.ext", "name - Copy (2).ext"... as Windows names
/// them. A move onto a taken name lands nowhere (the item is skipped).
pub fn destination(
    source: &str,
    dest: &str,
    copy: bool,
    mut taken: impl FnMut(&str) -> bool,
) -> Option<String> {
    let name = file_name(source);
    let first = join(dest, name);
    if !taken(&first) {
        return Some(first);
    }
    if !copy {
        return None;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (1..100).find_map(|n| {
        let candidate = if n == 1 {
            join(dest, &format!("{stem} - Copy{ext}"))
        } else {
            join(dest, &format!("{stem} - Copy ({n}){ext}"))
        };
        (!taken(&candidate)).then_some(candidate)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|p| String::from(*p)).collect()
    }
    #[test]
    fn a_press_becomes_a_drag_outside_the_drag_rectangle() {
        assert!(!drag_started((10, 10), (12, 8)));
        assert!(drag_started((10, 10), (13, 10)));
        assert!(drag_started((10, 10), (10, 7)));
    }
    #[test]
    fn offers_round_trip_as_files_payloads() {
        let p = s(&["/users/1000/Documents/a.txt", "/users/1000/Documents/文档"]);
        let bytes = payload(&p).unwrap();
        assert_eq!(paths(&bytes).unwrap(), p);
        assert!(payload(&s(&["relative"])).is_err());
        assert!(paths(b"/a\0").is_err(), "empty path");
    }
    #[test]
    fn folders_move_by_default_and_refuse_impossible_drops() {
        let docs = "/users/1000/Documents";
        let a = s(&["/users/1000/Documents/a.txt"]);
        assert_eq!(
            status(&a, "/users/1000/Music"),
            (dnd::COPY | dnd::MOVE, dnd::MOVE)
        );
        // Ctrl copies, Shift moves (the server applies the keys).
        let (effects, default) = status(&a, "/users/1000/Music");
        assert_eq!(
            dnd::resolve(dnd::COPY | dnd::MOVE, effects, default, 0),
            dnd::MOVE
        );
        assert_eq!(
            dnd::resolve(dnd::COPY | dnd::MOVE, effects, default, dnd::KEY_CONTROL),
            dnd::COPY
        );
        assert_eq!(
            dnd::resolve(dnd::COPY | dnd::MOVE, effects, default, dnd::KEY_SHIFT),
            dnd::MOVE
        );
        // Already there, onto itself, into its own subtree.
        assert_eq!(status(&a, docs), (dnd::NONE, dnd::NONE));
        let folder = s(&[docs]);
        assert_eq!(status(&folder, docs), (dnd::NONE, dnd::NONE));
        assert_eq!(
            status(&folder, "/users/1000/Documents/Old"),
            (dnd::NONE, dnd::NONE)
        );
        // A sibling whose name shares the prefix is a real destination.
        assert_eq!(
            status(&folder, "/users/1000/Documents2").0,
            dnd::COPY | dnd::MOVE
        );
        assert_eq!(status(&[], docs).0, dnd::NONE);
        assert_eq!(volume("/compat/x"), volume("/users/1000"));
    }
    #[test]
    fn labels_name_the_destination_folder() {
        assert_eq!(insert("/users/1000/Documents"), "Documents");
        assert_eq!(insert("/compat"), "Local Disk (C:)");
        let long = format!("/x/{}", "é".repeat(40));
        let label = insert(&long);
        assert!(label.len() <= dnd::MAX_DESCRIPTION && dnd::valid_description(&label));
    }
    #[test]
    fn copies_onto_taken_names_get_windows_copy_names_and_moves_skip() {
        let taken = ["/d/report.txt", "/d/report - Copy.txt"];
        let is_taken = |p: &str| taken.contains(&p);
        assert_eq!(
            destination("/s/notes.txt", "/d", false, is_taken).as_deref(),
            Some("/d/notes.txt")
        );
        assert_eq!(destination("/s/report.txt", "/d", false, is_taken), None);
        assert_eq!(
            destination("/s/report.txt", "/d", true, is_taken).as_deref(),
            Some("/d/report - Copy (2).txt")
        );
        assert_eq!(
            destination("/s/.profile", "/d", true, |p| p == "/d/.profile").as_deref(),
            Some("/d/.profile - Copy")
        );
        assert_eq!(parent("/a"), "/");
        assert_eq!(join("/", "a"), "/a");
    }
}

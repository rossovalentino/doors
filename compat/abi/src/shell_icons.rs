//! The shell's icon table, shared by shell32 and user32: Windows' own
//! shell32.dll icon indices (0 unknown file, 2 program, 3 folder, ...)
//! and Doors' private ones from 400 on, each drawn from a Doors asset
//! (`icons/<dir>/<stem>`) or a system icon (`system:<IDI id>`). Programs
//! see the location `C:\Windows\System32\shell32.dll,<index>`, as on
//! Windows, and ExtractIcon on that path answers from this table.
//!
//! File types map to icons the way File Explorer draws them.

pub const SHELL32_PATH: &str = "C:\\Windows\\System32\\shell32.dll";

/// (index, asset stem).
pub const ICONS: [(u32, &str); 38] = [
    (0, "icons/windows/file-generic"),
    (1, "icons/windows/file-generic"),
    (2, "icons/windows/file-application"),
    (3, "icons/explorer/folder"),
    (4, "icons/explorer/folder"),
    (8, "icons/windows/drive"),
    (15, "icons/windows/computer"),
    (16, "icons/windows/computer"),
    (17, "icons/windows/network"),
    (31, "icons/windows/recycle-bin-empty"),
    (32, "icons/windows/recycle-bin-full"),
    (70, "icons/windows/file-font"),
    (77, "system:32518"),
    (78, "system:32515"),
    (79, "system:32516"),
    (80, "system:32513"),
    (94, "icons/windows/computer"),
    (400, "icons/windows/text-file"),
    (401, "icons/windows/file-photo"),
    (402, "icons/windows/file-media"),
    (403, "icons/windows/file-zip"),
    (404, "icons/windows/file-pdf"),
    (405, "icons/windows/file-batch"),
    (406, "icons/windows/file-system"),
    (407, "icons/windows/file-installer"),
    (408, "icons/windows/file-settings"),
    (409, "icons/windows/file-powershell"),
    (410, "icons/windows/file-registry"),
    (411, "icons/windows/file-internet-shortcut"),
    (412, "icons/windows/file-disc-image"),
    (413, "icons/windows/file-virtual-disk"),
    (414, "icons/windows/file-cabinet"),
    (415, "icons/windows/file-certificate"),
    (416, "icons/windows/file-code"),
    (417, "icons/windows/file-word"),
    (418, "icons/windows/file-excel"),
    (419, "icons/windows/file-presentation"),
    (420, "icons/windows/file-model-light"),
];
pub const FOLDER: u32 = 3;
pub const FOLDER_OPEN: u32 = 4;
pub const APPLICATION: u32 = 2;
pub const DOCUMENT: u32 = 0;

pub fn stem(index: u32) -> Option<&'static str> {
    ICONS.iter().find(|(i, _)| *i == index).map(|(_, s)| *s)
}
/// Is `path` (a Windows path) the shell's own icon library?
pub fn is_shell_library(path: &str) -> bool {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    name.eq_ignore_ascii_case("shell32.dll") || name.eq_ignore_ascii_case("imageres.dll")
}
/// The icon index File Explorer shows for a file named `name`.
pub fn for_file(name: &str) -> u32 {
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    let is = |values: &[&str]| values.iter().any(|v| ext.eq_ignore_ascii_case(v));
    if is(&["exe", "com", "scr"]) {
        APPLICATION
    } else if is(&["bat", "cmd"]) {
        405
    } else if is(&["dll", "sys", "drv", "ocx", "cpl"]) {
        406
    } else if is(&["msi", "msp"]) {
        407
    } else if is(&["ini", "inf", "cfg"]) {
        408
    } else if is(&["ps1", "psm1"]) {
        409
    } else if is(&["reg"]) {
        410
    } else if is(&["url"]) {
        411
    } else if is(&["ttf", "otf", "fon", "ttc"]) {
        70
    } else if is(&["iso", "img"]) {
        412
    } else if is(&["vhd", "vhdx"]) {
        413
    } else if is(&["cab"]) {
        414
    } else if is(&["cer", "crt", "pfx"]) {
        415
    } else if is(&[
        "jpg", "jpeg", "png", "qoi", "bmp", "gif", "ico", "webp", "ppm", "pnm",
    ]) {
        401
    } else if is(&["txt", "md", "log", "csv", "tsv"]) {
        400
    } else if is(&[
        "json", "rs", "toml", "xml", "yaml", "yml", "js", "ts", "tsx", "jsx", "py", "c", "h",
        "cpp", "css", "html", "svg", "sh",
    ]) {
        416
    } else if is(&["pdf"]) {
        404
    } else if is(&["doc", "docx", "odt", "rtf"]) {
        417
    } else if is(&["xlsx", "xls", "ods"]) {
        418
    } else if is(&["ppt", "pptx", "odp"]) {
        419
    } else if is(&[
        "mp4", "m4v", "mkv", "webm", "avi", "mov", "mp3", "wav", "flac", "ogg", "opus", "m4a",
        "wma", "wmv",
    ]) {
        402
    } else if is(&["zip", "7z", "tar", "gz", "tgz", "rar"]) {
        403
    } else if is(&["glb", "gltf", "obj", "stl", "fbx"]) {
        420
    } else {
        DOCUMENT
    }
}
/// SHGetStockIconInfo's SHSTOCKICONID to a table index.
pub fn stock(id: u32) -> Option<u32> {
    Some(match id {
        0 | 1 => DOCUMENT,
        2 => APPLICATION,
        3 => FOLDER,
        4 => FOLDER_OPEN,
        8 => 8,
        15 => 15,
        // SIID_RECYCLER, SIID_RECYCLERFULL
        31 => 31,
        32 => 32,
        // SIID_SHIELD, WARNING, INFO, ERROR
        77..=80 => id,
        // SIID_DESKTOPPC
        94 => 94,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_index_has_one_stem_and_types_map_into_the_table() {
        for (i, (index, _)) in ICONS.iter().enumerate() {
            assert!(ICONS[..i].iter().all(|(j, _)| j != index), "{index}");
        }
        for name in [
            "a.exe", "b.TXT", "c.pdf", "d", "e.zip", "f.png", "g.ttf", "h.bat",
        ] {
            assert!(stem(for_file(name)).is_some(), "{name}");
        }
        assert_eq!(for_file("x.unknown"), DOCUMENT);
        assert!(is_shell_library("C:\\Windows\\System32\\SHELL32.dll"));
        assert!(!is_shell_library("C:\\x\\notshell32.dll"));
        assert_eq!(stock(3), Some(FOLDER));
        assert!(stock(5).is_none());
    }
}

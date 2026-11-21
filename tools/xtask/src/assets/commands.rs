//! Windows 11 26H2 File Explorer command icons: the two-tone 16 px SVGs
//! FileExplorerExtensions ships per theme (outline, light fill, accent), an
//! installed reference acquired from the owner's system for local
//! development (`assets acquire-commands`), then rasterized offline into
//! premultiplied BGRA frames the context menu paints 1:1.
use crate::evidence::{file_hash, write_json};
use serde_json::json;
use std::{fs, path::Path};

/// Doors key and the system file name, in menu order.
pub const ICONS: &[(&str, &str)] = &[
    ("cut", "windows.cut.svg"),
    ("copy", "windows.copy.svg"),
    ("paste", "windows.paste.svg"),
    ("rename", "windows.rename.svg"),
    ("share", "Windows.ModernShare.svg"),
    ("delete", "windows.recycle.svg"),
    ("open-folder", "windows.openfolder.svg"),
    ("open-new-tab", "windows.opennewtab.svg"),
    ("open-new-window", "windows.opennewwindow.svg"),
    ("open-with", "windows.openwith.svg"),
    ("open-location", "windows.shortcut.opencontaining.svg"),
    ("pin", "windows.pintohome.svg"),
    ("unpin", "windows.unpinfromhome.svg"),
    ("favorite", "windows.pintohomefile.svg"),
    ("copy-path", "windows.copyaspath.svg"),
    ("zip", "Windows.Zip.Action.svg"),
    ("extract", "windows.compressedfile.extract.svg"),
    ("properties", "windows.properties.svg"),
    ("show-more", "expandtoclassic.svg"),
    ("refresh", "windows.refresh.svg"),
    ("new", "windows.newitem.svg"),
    ("new-folder", "newfolder.svg"),
    ("view", "View.svg"),
    ("sort", "SortBy.svg"),
    ("customize", "windows.customizemenu.svg"),
    ("send-to", "windows.sendto.svg"),
    ("wallpaper", "windows.setdesktopwallpaper.svg"),
    ("empty-bin", "windows.recyclebin.empty.svg"),
    ("restore-all", "windows.recyclebin.restoreall.svg"),
    ("restore", "windows.recyclebin.restoreitems.svg"),
    ("select-all", "windows.selectall.svg"),
    ("select-none", "windows.selectnone.svg"),
    ("invert-selection", "windows.invertselection.svg"),
    ("more-actions", "windows.otheractionsflyout.svg"),
    ("extensions", "windows.extensionsflyout.svg"),
    ("remove", "windows.close.svg"),
    ("personalize", "Personalize.svg"),
    ("display", "DisplaySettings.svg"),
    ("undo", "windows.undo.svg"),
    ("redo", "windows.redo.svg"),
];
pub const THEMES: [(&str, &str); 2] = [("light", "theme-light"), ("dark", "theme-dark")];
/// Frames: 1x, 1.25x, 1.5x, 2x and 3x of the 16 px icon.
pub const SIZES: [u32; 5] = [16, 20, 24, 32, 48];
const DIR: &str = "assets/icons/explorer-commands";

/// Copies the icons from FileExplorerExtensions' `Assets\images` folder
/// (`contrast-standard\theme-*`) and records where they came from.
pub fn acquire(root: &Path, images: &str) -> Result<(), String> {
    let images = Path::new(images).join("contrast-standard");
    let mut files = Vec::new();
    for (theme, folder) in THEMES {
        let out = root.join(DIR).join(theme);
        fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        for (key, name) in ICONS {
            let source = images.join(folder).join(name);
            let bytes = fs::read(&source).map_err(|e| format!("{}: {e}", source.display()))?;
            if bytes.len() > 64 * 1024 || !bytes.starts_with(b"<svg") {
                return Err(format!("{name}: not a bounded SVG"));
            }
            let target = out.join(format!("{key}.svg"));
            fs::write(&target, &bytes).map_err(|e| e.to_string())?;
            files.push(
                json!({"key": key, "theme": theme, "source": source.to_string_lossy(),
                "sha256": file_hash(&target)?}),
            );
        }
    }
    write_json(
        &root.join(DIR).join("source.json"),
        &json!({"schema_version": 1, "kind": "installed_windows_icon_reference",
            "use": "Owner-provided installed Windows reference; local development",
            "method": "byte copy of the 26H2 FileExplorerExtensions command SVGs; rasterized offline",
            "files": files}),
    )?;
    println!(
        "Acquired {} command icons in {} themes",
        ICONS.len(),
        THEMES.len()
    );
    Ok(())
}

pub fn export(root: &Path, check: bool) -> Result<(), String> {
    let dir = root.join(DIR);
    let source = crate::image::read_json(&dir.join("source.json"))?;
    let files = source["files"].as_array().ok_or("command icon inventory")?;
    for (theme, _) in THEMES {
        for (key, _) in ICONS {
            let path = dir.join(theme).join(format!("{key}.svg"));
            let pinned = files
                .iter()
                .find(|f| f["key"] == *key && f["theme"] == theme)
                .ok_or("command icon provenance")?;
            if file_hash(&path)? != pinned["sha256"] {
                return Err(format!("command icon {theme}/{key} changed"));
            }
            let tree = resvg::usvg::Tree::from_data(
                &fs::read(&path).map_err(|e| e.to_string())?,
                &resvg::usvg::Options::default(),
            )
            .map_err(|e| e.to_string())?;
            let base = tree.size().width().max(tree.size().height());
            for size in SIZES {
                let mut image =
                    resvg::tiny_skia::Pixmap::new(size, size).ok_or("command icon extent")?;
                let s = size as f32 / base;
                resvg::render(
                    &tree,
                    resvg::tiny_skia::Transform::from_scale(s, s),
                    &mut image.as_mut(),
                );
                // tiny-skia is premultiplied RGBA; frames are premultiplied BGRA.
                let bytes: Vec<u8> = image
                    .data()
                    .chunks_exact(4)
                    .flat_map(|p| [p[2], p[1], p[0], p[3]])
                    .collect();
                super::branding_file(
                    &dir.join(format!("{key}.{theme}.{size}.bgra")),
                    &bytes,
                    check,
                )?;
            }
        }
    }
    Ok(())
}

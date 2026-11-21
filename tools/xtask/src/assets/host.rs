//! Assets the repository does not carry. Microsoft-owned ones (Windows
//! fonts, icons, cursors, sounds and wallpapers) come from the Windows PC
//! each Doors build runs on; every file the exporters generate (from those
//! and from Doors' own SVG artwork) and the IME dictionaries are rebuilt
//! there too. `assets/host-assets.json` lists each source with where a
//! Windows 11 host keeps it, and each generated file with its size.
//!
//! `cargo host-assets` (tools/host-assets) first writes zero-filled
//! placeholders for missing generated files so the workspace compiles, then
//! runs `cargo xtask host-assets acquire` (copy each source from this PC; a
//! source whose bytes differ from its pin is re-pinned in the asset
//! manifests) and `cargo xtask host-assets export` (regenerate everything
//! and mark the import complete).
use super::acquire::{Frame, archive, group_archive};
use crate::image::read_json;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub const INVENTORY: &str = "assets/host-assets.json";
/// Written when an import finished; builds refuse to run without it.
pub const STAMP: &str = "assets/.host-assets-ok";

/// Copies every exporter input from this PC and re-pins changed ones.
pub fn acquire(root: &Path) -> Result<(), String> {
    let _ = fs::remove_file(root.join(STAMP));
    let inventory = read_json(&root.join(INVENTORY))?;
    let mut packages = HashMap::new();
    let mut repins = Vec::new();
    let mut missing = Vec::new();
    let sources = inventory["sources"]
        .as_array()
        .ok_or("host asset inventory")?;
    for source in sources {
        let path = relative(source)?;
        let bytes = match fetch(root, &source["from"], &mut packages) {
            Ok(bytes) => bytes,
            Err(error) => {
                missing.push(format!("{path}: {error}"));
                continue;
            }
        };
        let target = root.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&target, &bytes).map_err(|e| format!("{path}: {e}"))?;
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let pinned = source["sha256"].as_str().ok_or("inventory pin")?;
        if hash != pinned {
            println!("re-pinned {path} (this PC's copy differs)");
            repins.push((pinned.to_string(), hash));
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "{} Microsoft-owned assets are not on this PC:\n  {}",
            missing.len(),
            missing.join("\n  ")
        ));
    }
    repin(root, &repins)?;
    println!("Copied {} host assets ({} re-pinned)", sources.len(), repins.len());
    Ok(())
}

/// Regenerates every generated asset and marks the import complete.
pub fn export(root: &Path) -> Result<(), String> {
    super::export(root, false)?;
    if !IME.iter().all(|path| root.join(path).is_file()) {
        crate::ime_dictionaries::run(root, &["--fetch".to_string(), "--japanese".to_string()])?;
    }
    let inventory = read_json(&root.join(INVENTORY))?;
    let mut paths = Vec::new();
    for entry in inventory["sources"].as_array().ok_or("host asset inventory")? {
        paths.push(relative(entry)?.to_string());
    }
    paths.extend(generated(&inventory)?.into_iter().map(|(path, _)| path));
    for path in paths {
        let bytes = fs::read(root.join(&path)).map_err(|e| format!("{path}: {e}"))?;
        if bytes.iter().all(|&b| b == 0) {
            return Err(format!("{path} is still a placeholder"));
        }
    }
    fs::write(root.join(STAMP), "complete\n").map_err(|e| e.to_string())?;
    println!("Host assets ready");
    Ok(())
}

/// Builds need the imported assets (the workspace embeds several of them).
pub fn require(root: &Path) -> Result<(), String> {
    if root.join(STAMP).is_file() {
        Ok(())
    } else {
        Err("generated and Microsoft-owned assets are missing: run `cargo host-assets` on a Windows 11 PC first".into())
    }
}

/// The Japanese IME dictionary (`cargo xtask ime-dictionaries --japanese`, from pinned
/// upstream downloads).
const IME: [&str; 1] = ["assets/ime/japanese.dict"];

fn relative(entry: &Value) -> Result<&str, String> {
    checked(entry["path"].as_str().ok_or("inventory path")?)
}
fn checked(path: &str) -> Result<&str, String> {
    if !path.starts_with("assets/") || path.contains("..") || path.contains('\\') {
        return Err(format!("inventory path {path}"));
    }
    Ok(path)
}

/// Each generated file and its size: `"folder": "name:bytes name:bytes"`.
fn generated(inventory: &Value) -> Result<Vec<(String, u64)>, String> {
    let mut files = Vec::new();
    for (folder, names) in inventory["generated"]
        .as_object()
        .ok_or("generated assets")?
    {
        for item in names.as_str().ok_or("generated assets")?.split(' ') {
            let (name, bytes) = item.rsplit_once(':').ok_or("generated asset size")?;
            let path = format!("{folder}/{name}");
            checked(&path)?;
            files.push((path, bytes.parse().map_err(|_| "generated asset size")?));
        }
    }
    Ok(files)
}

fn fetch(
    root: &Path,
    from: &Value,
    packages: &mut HashMap<String, Option<PathBuf>>,
) -> Result<Vec<u8>, String> {
    if let Some(file) = from["file"].as_str() {
        return read(&host_path(file, packages)?);
    }
    if let Some(url) = from["url"].as_str() {
        let bytes = download(url)?;
        // A published PNG profile icon, kept as a one-frame ICO.
        if from["ico"] == true {
            let size = super::ico::frame(&bytes)?.ok_or("PNG icon frame")?.width();
            let n = if size >= 256 { 0 } else { size as u8 };
            return archive(&[Frame {
                directory: [n, n, 0, 0, 1, 0, 32, 0],
                bytes,
            }]);
        }
        return Ok(bytes);
    }
    if let Some(icon) = from.get("icon") {
        let module = read(&host_path(
            icon["module"].as_str().ok_or("icon module")?,
            packages,
        )?)?;
        let image = super::resources::Image::parse(&module)?;
        let group = match &icon["group"] {
            Value::Number(id) => {
                image.read(14, id.as_u64().and_then(|v| u16::try_from(v).ok()).ok_or("icon group")?)?
            }
            Value::String(name) => image.read_named(14, name)?,
            _ => return Err("icon group".into()),
        };
        return group_archive(&group, |id| image.read(3, id));
    }
    if let Some(files) = from["pngs"].as_array() {
        let mut frames = Vec::new();
        for file in files {
            let bytes = read(&host_path(file.as_str().ok_or("PNG source")?, packages)?)?;
            let size = super::ico::frame(&bytes)?
                .ok_or("PNG icon frame")?
                .width();
            let n = if size >= 256 { 0 } else { size as u8 };
            frames.push(Frame {
                directory: [n, n, 0, 0, 1, 0, 32, 0],
                bytes,
            });
        }
        return archive(&frames);
    }
    if let Some(jpeg) = from["jpeg"].as_str() {
        return jpeg_to_png(&read(&root.join(jpeg))?);
    }
    Err(format!("unknown host asset source {from}"))
}

fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// A host path; `appx:<Package>\<path>` is inside an installed Store app
/// (its folder name carries the version, and WindowsApps cannot be listed),
/// and a `*` folder is the newest installed version (Edge's).
fn host_path(path: &str, packages: &mut HashMap<String, Option<PathBuf>>) -> Result<PathBuf, String> {
    let Some(rest) = path.strip_prefix("appx:") else {
        let Some((parent, inner)) = path.split_once("\\*\\") else {
            return Ok(PathBuf::from(path));
        };
        let version = |name: &str| -> Vec<u64> {
            name.split('.').map(|part| part.parse().unwrap_or(0)).collect()
        };
        let newest = fs::read_dir(parent)
            .map_err(|e| format!("{parent}: {e}"))?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().join(inner).is_file())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .max_by_key(|name| version(name))
            .ok_or(format!("{path}: not installed"))?;
        return Ok(Path::new(parent).join(newest).join(inner));
    };
    let (name, inner) = rest.split_once('\\').ok_or("appx path")?;
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.') {
        return Err("appx package name".into());
    }
    let location = packages.entry(name.to_string()).or_insert_with(|| {
        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("(Get-AppxPackage -Name {name} | Select-Object -First 1).InstallLocation"),
            ])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        (!text.is_empty()).then(|| PathBuf::from(text))
    });
    let location = location
        .as_ref()
        .ok_or(format!("the {name} app is not installed"))?;
    Ok(location.join(inner))
}

/// Files Windows does not keep locally, from where they were published.
fn download(url: &str) -> Result<Vec<u8>, String> {
    if !url.starts_with("https://") {
        return Err("only HTTPS sources".into());
    }
    let output = Command::new("curl.exe")
        .args(["-fsSL", "--retry", "4", "--retry-all-errors", "--max-time", "120", url])
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !output.status.success() || output.stdout.is_empty() {
        return Err(format!("download failed: {url}"));
    }
    Ok(output.stdout)
}

/// The wallpaper exporters read PNG: the JPEG's pixels, losslessly.
fn jpeg_to_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use zune_jpeg::zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};
    let options = DecoderOptions::default()
        .set_max_width(16384)
        .set_max_height(16384)
        .jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    decoder.decode_headers().map_err(|e| format!("JPEG: {e:?}"))?;
    let (width, height) = decoder.dimensions().ok_or("JPEG dimensions")?;
    let rgba = decoder.decode().map_err(|e| format!("JPEG: {e:?}"))?;
    let size = resvg::tiny_skia::IntSize::from_wh(width as u32, height as u32)
        .ok_or("JPEG extent")?;
    resvg::tiny_skia::Pixmap::from_vec(rgba, size)
        .ok_or("JPEG pixels")?
        .encode_png()
        .map_err(|e| e.to_string())
}

/// Replaces each old pin with this PC's hash in every asset manifest.
fn repin(root: &Path, repins: &[(String, String)]) -> Result<(), String> {
    if repins.is_empty() {
        return Ok(());
    }
    let mut stack = vec![root.join("assets")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "json") {
                let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
                let mut changed = text.clone();
                for (old, new) in repins {
                    changed = changed.replace(old.as_str(), new);
                }
                if changed != text {
                    fs::write(&path, changed).map_err(|e| e.to_string())?;
                }
            }
        }
    }
    Ok(())
}

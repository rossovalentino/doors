//! Reproducible exports of the small, licensed SVG set used by the guest.
//! SVG parsing remains a build-time concern; the guest only consumes alpha masks.
use crate::evidence::file_hash;
use std::{fs, path::Path};
pub(crate) mod acquire;
pub(crate) mod advanced;
pub(crate) mod commands;
pub(crate) mod folders;
pub(crate) mod host;
mod ico;
pub(crate) mod paint_icons;
mod raster;
mod resources;
mod shadow;
pub(crate) mod shell;

pub fn export(root: &Path, check: bool) -> Result<(), String> {
    advanced::export(root, check).map_err(|e| format!("advanced::export: {e}"))?;
    rounded_coverage(root, check).map_err(|e| format!("rounded_coverage: {e}"))?;
    shadow::export(root, check).map_err(|e| format!("shadow::export: {e}"))?;
    let font_dir = root.join("assets/fonts/segoe-ui");
    let mono_dir = root.join("assets/fonts/cascadia-mono");
    let mono = crate::image::read_json(&mono_dir.join("provenance.json"))?;
    if file_hash(&mono_dir.join("CascadiaMono.ttf"))? != mono["sha256"] {
        return Err("Terminal monospace font identity".into());
    }
    let fonts = crate::image::read_json(&font_dir.join("provenance.json"))?;
    for font in fonts["fonts"].as_array().ok_or("local font inventory")? {
        let file = font["file"].as_str().ok_or("font filename")?;
        if file.contains(['/', '\\'])
            || !file.ends_with(".ttf")
            || file_hash(&font_dir.join(file))? != font["sha256"]
        {
            return Err("local font identity".into());
        }
    }
    crate::typography::export(root, check)
        .map_err(|e| format!("crate::typography::export: {e}"))?;
    export_branding(root, check).map_err(|e| format!("export_branding: {e}"))?;
    shell::export(root, check).map_err(|e| format!("shell::export: {e}"))?;
    commands::export(root, check).map_err(|e| format!("commands::export: {e}"))?;
    paint_icons::export(root, check).map_err(|e| format!("paint_icons::export: {e}"))?;
    folders::export(root, check).map_err(|e| format!("folders::export: {e}"))?;
    export_wallpaper(root, check).map_err(|e| format!("export_wallpaper: {e}"))?;
    export_background_collection(root, check)
        .map_err(|e| format!("export_background_collection: {e}"))?;
    export_cursors(root, check).map_err(|e| format!("export_cursors: {e}"))?;
    export_windows(root, check).map_err(|e| format!("export_windows: {e}"))?;
    export_settings(root, check).map_err(|e| format!("export_settings: {e}"))?;
    let dir = root.join("assets/icons/fluent");
    let manifest: serde_json::Value = crate::image::read_json(&dir.join("manifest.json"))?;
    for icon in manifest["icons"]
        .as_array()
        .ok_or("icon manifest entries")?
    {
        let file = icon["file"].as_str().ok_or("icon filename")?;
        if file.contains(['/', '\\']) || !file.ends_with(".svg") {
            return Err("invalid icon filename".into());
        }
        let source = dir.join(file);
        if file_hash(&source)? != icon["sha256"].as_str().ok_or("icon source hash")? {
            return Err(format!("changed icon source: {file}"));
        }
        let size = icon["size"].as_u64().ok_or("icon extent")?;
        if size != 20 {
            return Err("unsupported icon extent".into());
        }
        let svg = fs::read(&source).map_err(|e| e.to_string())?;
        let tree = resvg::usvg::Tree::from_data(&svg, &resvg::usvg::Options::default())
            .map_err(|e| e.to_string())?;
        if tree.size().width() != 20.0 || tree.size().height() != 20.0 {
            return Err("SVG extent mismatch".into());
        }
        let sizes: &[u32] = if icon["id"] == "settings" {
            &[12, 16, 20, 24, 32, 36, 40, 48, 60, 64, 80]
        } else {
            &[12, 16, 20, 24, 32, 36, 40, 48, 60]
        };
        for &size in sizes {
            let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).ok_or("icon allocation")?;
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::from_scale(size as f32 / 20.0, size as f32 / 20.0),
                &mut pixmap.as_mut(),
            );
            let mask: Vec<u8> = pixmap.data().chunks_exact(4).map(|p| p[3]).collect();
            let output = source.with_extension(if size == 20 {
                "a8".into()
            } else {
                format!("{size}.a8")
            });
            if check {
                if fs::read(&output).map_err(|e| e.to_string())? != mask {
                    return Err(format!("stale icon export: {file}"));
                }
            } else {
                fs::write(output, mask).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

/// Exact sixteen-sample coverage for the supported native rounded radii.
/// This original geometric asset removes repeated circle tests during drags.
pub fn rounded_coverage(root: &Path, check: bool) -> Result<(), String> {
    rounded_radii(root, "rounded-coverage.a8", 1..=60, 73_810, check)?;
    // The sign-in avatar uses these physical radii at the three fixed densities.
    // Cache only those quadrants; all other large radii retain runtime sampling.
    rounded_radii(root, "avatar-coverage.a8", [96, 192, 288], 129_024, check)
}

fn rounded_radii(
    root: &Path,
    filename: &str,
    radii: impl IntoIterator<Item = i32>,
    length: usize,
    check: bool,
) -> Result<(), String> {
    let mut bytes = Vec::with_capacity(length);
    for radius in radii {
        for y in 0..radius {
            for x in 0..radius {
                let mut inside = 0;
                for dy in [2, 6, 10, 14] {
                    for dx in [2, 6, 10, 14] {
                        let a = x * 16 + dx - radius * 16;
                        let b = y * 16 + dy - radius * 16;
                        inside += i32::from(a * a + b * b <= radius * radius * 256);
                    }
                }
                bytes.push(((inside * 255 + 8) / 16) as u8);
            }
        }
    }
    if bytes.len() != length {
        return Err("rounded coverage extent".into());
    }
    let path = root.join("assets/raster").join(filename);
    if !check {
        fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    }
    branding_file(&path, &bytes, check)
}

fn export_cursors(root: &Path, check: bool) -> Result<(), String> {
    let dir = root.join("assets/cursors/windows");
    let manifest = crate::image::read_json(&dir.join("manifest.json"))?;
    for item in manifest["cursors"].as_array().ok_or("cursor inventory")? {
        let name = item["file"].as_str().ok_or("cursor filename")?;
        if name.contains(['/', '\\']) || !name.ends_with(".cur") {
            return Err("cursor filename".into());
        }
        let path = dir.join(name);
        if file_hash(&path)? != item["sha256"] {
            return Err("cursor hash".into());
        }
        let b = fs::read(&path).map_err(|e| e.to_string())?;
        if b.len() < 6 || b.len() > 1024 * 1024 || b[..4] != [0, 0, 2, 0] {
            return Err("CUR header".into());
        }
        let count = u16::from_le_bytes(b[4..6].try_into().unwrap()) as usize;
        if count > 64 || 6 + count * 16 > b.len() {
            return Err("CUR directory".into());
        }
        for size in [32usize, 64, 96] {
            let e = b[6..6 + count * 16]
                .chunks_exact(16)
                .find(|e| e[0] as usize == size && e[1] as usize == size)
                .ok_or("32px CUR frame")?;
            let hx = u16::from_le_bytes(e[4..6].try_into().unwrap());
            let hy = u16::from_le_bytes(e[6..8].try_into().unwrap());
            if hx as usize >= size || hy as usize >= size {
                return Err("CUR hotspot".into());
            }
            let n = u32::from_le_bytes(e[8..12].try_into().unwrap()) as usize;
            let at = u32::from_le_bytes(e[12..16].try_into().unwrap()) as usize;
            let d = b
                .get(at..at.checked_add(n).ok_or("CUR extent")?)
                .ok_or("CUR backing")?;
            if d.len() < 40 {
                return Err("CUR DIB".into());
            }
            let u32at = |p| u32::from_le_bytes(d[p..p + 4].try_into().unwrap());
            let bits = u16::from_le_bytes(d[14..16].try_into().unwrap());
            if u32at(0) != 40
                || u32at(4) != size as u32
                || u32at(8) != size as u32 * 2
                || u32at(16) != 0
                || !matches!(bits, 1 | 32)
            {
                return Err("CUR DIB format".into());
            }
            let base = 40 + if bits == 1 { 8 } else { 0 };
            let stride = if bits == 1 {
                size.div_ceil(32) * 4
            } else {
                size * 4
            };
            let mask_stride = size.div_ceil(32) * 4;
            let mask = base + stride * size;
            if mask + mask_stride * size > d.len() {
                return Err("CUR planes".into());
            }
            let alpha = bits == 32 && d[base..mask].chunks_exact(4).any(|p| p[3] != 0);
            let mut out = vec![hx as u8, hy as u8];
            for y in 0..size {
                for x in 0..size {
                    let row = size - 1 - y;
                    let and = d[mask + row * mask_stride + x / 8] & (0x80 >> (x % 8)) != 0;
                    let p = if bits == 1 {
                        let xor = d[base + row * mask_stride + x / 8] & (0x80 >> (x % 8)) != 0;
                        match (and, xor) {
                            (true, true) => [255, 255, 255, 0],
                            (true, false) => [0; 4],
                            (false, true) => [255; 4],
                            (false, false) => [0, 0, 0, 255],
                        }
                    } else {
                        let p = &d[base + row * stride + x * 4..base + row * stride + x * 4 + 4];
                        let a = if alpha {
                            p[3]
                        } else if and {
                            0
                        } else {
                            255
                        };
                        [
                            ((p[0] as u32 * a as u32 + 127) / 255) as u8,
                            ((p[1] as u32 * a as u32 + 127) / 255) as u8,
                            ((p[2] as u32 * a as u32 + 127) / 255) as u8,
                            a,
                        ]
                    };
                    out.extend_from_slice(&p);
                }
            }
            let dest = path.with_extension(if size == 32 {
                "cursor".into()
            } else {
                format!("{size}.cursor")
            });
            if check {
                if fs::read(dest).map_err(|e| e.to_string())? != out {
                    return Err("stale cursor export".into());
                }
            } else {
                fs::write(dest, out).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

pub(crate) fn export_windows(root: &Path, check: bool) -> Result<(), String> {
    let dir = root.join("assets/icons/windows");
    let manifest = crate::image::read_json(&dir.join("manifest.json"))?;
    for icon in manifest["icons"]
        .as_array()
        .ok_or("Windows icon inventory")?
    {
        let name = icon["file"].as_str().ok_or("ICO filename")?;
        if name.contains(['/', '\\']) || !name.ends_with(".ico") {
            return Err("ICO filename".into());
        }
        let path = dir.join(name);
        if file_hash(&path)? != icon["sha256"] {
            return Err("ICO source changed".into());
        }
        let bytes = fs::read(&path).map_err(|e| e.to_string())?;
        if bytes.len() < 6 || bytes.len() > 4 * 1024 * 1024 || bytes[..4] != [0, 0, 1, 0] {
            return Err("ICO header".into());
        }
        let count = u16::from_le_bytes(bytes[4..6].try_into().unwrap()) as usize;
        if count == 0 || count > 64 || 6 + count * 16 > bytes.len() {
            return Err("ICO directory".into());
        }
        let provenance = if let Some(reference) = icon.get("provenance") {
            let reference = reference.as_str().ok_or("ICO provenance filename")?;
            if reference.contains(['/', '\\'])
                || reference != format!("{}.source.json", name.trim_end_matches(".ico"))
            {
                return Err("ICO provenance filename".into());
            }
            let value = crate::image::read_json(&dir.join(reference))?;
            if value["schema_version"] != 1
                || value["kind"] != "installed_windows_icon_reference"
                || value["file"] != name
                || value["sha256"] != icon["sha256"]
                || value["frames"] != count
                || !value["sources"].is_object()
            {
                return Err("ICO provenance differs".into());
            }
            Some(value)
        } else {
            None
        };
        let mut frames = Vec::new();
        for entry in bytes[6..6 + count * 16].chunks_exact(16) {
            let n = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
            let start = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
            let data = bytes
                .get(start..start.checked_add(n).ok_or("ICO extent")?)
                .ok_or("ICO backing")?;
            if let Some(frame) = ico::frame(data)? {
                frames.push(frame);
            }
        }
        if provenance.is_some_and(|value| value["supported_frames"] != frames.len()) {
            return Err("ICO provenance supported frame count differs".into());
        }
        for size in [
            16, 20, 24, 28, 32, 40, 48, 56, 60, 64, 72, 80, 84, 96, 120, 160, 240,
        ] {
            // Exact authored small artwork is sharper than reducing the large
            // illustration. Otherwise reduce the nearest larger frame.
            let source = frames
                .iter()
                .min_by_key(|p| (p.width() < size, p.width().abs_diff(size)))
                .ok_or("ICO has no supported frame")?;
            let source = raster::prefilter(source, size, size)?;
            let mut output = resvg::tiny_skia::Pixmap::new(size, size).ok_or("icon allocation")?;
            let paint = resvg::tiny_skia::PixmapPaint {
                quality: resvg::tiny_skia::FilterQuality::Bicubic,
                ..Default::default()
            };
            output.draw_pixmap(
                0,
                0,
                source.as_ref(),
                &paint,
                resvg::tiny_skia::Transform::from_scale(
                    size as f32 / source.width() as f32,
                    size as f32 / source.height() as f32,
                ),
                None,
            );
            let bgra: Vec<u8> = output
                .data()
                .chunks_exact(4)
                .flat_map(|p| [p[2].min(p[3]), p[1].min(p[3]), p[0].min(p[3]), p[3]])
                .collect();
            if path.file_stem().is_some_and(|name| name == "file-model") {
                // 3D Viewer's authored monochrome logo is white. Apply the
                // Windows light-theme foreground while retaining its exact alpha.
                let light: Vec<u8> = bgra
                    .chunks_exact(4)
                    .flat_map(|p| {
                        let c = (32u16 * u16::from(p[3]) / 255) as u8;
                        [c, c, c, p[3]]
                    })
                    .collect();
                let export = dir.join(if size == 20 {
                    "file-model-light.bgra".into()
                } else {
                    format!("file-model-light.{size}.bgra")
                });
                branding_file(&export, &light, check)?;
                if size >= 16 {
                    let rgba: Vec<u8> = light
                        .chunks_exact(4)
                        .flat_map(|p| [p[2], p[1], p[0], p[3]])
                        .collect();
                    let q = qoi::encode_to_vec(&rgba, size, size).map_err(|e| e.to_string())?;
                    branding_file(&dir.join(format!("file-model-light.{size}.qoi")), &q, check)?;
                }
            }
            if size >= 16 {
                let rgba: Vec<u8> = bgra
                    .chunks_exact(4)
                    .flat_map(|p| [p[2], p[1], p[0], p[3]])
                    .collect();
                let compressed =
                    qoi::encode_to_vec(&rgba, size, size).map_err(|e| e.to_string())?;
                branding_file(
                    &path.with_extension(format!("{size}.qoi")),
                    &compressed,
                    check,
                )?;
            }
            let output = path.with_extension(if size == 20 {
                "bgra".into()
            } else {
                format!("{size}.bgra")
            });
            if check {
                if fs::read(&output).map_err(|e| e.to_string())? != bgra {
                    return Err("stale ICO export".into());
                }
            } else {
                fs::write(output, bgra).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

fn export_settings(root: &Path, check: bool) -> Result<(), String> {
    let dir = root.join("assets/icons/windows-settings");
    let manifest = crate::image::read_json(&dir.join("manifest.json"))?;
    for icon in manifest["icons"]
        .as_array()
        .ok_or("Settings icon inventory")?
    {
        let name = icon["file"].as_str().ok_or("Settings filename")?;
        if name.contains(['/', '\\']) || !name.ends_with(".png") {
            return Err("Settings filename".into());
        }
        let path = dir.join(name);
        if file_hash(&path)? != icon["sha256"] {
            return Err("Settings icon source changed".into());
        }
        let bytes = fs::read(&path).map_err(|e| e.to_string())?;
        if bytes.len() > 1024 * 1024
            || bytes.len() < 24
            || u32::from_be_bytes(bytes[16..20].try_into().unwrap()) > 128
            || u32::from_be_bytes(bytes[20..24].try_into().unwrap()) > 128
        {
            return Err("Settings PNG extent".into());
        }
        let source = resvg::tiny_skia::Pixmap::decode_png(&bytes).map_err(|e| e.to_string())?;
        for size in [20, 32, 40, 48, 60, 64, 96, 144] {
            let source = raster::prefilter(&source, size, size)?;
            let mut output =
                resvg::tiny_skia::Pixmap::new(size, size).ok_or("Settings allocation")?;
            output.draw_pixmap(
                0,
                0,
                source.as_ref(),
                &resvg::tiny_skia::PixmapPaint {
                    quality: resvg::tiny_skia::FilterQuality::Bicubic,
                    ..Default::default()
                },
                resvg::tiny_skia::Transform::from_scale(
                    size as f32 / source.width() as f32,
                    size as f32 / source.height() as f32,
                ),
                None,
            );
            // Bicubic resampling of premultiplied pixels can round a colour
            // channel one or two steps above alpha; the compositor refuses
            // such pixels from a translucent window (Settings over Mica), so
            // the export keeps every channel within its alpha.
            let bgra: Vec<_> = output
                .data()
                .chunks_exact(4)
                .flat_map(|p| [p[2].min(p[3]), p[1].min(p[3]), p[0].min(p[3]), p[3]])
                .collect();
            let path = path.with_extension(format!("{size}.bgra"));
            if check {
                if fs::read(path).map_err(|e| e.to_string())? != bgra {
                    return Err("stale Settings export".into());
                }
            } else {
                fs::write(path, bgra).map_err(|e| e.to_string())?;
            }
        }
    }
    export_settings_glyphs(root, check)
}
fn export_settings_glyphs(root: &Path, check: bool) -> Result<(), String> {
    use ab_glyph::{Font, FontRef, PxScale, point};
    let dir = root.join("assets/fonts/segoe-fluent-icons");
    let provenance = crate::image::read_json(&dir.join("provenance.json"))?;
    let font_path = dir.join("SegoeFluentIcons.ttf");
    if file_hash(&font_path)? != provenance["font_sha256"] {
        return Err("Segoe Fluent source changed".into());
    }
    let bytes = fs::read(font_path).map_err(|e| e.to_string())?;
    let font = FontRef::try_from_slice(&bytes).map_err(|_| "Segoe Fluent font")?;
    for (name, ch) in [
        ("bluetooth-devices", '\u{e702}'),
        ("windows-update", '\u{e895}'),
    ] {
        for size in [20, 32, 40, 48, 60, 64, 96, 144] {
            let n = size as usize;
            let mut output = vec![0u8; n * n * 4];
            // Source Windows glyph plus the blue category badge in the reference.
            let rx = if name == "bluetooth-devices" {
                size as f32 * 0.36
            } else {
                size as f32 * 0.5
            };
            let ry = size as f32 * 0.5;
            for y in 0..n {
                for x in 0..n {
                    let mut inside = 0u32;
                    for dy in [0.125, 0.375, 0.625, 0.875] {
                        for dx in [0.125, 0.375, 0.625, 0.875] {
                            let a = (x as f32 + dx - size as f32 / 2.0) / rx;
                            let b = (y as f32 + dy - size as f32 / 2.0) / ry;
                            inside += u32::from(a * a + b * b <= 1.0);
                        }
                    }
                    let alpha = (inside * 255 + 8) / 16;
                    let at = (y * n + x) * 4;
                    output[at] = (255 * alpha / 255) as u8;
                    output[at + 1] = (146 * alpha / 255) as u8;
                    output[at + 2] = 0;
                    output[at + 3] = alpha as u8;
                }
            }
            let em = font.units_per_em().ok_or("icon em")?;
            let scale =
                size as f32 * 0.66 * (font.ascent_unscaled() - font.descent_unscaled()) / em;
            let glyph = font
                .glyph_id(ch)
                .with_scale_and_position(PxScale::from(scale), point(0.0, 0.0));
            let outline = font
                .outline_glyph(glyph)
                .ok_or("Windows icon glyph absent")?;
            let bounds = outline.px_bounds();
            let ox = ((size as f32 - bounds.width()) / 2.0).round() as i32;
            let oy = ((size as f32 - bounds.height()) / 2.0).round() as i32;
            outline.draw(|x, y, c| {
                let x = x as i32 + ox;
                let y = y as i32 + oy;
                if x >= 0 && y >= 0 && x < size && y < size {
                    let at = (y as usize * n + x as usize) * 4;
                    let a = (c * 255.0 + 0.5) as u32;
                    for channel in &mut output[at..at + 4] {
                        *channel =
                            (a + (u32::from(*channel) * (255 - a) + 127) / 255).min(255) as u8;
                    }
                }
            });
            let path = root.join(format!("assets/icons/windows-settings/{name}.{size}.bgra"));
            if check {
                if fs::read(path).map_err(|e| e.to_string())? != output {
                    return Err("stale Windows font icon".into());
                }
            } else {
                fs::write(path, output).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}
fn export_wallpaper(root: &Path, check: bool) -> Result<(), String> {
    let dir = root.join("assets/wallpapers/windows");
    let manifest = crate::image::read_json(&dir.join("manifest.json"))?;
    for (name, key) in [("bloom.jpg", "source_sha256"), ("bloom.png", "png_sha256")] {
        if file_hash(&dir.join(name))? != manifest[key] {
            return Err("wallpaper source hash".into());
        }
    }
    let bytes = fs::read(dir.join("bloom.png")).map_err(|e| e.to_string())?;
    if bytes.len() > 32 * 1024 * 1024
        || bytes.get(..8) != Some(b"\x89PNG\r\n\x1a\n")
        || bytes.len() < 24
    {
        return Err("wallpaper PNG".into());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err("wallpaper dimensions".into());
    }
    let source = resvg::tiny_skia::Pixmap::decode_png(&bytes).map_err(|e| e.to_string())?;
    let packed = qoi::encode_to_vec(source.data(), width, height).map_err(|e| e.to_string())?;
    let output = dir.join("bloom.qoi");
    if check {
        if fs::read(&output).map_err(|e| e.to_string())? != packed {
            return Err("stale lossless wallpaper".into());
        }
    } else {
        fs::write(output, packed).map_err(|e| e.to_string())?;
    }
    for (ow, oh) in [(1024, 768), (1280, 720), (116, 76), (110, 86), (320, 200)] {
        let source = if ow >= 1024 {
            source.clone()
        } else {
            raster::prefilter(&source, ow, oh)?
        };
        let (width, height) = (source.width(), source.height());
        let mut target = resvg::tiny_skia::Pixmap::new(ow, oh).ok_or("wallpaper allocation")?;
        let scale = (ow as f32 / width as f32).max(oh as f32 / height as f32);
        let transform = resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(
            (ow as f32 - width as f32 * scale) / 2.0,
            (oh as f32 - height as f32 * scale) / 2.0,
        );
        target.draw_pixmap(
            0,
            0,
            source.as_ref(),
            &resvg::tiny_skia::PixmapPaint {
                quality: resvg::tiny_skia::FilterQuality::Bicubic,
                ..Default::default()
            },
            transform,
            None,
        );
        let pixels: Vec<u8> = target
            .data()
            .chunks_exact(4)
            .flat_map(|p| [p[2], p[1], p[0]])
            .collect();
        let output = dir.join(format!("bloom-{ow}x{oh}.bgr"));
        // The window server embeds the 1024x768 frame raw-deflated.
        let deflated = output.with_extension("bgr.deflate");
        if check {
            if fs::read(output).map_err(|e| e.to_string())? != pixels {
                return Err("stale wallpaper export".into());
            }
            if ow == 1024
                && miniz_oxide::inflate::decompress_to_vec(
                    &fs::read(&deflated).map_err(|e| e.to_string())?,
                )
                .ok()
                    != Some(pixels)
            {
                return Err("stale deflated wallpaper export".into());
            }
        } else {
            if ow == 1024 {
                fs::write(&deflated, miniz_oxide::deflate::compress_to_vec(&pixels, 10))
                    .map_err(|e| e.to_string())?;
            }
            fs::write(output, pixels).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn export_background_collection(root: &Path, check: bool) -> Result<(), String> {
    use door_initfs::desktop_assets::{COUNT, HEADER, JPEG, MAGIC, Pack, WALLPAPERS};
    let dir = root.join("assets/wallpapers/windows");
    let manifest = crate::image::read_json(&dir.join("collection.json"))?;
    let entries = manifest["backgrounds"]
        .as_array()
        .ok_or("background inventory")?;
    if entries.len() != 3 {
        return Err("background inventory count".into());
    }
    // The pack: every 26H2 picture as its original JPEG, in pack id order.
    let pictures = dir.join("26h2");
    let inventory = crate::image::read_json(&pictures.join("manifest.json"))?;
    let listed = inventory["pictures"]
        .as_array()
        .ok_or("picture inventory")?;
    if listed.len() != COUNT {
        return Err("picture inventory count".into());
    }
    let mut pack = vec![0; HEADER];
    pack[..8].copy_from_slice(MAGIC);
    pack[8..12].copy_from_slice(&(COUNT as u32).to_le_bytes());
    for (id, item) in listed.iter().enumerate() {
        let file = WALLPAPERS[id].file;
        let path = pictures.join(file);
        if item["id"] != id || item["file"] != file || file_hash(&path)? != item["sha256"] {
            return Err(format!("picture identity {file}"));
        }
        let jpeg = fs::read(&path).map_err(|e| e.to_string())?;
        let at = 16 + id * 16;
        pack[at..at + 4].copy_from_slice(&(id as u32).to_le_bytes());
        let offset = pack.len() as u32;
        pack[at + 4..at + 8].copy_from_slice(&offset.to_le_bytes());
        pack[at + 8..at + 12].copy_from_slice(&(jpeg.len() as u32).to_le_bytes());
        pack[at + 12..at + 16].copy_from_slice(&JPEG.to_le_bytes());
        pack.extend_from_slice(&jpeg);
        // Pictures new in version 2 get one 2x Settings preview; smaller
        // thumbnails are resampled from it at run time.
        if id >= 4 {
            let mut rgba = Vec::new();
            door_window::wallpaper::decode_native(&jpeg, &mut rgba)?;
            let size = resvg::tiny_skia::IntSize::from_wh(3840, 2400).ok_or("picture extent")?;
            let source = resvg::tiny_skia::Pixmap::from_vec(rgba, size).ok_or("picture pixmap")?;
            let preview = scaled_preview(&source, 640, 400)?;
            let stem = file.trim_end_matches(".jpg");
            asset_output(
                &pictures.join(format!("{stem}-640x400.qoi")),
                &qoi::encode_to_vec(preview.data(), 640, 400).map_err(|e| e.to_string())?,
                check,
            )?;
        }
    }
    for (id, name) in ["bloom", "bloom-dark", "glow", "sunrise"]
        .into_iter()
        .enumerate()
    {
        if id > 0 {
            let item = &entries[id - 1];
            if item["id"] != id
                || item["name"] != name
                || file_hash(&dir.join(format!("{name}.jpg")))? != item["source_sha256"]
                || file_hash(&dir.join(format!("{name}.png")))? != item["png_sha256"]
            {
                return Err("background identity".into());
            }
        }
        let source = resvg::tiny_skia::Pixmap::decode_png(
            &fs::read(dir.join(format!("{name}.png"))).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if (source.width(), source.height()) != (3840, 2400)
            || source.data().chunks_exact(4).any(|p| p[3] != 255)
        {
            return Err("background extent/opacity".into());
        }
        for (width, height) in [
            (110, 86),
            (320, 200),
            (116, 76),
            (220, 172),
            (640, 400),
            (232, 152),
            (330, 258),
            (960, 600),
            (348, 228),
        ] {
            let source = raster::prefilter(&source, width, height)?;
            let mut target =
                resvg::tiny_skia::Pixmap::new(width, height).ok_or("preview memory")?;
            let scale =
                (width as f32 / source.width() as f32).max(height as f32 / source.height() as f32);
            target.draw_pixmap(
                0,
                0,
                source.as_ref(),
                &resvg::tiny_skia::PixmapPaint {
                    quality: resvg::tiny_skia::FilterQuality::Bicubic,
                    ..Default::default()
                },
                resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(
                    (width as f32 - source.width() as f32 * scale) / 2.0,
                    (height as f32 - source.height() as f32 * scale) / 2.0,
                ),
                None,
            );
            let bytes: Vec<_> = target
                .data()
                .chunks_exact(4)
                .flat_map(|p| [p[2], p[1], p[0]])
                .collect();
            asset_output(
                &dir.join(format!("{name}-{width}x{height}.bgr")),
                &bytes,
                check,
            )?;
            let compressed =
                qoi::encode_to_vec(target.data(), width, height).map_err(|e| e.to_string())?;
            asset_output(
                &dir.join(format!("{name}-{width}x{height}.qoi")),
                &compressed,
                check,
            )?;
        }
    }
    let size = pack.len() as u32;
    pack[12..16].copy_from_slice(&size.to_le_bytes());
    Pack::parse(&pack).ok_or("desktop pack exceeds bounded resource policy")?;
    asset_output(&dir.join("desktop-assets.blob"), &pack, check)
}
/// Cover-fit `source` into a `width` x `height` preview: area prefilter, then
/// bicubic (the same resampling as the version 1 previews).
fn scaled_preview(
    source: &resvg::tiny_skia::Pixmap,
    width: u32,
    height: u32,
) -> Result<resvg::tiny_skia::Pixmap, String> {
    let source = raster::prefilter(source, width, height)?;
    let mut target = resvg::tiny_skia::Pixmap::new(width, height).ok_or("preview memory")?;
    let scale = (width as f32 / source.width() as f32).max(height as f32 / source.height() as f32);
    target.draw_pixmap(
        0,
        0,
        source.as_ref(),
        &resvg::tiny_skia::PixmapPaint {
            quality: resvg::tiny_skia::FilterQuality::Bicubic,
            ..Default::default()
        },
        resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(
            (width as f32 - source.width() as f32 * scale) / 2.0,
            (height as f32 - source.height() as f32 * scale) / 2.0,
        ),
        None,
    );
    Ok(target)
}
fn asset_output(path: &Path, bytes: &[u8], check: bool) -> Result<(), String> {
    if check {
        if fs::read(path).map_err(|e| e.to_string())? != bytes {
            return Err(format!("stale asset {}", path.display()));
        }
    } else {
        fs::write(path, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub fn export_branding(root: &Path, check: bool) -> Result<(), String> {
    for name in ["door", "preview", "monitor", "doorsh"] {
        let source = root.join(format!("assets/branding/{name}.svg"));
        let tree = resvg::usvg::Tree::from_data(
            &fs::read(&source).map_err(|e| e.to_string())?,
            &resvg::usvg::Options::default(),
        )
        .map_err(|e| e.to_string())?;
        if tree.size().width() != 24.0 || tree.size().height() != 24.0 {
            return Err("Door branding extent".into());
        }
        // 128-288 serve the boot splash and large identity surfaces at 1x-3x.
        for size in &[
            16, 20, 24, 28, 32, 40, 48, 56, 60, 64, 72, 80, 84, 96, 120, 128, 160, 192, 240, 288,
        ] {
            let size = *size;
            let mut pixmap =
                resvg::tiny_skia::Pixmap::new(size, size).ok_or("branding allocation")?;
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::from_scale(size as f32 / 24.0, size as f32 / 24.0),
                &mut pixmap.as_mut(),
            );
            let pixels: Vec<u8> = pixmap
                .data()
                .chunks_exact(4)
                .flat_map(|p| [p[2], p[1], p[0], p[3]])
                .collect();
            let path = source.with_extension(format!("{size}.bgra"));
            if check {
                if fs::read(path).map_err(|e| e.to_string())? != pixels {
                    return Err("stale Door branding".into());
                }
            } else {
                fs::write(path, pixels).map_err(|e| e.to_string())?;
            }
        }
        // Portable previews/desktop identity use PNG frames at their native
        // sizes. ICO stores those complete PNGs, including straight-alpha edges.
        let mut frames = Vec::new();
        for size in [16u32, 24, 32, 48, 64, 128, 256] {
            let mut pixmap =
                resvg::tiny_skia::Pixmap::new(size, size).ok_or("branding allocation")?;
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::from_scale(size as f32 / 24.0, size as f32 / 24.0),
                &mut pixmap.as_mut(),
            );
            let png = pixmap.encode_png().map_err(|e| e.to_string())?;
            if size == 256 {
                branding_file(&source.with_extension("png"), &png, check)?;
            }
            frames.push((size, png));
        }
        let mut ico = Vec::from([0, 0, 1, 0, 7, 0]);
        let mut offset = 6 + 16 * frames.len();
        for (size, png) in &frames {
            ico.extend_from_slice(&[*size as u8, *size as u8, 0, 0, 1, 0, 32, 0]);
            ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
            ico.extend_from_slice(&(offset as u32).to_le_bytes());
            offset += png.len();
        }
        for (_, png) in frames {
            ico.extend_from_slice(&png);
        }
        branding_file(&source.with_extension("ico"), &ico, check)?;
    }
    export_door_mask(root, check)
}
/// Monochrome door mark (spec 140.4): 8-bit coverage masks
/// (`door-mask.<size>.mask`, size*size bytes, the window server's glyph
/// format) tinted by the surrounding control, plus a 256 px preview. Each
/// size must keep the opening between leaf and frame visible.
fn export_door_mask(root: &Path, check: bool) -> Result<(), String> {
    let source = root.join("assets/branding/door-mask.svg");
    let svg = fs::read(&source).map_err(|e| e.to_string())?;
    if svg.windows(8).any(|w| w == b"Gradient") || svg.windows(7).any(|w| w == b"opacity") {
        return Err("door mask must be one flat colour".into());
    }
    let tree = resvg::usvg::Tree::from_data(&svg, &resvg::usvg::Options::default())
        .map_err(|e| e.to_string())?;
    if tree.size().width() != 24.0 || tree.size().height() != 24.0 {
        return Err("door mask extent".into());
    }
    let render = |size: u32| -> Result<resvg::tiny_skia::Pixmap, String> {
        let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).ok_or("mask allocation")?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(size as f32 / 24.0, size as f32 / 24.0),
            &mut pixmap.as_mut(),
        );
        Ok(pixmap)
    };
    for size in [
        16u32, 20, 24, 28, 32, 36, 40, 48, 56, 60, 64, 72, 80, 84, 96, 120, 128, 160, 192, 240, 288,
    ] {
        let pixmap = render(size)?;
        let alpha: Vec<u8> = pixmap.data().chunks_exact(4).map(|p| p[3]).collect();
        // Legibility: across the middle rows, the column centred in the
        // 10.5..12 unit opening is clearly lighter than leaf and frame.
        let column = |units: f32| ((units * size as f32 / 24.0) as usize).min(size as usize - 1);
        let rows = (size as usize * 8 / 24)..(size as usize * 16 / 24);
        let peak = |x: usize| {
            rows.clone()
                .map(|y| alpha[y * size as usize + x])
                .max()
                .unwrap_or(0)
        };
        let gap = peak(column(11.25));
        if gap > 96 || peak(column(9.0)) < 200 || peak(column(12.75)) < 200 {
            return Err(format!(
                "door mask opening is not legible at {size} px (gap coverage {gap})"
            ));
        }
        branding_file(
            &root.join(format!("assets/branding/door-mask.{size}.mask")),
            &alpha,
            check,
        )?;
    }
    let preview = render(256)?.encode_png().map_err(|e| e.to_string())?;
    branding_file(&root.join("assets/branding/door-mask.png"), &preview, check)
}
fn branding_file(path: &Path, bytes: &[u8], check: bool) -> Result<(), String> {
    if check {
        if fs::read(path).map_err(|e| e.to_string())? != bytes {
            return Err("stale portable Door identity".into());
        }
    } else {
        fs::write(path, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

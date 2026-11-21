//! Offline Segoe Fluent masks (and, for entries naming `"font": "clock"` or
//! `"paint"`, the Clock and Paint apps' own icon-font glyphs). Runtime never parses a font or uses
//! the network.
use super::*;
/// Glyph fonts by manifest key: (folder, file), each pinned by its
/// `provenance.json` `font_sha256`.
const FONTS: [(&str, &str, &str); 6] = [
    ("segoe", "segoe-fluent-icons", "SegoeFluentIcons.ttf"),
    ("clock", "clock-icons", "ClockIcons.ttf"),
    ("paint", "paint-icons", "PaintIcons.ttf"),
    ("snip", "snip-icons", "SnSkFluent.ttf"),
    ("msn", "msn-icons", "MSNMDL2.ttf"),
    ("msnx", "msn-icons-extend", "MSNMDL2Extend.ttf"),
];
fn font_bytes(root: &Path, folder: &str, file: &str) -> Result<Vec<u8>, String> {
    let source = root.join("assets/fonts").join(folder);
    let provenance = crate::image::read_json(&source.join("provenance.json"))?;
    let path = source.join(file);
    if file_hash(&path)? != provenance["font_sha256"] {
        return Err(format!("shell glyph font identity: {file}"));
    }
    fs::read(path).map_err(|error| error.to_string())
}
pub(crate) fn export(root: &Path, check: bool) -> Result<(), String> {
    use ab_glyph::{Font, FontRef, PxScale, point};
    let manifest = crate::image::read_json(&root.join("assets/icons/windows-shell/manifest.json"))?;
    let glyphs = manifest["glyphs"]
        .as_array()
        .ok_or("shell glyph inventory")?;
    // Only fonts some entry names are read (Segoe Fluent always).
    let mut loaded: Vec<(&str, Vec<u8>)> = Vec::new();
    for (key, folder, file) in FONTS {
        let used = key == "segoe" || glyphs.iter().any(|g| g["font"] == key);
        if used {
            loaded.push((key, font_bytes(root, folder, file)?));
        }
    }
    let fonts: Vec<(&str, FontRef<'_>)> = loaded
        .iter()
        .map(|(key, bytes)| {
            FontRef::try_from_slice(bytes)
                .map(|f| (*key, f))
                .map_err(|_| format!("shell glyph font {key}"))
        })
        .collect::<Result<_, _>>()?;
    for entry in glyphs {
        let key = entry["font"].as_str().unwrap_or("segoe");
        let font = &fonts
            .iter()
            .find(|(k, _)| *k == key)
            .ok_or_else(|| format!("unknown shell glyph font {key}"))?
            .1;
        let name = entry["name"].as_str().ok_or("shell glyph name")?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        {
            return Err("shell glyph path".into());
        }
        let parse = |codepoint: &serde_json::Value| {
            u32::from_str_radix(codepoint.as_str().ok_or("shell codepoint")?, 16)
                .ok()
                .and_then(char::from_u32)
                .ok_or("shell codepoint")
        };
        // `layers` composes glyphs drawn at one origin, as XAML stacks
        // FontIcons (Windows' filled pin is E840 Pinned + E841 PinnedFill).
        let chars = match entry.get("layers") {
            Some(layers) => layers
                .as_array()
                .filter(|l| !l.is_empty())
                .ok_or("shell glyph layers")?
                .iter()
                .map(parse)
                .collect::<Result<Vec<_>, _>>()?,
            None => vec![parse(&entry["codepoint"])?],
        };
        for size in entry["sizes"].as_array().ok_or("shell glyph sizes")? {
            let size = size
                .as_u64()
                .filter(|size| {
                    [
                        11, 12, 16, 20, 22, 24, 32, 33, 36, 40, 48, 60, 72, 96, 144, 192, 288,
                    ]
                    .contains(size)
                })
                .ok_or("shell glyph extent")? as usize;
            let em = font.units_per_em().ok_or("shell em")?;
            let mut scale = size as f32 * (font.ascent_unscaled() - font.descent_unscaled()) / em;
            let outline_at = |scale: f32| {
                chars
                    .iter()
                    .map(|ch| {
                        let glyph = font
                            .glyph_id(*ch)
                            .with_scale_and_position(PxScale::from(scale), point(0.0, 0.0));
                        font.outline_glyph(glyph).ok_or("shell glyph missing")
                    })
                    .collect::<Result<Vec<_>, _>>()
            };
            let mut outlines = outline_at(scale)?;
            // Glyphs drawn edge to edge of the em (E90F Repair, the Properties
            // wrench) overflow it by a fraction of a pixel; they are fitted to
            // the frame instead of being clipped.
            let extent = outlines.iter().fold(0.0f32, |e, o| {
                let b = o.px_bounds();
                e.max(b.max.x - b.min.x).max(b.max.y - b.min.y)
            });
            if extent > size as f32 {
                scale *= size as f32 / extent;
                outlines = outline_at(scale)?;
            }
            // Centre the union of the layers' pixel bounds.
            let (mut min, mut max) = (outlines[0].px_bounds().min, outlines[0].px_bounds().max);
            for outline in &outlines[1..] {
                let b = outline.px_bounds();
                (min.x, min.y) = (min.x.min(b.min.x), min.y.min(b.min.y));
                (max.x, max.y) = (max.x.max(b.max.x), max.y.max(b.max.y));
            }
            let ox = ((size as f32 - (max.x - min.x)) / 2.0).round() as i32;
            let oy = ((size as f32 - (max.y - min.y)) / 2.0).round() as i32;
            let mut mask = vec![0; size * size];
            for outline in &outlines {
                let b = outline.px_bounds();
                let (dx, dy) = ((b.min.x - min.x) as i32, (b.min.y - min.y) as i32);
                outline.draw(|x, y, coverage| {
                    let (x, y) = (x as i32 + ox + dx, y as i32 + oy + dy);
                    if x >= 0 && y >= 0 && x < size as i32 && y < size as i32 {
                        // Same-colour layers composite as alpha "over".
                        let at = &mut mask[y as usize * size + x as usize];
                        let over = (coverage.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
                        *at = (over + u32::from(*at) * (255 - over) / 255).min(255) as u8;
                    }
                });
            }
            let path = root.join(format!("assets/icons/windows-shell/{name}.{size}.mask"));
            if size.is_multiple_of(3) {
                super::asset_output(
                    &root.join(format!(
                        "assets/icons/windows-shell/{name}.{}-3x.mask",
                        size / 3
                    )),
                    &mask,
                    check,
                )?;
            }
            if check {
                if fs::read(path).map_err(|error| error.to_string())? != mask {
                    return Err("stale shell glyph".into());
                }
            } else {
                fs::write(path, mask).map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(())
}

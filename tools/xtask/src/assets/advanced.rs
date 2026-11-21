//! Original vector artwork, exported into consecutive premultiplied BGRA tiles.
use std::{fs, path::Path};
pub fn export(root: &Path, check: bool) -> Result<(), String> {
    let dir = root.join("assets/icons/advanced");
    let fold = resvg::usvg::Tree::from_data(
        &fs::read(dir.join("device-hero-fold.svg")).map_err(|e| e.to_string())?,
        &resvg::usvg::Options::default(),
    )
    .map_err(|e| e.to_string())?;
    for scale in 1..=3 {
        let mut image =
            resvg::tiny_skia::Pixmap::new(200 * scale, 170 * scale).ok_or("hero fold extent")?;
        resvg::render(
            &fold,
            resvg::tiny_skia::Transform::from_scale(scale as f32, scale as f32),
            &mut image.as_mut(),
        );
        let compressed = qoi::encode_to_vec(image.data(), 200 * scale, 170 * scale)
            .map_err(|e| e.to_string())?;
        super::branding_file(
            &dir.join(format!("device-hero-fold.{scale}.qoi")),
            &compressed,
            check,
        )?;
    }
    let collection =
        crate::image::read_json(&root.join("assets/wallpapers/windows/collection.json"))?;
    if crate::evidence::file_hash(&root.join("assets/wallpapers/windows/bloom-dark.png"))?
        != collection["backgrounds"][0]["png_sha256"]
    {
        return Err("device screen Bloom source identity".into());
    }
    let category_source = fs::read(dir.join("device-categories.svg")).map_err(|e| e.to_string())?;
    let category_tree =
        resvg::usvg::Tree::from_data(&category_source, &resvg::usvg::Options::default())
            .map_err(|e| e.to_string())?;
    if category_tree.size().width() != 256.0 || category_tree.size().height() != 32.0 {
        return Err("device category artwork extent".into());
    }
    for size in [32u32, 64, 96] {
        let mut image =
            resvg::tiny_skia::Pixmap::new(size * 8, size).ok_or("device category allocation")?;
        resvg::render(
            &category_tree,
            resvg::tiny_skia::Transform::from_scale(size as f32 / 32.0, size as f32 / 32.0),
            &mut image.as_mut(),
        );
        let mut bytes = Vec::new();
        for index in 0..8 {
            for y in 0..size {
                let start = ((y * size * 8 + index * size) * 4) as usize;
                for p in image.data()[start..start + size as usize * 4].chunks_exact(4) {
                    bytes.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
                }
            }
        }
        super::branding_file(
            &dir.join(format!("device-categories.{size}.bgra")),
            &bytes,
            check,
        )?;
    }
    let light_manifest =
        crate::image::read_json(&root.join("assets/wallpapers/windows/manifest.json"))?;
    if crate::evidence::file_hash(&root.join("assets/wallpapers/windows/bloom.png"))?
        != light_manifest["png_sha256"]
    {
        return Err("device light Bloom source identity".into());
    }
    for device in ["laptop", "monitor", "tablet"] {
        let template = fs::read_to_string(dir.join(format!("device-{device}.svg")))
            .map_err(|e| e.to_string())?;
        for theme in ["light", "dark"] {
            let wallpaper = resvg::tiny_skia::Pixmap::decode_png(
                &fs::read(root.join(format!(
                    "assets/wallpapers/windows/{}.png",
                    if theme == "dark" {
                        "bloom-dark"
                    } else {
                        "bloom"
                    }
                )))
                .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            let source = template.replace(
                "bloom-dark.png",
                if theme == "dark" {
                    "bloom-dark.png"
                } else {
                    "bloom.png"
                },
            );
            let tree = resvg::usvg::Tree::from_data(
                source.as_bytes(),
                &resvg::usvg::Options {
                    resources_dir: Some(dir.clone()),
                    ..Default::default()
                },
            )
            .map_err(|e| e.to_string())?;
            if tree.size().width() != 200.0 || tree.size().height() != 136.0 {
                return Err("device artwork extent".into());
            }
            for scale in 1..=3 {
                let mut image = resvg::tiny_skia::Pixmap::new(200 * scale, 136 * scale)
                    .ok_or("device artwork allocation")?;
                resvg::render(
                    &tree,
                    resvg::tiny_skia::Transform::from_scale(scale as f32, scale as f32),
                    &mut image.as_mut(),
                );
                // The host vector renderer excludes general raster codecs. Compose
                // the pinned PNG with the existing area-prefiltered bitmap path.
                let (x, y, w, h) = match device {
                    "laptop" => (27, 17, 146, 91),
                    "monitor" => (14, 13, 172, 91),
                    _ => (23, 15, 154, 104),
                };
                let (w, h) = (w * scale, h * scale);
                let reduced = super::raster::prefilter(&wallpaper, w, h)?;
                let mut screen =
                    resvg::tiny_skia::Pixmap::new(w, h).ok_or("device screen allocation")?;
                let cover =
                    (w as f32 / reduced.width() as f32).max(h as f32 / reduced.height() as f32);
                screen.draw_pixmap(
                    0,
                    0,
                    reduced.as_ref(),
                    &resvg::tiny_skia::PixmapPaint {
                        quality: resvg::tiny_skia::FilterQuality::Bicubic,
                        ..Default::default()
                    },
                    resvg::tiny_skia::Transform::from_scale(cover, cover).post_translate(
                        (w as f32 - reduced.width() as f32 * cover) / 2.0,
                        (h as f32 - reduced.height() as f32 * cover) / 2.0,
                    ),
                    None,
                );
                image.draw_pixmap(
                    (x * scale) as i32,
                    (y * scale) as i32,
                    screen.as_ref(),
                    &resvg::tiny_skia::PixmapPaint::default(),
                    resvg::tiny_skia::Transform::identity(),
                    None,
                );
                if device == "tablet" {
                    let mut paint = resvg::tiny_skia::Paint::default();
                    paint.set_color_rgba8(255, 255, 255, 166);
                    image.fill_rect(
                        resvg::tiny_skia::Rect::from_xywh(
                            84.0 * scale as f32,
                            115.0 * scale as f32,
                            32.0 * scale as f32,
                            2.0 * scale as f32,
                        )
                        .ok_or("device gesture extent")?,
                        &paint,
                        resvg::tiny_skia::Transform::identity(),
                        None,
                    );
                }
                let bytes: Vec<u8> = image
                    .data()
                    .chunks_exact(4)
                    .flat_map(|p| [p[2], p[1], p[0], p[3]])
                    .collect();
                let compressed = qoi::encode_to_vec(image.data(), 200 * scale, 136 * scale)
                    .map_err(|e| e.to_string())?;
                super::branding_file(
                    &dir.join(format!("device-{device}-{theme}.{scale}.bgra")),
                    &bytes,
                    check,
                )?;
                super::branding_file(
                    &dir.join(format!("device-{device}-{theme}.{scale}.qoi")),
                    &compressed,
                    check,
                )?;
                // Retain the existing dark aliases for earlier artwork consumers.
                if theme == "dark" {
                    super::branding_file(
                        &dir.join(format!("device-{device}.{scale}.bgra")),
                        &bytes,
                        check,
                    )?;
                    super::branding_file(
                        &dir.join(format!("device-{device}.{scale}.qoi")),
                        &compressed,
                        check,
                    )?;
                }
            }
        }
    }
    let svg = fs::read(dir.join("settings.svg")).map_err(|e| e.to_string())?;
    let tree = resvg::usvg::Tree::from_data(&svg, &resvg::usvg::Options::default())
        .map_err(|e| e.to_string())?;
    if tree.size().width() != 512.0 || tree.size().height() != 32.0 {
        return Err("advanced artwork extent".into());
    }
    for size in [20u32, 32, 40, 60, 64, 96] {
        let mut image =
            resvg::tiny_skia::Pixmap::new(size * 16, size).ok_or("artwork allocation")?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(size as f32 / 32.0, size as f32 / 32.0),
            &mut image.as_mut(),
        );
        let mut tiles = Vec::with_capacity((16 * size * size * 4) as usize);
        for index in 0..16 {
            let tile_at = tiles.len();
            for y in 0..size {
                let start = ((y * size * 16 + index * size) * 4) as usize;
                for pixel in image.data()[start..start + size as usize * 4].chunks_exact(4) {
                    tiles.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
                }
            }
            let rgba: Vec<_> = tiles[tile_at..]
                .chunks_exact(4)
                .flat_map(|p| [p[2], p[1], p[0], p[3]])
                .collect();
            let compressed = qoi::encode_to_vec(&rgba, size, size).map_err(|e| e.to_string())?;
            super::branding_file(
                &dir.join(format!("tool-{index}.{size}.qoi")),
                &compressed,
                check,
            )?;
        }
        super::branding_file(&dir.join(format!("settings.{size}.bgra")), &tiles, check)?;
    }
    Ok(())
}

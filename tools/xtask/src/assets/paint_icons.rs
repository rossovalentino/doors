//! Windows 11 26H2 Paint's own toolbar artwork: the 16 px SVGs the Paint
//! package ships per theme (`Assets\theme-light`, `theme-dark`), an installed
//! reference acquired from the owner's system for local development
//! (`assets acquire-paint-icons`), then rasterized offline into premultiplied
//! BGRA frames Door's Paint paints 1:1.
use crate::evidence::{file_hash, write_json};
use serde_json::json;
use std::{fs, path::Path};

/// Doors key and the package file name.
pub const ICONS: &[(&str, &str)] = &[
    ("pencil", "PencilTool.svg"),
    ("fill", "FillTool.svg"),
    ("text", "TextTool.svg"),
    ("eraser", "EraserTool.svg"),
    ("picker", "ColorPickerTool.svg"),
    ("magnifier", "MagnifierTool.svg"),
    ("brush", "BrushIcon.svg"),
    ("calligraphy-brush", "CalligraphyBrushIcon.svg"),
    ("calligraphy-pen", "CalligraphyPenIcon.svg"),
    ("airbrush", "AirBrushIcon.svg"),
    ("oil", "OilBrushIcon.svg"),
    ("crayon", "CrayonIcon.svg"),
    ("marker", "MarkerIcon.svg"),
    ("natural-pencil", "NaturalPencilIcon.svg"),
    ("watercolor", "WaterColorIcon.svg"),
    ("select", "RectangularSelectionLarge.svg"),
    ("freeform", "FreeFormSelectionLarge.svg"),
    ("object-select", "ObjectSelectTool.svg"),
    ("resize", "Resize.svg"),
    ("rotate", "RotateRight.svg"),
    ("flip", "Flip.svg"),
    ("remove-background", "BGRemoval.svg"),
    ("layers", "Layers.svg"),
    ("eye-on", "EyeOn.svg"),
    ("eye-off", "EyeOff.svg"),
    ("save", "Save.svg"),
    ("print", "Print.svg"),
    ("shapes", "Shapes.svg"),
    ("stroke-size", "ShapesStrokeSize.svg"),
    ("stroke-size-disabled", "DisabledShapesStrokeSize.svg"),
    ("size", "SizeButtonIcon.svg"),
    ("opacity", "OpacityIcon.svg"),
    ("colors", "Colors.svg"),
    ("zoom-in", "ZoomIn.svg"),
    ("zoom-out", "ZoomOut.svg"),
    ("cursor", "Cursor.svg"),
    ("canvas-size", "CanvasSize.svg"),
    ("object-size", "ObjectSize.svg"),
    ("chevron-down", "DownArrow.svg"),
    ("chevron-up", "UpArrow.svg"),
    ("close", "Close.svg"),
    ("tools", "Tools.svg"),
    ("tolerance", "ToleranceIcon.svg"),
    ("stickers", "Stickers2Icon.svg"),
    ("line", "LineTool.svg"),
    ("curve", "CurveTool.svg"),
    ("oval", "OvalTool.svg"),
    ("rectangle", "RectangleTool.svg"),
    ("rounded-rectangle", "RoundedRectTool.svg"),
    ("polygon", "PolygonTool.svg"),
    ("triangle", "TriangleTool.svg"),
    ("right-triangle", "RightTriangleTool.svg"),
    ("diamond", "DiamondTool.svg"),
    ("pentagon", "PentagonTool.svg"),
    ("hexagon", "HexagonTool.svg"),
    ("right-arrow", "RightArrowTool.svg"),
    ("left-arrow", "LeftArrowTool.svg"),
    ("up-arrow", "UpArrowTool.svg"),
    ("down-arrow", "DownArrowTool.svg"),
    ("star4", "FourPointStarTool.svg"),
    ("star5", "FivePointStarTool.svg"),
    ("star6", "SixPointStarTool.svg"),
    ("rounded-callout", "RoundedRectCalloutTool.svg"),
    ("oval-callout", "OvalCalloutTool.svg"),
    ("cloud-callout", "CloudCalloutTool.svg"),
    ("heart", "HeartTool.svg"),
    ("lightning", "LightningTool.svg"),
];
pub const THEMES: [(&str, &str); 2] = [("light", "theme-light"), ("dark", "theme-dark")];
/// Frames of the 16 px icons at 1x/2x/3x; the ribbon's large 20 px icons
/// (selection, resize, the brushes button, layers) add 20 and 40.
pub fn sizes(key: &str) -> &'static [u32] {
    const LARGE: &[&str] = &[
        "select",
        "freeform",
        "resize",
        "layers",
        "brush",
        "calligraphy-brush",
        "calligraphy-pen",
        "airbrush",
        "oil",
        "crayon",
        "marker",
        "natural-pencil",
        "watercolor",
        "stickers",
        "shapes",
    ];
    if LARGE.contains(&key) {
        &[16, 20, 32, 40, 48]
    } else {
        &[16, 32, 48]
    }
}
const DIR: &str = "assets/icons/paint";

/// Copies the icons from Paint's package `Assets` folder and records where
/// they came from.
pub fn acquire(root: &Path, assets: &str) -> Result<(), String> {
    let assets = Path::new(assets);
    let mut files = Vec::new();
    for (theme, folder) in THEMES {
        let out = root.join(DIR).join(theme);
        fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        for (key, name) in ICONS {
            let source = assets.join(folder).join(name);
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
            "method": "byte copy of the 26H2 Paint package toolbar SVGs; rasterized offline",
            "files": files}),
    )?;
    println!(
        "Acquired {} Paint icons in {} themes",
        ICONS.len(),
        THEMES.len()
    );
    Ok(())
}

pub fn export(root: &Path, check: bool) -> Result<(), String> {
    let dir = root.join(DIR);
    let source = crate::image::read_json(&dir.join("source.json"))?;
    let files = source["files"].as_array().ok_or("paint icon inventory")?;
    for (theme, _) in THEMES {
        for (key, _) in ICONS {
            let path = dir.join(theme).join(format!("{key}.svg"));
            let pinned = files
                .iter()
                .find(|f| f["key"] == *key && f["theme"] == theme)
                .ok_or("paint icon provenance")?;
            if file_hash(&path)? != pinned["sha256"] {
                return Err(format!("paint icon {theme}/{key} changed"));
            }
            let tree = resvg::usvg::Tree::from_data(
                &fs::read(&path).map_err(|e| e.to_string())?,
                &resvg::usvg::Options::default(),
            )
            .map_err(|e| e.to_string())?;
            let base = tree.size().width().max(tree.size().height());
            for &size in sizes(key) {
                let mut image =
                    resvg::tiny_skia::Pixmap::new(size, size).ok_or("paint icon extent")?;
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

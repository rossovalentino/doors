//! Door's original File Explorer folder art (P14 visual fidelity): the plain
//! folder, the back/front layers that sandwich folder contents, a folder
//! holding files, and the six known folders. The owner's 26H2 reference
//! (`reports/artifacts/reference-26h2/explorer-home.png`) fixes the silhouette and
//! palette roles; the drawing is Door's own.
//!
//! Every size is laid out on its own pixel grid: outer edges, the tab and the
//! plate tops snap to whole pixels, and the tab meets the body through one
//! smooth shoulder curve, so no size has the stepped notch or the blur of a
//! resampled frame. resvg renders each SVG; the guest loads premultiplied
//! RGBA QOI frames (`assets/icons/explorer/<name>.<px>.qoi`). Like Windows'
//! own frames, the known folders draw a compact glyph at 32 px and below and
//! the folder at 40 px and above.
use super::*;
use std::fmt::Write as _;

/// Physical frame sizes `desktop/workspace/src/icons.rs` loads (16/20/40/80 px
/// at 1x-3x).
const SIZES: [u32; 10] = [16, 20, 32, 40, 48, 60, 80, 120, 160, 240];
/// Contact sheets written to `target/explorer-review/` by `build`.
const REVIEW: [u32; 4] = [16, 32, 48, 96];
pub(crate) const NAMES: [&str; 10] = [
    "folder",
    "folder-back",
    "folder-front",
    "folder-files",
    "desktop",
    "documents",
    "downloads",
    "music",
    "pictures",
    "videos",
];

/// Folder geometry for one frame size, in pixels. Mirrors
/// `desktop/workspace/src/icons.rs::folder_content_box`.
struct Grid {
    u: f32,
    left: f32,
    right: f32,
    tab_top: f32,
    tab_end: f32,
    tab_foot: f32,
    back_top: f32,
    front_top: f32,
    live_top: f32,
    bottom: f32,
}
fn grid(size: u32) -> Grid {
    let s = size as f32;
    let u = s / 48.0;
    let left = (3.0 * u).round().max(1.0);
    let tab_top = (7.0 * u).round();
    let back_top = (11.0 * u).round().max(tab_top + 2.0);
    let front_top = (12.0 * u).round().max(back_top + 1.0);
    let tab_end = (17.0 * u).round();
    Grid {
        u,
        left,
        right: s - left,
        tab_top,
        tab_end,
        tab_foot: tab_end + (4.0 * u).max(1.5),
        back_top,
        front_top,
        live_top: (22.0 * u).round(),
        bottom: (40.0 * u).round(),
    }
}

struct Palette {
    back: (&'static str, &'static str),
    front: (&'static str, &'static str),
    highlight: &'static str,
    rim: &'static str,
}
const YELLOW: Palette = Palette {
    back: ("#FDBF1B", "#EAA210"),
    front: ("#FFE28E", "#FFCF4D"),
    highlight: "#FFF1C4",
    rim: "#E3A214",
};
fn known(name: &str) -> Palette {
    match name {
        "desktop" => Palette {
            back: ("#1477D2", "#0A5BB5"),
            front: ("#3ED2EC", "#178AD3"),
            highlight: "#8BE9F6",
            rim: "#0B4D86",
        },
        "documents" => Palette {
            back: ("#71899B", "#5A7184"),
            front: ("#C2D1DF", "#8CA2B8"),
            highlight: "#E3ECF4",
            rim: "#667E93",
        },
        "downloads" => Palette {
            back: ("#12876A", "#0B6F57"),
            front: ("#2CC49F", "#0E979E"),
            highlight: "#7DE3C9",
            rim: "#0A6E6E",
        },
        "music" => Palette {
            back: ("#DE6C16", "#C1530C"),
            front: ("#F69A62", "#E26C8D"),
            highlight: "#FBC6A3",
            rim: "#B9506B",
        },
        "pictures" => Palette {
            back: ("#1477D2", "#0A5BB5"),
            front: ("#41AAF2", "#1773D2"),
            highlight: "#97D3FA",
            rim: "#0E559F",
        },
        _ => Palette {
            back: ("#9457D6", "#7A3DC0"),
            front: ("#B672F3", "#8B42DA"),
            highlight: "#D7AEFA",
            rim: "#6A2FAE",
        },
    }
}

fn f(v: f32) -> String {
    let mut s = format!("{v:.3}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    s
}
/// Back plate: tab with a rounded top-left corner, a smooth shoulder down
/// to the body (no notch), rounded body corners.
fn back_path(g: &Grid) -> String {
    let (rt, rb, rbb) = (1.5 * g.u, 1.5 * g.u, 2.5 * g.u);
    let w = g.tab_foot - g.tab_end;
    format!(
        "M{l} {a}V{b}A{rt} {rt} 0 0 1 {c} {t}H{e}C{e1} {t} {f1} {k} {ft} {k}H{d}A{rb} {rb} 0 0 1 {r} {g1}V{h}A{rbb} {rbb} 0 0 1 {i} {bo}H{j}A{rbb} {rbb} 0 0 1 {l} {a}Z",
        l = f(g.left),
        a = f(g.bottom - rbb),
        b = f(g.tab_top + rt),
        rt = f(rt),
        c = f(g.left + rt),
        t = f(g.tab_top),
        e = f(g.tab_end),
        e1 = f(g.tab_end + 0.5 * w),
        f1 = f(g.tab_foot - 0.5 * w),
        k = f(g.back_top),
        ft = f(g.tab_foot),
        d = f(g.right - rb),
        rb = f(rb),
        r = f(g.right),
        g1 = f(g.back_top + rb),
        h = f(g.bottom - rbb),
        rbb = f(rbb),
        i = f(g.right - rbb),
        bo = f(g.bottom),
        j = f(g.left + rbb),
    )
}
/// A plate from `top` to `bottom` across the folder, with small top and
/// larger bottom radii.
fn plate_path(g: &Grid, top: f32, bottom: f32) -> String {
    let rt = (1.5 * g.u).min((bottom - top) / 2.0);
    let rb = (2.5 * g.u).min((bottom - top) / 2.0);
    format!(
        "M{l} {a}A{rt} {rt} 0 0 1 {b} {t}H{c}A{rt} {rt} 0 0 1 {r} {d}V{e}A{rb} {rb} 0 0 1 {h} {bo}H{i}A{rb} {rb} 0 0 1 {l} {e}Z",
        l = f(g.left),
        a = f(top + rt),
        rt = f(rt),
        b = f(g.left + rt),
        t = f(top),
        c = f(g.right - rt),
        r = f(g.right),
        d = f(top + rt),
        e = f(bottom - rb),
        rb = f(rb),
        h = f(g.right - rb),
        bo = f(bottom),
        i = f(g.left + rb),
    )
}
fn gradient(id: &str, (from, to): (&str, &str), (x1, y1, x2, y2): (f32, f32, f32, f32)) -> String {
    format!(
        "<linearGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"><stop offset=\"0\" stop-color=\"{from}\"/><stop offset=\"1\" stop-color=\"{to}\"/></linearGradient>",
        f(x1),
        f(y1),
        f(x2),
        f(y2)
    )
}
/// The front plate as three layers: a darker rim (bottom edge), a light top
/// edge and the gradient face, each the same rounded plate.
fn front(svg: &mut String, g: &Grid, p: &Palette, top: f32, id: &str) {
    let edge = (0.75 * g.u).max(1.0);
    let rim = (0.6 * g.u).max(1.0);
    let _ = write!(
        svg,
        "<defs>{}</defs><path d=\"{}\" fill=\"{}\"/><path d=\"{}\" fill=\"{}\"/><path d=\"{}\" fill=\"url(#{id})\"/>",
        gradient(id, p.front, (g.left, top, g.right, g.bottom)),
        plate_path(g, top, g.bottom),
        p.rim,
        plate_path(g, top, g.bottom - rim),
        p.highlight,
        plate_path(g, top + edge, g.bottom - rim),
    );
}
fn back(svg: &mut String, g: &Grid, p: &Palette) {
    let _ = write!(
        svg,
        "<defs>{}</defs><path d=\"{}\" fill=\"url(#back)\"/>",
        gradient("back", p.back, (0.0, g.tab_top, 0.0, g.bottom)),
        back_path(g),
    );
}
/// Two sheets of paper standing in the folder (folder holds files).
fn papers(svg: &mut String, g: &Grid) {
    let u = g.u;
    let line = (0.5 * u).max(0.75);
    for (x0, x1, y, fill) in [
        (
            g.left + 4.0 * u,
            g.right - 9.0 * u,
            g.back_top + 1.0 * u,
            "#F1F1F1",
        ),
        (
            g.left + 8.0 * u,
            g.right - 4.0 * u,
            g.back_top + 3.0 * u,
            "#FFFFFF",
        ),
    ] {
        let _ = write!(
            svg,
            "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"{fill}\" stroke=\"#C9C9C9\" stroke-width=\"{}\"/>",
            f(x0),
            f(y),
            f(x1 - x0),
            f(g.bottom - 3.0 * u - y),
            f(1.0 * u),
            f(line * 0.8),
        );
        for row in 0..3 {
            let ly = y + (2.5 + 2.2 * row as f32) * u;
            if ly + line < g.live_top {
                let _ = write!(
                    svg,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#D5D5D5\"/>",
                    f(x0 + 2.5 * u),
                    f(ly),
                    f((x1 - x0 - 5.0 * u) * if row == 2 { 0.6 } else { 1.0 }),
                    f(line),
                );
            }
        }
    }
}
/// White emblem on a known folder's front plate.
fn emblem(svg: &mut String, g: &Grid, name: &str) {
    let u = g.u;
    let (cx, cy) = (
        (g.left + g.right) / 2.0,
        (g.front_top + g.bottom) / 2.0 + 0.5 * u,
    );
    let k = 0.5 * (g.bottom - g.front_top);
    let clip = format!(
        "<clipPath id=\"face\"><path d=\"{}\"/></clipPath>",
        plate_path(g, g.front_top, g.bottom - (0.6 * u).max(1.0))
    );
    let stroke = |w: f32| {
        format!(
            "fill=\"none\" stroke=\"#FFFFFF\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"",
            f(w)
        )
    };
    match name {
        "desktop" => {
            let sq = 2.4 * u;
            let _ = write!(
                svg,
                "<defs>{clip}</defs><g clip-path=\"url(#face)\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#0B4D86\"/></g>",
                f(g.left),
                f(g.bottom - 3.0 * u),
                f(g.right - g.left),
                f(3.0 * u),
            );
            for (x, y, w) in [
                (g.left + 3.0 * u, g.front_top + 3.5 * u, sq),
                (g.left + 3.0 * u, g.front_top + 7.0 * u, sq),
                (g.right - 7.5 * u, g.bottom - 2.0 * u, 1.2 * u),
                (g.right - 5.0 * u, g.bottom - 2.0 * u, 1.2 * u),
            ] {
                let _ = write!(
                    svg,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"#FFFFFF\"/>",
                    f(x),
                    f(y),
                    f(w),
                    f(w),
                    f(0.3 * u)
                );
            }
        }
        "documents" => {
            let h = 0.11 * k;
            for (x0, x1, y) in [
                (cx - 0.62 * k, cx + 0.06 * k, cy - 0.42 * k),
                (cx - 0.62 * k, cx + 0.06 * k, cy - 0.16 * k),
                (cx - 0.62 * k, cx + 0.62 * k, cy + 0.12 * k),
                (cx - 0.62 * k, cx + 0.62 * k, cy + 0.38 * k),
            ] {
                let _ = write!(
                    svg,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"#FFFFFF\"/>",
                    f(x0),
                    f(y - h / 2.0),
                    f(x1 - x0),
                    f(h),
                    f(h / 3.0)
                );
            }
            let _ = write!(
                svg,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"#FFFFFF\"/>",
                f(cx + 0.22 * k),
                f(cy - 0.5 * k),
                f(0.4 * k),
                f(0.4 * k),
                f(0.06 * k)
            );
        }
        "downloads" => {
            let w = 0.13 * k;
            let _ = write!(
                svg,
                "<path d=\"M{} {}V{}M{} {}L{} {}L{} {}M{} {}H{}\" {}/>",
                f(cx),
                f(cy - 0.55 * k),
                f(cy + 0.22 * k),
                f(cx - 0.36 * k),
                f(cy - 0.12 * k),
                f(cx),
                f(cy + 0.24 * k),
                f(cx + 0.36 * k),
                f(cy - 0.12 * k),
                f(cx - 0.46 * k),
                f(cy + 0.55 * k),
                f(cx + 0.46 * k),
                stroke(w)
            );
        }
        "music" => {
            let _ = write!(
                svg,
                "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\" transform=\"rotate(-18 {} {})\" fill=\"#FFFFFF\"/><path d=\"M{} {}V{}C{} {} {} {} {} {}\" {}/>",
                f(cx - 0.18 * k),
                f(cy + 0.4 * k),
                f(0.25 * k),
                f(0.19 * k),
                f(cx - 0.18 * k),
                f(cy + 0.4 * k),
                f(cx + 0.02 * k),
                f(cy + 0.38 * k),
                f(cy - 0.58 * k),
                f(cx + 0.3 * k),
                f(cy - 0.5 * k),
                f(cx + 0.45 * k),
                f(cy - 0.3 * k),
                f(cx + 0.38 * k),
                f(cy - 0.08 * k),
                stroke(0.13 * k)
            );
        }
        "pictures" => {
            let _ = write!(
                svg,
                "<defs>{clip}</defs><g clip-path=\"url(#face)\"><path d=\"M{} {}L{} {}Q{} {} {} {}L{} {}Z\" fill=\"#FFFFFF\"/></g><circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"#FFFFFF\"/>",
                f(cx - 1.3 * k),
                f(g.bottom + u),
                f(cx - 0.12 * k),
                f(cy - 0.2 * k),
                f(cx),
                f(cy - 0.36 * k),
                f(cx + 0.12 * k),
                f(cy - 0.2 * k),
                f(cx + 1.3 * k),
                f(g.bottom + u),
                f(cx + 0.68 * k),
                f(cy - 0.5 * k),
                f(0.16 * k),
            );
        }
        _ => {
            let side = 2.0 * u;
            let rows = 4;
            let span = g.bottom - g.front_top - 6.0 * u;
            for i in 0..rows {
                let y = g.front_top + 3.0 * u + span * i as f32 / (rows - 1) as f32 - side / 2.0;
                for x in [g.left + 2.5 * u, g.right - 2.5 * u - side] {
                    let _ = write!(
                        svg,
                        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"#FFFFFF\" fill-opacity=\"0.92\"/>",
                        f(x),
                        f(y),
                        f(side),
                        f(side),
                        f(0.4 * u)
                    );
                }
            }
            let _ = write!(
                svg,
                "<path d=\"M{} {}L{} {}L{} {}Z\" fill=\"#FFFFFF\" stroke=\"#FFFFFF\" stroke-width=\"{}\" stroke-linejoin=\"round\"/>",
                f(cx - 0.3 * k),
                f(cy - 0.42 * k),
                f(cx + 0.45 * k),
                f(cy),
                f(cx - 0.3 * k),
                f(cy + 0.42 * k),
                f(0.08 * k),
            );
        }
    }
}
/// Known folders at 32 px and below: a compact glyph on a 16-unit grid
/// (scaled 1x, 1.25x, 2x), as Windows' small imageres frames.
fn small(svg: &mut String, name: &str, size: u32) {
    let p = known(name);
    let _ = write!(
        svg,
        "<g transform=\"scale({})\"><defs>{}</defs>",
        f(size as f32 / 16.0),
        gradient("glyph", p.front, (1.0, 1.0, 15.0, 15.0))
    );
    let body = match name {
        // Whole-unit edges: crisp at 16 and 32 px.
        "desktop" => {
            "<rect x=\"1\" y=\"3\" width=\"14\" height=\"10\" rx=\"1.5\" fill=\"url(#glyph)\"/><path d=\"M1 11H15V11.5A1.5 1.5 0 0 1 13.5 13H2.5A1.5 1.5 0 0 1 1 11.5Z\" fill=\"#0B4D86\"/><rect x=\"3\" y=\"5\" width=\"2\" height=\"2\" rx=\".3\" fill=\"#FFF\"/><rect x=\"3\" y=\"8\" width=\"2\" height=\"2\" rx=\".3\" fill=\"#FFF\"/>"
        }
        "documents" => {
            "<rect x=\"3\" y=\"1\" width=\"10\" height=\"14\" rx=\"1.5\" fill=\"url(#glyph)\"/><rect x=\"5\" y=\"4\" width=\"3\" height=\"1\" fill=\"#FFF\"/><rect x=\"5\" y=\"6\" width=\"3\" height=\"1\" fill=\"#FFF\"/><rect x=\"9\" y=\"4\" width=\"2\" height=\"3\" fill=\"#FFF\"/><rect x=\"5\" y=\"9\" width=\"6\" height=\"1\" fill=\"#FFF\"/><rect x=\"5\" y=\"11\" width=\"6\" height=\"1\" fill=\"#FFF\"/>"
        }
        "downloads" => {
            "<path d=\"M8 1.5V10M3.75 6.25L8 10.5L12.25 6.25\" fill=\"none\" stroke=\"url(#glyph)\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/><rect x=\"2\" y=\"13\" width=\"12\" height=\"2\" rx=\"1\" fill=\"url(#glyph)\"/>"
        }
        "music" => {
            "<circle cx=\"8\" cy=\"8\" r=\"7\" fill=\"url(#glyph)\"/><ellipse cx=\"6.6\" cy=\"10.6\" rx=\"1.9\" ry=\"1.5\" transform=\"rotate(-18 6.6 10.6)\" fill=\"#FFF\"/><path d=\"M8.1 10.4V4.2C9.6 4.6 10.6 5.4 10.5 6.9\" fill=\"none\" stroke=\"#FFF\" stroke-width=\"1.3\" stroke-linecap=\"round\"/>"
        }
        "pictures" => {
            "<clipPath id=\"pic\"><rect x=\"1\" y=\"2\" width=\"14\" height=\"12\" rx=\"1.5\"/></clipPath><rect x=\"1\" y=\"2\" width=\"14\" height=\"12\" rx=\"1.5\" fill=\"url(#glyph)\"/><path clip-path=\"url(#pic)\" d=\"M0 15L7 7.6Q8 6.6 9 7.6L16 15Z\" fill=\"#FFF\"/><circle cx=\"11.5\" cy=\"5\" r=\"1.3\" fill=\"#FFF\"/>"
        }
        _ => {
            "<rect x=\"1\" y=\"2\" width=\"14\" height=\"12\" rx=\"1.5\" fill=\"url(#glyph)\"/><path d=\"M2 4h1v1h-1zM2 7.5h1v1h-1zM2 11h1v1h-1zM13 4h1v1h-1zM13 7.5h1v1h-1zM13 11h1v1h-1z\" fill=\"#FFF\" fill-opacity=\".9\"/><path d=\"M6.5 5.5L10.5 8L6.5 10.5Z\" fill=\"#FFF\" stroke=\"#FFF\" stroke-width=\".6\" stroke-linejoin=\"round\"/>"
        }
    };
    svg.push_str(body);
    svg.push_str("</g>");
}
/// The SVG for one icon at one frame size.
pub(crate) fn svg(name: &str, size: u32) -> String {
    let g = grid(size);
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{size}\" height=\"{size}\" viewBox=\"0 0 {size} {size}\">"
    );
    match name {
        "folder" => {
            back(&mut svg, &g, &YELLOW);
            front(&mut svg, &g, &YELLOW, g.front_top, "front");
        }
        "folder-back" => back(&mut svg, &g, &YELLOW),
        "folder-front" => front(&mut svg, &g, &YELLOW, g.live_top, "front"),
        "folder-files" => {
            back(&mut svg, &g, &YELLOW);
            papers(&mut svg, &g);
            front(&mut svg, &g, &YELLOW, g.live_top, "front");
        }
        _ if size <= 32 => small(&mut svg, name, size),
        _ => {
            let p = known(name);
            back(&mut svg, &g, &p);
            front(&mut svg, &g, &p, g.front_top, "front");
            emblem(&mut svg, &g, name);
        }
    }
    svg.push_str("</svg>");
    svg
}
fn render(name: &str, size: u32) -> Result<resvg::tiny_skia::Pixmap, String> {
    let tree = resvg::usvg::Tree::from_str(&svg(name, size), &resvg::usvg::Options::default())
        .map_err(|e| format!("{name} {size}: {e}"))?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).ok_or("folder allocation")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap)
}
pub(crate) fn export(root: &Path, check: bool) -> Result<(), String> {
    let dir = root.join("assets/icons/explorer");
    if !check {
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    for name in NAMES {
        for size in SIZES {
            let pixmap = render(name, size)?;
            // Premultiplied RGBA, as the other Windows-style colour frames.
            let rgba: Vec<u8> = pixmap
                .data()
                .chunks_exact(4)
                .flat_map(|p| [p[0].min(p[3]), p[1].min(p[3]), p[2].min(p[3]), p[3]])
                .collect();
            let qoi = qoi::encode_to_vec(&rgba, size, size).map_err(|e| e.to_string())?;
            asset_output(&dir.join(format!("{name}.{size}.qoi")), &qoi, check)?;
        }
    }
    if !check {
        review(root)?;
    }
    Ok(())
}
/// Contact sheets on Explorer's dark and light content colours, plus an
/// image composited between the back and front layers.
fn review(root: &Path) -> Result<(), String> {
    use resvg::tiny_skia::{
        Color, FilterQuality, Paint, Pixmap, PixmapPaint, Rect, Shader, Transform,
    };
    let out = root.join("target/explorer-review");
    fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    for size in REVIEW {
        let zoom = (96 / size).clamp(1, 4);
        let cell = size * zoom + 8;
        let mut sheet =
            Pixmap::new(cell * (NAMES.len() as u32 + 1) + 8, 2 * cell + 8).ok_or("sheet")?;
        for (row, grey) in [(0u32, 25u8), (1, 255)] {
            sheet.fill_rect(
                Rect::from_xywh(
                    0.0,
                    (row * cell) as f32,
                    sheet.width() as f32,
                    cell as f32 + 8.0,
                )
                .ok_or("sheet row")?,
                &Paint {
                    shader: Shader::SolidColor(Color::from_rgba8(grey, grey, grey, 255)),
                    ..Default::default()
                },
                Transform::identity(),
                None,
            );
        }
        let paint = PixmapPaint {
            quality: FilterQuality::Nearest,
            ..Default::default()
        };
        let place = |sheet: &mut Pixmap, pixmap: &Pixmap, column: u32, row: u32| {
            sheet.draw_pixmap(
                0,
                0,
                pixmap.as_ref(),
                &paint,
                Transform::from_scale(zoom as f32, zoom as f32)
                    .post_translate((8 + column * cell) as f32, (8 + row * cell) as f32),
                None,
            );
        };
        for (column, name) in NAMES.iter().enumerate() {
            let pixmap = render(name, size)?;
            for row in 0..2 {
                place(&mut sheet, &pixmap, column as u32, row);
            }
        }
        // Back, a photo in the content box, front: the image-folder recipe.
        let mut photo = render("folder-back", size)?;
        let g = grid(size);
        let (x, y, w, h) = (
            g.left + 3.0 * g.u,
            g.back_top + 1.5 * g.u,
            g.right - g.left - 6.0 * g.u,
            g.live_top - g.back_top + 6.0 * g.u,
        );
        for (inset, color) in [
            (0.0, Color::from_rgba8(255, 255, 255, 255)),
            (g.u.max(1.0), Color::from_rgba8(52, 132, 214, 255)),
        ] {
            photo.fill_rect(
                Rect::from_xywh(x + inset, y + inset, w - 2.0 * inset, h - 2.0 * inset)
                    .ok_or("photo")?,
                &Paint {
                    shader: Shader::SolidColor(color),
                    ..Default::default()
                },
                Transform::identity(),
                None,
            );
        }
        photo.draw_pixmap(
            0,
            0,
            render("folder-front", size)?.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
        for row in 0..2 {
            place(&mut sheet, &photo, NAMES.len() as u32, row);
        }
        sheet
            .save_png(out.join(format!("folder-icons-{size}.png")))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_frame_snaps_its_edges_and_keeps_the_contents_window() {
        for size in SIZES.into_iter().chain(REVIEW) {
            let g = grid(size);
            for v in [
                g.left,
                g.right,
                g.tab_top,
                g.back_top,
                g.front_top,
                g.live_top,
                g.bottom,
            ] {
                assert_eq!(v, v.round(), "{size}");
            }
            assert!(g.tab_top < g.back_top && g.back_top < g.front_top);
            assert!(g.front_top < g.live_top && g.live_top < g.bottom);
            assert!(g.tab_end < g.tab_foot && g.tab_foot < g.right);
            for name in NAMES {
                let pixmap = render(name, size).unwrap();
                assert!(
                    pixmap.pixels().iter().any(|p| p.alpha() == 255),
                    "{name} {size}"
                );
            }
        }
    }
}

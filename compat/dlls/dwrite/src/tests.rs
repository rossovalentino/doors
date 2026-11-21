//! Host tests against the Windows fonts in `assets/fonts`.
extern crate std;
use crate::fallback;
use crate::font::*;
use crate::raster::{self, Command, Glyph, Matrix};
use alloc::{boxed::Box, string::String, sync::Arc, vec::Vec};

fn load(path: &str) -> Arc<File> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../assets/fonts");
    let bytes: &'static [u8] =
        Box::leak(std::fs::read(root.join(path)).unwrap().into_boxed_slice());
    Arc::new(File {
        bytes: Bytes::Static(bytes),
        path: Vec::new(),
        key: Vec::new(),
        loader: 0,
    })
}
fn system() -> Collection {
    let mut all = Vec::new();
    for p in [
        "segoe-ui/segoeui.ttf",
        "segoe-ui/seguisb.ttf",
        "segoe-ui/SegUIVar.ttf",
        "windows-unicode/seguisym.ttf",
        "cascadia-mono/CascadiaMono.ttf",
    ] {
        all.extend(faces(&load(p)));
    }
    let mut c = Collection::from_faces(all);
    for (alias, target) in crate::SUBSTITUTES {
        c.alias(alias, target);
    }
    c
}

#[test]
fn segoe_ui_has_directwrite_metrics() {
    let c = system();
    let family = c.find("segoe ui").expect("Segoe UI");
    let fonts = &c.families[family].fonts;
    assert_eq!(fonts.len(), 2, "regular and semibold");
    let regular = &fonts[matching(fonts, 400, 5, 0)[0].0];
    assert_eq!(regular.weight, 400);
    let m = regular.metrics();
    assert_eq!(m.units_per_em, 2048);
    assert_eq!((m.ascent, m.descent), (2210, 514));
    assert_eq!(m.line_gap, 0);
    assert!(m.cap_height > 1300 && m.cap_height < 1500, "{m:?}");
    assert!(m.x_height > 950 && m.x_height < 1100, "{m:?}");
    assert!(m.underline_position < 0 && m.underline_thickness > 0);
    let h = regular.glyph('H' as u32);
    assert_ne!(h, 0);
    let g = regular.glyph_metrics(h);
    assert!(g.advance_width > 1200 && g.left_side_bearing > 0, "{g:?}");
    assert_eq!(g.vertical_origin_y, 2210);
    assert!(regular.table(*b"cmap").is_some_and(|t| t.len() > 16));
    assert!(regular.table(*b"zzzz").is_none());
    let full: Vec<String> = regular
        .informational(16)
        .into_iter()
        .map(|(_, n)| n)
        .collect();
    assert_eq!(full.first().map(String::as_str), Some("Segoe UI"));
}

#[test]
fn variable_instances_and_matching() {
    let c = system();
    let v = c.find("Segoe UI Variable").expect("variable family");
    let fonts = &c.families[v].fonts;
    assert!(fonts.len() >= 3, "named instances: {}", fonts.len());
    assert!(fonts.iter().all(|f| !f.coords.is_empty()));
    let weights: Vec<u32> = fonts.iter().map(|f| f.weight).collect();
    assert!(
        weights.contains(&400) && weights.contains(&700),
        "{weights:?}"
    );
    // Segoe UI has no bold face: 700 matches semibold, without simulation.
    let s = c.find("Segoe UI").unwrap();
    let ui = &c.families[s].fonts;
    let (i, simulations) = matching(ui, 700, 5, 0)[0];
    assert_eq!((ui[i].weight, simulations), (600, 0));
    // Semibold is bold enough for 900 (no simulation); italic requests
    // simulate oblique; a regular face simulates bold.
    let (i, simulations) = matching(ui, 900, 5, 2)[0];
    assert_eq!((ui[i].weight, simulations), (600, SIMULATION_OBLIQUE));
    let regular_only = [ui[0].clone()];
    assert_eq!(matching(&regular_only, 700, 5, 0)[0].1, SIMULATION_BOLD);
    // Light requests prefer lighter, then the regular face.
    assert_eq!(ui[matching(ui, 300, 5, 0)[0].0].weight, 400);
    // Substitutes resolve.
    assert_eq!(c.find("Consolas"), c.find("Cascadia Mono"));
    assert_eq!(c.find("arial"), Some(s));
}

#[test]
fn glyph_runs_rasterize_inside_their_bounds() {
    let c = system();
    let s = c.find("Segoe UI").unwrap();
    let face = c.families[s].fonts[0].clone();
    let glyphs = [
        Glyph {
            id: face.glyph('H' as u32),
            advance: 10.0,
            ..Glyph::default()
        },
        Glyph {
            id: face.glyph('i' as u32),
            advance: 4.0,
            ..Glyph::default()
        },
    ];
    let outline = raster::run_outline(&face, 16.0, &glyphs, false);
    assert!(outline.iter().any(|c| matches!(c, Command::Close)));
    let device = raster::transform(&outline, &Matrix::IDENTITY.scale(1.5), (0.0, 0.0));
    let b = raster::bounds(&device);
    // Above the baseline (y down), about 1.5 * 16 * 0.7 pixels tall.
    assert!(
        b[1] < -14 && b[1] > -20 && b[3] <= 1 && b[0] >= 0 && b[2] > 15,
        "{b:?}"
    );
    let alpha = raster::coverage(&device, b, false);
    let width = (b[2] - b[0]) as usize;
    assert_eq!(alpha.len(), width * (b[3] - b[1]) as usize);
    let full = alpha.iter().filter(|a| **a == 255).count();
    assert!(full > 40, "solid stems: {full}");
    // The middle row of H crosses its crossbar.
    let aliased = raster::coverage(&device, b, true);
    assert!(aliased.iter().all(|a| *a == 0 || *a == 255));
    // Spaces have no ink.
    let space = raster::run_outline(
        &face,
        16.0,
        &[Glyph {
            id: face.glyph(' ' as u32),
            advance: 4.0,
            ..Glyph::default()
        }],
        false,
    );
    assert_eq!(raster::bounds(&space), [0; 4]);
}

#[test]
fn fallback_maps_runs_to_covering_fonts() {
    let c = system();
    let text: Vec<u16> = "Hi \u{2603}\u{2603}x".encode_utf16().collect();
    let (n, font) = fallback::map(&c, &text, Some("Segoe UI"), 400, 0, 5, "en-us");
    assert_eq!(n, 3);
    assert_eq!(font.unwrap().0.family_name(), "Segoe UI");
    let (n, font) = fallback::map(&c, &text[3..], Some("Segoe UI"), 400, 0, 5, "en-us");
    assert_eq!(n, 2);
    assert_eq!(font.unwrap().0.family_name(), "Segoe UI Symbol");
    // A family the collection lacks still falls back.
    let (n, font) = fallback::map(&c, &text, Some("No Such Font"), 400, 0, 5, "en-us");
    assert_eq!(n, 3);
    assert_eq!(font.unwrap().0.family_name(), "Segoe UI");
    // Nothing covers a private-use character outside these fonts.
    let pua: Vec<u16> = "\u{10FFFD}".encode_utf16().collect();
    let (n, font) = fallback::map(&c, &pua, Some("Segoe UI"), 400, 0, 5, "en-us");
    assert_eq!((n, font.is_none()), (2, true));
}

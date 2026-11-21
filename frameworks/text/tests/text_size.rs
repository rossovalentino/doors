//! Accessibility > Text size (P12-TEXT-SIZE): the atlas caches glyphs at
//! display x text while positions keep the display scale.
mod support;
use door_graphics_text::{
    Clip, Family, FontAtlas, Target, TextRun, TextStyle, TextSystem, text_grow, text_len,
};
use std::sync::Arc;

const STYLES: [TextStyle; 4] = [
    TextStyle::Regular12,
    TextStyle::Regular14,
    TextStyle::Semibold20,
    TextStyle::Semibold28,
];

fn atlas_bytes(name: &str) -> Vec<u8> {
    std::fs::read(support::root().join("assets/fonts/segoe-ui").join(name)).unwrap()
}

#[test]
fn text_size_scales_logical_metrics_exactly_at_every_display_scale() {
    for display in [100, 125, 150, 175, 200] {
        let base = FontAtlas::windows_reference_text(display, 100).unwrap();
        for text in [100, 125, 150, 175, 200, 225] {
            let atlas = FontAtlas::windows_reference_text(display, text).unwrap();
            assert_eq!((atlas.percent(), atlas.text_percent()), (display, text));
            for style in STYLES {
                let m = atlas.metrics(style);
                assert_eq!(m.pixel_size, text_len(style.pixel_size(), text));
                assert_eq!(m.line_height, text_len(style.line_height(), text));
                assert_eq!(atlas.line_height(style), m.line_height);
                // The baseline and ink grow with the text and stay in the box.
                let b = base.metrics(style);
                let ink = atlas.ink_metrics(style);
                assert!(
                    m.baseline + 2 >= b.baseline * text / 100,
                    "{display} {text} {style:?}"
                );
                assert!(ink.top >= 0 && ink.top as u32 + ink.height <= m.line_height + 1);
                let ink0 = base.ink_metrics(style).height;
                assert!(
                    // Hinting snaps small ink to whole pixels: 15% slack.
                    (ink.height + 2) * 100 >= ink0 * text * 85 / 100,
                    "{display} {text} {style:?}: ink {} vs {ink0}",
                    ink.height
                );
                let w = atlas.measure("Downloads and Documents", style).unwrap();
                let w0 = base.measure("Downloads and Documents", style).unwrap();
                let expect = w0.advance_width * text / 100;
                assert!(
                    w.advance_width.abs_diff(expect) <= 2 + expect / 25,
                    "{display}% x {text}% {style:?}: {} vs {expect}",
                    w.advance_width
                );
                assert_eq!(w.height, m.line_height);
                let two = atlas.measure("a\nb", style).unwrap();
                assert_eq!(two.height, 2 * m.line_height);
            }
        }
    }
}

#[test]
fn text_size_glyphs_match_the_effective_density_and_positions_keep_display_scale() {
    // 100% display x 200% text paints exactly the 2x glyphs.
    let double = FontAtlas::windows_reference_scaled(2).unwrap();
    let text = FontAtlas::windows_reference_text(100, 200).unwrap();
    let paint = |atlas: &FontAtlas, physical: bool, x: i32| {
        let mut pixels = vec![255u8; 400 * 80 * 4];
        let mut target = Target::new(&mut pixels, 400, 80, 1600).unwrap();
        let run = TextRun {
            text: "Text size 200%",
            x,
            top: 4,
            style: TextStyle::Regular14,
            color: [0, 0, 0, 255],
        };
        let clip = Clip::new(0, 0, 400, 80);
        if physical {
            atlas.draw_physical(&mut target, clip, run).unwrap();
        } else {
            atlas.draw(&mut target, clip, run).unwrap();
        }
        pixels
    };
    assert_eq!(paint(&double, true, 8), paint(&text, true, 8));
    // A logical origin maps with the display scale only (x=10 -> 15 at 150%).
    let fractional = FontAtlas::windows_reference_text(150, 200).unwrap();
    let pixels = paint(&fractional, false, 10);
    let first_ink = (0..400)
        .find(|&x| (0..80).any(|y| pixels[(y * 400 + x) * 4] != 255))
        .unwrap();
    assert!((15..=19).contains(&first_ink), "{first_ink}");
    // Line boxes advance by the grown line height.
    assert_eq!(
        fractional
            .measure("one\ntwo", TextStyle::Regular14)
            .unwrap()
            .height,
        2 * 40
    );
}

#[test]
fn packed_atlases_serve_only_their_exact_effective_density() {
    let one = atlas_bytes("hinted-1x.dfa");
    let p150 = atlas_bytes("hinted-150.dfa");
    let three = atlas_bytes("hinted-3x.dfa");
    let atlas = FontAtlas::from_packed_text(&p150, 100, 150).unwrap();
    assert_eq!((atlas.percent(), atlas.text_percent()), (100, 150));
    assert_eq!(atlas.metrics(TextStyle::Regular14).line_height, 30);
    assert!(FontAtlas::from_packed_text(&three, 150, 200).is_ok());
    assert!(FontAtlas::from_packed_text(&one, 100, 150).is_err());
    assert!(FontAtlas::from_packed_text(&p150, 100, 226).is_err());
    assert_eq!(FontAtlas::hinted_percent(100, 175), Some(175));
    assert_eq!(FontAtlas::hinted_percent(125, 120), Some(150));
    assert_eq!(FontAtlas::hinted_percent(150, 150), None); // 225%: outlines
    assert_eq!(FontAtlas::hinted_percent(125, 101), None); // 126.25%
    assert_eq!(FontAtlas::raster_percent(150, 225), Some(33_750));
    assert_eq!(FontAtlas::raster_percent(100, 99), None);
    // The hinted 150% atlas and the outline fallback agree on metrics.
    let outline = FontAtlas::windows_reference_text(125, 120).unwrap();
    let hinted = FontAtlas::from_packed_text(&p150, 125, 120).unwrap();
    for style in STYLES {
        assert_eq!(
            outline.metrics(style).line_height,
            hinted.metrics(style).line_height
        );
        assert_eq!(
            outline.metrics(style).pixel_size,
            hinted.metrics(style).pixel_size
        );
    }
    // Fractional products rasterize from outlines (337.5% here).
    let big = FontAtlas::windows_reference_text(150, 225).unwrap();
    assert_eq!(big.metrics(TextStyle::Semibold28).line_height, 86);
    assert!(
        big.measure("W", TextStyle::Regular14)
            .unwrap()
            .advance_width
            >= 25
    );
}

#[test]
fn unicode_text_uses_the_same_grown_line_box() {
    let system = Arc::new(TextSystem::new(support::fonts()));
    let base = FontAtlas::windows_reference_text(100, 100)
        .unwrap()
        .with_unicode(system.clone(), Family::Ui);
    let grown = FontAtlas::windows_reference_text(100, 200)
        .unwrap()
        .with_unicode(system, Family::Ui);
    let style = TextStyle::Regular14;
    let a = base.measure("Grüße 文件", style).unwrap();
    let b = grown.measure("Grüße 文件", style).unwrap();
    assert_eq!((a.height, b.height), (20, 40));
    assert!(
        b.advance_width.abs_diff(2 * a.advance_width) <= 3,
        "{a:?} {b:?}"
    );
}

#[test]
fn layout_growth_keeps_padding_and_never_shrinks() {
    // A 32 px row holding 20 px body text keeps its 12 px padding.
    assert_eq!(text_grow(32, TextStyle::Regular14, 100), 32);
    assert_eq!(text_grow(32, TextStyle::Regular14, 150), 42);
    assert_eq!(text_grow(32, TextStyle::Regular14, 225), 57);
    let atlas = FontAtlas::windows_reference_text(150, 225).unwrap();
    assert_eq!(atlas.text_grow(40, TextStyle::Regular14), 65);
    for text in 100..=225 {
        assert!(text_grow(30, TextStyle::Regular12, text) >= 30);
        assert!(text_len(20, text) >= 20);
    }
}

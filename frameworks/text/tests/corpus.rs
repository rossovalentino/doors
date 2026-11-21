//! Multilingual corpus: shaping, fallback, cluster maps and golden renders.
mod support;
use door_graphics_text::*;

struct Sample {
    id: String,
    locale: Locale,
    direction: BaseDirection,
    text: String,
}

fn unescape(raw: &str) -> String {
    let mut out = String::new();
    let mut rest = raw;
    while let Some(at) = rest.find('\\') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        if let Some(body) = rest.strip_prefix("\\u{") {
            let end = body.find('}').expect("closed escape");
            out.push(char::from_u32(u32::from_str_radix(&body[..end], 16).unwrap()).unwrap());
            rest = &body[end + 1..];
        } else if let Some(body) = rest.strip_prefix("\\n") {
            out.push('\n');
            rest = body;
        } else {
            out.push('\\');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

fn corpus() -> Vec<Sample> {
    let text =
        std::fs::read_to_string(support::root().join("tests/fixtures/text/corpus.txt")).unwrap();
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|line| {
            let mut parts = line.splitn(4, '|');
            let id = parts.next().unwrap().to_string();
            let locale = Locale::new(parts.next().unwrap());
            let direction = match parts.next().unwrap() {
                "ltr" => BaseDirection::Ltr,
                "rtl" => BaseDirection::Rtl,
                _ => BaseDirection::Auto,
            };
            Sample {
                id,
                locale,
                direction,
                text: unescape(parts.next().unwrap()),
            }
        })
        .collect()
}

fn sample(id: &str) -> Sample {
    corpus().into_iter().find(|s| s.id == id).unwrap()
}

fn layout(s: &Sample, scale: f32) -> TextLayout {
    support::system()
        .layout(
            &s.text,
            &TextFormat::new(14.0)
                .with_scale(scale)
                .with_locale(s.locale)
                .with_line_height(20.0),
            &LayoutOptions {
                direction: s.direction,
                wrap: Wrap::None,
                ..LayoutOptions::default()
            },
        )
        .unwrap()
}

fn role_of(face: usize) -> FontRole {
    support::system().fonts().face_info(face).unwrap().role
}

#[test]
fn every_sample_shapes_without_missing_glyphs_and_maps_every_grapheme() {
    for s in corpus() {
        let layout = layout(&s, 1.0);
        let glyphs = layout.glyphs();
        assert!(!glyphs.is_empty(), "{}", s.id);
        assert!(
            glyphs.iter().all(|g| g.glyph != 0),
            "{}: .notdef in {:?}",
            s.id,
            glyphs
        );
        let clusters = layout.clusters();
        let boundaries = grapheme_boundaries(&s.text);
        assert_eq!(clusters.len(), boundaries.len() - 1, "{}", s.id);
        for (cluster, pair) in clusters.iter().zip(boundaries.windows(2)) {
            assert_eq!(cluster.text, pair[0]..pair[1], "{}", s.id);
            assert!(cluster.width >= 0.0 && cluster.glyphs.end <= glyphs.len());
        }
        // Visual clusters tile the line without overlap.
        let mut spans: Vec<(f32, f32)> = clusters.iter().map(|c| (c.x, c.x + c.width)).collect();
        spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        for pair in spans.windows(2) {
            assert!(
                pair[1].0 + 0.01 >= pair[0].1,
                "{}: overlapping clusters {pair:?}",
                s.id
            );
        }
        let metrics = layout.metrics();
        assert!(metrics.width > 0.0 && metrics.lines == 1 && !metrics.truncated);
    }
}

#[test]
fn fallback_selects_windows_faces_by_script_and_locale() {
    let check = |id: &str, ch: char, role: FontRole| {
        let s = sample(id);
        let layout = layout(&s, 1.0);
        let at = s.text.find(ch).unwrap();
        let glyph = layout
            .glyphs()
            .into_iter()
            .find(|g| g.cluster == at)
            .unwrap();
        assert_eq!(role_of(glyph.face), role, "{id} {ch}");
    };
    check("latin", 'è', FontRole::UiVariable);
    check("greek", 'ἀ', FontRole::UiVariable);
    check("cyrillic", 'ё', FontRole::UiVariable);
    check("arabic", 'م', FontRole::Ui);
    check("hebrew", 'ש', FontRole::Ui);
    check("hindi", 'न', FontRole::Indic);
    check("chinese", '你', FontRole::ChineseSimplified);
    check("chinese", '，', FontRole::ChineseSimplified);
    check("chinese-traditional", '體', FontRole::ChineseTraditional);
    check("japanese", 'こ', FontRole::Japanese);
    check("japanese", '漢', FontRole::Japanese);
    check("korean", '안', FontRole::Korean);
    check("emoji", '\u{1F600}', FontRole::Emoji);
    check("mixed", '你', FontRole::ChineseSimplified);
}

#[test]
fn latin_kerning_and_cjk_em_advances() {
    let system = support::system();
    let format = TextFormat::new(14.0);
    let w = |t: &str| system.layout_line(t, &format).unwrap().metrics().width;
    assert!(w("AV") < w("A") + w("V") - 0.5, "GPOS kerning");
    assert!(w("To") < w("T") + w("o") - 0.2);
    let s = sample("chinese");
    let layout = layout(&s, 1.0);
    for c in layout.clusters().iter().filter(|c| {
        s.text[c.text.clone()]
            .chars()
            .all(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch))
    }) {
        assert!((c.width - 14.0).abs() < 0.05, "Han advance {}", c.width);
    }
}

#[test]
fn arabic_joins_and_runs_right_to_left() {
    let system = support::system();
    let format = TextFormat::new(14.0).with_locale(Locale::new("ar"));
    let isolated = system.layout_line("ب", &format).unwrap().glyphs()[0].glyph;
    let joined = system.layout_line("ببب", &format).unwrap();
    let ids: Vec<u32> = joined.glyphs().iter().map(|g| g.glyph).collect();
    assert_eq!(ids.len(), 3);
    assert!(
        ids.iter().all(|&id| id != isolated),
        "contextual forms {ids:?} vs {isolated}"
    );
    assert!(
        ids[0] != ids[1] && ids[1] != ids[2],
        "final, medial, initial forms"
    );
    let s = sample("arabic");
    let layout = layout(&s, 1.0);
    assert!(layout.lines()[0].rtl);
    let clusters = layout.clusters();
    assert!(
        clusters[0].rtl && clusters[0].x > clusters[1].x,
        "first logical letter is rightmost"
    );
    // Arabic-Indic digits keep left-to-right order inside the RTL paragraph.
    let one = clusters
        .iter()
        .find(|c| &s.text[c.text.clone()] == "١")
        .unwrap();
    let three = clusters
        .iter()
        .find(|c| &s.text[c.text.clone()] == "٣")
        .unwrap();
    assert!(one.x < three.x);
}

#[test]
fn hebrew_bidi_with_numbers_reorders_visually() {
    let s = sample("hebrew");
    let layout = layout(&s, 1.0);
    let clusters = layout.clusters();
    let x = |needle: &str| {
        clusters
            .iter()
            .find(|c| &s.text[c.text.clone()] == needle)
            .unwrap()
            .x
    };
    assert!(layout.lines()[0].rtl);
    assert!(
        x("ש") > x("ל") && x("ל") > x("ו"),
        "Hebrew letters right to left"
    );
    assert!(
        x("1") < x("2") && x("2") < x("3"),
        "European numbers left to right"
    );
    assert!(x("4") < x("5") && x("5") < x("6"));
    assert!(
        x("3") < x("ש"),
        "the number follows the first word to its left"
    );
    assert!(x("6") < x("1"), "later logical text continues leftwards");
    // Mirrored brackets: the logical '(' renders as the glyph of ')' and vice versa.
    let glyphs = layout.glyphs();
    let open = glyphs
        .iter()
        .find(|g| s.text[g.cluster..].starts_with('('))
        .unwrap()
        .glyph;
    let close = glyphs
        .iter()
        .find(|g| s.text[g.cluster..].starts_with(')'))
        .unwrap()
        .glyph;
    let system = support::system();
    let plain = system
        .layout_line("()", &TextFormat::new(14.0))
        .unwrap()
        .glyphs();
    assert_eq!((open, close), (plain[1].glyph, plain[0].glyph));
}

#[test]
fn devanagari_conjuncts_and_reordered_matra() {
    let system = support::system();
    let format = TextFormat::new(14.0).with_locale(Locale::new("hi"));
    let conjunct = system.layout_line("क्ष", &format).unwrap();
    assert_eq!(
        conjunct.clusters().len(),
        1,
        "virama conjunct is one grapheme"
    );
    assert!(conjunct.glyphs().len() < 3, "conjunct ligature");
    let ki = system.layout_line("कि", &format).unwrap();
    let ka = system.layout_line("क", &format).unwrap().glyphs()[0].glyph;
    let glyphs = ki.glyphs();
    assert_eq!(glyphs.len(), 2);
    assert!(
        glyphs[0].glyph != ka && glyphs[1].glyph == ka,
        "i-matra reorders before the consonant"
    );
}

#[test]
fn emoji_sequences_are_single_colour_clusters() {
    let system = support::system();
    let format = TextFormat::new(14.0);
    for sequence in [
        "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}",
        "\u{1F44D}\u{1F3FD}",
        "\u{1F44D}\u{1F3FF}",
        "1\u{FE0F}\u{20E3}",
        "\u{2764}\u{FE0F}",
    ] {
        let layout = system.layout_line(sequence, &format).unwrap();
        assert_eq!(layout.clusters().len(), 1, "{sequence:?}");
        let glyphs = layout.glyphs();
        assert!(
            glyphs.iter().all(|g| role_of(g.face) == FontRole::Emoji),
            "{sequence:?}"
        );
        let single = system
            .layout_line("\u{1F600}", &format)
            .unwrap()
            .metrics()
            .width;
        assert!(
            (layout.metrics().width - single).abs() <= single * 0.15,
            "{sequence:?} occupies one emoji cell"
        );
        assert_eq!(role_of(glyphs[0].face), FontRole::Emoji);
        let mut pixels = vec![255u8; 40 * 30 * 4];
        let mut target = Target::new(&mut pixels, 40, 30, 160).unwrap();
        let clip = target.bounds();
        let stats = system
            .draw(&layout, &mut target, clip, 4, 4, [0, 0, 0, 255])
            .unwrap();
        assert!(stats.painted_pixels > 50);
        assert!(
            pixels.chunks_exact(4).any(|p| p[0].abs_diff(p[2]) > 8),
            "{sequence:?} has colour"
        );
    }
    // Skin tones select different glyphs.
    let a = system
        .layout_line("\u{1F44D}\u{1F3FD}", &format)
        .unwrap()
        .glyphs()[0]
        .glyph;
    let b = system
        .layout_line("\u{1F44D}\u{1F3FF}", &format)
        .unwrap()
        .glyphs()[0]
        .glyph;
    assert_ne!(a, b);
}

#[test]
fn combining_marks_stay_in_their_grapheme_and_are_positioned() {
    let system = support::system();
    let format = TextFormat::new(14.0);
    for text in ["e\u{301}", "a\u{308}\u{323}", "Z\u{335}", "o\u{302}\u{303}"] {
        let layout = system.layout_line(text, &format).unwrap();
        assert_eq!(layout.clusters().len(), 1, "{text:?}");
        let glyphs = layout.glyphs();
        assert!(glyphs.iter().all(|g| g.glyph != 0));
        if glyphs.len() > 1 {
            assert!(glyphs[1..].iter().all(|g| g.x > glyphs[0].x - 0.01));
        }
        assert_eq!(next_grapheme(text, 0), text.len());
    }
}

#[test]
fn hit_testing_carets_and_selection_handle_bidi() {
    let system = support::system();
    let format = TextFormat::new(14.0);
    let ltr = system.layout_line("Door OS e\u{301}!", &format).unwrap();
    let text = "Door OS e\u{301}!";
    let width = ltr.metrics().width;
    assert_eq!(ltr.hit_test(-5.0, 5.0).offset, 0);
    let end = ltr.hit_test(width + 20.0, 5.0);
    assert_eq!((end.offset, end.affinity), (text.len(), Affinity::Upstream));
    let mut previous = -1.0;
    for offset in grapheme_boundaries(text) {
        let caret = ltr.caret(offset, Affinity::Downstream);
        assert!(caret.x > previous, "monotonic carets");
        previous = caret.x;
        let hit = ltr.hit_test(caret.x + 0.3, 5.0);
        assert_eq!(hit.offset, offset, "round trip at {offset}");
    }
    let e_acute = ltr
        .clusters()
        .into_iter()
        .find(|c| c.text == (8..11))
        .unwrap();
    let inside = ltr.hit_test(e_acute.x + e_acute.width * 0.4, 5.0).offset;
    assert!(inside == 8 || inside == 11, "never inside é: {inside}");

    let mixed = "abc \u{5D0}\u{5D1}\u{5D2} def";
    let bidi = system.layout_line(mixed, &format).unwrap();
    let boundary = 4; // between "abc " and the Hebrew run
    let up = bidi.caret(boundary, Affinity::Upstream);
    let down = bidi.caret(boundary, Affinity::Downstream);
    assert!(
        down.x > up.x + 5.0,
        "affinity selects the visually distinct sides: {up:?} {down:?}"
    );
    assert!(down.rtl && !up.rtl);
    let rects = bidi.selection_rects(2..6); // "c", " ", and the first Hebrew letter
    assert_eq!(rects.len(), 2, "{rects:?}");
    // Hit testing the right half of an RTL letter gives its logical start.
    let alef = bidi
        .clusters()
        .into_iter()
        .find(|c| c.text.start == 4)
        .unwrap();
    let hit = bidi.hit_test(alef.x + alef.width * 0.8, 5.0);
    assert_eq!((hit.offset, hit.affinity), (4, Affinity::Downstream));
    let hit = bidi.hit_test(alef.x + alef.width * 0.2, 5.0);
    assert_eq!((hit.offset, hit.affinity), (6, Affinity::Upstream));
    // Visual right-arrow movement walks every caret stop of the line once.
    let (mut offset, mut affinity) = (0usize, Affinity::Downstream);
    let mut seen = vec![];
    for _ in 0..32 {
        let next = bidi.move_visual(offset, affinity, true);
        if next == (offset, affinity) {
            break;
        }
        assert!(bidi.caret(next.0, next.1).x > bidi.caret(offset, affinity).x);
        (offset, affinity) = next;
        seen.push(offset);
    }
    assert!(seen.len() >= mixed.chars().count() - 1, "{seen:?}");
}

#[test]
fn utf16_offsets_never_split_surrogates() {
    let text = "a\u{1F600}b";
    assert_eq!(utf16_len(text), 4);
    assert_eq!(utf8_to_utf16(text, 5), 3);
    assert_eq!(utf16_to_utf8(text, 2), 1, "inside the pair snaps back");
    assert_eq!(utf16_to_utf8(text, 3), 5);
    assert_eq!(next_word("Hello, world", 0), 7);
    assert_eq!(prev_word("Hello, world", 12), 7);
    assert_eq!(word_at("Hello, world", 9), 7..12);
}

#[test]
fn wrapping_follows_uax14_and_bounds_every_line() {
    let system = support::system();
    let format = TextFormat::new(14.0).with_line_height(20.0);
    let options = |width: f32| LayoutOptions {
        max_width: Some(width),
        ..LayoutOptions::default()
    };
    let text = "The quick brown fox jumps over the lazy dog";
    let layout = system.layout(text, &format, &options(80.0)).unwrap();
    assert!(layout.metrics().lines >= 4);
    for line in layout.lines() {
        assert!(line.width <= 80.01, "{line:?}");
        let slice = &text[line.text.clone()];
        assert!(!slice.starts_with(' '), "breaks after spaces: {slice:?}");
    }
    assert_eq!(
        layout.metrics().height,
        layout.metrics().lines as f32 * 20.0
    );
    // CJK breaks between ideographs; a long word breaks at graphemes.
    let cjk = system
        .layout(
            "你好世界简体中文测试你好世界",
            &format.with_locale(Locale::new("zh-Hans")),
            &options(60.0),
        )
        .unwrap();
    assert!(cjk.metrics().lines >= 3 && cjk.lines().iter().all(|l| l.width <= 60.01));
    let long = system
        .layout(
            "Supercalifragilisticexpialidocious",
            &format,
            &options(50.0),
        )
        .unwrap();
    assert!(long.metrics().lines >= 3 && long.lines().iter().all(|l| l.width <= 50.01));
    // Ellipsis and line limits.
    let cut = system
        .layout(
            text,
            &format,
            &LayoutOptions {
                max_width: Some(90.0),
                max_lines: Some(1),
                ellipsis: true,
                ..LayoutOptions::default()
            },
        )
        .unwrap();
    assert!(cut.metrics().truncated && cut.metrics().lines == 1 && cut.metrics().width <= 90.01);
    let last = *cut.glyphs().last().unwrap();
    let ellipsis = system.layout_line("\u{2026}", &format).unwrap().glyphs()[0].glyph;
    assert_eq!(last.glyph, ellipsis);
    // Explicit breaks and RTL alignment.
    let paragraphs = system
        .layout("one\r\ntwo\nthree", &format, &LayoutOptions::default())
        .unwrap();
    assert_eq!(paragraphs.metrics().lines, 3);
    let rtl = system
        .layout(
            "שלום",
            &format,
            &LayoutOptions {
                max_width: Some(200.0),
                ..LayoutOptions::default()
            },
        )
        .unwrap();
    let line = &rtl.lines()[0];
    assert!(
        (line.x + line.width - 200.0).abs() < 0.01,
        "Start aligns RTL right: {line:?}"
    );
}

#[test]
fn glyph_cache_is_bounded_and_eviction_preserves_pixels() {
    let fonts = support::fonts();
    let small = TextSystem::with_limits(
        fonts,
        CacheLimits {
            glyph_bytes: 24 * 1024,
            shape_bytes: 8 * 1024,
        },
    );
    let text = "简体中文测试繁體中文測試こんにちは世界안녕하세요세계ΑΒΓΔΕΖΗΘ";
    let format = TextFormat::new(20.0)
        .with_scale(1.5)
        .with_locale(Locale::new("zh-Hans"));
    let draw = |system: &TextSystem| {
        let layout = system.layout_line(text, &format).unwrap();
        let mut pixels = vec![255u8; 1400 * 60 * 4];
        let mut target = Target::new(&mut pixels, 1400, 60, 1400 * 4).unwrap();
        let clip = target.bounds();
        system
            .draw(&layout, &mut target, clip, 2, 2, [16, 16, 16, 255])
            .unwrap();
        pixels
    };
    let reference = draw(support::system());
    for _ in 0..3 {
        assert!(
            draw(&small) == reference,
            "evicted glyphs re-rasterize identically"
        );
    }
    let stats = small.cache_stats();
    assert!(stats.glyph_bytes <= 24 * 1024, "{stats:?}");
    assert!(
        stats.glyph_evictions > 0 && stats.shaped_bytes <= 8 * 1024,
        "{stats:?}"
    );
    small.clear_caches();
    assert_eq!(small.cache_stats().glyphs, 0);
}

#[test]
fn malformed_and_missing_fonts_are_rejected_or_fall_back() {
    assert!(FontSet::from_files(&[]).is_err());
    let real = support::bytes()[0].0;
    for bad in [
        &real[..12],
        &real[..1000],
        b"not a font at all, definitely not".as_slice(),
    ] {
        assert_eq!(
            FontSet::from_files(&[(bad, FontRole::Ui)]).err(),
            Some(Error::InvalidFont)
        );
    }
    // Only Segoe UI Variable: everything still lays out, unsupported scripts use .notdef.
    let latin = TextSystem::new(FontSet::from_files(&[support::bytes()[0]]).unwrap());
    let layout = latin
        .layout_line("Door 你好", &TextFormat::new(14.0))
        .unwrap();
    assert!(layout.glyphs().iter().any(|g| g.glyph == 0));
    assert!(latin.layout_line("x", &TextFormat::new(f32::NAN)).is_err());
    assert!(
        latin
            .layout_line("x", &TextFormat::new(14.0).with_scale(0.0))
            .is_err()
    );
    // A layout from another system is refused before painting.
    let mut pixels = vec![255u8; 64 * 32 * 4];
    let mut target = Target::new(&mut pixels, 64, 32, 256).unwrap();
    let clip = target.bounds();
    assert_eq!(
        support::system().draw(&layout, &mut target, clip, 0, 0, [0, 0, 0, 255]),
        Err(Error::InvalidFont)
    );
    assert_eq!(
        latin.draw(&layout, &mut target, clip, 0, 0, [255, 0, 0, 0]),
        Err(Error::InvalidColor)
    );
    assert!(pixels.iter().all(|&b| b == 255));
}

struct Provider(std::sync::atomic::AtomicUsize);
impl FontProvider for Provider {
    fn load(&self, role: FontRole) -> Vec<&'static [u8]> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        support::bytes()
            .iter()
            .filter(|(_, r)| *r == role)
            .map(|(b, _)| *b)
            .collect()
    }
}

#[test]
fn provider_loads_cjk_and_emoji_roles_on_first_use_only() {
    let base: Vec<_> = support::bytes()
        .iter()
        .copied()
        .filter(|(_, r)| matches!(r, FontRole::UiVariable | FontRole::Ui))
        .collect();
    let provider: &'static Provider = Box::leak(Box::new(Provider(Default::default())));
    struct Shared(&'static Provider);
    impl FontProvider for Shared {
        fn load(&self, role: FontRole) -> Vec<&'static [u8]> {
            self.0.load(role)
        }
    }
    let system = TextSystem::new(
        FontSet::from_files(&base)
            .unwrap()
            .with_provider(Box::new(Shared(provider))),
    );
    let format = TextFormat::new(14.0).with_locale(Locale::new("zh-Hans"));
    let calls = || provider.0.load(std::sync::atomic::Ordering::SeqCst);
    system.layout_line("Plain Latin text", &format).unwrap();
    assert_eq!(calls(), 0, "no lazy load for covered text");
    let first = system.layout_line("你好", &format).unwrap();
    assert!(first.glyphs().iter().all(|g| g.glyph != 0));
    let after = calls();
    assert!(after >= 1 && system.fonts().len() > base.len());
    system.layout_line("世界 \u{1F600}", &format).unwrap();
    system.layout_line("世界 \u{1F600}", &format).unwrap();
    assert_eq!(
        calls(),
        after + 1,
        "emoji loaded once, CJK not requested again"
    );
}

#[test]
fn atlas_shim_routes_unicode_and_keeps_ascii_pixels() {
    use std::sync::Arc;
    let plain = FontAtlas::windows_reference_scaled(1).unwrap();
    let system = Arc::new(TextSystem::new(support::fonts()));
    let attached = FontAtlas::windows_reference_scaled(1)
        .unwrap()
        .with_unicode(system, Family::Ui);
    let style = TextStyle::Regular14;
    let draw = |atlas: &FontAtlas, text: &str| {
        let mut pixels = vec![255u8; 240 * 30 * 4];
        let mut target = Target::new(&mut pixels, 240, 30, 960).unwrap();
        let clip = target.bounds();
        let m = atlas
            .draw(
                &mut target,
                clip,
                TextRun {
                    text,
                    x: 2,
                    top: 2,
                    style,
                    color: [0, 0, 0, 255],
                },
            )
            .unwrap();
        (pixels, m)
    };
    assert_eq!(
        draw(&plain, "File Explorer"),
        draw(&attached, "File Explorer")
    );
    let (pixels, metrics) = draw(&attached, "文件 Explorer \u{1F4C1}");
    assert!(metrics.painted_pixels > 200 && pixels.iter().any(|&b| b != 255));
    assert_eq!(attached.measure("文件", style).unwrap().advance_width, 28);
    assert_ne!(
        plain.measure("文件", style),
        attached.measure("文件", style)
    );
    let ink = attached.label_ink("文件", style).unwrap();
    assert!(
        ink.right > ink.left && ink.left >= -2 && ink.right <= 30,
        "{ink:?}"
    );
}

#[test]
fn runtime_raster_matches_the_packed_hinted_atlases() {
    let system = support::system();
    for factor in 1..=3u32 {
        let atlas = FontAtlas::windows_reference_scaled(factor).unwrap();
        for style in TEXT_STYLES {
            let format = TextFormat::from_style(style, factor as f32);
            for byte in 33u8..=126 {
                let s = (byte as char).to_string();
                let size = 96 * factor;
                let mut a = vec![255u8; (size * size * 4) as usize];
                let mut b = a.clone();
                {
                    let mut t = Target::new(&mut a, size, size, size as usize * 4).unwrap();
                    let clip = t.bounds();
                    atlas
                        .draw_physical(
                            &mut t,
                            clip,
                            TextRun {
                                text: &s,
                                x: 8,
                                top: 8,
                                style,
                                color: [0, 0, 0, 255],
                            },
                        )
                        .unwrap();
                }
                {
                    let mut t = Target::new(&mut b, size, size, size as usize * 4).unwrap();
                    let clip = t.bounds();
                    let layout = system.layout_line(&s, &format).unwrap();
                    system
                        .draw(&layout, &mut t, clip, 8, 8, [0, 0, 0, 255])
                        .unwrap();
                }
                assert!(a == b, "{factor}x {style:?} {s}");
            }
        }
    }
}

#[test]
fn grayscale_mode_equals_lcd_on_a_grayscale_target() {
    let system = support::system();
    let text = "Search 搜索 بحث";
    let render = |raster: RasterMode, gray_target: bool| {
        let mut pixels = vec![255u8; 200 * 30 * 4];
        let target = Target::new(&mut pixels, 200, 30, 800).unwrap();
        let mut target = if gray_target {
            target.grayscale()
        } else {
            target.cleartype()
        };
        let clip = target.bounds();
        let layout = system
            .layout_line(text, &TextFormat::new(14.0).with_raster(raster))
            .unwrap();
        system
            .draw(&layout, &mut target, clip, 2, 2, [26, 26, 26, 255])
            .unwrap();
        pixels
    };
    let gray = render(RasterMode::Grayscale, false);
    assert!(gray == render(RasterMode::Lcd, true));
    assert!(gray.chunks_exact(4).all(|p| p[0] == p[1] && p[1] == p[2]));
    assert!(
        render(RasterMode::Lcd, false)
            .chunks_exact(4)
            .any(|p| p[0].abs_diff(p[2]) > 40)
    );
}

#[test]
fn shared_types_are_thread_safe() {
    fn check<T: Send + Sync>() {}
    check::<TextSystem>();
    check::<FontAtlas>();
    check::<TextLayout>();
}

/// Golden renders of the whole corpus at 100% and 150%. Set DOOR_TEXT_BLESS=1
/// to rewrite them. Tolerance absorbs float differences between libm and std
/// math in other feature-unified builds.
#[test]
fn golden_corpus_renders_at_1x_and_1_5x() {
    let system = support::system();
    let samples = corpus();
    for (scale, name) in [(1.0f32, "corpus-100"), (1.5, "corpus-150")] {
        let (w, h) = (
            (640.0 * scale) as u32,
            ((samples.len() as f32 * 24.0 + 8.0) * scale) as u32,
        );
        let mut pixels = vec![255u8; (w * h * 4) as usize];
        {
            // The goldens are ClearType (RGB-stripe) renders.
            let mut target = Target::new(&mut pixels, w, h, w as usize * 4)
                .unwrap()
                .cleartype();
            let clip = target.bounds();
            for (i, s) in samples.iter().enumerate() {
                let layout = layout(s, scale);
                let top = ((4.0 + i as f32 * 24.0) * scale).round() as i32;
                system
                    .draw(
                        &layout,
                        &mut target,
                        clip,
                        (6.0 * scale) as i32,
                        top,
                        [24, 24, 24, 255],
                    )
                    .unwrap();
            }
        }
        let path = support::root().join(format!("tests/fixtures/text/golden/{name}.qoi"));
        if std::env::var_os("DOOR_TEXT_BLESS").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, qoi::encode_to_vec(&pixels, w, h).unwrap()).unwrap();
            support::write_png(
                &std::env::temp_dir().join(format!("door-text-{name}.png")),
                &pixels,
                w,
                h,
            );
            continue;
        }
        let golden = std::fs::read(&path).expect("golden render (bless with DOOR_TEXT_BLESS=1)");
        let (header, expected) = qoi::decode_to_vec(&golden).unwrap();
        assert_eq!((header.width, header.height), (w, h));
        let differing = pixels
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 2))
            .count();
        assert!(
            differing * 1000 <= (w * h) as usize,
            "{name}: {differing} pixels differ"
        );
    }
}

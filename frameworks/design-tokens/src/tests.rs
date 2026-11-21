use super::*;
use std::{collections::HashSet, vec::Vec};

/// WCAG 2.x relative luminance of an opaque premultiplied BGRA8 colour.
fn luminance(c: Color) -> f64 {
    let channel = |v: u8| {
        let s = f64::from(v) / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(c[2]) + 0.7152 * channel(c[1]) + 0.0722 * channel(c[0])
}
fn ratio(a: Color, b: Color) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[test]
fn ids_are_unique_namespaced_and_resolvable() {
    let all: Vec<_> = tokens().collect();
    let ids: HashSet<_> = all.iter().map(|t| t.id).collect();
    assert_eq!(ids.len(), all.len(), "duplicate token id");
    assert_eq!(
        all.len(),
        COLORS.len()
            + TYPE_RAMP.len()
            + SPACING_TABLE.len()
            + RADII.len()
            + ELEVATIONS.len()
            + MOTIONS.len()
    );
    for t in &all {
        let prefix = match t.kind {
            Kind::Color => "color.",
            Kind::Typography => "type.",
            Kind::Spacing => {
                assert!(t.id.starts_with("space.") || t.id.starts_with("size."));
                ""
            }
            Kind::Radius => "radius.",
            Kind::Elevation => "elevation.",
            Kind::Motion => "motion.",
        };
        assert!(t.id.starts_with(prefix), "{}", t.id);
        assert!(!t.provenance.source.is_empty() && !t.controls.is_empty());
        assert_eq!(token(t.id), Some(*t));
        for theme in Theme::ALL {
            assert_eq!(lookup(t.id, theme), Some(t.value(theme)));
        }
    }
    assert_eq!(lookup("color.missing", Theme::Dark), None);
    // Typed enums index their own tables.
    for &c in ColorToken::ALL {
        assert_eq!(
            token(c.id()).unwrap().value(Theme::Dark),
            Value::Color(color(c, Theme::Dark))
        );
    }
    assert_eq!(ColorToken::ALL.len(), COLORS.len());
    assert_eq!(SpacingToken::ALL.len(), SPACING_TABLE.len());
    assert_eq!(MotionToken::ALL.len(), MOTIONS.len());
    // The cubic sampler: end points, and the curve the Settings entrance
    // was fitted to (DECELERATE_MAX: y 0.42 at x 0.06, 0.92 at x 0.37).
    assert_eq!(DECELERATE_MAX.sample(0), 0);
    assert_eq!(DECELERATE_MAX.sample(1000), 1000);
    assert!((400..440).contains(&DECELERATE_MAX.sample(60)));
    assert!((910..935).contains(&DECELERATE_MAX.sample(370)));
    assert_eq!(Easing::Linear.sample(250), 250);
    assert!(FAST_OUT_SLOW_IN.sample(500) > 800);
}

#[test]
fn every_token_has_all_three_theme_values_and_matching_kinds() {
    for t in tokens() {
        let kinds: Vec<_> = Theme::ALL
            .iter()
            .map(|&theme| core::mem::discriminant(&t.value(theme)))
            .collect();
        assert!(kinds.windows(2).all(|w| w[0] == w[1]), "{}", t.id);
        let expected = match t.kind {
            Kind::Color => matches!(t.light, Value::Color(_)),
            Kind::Typography => matches!(t.light, Value::Type(_)),
            Kind::Spacing | Kind::Radius => matches!(t.light, Value::Length(_)),
            Kind::Elevation => matches!(t.light, Value::Elevation(_)),
            Kind::Motion => matches!(t.light, Value::Motion(_)),
        };
        assert!(expected, "{}", t.id);
        // Typed accessors without a theme argument are honest only if the
        // kind is theme-independent.
        if matches!(
            t.kind,
            Kind::Typography | Kind::Spacing | Kind::Radius | Kind::Motion
        ) {
            assert!(t.light == t.dark && t.dark == t.contrast, "{}", t.id);
        }
    }
    // Contrast themes draw no shadows.
    for &e in ElevationToken::ALL {
        let hc = elevation(e, Theme::HighContrast);
        assert_eq!(
            (hc.left, hc.top, hc.right, hc.bottom, hc.color[3]),
            (0, 0, 0, 0, 0)
        );
    }
}

#[test]
fn text_on_surface_pairs_meet_4_5_to_1_in_every_theme() {
    use ColorToken::*;
    let pairs = [
        (TextPrimary, SurfaceWindow),
        (TextPrimary, SurfaceContent),
        (TextPrimary, SurfaceCaption),
        (TextPrimary, SurfaceCard),
        (TextPrimary, SurfaceFlyout),
        (TextPrimary, SurfaceSearchField),
        (TextPrimary, SurfaceTaskbar),
        (TextPrimary, SurfaceNotepadEditor),
        (TextPrimary, FillControl),
        (TextSecondary, SurfaceWindow),
        (TextSecondary, SurfaceCard),
        (TextSecondary, SurfaceFlyout),
        (TextOnAccent, AccentDefault),
        (AccentText, SurfaceWindow),
        (StatusCritical, SurfaceWindow),
    ];
    for theme in Theme::ALL {
        for (text, surface) in pairs {
            let (fg, bg) = (color(text, theme), color(surface, theme));
            assert_eq!(
                (fg[3], bg[3]),
                (255, 255),
                "{text:?}/{surface:?} must be opaque"
            );
            let r = ratio(fg, bg);
            assert!(
                r >= 4.5,
                "{theme:?}: {} on {} is {r:.2}:1",
                text.id(),
                surface.id()
            );
        }
    }
}

#[test]
fn provenance_follows_the_section_111_source_kinds() {
    for t in tokens() {
        let p = t.provenance;
        match p.source_kind {
            SourceKind::MeasuredReference => {
                assert!(p.measured_themes != 0, "{}", t.id);
                assert!(
                    p.source.starts_with("reports/artifacts/reference-26h2/"),
                    "{}",
                    t.id
                );
                assert!(matches!(
                    p.confidence,
                    Confidence::Measured | Confidence::Uncontrolled
                ));
            }
            SourceKind::BootstrapProposal => assert!(p.unresolved(), "{}", t.id),
            _ => assert_eq!(p.measured_themes, 0, "{}", t.id),
        }
    }
    assert_eq!(SourceKind::MeasuredReference.as_str(), "measured_reference");
    // The reference was captured in dark mode only.
    assert!(
        ColorToken::AccentDefault
            .entry()
            .provenance
            .measured(Theme::Dark)
    );
    assert!(
        !ColorToken::AccentDefault
            .entry()
            .provenance
            .measured(Theme::Light)
    );
    // Shell motion tracks are explicitly unresolved until captured; Start
    // opening was captured (VIS-01 start.open).
    assert!(MotionToken::StartClose.entry().provenance.unresolved());
    assert!(!MotionToken::StartOpen.entry().provenance.unresolved());
    assert!(!MotionToken::Normal.entry().provenance.unresolved());
}

#[test]
fn measured_reference_values_match_the_capture_arithmetic() {
    // 150 % captures: logical = physical / 1.5.
    for (token, physical) in [
        (SpacingToken::TaskbarHeight, 72),
        (SpacingToken::TaskbarButton, 66),
        (SpacingToken::TaskbarIcon, 36),
        (SpacingToken::TrayShowDesktop, 18),
        (SpacingToken::CaptionHeight, 48),
        (SpacingToken::CaptionButtonWidth, 69),
        (SpacingToken::NavPaneWidth, 450),
        (SpacingToken::NavItemHeight, 54),
    ] {
        assert_eq!(spacing(token) * 3, physical * 2, "{}", token.id());
    }
    assert_eq!(
        color(ColorToken::AccentDefault, Theme::Dark),
        rgb(0xdb, 0x9e, 0xe5)
    );
    assert_eq!(motion_with(MotionToken::StartOpen, true).duration_ms, 0);
    assert_eq!(motion_with(MotionToken::Fast, false).duration_ms, 167);
    assert_eq!(type_ramp(TypeToken::Body).size, 14);
    assert_eq!(radius(RadiusToken::Overlay), 8);
}

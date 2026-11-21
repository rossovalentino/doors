//! Display scale (spec §57): the shell font atlas for a scale and the
//! scales this server can render. Integer scales use the hinted 1x/2x/3x
//! atlases shared through graphics-text; 125/150/175% use exported atlases
//! embedded in the window server only and decoded through
//! `FontAtlas::from_packed_percent`, the same path a runtime-loaded atlas
//! takes. 225% and 250% have no exported atlas yet and are not offered.
extern crate alloc;
use alloc::string::String;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use door_graphics_core::Scale;
use door_graphics_text::{FontAtlas, FontRole, FontSet, TextSystem};
use door_service_protocol::window::{DISPLAY_SCALES, display_scale_offered};
/// Exported fractional shell atlases (`assets/fonts/segoe-ui/hinted-*.dfa`).
const FRACTIONAL: [(u32, &[u8]); 3] = [
    (
        125,
        include_bytes!("../../../assets/fonts/segoe-ui/hinted-125.dfa"),
    ),
    (
        150,
        include_bytes!("../../../assets/fonts/segoe-ui/hinted-150.dfa"),
    ),
    (
        175,
        include_bytes!("../../../assets/fonts/segoe-ui/hinted-175.dfa"),
    ),
];
/// A shell atlas exists for `percent`.
pub fn renderable(percent: u32) -> bool {
    matches!(percent, 100 | 200 | 300) || FRACTIONAL.iter().any(|(p, _)| *p == percent)
}
/// `percent` is offered on a `width` x `height` output: a Windows scale
/// that keeps at least 1024x600 logical pixels and that the shell renders.
pub fn offered(percent: u32, width: u32, height: u32) -> bool {
    renderable(percent) && display_scale_offered(width, height, percent)
}
/// `DISPLAY_SCALE_INFO` mask: bit i set when `DISPLAY_SCALES[i]` is offered.
pub fn offered_mask(width: u32, height: u32) -> u32 {
    DISPLAY_SCALES
        .iter()
        .enumerate()
        .filter(|(_, p)| offered(**p, width, height))
        .fold(0, |mask, (i, _)| mask | 1 << i)
}
/// Decoded shell font atlas for `scale`, or `None` when none is available.
/// Text outside ASCII renders through the installed Unicode engine.
pub fn shell_fonts(scale: Scale) -> Option<FontAtlas> {
    shell_fonts_text(scale, 100)
}
/// The shell atlas for `scale` at Accessibility > Text size `text`: glyphs
/// at display x text (a hinted atlas when one matches exactly, otherwise
/// Segoe UI outlines from the asset pack), positions at the display scale.
pub fn shell_fonts_text(scale: Scale, text: u32) -> Option<FontAtlas> {
    let mut atlas = if text == 100 {
        hinted_fonts(scale)?
    } else {
        text_fonts(scale, text)?
    };
    if let Some(system) = unicode() {
        atlas.attach_unicode(system, door_graphics_text::Family::Ui);
    }
    Some(atlas)
}
fn text_fonts(scale: Scale, text: u32) -> Option<FontAtlas> {
    let display = scale.percent();
    match FontAtlas::hinted_percent(display, text) {
        Some(p) if p.is_multiple_of(100) => FontAtlas::windows_reference_scaled(p / 100)
            .and_then(|a| a.for_text_size(display, text))
            .ok(),
        Some(p) => FRACTIONAL
            .iter()
            .find(|(f, _)| *f == p)
            .and_then(|(_, bytes)| FontAtlas::from_packed_text(bytes, display, text).ok()),
        None => outline_fonts(display, text),
    }
}
/// Segoe UI regular and semibold outlines from the asset pack (recorded by
/// `install_unicode`), for text sizes without an exported atlas.
static OUTLINES: [(AtomicPtr<u8>, AtomicUsize); 2] = [
    (AtomicPtr::new(core::ptr::null_mut()), AtomicUsize::new(0)),
    (AtomicPtr::new(core::ptr::null_mut()), AtomicUsize::new(0)),
];
fn outline(index: usize) -> Option<&'static [u8]> {
    let (ptr, len) = &OUTLINES[index];
    let len = len.load(Ordering::Acquire);
    let ptr = ptr.load(Ordering::Acquire);
    // SAFETY: only `install_unicode` stores these, from the process-lifetime
    // read-only pack mapping, before any reader; never freed.
    (len != 0 && !ptr.is_null()).then(|| unsafe { core::slice::from_raw_parts(ptr, len) })
}
#[cfg(target_os = "none")]
fn outline_fonts(display: u32, text: u32) -> Option<FontAtlas> {
    FontAtlas::outline_text(outline(0)?, outline(1)?, display, text).ok()
}
/// Host builds without an installed pack read the same outlines from the
/// repository.
#[cfg(not(target_os = "none"))]
fn outline_fonts(display: u32, text: u32) -> Option<FontAtlas> {
    match (outline(0), outline(1)) {
        (Some(regular), Some(semibold)) => {
            FontAtlas::outline_text(regular, semibold, display, text).ok()
        }
        _ => FontAtlas::windows_reference_text(display, text).ok(),
    }
}
static UNICODE: AtomicPtr<TextSystem> = AtomicPtr::new(core::ptr::null_mut());
/// The shell's Unicode text engine (ADR 0018): Segoe UI Variable, Segoe UI,
/// Segoe UI Symbol and Cascadia Mono from the application asset pack, plus
/// the on-demand fonts (emoji, CJK, Indic, historic) mapped on first use.
/// Installed once before the first `shell_fonts`; later calls are ignored.
pub fn install_unicode(pack: door_initfs::app_assets::Pack<'static>) -> bool {
    let files: alloc::vec::Vec<_> = [
        ("fonts/segoe-ui/SegUIVar.ttf", FontRole::UiVariable),
        ("fonts/segoe-ui/segoeui.ttf", FontRole::Ui),
        ("fonts/segoe-ui/seguisb.ttf", FontRole::Ui),
        ("fonts/windows-unicode/seguisym.ttf", FontRole::Symbol),
        ("fonts/cascadia-mono/CascadiaMono.ttf", FontRole::Monospace),
    ]
    .into_iter()
    .filter_map(|(key, role)| pack.get(key).map(|bytes| (bytes, role)))
    .collect();
    for (index, key) in ["fonts/segoe-ui/segoeui.ttf", "fonts/segoe-ui/seguisb.ttf"]
        .into_iter()
        .enumerate()
    {
        if let Some(bytes) = pack.get(key) {
            OUTLINES[index]
                .0
                .store(bytes.as_ptr() as *mut u8, Ordering::Release);
            OUTLINES[index].1.store(bytes.len(), Ordering::Release);
        }
    }
    let Ok(set) = FontSet::from_files(&files) else {
        return false;
    };
    let system = Arc::new(TextSystem::new(
        set.with_provider(alloc::boxed::Box::new(ResourceFonts)),
    ));
    let raw = Arc::into_raw(system) as *mut TextSystem;
    if UNICODE
        .compare_exchange(
            core::ptr::null_mut(),
            raw,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        // SAFETY: `raw` came from `Arc::into_raw` above and was never published.
        drop(unsafe { Arc::from_raw(raw) });
        return false;
    }
    true
}
fn unicode() -> Option<Arc<TextSystem>> {
    let raw = UNICODE.load(Ordering::Acquire);
    if raw.is_null() {
        return None;
    }
    // SAFETY: UNICODE keeps one strong reference for the process lifetime.
    unsafe {
        Arc::increment_strong_count(raw);
        Some(Arc::from_raw(raw))
    }
}
/// On-demand fonts (`door_abi::platform::resource`) for the shell's set.
struct ResourceFonts;
impl door_graphics_text::FontProvider for ResourceFonts {
    fn load(&self, role: FontRole) -> alloc::vec::Vec<&'static [u8]> {
        use door_abi::platform::resource as r;
        let id = match role {
            FontRole::Emoji => r::EMOJI,
            FontRole::Historic => r::HISTORIC,
            FontRole::ChineseSimplified => r::CHINESE_SIMPLIFIED,
            FontRole::ChineseTraditional => r::CHINESE_TRADITIONAL,
            FontRole::Japanese => r::JAPANESE,
            FontRole::Korean => r::KOREAN,
            FontRole::Indic => r::INDIC,
            _ => return alloc::vec::Vec::new(),
        };
        door_rt::resources::open(id).ok().into_iter().collect()
    }
}
fn hinted_fonts(scale: Scale) -> Option<FontAtlas> {
    match scale.factor() {
        Some(factor) => FontAtlas::windows_reference_scaled(factor).ok(),
        None => FRACTIONAL
            .iter()
            .find(|(p, _)| *p == scale.percent())
            .and_then(|(p, bytes)| FontAtlas::from_packed_percent(bytes, *p).ok()),
    }
}
/// Scale as a decimal factor for diagnostics: "2", "1.5", "1.25".
pub fn factor_text(scale: Scale) -> String {
    let p = scale.percent();
    let mut text = alloc::format!("{}", p / 100);
    if !p.is_multiple_of(100) {
        let fraction = alloc::format!("{:02}", p % 100);
        text.push('.');
        text.push_str(fraction.trim_end_matches('0'));
    }
    text
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_offered_scale_has_a_shell_atlas_at_its_percent() {
        for percent in Scale::PERCENTS {
            let scale = Scale::from_percent(percent).unwrap();
            match shell_fonts(scale) {
                Some(fonts) => {
                    assert!(renderable(percent));
                    assert_eq!(fonts.percent(), percent);
                    assert!(fonts.uses_packed_cache(), "{percent}%");
                }
                None => assert!(!renderable(percent), "{percent}%"),
            }
        }
        assert_eq!(offered_mask(1920, 1080), 0b1111);
        assert_eq!(offered_mask(2560, 1600), 0b001_1111);
        assert_eq!(offered_mask(3840, 2160), 0b1001_1111);
        assert_eq!(offered_mask(1280, 720), 0b1);
        let text = Scale::PERCENTS.map(|p| factor_text(Scale::from_percent(p).unwrap()));
        assert_eq!(text, ["1", "1.25", "1.5", "1.75", "2", "2.25", "2.5", "3"]);
    }
    /// The taskbar at fractional scales on a 2560x1600 output: the material
    /// region equals the painted band, every band pixel is painted (no seam
    /// row or column), nothing bleeds above it, and the chrome draws ink.
    #[test]
    fn fractional_taskbar_band_is_seam_free_and_matches_its_material() {
        use crate::taskbar::{Layout, View, draw, physical};
        use door_graphics_core::Canvas;
        let (pw, ph) = (2560u32, 1600u32);
        for percent in [125, 150, 175] {
            let scale = Scale::from_percent(percent).unwrap();
            let fonts = shell_fonts(scale).unwrap();
            let layout = Layout::new(scale.logical_len(pw), scale.logical_len(ph));
            let bar = physical(layout.bar(), scale);
            let material = crate::material::Taskbar::new_scaled(pw, ph, scale).unwrap();
            assert_eq!(
                material.pixels().len(),
                bar.width() as usize * bar.height() as usize * 4,
                "{percent}%"
            );
            assert_eq!(ph - bar.y() as u32, crate::policy::bar_rows(ph, scale));
            let tint = [40u8, 40, 40, 255];
            let backdrop = tint.repeat(bar.width() as usize * bar.height() as usize);
            const SENTINEL: [u8; 4] = [7, 11, 13, 255];
            let mut bytes = SENTINEL.repeat((pw * ph) as usize);
            let mut c = Canvas::new(&mut bytes, pw, ph, pw as usize * 4).unwrap();
            c.set_scale(scale).unwrap();
            draw(
                &mut c,
                &fonts,
                layout,
                &View {
                    buttons: &[],
                    pointer: (0, 0),
                    pressed: None,
                    press: None,
                    hover: [(None, 0); 2],
                    start_open: false,
                    search_open: false,
                    task_view_open: false,
                    quick_open: false,
                    calendar_open: false,
                    hidden_icons: None,
                    time: None,
                    region: 0,
                    overrides: 0,
                    network: crate::taskbar::Net::Connected,
                    vpn: false,
                    audio: None,
                    dark: true,
                    material: Some(&backdrop),
                },
            );
            let at = |x: i32, y: i32| {
                let i = (y as usize * pw as usize + x as usize) * 4;
                [bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]
            };
            let mut ink = 0;
            for y in bar.y()..bar.bottom() {
                for x in bar.x()..bar.right() {
                    let p = at(x, y);
                    assert_ne!(p, SENTINEL, "{percent}% unpainted band pixel {x},{y}");
                    ink += usize::from(p != tint);
                }
            }
            assert!(ink > 500, "{percent}%: taskbar chrome drew {ink} pixels");
            for x in (0..bar.right()).step_by(7) {
                assert_eq!(
                    at(x, bar.y() - 1),
                    SENTINEL,
                    "{percent}% bled above the band at {x}"
                );
            }
        }
    }
}

//! Settings over Mica (owner request 2026-11-12): a guest capture of the
//! Settings background is checked against the compositor's own Mica
//! arithmetic over the same wallpaper (the desktop asset pack's picture,
//! fitted to the output as the window server fits it), so the check stays
//! exact while the colour follows the wallpaper.
use door_window::graphics::{MicaField, Theme};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Probe colour standing for "Settings' background over Mica" on wallpaper
/// `picture` (`door_initfs::desktop_assets::WALLPAPERS` id) in the dark or
/// light theme. It matches the exact Mica at the probed screen point, or the
/// solid Mica fallback while Settings is not the active window (Windows 11
/// shows #202020 / #F3F3F3 then).
pub(super) const fn settings(picture: u8, dark: bool) -> [u8; 3] {
    [253, 0x80 | picture, 0x40 | dark as u8]
}
/// The Settings title-bar search box (its control fill's veil) over Mica.
pub(super) const fn settings_control(picture: u8, dark: bool) -> [u8; 3] {
    [253, 0x80 | picture, 0x40 | (CONTROL << 1) | dark as u8]
}
#[cfg(test)]
const CLEAR: u8 = 0;
const CONTROL: u8 = 1;
fn decode(expected: [u8; 3]) -> Option<(usize, bool, u8)> {
    let picture = usize::from(expected[1] & 0x7f);
    let fill = (expected[2] >> 1) & 0x1f;
    (expected[0] == 253
        && expected[1] & 0x80 != 0
        && expected[2] & 0xc0 == 0x40
        && fill <= CONTROL
        && picture < door_initfs::desktop_assets::COUNT)
        .then_some((picture, expected[2] & 1 == 1, fill))
}
/// The veil Settings paints for `fill` over Mica (premultiplied BGRA).
fn veil(fill: u8, dark: bool) -> [u8; 4] {
    use door_workspace::palette::{SETTINGS_BG, SETTINGS_CONTROL, mica_overlay};
    let pick = |pair: ([u8; 4], [u8; 4])| if dark { pair.1 } else { pair.0 };
    match fill {
        CONTROL => mica_overlay(pick(SETTINGS_CONTROL), pick(SETTINGS_BG)),
        _ => [0, 0, 0, 0],
    }
}
/// `Some(matched)` when `expected` is a Mica probe, else None.
pub(super) fn matches(
    expected: [u8; 3],
    got: [u8; 3],
    point: (usize, usize),
    output: (usize, usize),
) -> Option<bool> {
    let (picture, dark, fill) = decode(expected)?;
    let over = |base: [u8; 4]| {
        door_window::graphics::blend_bgra(base, veil(fill, dark))
            .map(|p| [p[2], p[1], p[0]])
            .unwrap_or([0; 3])
    };
    let solid = if dark {
        [32, 32, 32, 255]
    } else {
        [243, 243, 243, 255]
    };
    if got == over(solid) {
        return Some(true);
    }
    Some(
        field(picture, dark, output)
            .is_ok_and(|f| got == over(f.pixel(point.0 as i32, point.1 as i32))),
    )
}
type Key = (usize, bool, usize, usize);
fn field(picture: usize, dark: bool, output: (usize, usize)) -> Result<Arc<MicaField>, String> {
    static CACHE: OnceLock<Mutex<HashMap<Key, Arc<MicaField>>>> = OnceLock::new();
    let key = (picture, dark, output.0, output.1);
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(f) = cache.lock().map_err(|_| "mica cache")?.get(&key) {
        return Ok(f.clone());
    }
    let file = door_initfs::desktop_assets::WALLPAPERS[picture].file;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/wallpapers/windows/26h2")
        .join(file);
    let source = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (width, height) = (output.0 as u32, output.1 as u32);
    let (mut wallpaper, mut scratch) = (Vec::new(), Vec::new());
    door_window::wallpaper::load_fitted(
        &source,
        width,
        height,
        &mut wallpaper,
        &mut scratch,
        door_window::wallpaper::Fit::Fill,
        [0, 0, 0],
    )?;
    let theme = if dark { Theme::Dark } else { Theme::Light };
    let f =
        Arc::new(MicaField::new(&wallpaper, width, height, theme).map_err(|e| format!("{e:?}"))?);
    cache
        .lock()
        .map_err(|_| "mica cache")?
        .insert(key, f.clone());
    Ok(f)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mica_probes_decode_and_accept_the_fallback() {
        assert_eq!(decode(settings(0, false)), Some((0, false, CLEAR)));
        assert_eq!(decode(settings(17, true)), Some((17, true, CLEAR)));
        assert_eq!(
            decode(settings_control(3, false)),
            Some((3, false, CONTROL))
        );
        // The search box veil over the light fallback is a near-white grey.
        let box_light = veil(CONTROL, false);
        assert!(
            box_light[3] > 100 && box_light[0] == box_light[3],
            "{box_light:?}"
        );
        assert_eq!(decode([244, 247, 250]), None);
        assert_eq!(
            matches([244, 247, 250], [244, 247, 250], (0, 0), (8, 8)),
            None
        );
        assert_eq!(
            matches(settings(1, true), [32, 32, 32], (0, 0), (64, 64)),
            Some(true)
        );
        // Dark Mica over dark Bloom is the tinted, blue-leaning grey of the
        // owner's Settings, never the old flat background.
        let f = field(1, true, (1280, 720)).unwrap();
        let p = f.pixel(1200, 600);
        assert!(p[0] > p[2], "blue over red: {p:?}");
        assert!(
            matches(
                settings(1, true),
                [p[2], p[1], p[0]],
                (1200, 600),
                (1280, 720)
            ) == Some(true)
        );
        assert!(matches(settings(1, true), [31, 32, 36], (1200, 600), (1280, 720)) == Some(false));
    }
}

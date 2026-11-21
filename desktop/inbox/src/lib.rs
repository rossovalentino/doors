#![no_std]
//! Door's inbox apps: Windows 11 26H2's Calculator, Clock and the other
//! bundled tools, one protected image that runs the app its
//! kernel-authenticated manifest names (`door_abi::platform::manifest::
//! CALCULATOR`, `CLOCK`, ...).
extern crate alloc;
pub mod access;
pub mod calculator;
pub mod clock;
pub mod paint;
pub mod snipping;
pub mod weather;
pub mod weather_data;

use door_app_ui::View;
use door_graphics_text::TextStyle;

/// The bundled apps this image runs, by launch manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum App {
    Calculator,
    Clock,
    Paint,
    SnippingTool,
    Weather,
    Magnifier,
    Narrator,
    OnScreenKeyboard,
    VoiceAccess,
}
impl App {
    pub const ALL: [Self; 9] = [
        Self::Calculator,
        Self::Clock,
        Self::Paint,
        Self::SnippingTool,
        Self::Weather,
        Self::Magnifier,
        Self::Narrator,
        Self::OnScreenKeyboard,
        Self::VoiceAccess,
    ];
    pub const fn manifest(self) -> u64 {
        match self {
            Self::Calculator => door_abi::platform::manifest::CALCULATOR,
            Self::Clock => door_abi::platform::manifest::CLOCK,
            Self::Paint => door_abi::platform::manifest::PAINT,
            Self::SnippingTool => door_abi::platform::manifest::SNIPPING_TOOL,
            Self::Weather => door_abi::platform::manifest::WEATHER,
            Self::Magnifier => door_abi::platform::manifest::MAGNIFIER,
            Self::Narrator => door_abi::platform::manifest::NARRATOR,
            Self::OnScreenKeyboard => door_abi::platform::manifest::ON_SCREEN_KEYBOARD,
            Self::VoiceAccess => door_abi::platform::manifest::VOICE_ACCESS,
        }
    }
    pub fn from_manifest(manifest: u64) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.manifest() == manifest)
    }
    /// The shell icon reference of its toasts (`shell::icon`).
    pub const fn icon(self) -> u16 {
        match self {
            Self::Calculator => door_service_protocol::shell::icon::CALCULATOR,
            Self::Clock => door_service_protocol::shell::icon::CLOCK,
            Self::Paint => door_service_protocol::shell::icon::PAINT,
            Self::SnippingTool => door_service_protocol::shell::icon::SNIPPING_TOOL,
            Self::Weather => door_service_protocol::shell::icon::WEATHER,
            Self::Magnifier => door_service_protocol::shell::icon::MAGNIFIER,
            Self::Narrator => door_service_protocol::shell::icon::NARRATOR,
            Self::OnScreenKeyboard => door_service_protocol::shell::icon::ON_SCREEN_KEYBOARD,
            Self::VoiceAccess => door_service_protocol::shell::icon::VOICE_ACCESS,
        }
    }
}

/// The app's installed Windows icon (`assets/icons/windows`, its package's
/// unplated artwork), premultiplied BGRA, 16 px logical at an integer
/// density.
pub fn icon_frames(app: App, scale: u32) -> Option<&'static [u8]> {
    let bytes = match (app, scale) {
        (App::Calculator, 1) => {
            door_workspace::asset!("../../../assets/icons/windows/calculator.16.bgra")
        }
        (App::Calculator, 2) => {
            door_workspace::asset!("../../../assets/icons/windows/calculator.32.bgra")
        }
        (App::Calculator, _) => {
            door_workspace::asset!("../../../assets/icons/windows/calculator.48.bgra")
        }
        (App::Clock, 1) => door_workspace::asset!("../../../assets/icons/windows/clock.16.bgra"),
        (App::Clock, 2) => door_workspace::asset!("../../../assets/icons/windows/clock.32.bgra"),
        (App::Clock, _) => door_workspace::asset!("../../../assets/icons/windows/clock.48.bgra"),
        (App::Paint, 1) => door_workspace::asset!("../../../assets/icons/windows/paint.16.bgra"),
        (App::Paint, 2) => door_workspace::asset!("../../../assets/icons/windows/paint.32.bgra"),
        (App::Paint, _) => door_workspace::asset!("../../../assets/icons/windows/paint.48.bgra"),
        (App::SnippingTool, 1) => {
            door_workspace::asset!("../../../assets/icons/windows/snipping-tool.16.bgra")
        }
        (App::SnippingTool, 2) => {
            door_workspace::asset!("../../../assets/icons/windows/snipping-tool.32.bgra")
        }
        (App::SnippingTool, _) => {
            door_workspace::asset!("../../../assets/icons/windows/snipping-tool.48.bgra")
        }
        (App::Weather, 1) => {
            door_workspace::asset!("../../../assets/icons/windows/weather.16.bgra")
        }
        (App::Weather, 2) => {
            door_workspace::asset!("../../../assets/icons/windows/weather.32.bgra")
        }
        (App::Weather, _) => {
            door_workspace::asset!("../../../assets/icons/windows/weather.48.bgra")
        }
        (App::Magnifier, 1) => {
            door_workspace::asset!("../../../assets/icons/windows/magnifier.16.bgra")
        }
        (App::Magnifier, 2) => {
            door_workspace::asset!("../../../assets/icons/windows/magnifier.32.bgra")
        }
        (App::Magnifier, _) => {
            door_workspace::asset!("../../../assets/icons/windows/magnifier.48.bgra")
        }
        (App::Narrator, 1) => {
            door_workspace::asset!("../../../assets/icons/windows/narrator.16.bgra")
        }
        (App::Narrator, 2) => {
            door_workspace::asset!("../../../assets/icons/windows/narrator.32.bgra")
        }
        (App::Narrator, _) => {
            door_workspace::asset!("../../../assets/icons/windows/narrator.48.bgra")
        }
        (App::OnScreenKeyboard, 1) => {
            door_workspace::asset!("../../../assets/icons/windows/on-screen-keyboard.16.bgra")
        }
        (App::OnScreenKeyboard, 2) => {
            door_workspace::asset!("../../../assets/icons/windows/on-screen-keyboard.32.bgra")
        }
        (App::OnScreenKeyboard, _) => {
            door_workspace::asset!("../../../assets/icons/windows/on-screen-keyboard.48.bgra")
        }
        (App::VoiceAccess, 1) => {
            door_workspace::asset!("../../../assets/icons/windows/voice-access.16.bgra")
        }
        (App::VoiceAccess, 2) => {
            door_workspace::asset!("../../../assets/icons/windows/voice-access.32.bgra")
        }
        (App::VoiceAccess, _) => {
            door_workspace::asset!("../../../assets/icons/windows/voice-access.48.bgra")
        }
    };
    (!bytes.is_empty()).then_some(bytes)
}

/// A glyph mask (`assets/icons/windows-shell`: Segoe Fluent, or Clock's own
/// ClockIcons for `clock-*`) at `size` logical px.
pub fn glyph(v: &mut View<'_>, name: &str, x: i32, y: i32, size: u32, color: [u8; 4]) {
    let scale = v.scale.clamp(1, 3);
    let px = size * scale;
    macro_rules! mask {
        ($name:literal) => {
            match px {
                12 => door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-shell/",
                    $name,
                    ".12.mask"
                )),
                16 => door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-shell/",
                    $name,
                    ".16.mask"
                )),
                20 => door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-shell/",
                    $name,
                    ".20.mask"
                )),
                24 => door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-shell/",
                    $name,
                    ".24.mask"
                )),
                32 => door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-shell/",
                    $name,
                    ".32.mask"
                )),
                36 => door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-shell/",
                    $name,
                    ".36.mask"
                )),
                40 => door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-shell/",
                    $name,
                    ".40.mask"
                )),
                48 => door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-shell/",
                    $name,
                    ".48.mask"
                )),
                60 => door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-shell/",
                    $name,
                    ".60.mask"
                )),
                _ => &[][..],
            }
        };
    }
    let bytes: &[u8] = match name {
        "calc-percent" => mask!("calc-percent"),
        "calc-backspace" => mask!("calc-backspace"),
        "calc-divide" => mask!("calc-divide"),
        "calc-multiply" => mask!("calc-multiply"),
        "calc-subtract" => mask!("calc-subtract"),
        "calc-add" => mask!("calc-add"),
        "calc-equals" => mask!("calc-equals"),
        "calc-root" => mask!("calc-root"),
        "calc-nav" => mask!("calc-nav"),
        "calc-history" => mask!("calc-history"),
        "calc-delete" => mask!("calc-delete"),
        "calc-standard" => mask!("calc-standard"),
        "clock-focus" => mask!("clock-focus"),
        "clock-timer" => mask!("clock-timer"),
        "clock-alarm" => mask!("clock-alarm"),
        "clock-stopwatch" => mask!("clock-stopwatch"),
        "clock-world" => mask!("clock-world"),
        "clock-settings" => mask!("clock-settings"),
        "clock-play" => mask!("clock-play"),
        "clock-pause" => mask!("clock-pause"),
        "clock-reset" => mask!("clock-reset"),
        "clock-flag" => mask!("clock-flag"),
        "clock-up" => mask!("clock-up"),
        "clock-down" => mask!("clock-down"),
        "clock-check" => mask!("clock-check"),
        "clock-add" => mask!("clock-add"),
        "clock-delete" => mask!("clock-delete"),
        _ => &[],
    };
    if bytes.len() == (px * px) as usize {
        v.mask_pixels(x, y, size as usize, bytes, color, None);
    }
}

/// Right-aligned display text with its right edge at `right` and baseline
/// box bottom at `bottom`, Semibold at `size` px, shrunk until it fits
/// `width`. False when the process has no scalable text engine.
pub fn big_text(
    v: &mut View<'_>,
    text: &str,
    size: &mut f32,
    width: u32,
    right: i32,
    bottom: i32,
    color: [u8; 4],
) -> bool {
    let Some(system) = door_workspace::assets::text_system() else {
        return false;
    };
    let scale = v.zoom.percent() as f32 / 100.0;
    loop {
        let format = system.format(*size, 600, scale, "en-US");
        let Ok(layout) = system.layout_line(text, &format) else {
            return false;
        };
        let (pw, ph) = layout.physical_size();
        let (w, h) = ((pw as f32 / scale) as i32, (ph as f32 / scale) as i32);
        if w as u32 <= width || *size <= 20.0 {
            v.text_layout(right - w, bottom - h, &system, &layout, color);
            return true;
        }
        *size -= 2.0;
    }
}

/// "¹⁄ₓ"-style key faces: `top` raised small, a slash, `bottom` lowered.
pub fn fraction(v: &mut View<'_>, cx: i32, cy: i32, top: &str, bottom: &str, color: [u8; 4]) {
    let s = TextStyle::Regular12;
    let tw = v.measure_style(top, s) as i32;
    let bw = v.measure_style(bottom, TextStyle::Regular16) as i32;
    let slash = v.measure_style("/", TextStyle::Regular16) as i32;
    let x = cx - (tw + slash + bw) / 2;
    v.text_clip_centered(x, (cy - 16, 20), tw as u32 + 2, top, s, color);
    v.text_clip_centered(
        x + tw - 1,
        (cy - 10, 20),
        slash as u32 + 2,
        "/",
        TextStyle::Regular16,
        color,
    );
    v.text_clip_centered(
        x + tw + slash - 2,
        (cy - 6, 20),
        bw as u32 + 2,
        bottom,
        TextStyle::Regular16,
        color,
    );
}
/// "x²": the base with a raised small exponent.
pub fn power(v: &mut View<'_>, cx: i32, cy: i32, base: &str, exponent: &str, color: [u8; 4]) {
    let bw = v.measure_style(base, TextStyle::Regular16) as i32;
    let ew = v.measure_style(exponent, TextStyle::Regular12) as i32;
    let x = cx - (bw + ew) / 2;
    v.text_clip_centered(
        x,
        (cy - 10, 20),
        bw as u32 + 2,
        base,
        TextStyle::Regular16,
        color,
    );
    v.text_clip_centered(
        x + bw,
        (cy - 17, 20),
        ew as u32 + 2,
        exponent,
        TextStyle::Regular12,
        color,
    );
}
/// "²√x": a small index, the radical glyph and x.
pub fn root(v: &mut View<'_>, cx: i32, cy: i32, color: [u8; 4]) {
    let iw = v.measure_style("2", TextStyle::Regular12) as i32;
    let xw = v.measure_style("x", TextStyle::Regular16) as i32;
    let x = cx - (iw + 14 + xw) / 2;
    v.text_clip_centered(
        x,
        (cy - 17, 20),
        iw as u32 + 2,
        "2",
        TextStyle::Regular12,
        color,
    );
    glyph(v, "calc-root", x + iw - 3, cy - 8, 16, color);
    v.text_clip_centered(
        x + iw + 12,
        (cy - 10, 20),
        xw as u32 + 2,
        "x",
        TextStyle::Regular16,
        color,
    );
}

/// door.accessibility: Calculator's display, keys and memory row.
pub fn describe_calculator(
    c: &calculator::Calculator,
    s: &mut door_workspace::semantics::Semantics,
) {
    use door_workspace::semantics::{Role, States};
    let r4 = |r: calculator::Rect| (r.x, r.y, r.w, r.h);
    s.control(
        Role::Button,
        "Open Navigation",
        (4, calculator::TITLE + 2, 40, 40),
        Some(40),
    );
    let expression = c.engine.expression();
    if !expression.is_empty() {
        s.text(
            Role::Text,
            &alloc::format!("Expression is {expression}"),
            (0, calculator::TITLE + 44, c.width, 24),
        );
    }
    s.text(
        Role::Text,
        &alloc::format!("Display is {}", c.engine.display()),
        (0, calculator::TITLE + 68, c.width, 60),
    );
    for (i, (name, enabled, r)) in calculator::key_labels(c).into_iter().enumerate() {
        s.control(Role::Button, &name, r4(r), Some(1 + i as u32))
            .state(States::DISABLED, !enabled);
    }
}

/// Width in logical px of `text` in Semibold at `size`, when the process
/// has a scalable text engine.
pub fn text_width(v: &View<'_>, text: &str, size: f32) -> Option<u32> {
    let system = door_workspace::assets::text_system()?;
    let scale = v.zoom.percent() as f32 / 100.0;
    let layout = system
        .layout_line(text, &system.format(size, 600, scale, "en-US"))
        .ok()?;
    Some((layout.physical_size().0 as f32 / scale) as u32)
}
/// Semibold `text` at `size` px centred on `cx` with its line box top at
/// `top`. False without a scalable text engine.
pub fn big_text_centered(
    v: &mut View<'_>,
    text: &str,
    size: &mut f32,
    cx: i32,
    top: i32,
    color: [u8; 4],
) -> bool {
    let Some(system) = door_workspace::assets::text_system() else {
        return false;
    };
    let scale = v.zoom.percent() as f32 / 100.0;
    let Ok(layout) = system.layout_line(text, &system.format(*size, 600, scale, "en-US")) else {
        return false;
    };
    let w = (layout.physical_size().0 as f32 / scale) as i32;
    v.text_layout(cx - w / 2, top, &system, &layout, color);
    true
}
/// A progress ring of radius `r` and stroke `width` centred on (cx, cy):
/// the track, then the remaining `permille` clockwise from 12 o'clock, each
/// an antialiased annulus mask at the view's density (round caps on the
/// remaining arc, as WinUI's ProgressRing draws it).
#[allow(clippy::too_many_arguments)]
pub fn ring(
    v: &mut View<'_>,
    cx: i32,
    cy: i32,
    r: u32,
    width: u32,
    permille: u32,
    track: [u8; 4],
    fill: [u8; 4],
) {
    let scale = v.scale.clamp(1, 3);
    let side = 2 * r + width + 2;
    let px = (side * scale) as usize;
    let (radius, half) = ((r * scale) as f32, (width * scale) as f32 / 2.0);
    let centre = px as f32 / 2.0;
    let mut track_mask = alloc::vec![0u8; px * px];
    let mut fill_mask = alloc::vec![0u8; px * px];
    let sweep = permille.min(1000) as f32 / 1000.0;
    // Round caps: the arc's two ends as points on the circle.
    let tau = core::f32::consts::TAU;
    let end = |turn: f32| {
        let (s, c) = sin_cos(turn * tau);
        (centre + s * radius, centre - c * radius)
    };
    let caps = [end(0.0), end(sweep)];
    for y in 0..px {
        for x in 0..px {
            let (dx, dy) = (x as f32 + 0.5 - centre, y as f32 + 0.5 - centre);
            let d = sqrt(dx * dx + dy * dy);
            let band = (half + 0.5 - (d - radius).abs()).clamp(0.0, 1.0);
            let i = y * px + x;
            if band > 0.0 {
                track_mask[i] = (band * 255.0) as u8;
                // Clockwise turn from 12 o'clock in [0, 1).
                let turn = atan2(dx, -dy) / tau;
                let turn = if turn < 0.0 { turn + 1.0 } else { turn };
                if turn < sweep {
                    fill_mask[i] = (band * 255.0) as u8;
                }
            }
            if sweep > 0.0 && sweep < 1.0 {
                for (ex, ey) in caps {
                    let (ux, uy) = (x as f32 + 0.5 - ex, y as f32 + 0.5 - ey);
                    let cap = (half + 0.5 - sqrt(ux * ux + uy * uy)).clamp(0.0, 1.0);
                    fill_mask[i] = fill_mask[i].max((cap * 255.0) as u8);
                }
            }
        }
    }
    let (x0, y0) = (cx - side as i32 / 2, cy - side as i32 / 2);
    v.mask_pixels(x0, y0, side as usize, &track_mask, track, None);
    v.mask_pixels(x0, y0, side as usize, &fill_mask, fill, None);
}
/// Newton square root (no_std has no float sqrt without libm).
fn sqrt(v: f32) -> f32 {
    if v <= 0.0 {
        return 0.0;
    }
    let mut x = if v > 1.0 { v } else { 1.0 };
    for _ in 0..16 {
        x = 0.5 * (x + v / x);
    }
    x
}
/// atan2 to within 0.005 rad (polynomial on the octant, no libm).
fn atan2(y: f32, x: f32) -> f32 {
    use core::f32::consts::{FRAC_PI_2, PI};
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    let (ax, ay) = (x.abs(), y.abs());
    let (a, swap) = if ay > ax {
        (ax / ay, true)
    } else {
        (ay / ax, false)
    };
    let s = a * a;
    let mut r = ((-0.046_496_475 * s + 0.159_314_22) * s - 0.327_622_76) * s * a + a;
    if swap {
        r = FRAC_PI_2 - r;
    }
    if x < 0.0 {
        r = PI - r;
    }
    if y < 0.0 { -r } else { r }
}
/// sin and cos by Taylor series after range reduction (no libm).
fn sin_cos(t: f32) -> (f32, f32) {
    let tau = core::f32::consts::TAU;
    let mut x = t % tau;
    if x > core::f32::consts::PI {
        x -= tau;
    }
    let series = |x: f32, mut term: f32, start: u32| {
        let mut sum = term;
        let mut n = start;
        for _ in 0..8 {
            term *= -x * x / ((n + 1) * (n + 2)) as f32;
            sum += term;
            n += 2;
        }
        sum
    };
    (series(x, x, 1), series(x, 1.0, 0))
}
/// A WinUI toggle switch (40 x 20): accent track with the knob right when
/// on; off, an outlined track (filled with `fill`) with the knob left.
#[allow(clippy::too_many_arguments)]
pub fn switch(
    v: &mut View<'_>,
    r: (i32, i32, u32, u32),
    on: bool,
    accent: [u8; 4],
    outline: [u8; 4],
    knob_on: [u8; 4],
    fill: [u8; 4],
) {
    let (x, y, w, h) = r;
    if on {
        v.rounded_asset(x, y, w, h, h / 2, accent);
        v.rounded_asset(x + w as i32 - 16, y + 4, 12, 12, 6, knob_on);
    } else {
        v.rounded_asset(x, y, w, h, h / 2, outline);
        v.rounded_asset(x + 1, y + 1, w - 2, h - 2, (h - 2) / 2, fill);
        v.rounded_asset(x + 5, y + 5, 10, 10, 5, outline);
    }
}
/// Create `dir` and any missing parents under the user's home (a fresh
/// profile may not have Pictures yet). Errors surface at the save.
pub fn ensure_dirs(s: &mut door_service_client::Storage, dir: &str) {
    let mut path = alloc::string::String::new();
    for (i, part) in dir.split('/').filter(|p| !p.is_empty()).enumerate() {
        path.push('/');
        path.push_str(part);
        // /users/<uid> exists; create what lies below it.
        if i >= 2 {
            let _ = s.mkdir(&path);
        }
    }
}
/// The RTC's UTC wall time (FILETIME), None without a valid RTC or on host.
pub fn wall_utc() -> Option<u64> {
    door_sys::wall_time().ok()
}
/// Local time in zone `zone`: (civil time, weekday Sunday=0, day number).
pub fn local_now(zone: u32) -> Option<(door_abi::clock::WallClock, u8, u32)> {
    let local = door_locale::tz::zone_or_utc(zone).to_local(wall_utc()?)?;
    let w = local.time;
    let day = door_abi::clock::days_from_civil(w.year, w.month, w.day) as u32;
    Some((w, w.weekday()?, day))
}
/// Posts queued toasts from an inbox app (Init attributes them to it).
pub fn post_toasts(
    registry: u64,
    app: App,
    toasts: &mut alloc::vec::Vec<(alloc::string::String, alloc::string::String)>,
) {
    if registry == 0 {
        toasts.clear();
        return;
    }
    let Ok(mut n) = door_service_client::Notifications::connect(registry) else {
        toasts.clear();
        return;
    };
    for (title, body) in toasts.drain(..) {
        let _ = n.post(&door_service_client::notifications::Toast {
            icon: app.icon(),
            flags: 0,
            title: &title,
            body: &body,
            actions: ["", ""],
        });
    }
}
#[cfg(test)]
mod tests;

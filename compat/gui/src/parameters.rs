//! SystemParametersInfo: the system-wide settings Windows programs read
//! (fonts, the work area, accessibility, mouse, keyboard and animation
//! settings), answered with Windows 11's defaults at 100% scale. The
//! settings Doors does not let programs change (SPI_SET*) are accepted
//! without effect, as a standard user's per-session change that is not
//! persisted; unknown actions fail with ERROR_INVALID_PARAMETER.
use crate::Rect;
use crate::metrics::{CAPTION, system_metric};
use alloc::vec::Vec;

/// What SystemParametersInfo answers for an action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parameter {
    /// A BOOL, UINT or int written to pvParam (4 bytes).
    Value(i32),
    /// Bytes written to pvParam. `sized`: the structure starts with its
    /// cbSize, which the caller must have set to `bytes.len()` (or one of
    /// `also`, an older smaller layout: only that much is written).
    Struct {
        bytes: Vec<u8>,
        sized: bool,
        also: Option<usize>,
    },
    /// A string into pvParam's buffer of uiParam characters.
    Text(&'static str),
    /// An SPI_SET action: accepted, nothing changes.
    Set,
}

pub mod spi {
    pub const GETBEEP: u32 = 0x0001;
    pub const GETMOUSE: u32 = 0x0003;
    pub const GETBORDER: u32 = 0x0005;
    pub const GETKEYBOARDSPEED: u32 = 0x000A;
    pub const ICONHORIZONTALSPACING: u32 = 0x000D;
    pub const GETSCREENSAVETIMEOUT: u32 = 0x000E;
    pub const GETSCREENSAVEACTIVE: u32 = 0x0010;
    pub const GETGRIDGRANULARITY: u32 = 0x0012;
    pub const GETKEYBOARDDELAY: u32 = 0x0016;
    pub const ICONVERTICALSPACING: u32 = 0x0018;
    pub const GETICONTITLEWRAP: u32 = 0x0019;
    pub const GETMENUDROPALIGNMENT: u32 = 0x001B;
    pub const GETICONTITLELOGFONT: u32 = 0x001F;
    pub const GETFASTTASKSWITCH: u32 = 0x0023;
    pub const GETDRAGFULLWINDOWS: u32 = 0x0026;
    pub const GETNONCLIENTMETRICS: u32 = 0x0029;
    pub const GETMINIMIZEDMETRICS: u32 = 0x002B;
    pub const GETICONMETRICS: u32 = 0x002D;
    pub const GETWORKAREA: u32 = 0x0030;
    pub const GETFILTERKEYS: u32 = 0x0032;
    pub const GETTOGGLEKEYS: u32 = 0x0034;
    pub const GETMOUSEKEYS: u32 = 0x0036;
    pub const GETSHOWSOUNDS: u32 = 0x0038;
    pub const GETSTICKYKEYS: u32 = 0x003A;
    pub const GETACCESSTIMEOUT: u32 = 0x003C;
    pub const GETSOUNDSENTRY: u32 = 0x0040;
    pub const GETHIGHCONTRAST: u32 = 0x0042;
    pub const GETKEYBOARDPREF: u32 = 0x0044;
    pub const GETSCREENREADER: u32 = 0x0046;
    pub const GETANIMATION: u32 = 0x0048;
    pub const GETFONTSMOOTHING: u32 = 0x004A;
    pub const GETLOWPOWERACTIVE: u32 = 0x0053;
    pub const GETPOWEROFFACTIVE: u32 = 0x0054;
    pub const GETMOUSETRAILS: u32 = 0x005E;
    pub const GETSNAPTODEFBUTTON: u32 = 0x005F;
    pub const GETMOUSEHOVERWIDTH: u32 = 0x0062;
    pub const GETMOUSEHOVERHEIGHT: u32 = 0x0064;
    pub const GETMOUSEHOVERTIME: u32 = 0x0066;
    pub const GETWHEELSCROLLLINES: u32 = 0x0068;
    pub const GETMENUSHOWDELAY: u32 = 0x006A;
    pub const GETWHEELSCROLLCHARS: u32 = 0x006C;
    pub const GETSHOWIMEUI: u32 = 0x006E;
    pub const GETMOUSESPEED: u32 = 0x0070;
    pub const GETSCREENSAVERRUNNING: u32 = 0x0072;
    pub const GETDESKWALLPAPER: u32 = 0x0073;
    pub const GETAUDIODESCRIPTION: u32 = 0x0074;
    pub const GETSCREENSAVESECURE: u32 = 0x0076;
    pub const GETHUNGAPPTIMEOUT: u32 = 0x0078;
    pub const GETWAITTOKILLTIMEOUT: u32 = 0x007A;
    pub const GETWAITTOKILLSERVICETIMEOUT: u32 = 0x007C;
    pub const GETMOUSEDOCKTHRESHOLD: u32 = 0x007E;
    pub const GETPENDOCKTHRESHOLD: u32 = 0x0080;
    pub const GETWINARRANGING: u32 = 0x0082;
    pub const GETMOUSEDRAGOUTTHRESHOLD: u32 = 0x0084;
    pub const GETPENDRAGOUTTHRESHOLD: u32 = 0x0086;
    pub const GETMOUSESIDEMOVETHRESHOLD: u32 = 0x0088;
    pub const GETPENSIDEMOVETHRESHOLD: u32 = 0x008A;
    pub const GETDRAGFROMMAXIMIZE: u32 = 0x008C;
    pub const GETSNAPSIZING: u32 = 0x008E;
    pub const GETDOCKMOVING: u32 = 0x0090;
    pub const GETLOGICALDPIOVERRIDE: u32 = 0x009E;
    pub const GETACTIVEWINDOWTRACKING: u32 = 0x1000;
    pub const GETMENUANIMATION: u32 = 0x1002;
    pub const GETCOMBOBOXANIMATION: u32 = 0x1004;
    pub const GETLISTBOXSMOOTHSCROLLING: u32 = 0x1006;
    pub const GETGRADIENTCAPTIONS: u32 = 0x1008;
    pub const GETKEYBOARDCUES: u32 = 0x100A;
    pub const GETACTIVEWNDTRKZORDER: u32 = 0x100C;
    pub const GETHOTTRACKING: u32 = 0x100E;
    pub const GETMENUFADE: u32 = 0x1012;
    pub const GETSELECTIONFADE: u32 = 0x1014;
    pub const GETTOOLTIPANIMATION: u32 = 0x1016;
    pub const GETTOOLTIPFADE: u32 = 0x1018;
    pub const GETCURSORSHADOW: u32 = 0x101A;
    pub const GETMOUSESONAR: u32 = 0x101C;
    pub const GETMOUSECLICKLOCK: u32 = 0x101E;
    pub const GETMOUSEVANISH: u32 = 0x1020;
    pub const GETFLATMENU: u32 = 0x1022;
    pub const GETDROPSHADOW: u32 = 0x1024;
    pub const GETBLOCKSENDINPUTRESETS: u32 = 0x1026;
    pub const GETUIEFFECTS: u32 = 0x103E;
    pub const GETDISABLEOVERLAPPEDCONTENT: u32 = 0x1040;
    pub const GETCLIENTAREAANIMATION: u32 = 0x1042;
    pub const GETCLEARTYPE: u32 = 0x1048;
    pub const GETSPEECHRECOGNITION: u32 = 0x104A;
    pub const GETCARETBROWSING: u32 = 0x104C;
    pub const GETTHREADLOCALINPUTSETTINGS: u32 = 0x104E;
    pub const GETSYSTEMLANGUAGEBAR: u32 = 0x1050;
    pub const GETFOREGROUNDLOCKTIMEOUT: u32 = 0x2000;
    pub const GETACTIVEWNDTRKTIMEOUT: u32 = 0x2002;
    pub const GETFOREGROUNDFLASHCOUNT: u32 = 0x2004;
    pub const GETCARETWIDTH: u32 = 0x2006;
    pub const GETMOUSECLICKLOCKTIME: u32 = 0x2008;
    pub const GETFONTSMOOTHINGTYPE: u32 = 0x200A;
    pub const GETFONTSMOOTHINGCONTRAST: u32 = 0x200C;
    pub const GETFOCUSBORDERWIDTH: u32 = 0x200E;
    pub const GETFOCUSBORDERHEIGHT: u32 = 0x2010;
    pub const GETFONTSMOOTHINGORIENTATION: u32 = 0x2012;
    pub const GETMINIMUMHITRADIUS: u32 = 0x2014;
    pub const GETMESSAGEDURATION: u32 = 0x2016;
    pub const GETCONTACTVISUALIZATION: u32 = 0x2018;
    pub const GETGESTUREVISUALIZATION: u32 = 0x201A;
    pub const GETMOUSEWHEELROUTING: u32 = 0x201C;
    pub const GETPENVISUALIZATION: u32 = 0x201E;
    pub const GETPENARBITRATIONTYPE: u32 = 0x2020;
    pub const GETCARETTIMEOUT: u32 = 0x2022;
    pub const GETHANDEDNESS: u32 = 0x2024;
}

/// Windows 11's UI font at 100% scale: Segoe UI, 9 points.
const FONT_HEIGHT: i32 = -12;
const FONT_FACE: &str = "Segoe UI";
/// LOGFONTW (92 bytes) or LOGFONTA (60 bytes) of the UI font.
pub fn ui_font(ansi: bool, weight: i32) -> Vec<u8> {
    let mut out = Vec::with_capacity(92);
    for value in [FONT_HEIGHT, 0, 0, 0, weight] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    // Italic, underline, strike-out, ANSI_CHARSET, default precisions,
    // DEFAULT_QUALITY, DEFAULT_PITCH | FF_DONTCARE.
    out.extend_from_slice(&[0; 8]);
    let face = FONT_FACE.as_bytes();
    if ansi {
        let mut name = [0u8; 32];
        name[..face.len()].copy_from_slice(face);
        out.extend_from_slice(&name);
    } else {
        let mut name = [0u8; 64];
        for (i, &b) in face.iter().enumerate() {
            name[2 * i] = b;
        }
        out.extend_from_slice(&name);
    }
    out
}
fn words(values: &[i32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn sized(bytes: Vec<u8>, also: Option<usize>) -> Parameter {
    Parameter::Struct {
        bytes,
        sized: true,
        also,
    }
}
/// NONCLIENTMETRICSW (504 bytes, 500 before Vista's iPaddedBorderWidth) or
/// NONCLIENTMETRICSA (344 / 340).
pub fn nonclient_metrics(ansi: bool, screen: (i32, i32), work: Rect) -> Vec<u8> {
    let metric = |i| system_metric(i, screen, work);
    let font = ui_font(ansi, 400);
    let size = if ansi { 344 } else { 504 };
    let mut out = words(&[size, 1, metric(2), metric(3), metric(30), CAPTION - 1]);
    out.extend_from_slice(&font);
    out.extend_from_slice(&words(&[22, 21]));
    out.extend_from_slice(&font);
    out.extend_from_slice(&words(&[19, 19]));
    out.extend_from_slice(&font);
    out.extend_from_slice(&font);
    out.extend_from_slice(&font);
    out.extend_from_slice(&words(&[metric(92)]));
    debug_assert_eq!(out.len(), size as usize);
    out
}
fn yes(on: bool) -> Option<Parameter> {
    Some(Parameter::Value(i32::from(on)))
}
/// The answer for `action` (`ansi`: the A function; `pointer_bytes`: 8 for
/// a 64-bit caller, 4 for a 32-bit one).
pub fn system_parameter(
    action: u32,
    ansi: bool,
    pointer_bytes: usize,
    screen: (i32, i32),
    work: Rect,
) -> Option<Parameter> {
    use spi::*;
    let value = |v: i32| Some(Parameter::Value(v));
    match action {
        GETBEEP
        | GETICONTITLEWRAP
        | GETFASTTASKSWITCH
        | GETDRAGFULLWINDOWS
        | GETFONTSMOOTHING
        | GETSHOWIMEUI
        | GETWINARRANGING
        | GETDRAGFROMMAXIMIZE
        | GETSNAPSIZING
        | GETDOCKMOVING
        | GETMENUANIMATION
        | GETCOMBOBOXANIMATION
        | GETLISTBOXSMOOTHSCROLLING
        | GETGRADIENTCAPTIONS
        | GETHOTTRACKING
        | GETMENUFADE
        | GETSELECTIONFADE
        | GETTOOLTIPANIMATION
        | GETTOOLTIPFADE
        | GETCURSORSHADOW
        | GETMOUSEVANISH
        | GETFLATMENU
        | GETDROPSHADOW
        | GETUIEFFECTS
        | GETCLIENTAREAANIMATION
        | GETCLEARTYPE
        | GETCONTACTVISUALIZATION
        | GETGESTUREVISUALIZATION => yes(true),
        GETSCREENSAVEACTIVE
        | GETMENUDROPALIGNMENT
        | GETSHOWSOUNDS
        | GETKEYBOARDPREF
        | GETSCREENREADER
        | GETLOWPOWERACTIVE
        | GETPOWEROFFACTIVE
        | GETSNAPTODEFBUTTON
        | GETSCREENSAVERRUNNING
        | GETSCREENSAVESECURE
        | GETACTIVEWINDOWTRACKING
        | GETKEYBOARDCUES
        | GETACTIVEWNDTRKZORDER
        | GETMOUSESONAR
        | GETMOUSECLICKLOCK
        | GETBLOCKSENDINPUTRESETS
        | GETDISABLEOVERLAPPEDCONTENT
        | GETSPEECHRECOGNITION
        | GETCARETBROWSING
        | GETTHREADLOCALINPUTSETTINGS
        | GETSYSTEMLANGUAGEBAR => yes(false),
        GETBORDER
        | GETKEYBOARDDELAY
        | GETCARETWIDTH
        | GETFOCUSBORDERWIDTH
        | GETFOCUSBORDERHEIGHT
        | GETFONTSMOOTHINGORIENTATION => value(1),
        GETKEYBOARDSPEED => value(31),
        GETSCREENSAVETIMEOUT => value(600),
        GETGRIDGRANULARITY
        | GETMOUSETRAILS
        | GETACTIVEWNDTRKTIMEOUT
        | GETLOGICALDPIOVERRIDE
        | GETPENARBITRATIONTYPE
        | GETPENVISUALIZATION => value(0),
        ICONHORIZONTALSPACING | ICONVERTICALSPACING => value(system_metric(38, screen, work)),
        GETMOUSEHOVERWIDTH | GETMOUSEHOVERHEIGHT => value(4),
        GETMOUSEHOVERTIME | GETMENUSHOWDELAY => value(400),
        GETWHEELSCROLLLINES | GETWHEELSCROLLCHARS => value(3),
        GETMOUSESPEED => value(10),
        GETHUNGAPPTIMEOUT => value(5000),
        GETWAITTOKILLTIMEOUT => value(5000),
        GETWAITTOKILLSERVICETIMEOUT => value(5000),
        GETMOUSEDOCKTHRESHOLD | GETPENDOCKTHRESHOLD => value(1),
        GETMOUSEDRAGOUTTHRESHOLD | GETPENDRAGOUTTHRESHOLD => value(1),
        GETMOUSESIDEMOVETHRESHOLD | GETPENSIDEMOVETHRESHOLD => value(20),
        GETFOREGROUNDLOCKTIMEOUT => value(200_000),
        GETFOREGROUNDFLASHCOUNT => value(7),
        GETMOUSECLICKLOCKTIME => value(1200),
        // FE_FONTSMOOTHINGCLEARTYPE, at Windows' default contrast.
        GETFONTSMOOTHINGTYPE => value(2),
        GETFONTSMOOTHINGCONTRAST => value(1200),
        GETMINIMUMHITRADIUS => value(0),
        GETMESSAGEDURATION => value(5),
        GETMOUSEWHEELROUTING => value(2),
        GETCARETTIMEOUT => value(5000),
        // HANDEDNESS_RIGHT.
        GETHANDEDNESS => value(1),
        // Acceleration thresholds 6 and 10, acceleration on.
        GETMOUSE => Some(Parameter::Struct {
            bytes: words(&[6, 10, 1]),
            sized: false,
            also: None,
        }),
        GETWORKAREA => Some(Parameter::Struct {
            bytes: words(&[work.left, work.top, work.right, work.bottom]),
            sized: false,
            also: None,
        }),
        GETICONTITLELOGFONT => Some(Parameter::Struct {
            bytes: ui_font(ansi, 400),
            sized: false,
            also: None,
        }),
        GETNONCLIENTMETRICS => Some(sized(
            nonclient_metrics(ansi, screen, work),
            Some(if ansi { 340 } else { 500 }),
        )),
        // ICONMETRICS: spacing, title wrapping, the icon title font.
        GETICONMETRICS => {
            let spacing = system_metric(38, screen, work);
            let mut bytes = words(&[if ansi { 76 } else { 108 }, spacing, spacing, 1]);
            bytes.extend_from_slice(&ui_font(ansi, 400));
            Some(sized(bytes, None))
        }
        // MINIMIZEDMETRICS: width, gaps, ARW_BOTTOMLEFT | ARW_RIGHT.
        GETMINIMIZEDMETRICS => Some(sized(words(&[20, 160, 0, 0, 8]), None)),
        // ANIMATIONINFO: minimize/restore animation on.
        GETANIMATION => Some(sized(words(&[8, 1]), None)),
        // HIGHCONTRAST: available, hot key available, off; no scheme.
        GETHIGHCONTRAST => {
            let mut bytes = words(&[(8 + pointer_bytes) as i32, 0x42]);
            bytes.resize(8 + pointer_bytes, 0);
            Some(sized(bytes, None))
        }
        // The accessibility features, available and off, as a new
        // account has them.
        GETSTICKYKEYS => Some(sized(words(&[8, 0x1fa]), None)),
        GETTOGGLEKEYS => Some(sized(words(&[8, 0x3a]), None)),
        GETFILTERKEYS => Some(sized(words(&[24, 0x7e, 1000, 500, 500, 0]), None)),
        GETMOUSEKEYS => Some(sized(words(&[28, 0x3a, 80, 300, 0, 0, 0]), None)),
        GETACCESSTIMEOUT => Some(sized(words(&[12, 2, 300_000]), None)),
        GETAUDIODESCRIPTION => Some(sized(words(&[12, 0, 0x0409]), None)),
        GETSOUNDSENTRY => {
            let mut bytes = words(&[(40 + pointer_bytes) as i32, 2, 0, 0, 0, 0, 0, 0, 0, 0]);
            bytes.resize(if pointer_bytes == 8 { 56 } else { 48 }, 0);
            let n = bytes.len() as i32;
            bytes[..4].copy_from_slice(&n.to_le_bytes());
            Some(sized(bytes, None))
        }
        GETDESKWALLPAPER => Some(Parameter::Text(
            "C:\\Windows\\Web\\Wallpaper\\Windows\\img0.jpg",
        )),
        // SPI_SET*: the setter of each action above.
        a if is_set(a) => Some(Parameter::Set),
        _ => None,
    }
}
/// SPI_SET actions: in the 0x1000 and 0x2000 ranges each getter's odd
/// successor; below that, Windows' own list.
fn is_set(action: u32) -> bool {
    match action {
        0x1000..=0x2fff => action & 1 == 1,
        0x02 | 0x04 | 0x06 | 0x0B | 0x0F | 0x11 | 0x13 | 0x14 | 0x15 | 0x17 | 0x1A | 0x1C
        | 0x1D | 0x1E | 0x20 | 0x22 | 0x24 | 0x25 | 0x2A | 0x2C | 0x2E | 0x2F | 0x31 | 0x33
        | 0x35 | 0x37 | 0x39 | 0x3B | 0x3D | 0x41 | 0x43 | 0x45 | 0x47 | 0x49 | 0x4B | 0x51
        | 0x52 | 0x55 | 0x56 | 0x57 | 0x5B | 0x5D | 0x60 | 0x61 | 0x63 | 0x65 | 0x67 | 0x69
        | 0x6B | 0x6D | 0x6F | 0x71 | 0x75 | 0x77 | 0x79 | 0x7B | 0x7D | 0x7F | 0x81 | 0x83
        | 0x85 | 0x87 | 0x89 | 0x8B | 0x8D | 0x8F | 0x91 | 0x9F => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SCREEN: (i32, i32) = (1280, 720);
    fn work() -> Rect {
        Rect::new(0, 0, 1280, 672)
    }
    #[test]
    fn answers_have_windows_layouts_and_defaults() {
        let get = |a, ansi| system_parameter(a, ansi, 8, SCREEN, work());
        assert_eq!(get(spi::GETSCREENREADER, false), Some(Parameter::Value(0)));
        assert_eq!(
            get(spi::GETWHEELSCROLLLINES, true),
            Some(Parameter::Value(3))
        );
        let Some(Parameter::Struct { bytes, sized, also }) = get(spi::GETNONCLIENTMETRICS, false)
        else {
            panic!("metrics");
        };
        assert!(sized);
        assert_eq!((bytes.len(), also), (504, Some(500)));
        assert_eq!(&bytes[..4], &504u32.to_le_bytes());
        // iScrollWidth, then lfCaptionFont's face at 24 + 28.
        assert_eq!(
            i32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            crate::metrics::SCROLL
        );
        assert_eq!(&bytes[52..56], &[b'S', 0, b'e', 0]);
        let Some(Parameter::Struct { bytes, .. }) = get(spi::GETNONCLIENTMETRICS, true) else {
            panic!("metrics A");
        };
        assert_eq!(bytes.len(), 344);
        assert_eq!(&bytes[52..60], b"Segoe UI");
        let Some(Parameter::Struct { bytes, sized, .. }) = get(spi::GETWORKAREA, false) else {
            panic!("work area");
        };
        assert!(!sized);
        assert_eq!(bytes, words(&[0, 0, 1280, 672]));
        let Some(Parameter::Struct { bytes, .. }) = get(spi::GETHIGHCONTRAST, false) else {
            panic!("contrast");
        };
        assert_eq!(bytes.len(), 16);
        let Some(Parameter::Struct { bytes, .. }) =
            system_parameter(spi::GETHIGHCONTRAST, false, 4, SCREEN, work())
        else {
            panic!("contrast 32");
        };
        assert_eq!(bytes.len(), 12);
        let Some(Parameter::Struct { bytes, .. }) = get(spi::GETSOUNDSENTRY, false) else {
            panic!("sound sentry");
        };
        assert_eq!(bytes.len(), 56);
        let Some(Parameter::Struct { bytes, .. }) =
            system_parameter(spi::GETSOUNDSENTRY, false, 4, SCREEN, work())
        else {
            panic!("sound sentry 32");
        };
        assert_eq!(bytes.len(), 48);
        assert_eq!(
            get(spi::GETICONTITLELOGFONT, false).map(|p| match p {
                Parameter::Struct { bytes, .. } => bytes.len(),
                _ => 0,
            }),
            Some(92)
        );
        assert_eq!(get(0x1043, false), Some(Parameter::Set));
        assert_eq!(get(0x0047, false), Some(Parameter::Set));
        assert_eq!(get(0x0999, false), None);
    }
}

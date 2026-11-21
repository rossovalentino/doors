//! System metrics, system colors and non-client geometry. Top-level
//! captions belong to the Door window server (its 32 DIP title bar is the
//! non-client top inset of every captioned window); child-window borders,
//! client edges and scroll bars are drawn by user32 inside the parent.
use crate::Rect;
use crate::msg::{ws, ws_ex};

/// Server title-bar height at 100% (`door_graphics_core` TITLE_HEIGHT).
pub const CAPTION: i32 = 32;
pub const MENU_HEIGHT: i32 = 20;
pub const SCROLL: i32 = 17;
pub const BORDER: i32 = 1;
pub const EDGE: i32 = 2;

/// Invisible frame of a captioned top-level window on each side but the
/// top: the Win32 window rectangle extends this far beyond the native
/// window (SM_CXFRAME + SM_CXPADDEDBORDER for sizable frames, the fixed
/// dialog frame otherwise).
pub fn frame(style: u32) -> i32 {
    if style & ws::THICKFRAME != 0 { 8 } else { 3 }
}

/// Non-client insets (left, top, right, bottom) of a window.
pub fn insets(style: u32, ex_style: u32, has_menu: bool, child: bool) -> Rect {
    let caption = style & ws::CAPTION == ws::CAPTION;
    let mut r = if !child && caption {
        // The server draws the caption; like the invisible resize borders
        // of Windows 10/11, the frame lies outside the native window.
        let b = frame(style);
        Rect::new(b, CAPTION, b, b)
    } else {
        let mut edge = 0;
        if ex_style & ws_ex::CLIENTEDGE != 0 {
            edge += EDGE;
        }
        if ex_style & ws_ex::STATICEDGE != 0 {
            edge += BORDER;
        }
        if style & (ws::BORDER | ws::DLGFRAME) != 0 || ex_style & ws_ex::DLGMODALFRAME != 0 {
            edge += BORDER;
        }
        Rect::new(edge, edge, edge, edge)
    };
    if has_menu && !child {
        r.top += MENU_HEIGHT;
    }
    if style & ws::VSCROLL != 0 {
        r.right += SCROLL;
    }
    if style & ws::HSCROLL != 0 {
        r.bottom += SCROLL;
    }
    r
}

/// The client rectangle (relative to the window's top-left).
pub fn client_rect(width: i32, height: i32, insets: Rect) -> Rect {
    let r = Rect::new(
        insets.left,
        insets.top,
        width - insets.right,
        height - insets.bottom,
    );
    if r.right < r.left || r.bottom < r.top {
        Rect::new(r.left, r.top, r.left.max(r.right), r.top.max(r.bottom))
    } else {
        r
    }
}

/// GetSystemMetrics for a `screen` of logical pixels and its work area.
pub fn system_metric(index: i32, screen: (i32, i32), work: Rect) -> i32 {
    match index {
        0 | 16 | 61 | 78 => screen.0,
        1 => screen.1,
        17 => work.height() - CAPTION,
        62 => work.height(),
        76 | 77 => 0,
        79 => screen.1,
        2 | 3 | 20 | 21 | 9 | 10 => SCROLL,
        4 => CAPTION,
        5 | 6 => BORDER,
        7 | 8 => 3,
        11 | 12 => 32,
        13 | 14 => 32,
        15 => MENU_HEIGHT,
        19 | 75 => 1,
        43 => 3,
        23 | 22 | 18 | 40 => 0,
        28 => 136,
        29 => 39,
        30 | 31 => 46,
        32 | 33 => 4,
        34 => 136,
        35 => 39,
        36 | 37 => 4,
        38 | 39 => 75,
        45 | 46 => EDGE,
        49 | 50 => 16,
        51 => 22,
        59 => screen.0 + 12,
        60 => screen.1 + 12,
        71 | 72 => 15,
        80 => 1,
        81 => 1,
        83 | 84 => 1,
        92 => 4,
        _ => 0,
    }
}

/// GetSysColor: the Windows 11 light defaults (COLORREF).
pub const SYS_COLORS: [u32; 31] = [
    0x00c8_c8c8,
    0x0000_0000,
    0x00d1_b499,
    0x00db_cdbf,
    0x00f0_f0f0,
    0x00ff_ffff,
    0x0064_6464,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x00b4_b4b4,
    0x00fc_f7f4,
    0x00ab_abab,
    0x00d7_7800,
    0x00ff_ffff,
    0x00f0_f0f0,
    0x00a0_a0a0,
    0x006d_6d6d,
    0x0000_0000,
    0x0000_0000,
    0x00ff_ffff,
    0x0069_6969,
    0x00e3_e3e3,
    0x0000_0000,
    0x00e1_ffff,
    0x00b5_b5b5,
    0x00cc_6600,
    0x00ea_d1b9,
    0x00f2_e4d7,
    0x00ff_9933,
    0x00f0_f0f0,
];
pub fn sys_color(index: i32) -> Option<u32> {
    usize::try_from(index)
        .ok()
        .and_then(|i| SYS_COLORS.get(i).copied())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn insets_for_top_level_children_and_scrollbars() {
        let top = insets(ws::OVERLAPPEDWINDOW, 0, true, false);
        assert_eq!(top, Rect::new(8, CAPTION + MENU_HEIGHT, 8, 8));
        let c = client_rect(480, 320, top);
        assert_eq!(c, Rect::new(8, 52, 472, 312));
        assert!(c.width() < 480 && c.height() < 320);
        let dialog = insets(ws::POPUP | ws::CAPTION | ws::SYSMENU, 0, false, false);
        assert_eq!(dialog, Rect::new(3, CAPTION, 3, 3));
        let edit = insets(ws::CHILD | ws::VSCROLL, ws_ex::CLIENTEDGE, false, true);
        assert_eq!(edit, Rect::new(2, 2, 2 + SCROLL, 2));
        assert_eq!(client_rect(240, 96, edit), Rect::new(2, 2, 221, 94));
        assert_eq!(insets(ws::POPUP, 0, false, false), Rect::EMPTY);
        assert_eq!(
            client_rect(3, 3, Rect::new(2, 2, 2, 2)),
            Rect::new(2, 2, 2, 2)
        );
    }
    #[test]
    fn metrics_and_colors() {
        let work = Rect::new(0, 0, 1280, 672);
        assert_eq!(system_metric(0, (1280, 720), work), 1280);
        assert_eq!(system_metric(1, (1280, 720), work), 720);
        assert_eq!(system_metric(4, (1280, 720), work), CAPTION);
        assert_eq!(system_metric(2, (1280, 720), work), SCROLL);
        assert_eq!(system_metric(9999, (1280, 720), work), 0);
        assert_eq!(sys_color(5), Some(0x00ff_ffff));
        assert_eq!(sys_color(13), Some(0x00d7_7800));
        assert_eq!(sys_color(31), None);
        assert_eq!(sys_color(-1), None);
    }
}

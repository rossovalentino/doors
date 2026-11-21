//! Windows 11 Settings colors (WinUI theme resources resolved over the Mica
//! window background). BGRA byte order. Dark values are chosen to pass the
//! shared `View` palette unchanged: surfaces stay below its remapped range
//! and secondary text avoids exact greys, so every role is exact.
#[derive(Clone, Copy)]
pub(super) struct Theme {
    pub dark: bool,
    /// Mica window background.
    pub bg: [u8; 4],
    pub card: [u8; 4],
    pub card_hover: [u8; 4],
    pub card_pressed: [u8; 4],
    pub card_border: [u8; 4],
    pub divider: [u8; 4],
    pub rail_selected: [u8; 4],
    pub rail_hover: [u8; 4],
    pub rail_pressed: [u8; 4],
    pub text: [u8; 4],
    pub secondary: [u8; 4],
    pub disabled: [u8; 4],
    pub accent: [u8; 4],
    pub accent_hover: [u8; 4],
    pub on_accent: [u8; 4],
    /// Combo boxes, buttons and the title search box.
    pub control: [u8; 4],
    pub control_hover: [u8; 4],
    pub control_pressed: [u8; 4],
    pub control_border: [u8; 4],
    pub control_bottom: [u8; 4],
    pub control_disabled: [u8; 4],
    /// Toggle switch off stroke/knob.
    pub strong: [u8; 4],
    pub strong_disabled: [u8; 4],
    pub focus: [u8; 4],
    pub popup: [u8; 4],
    pub popup_border: [u8; 4],
    pub popup_selected: [u8; 4],
    pub scrollbar: [u8; 4],
}
pub(super) const LIGHT: Theme = Theme {
    dark: false,
    bg: door_workspace::palette::SETTINGS_BG.0,
    card: [253, 252, 251, 255],
    card_hover: [249, 248, 246, 255],
    card_pressed: [246, 245, 243, 255],
    card_border: [233, 230, 228, 255],
    divider: [235, 234, 233, 255],
    rail_selected: [237, 233, 230, 255],
    rail_hover: [242, 238, 235, 255],
    rail_pressed: [239, 235, 232, 255],
    text: [26, 26, 26, 255],
    secondary: [96, 96, 96, 255],
    disabled: [160, 160, 160, 255],
    accent: [203, 103, 0, 255],
    accent_hover: [212, 118, 25, 255],
    on_accent: [255, 255, 255, 255],
    control: door_workspace::palette::SETTINGS_CONTROL.0,
    control_hover: [249, 249, 249, 255],
    control_pressed: [245, 245, 245, 255],
    control_border: [229, 229, 229, 255],
    control_bottom: [205, 205, 205, 255],
    control_disabled: [245, 245, 245, 255],
    strong: [128, 128, 128, 255],
    strong_disabled: [196, 196, 196, 255],
    focus: [26, 26, 26, 255],
    popup: [252, 252, 252, 255],
    popup_border: [220, 220, 220, 255],
    popup_selected: [240, 240, 240, 255],
    scrollbar: [138, 138, 138, 255],
};
pub(super) const DARK: Theme = Theme {
    dark: true,
    bg: door_workspace::palette::SETTINGS_BG.1,
    card: [47, 44, 43, 255],
    card_hover: [54, 51, 50, 255],
    card_pressed: [43, 40, 39, 255],
    card_border: [30, 27, 26, 255],
    divider: [32, 29, 28, 255],
    rail_selected: [52, 48, 46, 255],
    rail_hover: [45, 41, 40, 255],
    rail_pressed: [41, 37, 36, 255],
    text: [255, 255, 255, 255],
    secondary: [207, 206, 206, 255],
    disabled: [119, 118, 118, 255],
    accent: [255, 205, 96, 255],
    accent_hover: [240, 192, 88, 255],
    on_accent: [1, 0, 0, 255],
    control: door_workspace::palette::SETTINGS_CONTROL.1,
    control_hover: [65, 62, 61, 255],
    control_pressed: [52, 49, 48, 255],
    control_border: [70, 67, 66, 255],
    control_bottom: [82, 79, 78, 255],
    control_disabled: [50, 47, 46, 255],
    strong: [166, 165, 165, 255],
    strong_disabled: [86, 85, 85, 255],
    focus: [255, 255, 255, 255],
    popup: [48, 45, 44, 255],
    popup_border: [24, 22, 21, 255],
    popup_selected: [62, 59, 58, 255],
    scrollbar: [155, 154, 154, 255],
};
pub(super) fn theme(dark: bool) -> Theme {
    if dark { DARK } else { LIGHT }
}
/// `t` over Mica: the page background is cleared so the compositor's Mica
/// shows, and each fill becomes the white or black overlay that resolves to
/// it over `t.bg` (premultiplied BGRA), as WinUI's fills are (a dark card
/// is white at 5 %). Text, accent, strokes and popups stay opaque.
pub(super) fn mica(t: Theme) -> Theme {
    let base = t.bg;
    let over = |fill: [u8; 4]| overlay(fill, base);
    Theme {
        bg: [0, 0, 0, 0],
        card: over(t.card),
        card_hover: over(t.card_hover),
        card_pressed: over(t.card_pressed),
        card_border: over(t.card_border),
        divider: over(t.divider),
        rail_selected: over(t.rail_selected),
        rail_hover: over(t.rail_hover),
        rail_pressed: over(t.rail_pressed),
        control: over(t.control),
        control_hover: over(t.control_hover),
        control_pressed: over(t.control_pressed),
        control_disabled: over(t.control_disabled),
        ..t
    }
}
use door_workspace::palette::mica_overlay as overlay;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mica_fills_resolve_back_to_the_opaque_theme() {
        // Over the base each overlay gives back the calibrated colour
        // (within rounding), and a dark card is WinUI's white at 5 %.
        for t in [DARK, LIGHT] {
            let m = mica(t);
            assert_eq!(m.bg, [0, 0, 0, 0]);
            for (fill, over) in [
                (t.card, m.card),
                (t.card_hover, m.card_hover),
                (t.rail_selected, m.rail_selected),
                (t.control, m.control),
            ] {
                for c in 0..3 {
                    let base = u32::from(t.bg[c]);
                    let resolved = u32::from(over[c]) + base * (255 - u32::from(over[3])) / 255;
                    assert!(
                        resolved.abs_diff(u32::from(fill[c])) <= 4,
                        "{fill:?} vs {resolved}"
                    );
                }
            }
            assert_eq!(m.text, t.text);
            assert_eq!(m.popup, t.popup);
        }
        let card = mica(DARK).card[3];
        assert!((10..=16).contains(&card), "dark card alpha {card}");
    }
}

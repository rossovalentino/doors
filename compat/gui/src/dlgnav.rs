//! Dialog keyboard navigation (`IsDialogMessageW`): Tab/Shift+Tab over
//! WS_TABSTOP controls, arrow keys inside WS_GROUP groups, Enter for the
//! focused or default push button, Escape for IDCANCEL and mnemonics, all
//! honoring each control's WM_GETDLGCODE answer.
use crate::keyboard::vk;
use crate::msg::{dlgc, id, ws};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Control {
    pub id: u32,
    pub style: u32,
    pub visible: bool,
    pub enabled: bool,
    /// WM_GETDLGCODE result.
    pub code: u32,
    pub mnemonic: Option<u16>,
}
impl Control {
    fn usable(&self) -> bool {
        self.visible && self.enabled
    }
    fn tab_stop(&self) -> bool {
        self.usable() && self.style & ws::TABSTOP != 0
    }
    fn push_button(&self) -> bool {
        self.code & (dlgc::DEFPUSHBUTTON | dlgc::UNDEFPUSHBUTTON) != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    /// Not a dialog-navigation message: dispatch normally.
    Pass,
    /// Handled without effect.
    Handled,
    Focus(usize),
    /// Focus and click (radio buttons by arrow, buttons by mnemonic).
    Click(usize),
    /// Send WM_COMMAND(id, BN_CLICKED) to the dialog.
    Command(u32),
}

/// Next (or previous) tab stop after `from`, wrapping.
pub fn next_tab(c: &[Control], from: Option<usize>, back: bool) -> Option<usize> {
    let n = c.len();
    if n == 0 {
        return None;
    }
    let start = from.unwrap_or(if back { 0 } else { n - 1 });
    let mut i = start;
    for _ in 0..n {
        i = if back { (i + n - 1) % n } else { (i + 1) % n };
        if c[i].tab_stop() {
            return Some(i);
        }
    }
    None
}
pub fn first_tab(c: &[Control]) -> Option<usize> {
    c.iter().position(Control::tab_stop)
}
/// Group bounds containing `i`: a group starts at a WS_GROUP control.
fn group(c: &[Control], i: usize) -> (usize, usize) {
    let start = (0..=i)
        .rev()
        .find(|&k| c[k].style & ws::GROUP != 0)
        .unwrap_or(0);
    let end = (i + 1..c.len())
        .find(|&k| c[k].style & ws::GROUP != 0)
        .unwrap_or(c.len());
    (start, end)
}
/// Next usable control inside the group of `from`, wrapping in the group.
pub fn next_in_group(c: &[Control], from: usize, back: bool) -> Option<usize> {
    let (s, e) = group(c, from);
    let n = e - s;
    let mut i = from - s;
    for _ in 0..n {
        i = if back { (i + n - 1) % n } else { (i + 1) % n };
        if c[s + i].usable() {
            return Some(s + i);
        }
    }
    None
}
/// The control a mnemonic selects: a labeled control itself, or the next
/// tab stop after a static label carrying it.
pub fn mnemonic_target(c: &[Control], ch: u16) -> Option<usize> {
    let ch = crate::geom::wide::fold(ch);
    let i = c.iter().position(|x| x.visible && x.mnemonic == Some(ch))?;
    if c[i].code & dlgc::STATIC != 0 {
        return next_tab(c, Some(i), false).filter(|&k| k > i);
    }
    c[i].enabled.then_some(i)
}

pub const MSG_KEYDOWN: u32 = 0x0100;
pub const MSG_CHAR: u32 = 0x0102;
pub const MSG_SYSCHAR: u32 = 0x0106;

/// One message for `IsDialogMessageW`. `default_id` is DM_GETDEFID.
pub fn translate(
    c: &[Control],
    focus: Option<usize>,
    message: u32,
    key: u32,
    shift: bool,
    default_id: u32,
) -> Nav {
    let code = focus.map_or(0, |f| c[f].code);
    match message {
        MSG_KEYDOWN => match key as u8 {
            vk::TAB => {
                if code & (dlgc::WANTTAB | dlgc::WANTALLKEYS) != 0 {
                    return Nav::Pass;
                }
                next_tab(c, focus, shift).map_or(Nav::Handled, Nav::Focus)
            }
            vk::LEFT | vk::UP | vk::RIGHT | vk::DOWN => {
                if code & (dlgc::WANTARROWS | dlgc::WANTALLKEYS) != 0 {
                    return Nav::Pass;
                }
                let Some(f) = focus else {
                    return Nav::Handled;
                };
                let back = matches!(key as u8, vk::LEFT | vk::UP);
                match next_in_group(c, f, back) {
                    Some(n) if c[n].code & dlgc::RADIOBUTTON != 0 => Nav::Click(n),
                    Some(n) => Nav::Focus(n),
                    None => Nav::Handled,
                }
            }
            vk::RETURN => {
                if code & dlgc::WANTALLKEYS != 0 {
                    return Nav::Pass;
                }
                match focus {
                    Some(f) if c[f].push_button() && c[f].enabled => Nav::Command(c[f].id),
                    _ => {
                        let target = if default_id != 0 { default_id } else { id::OK };
                        match c.iter().find(|x| x.id == target) {
                            Some(x) if !x.enabled => Nav::Handled,
                            _ => Nav::Command(target),
                        }
                    }
                }
            }
            vk::ESCAPE => {
                if code & dlgc::WANTALLKEYS != 0 {
                    return Nav::Pass;
                }
                Nav::Command(id::CANCEL)
            }
            _ => Nav::Pass,
        },
        MSG_CHAR | MSG_SYSCHAR => {
            if message == MSG_CHAR && code & (dlgc::WANTCHARS | dlgc::WANTALLKEYS) != 0 {
                return Nav::Pass;
            }
            let Ok(ch) = u16::try_from(key) else {
                return Nav::Pass;
            };
            match mnemonic_target(c, ch) {
                Some(i) if c[i].push_button() || c[i].code & dlgc::BUTTON != 0 => Nav::Click(i),
                Some(i) => Nav::Focus(i),
                None if message == MSG_SYSCHAR => Nav::Pass,
                None => Nav::Handled,
            }
        }
        _ => Nav::Pass,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctl(id: u32, style: u32, code: u32, m: Option<u8>) -> Control {
        Control {
            id,
            style,
            visible: true,
            enabled: true,
            code,
            mnemonic: m.map(u16::from),
        }
    }
    fn dialog() -> [Control; 6] {
        [
            ctl(10, 0, dlgc::STATIC, Some(b'n')),
            ctl(
                11,
                ws::TABSTOP | ws::GROUP,
                dlgc::WANTCHARS | dlgc::HASSETSEL | dlgc::WANTARROWS,
                None,
            ),
            ctl(
                20,
                ws::TABSTOP | ws::GROUP,
                dlgc::RADIOBUTTON | dlgc::BUTTON,
                Some(b'a'),
            ),
            ctl(21, 0, dlgc::RADIOBUTTON | dlgc::BUTTON, Some(b'b')),
            ctl(
                id::OK,
                ws::TABSTOP | ws::GROUP,
                dlgc::DEFPUSHBUTTON | dlgc::BUTTON,
                None,
            ),
            ctl(
                id::CANCEL,
                ws::TABSTOP,
                dlgc::UNDEFPUSHBUTTON | dlgc::BUTTON,
                None,
            ),
        ]
    }
    #[test]
    fn tab_order_wraps_and_skips_disabled() {
        let mut c = dialog();
        assert_eq!(first_tab(&c), Some(1));
        assert_eq!(
            translate(&c, Some(1), MSG_KEYDOWN, u32::from(vk::TAB), false, 0),
            Nav::Focus(2)
        );
        assert_eq!(
            translate(&c, Some(5), MSG_KEYDOWN, u32::from(vk::TAB), false, 0),
            Nav::Focus(1)
        );
        assert_eq!(
            translate(&c, Some(1), MSG_KEYDOWN, u32::from(vk::TAB), true, 0),
            Nav::Focus(5)
        );
        c[2].enabled = false;
        assert_eq!(next_tab(&c, Some(1), false), Some(4));
        c[2].enabled = true;
        c[2].visible = false;
        assert_eq!(next_tab(&c, Some(1), false), Some(4));
    }
    #[test]
    fn arrows_move_within_groups_and_click_radios() {
        let c = dialog();
        assert_eq!(
            translate(&c, Some(2), MSG_KEYDOWN, u32::from(vk::DOWN), false, 0),
            Nav::Click(3)
        );
        assert_eq!(
            translate(&c, Some(3), MSG_KEYDOWN, u32::from(vk::DOWN), false, 0),
            Nav::Click(2),
            "wraps in group"
        );
        assert_eq!(
            translate(&c, Some(1), MSG_KEYDOWN, u32::from(vk::DOWN), false, 0),
            Nav::Pass,
            "edit wants arrows"
        );
        assert_eq!(
            translate(&c, Some(4), MSG_KEYDOWN, u32::from(vk::RIGHT), false, 0),
            Nav::Focus(5)
        );
    }
    #[test]
    fn enter_escape_and_mnemonics() {
        let mut c = dialog();
        assert_eq!(
            translate(&c, Some(1), MSG_KEYDOWN, u32::from(vk::RETURN), false, 0),
            Nav::Command(id::OK)
        );
        assert_eq!(
            translate(&c, Some(5), MSG_KEYDOWN, u32::from(vk::RETURN), false, 0),
            Nav::Command(id::CANCEL)
        );
        assert_eq!(
            translate(&c, Some(1), MSG_KEYDOWN, u32::from(vk::RETURN), false, 77),
            Nav::Command(77)
        );
        assert_eq!(
            translate(&c, Some(1), MSG_KEYDOWN, u32::from(vk::ESCAPE), false, 0),
            Nav::Command(id::CANCEL)
        );
        assert_eq!(
            translate(&c, Some(4), MSG_CHAR, u32::from(b'N'), false, 0),
            Nav::Focus(1),
            "label selects next"
        );
        assert_eq!(
            translate(&c, Some(4), MSG_CHAR, u32::from(b'b'), false, 0),
            Nav::Click(3)
        );
        assert_eq!(
            translate(&c, Some(1), MSG_CHAR, u32::from(b'b'), false, 0),
            Nav::Pass,
            "edit takes chars"
        );
        assert_eq!(
            translate(&c, Some(1), MSG_SYSCHAR, u32::from(b'b'), false, 0),
            Nav::Click(3)
        );
        assert_eq!(
            translate(&c, Some(4), MSG_CHAR, u32::from(b'z'), false, 0),
            Nav::Handled
        );
        c[1].code |= dlgc::WANTALLKEYS;
        assert_eq!(
            translate(&c, Some(1), MSG_KEYDOWN, u32::from(vk::RETURN), false, 0),
            Nav::Pass
        );
        assert_eq!(
            translate(&c, Some(1), MSG_KEYDOWN, u32::from(vk::TAB), false, 0),
            Nav::Pass
        );
        c[4].enabled = false;
        assert_eq!(
            translate(&c, Some(2), MSG_KEYDOWN, u32::from(vk::RETURN), false, 0),
            Nav::Handled
        );
        assert_eq!(translate(&c, None, 0x0200, 0, false, 0), Nav::Pass);
    }
}

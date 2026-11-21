//! Bounded parsers for the Win32 resource formats user32 interprets:
//! `DIALOG`/`DIALOGEX` templates (also passed in memory to the *Indirect
//! APIs), `MENU`/`MENUEX`, `ACCELERATORS` and `STRINGTABLE` blocks.
//!
//! Inputs are untrusted bytes. Every read is bounds-checked, counts, nesting
//! depth and string lengths are limited, and malformed or truncated data is
//! an error rather than a partially built object.
use crate::{Error, Result};
use alloc::vec::Vec;

pub const MAX_ITEMS: usize = 1024;
pub const MAX_DEPTH: usize = 16;
pub const MAX_TEXT: usize = 4096;
pub const MAX_EXTRA: usize = 4096;

/// `sz_Or_Ord`: absent, an ordinal, or a string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameOrId {
    None,
    Id(u16),
    Name(Vec<u16>),
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, at: 0 }
    }
    fn u16(&mut self) -> Result<u16> {
        let v = self.b.get(self.at..self.at + 2).ok_or(Error::Malformed)?;
        self.at += 2;
        Ok(u16::from_le_bytes([v[0], v[1]]))
    }
    fn u32(&mut self) -> Result<u32> {
        let v = self.b.get(self.at..self.at + 4).ok_or(Error::Malformed)?;
        self.at += 4;
        Ok(u32::from_le_bytes([v[0], v[1], v[2], v[3]]))
    }
    fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }
    fn u8(&mut self) -> Result<u8> {
        let v = *self.b.get(self.at).ok_or(Error::Malformed)?;
        self.at += 1;
        Ok(v)
    }
    fn align4(&mut self) {
        self.at = (self.at + 3) & !3;
    }
    fn text(&mut self) -> Result<Vec<u16>> {
        let mut out = Vec::new();
        loop {
            let u = self.u16()?;
            if u == 0 {
                return Ok(out);
            }
            if out.len() >= MAX_TEXT {
                return Err(Error::Malformed);
            }
            out.try_reserve(1).map_err(|_| Error::Quota)?;
            out.push(u);
        }
    }
    fn name_or_id(&mut self) -> Result<NameOrId> {
        match self.u16()? {
            0 => Ok(NameOrId::None),
            0xffff => Ok(NameOrId::Id(self.u16()?)),
            _ => {
                self.at -= 2;
                Ok(NameOrId::Name(self.text()?))
            }
        }
    }
    fn bytes(&mut self, n: usize) -> Result<Vec<u8>> {
        let v = self
            .b
            .get(self.at..self.at.checked_add(n).ok_or(Error::Malformed)?)
            .ok_or(Error::Malformed)?;
        self.at += n;
        Ok(v.to_vec())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogFont {
    pub point_size: u16,
    pub weight: u16,
    pub italic: bool,
    pub charset: u8,
    pub face: Vec<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogItem {
    pub help_id: u32,
    pub ex_style: u32,
    pub style: u32,
    pub x: i16,
    pub y: i16,
    pub cx: i16,
    pub cy: i16,
    pub id: u32,
    pub class: NameOrId,
    pub title: NameOrId,
    pub extra: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogTemplate {
    pub extended: bool,
    pub help_id: u32,
    pub ex_style: u32,
    pub style: u32,
    pub x: i16,
    pub y: i16,
    pub cx: i16,
    pub cy: i16,
    pub menu: NameOrId,
    pub class: NameOrId,
    pub title: Vec<u16>,
    pub font: Option<DialogFont>,
    pub items: Vec<DialogItem>,
}

const DS_SETFONT: u32 = 0x40;
const DS_SHELLFONT: u32 = 0x48;

/// Predefined dialog control classes by ordinal (0x0080 .. 0x0085).
pub fn control_class(ordinal: u16) -> Option<&'static str> {
    Some(match ordinal {
        0x0080 => "Button",
        0x0081 => "Edit",
        0x0082 => "Static",
        0x0083 => "ListBox",
        0x0084 => "ScrollBar",
        0x0085 => "ComboBox",
        _ => return None,
    })
}

impl DialogTemplate {
    pub fn parse(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let first = r.u16()?;
        let second = r.u16()?;
        let extended = first == 1 && second == 0xffff;
        let (help_id, ex_style, style) = if extended {
            let help = r.u32()?;
            let ex = r.u32()?;
            (help, ex, r.u32()?)
        } else {
            let style = u32::from(first) | u32::from(second) << 16;
            (0, r.u32()?, style)
        };
        let count = r.u16()? as usize;
        if count > MAX_ITEMS {
            return Err(Error::Malformed);
        }
        let (x, y, cx, cy) = (r.i16()?, r.i16()?, r.i16()?, r.i16()?);
        let menu = r.name_or_id()?;
        let class = r.name_or_id()?;
        let title = r.text()?;
        let font = if style & DS_SETFONT != 0 || (extended && style & DS_SHELLFONT == DS_SHELLFONT)
        {
            let point_size = r.u16()?;
            let (weight, italic, charset) = if extended {
                (r.u16()?, r.u8()? != 0, r.u8()?)
            } else {
                (400, false, 1)
            };
            Some(DialogFont {
                point_size,
                weight,
                italic,
                charset,
                face: r.text()?,
            })
        } else {
            None
        };
        let mut items = Vec::new();
        items.try_reserve(count).map_err(|_| Error::Quota)?;
        for _ in 0..count {
            r.align4();
            let (help_id, ex_style, style) = if extended {
                let help = r.u32()?;
                let ex = r.u32()?;
                (help, ex, r.u32()?)
            } else {
                let style = r.u32()?;
                (0, r.u32()?, style)
            };
            let (x, y, cx, cy) = (r.i16()?, r.i16()?, r.i16()?, r.i16()?);
            let id = if extended {
                r.u32()?
            } else {
                u32::from(r.u16()?)
            };
            let class = r.name_or_id()?;
            let title = r.name_or_id()?;
            let extra_bytes = r.u16()? as usize;
            if extra_bytes > MAX_EXTRA {
                return Err(Error::Malformed);
            }
            // The size word is followed by exactly that many creation bytes.
            let extra = r.bytes(extra_bytes)?;
            items.push(DialogItem {
                help_id,
                ex_style,
                style,
                x,
                y,
                cx,
                cy,
                id,
                class,
                title,
                extra,
            });
        }
        Ok(Self {
            extended,
            help_id,
            ex_style,
            style,
            x,
            y,
            cx,
            cy,
            menu,
            class,
            title,
            font,
            items,
        })
    }
}

/// One menu template node (both formats). `flags` holds the MF_*/MFT_*
/// type bits and `state` the MFS_* state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuNode {
    pub flags: u32,
    pub state: u32,
    pub id: u32,
    pub text: Vec<u16>,
    pub help_id: u32,
    pub children: Option<Vec<MenuNode>>,
}

const MF_POPUP: u16 = 0x0010;
const MF_END: u16 = 0x0080;

fn count_nodes(nodes: &[MenuNode]) -> usize {
    nodes
        .iter()
        .map(|n| 1 + n.children.as_deref().map_or(0, count_nodes))
        .sum()
}

pub fn parse_menu(b: &[u8]) -> Result<Vec<MenuNode>> {
    let mut r = Reader::new(b);
    let version = r.u16()?;
    let offset = r.u16()?;
    let mut total = 0usize;
    let nodes = match version {
        0 => {
            r.at += offset as usize;
            menu_items(&mut r, 0, &mut total)?
        }
        1 => {
            // MENUEX: wOffset counts from the end of the version word.
            r.at = 2 + offset as usize;
            menuex_items(&mut r, 0, &mut total)?
        }
        _ => return Err(Error::Malformed),
    };
    debug_assert_eq!(count_nodes(&nodes), total);
    Ok(nodes)
}

fn menu_items(r: &mut Reader, depth: usize, total: &mut usize) -> Result<Vec<MenuNode>> {
    if depth >= MAX_DEPTH {
        return Err(Error::Malformed);
    }
    let mut out = Vec::new();
    loop {
        *total += 1;
        if *total > MAX_ITEMS {
            return Err(Error::Malformed);
        }
        let flags = r.u16()?;
        let popup = flags & MF_POPUP != 0;
        let id = if popup { 0 } else { u32::from(r.u16()?) };
        let text = r.text()?;
        let children = if popup {
            Some(menu_items(r, depth + 1, total)?)
        } else {
            None
        };
        // The state bits of MENU items (grayed/disabled/checked) live in
        // the same word; MF_END and MF_POPUP are structure, not state.
        let state = u32::from(flags & 0x0b);
        out.push(MenuNode {
            flags: u32::from(flags & !(MF_END | MF_POPUP | 0x0b)),
            state,
            id,
            text,
            help_id: 0,
            children,
        });
        if flags & MF_END != 0 {
            return Ok(out);
        }
    }
}

fn menuex_items(r: &mut Reader, depth: usize, total: &mut usize) -> Result<Vec<MenuNode>> {
    if depth >= MAX_DEPTH {
        return Err(Error::Malformed);
    }
    let mut out = Vec::new();
    loop {
        *total += 1;
        if *total > MAX_ITEMS {
            return Err(Error::Malformed);
        }
        r.align4();
        let flags = r.u32()?;
        let state = r.u32()?;
        let id = r.u32()?;
        let info = r.u16()?;
        let text = r.text()?;
        let (help_id, children) = if info & 0x01 != 0 {
            r.align4();
            let help = r.u32()?;
            (help, Some(menuex_items(r, depth + 1, total)?))
        } else {
            (0, None)
        };
        out.push(MenuNode {
            flags,
            state,
            id,
            text,
            help_id,
            children,
        });
        if info & 0x80 != 0 {
            return Ok(out);
        }
    }
}

/// One accelerator: `virt` holds FVIRTKEY/FSHIFT/FCONTROL/FALT/FNOINVERT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accel {
    pub virt: u8,
    pub key: u16,
    pub cmd: u16,
}

/// The 8-byte `ACCELTABLEENTRY` records of an RT_ACCELERATOR resource.
pub fn parse_accelerators(b: &[u8]) -> Result<Vec<Accel>> {
    let mut out = Vec::new();
    for chunk in b.chunks(8) {
        if chunk.len() < 6 {
            return Err(Error::Malformed);
        }
        if out.len() >= MAX_ITEMS {
            return Err(Error::Malformed);
        }
        let flags = u16::from_le_bytes([chunk[0], chunk[1]]);
        out.push(Accel {
            virt: (flags & 0x7f) as u8,
            key: u16::from_le_bytes([chunk[2], chunk[3]]),
            cmd: u16::from_le_bytes([chunk[4], chunk[5]]),
        });
        if flags & 0x80 != 0 {
            return Ok(out);
        }
    }
    Err(Error::Malformed)
}

/// The 6-byte application `ACCEL` array (`CreateAcceleratorTableW`).
pub fn parse_accel_array(b: &[u8]) -> Result<Vec<Accel>> {
    if b.is_empty() || !b.len().is_multiple_of(6) || b.len() / 6 > MAX_ITEMS {
        return Err(Error::InvalidParameter);
    }
    Ok(b.chunks_exact(6)
        .map(|c| Accel {
            virt: c[0] & 0x7f,
            key: u16::from_le_bytes([c[2], c[3]]),
            cmd: u16::from_le_bytes([c[4], c[5]]),
        })
        .collect())
}

/// String `index` (0..16) of an RT_STRING block; an empty slot is `None`.
pub fn string_from_block(b: &[u8], index: usize) -> Result<Option<Vec<u16>>> {
    if index >= 16 {
        return Err(Error::InvalidParameter);
    }
    let mut r = Reader::new(b);
    for i in 0..16 {
        let len = r.u16()? as usize;
        if len > MAX_TEXT {
            return Err(Error::Malformed);
        }
        if i == index {
            if len == 0 {
                return Ok(None);
            }
            let raw = r.bytes(len * 2)?;
            return Ok(Some(
                raw.chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect(),
            ));
        }
        r.at = r.at.checked_add(len * 2).ok_or(Error::Malformed)?;
        if r.at > b.len() {
            return Err(Error::Malformed);
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    fn w(s: &str) -> Vec<u8> {
        s.encode_utf16()
            .chain(core::iter::once(0))
            .flat_map(|u| u.to_le_bytes())
            .collect()
    }
    fn pad(b: &mut Vec<u8>) {
        while !b.len().is_multiple_of(4) {
            b.push(0);
        }
    }
    /// The IDD_ABOUT DIALOGEX of the editor fixture, as rc.exe emits it.
    fn about() -> Vec<u8> {
        let mut b = Vec::new();
        for v in [1u16, 0xffff] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        let style: u32 = 0x80 | 0x48 | 0x8000_0000 | 0x00c0_0000 | 0x0008_0000;
        for v in [0u32, 0, style] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&3u16.to_le_bytes());
        for v in [0i16, 0, 220, 72] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&[0, 0, 0, 0]);
        b.extend(w("About Doors Editor"));
        b.extend_from_slice(&9u16.to_le_bytes());
        b.extend_from_slice(&400u16.to_le_bytes());
        b.extend_from_slice(&[0, 1]);
        b.extend(w("Segoe UI"));
        for (id, class, text) in [
            (1001u32, 0x82u16, "Doors Editor"),
            (0xffff_ffff, 0x82, "\u{3bb} \u{9580}"),
            (1, 0x80, "OK"),
        ] {
            pad(&mut b);
            for v in [0u32, 0, 0x5000_0000] {
                b.extend_from_slice(&v.to_le_bytes());
            }
            for v in [10i16, 10, 200, 10] {
                b.extend_from_slice(&v.to_le_bytes());
            }
            b.extend_from_slice(&id.to_le_bytes());
            b.extend_from_slice(&[0xff, 0xff]);
            b.extend_from_slice(&class.to_le_bytes());
            b.extend(w(text));
            b.extend_from_slice(&0u16.to_le_bytes());
        }
        b
    }
    #[test]
    fn dialogex_parses_fonts_items_and_classes() {
        let d = DialogTemplate::parse(&about()).unwrap();
        assert!(d.extended);
        assert_eq!((d.cx, d.cy, d.items.len()), (220, 72, 3));
        assert_eq!(
            d.title,
            "About Doors Editor".encode_utf16().collect::<Vec<_>>()
        );
        let f = d.font.as_ref().unwrap();
        assert_eq!((f.point_size, f.weight, f.charset), (9, 400, 1));
        assert_eq!(d.items[2].id, 1);
        assert_eq!(d.items[1].id, 0xffff_ffff);
        assert_eq!(d.items[0].class, NameOrId::Id(0x82));
        assert_eq!(control_class(0x82), Some("Static"));
        assert_eq!(control_class(0x86), None);
        assert_eq!(
            d.items[2].title,
            NameOrId::Name("OK".encode_utf16().collect())
        );
    }
    #[test]
    fn classic_dialog_with_named_class_and_extra_data() {
        let mut b = Vec::new();
        b.extend_from_slice(&(0x80c0_0000u32 | 0x40).to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        for v in [1i16, 2, 3, 4] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&[0xff, 0xff, 7, 0]);
        b.extend(w("MyClass"));
        b.extend(w("T"));
        b.extend_from_slice(&8u16.to_le_bytes());
        b.extend(w("MS Shell Dlg"));
        pad(&mut b);
        b.extend_from_slice(&0x5001_0000u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        for v in [0i16, 0, 10, 10] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&5u16.to_le_bytes());
        b.extend(w("Edit"));
        b.extend_from_slice(&[0, 0]);
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&[9, 8]);
        let d = DialogTemplate::parse(&b).unwrap();
        assert!(!d.extended);
        assert_eq!(d.menu, NameOrId::Id(7));
        assert_eq!(d.class, NameOrId::Name("MyClass".encode_utf16().collect()));
        assert_eq!(d.items[0].id, 5);
        assert_eq!(d.items[0].title, NameOrId::None);
        assert_eq!(d.items[0].extra, [9, 8]);
        assert_eq!(d.font.unwrap().point_size, 8);
    }
    #[test]
    fn hostile_dialogs_fail_cleanly() {
        let good = about();
        for n in 0..good.len() {
            assert!(
                DialogTemplate::parse(&good[..n]).is_err(),
                "truncated at {n}"
            );
        }
        let mut many = good.clone();
        many[16..18].copy_from_slice(&(MAX_ITEMS as u16 + 1).to_le_bytes());
        assert!(DialogTemplate::parse(&many).is_err());
        // An unterminated title runs off the end.
        let mut open = good[..34].to_vec();
        open.extend(core::iter::repeat_n(0x41, 64));
        assert!(DialogTemplate::parse(&open).is_err());
    }
    fn editor_menu() -> Vec<u8> {
        let mut b = alloc::vec![0, 0, 0, 0];
        let item = |b: &mut Vec<u8>, flags: u16, id: Option<u16>, text: &str| {
            b.extend_from_slice(&flags.to_le_bytes());
            if let Some(id) = id {
                b.extend_from_slice(&id.to_le_bytes());
            }
            b.extend(w(text));
        };
        item(&mut b, 0x10, None, "&File");
        item(&mut b, 0, Some(40001), "&New\tCtrl+N");
        item(&mut b, 0, Some(0), "");
        item(&mut b, 0x80, Some(40005), "E&xit");
        item(&mut b, 0x90, None, "&Help");
        item(&mut b, 0x80 | 0x01, Some(40020), "&About");
        b
    }
    #[test]
    fn menus_in_both_formats() {
        let m = parse_menu(&editor_menu()).unwrap();
        assert_eq!(m.len(), 2);
        let file = m[0].children.as_ref().unwrap();
        assert_eq!(file.len(), 3);
        assert_eq!(file[0].id, 40001);
        assert_eq!(file[1].text, Vec::<u16>::new(), "separator");
        let help = m[1].children.as_ref().unwrap();
        assert_eq!(help[0].state, 1, "grayed");
        // MENUEX with a popup and a help id.
        let mut b = Vec::new();
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&4u16.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        for v in [0u32, 0, 0] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&0x81u16.to_le_bytes());
        b.extend(w("&View"));
        pad(&mut b);
        b.extend_from_slice(&77u32.to_le_bytes());
        for v in [0u32, 8, 9] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&0x80u16.to_le_bytes());
        b.extend(w("Zoom"));
        let m = parse_menu(&b).unwrap();
        assert_eq!(m[0].help_id, 77);
        let kids = m[0].children.as_ref().unwrap();
        assert_eq!((kids[0].id, kids[0].state), (9, 8));
    }
    #[test]
    fn hostile_menus_fail_cleanly() {
        let good = editor_menu();
        for n in 0..good.len() {
            assert!(parse_menu(&good[..n]).is_err(), "truncated at {n}");
        }
        // Unbounded nesting: popups that never end.
        let mut deep = alloc::vec![0, 0, 0, 0];
        for _ in 0..(MAX_DEPTH + 2) {
            deep.extend_from_slice(&0x10u16.to_le_bytes());
            deep.extend(w("x"));
        }
        assert!(parse_menu(&deep).is_err());
        assert!(parse_menu(&[2, 0, 0, 0]).is_err(), "unknown version");
    }
    #[test]
    fn accelerators_and_strings() {
        let mut b = Vec::new();
        for (flags, key, cmd) in [
            (0x09u16, b'N' as u16, 40001u16),
            (0x0d | 0x80, b'S' as u16, 40004),
        ] {
            b.extend_from_slice(&flags.to_le_bytes());
            b.extend_from_slice(&key.to_le_bytes());
            b.extend_from_slice(&cmd.to_le_bytes());
            b.extend_from_slice(&0u16.to_le_bytes());
        }
        let a = parse_accelerators(&b).unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(
            a[1],
            Accel {
                virt: 0x0d,
                key: b'S' as u16,
                cmd: 40004
            }
        );
        assert!(parse_accelerators(&b[..8]).is_err(), "no terminating entry");
        assert!(parse_accelerators(&b[..4]).is_err());
        let arr = parse_accel_array(&[1, 0, 0x74, 0, 101, 0]).unwrap();
        assert_eq!(
            arr[0],
            Accel {
                virt: 1,
                key: 0x74,
                cmd: 101
            }
        );
        assert!(parse_accel_array(&[1, 0, 0]).is_err());
        // Strings 103..105 live in block 7 (103 / 16 + 1) at indexes 7..9.
        let mut block = Vec::new();
        for i in 0..16u16 {
            let s: Vec<u16> = if i == 7 {
                "Doors Editor".encode_utf16().collect()
            } else {
                Vec::new()
            };
            block.extend_from_slice(&(s.len() as u16).to_le_bytes());
            for u in s {
                block.extend_from_slice(&u.to_le_bytes());
            }
        }
        assert_eq!(
            string_from_block(&block, 7).unwrap().unwrap(),
            "Doors Editor".encode_utf16().collect::<Vec<_>>()
        );
        assert_eq!(string_from_block(&block, 8), Ok(None));
        assert!(string_from_block(&block, 16).is_err());
        assert!(string_from_block(&block[..20], 9).is_err());
        let mut lying = block.clone();
        lying[0..2].copy_from_slice(&5000u16.to_le_bytes());
        assert!(string_from_block(&lying, 3).is_err());
    }
}

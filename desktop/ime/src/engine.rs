//! Engine interface shared by the three input methods.
extern crate alloc;
use alloc::{string::String, vec::Vec};
use door_abi::graphics as g;
use door_keyboard_layouts::vk;
use door_service_protocol::{BODY_BYTES, text_input as ti};
/// One key as the IME sees it (from an `IME_KEY` record).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyIn {
    pub code: u32,
    pub vk: u8,
    pub flags: u32,
    /// The layout's character for this key (None for control keys).
    pub ch: Option<char>,
}
impl KeyIn {
    pub fn down(&self) -> bool {
        self.flags & g::DOWN != 0
    }
    pub fn shift(&self) -> bool {
        self.flags & g::SHIFT != 0
    }
    /// Ctrl or (left) Alt held: a shortcut, never composition input.
    pub fn chord(&self) -> bool {
        self.flags & g::CONTROL != 0 || self.flags & g::ALT != 0 && self.flags & g::RIGHT_ALT == 0
    }
    pub fn from_key(k: &ti::Key) -> Self {
        Self {
            code: k.code,
            vk: k.vk as u8,
            flags: k.flags,
            ch: k.text().chars().next(),
        }
    }
    /// Test/corpus helper: a key press typing `c` (US positions).
    pub fn typed(c: char) -> Self {
        let upper = c.to_ascii_uppercase();
        let vk = if c.is_ascii_alphanumeric() {
            upper as u8
        } else if c == ' ' {
            vk::SPACE
        } else {
            0
        };
        let shift = if c.is_ascii_uppercase() {
            g::SHIFT | g::LEFT_SHIFT
        } else {
            0
        };
        Self {
            code: 0,
            vk,
            flags: g::DOWN | shift,
            ch: Some(c),
        }
    }
    pub fn special(vk: u8, flags: u32) -> Self {
        Self {
            code: 0,
            vk,
            flags: flags | g::DOWN,
            ch: None,
        }
    }
}
/// A preedit segment: byte range and `ti::STYLE_*`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub start: usize,
    pub end: usize,
    pub style: u8,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preedit {
    pub text: String,
    /// Caret byte offset.
    pub cursor: usize,
    pub segments: Vec<Segment>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub items: Vec<String>,
    pub selected: usize,
    pub page: usize,
    pub pages: usize,
    pub vertical: bool,
}
/// The result of one key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// The key was consumed (otherwise it is forwarded after `commit`).
    pub handled: bool,
    /// Text to commit before anything else.
    pub commit: String,
    /// The composition after this key (empty: none).
    pub preedit: Preedit,
    /// Candidate window page (None: hidden).
    pub candidates: Option<Page>,
}
impl Output {
    pub fn forward() -> Self {
        Self::default()
    }
}
pub trait Engine {
    fn key(&mut self, k: &KeyIn) -> Output;
    /// Cancel the composition (Escape semantics, lock).
    fn cancel(&mut self);
    /// Commit the composition as displayed (focus change, RESET_COMPLETE).
    fn complete(&mut self) -> String;
    /// `ti::MODE_*` conversion mode.
    fn mode(&self) -> u32;
    fn composing(&self) -> bool;
    /// Private contexts: no learning.
    fn set_private(&mut self, _private: bool) {}
}
/// Most candidates on one page (digits 1-9).
pub const PAGE_SIZE: usize = 9;
/// IME_CANDIDATES body bytes before the per-item lengths and strings.
const CANDIDATES_HEADER: usize = 12;
/// A candidate as the window shows it: invalid characters replaced, at most
/// `MAX_CANDIDATE_BYTES` (cut on a grapheme boundary).
pub fn display(s: &str) -> String {
    let mut clean: String = s
        .chars()
        .map(|c| {
            if c.is_control() || c == '\u{feff}' || c == '\u{fffe}' {
                '\u{fffd}'
            } else {
                c
            }
        })
        .collect();
    clean.truncate(ti::grapheme_floor(&clean, ti::MAX_CANDIDATE_BYTES));
    if clean.is_empty() {
        clean.push('\u{fffd}');
    }
    clean
}
/// The page holding a candidate: `[start, end)` of the full list, its index
/// and the page count. Pages take up to `PAGE_SIZE` candidates whose
/// IME_CANDIDATES body fits one request, so every page encodes and the
/// digits always index the candidates the window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub start: usize,
    pub end: usize,
    pub page: usize,
    pub pages: usize,
}
pub fn bounds<'a>(all: impl IntoIterator<Item = &'a str>, selected: usize) -> Bounds {
    let mut b = Bounds {
        start: 0,
        end: 0,
        page: 0,
        pages: 0,
    };
    let (mut start, mut count, mut bytes, mut i) = (0, 0, CANDIDATES_HEADER, 0);
    for s in all {
        let len = 1 + display(s).len();
        if count == PAGE_SIZE || bytes + len > BODY_BYTES {
            if (start..i).contains(&selected) {
                (b.start, b.end, b.page) = (start, i, b.pages);
            }
            b.pages += 1;
            (start, count, bytes) = (i, 0, CANDIDATES_HEADER);
        }
        count += 1;
        bytes += len;
        i += 1;
    }
    if count > 0 {
        if (start..i).contains(&selected) || selected >= i {
            (b.start, b.end, b.page) = (start, i, b.pages);
        }
        b.pages += 1;
    }
    b
}
/// Build the candidate page holding `selected` in `all`.
pub fn page(all: &[String], selected: usize, vertical: bool) -> Option<Page> {
    if all.is_empty() {
        return None;
    }
    let b = bounds(all.iter().map(String::as_str), selected);
    Some(Page {
        items: all[b.start..b.end].iter().map(|s| display(s)).collect(),
        selected: selected.clamp(b.start, b.end - 1) - b.start,
        page: b.page,
        pages: b.pages,
        vertical,
    })
}
/// The protocol header of `page`: page numbers saturate at 255 pages (the
/// u8 wire field) and stay below the count, so the request always encodes.
pub fn wire_page(page: &Page) -> (u8, u8, u8) {
    let pages = page.pages.clamp(1, 255);
    (
        page.selected.min(PAGE_SIZE - 1) as u8,
        page.page.min(pages - 1) as u8,
        pages as u8,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{format, vec};
    #[test]
    fn every_candidate_page_fits_one_request_and_digits_index_it() {
        // Long phrases, control characters and more than 255 pages.
        let mut all: Vec<String> = (0..3000)
            .map(|i| match i % 4 {
                0 => format!("{i}"),
                1 => "長い候補".repeat(5),
                2 => "a\tb\u{7}c".into(),
                _ => "👨\u{200d}👩\u{200d}👧".repeat(4),
            })
            .collect();
        all.push(String::new());
        let mut seen = 0;
        let mut selected = 0;
        let mut pages = 0;
        while selected < all.len() {
            let p = page(&all, selected, true).unwrap();
            let b = bounds(all.iter().map(String::as_str), selected);
            assert_eq!(b.start, selected, "page starts follow each other");
            assert!(!p.items.is_empty() && p.items.len() <= PAGE_SIZE);
            let items: Vec<&str> = p.items.iter().map(String::as_str).collect();
            let (sel, index, count) = wire_page(&p);
            let mut out = [0u8; BODY_BYTES];
            ti::encode_candidates(
                ti::Candidates {
                    serial: 1,
                    selected: sel,
                    page: index,
                    pages: count,
                    flags: ti::CANDIDATES_VERTICAL,
                    items: &items,
                },
                &mut out,
            )
            .unwrap_or_else(|e| panic!("page {} does not encode: {e:?}", p.page));
            // Digit n picks the n-th shown candidate.
            for (n, shown) in p.items.iter().enumerate() {
                assert_eq!(&display(&all[b.start + n]), shown);
            }
            assert_eq!(p.page, pages);
            seen += p.items.len();
            pages += 1;
            selected = b.end;
        }
        assert_eq!(seen, all.len());
        assert_eq!(page(&all, 0, true).unwrap().pages, pages);
        assert!(pages > 255, "saturating wire page numbers exercised");
        assert_eq!(
            page(&vec!["一".into(); 9], 8, false).unwrap().items.len(),
            9
        );
    }
}

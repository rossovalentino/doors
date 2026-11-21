//! Installed input methods (Settings > Time & language > Language &
//! region > Keyboard), the persisted preference word and the taskbar input
//! indicator (ENG/US, 中, あ, 가) as in Windows 11.
use crate::layout::Layout;
use crate::layouts;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ime {
    None,
    /// Microsoft Pinyin (Simplified Chinese).
    Pinyin,
    /// Microsoft IME (Japanese): romaji input, kana–kanji conversion.
    Japanese,
    /// Microsoft IME (Korean): 2-set (Dubeolsik) Hangul.
    Korean,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Method {
    pub id: u8,
    /// BCP 47 language tag; methods sharing a tag form one language.
    pub tag: &'static str,
    /// Language display name (Windows English UI).
    pub language: &'static str,
    /// Keyboard/IME display name.
    pub keyboard: &'static str,
    pub layout: &'static Layout,
    pub ime: Ime,
    /// Taskbar indicator lines ("ENG" over "US").
    pub abbreviation: &'static str,
    pub short: &'static str,
}
#[allow(clippy::too_many_arguments)]
const fn m(
    id: u8,
    tag: &'static str,
    language: &'static str,
    keyboard: &'static str,
    layout: &'static Layout,
    ime: Ime,
    abbreviation: &'static str,
    short: &'static str,
) -> Method {
    Method {
        id,
        tag,
        language,
        keyboard,
        layout,
        ime,
        abbreviation,
        short,
    }
}
pub const METHODS: [Method; 9] = [
    m(
        1,
        "en-US",
        "English (United States)",
        "US",
        &layouts::US,
        Ime::None,
        "ENG",
        "US",
    ),
    m(
        2,
        "en-US",
        "English (United States)",
        "United States-International",
        &layouts::US_INTERNATIONAL,
        Ime::None,
        "ENG",
        "INTL",
    ),
    m(
        3,
        "en-GB",
        "English (United Kingdom)",
        "United Kingdom",
        &layouts::UNITED_KINGDOM,
        Ime::None,
        "ENG",
        "UK",
    ),
    m(
        4,
        "it-IT",
        "Italiano (Italia)",
        "Italian",
        &layouts::ITALIAN,
        Ime::None,
        "ITA",
        "IT",
    ),
    m(
        5,
        "de-DE",
        "Deutsch (Deutschland)",
        "German",
        &layouts::GERMAN,
        Ime::None,
        "DEU",
        "DE",
    ),
    m(
        6,
        "fr-FR",
        "Français (France)",
        "French",
        &layouts::FRENCH,
        Ime::None,
        "FRA",
        "FR",
    ),
    m(
        7,
        "zh-CN",
        "中文(中华人民共和国)",
        "Macrohard Pinyin",
        &layouts::US,
        Ime::Pinyin,
        "中",
        "拼",
    ),
    m(
        8,
        "ja-JP",
        "日本語",
        "Macrohard IME",
        &layouts::JAPANESE,
        Ime::Japanese,
        "あ",
        "JP",
    ),
    m(
        9,
        "ko-KR",
        "한국어",
        "Macrohard IME",
        &layouts::KOREAN,
        Ime::Korean,
        "가",
        "KO",
    ),
];
pub fn method(id: u8) -> Option<&'static Method> {
    METHODS.iter().find(|m| m.id == id)
}
/// Bit n set: method n is handled by an input-method engine.
pub const IME_METHODS: u32 = (1 << 7) | (1 << 8) | (1 << 9);
/// Persisted per-user word (Setting `InputMethods`): six 4-bit method ids in
/// order (bits 0..24, 0 = unused, no gaps, no duplicates), the current index
/// (bits 24..27) and "Let me use a different input method for each app
/// window" (bit 27). The Windows default is one method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preferences(u32);
pub const MAX_METHODS: usize = 6;
pub const PER_WINDOW: u32 = 1 << 27;
impl Default for Preferences {
    fn default() -> Self {
        Self::DEFAULT
    }
}
impl Preferences {
    /// English (United States), US keyboard.
    pub const DEFAULT: Self = Self(1);
    pub fn from_bits(bits: u32) -> Option<Self> {
        let p = Self(bits);
        let n = p.count();
        if bits & 0xf000_0000 != 0 || n == 0 || p.index() >= n {
            return None;
        }
        for i in 0..MAX_METHODS {
            let id = p.slot(i);
            if (i < n) != (id != 0)
                || i < n && (method(id).is_none() || (0..i).any(|j| p.slot(j) == id))
            {
                return None;
            }
        }
        Some(p)
    }
    pub const fn bits(self) -> u32 {
        self.0
    }
    fn slot(self, i: usize) -> u8 {
        ((self.0 >> (4 * i)) & 15) as u8
    }
    fn index(self) -> usize {
        ((self.0 >> 24) & 7) as usize
    }
    pub fn count(self) -> usize {
        (0..MAX_METHODS).take_while(|&i| self.slot(i) != 0).count()
    }
    pub fn ids(self) -> impl Iterator<Item = u8> {
        (0..self.count()).map(move |i| self.slot(i))
    }
    pub fn methods(self) -> impl Iterator<Item = &'static Method> {
        self.ids().filter_map(method)
    }
    pub fn current_index(self) -> usize {
        self.index()
    }
    pub fn current(self) -> &'static Method {
        method(self.slot(self.index())).unwrap_or(&METHODS[0])
    }
    pub fn per_window(self) -> bool {
        self.0 & PER_WINDOW != 0
    }
    /// A preference list; `current` indexes `ids`.
    pub fn new(ids: &[u8], current: usize, per_window: bool) -> Option<Self> {
        if ids.is_empty() || ids.len() > MAX_METHODS || current >= ids.len() {
            return None;
        }
        let mut bits = (current as u32) << 24 | if per_window { PER_WINDOW } else { 0 };
        for (i, id) in ids.iter().enumerate() {
            if *id > 15 {
                return None;
            }
            bits |= u32::from(*id) << (4 * i);
        }
        Self::from_bits(bits)
    }
    pub fn select(self, index: usize) -> Option<Self> {
        (index < self.count()).then_some(Self(self.0 & !(7 << 24) | (index as u32) << 24))
    }
    /// Win+Space (Shift: backwards): the next installed method.
    pub fn next(self, backwards: bool) -> Self {
        let n = self.count();
        let i = if backwards {
            (self.index() + n - 1) % n
        } else {
            (self.index() + 1) % n
        };
        self.select(i).unwrap_or(self)
    }
    /// Left Alt+Shift: the first method of the next language.
    pub fn next_language(self) -> Self {
        let n = self.count();
        let tag = self.current().tag;
        let ids: [u8; MAX_METHODS] = core::array::from_fn(|i| self.slot(i));
        for step in 1..n {
            let i = (self.index() + step) % n;
            let candidate = method(ids[i]).map_or("", |m| m.tag);
            if candidate != tag && (0..i).all(|j| method(ids[j]).map_or("", |m| m.tag) != candidate)
            {
                return self.select(i).unwrap_or(self);
            }
        }
        self
    }
    /// Ctrl+Shift: the next method of the current language.
    pub fn next_layout(self) -> Self {
        let n = self.count();
        let tag = self.current().tag;
        for step in 1..n {
            let i = (self.index() + step) % n;
            if method(self.slot(i)).is_some_and(|m| m.tag == tag) {
                return self.select(i).unwrap_or(self);
            }
        }
        self
    }
}
/// What the taskbar input indicator shows (Windows 11): two lines for a
/// keyboard ("ENG" / "US"), or the IME conversion-mode glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indicator {
    Keyboard {
        language: &'static str,
        keyboard: &'static str,
    },
    Ime {
        glyph: &'static str,
        language: &'static str,
    },
}
/// IME conversion modes (`text_input::MODE_*`).
pub fn indicator(method: &Method, mode: u32) -> Indicator {
    let glyph = match (method.ime, mode) {
        (Ime::None, _) => {
            return Indicator::Keyboard {
                language: method.abbreviation,
                keyboard: method.short,
            };
        }
        (Ime::Pinyin, 0) => "英",
        (Ime::Pinyin, _) => "中",
        (Ime::Japanese, 1) => "あ",
        (Ime::Japanese, 2) => "カ",
        (Ime::Japanese, 3) => "Ａ",
        (Ime::Japanese, _) => "A",
        (Ime::Korean, 1) => "가",
        (Ime::Korean, _) => "A",
    };
    Indicator::Ime {
        glyph,
        language: method.tag,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_validate_and_cycle_like_windows() {
        assert_eq!(Preferences::from_bits(1), Some(Preferences::DEFAULT));
        for bad in [0, 0x10, 0x11, 0xa, 0x0100_0001, 0x1000_0001, 0x0010_0001] {
            assert!(Preferences::from_bits(bad).is_none(), "{bad:#x}");
        }
        // en-US US, it-IT, en-US Intl, zh-CN Pinyin; current = Italian.
        let p = Preferences::new(&[1, 4, 2, 7], 1, false).unwrap();
        assert_eq!(Preferences::from_bits(p.bits()), Some(p));
        assert_eq!(p.current().id, 4);
        assert_eq!(p.next(false).current().id, 2);
        assert_eq!(p.next(true).current().id, 1);
        assert_eq!(p.select(3).unwrap().next(false).current().id, 1, "wraps");
        // Alt+Shift skips the second en-US entry; Ctrl+Shift stays in en-US.
        let us = p.select(0).unwrap();
        assert_eq!(us.next_language().current().id, 4);
        assert_eq!(us.next_language().next_language().current().id, 7);
        assert_eq!(us.next_layout().current().id, 2);
        assert_eq!(us.next_layout().next_layout().current().id, 1);
        assert_eq!(p.next_layout(), p, "Italian has one layout");
        assert!(Preferences::new(&[1, 1], 0, false).is_none(), "duplicate");
        assert!(
            Preferences::new(&[1, 2, 3, 4, 5, 6, 7], 0, false).is_none(),
            "seven"
        );
        assert!(Preferences::new(&[1], 0, true).unwrap().per_window());
        assert_eq!(
            p.methods().map(|m| m.short).collect::<std::vec::Vec<_>>(),
            ["US", "IT", "INTL", "拼"]
        );
    }
    extern crate std;
    #[test]
    fn indicator_matches_windows_labels() {
        assert_eq!(
            indicator(method(1).unwrap(), 0),
            Indicator::Keyboard {
                language: "ENG",
                keyboard: "US"
            }
        );
        assert_eq!(
            indicator(method(7).unwrap(), 1),
            Indicator::Ime {
                glyph: "中",
                language: "zh-CN"
            }
        );
        assert_eq!(
            indicator(method(7).unwrap(), 0),
            Indicator::Ime {
                glyph: "英",
                language: "zh-CN"
            }
        );
        assert_eq!(
            indicator(method(8).unwrap(), 2),
            Indicator::Ime {
                glyph: "カ",
                language: "ja-JP"
            }
        );
        assert_eq!(
            indicator(method(9).unwrap(), 1),
            Indicator::Ime {
                glyph: "가",
                language: "ko-KR"
            }
        );
        for m in METHODS {
            assert_eq!(IME_METHODS & (1 << m.id) != 0, m.ime != Ime::None);
        }
    }
}

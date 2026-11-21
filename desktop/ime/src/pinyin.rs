//! Chinese Pinyin (Microsoft Pinyin behaviour): a syllable parser with
//! apostrophe separators and an incomplete final syllable, dictionary
//! candidates (whole input, then shorter prefixes, then single characters),
//! frequency order with in-session learning, partial selection, paging with
//! -/= and PageUp/PageDown, digits to select, Space for the highlighted
//! candidate, Enter for the letters, Shift alone for Chinese/English mode
//! and Chinese punctuation in Chinese mode.
extern crate alloc;
use crate::dict::Dictionary;
use crate::engine::{Engine, KeyIn, Output, Preedit, Segment, bounds, page};
use alloc::{string::String, vec::Vec};
use door_abi::graphics as g;
use door_keyboard_layouts::vk;
use door_service_protocol::text_input as ti;
const INITIALS: [&str; 23] = [
    "zh", "ch", "sh", "b", "p", "m", "f", "d", "t", "n", "l", "g", "k", "h", "j", "q", "x", "r",
    "z", "c", "s", "y", "w",
];
const FINALS: [&str; 36] = [
    "iang", "iong", "uang", "ang", "eng", "ing", "ong", "iao", "ian", "uai", "uan", "van", "ai",
    "ei", "ao", "ou", "an", "en", "in", "un", "vn", "ia", "ie", "iu", "ua", "uo", "ui", "ue", "ve",
    "er", "a", "o", "e", "i", "u", "v",
];
/// Whether `initial + final` is a Mandarin syllable (rule form of the
/// standard syllable table; `v` spells ü). Exclusions stay one rule per
/// line rather than the minimised boolean form.
#[allow(clippy::nonminimal_bool)]
pub fn valid(initial: &str, fin: &str) -> bool {
    if !FINALS.contains(&fin) {
        return false;
    }
    let i_final = fin.starts_with('i') && fin != "i";
    let u_like = matches!(fin, "u" | "ue" | "uan" | "un");
    match initial {
        "" => matches!(
            fin,
            "a" | "ai" | "an" | "ang" | "ao" | "e" | "ei" | "en" | "eng" | "er" | "o" | "ou"
        ),
        "j" | "q" | "x" => fin.starts_with('i') || u_like,
        "y" => matches!(
            fin,
            "a" | "ao"
                | "an"
                | "ang"
                | "e"
                | "i"
                | "in"
                | "ing"
                | "o"
                | "ong"
                | "ou"
                | "u"
                | "ue"
                | "uan"
                | "un"
        ),
        "w" => matches!(
            fin,
            "a" | "ai" | "an" | "ang" | "ei" | "en" | "eng" | "o" | "u"
        ),
        _ if fin.starts_with('v') || fin == "ue" => {
            matches!(initial, "n" | "l") && matches!(fin, "v" | "ve" | "ue")
        }
        "zh" | "ch" | "sh" | "r" | "z" | "c" | "s" => {
            !i_final
                && !matches!(fin, "o" | "iu" | "ia" | "ie")
                && !(initial == "r" && matches!(fin, "a" | "ai" | "ei" | "ua" | "uai" | "uang"))
                && !(matches!(initial, "z" | "c" | "s") && matches!(fin, "ua" | "uai" | "uang"))
                && fin != "er"
        }
        "b" | "p" | "m" | "f" => {
            !matches!(
                fin,
                "er" | "ong"
                    | "ua"
                    | "uo"
                    | "uai"
                    | "uan"
                    | "uang"
                    | "ui"
                    | "un"
                    | "ia"
                    | "iang"
                    | "iong"
            ) && !(initial == "f"
                && (fin.starts_with('i') || matches!(fin, "ai" | "ao" | "e"))
                && fin != "iao")
                && !(initial == "f" && fin == "iao")
        }
        "d" | "t" | "n" | "l" => {
            !matches!(fin, "er" | "ua" | "uai" | "uang" | "iong")
                && !(matches!(initial, "t") && matches!(fin, "en" | "ia" | "in" | "iang" | "iu"))
                && !(initial == "d" && matches!(fin, "in" | "iang" | "en"))
                && !(matches!(initial, "n" | "l") && matches!(fin, "ui" | "ua" | "uai"))
        }
        "g" | "k" | "h" => !fin.starts_with('i') && !matches!(fin, "er" | "o"),
        _ => false,
    }
}
fn initial_of(s: &str) -> &str {
    INITIALS
        .iter()
        .find(|i| s.starts_with(**i))
        .copied()
        .unwrap_or("")
}
/// The longest syllable at the start of `s` (bytes).
fn syllable_len(s: &str) -> Option<usize> {
    let init = initial_of(s);
    let rest = &s[init.len()..];
    FINALS
        .iter()
        .filter(|f| rest.starts_with(**f) && valid(init, f))
        .map(|f| init.len() + f.len())
        .max()
}
/// Could `s` begin a syllable (an initial, or a syllable prefix)?
fn partial(s: &str) -> bool {
    let init = initial_of(s);
    let rest = &s[init.len()..];
    !s.is_empty()
        && (rest.is_empty() && !init.is_empty()
            || FINALS.iter().any(|f| f.starts_with(rest) && valid(init, f)))
}
/// Split letters (with optional `'` separators) into syllables; the last
/// one may be incomplete. Backtracks when the longest match strands a
/// remainder ("xian" stays one syllable, "fangan" is fang'an).
pub fn parse(input: &str) -> Option<Vec<&str>> {
    let mut out = Vec::new();
    for part in input.split('\'').filter(|p| !p.is_empty()) {
        out.extend(split(part)?);
    }
    Some(out)
}
fn split(s: &str) -> Option<Vec<&str>> {
    if s.is_empty() {
        return Some(Vec::new());
    }
    let longest = syllable_len(s);
    let mut lens: Vec<usize> = (1..=s.len())
        .rev()
        .filter(|&n| {
            let init = initial_of(&s[..n]);
            valid(init, &s[init.len()..n]) && Some(n) <= longest
        })
        .collect();
    if lens.is_empty() {
        return partial(s).then(|| alloc::vec![s]);
    }
    lens.dedup();
    for n in lens {
        if let Some(mut rest) = split(&s[n..]) {
            rest.insert(0, &s[..n]);
            return Some(rest);
        }
    }
    partial(s).then(|| alloc::vec![s])
}
fn complete(syllable: &str) -> bool {
    let init = initial_of(syllable);
    valid(init, &syllable[init.len()..])
}
/// Microsoft Pinyin Chinese punctuation (Chinese mode).
fn punctuation(c: char, quote: &mut bool, double: &mut bool) -> Option<&'static str> {
    Some(match c {
        ',' => "，",
        '.' => "。",
        '?' => "？",
        '!' => "！",
        ';' => "；",
        ':' => "：",
        '(' => "（",
        ')' => "）",
        '[' => "【",
        ']' => "】",
        '<' => "《",
        '>' => "》",
        '\\' => "、",
        '$' => "￥",
        '^' => "……",
        '_' => "——",
        '\'' => {
            *quote = !*quote;
            if *quote { "‘" } else { "’" }
        }
        '"' => {
            *double = !*double;
            if *double { "“" } else { "”" }
        }
        _ => return None,
    })
}
pub struct Pinyin {
    dict: Dictionary,
    /// Raw letters not yet converted.
    input: String,
    /// Partial selections: (Chinese text, raw letters it consumed).
    converted: Vec<(String, String)>,
    candidates: Vec<(String, usize)>,
    selected: usize,
    english: bool,
    shift_alone: bool,
    quote: bool,
    double: bool,
    private: bool,
    /// In-session learning: (pinyin key, preferred text), newest last.
    learned: Vec<(String, String)>,
}
impl Pinyin {
    pub fn new(dict: Dictionary) -> Self {
        Self {
            dict,
            input: String::new(),
            converted: Vec::new(),
            candidates: Vec::new(),
            selected: 0,
            english: false,
            shift_alone: false,
            quote: false,
            double: false,
            private: false,
            learned: Vec::new(),
        }
    }
    fn key_of(syllables: &[&str]) -> String {
        syllables.join("'")
    }
    /// Candidates for the remaining input: (text, syllables consumed).
    fn refresh(&mut self) {
        self.selected = 0;
        self.candidates.clear();
        let Some(sy) = parse(&self.input) else { return };
        if sy.is_empty() {
            return;
        }
        let mut out: Vec<(String, usize)> = Vec::new();
        let add = |t: &str, n: usize, out: &mut Vec<(String, usize)>| {
            if !out.iter().any(|c| c.0 == t) {
                out.push((String::from(t), n));
            }
        };
        let full = Self::key_of(&sy);
        for (key, text) in self.learned.iter().rev() {
            if *key == full {
                add(text, sy.len(), &mut out);
            }
        }
        let last_partial = !complete(sy[sy.len() - 1]);
        if !last_partial && let Some(e) = self.dict.get(&full) {
            e.candidates().for_each(|c| add(c.text, sy.len(), &mut out));
        }
        if last_partial || sy.len() > 1 {
            // Abbreviated or incomplete last syllable: phrases by prefix,
            // capped so a single initial cannot flood the list.
            let prefix = full.clone();
            for e in self
                .dict
                .prefix(&prefix)
                .filter(|e| e.reading.split('\'').count() == sy.len())
                .take(64)
            {
                e.candidates().for_each(|c| add(c.text, sy.len(), &mut out));
            }
        }
        for n in (1..sy.len()).rev() {
            let key = Self::key_of(&sy[..n]);
            if let Some(e) = self.dict.get(&key) {
                e.candidates().for_each(|c| add(c.text, n, &mut out));
            }
        }
        if sy.len() == 1 && last_partial {
            for e in self
                .dict
                .prefix(sy[0])
                .filter(|e| !e.reading.contains('\''))
                .take(8)
            {
                e.candidates()
                    .take(4)
                    .for_each(|c| add(c.text, 1, &mut out));
            }
        }
        self.candidates = out;
    }
    fn raw_for(&self, syllables: usize) -> usize {
        // Bytes of `input` covering the first `syllables` syllables.
        let mut at = 0;
        let parts = parse(&self.input).unwrap_or_default();
        for s in parts.iter().take(syllables) {
            let found = self.input[at..].find(s).map_or(0, |i| i + at);
            at = found + s.len();
        }
        // Swallow a following separator.
        if self.input[at..].starts_with('\'') {
            at + 1
        } else {
            at
        }
    }
    fn choose(&mut self, index: usize) -> String {
        let Some((text, n)) = self.candidates.get(index).cloned() else {
            return String::new();
        };
        let used = self.raw_for(n);
        let raw: String = self.input[..used].into();
        if !self.private {
            let key = Self::key_of(&parse(&raw).unwrap_or_default());
            self.learned.retain(|l| !(l.0 == key && l.1 == text));
            if self.learned.len() == 256 {
                self.learned.remove(0);
            }
            self.learned.push((key, text.clone()));
        }
        self.converted.push((text, raw));
        self.input.drain(..used);
        if self.input.is_empty() {
            let all: String = self.converted.drain(..).map(|c| c.0).collect();
            self.candidates.clear();
            return all;
        }
        self.refresh();
        String::new()
    }
    fn preedit(&self) -> Preedit {
        let mut text = String::new();
        let mut segments = Vec::new();
        for (c, _) in &self.converted {
            text.push_str(c);
        }
        if !text.is_empty() {
            segments.push(Segment {
                start: 0,
                end: text.len(),
                style: ti::STYLE_CONVERTED,
            });
        }
        let start = text.len();
        match parse(&self.input) {
            Some(sy) => text.push_str(&sy.join("'")),
            None => text.push_str(&self.input),
        }
        if text.len() > start {
            segments.push(Segment {
                start,
                end: text.len(),
                style: ti::STYLE_INPUT,
            });
        }
        let cursor = text.len();
        Preedit {
            text,
            cursor,
            segments,
        }
    }
    fn output(&self, handled: bool, commit: String) -> Output {
        let names: Vec<String> = self.candidates.iter().map(|c| c.0.clone()).collect();
        let composing = self.composing();
        Output {
            handled,
            commit,
            preedit: if composing {
                self.preedit()
            } else {
                Preedit::default()
            },
            candidates: if composing {
                page(&names, self.selected, false)
            } else {
                None
            },
        }
    }
    fn raw(&mut self) -> String {
        let mut s: String = self.converted.drain(..).map(|c| c.0).collect();
        s.push_str(&core::mem::take(&mut self.input));
        self.candidates.clear();
        s
    }
}
impl Engine for Pinyin {
    fn key(&mut self, k: &KeyIn) -> Output {
        let shift_key = matches!(k.code, 0x2a | 0x36) || k.vk == vk::SHIFT;
        if !k.down() {
            // Shift pressed and released alone toggles Chinese/English.
            if shift_key && core::mem::take(&mut self.shift_alone) {
                let commit = self.raw();
                self.english = !self.english;
                return Output {
                    handled: true,
                    commit,
                    ..Output::default()
                };
            }
            return Output {
                handled: self.composing(),
                ..self.output(self.composing(), String::new())
            };
        }
        if shift_key {
            self.shift_alone = k.flags & (g::CONTROL | g::ALT | g::LOGO) == 0;
            return self.output(self.composing(), String::new());
        }
        self.shift_alone = false;
        if k.chord() {
            return self.output(self.composing(), String::new());
        }
        let composing = self.composing();
        if self.english {
            return Output::forward();
        }
        if let Some(c) =
            k.ch.filter(|c| c.is_ascii_lowercase() || *c == '\'' && composing)
        {
            if self.input.len() < 64 {
                self.input.push(c);
                self.refresh();
            }
            return self.output(true, String::new());
        }
        if !composing {
            if let Some(p) =
                k.ch.and_then(|c| punctuation(c, &mut self.quote, &mut self.double))
            {
                return Output {
                    handled: true,
                    commit: p.into(),
                    ..Output::default()
                };
            }
            return Output::forward();
        }
        let shown = bounds(self.candidates.iter().map(|c| c.0.as_str()), self.selected);
        match k.vk {
            vk::SPACE => {
                let commit = if self.candidates.is_empty() {
                    self.raw()
                } else {
                    self.choose(self.selected)
                };
                self.output(true, commit)
            }
            vk::RETURN => {
                let commit = self.raw();
                self.output(true, commit)
            }
            vk::ESCAPE => {
                self.cancel();
                self.output(true, String::new())
            }
            vk::BACK => {
                if self.input.pop().is_none()
                    && let Some((_, raw)) = self.converted.pop()
                {
                    self.input = raw;
                }
                self.refresh();
                self.output(true, String::new())
            }
            b'1'..=b'9' if k.flags & g::SHIFT == 0 => {
                let i = shown.start + (k.vk - b'1') as usize;
                let commit = if i < shown.end {
                    self.choose(i)
                } else {
                    String::new()
                };
                self.output(true, commit)
            }
            vk::OEM_MINUS | vk::PRIOR => {
                if shown.start > 0 {
                    let names = self.candidates.iter().map(|c| c.0.as_str());
                    self.selected = bounds(names, shown.start - 1).start;
                }
                self.output(true, String::new())
            }
            vk::OEM_PLUS | vk::NEXT => {
                if shown.end < self.candidates.len() {
                    self.selected = shown.end;
                }
                self.output(true, String::new())
            }
            vk::LEFT | vk::UP => {
                self.selected = self.selected.saturating_sub(1);
                self.output(true, String::new())
            }
            vk::RIGHT | vk::DOWN => {
                if self.selected + 1 < self.candidates.len() {
                    self.selected += 1;
                }
                self.output(true, String::new())
            }
            _ => {
                // Punctuation commits the highlighted candidate first.
                if let Some(p) =
                    k.ch.and_then(|c| punctuation(c, &mut self.quote, &mut self.double))
                {
                    let mut commit = if self.candidates.is_empty() {
                        self.raw()
                    } else {
                        self.choose(self.selected)
                    };
                    if self.composing() {
                        commit.push_str(&self.raw());
                    }
                    commit.push_str(p);
                    return self.output(true, commit);
                }
                self.output(true, String::new())
            }
        }
    }
    fn cancel(&mut self) {
        self.input.clear();
        self.converted.clear();
        self.candidates.clear();
    }
    fn complete(&mut self) -> String {
        if self.candidates.is_empty() {
            self.raw()
        } else {
            let mut s = self.choose(0);
            if self.composing() {
                s.push_str(&self.raw());
            }
            s
        }
    }
    fn mode(&self) -> u32 {
        if self.english {
            ti::MODE_DIRECT
        } else {
            ti::MODE_NATIVE
        }
    }
    fn composing(&self) -> bool {
        !self.input.is_empty() || !self.converted.is_empty()
    }
    fn set_private(&mut self, private: bool) {
        self.private = private;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn syllable_rules_and_parsing() {
        for (i, f) in [
            ("", "a"),
            ("zh", "ong"),
            ("x", "iang"),
            ("l", "ve"),
            ("n", "v"),
            ("y", "uan"),
            ("w", "o"),
            ("j", "u"),
        ] {
            assert!(valid(i, f), "{i}{f}");
        }
        for (i, f) in [
            ("", "i"),
            ("zh", "i"),
            ("j", "a"),
            ("g", "i"),
            ("b", "ong"),
            ("f", "ai"),
            ("w", "i"),
        ] {
            assert_eq!(valid(i, f), i == "zh", "{i}{f}");
        }
        assert_eq!(parse("nihao").unwrap(), ["ni", "hao"]);
        assert_eq!(parse("xian").unwrap(), ["xian"]);
        assert_eq!(parse("xi'an").unwrap(), ["xi", "an"]);
        assert_eq!(parse("zhongguo").unwrap(), ["zhong", "guo"]);
        assert_eq!(parse("fangan").unwrap(), ["fang", "an"]);
        assert_eq!(parse("nih").unwrap(), ["ni", "h"]);
        assert_eq!(parse("zh").unwrap(), ["zh"]);
        assert!(parse("vvv").is_none());
    }
}

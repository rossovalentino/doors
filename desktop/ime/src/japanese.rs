//! Japanese input (Microsoft IME behaviour): romaji to hiragana, kana–kanji
//! conversion over a word lattice (dictionary costs, deinflected verb and
//! adjective forms, particles joined to their phrase), phrase (bunsetsu)
//! segments with focus (Left/Right) and resizing (Shift+Left/Right),
//! candidate selection (Space/Up/Down, digits, vertical window from the
//! second Space), F6–F10 forms, Hankaku/Zenkaku toggling direct input.
//!
//! Dictionary classes: `p` particle/auxiliary (joins the preceding phrase),
//! `v5k v5g v5s v5t v5n v5b v5m v5r v5u` godan verbs, `v1` ichidan verbs,
//! `adj` i-adjectives, `vs` suru nouns; anything else is a noun-like word.
extern crate alloc;
use crate::dict::Dictionary;
use crate::engine::{Engine, KeyIn, Output, Preedit, Segment, bounds, page};
use crate::kana;
use alloc::{string::String, vec::Vec};
use door_abi::graphics as g;
use door_keyboard_layouts::vk;
use door_service_protocol::text_input as ti;
const DEFAULT_COST: u32 = 6000;
const PARTICLE_COST: u32 = 1500;
const UNKNOWN_COST: u32 = 12000;
const MAX_WORD: usize = 12;
/// Per-word transition cost standing in for Mozc's connection matrix
/// (not shipped): without it, zero-cost kana function words (に, ほん,
/// ご) fragment every reading and conversion never leaves kana. Particles
/// attach cheaply to the word before them.
const CONTENT_TRANSITION: u32 = 4000;
const PARTICLE_TRANSITION: u32 = 300;
/// Least cost of a content word written in its own kana (Mozc lists many
/// such function morphemes at cost 0, balanced by connection costs).
const KANA_FLOOR: u32 = 4000;
/// Godan row: (class, a, i, u, e, o, te/ta stem, voiced te/ta).
type GodanRow = (
    &'static str,
    char,
    char,
    char,
    char,
    char,
    &'static str,
    bool,
);
const GODAN: [GodanRow; 9] = [
    ("v5k", 'か', 'き', 'く', 'け', 'こ', "い", false),
    ("v5g", 'が', 'ぎ', 'ぐ', 'げ', 'ご', "い", true),
    ("v5s", 'さ', 'し', 'す', 'せ', 'そ', "し", false),
    ("v5t", 'た', 'ち', 'つ', 'て', 'と', "っ", false),
    ("v5n", 'な', 'に', 'ぬ', 'ね', 'の', "ん", true),
    ("v5b", 'ば', 'び', 'ぶ', 'べ', 'ぼ', "ん", true),
    ("v5m", 'ま', 'み', 'む', 'め', 'も', "ん", true),
    ("v5r", 'ら', 'り', 'る', 'れ', 'ろ', "っ", false),
    ("v5u", 'わ', 'い', 'う', 'え', 'お', "っ", false),
];
const AFTER_A: [&str; 3] = ["ない", "なかった", "ず"];
const AFTER_I: [&str; 6] = ["ます", "ました", "ません", "ましょう", "たい", "ながら"];
const AFTER_E: [&str; 3] = ["ば", "る", ""];
const TE: [&str; 5] = ["て", "た", "ている", "ていた", "てください"];
const ICHIDAN: [&str; 15] = [
    "る",
    "ない",
    "なかった",
    "ます",
    "ました",
    "ません",
    "ましょう",
    "たい",
    "て",
    "た",
    "ている",
    "ていた",
    "れば",
    "られる",
    "よう",
];
const ADJ: [&str; 7] = ["い", "く", "かった", "くない", "くて", "ければ", "さ"];
const SURU: [&str; 10] = [
    "する",
    "します",
    "しました",
    "しません",
    "しない",
    "して",
    "した",
    "している",
    "したい",
    "すれば",
];
#[derive(Clone, Debug, PartialEq, Eq)]
struct Word {
    reading_len: usize,
    surface: String,
    cost: u32,
    particle: bool,
}
fn chars_len(s: &str) -> usize {
    s.chars().count()
}
fn voiced(s: &str) -> String {
    s.replacen('て', "で", 1).replacen('た', "だ", 1)
}
/// Every (inflected suffix, dictionary ending, class) a reading may end in.
fn inflections(mut f: impl FnMut(&str, &str, &str)) {
    let mut buf = String::new();
    for (class, a, i, u, e, o, onbin, voice) in GODAN {
        let mut tmp = [0u8; 4];
        let ending = String::from(&*u.encode_utf8(&mut tmp));
        f(&ending, &ending, class);
        for (row, list) in [(a, &AFTER_A[..]), (i, &AFTER_I[..]), (e, &AFTER_E[..])] {
            for s in list {
                buf.clear();
                buf.push(row);
                buf.push_str(s);
                f(&buf, &ending, class);
            }
        }
        buf.clear();
        buf.push(o);
        buf.push('う');
        f(&buf, &ending, class);
        for t in TE {
            buf.clear();
            buf.push_str(onbin);
            buf.push_str(&if voice { voiced(t) } else { String::from(t) });
            // 行く: いって/いった (irregular), covered by its own entries.
            f(&buf, &ending, class);
        }
    }
    for s in ICHIDAN {
        f(s, "る", "v1");
    }
    for s in ADJ {
        f(s, "い", "adj");
    }
    for s in SURU {
        f(s, "", "vs");
    }
}
/// All dictionary words whose reading is exactly `r` (direct or inflected).
fn words(dict: &Dictionary, r: &str) -> Vec<Word> {
    let n = chars_len(r);
    let mut out = Vec::new();
    if let Some(e) = dict.get(r) {
        for c in e.candidates() {
            let particle = c.class == "p";
            let cost = c.cost.unwrap_or(if particle {
                PARTICLE_COST
            } else {
                DEFAULT_COST
            });
            out.push(Word {
                reading_len: n,
                surface: c.text.into(),
                cost: if !particle && c.text == r {
                    cost.max(KANA_FLOOR)
                } else {
                    cost
                },
                particle,
            });
        }
    }
    inflections(|suffix, ending, class| {
        let Some(stem) = r.strip_suffix(suffix).filter(|s| !s.is_empty()) else {
            return;
        };
        if suffix.is_empty() && class != "vs" {
            return;
        }
        let mut base = String::from(stem);
        base.push_str(ending);
        if base == r && class != "vs" {
            return;
        }
        let Some(e) = dict.get(&base) else { return };
        for c in e.candidates().filter(|c| c.class == class) {
            let Some(head) = c.text.strip_suffix(ending) else {
                continue;
            };
            let mut surface = String::from(head);
            surface.push_str(suffix);
            if !out.iter().any(|w: &Word| w.surface == surface) {
                out.push(Word {
                    reading_len: n,
                    surface,
                    cost: c.cost.unwrap_or(DEFAULT_COST) + 200,
                    particle: false,
                });
            }
        }
    });
    out.sort_by_key(|w| w.cost);
    out
}
/// The cheapest path over `chars` (Viterbi); unknown kana cost the most.
/// States track whether the path ends in a content word: only a particle
/// right after one attaches at `PARTICLE_TRANSITION` (a leading or chained
/// particle costs like a content word).
fn best_path(dict: &Dictionary, chars: &[char]) -> Vec<Word> {
    const START: usize = 0;
    const CONTENT: usize = 1;
    const PARTICLE: usize = 2;
    let n = chars.len();
    let mut best: Vec<[Option<(usize, usize, Word)>; 3]> = alloc::vec![[None, None, None]; n + 1];
    let mut cost = alloc::vec![[u32::MAX; 3]; n + 1];
    cost[0][START] = 0;
    for i in 0..n {
        if cost[i].iter().all(|c| *c == u32::MAX) {
            continue;
        }
        for j in i + 1..=(i + MAX_WORD).min(n) {
            let r: String = chars[i..j].iter().collect();
            let mut options = words(dict, &r);
            if j == i + 1 {
                options.push(Word {
                    reading_len: 1,
                    surface: r.clone(),
                    cost: UNKNOWN_COST,
                    particle: false,
                });
            }
            // The cheapest content word and the cheapest particle.
            for particle in [false, true] {
                let Some(w) = options.iter().find(|w| w.particle == particle) else {
                    continue;
                };
                let to = if particle { PARTICLE } else { CONTENT };
                for from in [START, CONTENT, PARTICLE] {
                    if cost[i][from] == u32::MAX {
                        continue;
                    }
                    let transition = if particle && from == CONTENT {
                        PARTICLE_TRANSITION
                    } else {
                        CONTENT_TRANSITION
                    };
                    let total = cost[i][from]
                        .saturating_add(w.cost)
                        .saturating_add(transition);
                    if total < cost[j][to] {
                        cost[j][to] = total;
                        best[j][to] = Some((i, from, w.clone()));
                    }
                }
            }
        }
    }
    let mut state = (0..3).min_by_key(|s| cost[n][*s]).unwrap_or(CONTENT);
    let mut path = Vec::new();
    let mut at = n;
    while at > 0 {
        let Some((from, previous, w)) = best[at][state].take() else {
            break;
        };
        path.push(w);
        at = from;
        state = previous;
    }
    path.reverse();
    path
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Phrase {
    reading: String,
    candidates: Vec<String>,
    selected: usize,
}
fn hiragana_forms(reading: &str, out: &mut Vec<String>) {
    for s in [String::from(reading), kana::katakana(reading)] {
        if !out.contains(&s) {
            out.push(s);
        }
    }
}
/// Candidates for one phrase reading: the best analysis, whole-reading
/// words, alternative content words before the same particles, then kana.
fn phrase_candidates(dict: &Dictionary, reading: &str) -> Vec<String> {
    let chars: Vec<char> = reading.chars().collect();
    let path = best_path(dict, &chars);
    let mut out: Vec<String> = Vec::new();
    let joined: String = path.iter().map(|w| w.surface.as_str()).collect();
    out.push(joined);
    for w in words(dict, reading) {
        if !out.contains(&w.surface) {
            out.push(w.surface);
        }
    }
    if let Some(first) = path.first().filter(|_| path.len() > 1) {
        let head: String = chars[..first.reading_len].iter().collect();
        let tail: String = path[1..].iter().map(|w| w.surface.as_str()).collect();
        for w in words(dict, &head) {
            let mut s = w.surface;
            s.push_str(&tail);
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    hiragana_forms(reading, &mut out);
    for e in EMOJI.iter().filter(|e| e.0 == reading).flat_map(|e| e.1) {
        if !out.iter().any(|s| s == e) {
            out.push(String::from(*e));
        }
    }
    out
}
/// Emoji the Microsoft IME offers after the words and kana forms of a few
/// common readings, including modifier and ZWJ sequences (one grapheme
/// cluster each). Built in: the open dictionaries carry no emoji.
const EMOJI: &[(&str, &[&str])] = &[
    ("いいね", &["\u{1f44d}", "\u{1f44d}\u{1f3fd}"]),
    ("えがお", &["\u{1f60a}", "\u{1f604}"]),
    (
        "かぞく",
        &["\u{1f46a}", "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}"],
    ),
    ("にほん", &["\u{1f1ef}\u{1f1f5}"]),
    ("ねこ", &["\u{1f431}", "\u{1f408}\u{200d}\u{2b1b}"]),
    ("はーと", &["\u{2764}\u{fe0f}"]),
];
/// Split a reading into phrases: each content word starts one; particles
/// join the phrase before them.
fn phrases(dict: &Dictionary, chars: &[char]) -> Vec<Phrase> {
    let mut readings: Vec<String> = Vec::new();
    let mut at = 0;
    for w in best_path(dict, chars) {
        let r: String = chars[at..at + w.reading_len].iter().collect();
        at += w.reading_len;
        match readings.last_mut() {
            Some(last) if w.particle => last.push_str(&r),
            _ => readings.push(r),
        }
    }
    readings
        .into_iter()
        .map(|reading| Phrase {
            candidates: phrase_candidates(dict, &reading),
            reading,
            selected: 0,
        })
        .collect()
}
struct Conversion {
    phrases: Vec<Phrase>,
    focus: usize,
    window: bool,
}
pub struct Japanese {
    dict: Dictionary,
    kana: String,
    tail: String,
    raw: String,
    conv: Option<Conversion>,
    mode: u32,
}
impl Japanese {
    pub fn new(dict: Dictionary) -> Self {
        Self {
            dict,
            kana: String::new(),
            tail: String::new(),
            raw: String::new(),
            conv: None,
            mode: ti::MODE_NATIVE,
        }
    }
    fn reading(&self) -> String {
        let mut s = self.kana.clone();
        kana::flush(&mut self.tail.clone(), &mut s);
        s
    }
    fn shown(&self, s: &str) -> String {
        if self.mode == ti::MODE_KATAKANA {
            kana::katakana(s)
        } else {
            String::from(s)
        }
    }
    fn convert(&mut self) {
        let mut tail = core::mem::take(&mut self.tail);
        kana::flush(&mut tail, &mut self.kana);
        let chars: Vec<char> = self.kana.chars().collect();
        if !chars.is_empty() {
            self.conv = Some(Conversion {
                phrases: phrases(&self.dict, &chars),
                focus: 0,
                window: false,
            });
        }
    }
    /// One phrase holding `text` (F6–F10 forms).
    fn form(&mut self, text: String) {
        let reading = self.reading();
        self.tail.clear();
        self.kana = reading.clone();
        self.conv = Some(Conversion {
            phrases: alloc::vec![Phrase {
                reading,
                candidates: alloc::vec![text],
                selected: 0
            }],
            focus: 0,
            window: false,
        });
    }
    fn take(&mut self) -> String {
        let out = match self.conv.take() {
            Some(c) => c
                .phrases
                .iter()
                .map(|p| p.candidates[p.selected].clone())
                .collect(),
            None => {
                let r = self.reading();
                self.shown(&r)
            }
        };
        self.kana.clear();
        self.tail.clear();
        self.raw.clear();
        out
    }
    /// Shift+Left/Right: the focused phrase gives or takes one kana from
    /// the rest, which is converted again.
    fn resize(&mut self, grow: bool) {
        let Some(c) = self.conv.as_mut() else { return };
        let all: Vec<char> = self.kana.chars().collect();
        let start: usize = c.phrases[..c.focus]
            .iter()
            .map(|p| chars_len(&p.reading))
            .sum();
        let len = chars_len(&c.phrases[c.focus].reading);
        let new_len = if grow { len + 1 } else { len.saturating_sub(1) };
        if new_len == 0 || start + new_len > all.len() {
            return;
        }
        let reading: String = all[start..start + new_len].iter().collect();
        let focused = Phrase {
            candidates: phrase_candidates(&self.dict, &reading),
            reading,
            selected: 0,
        };
        c.phrases.truncate(c.focus);
        c.phrases.push(focused);
        let rest = &all[start + new_len..];
        if !rest.is_empty() {
            c.phrases.extend(phrases(&self.dict, rest));
        }
        c.window = false;
    }
    fn preedit(&self) -> Preedit {
        let mut p = Preedit::default();
        if let Some(c) = &self.conv {
            for (i, ph) in c.phrases.iter().enumerate() {
                let start = p.text.len();
                p.text.push_str(&ph.candidates[ph.selected]);
                let style = if i == c.focus {
                    ti::STYLE_TARGET_CONVERTED
                } else {
                    ti::STYLE_CONVERTED
                };
                p.segments.push(Segment {
                    start,
                    end: p.text.len(),
                    style,
                });
                if i == c.focus {
                    p.cursor = p.text.len();
                }
            }
        } else {
            p.text = self.shown(&self.kana);
            p.text.push_str(&self.tail);
            p.cursor = p.text.len();
            if !p.text.is_empty() {
                p.segments.push(Segment {
                    start: 0,
                    end: p.text.len(),
                    style: ti::STYLE_INPUT,
                });
            }
        }
        p
    }
    fn output(&self, handled: bool, commit: String) -> Output {
        let candidates = self.conv.as_ref().filter(|c| c.window).and_then(|c| {
            let ph = &c.phrases[c.focus];
            page(&ph.candidates, ph.selected, true)
        });
        Output {
            handled,
            commit,
            preedit: self.preedit(),
            candidates,
        }
    }
    /// F6 hiragana, F7 katakana, F8 half-width katakana (shown full-width:
    /// no half-width forms in the font set), F9/F10 the typed romaji.
    fn forms(&mut self, key: u8) {
        let r = self.reading();
        let text = match key {
            vk::F6 => r,
            vk::F7 | vk::F8 => kana::katakana(&r),
            vk::F9 => kana::full_width(&self.raw),
            _ => self.raw.clone(),
        };
        self.form(text);
    }
    fn conversion_key(&mut self, k: &KeyIn) -> Output {
        match k.vk {
            vk::LEFT | vk::RIGHT if k.shift() => {
                self.resize(k.vk == vk::RIGHT);
                return self.output(true, String::new());
            }
            vk::RETURN => {
                let commit = self.take();
                return self.output(true, commit);
            }
            vk::F6..=vk::F10 => {
                self.forms(k.vk);
                return self.output(true, String::new());
            }
            _ => {}
        }
        let c = self.conv.as_mut().unwrap();
        let n = c.phrases.len();
        let ph = &mut c.phrases[c.focus];
        let count = ph.candidates.len();
        let mut leave = false;
        match k.vk {
            vk::SPACE | vk::DOWN | vk::CONVERT if !k.shift() => {
                ph.selected = (ph.selected + 1) % count;
                c.window = true;
            }
            vk::SPACE | vk::UP => {
                ph.selected = (ph.selected + count - 1) % count;
                c.window = true;
            }
            vk::PRIOR | vk::NEXT if c.window => {
                let names = || ph.candidates.iter().map(String::as_str);
                let shown = bounds(names(), ph.selected);
                ph.selected = if k.vk == vk::NEXT {
                    shown.end.min(count - 1)
                } else if shown.start > 0 {
                    bounds(names(), shown.start - 1).start
                } else {
                    0
                };
            }
            b'1'..=b'9' if c.window && !k.shift() => {
                let shown = bounds(ph.candidates.iter().map(String::as_str), ph.selected);
                let i = shown.start + (k.vk - b'1') as usize;
                if i < shown.end {
                    ph.selected = i;
                    c.window = false;
                    c.focus = (c.focus + 1).min(n - 1);
                }
            }
            vk::LEFT => {
                c.focus = c.focus.saturating_sub(1);
                c.window = false;
            }
            vk::RIGHT => {
                c.focus = (c.focus + 1).min(n - 1);
                c.window = false;
            }
            vk::ESCAPE if c.window => c.window = false,
            vk::ESCAPE | vk::BACK => leave = true,
            _ => {
                // New input commits the conversion first; other keys commit
                // and reach the application.
                let commit = self.take();
                if k.ch.is_some_and(|ch| ch.is_ascii_graphic()) {
                    let mut out = self.key(k);
                    out.commit.insert_str(0, &commit);
                    return out;
                }
                return Output {
                    handled: false,
                    commit,
                    ..Output::default()
                };
            }
        }
        if leave {
            self.conv = None;
        }
        self.output(true, String::new())
    }
    fn input(&mut self, c: char) -> bool {
        self.raw.push(c);
        if kana::feed(&mut self.tail, &mut self.kana, c) {
            return true;
        }
        // Digits and other symbols: full-width in hiragana mode.
        if c.is_ascii_graphic() {
            let mut t = core::mem::take(&mut self.tail);
            kana::flush(&mut t, &mut self.kana);
            self.kana
                .push_str(&kana::full_width(c.encode_utf8(&mut [0; 4])));
            return true;
        }
        self.raw.pop();
        false
    }
}
impl Engine for Japanese {
    fn key(&mut self, k: &KeyIn) -> Output {
        if !k.down() {
            return Output {
                handled: self.composing(),
                ..self.output(self.composing(), String::new())
            };
        }
        let toggle = k.vk == vk::KANJI || k.code == 0x29 && k.flags & g::ALT != 0;
        if toggle {
            let commit = self.take();
            self.mode = if self.mode == ti::MODE_DIRECT {
                ti::MODE_NATIVE
            } else {
                ti::MODE_DIRECT
            };
            return Output {
                handled: true,
                commit,
                ..Output::default()
            };
        }
        if k.vk == vk::DBE_HIRAGANA {
            self.mode = if k.shift() {
                ti::MODE_KATAKANA
            } else {
                ti::MODE_NATIVE
            };
            return self.output(true, String::new());
        }
        if self.mode == ti::MODE_DIRECT {
            return Output::forward();
        }
        if k.chord() {
            return self.output(self.composing(), String::new());
        }
        if self.conv.is_some() {
            return self.conversion_key(k);
        }
        let composing = self.composing();
        if let Some(c) =
            k.ch.filter(|c| c.is_ascii_graphic() && (composing || !c.is_ascii_digit()))
            && self.input(c)
        {
            return self.output(true, String::new());
        }
        if !composing {
            return Output::forward();
        }
        match k.vk {
            vk::SPACE | vk::CONVERT => self.convert(),
            vk::RETURN => {
                let commit = self.take();
                return self.output(true, commit);
            }
            vk::ESCAPE => self.cancel(),
            vk::BACK => {
                if self.tail.pop().is_none() {
                    self.kana.pop();
                }
                self.raw.pop();
            }
            vk::F6..=vk::F10 => self.forms(k.vk),
            _ => {}
        }
        self.output(true, String::new())
    }
    fn cancel(&mut self) {
        self.conv = None;
        self.kana.clear();
        self.tail.clear();
        self.raw.clear();
    }
    fn complete(&mut self) -> String {
        self.take()
    }
    fn mode(&self) -> u32 {
        self.mode
    }
    fn composing(&self) -> bool {
        self.conv.is_some() || !self.kana.is_empty() || !self.tail.is_empty()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;
    /// A handful of synthetic entries (not a real dictionary).
    fn dict() -> Dictionary {
        let mut lines: Vec<String> = [
            "か\t蚊,7000",
            "かく\t書く,4000,v5k\t描く,5000,v5k",
            "がっこう\t学校,3000",
            "たべる\t食べる,4000,v1",
            "たかい\t高い,4000,adj",
            "に\tに,1000,p",
            "は\tは,1000,p",
            "べんきょう\t勉強,3500,vs",
            "わたし\t私,3000",
        ]
        .iter()
        .map(|s| String::from(*s))
        .collect();
        lines.sort();
        let mut text = String::from("# door-ime dictionary v1\n");
        for l in lines {
            text.push_str(&l);
            text.push('\n');
        }
        Dictionary::parse(alloc::boxed::Box::leak(
            text.into_bytes().into_boxed_slice(),
        ))
        .unwrap()
    }
    #[test]
    fn deinflection_and_lattice() {
        let d = dict();
        assert_eq!(words(&d, "かきます")[0].surface, "書きます");
        assert_eq!(words(&d, "かいた")[0].surface, "書いた");
        assert_eq!(words(&d, "たべない")[0].surface, "食べない");
        assert_eq!(words(&d, "たかかった")[0].surface, "高かった");
        assert_eq!(words(&d, "べんきょうした")[0].surface, "勉強した");
        let chars: Vec<char> = "わたしはがっこうにかきます".chars().collect();
        let p = phrases(&d, &chars);
        let shown: Vec<&str> = p.iter().map(|p| p.candidates[0].as_str()).collect();
        assert_eq!(shown, ["私は", "学校に", "書きます"]);
        assert!(p[2].candidates.contains(&String::from("描きます")));
        assert!(p[2].candidates.contains(&String::from("かきます")));
    }
}

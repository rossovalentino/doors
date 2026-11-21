//! Korean 2-set (Dubeolsik) Hangul composition, algorithmic (Unicode
//! syllable arithmetic). Behaviour follows the Microsoft Korean IME: compound
//! vowels and finals combine, a vowel after a final consonant takes it as
//! its initial (the second half of a compound final), Backspace removes the
//! last jamo, non-jamo keys commit and pass through, and the Hangul key
//! (or Right Alt on Type 1 keyboards) toggles Hangul/English input.
extern crate alloc;
use crate::engine::{Engine, KeyIn, Output, Preedit, Segment};
use alloc::{string::String, vec::Vec};
use door_keyboard_layouts::vk;
use door_service_protocol::text_input as ti;
/// Compatibility jamo of the 19 initials, 21 vowels and 27 finals.
const INITIALS: [char; 19] = [
    'ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ',
    'ㅌ', 'ㅍ', 'ㅎ',
];
const VOWELS: [char; 21] = [
    'ㅏ', 'ㅐ', 'ㅑ', 'ㅒ', 'ㅓ', 'ㅔ', 'ㅕ', 'ㅖ', 'ㅗ', 'ㅘ', 'ㅙ', 'ㅚ', 'ㅛ', 'ㅜ', 'ㅝ', 'ㅞ',
    'ㅟ', 'ㅠ', 'ㅡ', 'ㅢ', 'ㅣ',
];
const FINALS: [char; 27] = [
    'ㄱ', 'ㄲ', 'ㄳ', 'ㄴ', 'ㄵ', 'ㄶ', 'ㄷ', 'ㄹ', 'ㄺ', 'ㄻ', 'ㄼ', 'ㄽ', 'ㄾ', 'ㄿ', 'ㅀ', 'ㅁ',
    'ㅂ', 'ㅄ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅊ', 'ㅋ', 'ㅌ', 'ㅍ', 'ㅎ',
];
const COMPOUND_VOWELS: [(char, char, char); 7] = [
    ('ㅗ', 'ㅏ', 'ㅘ'),
    ('ㅗ', 'ㅐ', 'ㅙ'),
    ('ㅗ', 'ㅣ', 'ㅚ'),
    ('ㅜ', 'ㅓ', 'ㅝ'),
    ('ㅜ', 'ㅔ', 'ㅞ'),
    ('ㅜ', 'ㅣ', 'ㅟ'),
    ('ㅡ', 'ㅣ', 'ㅢ'),
];
const COMPOUND_FINALS: [(char, char, char); 11] = [
    ('ㄱ', 'ㅅ', 'ㄳ'),
    ('ㄴ', 'ㅈ', 'ㄵ'),
    ('ㄴ', 'ㅎ', 'ㄶ'),
    ('ㄹ', 'ㄱ', 'ㄺ'),
    ('ㄹ', 'ㅁ', 'ㄻ'),
    ('ㄹ', 'ㅂ', 'ㄼ'),
    ('ㄹ', 'ㅅ', 'ㄽ'),
    ('ㄹ', 'ㅌ', 'ㄾ'),
    ('ㄹ', 'ㅍ', 'ㄿ'),
    ('ㄹ', 'ㅎ', 'ㅀ'),
    ('ㅂ', 'ㅅ', 'ㅄ'),
];
/// Dubeolsik: QWERTY letter (lower case) to jamo; Shift gives the tense
/// consonants and ㅒ/ㅖ.
pub fn jamo(letter: char, shift: bool) -> Option<char> {
    let shifted = match letter {
        'q' => Some('ㅃ'),
        'w' => Some('ㅉ'),
        'e' => Some('ㄸ'),
        'r' => Some('ㄲ'),
        't' => Some('ㅆ'),
        'o' => Some('ㅒ'),
        'p' => Some('ㅖ'),
        _ => None,
    };
    if shift && shifted.is_some() {
        return shifted;
    }
    let i = (letter as u32).checked_sub('a' as u32)? as usize;
    "ㅁㅠㅊㅇㄷㄹㅎㅗㅑㅓㅏㅣㅡㅜㅐㅔㅂㄱㄴㅅㅕㅍㅈㅌㅛㅋ"
        .chars()
        .nth(i)
        .filter(|_| i < 26)
}
fn is_vowel(c: char) -> bool {
    VOWELS.contains(&c)
}
fn index(table: &[char], c: char) -> Option<usize> {
    table.iter().position(|x| *x == c)
}
fn combine(table: &[(char, char, char)], a: char, b: char) -> Option<char> {
    table.iter().find(|t| t.0 == a && t.1 == b).map(|t| t.2)
}
/// The text of a keystroke sequence if it forms one syllable (or a lone
/// jamo), following the 2-set grammar: C? V V? (F F?)?.
fn syllable(keys: &[char]) -> Option<char> {
    match keys {
        [] => None,
        [c] => Some(*c),
        [v, w] if is_vowel(*v) => combine(&COMPOUND_VOWELS, *v, *w),
        [l, rest @ ..] if !is_vowel(*l) && rest.first().is_some_and(|v| is_vowel(*v)) => {
            let li = index(&INITIALS, *l)?;
            let (vowel, tail) = match rest {
                [v, w, tail @ ..] if is_vowel(*w) => (combine(&COMPOUND_VOWELS, *v, *w)?, tail),
                [v, tail @ ..] => (*v, tail),
                [] => return None,
            };
            let vi = index(&VOWELS, vowel)?;
            let fi = match tail {
                [] => 0,
                [f] => index(&FINALS, *f)? + 1,
                [f, h] => index(&FINALS, combine(&COMPOUND_FINALS, *f, *h)?)? + 1,
                _ => return None,
            };
            char::from_u32(0xac00 + ((li * 21 + vi) * 28 + fi) as u32)
        }
        _ => None,
    }
}
#[derive(Default)]
pub struct Hangul {
    keys: Vec<char>,
    english: bool,
}
impl Hangul {
    pub fn new() -> Self {
        Self::default()
    }
    fn text(&self) -> String {
        syllable(&self.keys).map(String::from).unwrap_or_default()
    }
    fn preedit(&self) -> Preedit {
        let text = self.text();
        let n = text.len();
        Preedit {
            text,
            cursor: n,
            segments: if n > 0 {
                alloc::vec![Segment {
                    start: 0,
                    end: n,
                    style: ti::STYLE_INPUT
                }]
            } else {
                Vec::new()
            },
        }
    }
    fn take(&mut self) -> String {
        let t = self.text();
        self.keys.clear();
        t
    }
    fn push(&mut self, j: char) -> String {
        let mut next = self.keys.clone();
        next.push(j);
        if syllable(&next).is_some() {
            self.keys = next;
            return String::new();
        }
        // A vowel after a final consonant: the (last) final starts the next
        // syllable (도깨비불 rule).
        if is_vowel(j) && self.keys.len() >= 3 && !is_vowel(*self.keys.last().unwrap()) {
            let moved = self.keys.pop().unwrap();
            let done = self.take();
            self.keys = alloc::vec![moved, j];
            if syllable(&self.keys).is_some() {
                return done;
            }
            self.keys = alloc::vec![moved];
            let mut out = done;
            out.push_str(&self.take());
            self.keys.push(j);
            return out;
        }
        let done = self.take();
        self.keys.push(j);
        done
    }
}
impl Engine for Hangul {
    fn key(&mut self, k: &KeyIn) -> Output {
        if !k.down() {
            return Output {
                handled: false,
                preedit: self.preedit(),
                ..Output::default()
            };
        }
        if k.vk == vk::HANGUL {
            let commit = self.take();
            self.english = !self.english;
            return Output {
                handled: true,
                commit,
                ..Output::default()
            };
        }
        if k.vk == vk::HANJA {
            // Hanja conversion needs a dictionary; the syllable stays.
            return Output {
                handled: true,
                preedit: self.preedit(),
                ..Output::default()
            };
        }
        let letter = k.vk.is_ascii_uppercase().then(|| (k.vk + 32) as char);
        if !self.english
            && !k.chord()
            && let Some(j) = letter.and_then(|l| jamo(l, k.shift()))
        {
            let commit = self.push(j);
            return Output {
                handled: true,
                commit,
                preedit: self.preedit(),
                ..Output::default()
            };
        }
        if k.vk == vk::BACK && !self.keys.is_empty() && !k.chord() {
            self.keys.pop();
            return Output {
                handled: true,
                preedit: self.preedit(),
                ..Output::default()
            };
        }
        if k.vk == vk::ESCAPE && !self.keys.is_empty() {
            // Microsoft IME commits the syllable on Escape.
            return Output {
                handled: true,
                commit: self.take(),
                ..Output::default()
            };
        }
        // Anything else commits the syllable and reaches the application.
        Output {
            handled: false,
            commit: self.take(),
            ..Output::default()
        }
    }
    fn cancel(&mut self) {
        self.keys.clear();
    }
    fn complete(&mut self) -> String {
        self.take()
    }
    fn mode(&self) -> u32 {
        if self.english {
            ti::MODE_DIRECT
        } else {
            ti::MODE_NATIVE
        }
    }
    fn composing(&self) -> bool {
        !self.keys.is_empty()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn syllable_arithmetic() {
        assert_eq!(syllable(&['ㅎ', 'ㅏ', 'ㄴ']), Some('한'));
        assert_eq!(syllable(&['ㄱ', 'ㅡ', 'ㄹ']), Some('글'));
        assert_eq!(syllable(&['ㄷ', 'ㅗ', 'ㅐ']), Some('돼'));
        assert_eq!(syllable(&['ㄷ', 'ㅏ', 'ㄹ', 'ㄱ']), Some('닭'));
        assert_eq!(syllable(&['ㅇ', 'ㅏ', 'ㄸ']), None, "ㄸ is never a final");
        assert_eq!(syllable(&['ㅗ', 'ㅏ']), Some('ㅘ'));
        assert_eq!(jamo('r', true), Some('ㄲ'));
        assert_eq!(jamo('k', true), Some('ㅏ'));
    }
}

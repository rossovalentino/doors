//! Romaji to hiragana (Microsoft IME romaji table: Hepburn, Kunrei and
//! wāpuro spellings, small kana with x/l, sokuon from doubled consonants,
//! ん from nn, n' or n before a consonant) and kana width/script helpers.
extern crate alloc;
use alloc::string::String;
/// (romaji, kana), longest spellings first where prefixes overlap.
const TABLE: &[(&str, &str)] = &[
    ("a", "あ"),
    ("i", "い"),
    ("u", "う"),
    ("e", "え"),
    ("o", "お"),
    ("ka", "か"),
    ("ki", "き"),
    ("ku", "く"),
    ("ke", "け"),
    ("ko", "こ"),
    ("ga", "が"),
    ("gi", "ぎ"),
    ("gu", "ぐ"),
    ("ge", "げ"),
    ("go", "ご"),
    ("sa", "さ"),
    ("si", "し"),
    ("shi", "し"),
    ("su", "す"),
    ("se", "せ"),
    ("so", "そ"),
    ("za", "ざ"),
    ("zi", "じ"),
    ("ji", "じ"),
    ("zu", "ず"),
    ("ze", "ぜ"),
    ("zo", "ぞ"),
    ("ta", "た"),
    ("ti", "ち"),
    ("chi", "ち"),
    ("tu", "つ"),
    ("tsu", "つ"),
    ("te", "て"),
    ("to", "と"),
    ("da", "だ"),
    ("di", "ぢ"),
    ("du", "づ"),
    ("de", "で"),
    ("do", "ど"),
    ("na", "な"),
    ("ni", "に"),
    ("nu", "ぬ"),
    ("ne", "ね"),
    ("no", "の"),
    ("ha", "は"),
    ("hi", "ひ"),
    ("hu", "ふ"),
    ("fu", "ふ"),
    ("he", "へ"),
    ("ho", "ほ"),
    ("ba", "ば"),
    ("bi", "び"),
    ("bu", "ぶ"),
    ("be", "べ"),
    ("bo", "ぼ"),
    ("pa", "ぱ"),
    ("pi", "ぴ"),
    ("pu", "ぷ"),
    ("pe", "ぺ"),
    ("po", "ぽ"),
    ("ma", "ま"),
    ("mi", "み"),
    ("mu", "む"),
    ("me", "め"),
    ("mo", "も"),
    ("ya", "や"),
    ("yu", "ゆ"),
    ("yo", "よ"),
    ("ye", "いぇ"),
    ("ra", "ら"),
    ("ri", "り"),
    ("ru", "る"),
    ("re", "れ"),
    ("ro", "ろ"),
    ("la", "ぁ"),
    ("li", "ぃ"),
    ("lu", "ぅ"),
    ("le", "ぇ"),
    ("lo", "ぉ"),
    ("xa", "ぁ"),
    ("xi", "ぃ"),
    ("xu", "ぅ"),
    ("xe", "ぇ"),
    ("xo", "ぉ"),
    ("wa", "わ"),
    ("wi", "うぃ"),
    ("wu", "う"),
    ("we", "うぇ"),
    ("wo", "を"),
    ("nn", "ん"),
    ("n'", "ん"),
    ("xn", "ん"),
    ("kya", "きゃ"),
    ("kyi", "きぃ"),
    ("kyu", "きゅ"),
    ("kye", "きぇ"),
    ("kyo", "きょ"),
    ("gya", "ぎゃ"),
    ("gyi", "ぎぃ"),
    ("gyu", "ぎゅ"),
    ("gye", "ぎぇ"),
    ("gyo", "ぎょ"),
    ("sya", "しゃ"),
    ("syu", "しゅ"),
    ("sye", "しぇ"),
    ("syo", "しょ"),
    ("sha", "しゃ"),
    ("shu", "しゅ"),
    ("she", "しぇ"),
    ("sho", "しょ"),
    ("zya", "じゃ"),
    ("zyu", "じゅ"),
    ("zye", "じぇ"),
    ("zyo", "じょ"),
    ("ja", "じゃ"),
    ("ju", "じゅ"),
    ("je", "じぇ"),
    ("jo", "じょ"),
    ("jya", "じゃ"),
    ("jyu", "じゅ"),
    ("jye", "じぇ"),
    ("jyo", "じょ"),
    ("tya", "ちゃ"),
    ("tyu", "ちゅ"),
    ("tye", "ちぇ"),
    ("tyo", "ちょ"),
    ("cha", "ちゃ"),
    ("chu", "ちゅ"),
    ("che", "ちぇ"),
    ("cho", "ちょ"),
    ("cya", "ちゃ"),
    ("cyu", "ちゅ"),
    ("cye", "ちぇ"),
    ("cyo", "ちょ"),
    ("dya", "ぢゃ"),
    ("dyu", "ぢゅ"),
    ("dye", "ぢぇ"),
    ("dyo", "ぢょ"),
    ("nya", "にゃ"),
    ("nyi", "にぃ"),
    ("nyu", "にゅ"),
    ("nye", "にぇ"),
    ("nyo", "にょ"),
    ("hya", "ひゃ"),
    ("hyi", "ひぃ"),
    ("hyu", "ひゅ"),
    ("hye", "ひぇ"),
    ("hyo", "ひょ"),
    ("bya", "びゃ"),
    ("byi", "びぃ"),
    ("byu", "びゅ"),
    ("bye", "びぇ"),
    ("byo", "びょ"),
    ("pya", "ぴゃ"),
    ("pyi", "ぴぃ"),
    ("pyu", "ぴゅ"),
    ("pye", "ぴぇ"),
    ("pyo", "ぴょ"),
    ("mya", "みゃ"),
    ("myi", "みぃ"),
    ("myu", "みゅ"),
    ("mye", "みぇ"),
    ("myo", "みょ"),
    ("rya", "りゃ"),
    ("ryi", "りぃ"),
    ("ryu", "りゅ"),
    ("rye", "りぇ"),
    ("ryo", "りょ"),
    ("fa", "ふぁ"),
    ("fi", "ふぃ"),
    ("fe", "ふぇ"),
    ("fo", "ふぉ"),
    ("fyu", "ふゅ"),
    ("va", "ゔぁ"),
    ("vi", "ゔぃ"),
    ("vu", "ゔ"),
    ("ve", "ゔぇ"),
    ("vo", "ゔぉ"),
    ("tsa", "つぁ"),
    ("tsi", "つぃ"),
    ("tse", "つぇ"),
    ("tso", "つぉ"),
    ("thi", "てぃ"),
    ("thu", "てゅ"),
    ("dhi", "でぃ"),
    ("dhu", "でゅ"),
    ("twu", "とぅ"),
    ("dwu", "どぅ"),
    ("kwa", "くぁ"),
    ("gwa", "ぐぁ"),
    ("lya", "ゃ"),
    ("lyu", "ゅ"),
    ("lyo", "ょ"),
    ("xya", "ゃ"),
    ("xyu", "ゅ"),
    ("xyo", "ょ"),
    ("ltu", "っ"),
    ("xtu", "っ"),
    ("ltsu", "っ"),
    ("xtsu", "っ"),
    ("lwa", "ゎ"),
    ("xwa", "ゎ"),
    ("lka", "ゕ"),
    ("lke", "ゖ"),
    ("-", "ー"),
    (",", "、"),
    (".", "。"),
    ("[", "「"),
    ("]", "」"),
    ("/", "・"),
    ("~", "～"),
];
fn exact(s: &str) -> Option<&'static str> {
    TABLE.iter().find(|e| e.0 == s).map(|e| e.1)
}
fn prefix(s: &str) -> bool {
    TABLE
        .iter()
        .any(|e| e.0.len() > s.len() && e.0.starts_with(s))
}
fn consonant(c: char) -> bool {
    c.is_ascii_lowercase() && !"aeiou".contains(c)
}
/// Feed one ASCII character into the pending romaji `tail`; converted kana
/// is appended to `out`. Returns false when `c` is not romaji input.
pub fn feed(tail: &mut String, out: &mut String, c: char) -> bool {
    let c = c.to_ascii_lowercase();
    if !(c.is_ascii_lowercase() || "-,.[]/~'".contains(c)) {
        return false;
    }
    tail.push(c);
    loop {
        if let Some(k) = exact(tail) {
            if !prefix(tail) {
                out.push_str(k);
                tail.clear();
            }
            return true;
        }
        if prefix(tail) {
            return true;
        }
        // Doubled consonant (not n): っ and keep the second consonant.
        let b = tail.as_bytes();
        if b.len() >= 2 && b[0] == b[1] && consonant(b[0] as char) && b[0] != b'n' {
            out.push('っ');
            tail.remove(0);
            continue;
        }
        // n before a consonant (other than y) is ん.
        if b.len() >= 2 && b[0] == b'n' && consonant(b[1] as char) && b[1] != b'y' {
            out.push('ん');
            tail.remove(0);
            continue;
        }
        // Not romaji: keep the first character as typed.
        let first = tail.remove(0);
        out.push(first);
        if tail.is_empty() {
            return true;
        }
    }
}
/// Flush a pending tail at the end of input (a final n becomes ん).
pub fn flush(tail: &mut String, out: &mut String) {
    if tail == "n" {
        out.push('ん');
    } else {
        out.push_str(tail);
    }
    tail.clear();
}
/// Hiragana to full-width katakana (F7).
pub fn katakana(s: &str) -> String {
    s.chars()
        .map(|c| {
            if ('ぁ'..='ゖ').contains(&c) {
                char::from_u32(c as u32 + 0x60).unwrap()
            } else {
                c
            }
        })
        .collect()
}
/// Full-width ASCII forms (F9) and back (F10).
pub fn full_width(s: &str) -> String {
    s.chars()
        .map(|c| {
            if ('!'..='~').contains(&c) {
                char::from_u32(c as u32 + 0xfee0).unwrap()
            } else if c == ' ' {
                '　'
            } else {
                c
            }
        })
        .collect()
}
/// Romaji of hiragana for F9/F10 (inverse of the primary table entries).
pub fn romaji(s: &str) -> String {
    let mut out = String::new();
    let chars: alloc::vec::Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == 'っ' && i + 1 < chars.len() {
            if let Some(r) = romaji_of(chars[i + 1], chars.get(i + 2).copied()).map(|(r, _)| r) {
                out.push(r.chars().next().unwrap());
            }
            i += 1;
            continue;
        }
        match romaji_of(chars[i], chars.get(i + 1).copied()) {
            Some((r, n)) => {
                out.push_str(r);
                i += n;
            }
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}
/// Hepburn spellings preferred when a kana has several.
const PREFERRED: &[(&str, &str)] = &[
    ("し", "shi"),
    ("じ", "ji"),
    ("ち", "chi"),
    ("つ", "tsu"),
    ("ふ", "fu"),
    ("しゃ", "sha"),
    ("しゅ", "shu"),
    ("しょ", "sho"),
    ("じゃ", "ja"),
    ("じゅ", "ju"),
    ("じょ", "jo"),
    ("ちゃ", "cha"),
    ("ちゅ", "chu"),
    ("ちょ", "cho"),
];
fn spell(kana: &str) -> Option<&'static str> {
    PREFERRED
        .iter()
        .find(|e| e.0 == kana)
        .map(|e| e.1)
        .or_else(|| {
            TABLE
                .iter()
                .find(|e| e.1 == kana && !e.0.starts_with(['x', 'l']) && e.0 != "nn")
                .map(|e| e.0)
        })
}
fn romaji_of(c: char, next: Option<char>) -> Option<(&'static str, usize)> {
    if c == 'ん' {
        let vowel_next = next
            .and_then(|n| spell(n.encode_utf8(&mut [0; 4])))
            .is_some_and(|r| r.starts_with(['a', 'i', 'u', 'e', 'o', 'y', 'n']));
        return Some((if vowel_next { "nn" } else { "n" }, 1));
    }
    if let Some(n) = next {
        let mut pair = String::new();
        pair.push(c);
        pair.push(n);
        if let Some(r) = spell(&pair) {
            return Some((r, 2));
        }
    }
    spell(c.encode_utf8(&mut [0; 4])).map(|r| (r, 1))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn kana(input: &str) -> String {
        let (mut tail, mut out) = (String::new(), String::new());
        for c in input.chars() {
            feed(&mut tail, &mut out, c);
        }
        flush(&mut tail, &mut out);
        out
    }
    #[test]
    fn romaji_conversion_matches_the_microsoft_table() {
        assert_eq!(kana("nihongo"), "にほんご");
        assert_eq!(kana("kitte"), "きって");
        assert_eq!(kana("shinbun"), "しんぶん");
        assert_eq!(kana("kon'ya"), "こんや");
        assert_eq!(kana("konnya"), "こんや");
        assert_eq!(kana("tokyo"), "ときょ");
        assert_eq!(kana("toukyou"), "とうきょう");
        assert_eq!(kana("chotto"), "ちょっと");
        assert_eq!(kana("ra-men"), "らーめん");
        assert_eq!(kana("fairu"), "ふぁいる");
        assert_eq!(kana("xtu"), "っ");
        assert_eq!(kana("n"), "ん");
        assert_eq!(katakana("こんぴゅーた"), "コンピュータ");
        assert_eq!(full_width("Ab1"), "Ａｂ１");
        assert_eq!(romaji("にほんご"), "nihongo");
    }
}

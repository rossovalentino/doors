//! door.text-input records to engine calls, and engine output to IME
//! requests (`Action`), in the order the protocol requires: commit, then
//! preedit, then candidates, then the key verdict.
extern crate alloc;
use crate::dict::Dictionary;
use crate::engine::{Engine, KeyIn, Output, Page, Preedit};
use crate::{hangul::Hangul, japanese::Japanese, pinyin::Pinyin};
use alloc::{string::String, vec::Vec};
use door_keyboard_layouts::{Ime, methods};
use door_service_protocol::text_input::{self as ti, Record};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Commit { serial: u32, text: String },
    Preedit { serial: u32, preedit: Preedit },
    Candidates { serial: u32, page: Option<Page> },
    KeyDone { key_serial: u32, forward: bool },
    Mode(u32),
}
pub struct Session {
    pinyin: Pinyin,
    japanese: Japanese,
    hangul: Hangul,
    ime: Ime,
    active: Option<u32>,
    preedit: Preedit,
    page: Option<Page>,
    mode: Option<u32>,
}
impl Session {
    pub fn new(pinyin: Dictionary, japanese: Dictionary) -> Self {
        Self {
            pinyin: Pinyin::new(pinyin),
            japanese: Japanese::new(japanese),
            hangul: Hangul::new(),
            ime: Ime::None,
            active: None,
            preedit: Preedit::default(),
            page: None,
            mode: None,
        }
    }
    fn engine(&mut self) -> Option<&mut dyn Engine> {
        match self.ime {
            Ime::Pinyin => Some(&mut self.pinyin),
            Ime::Japanese => Some(&mut self.japanese),
            Ime::Korean => Some(&mut self.hangul),
            Ime::None => None,
        }
    }
    pub fn set_method(&mut self, id: u32, out: &mut Vec<Action>) {
        self.ime = u8::try_from(id)
            .ok()
            .and_then(methods::method)
            .map_or(Ime::None, |m| m.ime);
        self.sync_mode(out);
    }
    fn sync_mode(&mut self, out: &mut Vec<Action>) {
        if let Some(mode) = self.engine().map(|e| e.mode())
            && self.mode != Some(mode)
        {
            self.mode = Some(mode);
            out.push(Action::Mode(mode));
        }
    }
    /// Commit (or drop) the composition of `serial` and clear the UI.
    fn finish(&mut self, serial: u32, commit: bool, out: &mut Vec<Action>) {
        let text = match self.engine() {
            Some(e) if commit => e.complete(),
            Some(e) => {
                e.cancel();
                String::new()
            }
            None => String::new(),
        };
        if !text.is_empty() {
            out.push(Action::Commit { serial, text });
        } else if !self.preedit.text.is_empty() {
            out.push(Action::Preedit {
                serial,
                preedit: Preedit::default(),
            });
        }
        if self.page.take().is_some() {
            out.push(Action::Candidates { serial, page: None });
        }
        self.preedit = Preedit::default();
    }
    fn apply(&mut self, serial: u32, o: Output, out: &mut Vec<Action>) {
        if !o.commit.is_empty() {
            out.push(Action::Commit {
                serial,
                text: o.commit,
            });
            self.preedit = Preedit::default();
        }
        if o.preedit != self.preedit {
            self.preedit = o.preedit.clone();
            out.push(Action::Preedit {
                serial,
                preedit: o.preedit,
            });
        }
        if o.candidates != self.page {
            self.page = o.candidates.clone();
            out.push(Action::Candidates {
                serial,
                page: o.candidates,
            });
        }
    }
    pub fn record(&mut self, r: &Record<'_>, out: &mut Vec<Action>) {
        match *r {
            Record::Method { method } => self.set_method(method, out),
            Record::Activate {
                serial,
                hints,
                method,
                ..
            } => {
                self.set_method(method, out);
                self.active = Some(serial);
                if let Some(e) = self.engine() {
                    e.set_private(hints & ti::HINT_PRIVATE != 0);
                }
            }
            Record::Deactivate { serial } => {
                if self.active == Some(serial) {
                    self.finish(serial, true, out);
                    self.active = None;
                }
            }
            Record::Reset { serial, mode } => {
                if self.active == Some(serial) {
                    self.finish(serial, mode == ti::RESET_COMPLETE, out);
                }
            }
            Record::ImeKey {
                key_serial,
                serial,
                key,
            } => {
                let output = match (self.active == Some(serial), self.engine()) {
                    (true, Some(e)) => e.key(&KeyIn::from_key(&key)),
                    _ => Output::forward(),
                };
                let forward = !output.handled;
                if self.active == Some(serial) {
                    self.apply(serial, output, out);
                }
                out.push(Action::KeyDone {
                    key_serial,
                    forward,
                });
                self.sync_mode(out);
            }
            _ => {}
        }
    }
}
/// Protocol preedit: attributes from segments, text truncated to
/// `ti::MAX_PREEDIT` on a grapheme-cluster boundary (the composition itself
/// is kept; a commit carries all of it, split by `ime_commit`).
pub fn wire_preedit(p: &Preedit, attrs: &mut [ti::Attr; ti::MAX_ATTRS]) -> (usize, usize, u16) {
    let len = ti::grapheme_floor(&p.text, ti::MAX_PREEDIT);
    let mut n = 0;
    for s in p.segments.iter().take(ti::MAX_ATTRS) {
        let (start, end) = (s.start.min(len), s.end.min(len));
        if start < end {
            attrs[n] = ti::Attr {
                start: start as u16,
                end: end as u16,
                style: s.style,
            };
            n += 1;
        }
    }
    let cursor = if p.cursor <= len {
        p.cursor as u16
    } else {
        len as u16
    };
    (len, n, cursor)
}
#[cfg(test)]
mod tests {
    use super::*;
    use door_abi::graphics as g;
    fn ime_key(serial: u32, key_serial: u32, vk: u16, text: &str) -> Record<'static> {
        let key = ti::Key {
            code: 0x10,
            flags: g::DOWN,
            vk,
            ..ti::Key::default()
        }
        .with_text(text);
        Record::ImeKey {
            key_serial,
            serial,
            key,
        }
    }
    #[test]
    fn korean_session_orders_commit_preedit_and_verdicts() {
        let mut s = Session::new(Dictionary::empty(), Dictionary::empty());
        let mut out = Vec::new();
        s.record(
            &Record::Activate {
                serial: 4,
                content: 0,
                hints: 0,
                method: 9,
                scale: 100,
                caret: Default::default(),
            },
            &mut out,
        );
        assert_eq!(out, [Action::Mode(ti::MODE_NATIVE)]);
        out.clear();
        for (i, (vk, t)) in [(b'G', "g"), (b'K', "k"), (b'S', "s"), (b'K', "k")]
            .into_iter()
            .enumerate()
        {
            s.record(&ime_key(4, i as u32 + 1, u16::from(vk), t), &mut out);
        }
        let commits: Vec<&str> = out
            .iter()
            .filter_map(|a| {
                if let Action::Commit { text, .. } = a {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(commits, ["하"]);
        assert!(matches!(
            out.last(),
            Some(Action::KeyDone {
                key_serial: 4,
                forward: false
            })
        ));
        out.clear();
        // Space commits 나 and is forwarded; a stale context is forwarded.
        s.record(&ime_key(4, 5, 0x20, " "), &mut out);
        assert_eq!(
            out[0],
            Action::Commit {
                serial: 4,
                text: "나".into()
            }
        );
        assert_eq!(
            out.last(),
            Some(&Action::KeyDone {
                key_serial: 5,
                forward: true
            })
        );
        out.clear();
        s.record(&ime_key(3, 6, u16::from(b'G'), "g"), &mut out);
        assert_eq!(
            out,
            [Action::KeyDone {
                key_serial: 6,
                forward: true
            }]
        );
        out.clear();
        s.record(&ime_key(4, 7, u16::from(b'G'), "g"), &mut out);
        out.clear();
        s.record(&Record::Deactivate { serial: 4 }, &mut out);
        assert_eq!(
            out,
            [Action::Commit {
                serial: 4,
                text: "ㅎ".into()
            }],
            "commit on focus change"
        );
        let p = Preedit {
            text: "日本".into(),
            cursor: 6,
            segments: alloc::vec![crate::engine::Segment {
                start: 0,
                end: 6,
                style: 1
            }],
        };
        let mut attrs = [ti::Attr::default(); ti::MAX_ATTRS];
        assert_eq!(wire_preedit(&p, &mut attrs), (6, 1, 6));
    }
    #[test]
    fn long_preedit_is_cut_on_a_grapheme_and_always_encodes() {
        // 170 bytes of kanji, then a ZWJ family straddling the cap.
        let mut text: String = "日".repeat(56);
        text.push_str("a👨\u{200d}👩\u{200d}👧");
        let segments = (0..12)
            .map(|i| crate::engine::Segment {
                start: i * 3,
                end: i * 3 + 3,
                style: 1,
            })
            .collect();
        let p = Preedit {
            cursor: text.len(),
            text,
            segments,
        };
        let mut attrs = [ti::Attr::default(); ti::MAX_ATTRS];
        let (len, n, cursor) = wire_preedit(&p, &mut attrs);
        assert_eq!((len, n, cursor as usize), (169, ti::MAX_ATTRS, 169));
        let wire = ti::Preedit {
            text: &p.text[..len],
            cursor,
            attrs: &attrs[..n],
        };
        let mut body = [0u8; door_service_protocol::BODY_BYTES];
        ti::encode_ime_preedit(1, wire, &mut body).unwrap();
        let record = Record::Preedit {
            window: 1,
            serial: 1,
            preedit: ti::PreeditBuf {
                text: wire.text,
                cursor,
                count: n,
                attrs,
            },
        };
        assert!(record.encode(&mut [0u8; ti::MAX_RECORD]).is_ok());
    }
}

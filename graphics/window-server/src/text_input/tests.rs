extern crate std;
use super::*;
use std::string::String;
use std::vec;
const APP: Owner = Owner {
    pid: 10,
    session: 1,
    epoch: 1,
};
const OTHER: Owner = Owner {
    pid: 11,
    session: 1,
    epoch: 1,
};
const IME: Owner = Owner {
    pid: 12,
    session: 1,
    epoch: 1,
};
const W: u64 = 5;
const W2: u64 = 6;
struct Fake {
    focus: Option<(Owner, u64)>,
    windows: Vec<(Owner, u64)>,
    locked: bool,
    now: u64,
}
impl Env for Fake {
    fn focused(&self) -> Option<(Owner, u64)> {
        self.focus
    }
    fn owns(&self, owner: Owner, window: u64) -> bool {
        self.windows.contains(&(owner, window))
    }
    fn content_origin(&self, _: u64) -> Option<(i32, i32)> {
        Some((100, 200))
    }
    fn scale_percent(&self) -> u32 {
        150
    }
    fn locked(&self) -> bool {
        self.locked
    }
    fn now(&self) -> u64 {
        self.now
    }
}
fn env() -> Fake {
    Fake {
        focus: Some((APP, W)),
        windows: vec![(APP, W), (APP, W2), (OTHER, 7)],
        locked: false,
        now: 1000,
    }
}
fn key(code: u32, flags: u32) -> d::Input {
    d::Input {
        kind: d::KEY,
        code,
        flags,
        timestamp: 9,
        ..d::Input::default()
    }
}
fn caret() -> Rect {
    Rect {
        x: 40,
        y: 10,
        width: 2,
        height: 20,
    }
}
fn enable(b: &mut Broker, e: &Fake, window: u64, content: u32) -> u32 {
    let body = ti::Enable {
        window,
        content,
        hints: 0,
        caret: caret(),
    }
    .encode();
    match b.request(e, Peer::App, APP, ti::ENABLE, &body).unwrap() {
        Reply::Scalar(s) => s as u32,
        _ => panic!(),
    }
}
/// Owned copies of polled records, for assertions.
#[derive(Debug, PartialEq, Eq)]
enum R {
    Key(u64, String, u32),
    Preedit(u64, String),
    Commit(u64, String),
    Delete(u32, u32),
    Disabled(u32),
    Activate(u32, u32),
    Deactivate(u32),
    Surrounding(String),
    Cursor(i32, i32),
    ImeKey(u32, String),
    Method(u32),
    Reset(u32),
}
fn poll(b: &mut Broker, e: &Fake, peer: Peer, owner: Owner) -> Vec<R> {
    let mut out = Vec::new();
    while let Ok(Reply::Bytes(bytes)) = b.request(e, peer, owner, ti::POLL, &[]) {
        for r in ti::records(&bytes) {
            out.push(match r.unwrap() {
                Record::Key { window, key } => R::Key(window, key.text().into(), key.code),
                Record::Preedit {
                    window, preedit, ..
                } => R::Preedit(window, preedit.text.into()),
                Record::Commit { window, text, .. } => R::Commit(window, text.into()),
                Record::Delete { before, after, .. } => R::Delete(before, after),
                Record::Disabled { reason, .. } => R::Disabled(reason),
                Record::Activate {
                    serial, content, ..
                } => R::Activate(serial, content),
                Record::Deactivate { serial } => R::Deactivate(serial),
                Record::Surrounding { surrounding, .. } => R::Surrounding(surrounding.text.into()),
                Record::Cursor { caret, .. } => R::Cursor(caret.x, caret.y),
                Record::ImeKey {
                    key_serial, key, ..
                } => R::ImeKey(key_serial, key.text().into()),
                Record::Method { method } => R::Method(method),
                Record::Reset { mode, .. } => R::Reset(mode),
            });
        }
    }
    out
}
fn ime(b: &mut Broker, e: &Fake, op: u32, body: &[u8]) -> Result<Reply, Code> {
    b.request(e, Peer::Ime, IME, op, body)
}
fn words(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|w| w.to_le_bytes()).collect()
}
/// Pinyin installed and current, the IME registered.
fn pinyin(e: &Fake) -> Broker {
    let mut b = Broker::new();
    let p = Preferences::new(&[1, 7], 1, false).unwrap();
    b.request(
        e,
        Peer::Init,
        OTHER,
        ti::SET_PREFERENCES,
        &p.bits().to_le_bytes(),
    )
    .unwrap();
    ime(
        &mut b,
        e,
        ti::IME_REGISTER,
        &words(&[ti::VERSION, kl::methods::IME_METHODS]),
    )
    .unwrap();
    b
}
#[test]
fn windows_without_a_context_keep_legacy_codes_from_the_session_layout() {
    let e = env();
    let mut b = Broker::new();
    assert_eq!(
        b.key(&e, &key(0x1e, d::DOWN | d::NUM_LOCK)),
        KeyRoute::Legacy(0x1e | (u32::from(b'a') << 16))
    );
    let p = Preferences::new(&[5], 0, false).unwrap();
    b.request(
        &e,
        Peer::Settings,
        APP,
        ti::SET_METHODS,
        &p.bits().to_le_bytes(),
    )
    .unwrap();
    // German: the Y position types z; AltGr+Q is @ without Ctrl/Alt bits.
    assert_eq!(
        b.key(&e, &key(0x15, d::DOWN)),
        KeyRoute::Legacy(0x15 | (u32::from(b'z') << 16))
    );
    let altgr = d::DOWN | d::ALT | d::RIGHT_ALT;
    assert_eq!(
        b.key(&e, &key(0x10, altgr)),
        KeyRoute::Legacy(0x10 | (d::RIGHT_ALT << 9) | (u32::from(b'@') << 16))
    );
    // Lock state never reaches the ASCII byte.
    let KeyRoute::Legacy(c) = b.key(
        &e,
        &key(
            0x1e,
            d::DOWN | d::CAPS_LOCK | d::NUM_LOCK | d::SCROLL_LOCK | d::REPEAT,
        ),
    ) else {
        panic!()
    };
    assert_eq!(
        (c >> 16, c & 511, (c >> 9) & 127),
        (u32::from(b'A'), 0x1e, d::REPEAT)
    );
    assert!(b.indicator().is_none(), "one keyboard: no indicator");
}
#[test]
fn an_enabled_context_receives_ordered_unicode_key_records_and_one_pending_event() {
    let e = env();
    let mut b = Broker::new();
    b.listen(APP);
    let p = Preferences::new(&[4], 0, false).unwrap();
    b.request(
        &e,
        Peer::Settings,
        APP,
        ti::SET_METHODS,
        &p.bits().to_le_bytes(),
    )
    .unwrap();
    enable(&mut b, &e, W, ti::NORMAL);
    for code in [0x1a, 0x28, 0x12] {
        assert_eq!(b.key(&e, &key(code, d::DOWN)), KeyRoute::Consumed);
        b.key(&e, &key(code, 0));
    }
    let ev = b.take_event(APP).unwrap();
    assert_eq!((ev.kind, ev.a), (ti::PENDING, 6));
    assert!(b.take_event(APP).is_none(), "coalesced");
    let r = poll(&mut b, &e, Peer::App, APP);
    let texts: String = r
        .iter()
        .filter_map(|r| {
            if let R::Key(_, t, _) = r {
                Some(t.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!((r.len(), texts.as_str()), (6, "èàe"));
    // Another window of the same app has no context: legacy.
    let e2 = Fake {
        focus: Some((APP, W2)),
        ..env()
    };
    assert!(matches!(
        b.key(&e2, &key(0x1e, d::DOWN)),
        KeyRoute::Legacy(_)
    ));
    assert!(
        b.request(
            &e,
            Peer::App,
            OTHER,
            ti::ENABLE,
            &ti::Enable {
                window: W,
                content: 0,
                hints: 0,
                caret: caret()
            }
            .encode()
        )
        .is_err(),
        "foreign window"
    );
}
#[test]
fn ime_composition_flows_in_order_and_forwards_unhandled_keys() {
    let e = env();
    let mut b = pinyin(&e);
    let serial = enable(&mut b, &e, W, ti::NORMAL);
    let body = {
        let mut o = [0u8; 64];
        let n = ti::encode_surrounding(
            W,
            serial,
            ti::Surrounding {
                text: "你好",
                cursor: 6,
                anchor: 6,
            },
            &mut o,
        )
        .unwrap();
        o[..n].to_vec()
    };
    b.request(&e, Peer::App, APP, ti::SURROUNDING, &body)
        .unwrap();
    b.request(
        &e,
        Peer::App,
        APP,
        ti::CURSOR,
        &ti::encode_cursor(W, serial, Rect { x: 50, ..caret() }),
    )
    .unwrap();
    assert_eq!(b.key(&e, &key(0x31, d::DOWN)), KeyRoute::Consumed);
    assert_eq!(b.key(&e, &key(0x17, d::DOWN)), KeyRoute::Consumed);
    let r = poll(&mut b, &e, Peer::Ime, IME);
    assert_eq!(
        r,
        [
            R::Activate(serial, ti::NORMAL),
            R::Surrounding("你好".into()),
            R::Cursor(150, 210),
            R::ImeKey(1, "n".into()),
            R::ImeKey(2, "i".into())
        ]
    );
    let mut o = [0u8; 240];
    let n = ti::encode_ime_preedit(
        serial,
        ti::Preedit {
            text: "ni",
            cursor: 2,
            attrs: &[],
        },
        &mut o,
    )
    .unwrap();
    ime(&mut b, &e, ti::IME_PREEDIT, &o[..n]).unwrap();
    ime(&mut b, &e, ti::IME_KEY_DONE, &words(&[1, ti::CONSUMED])).unwrap();
    ime(&mut b, &e, ti::IME_KEY_DONE, &words(&[2, ti::CONSUMED])).unwrap();
    // Space commits; then an arrow key is forwarded after the commit.
    b.key(&e, &key(0x39, d::DOWN));
    b.key(&e, &key(0x14b, d::DOWN));
    let n = ti::encode_ime_commit(serial, "你", &mut o).unwrap();
    ime(&mut b, &e, ti::IME_COMMIT, &o[..n]).unwrap();
    ime(&mut b, &e, ti::IME_KEY_DONE, &words(&[3, ti::CONSUMED])).unwrap();
    ime(&mut b, &e, ti::IME_KEY_DONE, &words(&[4, ti::FORWARD])).unwrap();
    assert_eq!(
        poll(&mut b, &e, Peer::App, APP),
        [
            R::Preedit(W, "ni".into()),
            R::Commit(W, "你".into()),
            R::Key(W, "".into(), 0x14b)
        ]
    );
    // Candidates are drawn at the screen caret; IME_MODE drives the indicator.
    let n = ti::encode_candidates(
        ti::Candidates {
            serial,
            selected: 0,
            page: 0,
            pages: 2,
            flags: 0,
            items: &["你", "呢"],
        },
        &mut o,
    )
    .unwrap();
    ime(&mut b, &e, ti::IME_CANDIDATES, &o[..n]).unwrap();
    let c = b.candidates().unwrap();
    assert_eq!(
        (c.anchor.x, c.anchor.y, c.items.len(), c.scale),
        (150, 210, 2, 150)
    );
    ime(&mut b, &e, ti::IME_MODE, &ti::MODE_DIRECT.to_le_bytes()).unwrap();
    assert_eq!(
        b.indicator(),
        Some(kl::Indicator::Ime {
            glyph: "英",
            language: "zh-CN"
        })
    );
    assert!(b.take_dirty());
    // A stale serial is refused.
    let n = ti::encode_ime_commit(serial + 9, "x", &mut o).unwrap();
    assert_eq!(
        ime(&mut b, &e, ti::IME_COMMIT, &o[..n]).err(),
        Some(Code::Changed)
    );
}
#[test]
fn secure_and_latin_fields_bypass_the_ime_and_keep_no_text() {
    let e = env();
    let mut b = pinyin(&e);
    let serial = enable(&mut b, &e, W, ti::PASSWORD);
    let mut o = [0u8; 64];
    let n = ti::encode_surrounding(
        W,
        serial,
        ti::Surrounding {
            text: "hunter2",
            cursor: 7,
            anchor: 7,
        },
        &mut o,
    )
    .unwrap();
    assert_eq!(
        b.request(&e, Peer::App, APP, ti::SURROUNDING, &o[..n])
            .err(),
        Some(Code::Unsupported)
    );
    assert_eq!(b.key(&e, &key(0x23, d::DOWN)), KeyRoute::Consumed);
    assert!(
        poll(&mut b, &e, Peer::Ime, IME).is_empty(),
        "the IME learns nothing"
    );
    assert_eq!(
        poll(&mut b, &e, Peer::App, APP),
        [R::Key(W, "h".into(), 0x23)]
    );
    let n = ti::encode_ime_commit(serial, "x", &mut o).unwrap();
    assert_eq!(
        ime(&mut b, &e, ti::IME_COMMIT, &o[..n]).err(),
        Some(Code::Changed),
        "no injection"
    );
    enable(&mut b, &e, W, ti::URL);
    b.key(&e, &key(0x23, d::DOWN));
    assert!(poll(&mut b, &e, Peer::Ime, IME).is_empty());
}
#[test]
fn focus_change_commits_to_the_old_field_and_lock_cancels_everything() {
    let mut e = env();
    let mut b = pinyin(&e);
    let first = enable(&mut b, &e, W, ti::NORMAL);
    e.focus = Some((APP, W2));
    let second = enable(&mut b, &e, W2, ti::NORMAL);
    assert_eq!(
        poll(&mut b, &e, Peer::Ime, IME),
        [
            R::Activate(first, 0),
            R::Deactivate(first),
            R::Activate(second, 0)
        ]
    );
    let mut o = [0u8; 64];
    let n = ti::encode_ime_commit(first, "好", &mut o).unwrap();
    ime(&mut b, &e, ti::IME_COMMIT, &o[..n]).unwrap();
    assert_eq!(
        poll(&mut b, &e, Peer::App, APP),
        [R::Commit(W, "好".into())],
        "commit on blur"
    );
    b.key(&e, &key(0x31, d::DOWN));
    e.locked = true;
    assert_eq!(
        ime(&mut b, &e, ti::IME_MODE, &1u32.to_le_bytes()).err(),
        Some(Code::Denied)
    );
    assert_eq!(
        poll(&mut b, &e, Peer::Ime, IME),
        [
            R::ImeKey(1, "n".into()),
            R::Reset(ti::RESET_CANCEL),
            R::Deactivate(second)
        ]
    );
    assert_eq!(
        poll(&mut b, &e, Peer::App, APP),
        [R::Preedit(W2, "".into())]
    );
    assert!(ime(&mut b, &e, ti::IME_REGISTER, &words(&[ti::VERSION, 0])).is_err());
}
#[test]
fn a_hung_ime_is_bypassed_and_a_departed_one_releases_its_keys() {
    let mut e = env();
    let mut b = pinyin(&e);
    enable(&mut b, &e, W, ti::NORMAL);
    b.key(&e, &key(0x31, d::DOWN));
    e.now += ti::IME_KEY_TIMEOUT_TICKS;
    // The timeout forwards the key (its composition is cancelled and the
    // preedit cleared); the next key goes straight to the app.
    b.key(&e, &key(0x17, d::DOWN));
    assert_eq!(
        poll(&mut b, &e, Peer::App, APP),
        [
            R::Key(W, "n".into(), 0x31),
            R::Preedit(W, "".into()),
            R::Key(W, "i".into(), 0x17)
        ]
    );
    ime(&mut b, &e, ti::IME_KEY_DONE, &words(&[1, ti::FORWARD])).unwrap();
    b.key(&e, &key(0x12, d::DOWN));
    b.revoke(IME);
    assert_eq!(
        poll(&mut b, &e, Peer::App, APP),
        [R::Key(W, "e".into(), 0x12), R::Preedit(W, "".into())],
        "the departed IME's composition is cleared"
    );
    assert_eq!(
        b.request(&e, Peer::Ime, IME, ti::POLL, &[]).err(),
        Some(Code::Denied)
    );
}
#[test]
fn authority_hotkeys_and_overflow() {
    let e = env();
    let mut b = pinyin(&e);
    let p = Preferences::new(&[1, 7], 0, false)
        .unwrap()
        .bits()
        .to_le_bytes();
    assert_eq!(
        b.request(&e, Peer::App, APP, ti::SET_METHODS, &p).err(),
        Some(Code::Denied)
    );
    assert_eq!(
        b.request(&e, Peer::Settings, APP, ti::SET_PREFERENCES, &p)
            .err(),
        Some(Code::Denied)
    );
    assert_eq!(
        b.request(&e, Peer::App, APP, ti::IME_MODE, &[0; 4]).err(),
        Some(Code::Denied)
    );
    assert_eq!(
        b.request(&e, Peer::Ime, OTHER, ti::IME_REGISTER, &words(&[1, 0]))
            .err(),
        Some(Code::Exists)
    );
    assert_eq!(
        b.request(&e, Peer::Settings, APP, ti::SET_METHODS, &[0; 4])
            .err(),
        Some(Code::InvalidRequest)
    );
    let _ = poll(&mut b, &e, Peer::Ime, IME);
    // Win+Space: Pinyin -> US; the IME hears the method change.
    assert!(b.hotkey(&e, &key(0x39, d::DOWN | d::LOGO | d::LEFT_LOGO)));
    assert_eq!(b.method().id, 1);
    assert!(poll(&mut b, &e, Peer::Ime, IME).contains(&R::Method(1)));
    assert!(matches!(
        b.indicator(),
        Some(kl::Indicator::Keyboard {
            language: "ENG",
            keyboard: "US"
        })
    ));
    enable(&mut b, &e, W, ti::NORMAL);
    for _ in 0..ti::MAX_QUEUED_RECORDS {
        assert_eq!(b.key(&e, &key(0x1e, d::DOWN)), KeyRoute::Consumed);
    }
    assert!(
        matches!(b.key(&e, &key(0x1e, d::DOWN)), KeyRoute::Legacy(_)),
        "overflow falls back"
    );
    assert_eq!(
        poll(&mut b, &e, Peer::App, APP),
        [R::Disabled(ti::DISABLED_OVERFLOW)]
    );
    assert!(matches!(
        b.key(&e, &key(0x1e, d::DOWN)),
        KeyRoute::Legacy(_)
    ));
}
#[test]
fn candidate_window_follows_the_caret_scale_and_work_area() {
    let c = CandidateWindow {
        serial: 1,
        items: vec!["你好".into(), "拟好".into(), "你".into()],
        selected: 1,
        page: 0,
        pages: 3,
        vertical: false,
        anchor: Rect {
            x: 300,
            y: 100,
            width: 2,
            height: 30,
        },
        scale: 150,
    };
    let work = Rect {
        x: 0,
        y: 0,
        width: 1707,
        height: 1000,
    };
    let measure = |s: &str, px: u32| s.chars().count() as u32 * px * 3 / 2;
    let l = c.layout(work, &measure);
    assert_eq!(l.frame.y, 100 + 30 + 6, "below the caret, 4 px at 150%");
    assert_eq!(l.frame.height, 54);
    assert_eq!(
        l.runs
            .iter()
            .filter(|r| r.role == candidate::Role::Candidate)
            .count(),
        3
    );
    assert!(l.chevrons.is_some());
    let first = l
        .runs
        .iter()
        .find(|r| r.role == candidate::Role::Candidate)
        .unwrap();
    assert!(
        first.x > c.anchor.x - 30 && first.x < c.anchor.x + 30,
        "text under the caret"
    );
    // Near the bottom and right edges: above the caret, inside the area.
    let low = CandidateWindow {
        anchor: Rect {
            x: 1700,
            y: 980,
            width: 2,
            height: 18,
        },
        vertical: true,
        ..c.clone()
    };
    let l = low.layout(work, &measure);
    assert!(l.frame.y + l.frame.height as i32 <= 980 && l.frame.x + l.frame.width as i32 <= 1707);
    assert!(
        l.runs
            .iter()
            .any(|r| r.role == candidate::Role::Page && r.text == "1/3")
    );
    let mut pixels = vec![0u8; 400 * 300 * 4];
    let mut t = candidate::Target {
        pixels: &mut pixels,
        stride: 1600,
        area: Rect {
            x: l.frame.x,
            y: l.frame.y,
            width: 400,
            height: 300,
        },
    };
    let palette = candidate::Palette::from_shell(&crate::paint::Palette::themed(false));
    candidate::paint(&low, &l, &palette, &mut t);
    let at = |x: i32, y: i32| pixels[(y as usize * 400 + x as usize) * 4..][..4].to_vec();
    assert_eq!(at(0, 0)[3], 0, "rounded corner left untouched");
    assert_eq!(at(l.frame.width as i32 / 2, 3), palette.background.to_vec());
}
#[test]
fn state_reports_methods_registered_without_candidate_dictionaries() {
    let e = env();
    let mut b = Broker::new();
    assert!(!b.state().candidates_available(7), "no IME");
    let p = Preferences::new(&[1, 7, 8, 9], 1, false).unwrap();
    b.request(
        &e,
        Peer::Init,
        OTHER,
        ti::SET_PREFERENCES,
        &p.bits().to_le_bytes(),
    )
    .unwrap();
    // Pinyin and Japanese dictionaries missing; Korean needs none.
    let missing = ((1 << 7) | (1 << 8) | (1 << 1)) << ti::NO_CANDIDATES_SHIFT;
    ime(
        &mut b,
        &e,
        ti::IME_REGISTER,
        &words(&[ti::VERSION, kl::methods::IME_METHODS | missing]),
    )
    .unwrap();
    let s = ti::State::decode(&b.state().encode()).unwrap();
    assert_eq!(
        s.flags,
        ti::STATE_IME | ((1 << 7) | (1 << 8)) << ti::NO_CANDIDATES_SHIFT,
        "non-IME bit dropped"
    );
    assert!(!s.candidates_available(7) && !s.candidates_available(8) && s.candidates_available(9));
    b.revoke(IME);
    assert_eq!(b.state().flags, 0);
}
fn preedit_body(serial: u32, text: &str) -> Vec<u8> {
    let mut o = [0u8; BODY_BYTES];
    let n = ti::encode_ime_preedit(
        serial,
        ti::Preedit {
            text,
            cursor: text.len() as u16,
            attrs: &[],
        },
        &mut o,
    )
    .unwrap();
    o[..n].to_vec()
}
fn commit_body(serial: u32, text: &str) -> Vec<u8> {
    let mut o = [0u8; BODY_BYTES];
    let n = ti::encode_ime_commit(serial, text, &mut o).unwrap();
    o[..n].to_vec()
}
/// Pinyin broker with an active context on W showing preedit "ni".
fn composing(e: &Fake) -> (Broker, u32) {
    let mut b = pinyin(e);
    let serial = enable(&mut b, e, W, ti::NORMAL);
    b.key(e, &key(0x31, d::DOWN));
    ime(&mut b, e, ti::IME_PREEDIT, &preedit_body(serial, "ni")).unwrap();
    ime(&mut b, e, ti::IME_KEY_DONE, &words(&[1, ti::CONSUMED])).unwrap();
    assert_eq!(
        poll(&mut b, e, Peer::App, APP),
        [R::Preedit(W, "ni".into())]
    );
    let _ = poll(&mut b, e, Peer::Ime, IME);
    (b, serial)
}
#[test]
fn disable_retires_the_context_like_a_focus_change() {
    let e = env();
    let (mut b, serial) = composing(&e);
    b.request(&e, Peer::App, APP, ti::DISABLE, &W.to_le_bytes())
        .unwrap();
    assert_eq!(
        poll(&mut b, &e, Peer::Ime, IME),
        [R::Deactivate(serial)],
        "the IME commits, it does not cancel"
    );
    // The commit-on-blur and the IME's preedit clear reach the retired field.
    ime(&mut b, &e, ti::IME_COMMIT, &commit_body(serial, "你")).unwrap();
    ime(&mut b, &e, ti::IME_PREEDIT, &preedit_body(serial, "")).unwrap();
    assert_eq!(
        poll(&mut b, &e, Peer::App, APP),
        [R::Commit(W, "你".into()), R::Preedit(W, "".into())]
    );
    // Preedit text (not a clear) for a retired context is refused.
    assert_eq!(
        ime(&mut b, &e, ti::IME_PREEDIT, &preedit_body(serial, "x")).err(),
        Some(Code::Changed)
    );
}
#[test]
fn re_enable_and_window_close_cancel_and_clear_the_preedit() {
    let mut e = env();
    let (mut b, first) = composing(&e);
    let second = enable(&mut b, &e, W, ti::NORMAL);
    assert_eq!(
        poll(&mut b, &e, Peer::Ime, IME),
        [
            R::Reset(ti::RESET_CANCEL),
            R::Deactivate(first),
            R::Activate(second, ti::NORMAL)
        ]
    );
    assert_eq!(poll(&mut b, &e, Peer::App, APP), [R::Preedit(W, "".into())]);
    ime(&mut b, &e, ti::IME_PREEDIT, &preedit_body(second, "ni")).unwrap();
    let _ = poll(&mut b, &e, Peer::App, APP);
    // The window closes: the next request retires its context.
    e.windows.retain(|w| *w != (APP, W));
    b.request(&e, Peer::App, APP, ti::STATE, &[]).unwrap();
    assert_eq!(
        poll(&mut b, &e, Peer::Ime, IME),
        [R::Reset(ti::RESET_CANCEL), R::Deactivate(second)]
    );
    assert_eq!(poll(&mut b, &e, Peer::App, APP), [R::Preedit(W, "".into())]);
    assert!(b.candidates().is_none());
}
#[test]
fn late_ime_output_for_keys_forwarded_after_a_timeout_is_dropped() {
    let mut e = env();
    let mut b = pinyin(&e);
    let serial = enable(&mut b, &e, W, ti::NORMAL);
    b.key(&e, &key(0x31, d::DOWN));
    e.now += ti::IME_KEY_TIMEOUT_TICKS;
    b.key(&e, &key(0x17, d::DOWN));
    // The slow IME still answers "n": a preedit, a commit, then the verdict.
    ime(&mut b, &e, ti::IME_PREEDIT, &preedit_body(serial, "n")).unwrap();
    ime(&mut b, &e, ti::IME_COMMIT, &commit_body(serial, "你")).unwrap();
    assert_eq!(
        b.key(&e, &key(0x12, d::DOWN)),
        KeyRoute::Consumed,
        "still bypassed while it catches up"
    );
    ime(&mut b, &e, ti::IME_KEY_DONE, &words(&[1, ti::CONSUMED])).unwrap();
    // Caught up: its Reset(CANCEL) clear counts again, keys reach it again.
    ime(&mut b, &e, ti::IME_PREEDIT, &preedit_body(serial, "")).unwrap();
    assert_eq!(
        poll(&mut b, &e, Peer::App, APP),
        [
            R::Key(W, "n".into(), 0x31),
            R::Preedit(W, "".into()),
            R::Key(W, "i".into(), 0x17),
            R::Key(W, "e".into(), 0x12),
            R::Preedit(W, "".into())
        ],
        "each key once, no duplicate preedit or commit"
    );
    let r = poll(&mut b, &e, Peer::Ime, IME);
    assert_eq!(
        &r[r.len() - 2..],
        [R::ImeKey(1, "n".into()), R::Reset(ti::RESET_CANCEL)]
    );
    b.key(&e, &key(0x31, d::DOWN));
    assert_eq!(poll(&mut b, &e, Peer::Ime, IME), [R::ImeKey(2, "n".into())]);
}
#[test]
fn win_space_with_keys_at_the_ime_drops_their_late_commit() {
    let e = env();
    let mut b = pinyin(&e);
    let serial = enable(&mut b, &e, W, ti::NORMAL);
    b.key(&e, &key(0x31, d::DOWN));
    b.key(&e, &key(0x17, d::DOWN));
    assert!(b.hotkey(&e, &key(0x39, d::DOWN | d::LOGO | d::LEFT_LOGO)));
    assert_eq!(b.take_method_change().map(|m| m.id), Some(1));
    assert_eq!(b.take_method_change(), None, "reported once");
    // The IME commits "ni" for both keys (the duplicate scenario): it
    // predates the keys' epoch and is dropped. The IME then cancels before
    // it deactivates, so its commit-on-blur is empty.
    ime(&mut b, &e, ti::IME_COMMIT, &commit_body(serial, "你")).unwrap();
    ime(&mut b, &e, ti::IME_KEY_DONE, &words(&[2, ti::CONSUMED])).unwrap();
    let r = poll(&mut b, &e, Peer::Ime, IME);
    assert_eq!(
        &r[r.len() - 5..],
        [
            R::ImeKey(1, "n".into()),
            R::ImeKey(2, "i".into()),
            R::Reset(ti::RESET_CANCEL),
            R::Deactivate(serial),
            R::Method(1)
        ]
    );
    assert_eq!(
        poll(&mut b, &e, Peer::App, APP),
        [
            R::Key(W, "n".into(), 0x31),
            R::Key(W, "i".into(), 0x17),
            R::Preedit(W, "".into())
        ]
    );
}
#[test]
fn contexts_are_capped_per_application() {
    let mut e = env();
    for w in 100..100 + ti::MAX_CONTEXTS as u64 + 1 {
        e.windows.push((APP, w));
    }
    let mut b = Broker::new();
    for w in 100..100 + ti::MAX_CONTEXTS as u64 {
        enable(&mut b, &e, w, ti::NORMAL);
    }
    let body = ti::Enable {
        window: 100 + ti::MAX_CONTEXTS as u64,
        content: ti::NORMAL,
        hints: 0,
        caret: caret(),
    };
    assert_eq!(
        b.request(&e, Peer::App, APP, ti::ENABLE, &body.encode())
            .err(),
        Some(Code::Resources)
    );
    let other = ti::Enable { window: 7, ..body }.encode();
    assert!(
        b.request(&e, Peer::App, OTHER, ti::ENABLE, &other).is_ok(),
        "another application still gets a context"
    );
}
#[test]
fn queue_overflow_keeps_control_records_and_drops_only_data() {
    let e = env();
    let mut b = pinyin(&e);
    let serial = enable(&mut b, &e, W, ti::NORMAL);
    ime(&mut b, &e, ti::IME_COMMIT, &commit_body(serial, "你")).unwrap();
    // US keyboard: keys go to the application queue directly.
    assert!(b.hotkey(&e, &key(0x39, d::DOWN | d::LOGO | d::LEFT_LOGO)));
    let mut accepted = 0;
    while b.key(&e, &key(0x1e, d::DOWN)) == KeyRoute::Consumed {
        accepted += 1;
    }
    assert_eq!(accepted, ti::MAX_QUEUED_RECORDS - 1);
    // An overflowed queue admits no new context until it is polled.
    let body = ti::Enable {
        window: W2,
        content: ti::NORMAL,
        hints: 0,
        caret: caret(),
    }
    .encode();
    assert_eq!(
        b.request(&e, Peer::App, APP, ti::ENABLE, &body).err(),
        Some(Code::Resources)
    );
    assert_eq!(
        poll(&mut b, &e, Peer::App, APP),
        [
            R::Commit(W, "你".into()),
            R::Disabled(ti::DISABLED_OVERFLOW)
        ]
    );
    assert!(b.request(&e, Peer::App, APP, ti::ENABLE, &body).is_ok());
}
#[test]
fn an_ime_crash_clears_the_preedit_and_resets_its_state() {
    let e = env();
    let (mut b, serial) = composing(&e);
    let mut o = [0u8; BODY_BYTES];
    let n = ti::encode_candidates(
        ti::Candidates {
            serial,
            selected: 0,
            page: 0,
            pages: 1,
            flags: 0,
            items: &["你"],
        },
        &mut o,
    )
    .unwrap();
    ime(&mut b, &e, ti::IME_CANDIDATES, &o[..n]).unwrap();
    assert!(b.candidates().is_some() && b.announce_candidates());
    assert!(!b.announce_candidates(), "once per window");
    b.key(&e, &key(0x17, d::DOWN));
    b.revoke(IME);
    assert_eq!(
        poll(&mut b, &e, Peer::App, APP),
        [R::Key(W, "i".into(), 0x17), R::Preedit(W, "".into())]
    );
    assert!(b.candidates().is_none());
    assert_eq!((b.state().flags, b.state().mode), (0, ti::MODE_DIRECT));
    // A restarted IME starts clean on the focused field.
    ime(
        &mut b,
        &e,
        ti::IME_REGISTER,
        &words(&[ti::VERSION, kl::methods::IME_METHODS]),
    )
    .unwrap();
    let r = poll(&mut b, &e, Peer::Ime, IME);
    assert!(matches!(r.as_slice(), [R::Activate(s, _)] if *s == serial));
    b.key(&e, &key(0x31, d::DOWN));
    assert!(poll(&mut b, &e, Peer::Ime, IME).contains(&R::ImeKey(3, "n".into())));
}
#[test]
fn the_ime_mode_glyph_needs_a_registered_ime() {
    let e = env();
    let mut b = Broker::new();
    let p = Preferences::new(&[1, 7], 1, false).unwrap();
    b.request(
        &e,
        Peer::Init,
        OTHER,
        ti::SET_PREFERENCES,
        &p.bits().to_le_bytes(),
    )
    .unwrap();
    assert_eq!(
        b.indicator(),
        Some(kl::Indicator::Ime {
            glyph: "英",
            language: "zh-CN"
        }),
        "no IME: direct input, never 中"
    );
    assert_eq!(b.state().mode, ti::MODE_DIRECT);
    ime(
        &mut b,
        &e,
        ti::IME_REGISTER,
        &words(&[ti::VERSION, kl::methods::IME_METHODS]),
    )
    .unwrap();
    assert_eq!(
        b.indicator(),
        Some(kl::Indicator::Ime {
            glyph: "中",
            language: "zh-CN"
        })
    );
    b.revoke(IME);
    assert!(matches!(
        b.indicator(),
        Some(kl::Indicator::Ime { glyph: "英", .. })
    ));
}
#[test]
fn rounded_fills_never_panic_for_narrow_or_empty_rectangles() {
    let mut pixels = vec![0u8; 32 * 32 * 4];
    let area = Rect {
        x: 0,
        y: 0,
        width: 32,
        height: 32,
    };
    for width in 0..14 {
        for height in 0..14 {
            for radius in 0..12 {
                pixels.fill(0);
                let mut t = candidate::Target {
                    pixels: &mut pixels,
                    stride: 128,
                    area,
                };
                let r = Rect {
                    x: 2,
                    y: 3,
                    width,
                    height,
                };
                t.rounded(r, radius, [9, 9, 9, 255]);
                for (i, p) in pixels.chunks_exact(4).enumerate() {
                    let (x, y) = ((i % 32) as i32, (i / 32) as i32);
                    let inside = x >= 2 && x < 2 + width as i32 && y >= 3 && y < 3 + height as i32;
                    assert!(inside || p[3] == 0, "{width}x{height} r{radius} at {x},{y}");
                }
            }
        }
    }
}
/// The real renderer (shaped CJK text from the owner-local fonts) at every
/// Windows scale, for the Pinyin bar and the Japanese list, in light, dark
/// and a contrast theme: no panic (C2: the accent pill is 4/6/8 px wide at
/// 125/200/250%), the frame stays in the work area, and chrome, selection,
/// accent pill and text are all painted.
#[test]
fn candidate_window_renders_at_every_scale_layout_and_theme() {
    use door_graphics_text::{FontRole, FontSet, TextSystem};
    use door_service_protocol::desktop::ContrastTheme;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let files: Vec<(&'static [u8], FontRole)> = [
        ("assets/fonts/segoe-ui/segoeui.ttf", FontRole::Ui),
        (
            "assets/fonts/windows-unicode/msyh.ttc",
            FontRole::ChineseSimplified,
        ),
        (
            "assets/fonts/windows-unicode/YuGothM.ttc",
            FontRole::Japanese,
        ),
    ]
    .iter()
    .map(|(path, role)| {
        let data = std::fs::read(root.join(path))
            .unwrap_or_else(|e| panic!("owner-local font {path} missing: {e}"));
        (&*std::boxed::Box::leak(data.into_boxed_slice()), *role)
    })
    .collect();
    let text = TextSystem::new(FontSet::from_files(&files).unwrap());
    let palettes = [
        candidate::Palette::from_shell(&crate::paint::Palette::themed(false)),
        candidate::Palette::from_shell(&crate::paint::Palette::themed(true)),
        candidate::Palette::from_shell(&crate::paint::Palette::contrast_theme(
            ContrastTheme::NightSky,
        )),
    ];
    let pinyin = ["你好", "拟好", "你", "尼", "泥", "腻", "逆", "妮", "倪"];
    let japanese = ["日本語", "日本後", "二本", "2本", "にほんご"];
    for scale in [100u32, 125, 150, 175, 200, 250, 300] {
        // A 1280x800 logical work area at this scale.
        let (ww, wh) = (1280 * scale / 100, 800 * scale / 100);
        let work = Rect {
            x: 0,
            y: 0,
            width: ww,
            height: wh,
        };
        let mut pixels = vec![0u8; (ww * wh * 4) as usize];
        for (vertical, items, locale) in [
            (false, &pinyin[..], "zh-CN"),
            (true, &japanese[..], "ja-JP"),
        ] {
            let window = CandidateWindow {
                serial: 1,
                items: items.iter().map(|s| String::from(*s)).collect(),
                selected: 1,
                page: 0,
                pages: 3,
                vertical,
                anchor: Rect {
                    x: 300,
                    y: 200,
                    width: 2,
                    height: 20 * scale / 100,
                },
                scale,
            };
            for palette in &palettes {
                pixels.fill(0);
                let mut t = candidate::Target {
                    pixels: &mut pixels,
                    stride: ww as usize * 4,
                    area: work,
                };
                let l = candidate::render(&window, work, palette, &text, locale, &mut t);
                let f = l.frame;
                let case = std::format!("{scale}% vertical={vertical} {palette:?}");
                assert!(
                    f.x >= 0
                        && f.y >= 0
                        && f.x + f.width as i32 <= ww as i32
                        && f.y + f.height as i32 <= wh as i32,
                    "{case}: frame {f:?}"
                );
                let at = |x: i32, y: i32| -> [u8; 4] {
                    pixels[(y as usize * ww as usize + x as usize) * 4..][..4]
                        .try_into()
                        .unwrap()
                };
                assert_eq!(at(f.x - 40, f.y - 40)[3], 0, "{case}: outside untouched");
                let cell = l.cells[1];
                assert_ne!(
                    at(cell.x + cell.width as i32 / 2, cell.y + 1),
                    palette.background,
                    "{case}: selection"
                );
                if vertical {
                    let pill = (cell.x + 1, cell.y + cell.height as i32 / 2);
                    assert_eq!(at(pill.0, pill.1), palette.accent, "{case}: accent pill");
                }
                let mut colors = std::collections::BTreeSet::new();
                for y in f.y..f.y + f.height as i32 {
                    for x in f.x..f.x + f.width as i32 {
                        colors.insert(at(x, y));
                    }
                }
                assert!(
                    colors.len() > 8,
                    "{case}: text drawn ({} colours)",
                    colors.len()
                );
                assert_eq!(
                    l.runs
                        .iter()
                        .filter(|r| r.role == candidate::Role::Candidate)
                        .count(),
                    items.len()
                );
            }
        }
    }
}

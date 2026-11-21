extern crate std;
use crate::input::{self, Decoded, KeyEvent, control, vk};
use crate::*;

#[cfg(feature = "alloc")]
mod screen {
    use super::*;
    use std::string::{String, ToString};

    fn run(columns: usize, rows: usize, input: &str) -> Screen {
        let mut s = Screen::new(columns, rows);
        Parser::new().feed(&mut s, input.as_bytes());
        s
    }

    #[test]
    fn text_wraps_and_scrolls_into_scrollback() {
        let s = run(5, 2, "hello world\r\nend");
        assert_eq!(s.scrolled.len(), 2);
        assert_eq!(
            s.scrolled[0].iter().map(|c| c.ch).collect::<String>(),
            "hello"
        );
        assert_eq!(
            s.scrolled[1].iter().map(|c| c.ch).collect::<String>(),
            " worl"
        );
        assert_eq!(s.row_text(0), "d");
        assert_eq!(s.row_text(1), "end");
        assert_eq!(s.cursor(), (3, 1));
    }

    #[test]
    fn a_line_feed_returns_unless_newline_mode_is_off() {
        let s = run(10, 3, "ab\ncd");
        assert_eq!((s.row_text(1), s.cursor()), ("cd".to_string(), (2, 1)));
        let s = run(10, 3, "\x1b[20lab\ncd");
        assert_eq!((s.row_text(1), s.cursor()), ("  cd".to_string(), (4, 1)));
    }

    #[test]
    fn cursor_movement_and_erase() {
        let s = run(
            10,
            3,
            "abcdef\x1b[1;3HX\x1b[2;1Hline\x1b[1K\x1b[3;5H*\x1b[H\x1b[0K",
        );
        assert_eq!(s.row_text(0), "");
        assert_eq!(s.row_text(1), "");
        assert_eq!(s.row_text(2), "    *");
        let s = run(10, 2, "0123456789\x1b[1;4H\x1b[2P\x1b[1@");
        assert_eq!(s.row_text(0), "012 56789");
    }

    #[test]
    fn colours_and_attributes() {
        let s = run(10, 1, "\x1b[1;31mR\x1b[38;5;200mI\x1b[48;2;1;2;3mT\x1b[0mn");
        let row = s.row(0);
        assert_eq!(row[0].fg, Color::Indexed(1));
        assert_eq!(row[0].attrs, attr::BOLD);
        assert_eq!(row[1].fg, Color::Indexed(200));
        assert_eq!(row[2].bg, Color::Rgb(1, 2, 3));
        assert_eq!(row[3].fg, Color::Default);
        assert_eq!(row[3].attrs, 0);
    }

    #[test]
    fn alternate_screen_keeps_the_main_one() {
        let mut s = Screen::new(10, 2);
        let mut p = Parser::new();
        p.feed(&mut s, b"main\x1b[?1049h\x1b[2Jalt");
        assert!(s.alternate());
        // 1049 keeps the cursor where it was.
        assert_eq!(s.row_text(0), "    alt");
        p.feed(&mut s, b"\x1b[?1049l");
        assert_eq!(s.row_text(0), "main");
        assert_eq!(s.cursor(), (4, 0));
    }

    #[test]
    fn titles_reports_and_split_utf8() {
        let mut s = Screen::new(20, 2);
        let mut p = Parser::new();
        p.feed(&mut s, b"\x1b]0;PowerShell 7\x07\x1b[2;5H\x1b[6n\x1b[c");
        assert_eq!(s.title.as_deref(), Some("PowerShell 7"));
        assert_eq!(
            core::str::from_utf8(&s.replies).unwrap(),
            "\x1b[2;5R\x1b[?1;0c"
        );
        let bytes = "\u{2713}ok".as_bytes();
        p.feed(&mut s, &bytes[..2]);
        p.feed(&mut s, &bytes[2..]);
        assert_eq!(s.row(1)[4].ch, '\u{2713}');
        assert_eq!(s.row_text(1), "    \u{2713}ok");
    }

    #[test]
    fn scroll_regions_and_line_insertion() {
        let s = run(4, 4, "a\r\nb\r\nc\r\nd\x1b[2;3r\x1b[2;1H\x1b[L");
        assert_eq!(
            (s.row_text(0), s.row_text(1), s.row_text(2), s.row_text(3)),
            (
                "a".to_string(),
                "".to_string(),
                "b".to_string(),
                "d".to_string()
            )
        );
        assert!(s.scrolled.is_empty());
        // Deleting lines at the top keeps nothing as scrollback.
        let s = run(4, 2, "a\r\nb\x1b[H\x1b[M");
        assert!(s.scrolled.is_empty());
        assert_eq!(s.row_text(0), "b");
    }

    #[test]
    fn resize_keeps_the_cursor_row() {
        let mut s = run(10, 4, "1\r\n2\r\n3\r\n4");
        s.resize(6, 2);
        assert_eq!(s.row_text(0), "3");
        assert_eq!(s.row_text(1), "4");
        assert_eq!(s.scrolled.len(), 2);
        assert_eq!(s.cursor(), (1, 1));
    }

    /// The console service's tracker and the Terminal's screen see the
    /// same output: their cursors never disagree.
    #[test]
    fn the_tracker_follows_the_screen() {
        let output = [
            "PS C:\\> ",
            "\x1b[?25l\x1b[93mGet-Process\x1b[0m\x1b[?25h",
            "\x1b[3D\x1b[K\x1b[2A\x1b[10;3H\x1b[s",
            "a very long line that wraps around the narrow screen twice over\r\n",
            "\x1b[2;4r\x1b[4;1H\n\n\n\x1b[r\x1b[u\x1b7\x1b[1;1H\x1b8",
            "\x1b[?1049h\x1b[H\x1b[2Jfull screen\x1b[?1049l\x1bM\x1bM\x1bD\x1bE",
            "\t\tx\x08\x08\x1b[5G\x1b[2d\x1b[?6h\x1b[2;2H\x1b[?6l\x1b[20l\n\x1b[20h",
            "\x1b[3S\x1b[2T\x1b[2L\x1b[2M\x1b[4@\x1b[4P\x1b[3X\x1bc\u{2713}\u{1f600}",
        ];
        let mut screen = Screen::new(20, 8);
        let mut tracker = Tracker::new(20, 8);
        let mut p = Parser::new();
        for (i, chunk) in output.iter().enumerate() {
            p.feed(&mut screen, chunk.as_bytes());
            tracker.feed(chunk.as_bytes());
            assert_eq!(tracker.cursor(), screen.state(), "after chunk {i}");
            if i == 3 {
                screen.resize(14, 6);
                tracker.resize(14, 6);
                assert_eq!(tracker.cursor(), screen.state(), "after resize");
            }
        }
    }
}

#[test]
fn the_tracker_answers_cursor_reports() {
    let mut t = Tracker::new(80, 25);
    t.feed(b"PS C:\\> \x1b[6n\x1b[5n");
    let mut out = [0u8; 32];
    let n = t.take_replies(&mut out);
    assert_eq!(&out[..n], b"\x1b[1;9R\x1b[0n");
    assert_eq!(t.take_replies(&mut out), 0);
    assert_eq!(t.cursor().position(), (8, 0));
}

#[test]
fn keys_travel_as_win32_input_sequences() {
    let a = input::from_scan(30, 'a', false, false, false).unwrap();
    assert_eq!((a.vk, a.ch, a.control, a.scan), (0x41, b'a' as u16, 0, 30));
    let mut out = [0u8; 48];
    let n = input::encode_win32(&a, &mut out);
    assert_eq!(&out[..n], b"\x1b[65;30;97;1;0;1_");
    assert_eq!(input::decode(&out[..n], false), Decoded::Keys(keys(a), n));
    let up = input::from_scan(0x148, '\0', true, false, false).unwrap();
    assert_eq!(up.control, control::LEFT_CTRL | control::ENHANCED);
    let ctrl_c = input::from_scan(46, 'c', true, false, false).unwrap();
    assert_eq!((ctrl_c.vk, ctrl_c.ch), (b'C' as u16, 3));
    let shift_1 = input::from_scan(2, '!', false, true, false).unwrap();
    assert_eq!(
        (shift_1.vk, shift_1.ch, shift_1.control),
        (0x31, b'!' as u16, control::SHIFT)
    );
    let f5 = input::from_scan(63, '\0', false, false, false).unwrap();
    assert_eq!(f5.vk, vk::F1 + 4);
    assert!(input::from_scan(0x1ff, '\0', false, false, false).is_none());
}

fn keys(e: KeyEvent) -> input::Keys {
    input::Keys {
        events: [e, KeyEvent::default()],
        count: 1,
    }
}
fn first(bytes: &[u8]) -> (KeyEvent, usize) {
    match input::decode(bytes, false) {
        Decoded::Keys(k, n) => (k.events[0], n),
        other => panic!("{other:?}"),
    }
}

#[test]
fn virtual_terminal_input_decodes_to_keys() {
    assert_eq!(first(b"\x1b[A").0.vk, vk::UP);
    let (e, n) = first(b"\x1b[1;5Cx");
    assert_eq!(
        (e.vk, e.control & control::CTRL != 0, n),
        (vk::RIGHT, true, 6)
    );
    assert_eq!(first(b"\x1bOP").0.vk, vk::F1);
    assert_eq!(first(b"\x1b[3~").0.vk, vk::DELETE);
    assert_eq!(
        first(b"\x1b[24;2~").0,
        KeyEvent {
            down: true,
            repeat: 1,
            vk: vk::F12,
            scan: 88,
            ch: 0,
            control: control::SHIFT,
        }
    );
    assert_eq!(first(b"\x1b[Z").0.control, control::SHIFT);
    assert_eq!(first(b"\x7f").0.vk, vk::BACK);
    let (e, n) = first(b"\x1bx");
    assert_eq!(
        (e.vk, e.ch, e.control, n),
        (b'X' as u16, b'x' as u16, control::LEFT_ALT, 2)
    );
    let (e, _) = first(b"A");
    assert_eq!((e.vk, e.control), (b'A' as u16, control::SHIFT));
    assert_eq!(first(b"\x03").0.vk, b'C' as u16);
    // A lone ESC waits while more may come, else is the Escape key.
    assert_eq!(input::decode(b"\x1b", true), Decoded::Need);
    assert_eq!(first(b"\x1b").0.vk, vk::ESCAPE);
    assert_eq!(input::decode(b"\x1b[200~", false), Decoded::Skip(6));
    assert_eq!(input::decode(b"\x1b[65;30", true), Decoded::Need);
    // Outside the BMP: a surrogate pair.
    match input::decode("\u{1f600}".as_bytes(), false) {
        Decoded::Keys(k, 4) => {
            assert_eq!(k.count, 2);
            assert_eq!((k.events[0].ch, k.events[1].ch), (0xd83d, 0xde00));
        }
        other => panic!("{other:?}"),
    }
    let (e, _) = first("é".as_bytes());
    assert_eq!((e.ch, e.vk), (0xe9, 0));
}

#[test]
fn keys_become_virtual_terminal_input() {
    let vt = |e: KeyEvent, app: bool| {
        let mut out = [0u8; 16];
        let n = input::to_vt(&e, app, &mut out);
        std::vec::Vec::from(&out[..n])
    };
    let key =
        |scan, text, ctrl, shift, alt| input::from_scan(scan, text, ctrl, shift, alt).unwrap();
    assert_eq!(vt(key(0x148, '\0', false, false, false), false), b"\x1b[A");
    assert_eq!(vt(key(0x148, '\0', false, false, false), true), b"\x1bOA");
    assert_eq!(vt(key(0x14d, '\0', true, false, false), true), b"\x1b[1;5C");
    assert_eq!(vt(key(0x153, '\0', false, false, false), false), b"\x1b[3~");
    assert_eq!(
        vt(key(0x151, '\0', false, true, false), false),
        b"\x1b[6;2~"
    );
    assert_eq!(vt(key(59, '\0', false, false, false), false), b"\x1bOP");
    assert_eq!(vt(key(88, '\0', false, false, false), false), b"\x1b[24~");
    assert_eq!(vt(key(15, '\t', false, true, false), false), b"\x1b[Z");
    assert_eq!(vt(key(14, '\0', false, false, false), false), b"\x7f");
    assert_eq!(vt(key(30, 'a', false, false, true), false), b"\x1ba");
    assert_eq!(vt(key(46, 'c', true, false, false), false), b"\x03");
    assert_eq!(vt(key(28, '\r', false, false, false), false), b"\r");
    assert_eq!(vt(key(42, '\0', false, true, false), false), b"");
    // Every key the Terminal sends decodes back to the same key.
    for scan in 1..0x160u16 {
        for (ctrl, shift, alt) in [
            (false, false, false),
            (true, false, false),
            (false, true, true),
        ] {
            let Some(e) = input::from_scan(scan, '\0', ctrl, shift, alt) else {
                continue;
            };
            let mut out = [0u8; 48];
            let n = input::encode_win32(&e, &mut out);
            assert_eq!(
                input::decode(&out[..n], false),
                Decoded::Keys(keys(e), n),
                "{scan}"
            );
        }
    }
}

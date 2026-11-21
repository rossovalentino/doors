//! door.text-input records (`specs/contracts/text-input-v1.md`) routed to a
//! workspace window in their original order. Keys keep the legacy key path
//! (same shortcuts and handling as window INPUT events); Unicode a key
//! produced that the legacy code cannot carry, and IME preedit, commit,
//! deletion and disable records reach the application as owned events.
extern crate alloc;
use alloc::string::String;
use door_abi::graphics as g;
use door_service_protocol::text_input::{self as ti, Record};
use door_service_protocol::window as w;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextEvent {
    /// Layout text of one key (non-ASCII, AltGr or dead-key composed).
    Typed(String),
    /// IME composition; empty text clears it. `cursor` is a byte offset.
    Preedit {
        serial: u32,
        text: String,
        cursor: Option<usize>,
    },
    Commit {
        serial: u32,
        text: String,
    },
    /// Delete bytes before/after the caret (IME surrounding-text edits).
    Delete {
        serial: u32,
        before: u32,
        after: u32,
    },
    /// The server disabled the context (overflow, session lock).
    Disabled {
        serial: u32,
    },
}
/// A routed record: the window event to dispatch, with the text payload of
/// a `ti::PENDING` event (None for a legacy key event).
pub type Routed = (w::Event, Option<TextEvent>);
fn text_event(window: u64, event: TextEvent) -> Routed {
    (
        w::Event {
            kind: ti::PENDING,
            window,
            ..w::Event::default()
        },
        Some(event),
    )
}
/// Route one record for an application window; IME-side records and key
/// releases yield nothing.
pub fn route(record: &Record<'_>) -> Option<Routed> {
    Some(match *record {
        Record::Key { window, key } => {
            if !key.down() {
                return None;
            }
            let text = key.text();
            let altgr = key.status & ti::KEY_ALTGR != 0;
            let chord = !altgr && key.flags & (g::CONTROL | g::ALT) != 0;
            let legacy_text = ((key.legacy >> 16) & 0xff) as u8;
            let plain = text.len() == 1 && text.as_bytes()[0] == legacy_text && !altgr;
            if !chord && !text.is_empty() && !plain && !text.chars().any(char::is_control) {
                text_event(window, TextEvent::Typed(text.into()))
            } else {
                (
                    w::Event {
                        kind: w::INPUT,
                        detail: 1,
                        a: key.legacy,
                        b: 1,
                        window,
                        ..w::Event::default()
                    },
                    None,
                )
            }
        }
        Record::Preedit {
            window,
            serial,
            preedit,
        } => text_event(
            window,
            TextEvent::Preedit {
                serial,
                text: preedit.text.into(),
                cursor: (preedit.cursor != ti::HIDDEN_CURSOR)
                    .then_some(usize::from(preedit.cursor)),
            },
        ),
        Record::Commit {
            window,
            serial,
            text,
        } => text_event(
            window,
            TextEvent::Commit {
                serial,
                text: text.into(),
            },
        ),
        Record::Delete {
            window,
            serial,
            before,
            after,
        } => text_event(
            window,
            TextEvent::Delete {
                serial,
                before,
                after,
            },
        ),
        Record::Disabled { window, serial, .. } => {
            text_event(window, TextEvent::Disabled { serial })
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(text: &str, legacy: u32, flags: u32, status: u8) -> Record<'static> {
        Record::Key {
            window: 7,
            key: ti::Key {
                code: 0x1e,
                flags: flags | g::DOWN,
                legacy,
                status,
                ..ti::Key::default()
            }
            .with_text(text),
        }
    }
    #[test]
    fn keys_keep_the_legacy_path_and_unicode_text_becomes_typed() {
        // ASCII the legacy code carries: the legacy key event, unchanged.
        let (event, text) = route(&key("a", 0x1e | (u32::from(b'a') << 16), 0, 0)).unwrap();
        assert_eq!(
            (event.kind, event.detail, event.a, event.b, event.window),
            (w::INPUT, 1, 0x1e | (97 << 16), 1, 7)
        );
        assert_eq!(text, None);
        // Italian layout: à, and AltGr+ò = @.
        let (event, text) = route(&key("à", 0x28, 0, 0)).unwrap();
        assert_eq!(
            (event.kind, text),
            (ti::PENDING, Some(TextEvent::Typed("à".into())))
        );
        let (_, text) = route(&key(
            "@",
            0x28 | (u32::from(b'@') << 16),
            g::CONTROL | g::ALT,
            ti::KEY_ALTGR,
        ))
        .unwrap();
        assert_eq!(text, Some(TextEvent::Typed("@".into())));
        // Ctrl+C stays a shortcut; Enter and Backspace stay keys; releases vanish.
        assert_eq!(
            route(&key("c", 0x2e | (99 << 16), g::CONTROL, 0))
                .unwrap()
                .1,
            None
        );
        assert_eq!(route(&key("\r", 28 | (10 << 16), 0, 0)).unwrap().1, None);
        let mut up = key("a", 0x1e, 0, 0);
        if let Record::Key { key, .. } = &mut up {
            key.flags &= !g::DOWN;
        }
        assert_eq!(route(&up), None);
    }
    #[test]
    fn ime_records_become_ordered_text_events() {
        let preedit = ti::PreeditBuf {
            text: "にほ",
            cursor: ti::HIDDEN_CURSOR,
            count: 0,
            attrs: [ti::Attr::default(); ti::MAX_ATTRS],
        };
        let (event, text) = route(&Record::Preedit {
            window: 3,
            serial: 9,
            preedit,
        })
        .unwrap();
        assert_eq!((event.kind, event.window), (ti::PENDING, 3));
        assert_eq!(
            text,
            Some(TextEvent::Preedit {
                serial: 9,
                text: "にほ".into(),
                cursor: None
            })
        );
        assert_eq!(
            route(&Record::Commit {
                window: 3,
                serial: 9,
                text: "日本"
            })
            .unwrap()
            .1,
            Some(TextEvent::Commit {
                serial: 9,
                text: "日本".into()
            })
        );
        assert_eq!(
            route(&Record::Disabled {
                window: 3,
                serial: 9,
                reason: ti::DISABLED_LOCKED
            })
            .unwrap()
            .1,
            Some(TextEvent::Disabled { serial: 9 })
        );
        assert_eq!(route(&Record::Method { method: 1 }), None);
    }
}

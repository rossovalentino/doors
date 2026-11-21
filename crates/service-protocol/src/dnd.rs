//! door.dnd v1 (`specs/contracts/native-ui-v1.md`): drag and drop brokered
//! by the window server with scoped offers, as OLE DoDragDrop/IDropTarget.
//!
//! A source stages its data like the clipboard (BEGIN here, then
//! `clipboard::FORMAT`/`WRITE`/`WRITE_SECTION`) and calls START while the
//! user holds the primary button over the source window. The server then
//! owns the pointer: it hit-tests registered drop targets, tells them only
//! which offered formats they accept (ENTER/OVER/LEAVE, never content),
//! resolves the effect (source's allowed effects, the target's STATUS, and
//! Ctrl/Shift/Alt) and shows the feedback. Esc or a release outside a
//! willing target cancels. On a drop only that target may READ, and only
//! the formats it registered for; FINISH reports the performed effect to the
//! source (DRAG_FINISHED). Files travel as paths, opened with the target's
//! own identity, so a drop never bypasses file permissions.
use crate::Code;
use crate::window::Event;
/// HELLO capability (bit 21): drag/drop requests and events.
pub const CAP_DRAG_DROP: u32 = 1 << 21;
/// Opcodes 0x5180..=0x51FF are reserved for drag and drop.
pub const FIRST: u32 = 0x5180;
pub const LAST: u32 = 0x51FF;
/// `[window u64][formats u32][effects u32][default u32]`: RegisterDragDrop
/// for an owned window: accepted `clipboard::mask` bits, effects it can
/// perform, and its usual effect (before any STATUS).
pub const REGISTER: u32 = 0x5180;
/// `[window u64]`: RevokeDragDrop.
pub const REVOKE: u32 = 0x5181;
/// `[window u64][effects u32]`: open a drag staging for an owned window
/// with the effects the source allows.
pub const BEGIN: u32 = 0x5182;
/// Empty: start the staged drag (the primary button must be held over the
/// source window). Reply `[serial u32]`. The source loses pointer capture.
pub const START: u32 = 0x5183;
/// `[serial u32][window u64][effects u32][default u32][insert utf8]`: the
/// target's answer to ENTER/OVER at the current position (NONE: refuse
/// here). The optional insert (up to `MAX_DESCRIPTION` bytes, no control
/// characters) names what is under the pointer, as DROPDESCRIPTION's
/// szInsert: the server labels the drag "Copy to %1", "Move to %1" or
/// "Create link in %1". Without it the label is the bare effect.
pub const STATUS: u32 = 0x5184;
/// Drop description insert bound in UTF-8 bytes.
pub const MAX_DESCRIPTION: usize = 64;
/// A STATUS insert: bounded, no control characters (empty: none).
pub fn valid_description(s: &str) -> bool {
    s.len() <= MAX_DESCRIPTION && !s.chars().any(char::is_control)
}
/// `[serial u32][format u32][offset u32]`: dropped data, drop target only,
/// accepted formats only, until FINISH. Up to `clipboard::READ_BYTES`.
pub const READ: u32 = 0x5185;
/// `[serial u32][effect u32]`: the target is done (performed effect; NONE
/// when it failed). The source receives DRAG_FINISHED.
pub const FINISH: u32 = 0x5186;
/// `[serial u32]`: the source cancels its own drag.
pub const CANCEL: u32 = 0x5187;
/// Effects (DROPEFFECT values).
pub const NONE: u32 = 0;
pub const COPY: u32 = 1;
pub const MOVE: u32 = 2;
pub const LINK: u32 = 4;
pub const EFFECTS: u32 = COPY | MOVE | LINK;
/// Key state in `Event.detail` (door_abi::graphics modifier bits).
pub const KEY_CONTROL: u32 = door_abi::graphics::CONTROL;
pub const KEY_SHIFT: u32 = door_abi::graphics::SHIFT;
pub const KEY_ALT: u32 = door_abi::graphics::ALT;
/// Event kinds (window Event layout, CAP_DRAG_DROP clients only). Target
/// events: window = target, value = serial, rect.x/y = content-local
/// physical pointer, a = offered formats the target accepts, b = allowed
/// effects (DROP: the resolved effect), detail = key state.
pub const ENTER: u32 = 0x110;
pub const OVER: u32 = 0x111;
pub const LEAVE: u32 = 0x112;
pub const DROP: u32 = 0x113;
/// Source event: window = source, value = serial, a = performed effect,
/// b = outcome.
pub const FINISHED: u32 = 0x114;
pub const OUTCOME_DROPPED: u32 = 0;
pub const OUTCOME_CANCELLED: u32 = 1;
/// Released where nothing accepted the drop.
pub const OUTCOME_REFUSED: u32 = 2;
/// The target exited, or never finished in time.
pub const OUTCOME_LOST: u32 = 3;
/// Effect for a drop given the source's allowed effects, the target's
/// effects and default at this point, and the keys (Windows: Ctrl copies,
/// Shift moves, Ctrl+Shift or Alt links; otherwise the target's default).
pub const fn resolve(allowed: u32, target: u32, default: u32, keys: u32) -> u32 {
    let possible = allowed & target & EFFECTS;
    let control = keys & KEY_CONTROL != 0;
    let shift = keys & KEY_SHIFT != 0;
    let wanted = if control && shift || keys & KEY_ALT != 0 {
        LINK
    } else if control {
        COPY
    } else if shift {
        MOVE
    } else if default & possible != 0 && default.is_power_of_two() {
        default
    } else if possible & MOVE != 0 {
        MOVE
    } else if possible & COPY != 0 {
        COPY
    } else {
        LINK
    };
    wanted & possible
}
pub fn decode(b: &[u8]) -> Result<Event, Code> {
    crate::clipboard::decode_event(b, ENTER..=FINISHED)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn effects_follow_windows_modifiers_and_never_exceed_both_sides() {
        let all = EFFECTS;
        assert_eq!(resolve(all, all, MOVE, 0), MOVE);
        assert_eq!(resolve(all, all, MOVE, KEY_CONTROL), COPY);
        assert_eq!(resolve(all, all, COPY, KEY_SHIFT), MOVE);
        assert_eq!(resolve(all, all, COPY, KEY_CONTROL | KEY_SHIFT), LINK);
        assert_eq!(resolve(all, all, COPY, KEY_ALT), LINK);
        // The source only allows copy: Shift cannot turn it into a move.
        assert_eq!(resolve(COPY, all, MOVE, KEY_SHIFT), NONE);
        assert_eq!(resolve(COPY, all, MOVE, 0), COPY);
        // A target that refuses here gets nothing whatever the keys.
        assert_eq!(resolve(all, NONE, NONE, KEY_CONTROL), NONE);
        // A bogus default falls back to the strongest common effect.
        assert_eq!(resolve(COPY | LINK, all, MOVE | COPY, 0), COPY);
        // Drop description inserts: bounded, printable.
        assert!(
            valid_description("") && valid_description("Documents") && valid_description("文档")
        );
        assert!(!valid_description("a\nb"));
        assert!(!valid_description(
            core::str::from_utf8(&[b'x'; MAX_DESCRIPTION + 1]).unwrap()
        ));
        let e = Event {
            kind: DROP,
            value: 9,
            a: 2,
            b: COPY,
            ..Event::default()
        };
        assert_eq!(decode(&e.encode()), Ok(e));
        assert!(
            decode(
                &Event {
                    kind: crate::clipboard::CHANGED,
                    ..e
                }
                .encode()
            )
            .is_err()
        );
    }
}

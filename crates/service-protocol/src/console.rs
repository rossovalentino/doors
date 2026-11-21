//! The console screens the console service keeps (`CONSOLE_STATE`): the
//! session's console (0) and the pseudoconsoles the Terminal opens, one per
//! tab running a program (1..CONSOLES), as Windows Terminal gives each tab
//! its own pseudoconsole. Each has its grid's size and the cursor as its
//! programs left it, the modes their output set, its input mode and typed
//! input. A client writes to and reads from the console its channel is
//! attached to (0 until `Attach`); a program the Terminal started attaches
//! to its tab's at startup (`DOOR_CONSOLE=<n>`, which Init gives it and its
//! children).

/// Windows Terminal's default window: 120 x 30.
pub const COLUMNS: usize = 120;
pub const ROWS: usize = 30;
/// Console ids: the session's console (0) and pseudoconsoles, one per
/// running Terminal tab program (the console service makes each on demand;
/// the id is one byte on the wire).
pub const CONSOLES: usize = 256;
/// The input mode a console starts with: ENABLE_PROCESSED_INPUT,
/// ENABLE_LINE_INPUT, ENABLE_ECHO_INPUT, ENABLE_MOUSE_INPUT,
/// ENABLE_INSERT_MODE, ENABLE_QUICK_EDIT_MODE, ENABLE_EXTENDED_FLAGS and
/// ENABLE_AUTO_POSITION, as conhost reports it.
pub const INPUT_MODE: u32 = 0x1f7;
/// `State::flags`.
pub const VISIBLE: u32 = 1;
pub const ALTERNATE: u32 = 2;
pub const APPLICATION_CURSOR: u32 = 4;
pub const BRACKETED_PASTE: u32 = 8;
pub const NEWLINE: u32 = 16;
pub const AUTOWRAP: u32 = 32;
pub const STATE_BYTES: usize = 32;
/// The prefix of `State` the Windows console APIs read (size, cursor,
/// margins, flags, input mode).
pub const LEGACY_STATE_BYTES: usize = 20;
pub const COMMAND_BYTES: usize = 12;
/// The largest grid a command may set.
pub const MAX_CELLS: u32 = 1000;
/// The environment variable naming a program's pseudoconsole.
pub const ENVIRONMENT: &str = "DOOR_CONSOLE=";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct State {
    pub columns: u16,
    pub rows: u16,
    /// The cursor's cell, from the top-left of the screen.
    pub x: u16,
    pub y: u16,
    /// The scrolling region.
    pub top: u16,
    pub bottom: u16,
    pub flags: u32,
    pub input_mode: u32,
    /// Which console this is (0 the session's).
    pub console: u16,
    /// Ctrl+C signals sent to the console's programs so far, and the last
    /// one a program's console control handlers took.
    pub signals: u32,
    pub seen: u32,
}
impl State {
    pub fn encode(&self) -> [u8; STATE_BYTES] {
        let mut out = [0; STATE_BYTES];
        for (i, v) in [
            self.columns,
            self.rows,
            self.x,
            self.y,
            self.top,
            self.bottom,
        ]
        .into_iter()
        .enumerate()
        {
            out[2 * i..2 * i + 2].copy_from_slice(&v.to_le_bytes());
        }
        out[12..16].copy_from_slice(&self.flags.to_le_bytes());
        out[16..20].copy_from_slice(&self.input_mode.to_le_bytes());
        out[20..22].copy_from_slice(&self.console.to_le_bytes());
        out[24..28].copy_from_slice(&self.signals.to_le_bytes());
        out[28..32].copy_from_slice(&self.seen.to_le_bytes());
        out
    }
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != STATE_BYTES {
            return None;
        }
        let half = |i: usize| u16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]]);
        let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let state = Self {
            columns: half(0),
            rows: half(1),
            x: half(2),
            y: half(3),
            top: half(4),
            bottom: half(5),
            flags: word(12),
            input_mode: word(16),
            console: half(10),
            signals: word(24),
            seen: word(28),
        };
        (state.columns > 0
            && state.seen <= state.signals
            && state.rows > 0
            && state.x < state.columns
            && state.y < state.rows
            && state.top <= state.bottom
            && state.bottom < state.rows
            && usize::from(state.console) < CONSOLES)
            .then_some(state)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Command {
    /// The Terminal opens a pseudoconsole for a program in a grid of this
    /// size, the cursor starting on `row` (below the text already shown)
    /// with the default pen and modes; the state replied names it.
    Open { columns: u32, rows: u32, row: u32 },
    /// The Terminal's grid of `console` changed size.
    Resize {
        console: u8,
        columns: u32,
        rows: u32,
    },
    /// The Terminal closes `console` (its program and children ended).
    Close(u8),
    /// The Terminal reads `console`'s state.
    Query(u8),
    /// This channel's programs use `console` (a program at startup).
    Attach(u8),
    /// SetConsoleMode on the input handle (the channel's console).
    InputMode(u32),
    /// FlushConsoleInputBuffer: typed input not yet read is dropped.
    FlushInput,
    /// A program's console control handlers took the Ctrl+C signals up
    /// to this count (the channel's console).
    Seen(u32),
}
impl Command {
    pub fn encode(&self) -> [u8; COMMAND_BYTES] {
        let (kind, console, a, b) = match *self {
            Self::Open { columns, rows, row } => (1u32, 0u8, columns, rows | (row << 16)),
            Self::Resize {
                console,
                columns,
                rows,
            } => (2, console, columns, rows),
            Self::InputMode(mode) => (3, 0, mode, 0),
            Self::FlushInput => (4, 0, 0, 0),
            Self::Close(console) => (5, console, 0, 0),
            Self::Attach(console) => (6, console, 0, 0),
            Self::Query(console) => (7, console, 0, 0),
            Self::Seen(signals) => (8, 0, signals, 0),
        };
        let mut out = [0; COMMAND_BYTES];
        out[..4].copy_from_slice(&(kind | u32::from(console) << 8).to_le_bytes());
        out[4..8].copy_from_slice(&a.to_le_bytes());
        out[8..].copy_from_slice(&b.to_le_bytes());
        out
    }
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != COMMAND_BYTES {
            return None;
        }
        let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let (kind, a, b) = (word(0), word(4), word(8));
        let console = (kind >> 8) as u8;
        if kind >> 16 != 0 || usize::from(console) >= CONSOLES {
            return None;
        }
        let cells = |v: u32| (1..=MAX_CELLS).contains(&v);
        let pseudo = console != 0;
        match kind & 0xff {
            1 if console == 0 && cells(a) && cells(b & 0xffff) && b >> 16 < (b & 0xffff) => {
                Some(Self::Open {
                    columns: a,
                    rows: b & 0xffff,
                    row: b >> 16,
                })
            }
            2 if pseudo && cells(a) && cells(b) => Some(Self::Resize {
                console,
                columns: a,
                rows: b,
            }),
            3 if console == 0 && b == 0 => Some(Self::InputMode(a)),
            4 if console == 0 && a == 0 && b == 0 => Some(Self::FlushInput),
            5 if pseudo && a == 0 && b == 0 => Some(Self::Close(console)),
            6 if a == 0 && b == 0 => Some(Self::Attach(console)),
            7 if a == 0 && b == 0 => Some(Self::Query(console)),
            8 if console == 0 && b == 0 => Some(Self::Seen(a)),
            _ => None,
        }
    }
    /// Only the session's Terminal opens, sizes, closes and reads
    /// pseudoconsoles.
    pub fn terminal_only(&self) -> bool {
        matches!(
            self,
            Self::Open { .. } | Self::Resize { .. } | Self::Close(_) | Self::Query(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn state_and_commands_round_trip_and_reject_malformed_bytes() {
        let state = State {
            columns: 120,
            rows: 30,
            x: 7,
            y: 29,
            top: 0,
            bottom: 29,
            flags: VISIBLE | NEWLINE,
            input_mode: INPUT_MODE,
            console: 3,
            signals: 2,
            seen: 1,
        };
        assert_eq!(State::decode(&state.encode()), Some(state));
        let mut outside = state;
        outside.y = 30;
        assert_eq!(State::decode(&outside.encode()), None);
        let mut unknown = state;
        unknown.console = CONSOLES as u16;
        assert_eq!(State::decode(&unknown.encode()), None);
        assert_eq!(State::decode(&state.encode()[..19]), None);
        for command in [
            Command::Open {
                columns: 80,
                rows: 25,
                row: 3,
            },
            Command::Resize {
                console: 2,
                columns: 1000,
                rows: 1,
            },
            Command::InputMode(0x200),
            Command::FlushInput,
            Command::Close(8),
            Command::Attach(0),
            Command::Query(4),
            Command::Seen(7),
        ] {
            assert_eq!(Command::decode(&command.encode()), Some(command));
        }
        // The session console is not resized or closed by command.
        let mut zero = Command::Close(1).encode();
        zero[1] = 0;
        assert_eq!(Command::decode(&zero), None);
        let mut too_big = Command::Resize {
            console: 1,
            columns: 1001,
            rows: 1,
        }
        .encode();
        assert_eq!(Command::decode(&too_big), None);
        too_big[0] = 9;
        assert_eq!(Command::decode(&too_big), None);
        assert_eq!(Command::decode(&[0; 11]), None);
        assert!(Command::Query(1).terminal_only());
        assert!(!Command::Attach(1).terminal_only());
    }
}

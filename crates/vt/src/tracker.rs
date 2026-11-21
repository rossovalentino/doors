//! The cursor alone, as the console service follows a program's output:
//! enough to answer GetConsoleScreenBufferInfo and the cursor-position
//! report (DSR 6) while the program writes, without keeping cells.
use crate::cursor::Cursor;
use crate::parser::{Parser, Sink};

const REPLY_BYTES: usize = 128;

pub struct Tracker {
    cursor: Cursor,
    parser: Parser,
    replies: [u8; REPLY_BYTES],
    reply_len: usize,
}
struct View<'a> {
    cursor: &'a mut Cursor,
    replies: &'a mut [u8; REPLY_BYTES],
    reply_len: &'a mut usize,
}
impl Sink for View<'_> {
    fn cursor(&mut self) -> &mut Cursor {
        self.cursor
    }
    fn reply(&mut self, bytes: &[u8]) {
        let at = *self.reply_len;
        if at + bytes.len() <= REPLY_BYTES {
            self.replies[at..at + bytes.len()].copy_from_slice(bytes);
            *self.reply_len += bytes.len();
        }
    }
}
impl Tracker {
    pub const fn new(columns: usize, rows: usize) -> Self {
        Self {
            cursor: Cursor::new(columns, rows),
            parser: Parser::new(),
            replies: [0; REPLY_BYTES],
            reply_len: 0,
        }
    }
    pub fn feed(&mut self, bytes: &[u8]) {
        let mut view = View {
            cursor: &mut self.cursor,
            replies: &mut self.replies,
            reply_len: &mut self.reply_len,
        };
        self.parser.feed(&mut view, bytes);
    }
    pub fn cursor(&self) -> &Cursor {
        &self.cursor
    }
    pub fn place(&mut self, x: usize, y: usize) {
        self.cursor.place(x, y);
    }
    /// A new size (the Terminal's grid), as `Screen::resize` follows it.
    pub fn resize(&mut self, columns: usize, rows: usize) {
        self.cursor.resize(columns, rows);
    }
    /// A new program's empty screen: home, default pen and modes.
    pub fn reset(&mut self) {
        let (columns, rows) = (self.cursor.columns(), self.cursor.rows());
        self.cursor = Cursor::new(columns, rows);
        self.parser = Parser::new();
        self.reply_len = 0;
    }
    /// Replies the program asked for since the last call.
    pub fn take_replies(&mut self, out: &mut [u8]) -> usize {
        let n = self.reply_len.min(out.len());
        out[..n].copy_from_slice(&self.replies[..n]);
        self.replies.copy_within(n..self.reply_len, 0);
        self.reply_len -= n;
        n
    }
}

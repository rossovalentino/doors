//! The terminal state that is not cells: the cursor (position, pending
//! wrap, visibility), the pen printed characters take, the scrolling
//! margins and the modes. The console service tracks this alone (to
//! answer cursor queries as the program writes); a screen keeps it beside
//! its cells. Both move it with the same code, so they always agree.

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum Color {
    #[default]
    Default,
    /// The 16 console colours (0-15) and the xterm 256-colour cube.
    Indexed(u8),
    Rgb(u8, u8, u8),
}

pub mod attr {
    pub const BOLD: u16 = 1;
    pub const DIM: u16 = 2;
    pub const ITALIC: u16 = 4;
    pub const UNDERLINE: u16 = 8;
    pub const BLINK: u16 = 16;
    pub const INVERSE: u16 = 32;
    pub const HIDDEN: u16 = 64;
    pub const STRIKE: u16 = 128;
    pub const DOUBLE_UNDERLINE: u16 = 256;
}

/// What printed characters take.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Pen {
    pub fg: Color,
    pub bg: Color,
    pub attrs: u16,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Saved {
    x: usize,
    y: usize,
    pen: Pen,
    origin: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cursor {
    columns: usize,
    rows: usize,
    pub(crate) x: usize,
    pub(crate) y: usize,
    /// The last column was written: the next character wraps first.
    pub(crate) wrap_pending: bool,
    pub pen: Pen,
    saved: Saved,
    pub(crate) top: usize,
    pub(crate) bottom: usize,
    pub(crate) origin: bool,
    pub autowrap: bool,
    /// IRM: printed characters shift the rest of the row right.
    pub insert: bool,
    pub visible: bool,
    /// DECCKM: the cursor keys send SS3 (application) sequences.
    pub application_cursor: bool,
    pub bracketed_paste: bool,
    /// LNM: a line feed also returns to the first column, as the Windows
    /// console does unless a program sets DISABLE_NEWLINE_AUTO_RETURN.
    pub newline: bool,
    /// The alternate screen is shown (rows leaving its top are lost, not
    /// kept as scrollback).
    pub alternate: bool,
}

impl Cursor {
    pub const fn new(columns: usize, rows: usize) -> Self {
        let columns = if columns == 0 { 1 } else { columns };
        let rows = if rows == 0 { 1 } else { rows };
        Self {
            columns,
            rows,
            x: 0,
            y: 0,
            wrap_pending: false,
            pen: Pen {
                fg: Color::Default,
                bg: Color::Default,
                attrs: 0,
            },
            saved: Saved {
                x: 0,
                y: 0,
                pen: Pen {
                    fg: Color::Default,
                    bg: Color::Default,
                    attrs: 0,
                },
                origin: false,
            },
            top: 0,
            bottom: rows - 1,
            origin: false,
            autowrap: true,
            insert: false,
            visible: true,
            application_cursor: false,
            bracketed_paste: false,
            newline: true,
            alternate: false,
        }
    }
    pub fn columns(&self) -> usize {
        self.columns
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    /// The cursor's cell (a pending wrap keeps it on the last column).
    pub fn position(&self) -> (usize, usize) {
        (self.x.min(self.columns - 1), self.y)
    }
    /// The scrolling region's first and last rows.
    pub fn margins(&self) -> (usize, usize) {
        (self.top, self.bottom)
    }
    /// The cursor's linear cell index.
    pub(crate) fn at(&self) -> usize {
        self.y * self.columns + self.x.min(self.columns - 1)
    }
    /// A new size, keeping the cursor's row on screen: returns how many
    /// rows at the top no longer fit (a shrinking terminal keeps the
    /// bottom), which a screen moves to its scrollback.
    pub fn resize(&mut self, columns: usize, rows: usize) -> usize {
        let (columns, rows) = (columns.max(1), rows.max(1));
        let shift = if rows < self.rows && self.y >= rows {
            self.y + 1 - rows
        } else {
            0
        };
        self.columns = columns;
        self.rows = rows;
        self.y = self.y.saturating_sub(shift).min(rows - 1);
        self.x = self.x.min(columns - 1);
        self.top = 0;
        self.bottom = rows - 1;
        self.wrap_pending = false;
        self.saved.x = self.saved.x.min(columns - 1);
        self.saved.y = self.saved.y.min(rows - 1);
        shift
    }
    /// CUP: `x`, `y` from the origin (the region's top in origin mode).
    pub(crate) fn move_to(&mut self, x: usize, y: usize) {
        let (low, high) = if self.origin {
            (self.top, self.bottom)
        } else {
            (0, self.rows - 1)
        };
        self.x = x.min(self.columns - 1);
        self.y = y
            .saturating_add(if self.origin { self.top } else { 0 })
            .clamp(low, high);
        self.wrap_pending = false;
    }
    pub(crate) fn save(&mut self) {
        self.saved = Saved {
            x: self.x.min(self.columns - 1),
            y: self.y,
            pen: self.pen,
            origin: self.origin,
        };
    }
    pub(crate) fn restore(&mut self) {
        let s = self.saved;
        self.x = s.x.min(self.columns - 1);
        self.y = s.y.min(self.rows - 1);
        self.pen = s.pen;
        self.origin = s.origin;
        self.wrap_pending = false;
    }
    /// RIS: everything but the size.
    pub(crate) fn reset(&mut self) {
        *self = Self::new(self.columns, self.rows);
    }
    /// Put the cursor on a cell (clamped to the screen), as a screen that
    /// starts below text already shown places it.
    pub fn place(&mut self, x: usize, y: usize) {
        self.x = x.min(self.columns - 1);
        self.y = y.min(self.rows - 1);
        self.wrap_pending = false;
    }
    /// DECSTBM.
    pub(crate) fn set_margins(&mut self, top: usize, bottom: usize) {
        let bottom = bottom.min(self.rows - 1);
        if top < bottom {
            self.top = top;
            self.bottom = bottom;
            self.move_to(0, 0);
        }
    }
}

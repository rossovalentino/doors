//! A screen of cells (character, colours, attributes) driven through the
//! parser, as a Windows Terminal tab shows a running program. Rows that a
//! line feed pushes off the main screen's top go to `scrolled` for the
//! caller's scrollback.
use crate::cursor::{Color, Cursor};
use crate::parser::Sink;
use alloc::{string::String, vec, vec::Vec};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub attrs: u16,
}
impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::Default,
            bg: Color::Default,
            attrs: 0,
        }
    }
}

pub struct Screen {
    cursor: Cursor,
    cells: Vec<Cell>,
    /// The main screen's cells while the alternate one is shown.
    main: Option<Vec<Cell>>,
    /// OSC 0/2: the program's title for the tab.
    pub title: Option<String>,
    /// Rows that scrolled off the top of the main screen, oldest first.
    pub scrolled: Vec<Vec<Cell>>,
    /// Replies to reports the program asked for (DSR, DA), for its input.
    pub replies: Vec<u8>,
    /// The visible content changed since the caller last cleared this.
    pub dirty: bool,
}

impl Screen {
    pub fn new(columns: usize, rows: usize) -> Self {
        let cursor = Cursor::new(columns, rows);
        Self {
            cells: vec![Cell::default(); cursor.columns() * cursor.rows()],
            cursor,
            main: None,
            title: None,
            scrolled: Vec::new(),
            replies: Vec::new(),
            dirty: true,
        }
    }
    pub fn columns(&self) -> usize {
        self.cursor.columns()
    }
    pub fn rows(&self) -> usize {
        self.cursor.rows()
    }
    /// The cursor's cell.
    pub fn cursor(&self) -> (usize, usize) {
        self.cursor.position()
    }
    pub fn state(&self) -> &Cursor {
        &self.cursor
    }
    pub fn alternate(&self) -> bool {
        self.cursor.alternate
    }
    pub fn row(&self, y: usize) -> &[Cell] {
        let columns = self.columns();
        &self.cells[y * columns..(y + 1) * columns]
    }
    /// A row's text without trailing blanks.
    pub fn row_text(&self, y: usize) -> String {
        let row = self.row(y);
        let end = row.iter().rposition(|c| c.ch != ' ').map_or(0, |i| i + 1);
        row[..end].iter().map(|c| c.ch).collect()
    }
    fn blank(&self) -> Cell {
        Cell {
            ch: ' ',
            fg: self.cursor.pen.fg,
            bg: self.cursor.pen.bg,
            attrs: 0,
        }
    }
    pub fn place(&mut self, x: usize, y: usize) {
        self.cursor.place(x, y);
    }
    /// Text already shown (the scrollback's last rows) on row `y`, in the
    /// default pen.
    pub fn set_row(&mut self, y: usize, text: &str) {
        if y >= self.rows() {
            return;
        }
        let columns = self.columns();
        let row = &mut self.cells[y * columns..(y + 1) * columns];
        row.fill(Cell::default());
        for (cell, ch) in row.iter_mut().zip(text.chars()) {
            cell.ch = if ch.is_control() { ' ' } else { ch };
        }
        self.dirty = true;
    }
    /// Resize, keeping the top-left of the content and the cursor's row on
    /// screen (rows that no longer fit above it go to the scrollback).
    pub fn resize(&mut self, columns: usize, rows: usize) {
        let (old_columns, old_rows) = (self.columns(), self.rows());
        if columns.max(1) == old_columns && rows.max(1) == old_rows {
            return;
        }
        let shift = self.cursor.resize(columns, rows);
        let (columns, rows) = (self.columns(), self.rows());
        let refit = |cells: &[Cell], shift: usize| {
            let mut out = vec![Cell::default(); columns * rows];
            for y in 0..rows {
                let from = y + shift;
                if from >= old_rows {
                    break;
                }
                for x in 0..columns.min(old_columns) {
                    out[y * columns + x] = cells[from * old_columns + x];
                }
            }
            out
        };
        if !self.cursor.alternate {
            for y in 0..shift {
                self.scrolled
                    .push(self.cells[y * old_columns..(y + 1) * old_columns].to_vec());
            }
        }
        self.cells = refit(&self.cells, shift);
        if let Some(main) = self.main.take() {
            self.main = Some(refit(&main, 0));
        }
        self.dirty = true;
    }
}

impl Sink for Screen {
    fn cursor(&mut self) -> &mut Cursor {
        &mut self.cursor
    }
    fn put(&mut self, ch: char) {
        let columns = self.columns();
        let (x, y) = self.cursor.position();
        let start = y * columns;
        if self.cursor.insert {
            self.cells
                .copy_within(start + x..start + columns - 1, start + x + 1);
        }
        let pen = self.cursor.pen;
        self.cells[start + x] = Cell {
            ch,
            fg: pen.fg,
            bg: pen.bg,
            attrs: pen.attrs,
        };
        self.dirty = true;
    }
    fn scroll_up(&mut self, top: usize, bottom: usize, count: usize, keep: bool) {
        let columns = self.columns();
        let blank = self.blank();
        for _ in 0..count.min(bottom - top + 1) {
            if keep && top == 0 && !self.cursor.alternate {
                self.scrolled.push(self.row(0).to_vec());
            }
            self.cells
                .copy_within((top + 1) * columns..(bottom + 1) * columns, top * columns);
            self.cells[bottom * columns..(bottom + 1) * columns].fill(blank);
        }
        self.dirty = true;
    }
    fn scroll_down(&mut self, top: usize, bottom: usize, count: usize) {
        let columns = self.columns();
        let blank = self.blank();
        for _ in 0..count.min(bottom - top + 1) {
            self.cells
                .copy_within(top * columns..bottom * columns, (top + 1) * columns);
            self.cells[top * columns..(top + 1) * columns].fill(blank);
        }
        self.dirty = true;
    }
    fn erase(&mut self, from: usize, to: usize) {
        let blank = self.blank();
        let end = to.min(self.cells.len());
        self.cells[from.min(end)..end].fill(blank);
        self.dirty = true;
    }
    fn shift(&mut self, insert: bool, count: usize) {
        let columns = self.columns();
        let (x, y) = self.cursor.position();
        let start = y * columns;
        let n = count.min(columns - x);
        if insert {
            self.cells
                .copy_within(start + x..start + columns - n, start + x + n);
            self.erase(start + x, start + x + n);
        } else {
            self.cells
                .copy_within(start + x + n..start + columns, start + x);
            self.erase(start + columns - n, start + columns);
        }
    }
    fn alternate(&mut self, on: bool) {
        let size = self.cells.len();
        if on {
            let main = core::mem::replace(&mut self.cells, vec![Cell::default(); size]);
            self.main = Some(main);
        } else {
            self.cells = self
                .main
                .take()
                .filter(|m| m.len() == size)
                .unwrap_or_else(|| vec![Cell::default(); size]);
        }
        self.dirty = true;
    }
    fn reset(&mut self) {
        self.cells.fill(Cell::default());
        self.main = None;
        self.dirty = true;
    }
    fn title(&mut self, title: &str) {
        self.title = Some(
            title
                .chars()
                .filter(|c| !c.is_control())
                .take(240)
                .collect(),
        );
        self.dirty = true;
    }
    fn reply(&mut self, bytes: &[u8]) {
        if self.replies.len() < 4096 {
            self.replies.extend_from_slice(bytes);
        }
    }
}

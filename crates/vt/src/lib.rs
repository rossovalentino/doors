#![no_std]
//! The console as Windows Terminal and the Windows console host keep it
//! for a running program: its output parsed as UTF-8 text with C0
//! controls and xterm/VT escape sequences (cursor movement and position,
//! erase, insert/delete, scroll regions, SGR colours: the 16 console
//! colours, 256 indexed and 24-bit, bold/dim/italic/underline/blink/
//! inverse/hidden/strike, the alternate screen, cursor visibility, the
//! window title, and the device status/attribute reports a program asks
//! for), and its input as key events.
//!
//! One parser drives either a `Screen` of cells (the Terminal's grid,
//! with the `alloc` feature) or a `Tracker` that keeps only the cursor
//! (the console service, answering cursor queries as the program
//! writes); both move the cursor with the same code.
#[cfg(feature = "alloc")]
extern crate alloc;

mod cursor;
pub mod input;
mod parser;
#[cfg(feature = "alloc")]
mod screen;
mod tracker;

pub use cursor::{Color, Cursor, Pen, attr};
pub use parser::{Parser, Sink};
#[cfg(feature = "alloc")]
pub use screen::{Cell, Screen};
pub use tracker::Tracker;

#[cfg(test)]
mod tests;

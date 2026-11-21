#![no_std]
//! Door locale data (spec §109): time zones, regional formats and the
//! Explorer name collation. Allocation-free; data are versioned test inputs.
//!
//! * `tz` — compact current-rule table derived from the IANA tz database
//!   (release 2025b, public domain): standard offsets and recurring DST rules
//!   only, no historical transitions. UTC to local conversion is exact for
//!   instants under the current rules; local to UTC reports ambiguous and
//!   nonexistent times explicitly.
//! * `region` — Windows default short/long date and time patterns, first day
//!   of week and number separators for seven regional formats.
//! * `collate` — StrCmpLogicalW-style numeric, case-insensitive comparison.
pub mod collate;
pub mod region;
pub mod tz;

pub use door_abi::clock::{FILETIME_PER_SECOND, WallClock};

#[cfg(test)]
extern crate std;

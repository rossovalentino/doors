#![no_std]
#![forbid(unsafe_code)]
//! Deterministic CPU window authority. Callers supply kernel-authenticated owners
//! and owned, fault-safe pixel captures, never shared client-memory references.
extern crate alloc;
mod cursor;
mod engine;
pub use cursor::CursorIcon;
mod events;
mod geometry;
mod placement;
mod render;
mod resample;
mod shadow;
pub use resample::{area_resample, area_resample_bgra, area_sample};
pub use shadow::{ShadowMargins, WINDOW_SHADOW, window_shadow_bounds};
mod surface;
/// Window transition durations (ms); MOTION-01 judges guest records against them.
pub use engine::animation::{
    CLOSE_MS as WINDOW_CLOSE_MS, MINIMIZE_MS as WINDOW_MINIMIZE_MS, OPEN_MS as WINDOW_OPEN_MS,
    RESTORE_MS as WINDOW_RESTORE_MS, SNAP_MS as WINDOW_SNAP_MS,
};
pub use engine::*;
pub use events::*;
pub use geometry::*;
pub use placement::*;
pub use render::*;
pub use surface::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidGeometry,
    InvalidSurface,
    InvalidPixels,
    UnknownId,
    WrongOwner,
    Quota,
    UnsupportedRole,
    StaleEpoch,
    NotAcknowledged,
    NoAttachment,
    Busy,
    Closing,
    QueueFull,
    InvalidState,
    UnknownFrame,
}
pub type Result<T> = core::result::Result<T, Error>;

#[cfg(test)]
extern crate std;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod bench_tests;
#[cfg(test)]
mod compositor_tests;
#[cfg(test)]
mod display_scale_tests;
#[cfg(test)]
mod snap_tests;
#[cfg(test)]
mod tabbed_caption_tests;

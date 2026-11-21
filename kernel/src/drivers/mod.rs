//! Original, source-versioned device framework for the selected PCI profile.
//! A live device owns its BAR and DMA mappings until acknowledged quiescence.
pub mod pci;
mod queue;
pub mod virtio_block;
pub mod virtio_net;
mod virtio_pci;
pub mod virtio_sound;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    NotFound,
    Ambiguous,
    Configuration,
    Unsupported,
    Bounds,
    Alignment,
    QueueFull,
    Stale,
    InFlight,
    Canceled,
    Reset,
    ShortIo,
    Malformed,
    Io,
    Timeout,
    Removed,
}
pub type Result<T> = core::result::Result<T, Error>;

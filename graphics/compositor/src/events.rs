use crate::{BufferId, Rect, WindowId};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitRegion {
    Client,
    TitleBar,
    Close,
    Minimize,
    Maximize,
    Resize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    PointerMove {
        x: i32,
        y: i32,
    },
    PointerButton {
        button: u8,
        down: bool,
        x: i32,
        y: i32,
    },
    Key {
        code: u32,
        down: bool,
    },
    /// Wheel detents at the pointer (> 0 = away from the user). Routed to the
    /// window under the pointer (or the capture owner) without activating it.
    Wheel {
        x: i32,
        y: i32,
        delta: i32,
    },
}
/// Coalesced wheel deltas saturate here (the device ABI bound).
pub const MAX_WHEEL_DELTA: i32 = 127;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentationStatus {
    CpuTransferred,
    TransferFailed,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    DismissTransients {
        window: WindowId,
    },
    Configure {
        window: WindowId,
        epoch: u64,
        outer: Rect,
        content_width: u32,
        content_height: u32,
    },
    BufferReleased {
        buffer: BufferId,
    },
    Input {
        window: WindowId,
        input: Input,
        timestamp: u64,
    },
    Focus {
        window: WindowId,
        focused: bool,
    },
    CaptureLost {
        window: WindowId,
    },
    CloseRequested {
        window: WindowId,
    },
    Frame {
        window: WindowId,
        serial: u64,
    },
    Presentation {
        window: WindowId,
        serial: u64,
        timestamp: u64,
        status: PresentationStatus,
    },
}

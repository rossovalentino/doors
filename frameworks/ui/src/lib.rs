#![no_std]
#![forbid(unsafe_code)]
//! Door UI: the native retained toolkit (master specification sections
//! 49-51, 55, 58 and 113).
//!
//! * `Ui` — the retained widget tree with generational `WidgetId`s, event
//!   routing (capture/target/bubble), pointer capture, the focus manager
//!   (tab order, focus scopes, modality, restoration), layers (dialogs,
//!   flyouts), owner popups, tool tips, async results tagged by widget
//!   generation, and the inspector.
//! * `layout` — stacks with flex weights, grids, overlays, scrolling,
//!   baseline alignment, min/max, in fixed-point logical units (`units`).
//! * `display` / `render` — backend-neutral display lists with exact damage
//!   accounting, replayed on the CPU canvas at any display scale.
//! * `theme` — immutable token snapshots: light, dark, contrast themes,
//!   text size, reduced motion and transparency.
//! * `controls` — the Windows 11 control catalog with state machines and
//!   semantics; `door_accessibility` carries the semantic tree.
//!
//! Application lifecycle: create a `Ui` with a theme and a `TextSystem`,
//! build widgets, then per platform event call the input methods, read
//! `take_notifications`, update models with `edit`, and on each frame call
//! `render` (layout, paint and semantics run only when invalidated) and
//! publish `take_semantics`. The native platform adapter is `door-ui-native`.
extern crate alloc;
pub mod controls;
pub mod display;
pub mod event;
pub mod id;
pub mod layout;
pub mod render;
pub mod text;
pub mod theme;
mod ui;
pub mod units;
pub mod widget;
pub use door_accessibility as accessibility;
pub use event::{
    Button as MouseButton, Composition, Event, Key, KeyEvent, Modifiers, Phase, Pointer,
};
pub use id::WidgetId;
pub use layout::{Align, Axis, Dim, Justify, Kind, Measured, Style, Track};
pub use render::{Fonts, Frame, RenderStats};
pub use text::{AtlasText, TextSystem};
pub use theme::{Color, TextRole, Theme};
pub use ui::*;
pub use units::{Insets, LPoint, LRect, LSize, Lu};
pub use widget::{
    Clipboard, Cursor, EventCx, LayoutCx, LocalClipboard, Notification, Notify, PaintCx,
    SemanticsCx, Widget, WidgetState,
};

#[cfg(test)]
extern crate std;
#[cfg(test)]
mod tests;

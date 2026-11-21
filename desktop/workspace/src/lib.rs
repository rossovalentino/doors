#![no_std]
//! Reusable application interaction models, independently host-tested.
/// Application resource bytes by source-relative asset path, served from the
/// shared runtime pack instead of being compiled in (see `assets`).
#[macro_export]
macro_rules! asset {
    ($path:expr) => {
        $crate::assets::get($path)
    };
}
pub mod archive_codec;
pub mod assets;
pub mod audio_codec;
pub mod https;
pub mod lzma;
pub mod phonebook;
pub mod proxy;
pub mod rar5;
pub mod sevenzip;
/// Display scale carried by `ui::View` (percent; snapped logical edges).
pub use door_graphics_core::Scale;
pub mod caption_tabs;
pub mod context_menu;
pub mod context_menu_profile;
pub mod converter;
pub mod default_apps;
pub mod dialog;
pub mod document;
pub mod document_format;
pub mod document_layout;
pub mod document_prompt;
pub mod document_save;
pub mod document_tabs;
pub mod document_view;
pub mod explorer_dnd;
pub mod explorer_layout;
pub mod explorer_path;
pub mod explorer_preview;
pub mod explorer_search;
pub mod explorer_size;
pub mod explorer_sort;
pub mod explorer_tags;
pub mod explorer_tree;
pub mod file_metadata;
pub mod file_name;
pub mod file_operations;
pub mod file_removal;
pub mod installed_programs;
pub mod line_edit;
pub mod media_preview;
pub mod menu;
pub mod nav_pane;
pub mod open_with;
pub mod palette;
pub mod program_icon;
#[cfg(not(target_os = "none"))]
#[doc(hidden)]
pub mod raster_check;
pub mod recent;
pub mod registry_browser;
pub mod registry_value;
pub mod security_warning;
pub mod semantics;
pub mod settings_catalog;
pub mod settings_desktop_layout;
pub mod settings_layout;
pub mod settings_navigation;
pub mod settings_watch;
pub mod shell_search;
pub mod terminal_model;
pub mod terminal_selection;
pub mod text_entry;

extern crate alloc;
extern crate self as door_workspace;
pub mod fluent;
pub mod icons;
pub mod runtime;
mod surface;
/// A new section holding a copy of `bytes` for a service that snapshots
/// it (a clipboard image, a staged save); the caller closes the handle.
pub fn section_from(bytes: &[u8]) -> Result<door_abi::Handle, door_abi::Status> {
    surface::upload_section(bytes)
}
/// Private-frame admission used by window surfaces (host-tested budgets).
pub use surface::{FRAME_MARGIN, admit_frame, grow_frame};
mod tokens;
pub mod ui;
pub use ui::{ImageCache, View};

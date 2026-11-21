//! The built-in schema: every preference Door OS persists today, with its
//! storage word, bit field and encoding unchanged.
//!
//! Words are the storage service's per-user `Setting` ids 0..=8 and the
//! validators are storage-core's (`persistence/config.rs`). No HKLM or
//! other typed registry value is persisted by Settings today: the
//! Environment variables page edits free-form HKCU\Environment strings and
//! the Registry Editor edits raw values, so neither is a typed property.
//! Machine-scope [`Store::Registry`] words are supported for privileged
//! brokers as they gain real providers.
use crate::schema::{
    Apply, Catalog, Coupling, Dependency, Kind, Property, Schema, Scope, Store, Visibility, Word,
};
use door_keyboard_layouts::Preferences;
use door_service_client::Setting;
use door_service_protocol::desktop::{Controls, ShellLayout};
use door_settings_map::Page;

/// Stable keys of the built-in properties.
pub mod key {
    pub const CONSOLE_THEME: &str = "console.theme";
    pub const DISPLAY_SCALE: &str = "display.scale";
    pub const APPEARANCE_MODE: &str = "appearance.mode";
    pub const APPEARANCE_BACKGROUND: &str = "appearance.background";
    pub const TASKBAR_POSITION: &str = "taskbar.position";
    pub const TASKBAR_ALIGNMENT: &str = "taskbar.alignment";
    pub const TASKBAR_SIZE: &str = "taskbar.size";
    pub const CONTEXT_INLINE_ACTIONS: &str = "context_menu.inline_actions";
    pub const CONTEXT_PROPERTIES_PLACEMENT: &str = "context_menu.properties_placement";
    pub const DESKTOP_ICONS: &str = "desktop.show_icons";
    pub const START_LAYOUT: &str = "start.layout";
    pub const REDUCE_TRANSPARENCY: &str = "appearance.reduce_transparency";
    pub const CONTEXT_CLASSIC: &str = "context_menu.classic";
    pub const TASKBAR_SEARCH: &str = "taskbar.search";
    pub const REDUCE_MOTION: &str = "accessibility.reduce_motion";
    pub const START_SIZE: &str = "start.size";
    pub const START_HIDE_PINNED: &str = "start.hide_pinned";
    pub const START_HIDE_RECENT: &str = "start.hide_recent";
    pub const START_HIDE_ALL: &str = "start.hide_all";
    pub const START_HIDE_ACCOUNT: &str = "start.hide_account";
    pub const START_HIDE_RECENT_FILES: &str = "start.hide_recent_files";
    pub const CONTEXT_HIDE_FILE_ACTIONS: &str = "context_menu.hide_file_actions";
    pub const CONTEXT_HIDE_MORE_OPTIONS: &str = "context_menu.hide_more_options";
    pub const CONTRAST_THEME: &str = "accessibility.contrast_theme";
    pub const CONTRAST: &str = "accessibility.contrast";
    pub const TEXT_SIZE: &str = "accessibility.text_size";
    pub const TASKBAR_UNPINNED: &str = "taskbar.unpinned_apps";
    pub const START_UNPINNED: &str = "start.unpinned_apps";
    pub const START_ALL_VIEW: &str = "start.all_view";
    pub const START_ALL_SORTED: &str = "start.all_sorted";
    pub const START_PIN_ORDER: &str = "start.pin_order";
    pub const TASKBAR_PIN_ORDER: &str = "taskbar.pin_order";
    pub const INPUT_METHODS: &str = "input.methods";
    pub const INPUT_PER_WINDOW: &str = "input.per_window";
    pub const TIME_ZONE: &str = "time.zone";
    pub const REGIONAL_FORMAT: &str = "region.format";
}

// Word indices (the order of `WORDS`).
const THEME: u8 = 0;
const SCALE: u8 = 1;
const APPEARANCE: u8 = 2;
const CONTROLS: u8 = 3;
const SHELL_LAYOUT: u8 = 4;
const SHELL_ORDER: u8 = 5;
const INPUT: u8 = 6;
const ZONE: u8 = 7;
const REGION: u8 = 8;

const fn user(store: Setting, default: u32, valid: fn(u32) -> bool) -> Word {
    Word {
        store: Store::Setting(store),
        default,
        valid,
        scope: Scope::User,
        write: crate::schema::Authority::User,
        couple: None,
    }
}

/// storage-core: `Theme` and `DesktopAppearance` share the dark bit. Writing
/// `Theme` sets it to `theme != 0`; writing `DesktopAppearance` stores its
/// bit 0 as the theme.
fn theme_to_appearance(theme: u32, appearance: u32) -> u32 {
    (appearance & !1) | u32::from(theme != 0)
}
fn appearance_to_theme(appearance: u32, _theme: u32) -> u32 {
    appearance & 1
}
fn valid_scale(value: u32) -> bool {
    value == 0 || (100..=300).contains(&value) && value.is_multiple_of(25)
}

pub static WORDS: [Word; 9] = [
    Word {
        couple: Some(Coupling {
            word: APPEARANCE,
            map: theme_to_appearance,
        }),
        ..user(Setting::Theme, 0, |v| v <= 2)
    },
    user(Setting::ScalePercent, 0, valid_scale),
    Word {
        couple: Some(Coupling {
            word: THEME,
            map: appearance_to_theme,
        }),
        ..user(Setting::DesktopAppearance, 0, |v| {
            v < door_service_protocol::window::APPEARANCE_VALUES
        })
    },
    user(Setting::DesktopControls, Controls::DEFAULT.bits(), |v| {
        Controls::from_bits(v).is_some()
    }),
    user(Setting::ShellLayout, 0, ShellLayout::flags_valid),
    user(Setting::ShellOrder, 0, ShellLayout::order_valid),
    user(Setting::InputMethods, Preferences::DEFAULT.bits(), |v| {
        Preferences::from_bits(v).is_some()
    }),
    user(
        Setting::TimeZone,
        door_locale::tz::DEFAULT_ID as u32,
        door_locale::tz::valid,
    ),
    user(
        Setting::RegionalFormat,
        door_locale::region::DEFAULT_ID as u32,
        door_locale::region::valid,
    ),
];

static ZONES: Catalog = Catalog {
    name: |id| door_locale::tz::by_id(id).map(|z| z.display),
    nth: |i| door_locale::tz::ZONES.get(i).map(|z| u32::from(z.id)),
};
static REGIONS: Catalog = Catalog {
    name: |id| door_locale::region::by_id(id).map(|r| r.name),
    nth: |i| door_locale::region::REGIONS.get(i).map(|r| u32::from(r.id)),
};
/// The six desktop apps in pin-mask bit order (window server `App::index`).
const APPS: &[&str] = &[
    "File Explorer",
    "Notes",
    "Settings",
    "Terminal",
    "Preview",
    "Task Manager",
];

#[allow(clippy::too_many_arguments)]
const fn p(
    key: &'static str,
    title: &'static str,
    keywords: &'static str,
    kind: Kind,
    default: u32,
    word: u8,
    shift: u8,
    width: u8,
    page: Page,
    anchor: Option<&'static str>,
) -> Property {
    Property {
        key,
        title,
        keywords,
        kind,
        default,
        word,
        shift,
        width,
        apply: Apply::Live,
        page,
        anchor,
        visibility: Visibility::Public,
        depends: &[],
    }
}
const fn flag(
    key: &'static str,
    title: &'static str,
    keywords: &'static str,
    word: u8,
    bit: u8,
    page: Page,
    anchor: Option<&'static str>,
) -> Property {
    p(
        key,
        title,
        keywords,
        Kind::Bool,
        0,
        word,
        bit,
        1,
        page,
        anchor,
    )
}

use key::*;
pub static PROPERTIES: [Property; 36] = [
    // Setting 0: the console theme (shell `theme`), coupled to the dark bit.
    p(
        CONSOLE_THEME,
        "Console color theme",
        "terminal console shell theme colors",
        Kind::Choice(&[(0, "Default"), (1, "Theme 1"), (2, "Theme 2")]),
        0,
        THEME,
        0,
        32,
        Page::SystemAdvanced,
        Some("terminal"),
    ),
    // Setting 1: 0 automatic or 100..=300 in steps of 25.
    p(
        DISPLAY_SCALE,
        "Scale",
        "display scale dpi zoom size of text apps and other items",
        Kind::Integer {
            min: 100,
            max: 300,
            step: 25,
            special: Some((0, "Automatic")),
        },
        0,
        SCALE,
        0,
        32,
        Page::Display,
        Some("scale"),
    ),
    // Setting 2: bit 0 dark, bits 1..=5 the background picture.
    p(
        APPEARANCE_MODE,
        "Choose your mode",
        "dark mode light mode theme appearance colors",
        Kind::Choice(&[(0, "Light"), (1, "Dark")]),
        0,
        APPEARANCE,
        0,
        1,
        Page::Colors,
        None,
    ),
    p(
        APPEARANCE_BACKGROUND,
        "Personalize your background",
        "background wallpaper picture image desktop",
        // The 18 pictures of `door_initfs::desktop_assets::WALLPAPERS`, by id.
        Kind::Choice(&[
            (0, "Bloom"),
            (1, "Dark Bloom"),
            (2, "Glow 1"),
            (3, "Sunrise 1"),
            (4, "Glow 2"),
            (5, "Glow 3"),
            (6, "Glow 4"),
            (7, "Captured Motion 1"),
            (8, "Captured Motion 2"),
            (9, "Captured Motion 3"),
            (10, "Captured Motion 4"),
            (11, "Sunrise 2"),
            (12, "Sunrise 3"),
            (13, "Sunrise 4"),
            (14, "Flow 1"),
            (15, "Flow 2"),
            (16, "Flow 3"),
            (17, "Flow 4"),
        ]),
        0,
        APPEARANCE,
        1,
        5,
        Page::Background,
        None,
    ),
    // Setting 3: `desktop::Controls`, every bit.
    p(
        TASKBAR_POSITION,
        "Taskbar position",
        "taskbar location edge top bottom left right screen",
        Kind::Choice(&[(0, "Bottom"), (1, "Top"), (2, "Left"), (3, "Right")]),
        0,
        CONTROLS,
        0,
        2,
        Page::Taskbar,
        None,
    ),
    p(
        TASKBAR_ALIGNMENT,
        "Taskbar alignment",
        "taskbar alignment left center start button",
        Kind::Choice(&[(0, "Center"), (1, "Left")]),
        0,
        CONTROLS,
        2,
        1,
        Page::Taskbar,
        None,
    ),
    p(
        TASKBAR_SIZE,
        "Taskbar size",
        "taskbar small buttons icons size",
        Kind::Choice(&[(0, "Default"), (1, "Small")]),
        0,
        CONTROLS,
        3,
        1,
        Page::Taskbar,
        None,
    ),
    Property {
        default: 1,
        ..flag(
            CONTEXT_INLINE_ACTIONS,
            "Arrange primary actions inline",
            "context menu cut copy paste rename share delete icons",
            CONTROLS,
            4,
            Page::ContextMenu,
            None,
        )
    },
    p(
        CONTEXT_PROPERTIES_PLACEMENT,
        "Properties placement",
        "context menu properties top bottom file explorer",
        Kind::Choice(&[(0, "Top of the menu"), (1, "Bottom of the menu")]),
        1,
        CONTROLS,
        5,
        1,
        Page::ContextMenu,
        None,
    ),
    flag(
        DESKTOP_ICONS,
        "Desktop icon settings",
        "show desktop icons this pc recycle bin",
        CONTROLS,
        6,
        Page::Themes,
        Some("desktop-icons"),
    ),
    p(
        START_LAYOUT,
        "Start layout",
        "start more pins more recommendations",
        Kind::Choice(&[
            (0, "Default"),
            (1, "More pins"),
            (2, "More recommendations"),
        ]),
        0,
        CONTROLS,
        7,
        2,
        Page::Start,
        None,
    ),
    flag(
        REDUCE_TRANSPARENCY,
        "Transparency effects off",
        "transparency effects acrylic mica translucent windows",
        CONTROLS,
        9,
        Page::Colors,
        Some("transparency"),
    ),
    flag(
        CONTEXT_CLASSIC,
        "Show the classic context menu",
        "context menu classic full menu show more options",
        CONTROLS,
        10,
        Page::ContextMenu,
        None,
    ),
    Property {
        depends: &[Dependency::ForcedUnless {
            key: TASKBAR_POSITION,
            value: 0,
            forced: 1,
        }],
        ..p(
            TASKBAR_SEARCH,
            "Search",
            "taskbar search box search icon only",
            Kind::Choice(&[(0, "Search box"), (1, "Search icon only")]),
            0,
            CONTROLS,
            11,
            1,
            Page::Taskbar,
            None,
        )
    },
    flag(
        REDUCE_MOTION,
        "Animation effects off",
        "animation effects reduce motion visual effects",
        CONTROLS,
        12,
        Page::VisualEffects,
        Some("visual-performance"),
    ),
    p(
        START_SIZE,
        "Start size",
        "start menu size small large automatic",
        Kind::Choice(&[(0, "Automatic"), (1, "Small"), (2, "Large")]),
        0,
        CONTROLS,
        13,
        2,
        Page::Start,
        None,
    ),
    flag(
        START_HIDE_PINNED,
        "Hide Pinned in Start",
        "start pinned apps section",
        CONTROLS,
        15,
        Page::Start,
        None,
    ),
    flag(
        START_HIDE_RECENT,
        "Hide Recent in Start",
        "start recent recently installed apps files section",
        CONTROLS,
        16,
        Page::Start,
        None,
    ),
    flag(
        START_HIDE_ALL,
        "Hide All in Start",
        "start all apps section",
        CONTROLS,
        17,
        Page::Start,
        None,
    ),
    flag(
        START_HIDE_ACCOUNT,
        "Hide account name and picture in Start",
        "start account name picture user",
        CONTROLS,
        18,
        Page::Start,
        None,
    ),
    Property {
        depends: &[Dependency::ActiveWhen {
            key: START_HIDE_RECENT,
            value: 0,
        }],
        ..flag(
            START_HIDE_RECENT_FILES,
            "Hide recommended files in Recent",
            "start recent recommended files",
            CONTROLS,
            19,
            Page::Start,
            None,
        )
    },
    flag(
        CONTEXT_HIDE_FILE_ACTIONS,
        "Hide file actions",
        "context menu file actions send to open with",
        CONTROLS,
        20,
        Page::ContextMenu,
        None,
    ),
    flag(
        CONTEXT_HIDE_MORE_OPTIONS,
        "Hide Show more options",
        "context menu show more options classic entry",
        CONTROLS,
        21,
        Page::ContextMenu,
        None,
    ),
    Property {
        depends: &[Dependency::ActiveWhen {
            key: CONTRAST,
            value: 1,
        }],
        ..p(
            CONTRAST_THEME,
            "Contrast theme",
            "contrast themes high contrast aquatic desert dusk night sky",
            Kind::Choice(&[(0, "Night sky"), (1, "Aquatic"), (2, "Desert"), (3, "Dusk")]),
            0,
            CONTROLS,
            22,
            2,
            Page::ContrastThemes,
            None,
        )
    },
    flag(
        CONTRAST,
        "Contrast themes on",
        "contrast themes high contrast accessibility",
        CONTROLS,
        24,
        Page::ContrastThemes,
        None,
    ),
    p(
        TEXT_SIZE,
        "Text size",
        "text size font larger accessibility percent above 100",
        Kind::Integer {
            min: 0,
            max: Controls::TEXT_STEP_MAX,
            step: 1,
            special: None,
        },
        0,
        CONTROLS,
        25,
        7,
        Page::TextSize,
        None,
    ),
    // Setting 4: `desktop::ShellLayout` flags (bits set = unpinned).
    p(
        TASKBAR_UNPINNED,
        "Apps not pinned to the taskbar",
        "taskbar pinned apps pin unpin",
        Kind::Bitset(APPS),
        0,
        SHELL_LAYOUT,
        0,
        6,
        Page::Taskbar,
        None,
    ),
    p(
        START_UNPINNED,
        "Apps not pinned to Start",
        "start pinned apps pin unpin",
        Kind::Bitset(APPS),
        0,
        SHELL_LAYOUT,
        6,
        6,
        Page::Start,
        None,
    ),
    p(
        START_ALL_VIEW,
        "All apps view",
        "start all apps category grid list view",
        Kind::Choice(&[(0, "Category"), (1, "Grid"), (2, "List")]),
        0,
        SHELL_LAYOUT,
        12,
        2,
        Page::Start,
        None,
    ),
    flag(
        START_ALL_SORTED,
        "Sort All apps by name",
        "start all apps sort",
        SHELL_LAYOUT,
        14,
        Page::Start,
        None,
    ),
    // Setting 5: `desktop::ShellLayout` order (Lehmer ranks).
    Property {
        visibility: Visibility::Internal,
        ..p(
            START_PIN_ORDER,
            "Start pin order",
            "",
            Kind::Integer {
                min: 0,
                max: 719,
                step: 1,
                special: None,
            },
            0,
            SHELL_ORDER,
            0,
            10,
            Page::Start,
            None,
        )
    },
    Property {
        visibility: Visibility::Internal,
        ..p(
            TASKBAR_PIN_ORDER,
            "Taskbar pin order",
            "",
            Kind::Integer {
                min: 0,
                max: 719,
                step: 1,
                special: None,
            },
            0,
            SHELL_ORDER,
            10,
            10,
            Page::Taskbar,
            None,
        )
    },
    // Setting 6: `door_keyboard_layouts::Preferences`.
    p(
        INPUT_METHODS,
        "Keyboards and input methods",
        "keyboard layout input method language typing ime override",
        Kind::Encoded,
        1,
        INPUT,
        0,
        27,
        Page::Typing,
        None,
    ),
    flag(
        INPUT_PER_WINDOW,
        "Use a different input method for each app window",
        "keyboard input method per window app typing",
        INPUT,
        27,
        Page::Typing,
        None,
    ),
    // Settings 7 and 8: catalog ids.
    p(
        TIME_ZONE,
        "Time zone",
        "time zone clock utc date",
        Kind::Catalog(&ZONES),
        door_locale::tz::DEFAULT_ID as u32,
        ZONE,
        0,
        32,
        Page::DateTime,
        Some("time-zone"),
    ),
    p(
        REGIONAL_FORMAT,
        "Regional format",
        "region regional format date time number locale language",
        Kind::Catalog(&REGIONS),
        door_locale::region::DEFAULT_ID as u32,
        REGION,
        0,
        32,
        Page::LanguageRegion,
        Some("regional-format"),
    ),
];

/// The built-in schema.
pub static BUILTIN: Schema = Schema {
    words: &WORDS,
    properties: &PROPERTIES,
};

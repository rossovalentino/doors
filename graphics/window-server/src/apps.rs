//! The installed desktop applications and their real shell metadata. Every
//! entry is a bundled, launchable program; the shell never lists phantom apps.
use door_graphics_core::{Canvas, Rect, Scale};
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum App {
    Files,
    Notes,
    Settings,
    Terminal,
    Preview,
    Monitor,
}
/// Start "All" categories. As in Windows 11, a category is only formed when
/// at least three installed apps belong to it; the rest are listed in Other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Creativity,
    DeveloperTools,
    Productivity,
    UtilitiesTools,
    Other,
}
impl Category {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Creativity => "Creativity",
            Self::DeveloperTools => "Developer Tools",
            Self::Productivity => "Productivity",
            Self::UtilitiesTools => "Utilities & tools",
            Self::Other => "Other",
        }
    }
}
pub const COUNT: usize = door_service_protocol::application::DESKTOP_APPS;
// Launch requests are one bit per app then per tool (`pending_launches`,
// u32); blocked tools are one bit per tool (u16).
const _: () = assert!(COUNT + door_service_protocol::application::ACCESSORIES <= 32);
const _: () = assert!(door_service_protocol::application::ACCESSORIES <= 16);
impl App {
    pub const ALL: [Self; COUNT] = [
        Self::Files,
        Self::Notes,
        Self::Settings,
        Self::Terminal,
        Self::Preview,
        Self::Monitor,
    ];
    pub fn all() -> [Self; COUNT] {
        Self::ALL
    }
    pub const fn index(self) -> usize {
        match self {
            Self::Files => 0,
            Self::Notes => 1,
            Self::Settings => 2,
            Self::Terminal => 3,
            Self::Preview => 4,
            Self::Monitor => 5,
        }
    }
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
    /// Init's authenticated desktop launch command.
    pub const fn command(self) -> u8 {
        door_service_protocol::application::DESKTOP_COMMANDS[self.index()]
    }
    pub fn from_command(command: u8) -> Option<Self> {
        door_service_protocol::application::desktop_index(command).and_then(Self::from_index)
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Files => "File Explorer",
            Self::Notes => "Notes",
            Self::Settings => "Settings",
            Self::Terminal => "Terminal",
            Self::Preview => "Preview",
            // The Monitor app presents itself as Windows' Task Manager.
            Self::Monitor => "Task Manager",
        }
    }
    /// The natural category before the three-app threshold is applied.
    pub const fn category(self) -> Category {
        match self {
            Self::Files | Self::Settings | Self::Monitor => Category::UtilitiesTools,
            Self::Notes => Category::Productivity,
            Self::Terminal => Category::DeveloperTools,
            Self::Preview => Category::Creativity,
        }
    }
    /// Extra search words (lower case) a user may type for this app.
    pub const fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::Files => &["explorer", "files", "folders", "documents", "this pc"],
            Self::Notes => &["notepad", "text", "editor", "notes"],
            Self::Settings => &["control panel", "preferences", "options", "configuration"],
            Self::Terminal => &["command prompt", "cmd", "shell", "console", "powershell"],
            Self::Preview => &["photos", "images", "pictures", "viewer", "pdf"],
            Self::Monitor => &[
                "taskmgr",
                "monitor",
                "processes",
                "performance",
                "cpu",
                "memory",
            ],
        }
    }
    /// Shift+click / "new window" support (multi-window clients).
    pub const fn description(self) -> &'static str {
        match self {
            Self::Files => "App",
            _ => "App",
        }
    }
}
/// Bundled tools: the Door UI component gallery (an engineering fixture),
/// Live captions (`door-assist`) and the inbox apps (`door-inbox`:
/// Calculator, Clock, Paint, Snipping Tool, Weather and the accessibility
/// apps), with taskbar buttons like any app.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tool {
    Gallery,
    Captions,
    Calculator,
    Clock,
    Paint,
    SnippingTool,
    Weather,
    Magnifier,
    Narrator,
    OnScreenKeyboard,
    VoiceAccess,
}
impl Tool {
    pub const ALL: [Self; door_service_protocol::application::ACCESSORIES] = [
        Self::Gallery,
        Self::Captions,
        Self::Calculator,
        Self::Clock,
        Self::Paint,
        Self::SnippingTool,
        Self::Weather,
        Self::Magnifier,
        Self::Narrator,
        Self::OnScreenKeyboard,
        Self::VoiceAccess,
    ];
    pub const fn index(self) -> usize {
        match self {
            Self::Gallery => 0,
            Self::Captions => 1,
            Self::Calculator => 2,
            Self::Clock => 3,
            Self::Paint => 4,
            Self::SnippingTool => 5,
            Self::Weather => 6,
            Self::Magnifier => 7,
            Self::Narrator => 8,
            Self::OnScreenKeyboard => 9,
            Self::VoiceAccess => 10,
        }
    }
    /// The Start category (as Windows 11 26H2 groups them).
    pub const fn category(self) -> Category {
        match self {
            Self::Paint => Category::Creativity,
            Self::Gallery => Category::DeveloperTools,
            Self::Weather => Category::Other,
            _ => Category::UtilitiesTools,
        }
    }
    /// Windows 11 files these under Start's Accessibility folder.
    pub const fn accessibility(self) -> bool {
        matches!(
            self,
            Self::Captions
                | Self::Magnifier
                | Self::Narrator
                | Self::OnScreenKeyboard
                | Self::VoiceAccess
        )
    }
    /// Init's authenticated accessory launch command.
    pub const fn command(self) -> u8 {
        door_service_protocol::application::ACCESSORY_COMMANDS[self.index()]
    }
    /// The kernel-authenticated manifest identity the tool's process runs as.
    pub const fn manifest(self) -> u64 {
        match self {
            Self::Gallery => door_abi::platform::manifest::GALLERY,
            Self::Captions => door_abi::platform::manifest::ASSIST,
            Self::Calculator => door_abi::platform::manifest::CALCULATOR,
            Self::Clock => door_abi::platform::manifest::CLOCK,
            Self::Paint => door_abi::platform::manifest::PAINT,
            Self::SnippingTool => door_abi::platform::manifest::SNIPPING_TOOL,
            Self::Weather => door_abi::platform::manifest::WEATHER,
            Self::Magnifier => door_abi::platform::manifest::MAGNIFIER,
            Self::Narrator => door_abi::platform::manifest::NARRATOR,
            Self::OnScreenKeyboard => door_abi::platform::manifest::ON_SCREEN_KEYBOARD,
            Self::VoiceAccess => door_abi::platform::manifest::VOICE_ACCESS,
        }
    }
    pub fn from_manifest(manifest: u64) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.manifest() == manifest)
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Gallery => "Doors UI Gallery",
            Self::Captions => "Live captions",
            Self::Calculator => "Calculator",
            Self::Clock => "Clock",
            Self::Paint => "Paint",
            Self::SnippingTool => "Snipping Tool",
            Self::Weather => "Weather",
            Self::Magnifier => "Magnifier",
            Self::Narrator => "Narrator",
            Self::OnScreenKeyboard => "On-Screen Keyboard",
            Self::VoiceAccess => "Voice access",
        }
    }
    pub const fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::Gallery => &["controls", "components", "winui", "gallery", "toolkit"],
            Self::Captions => &["captions", "subtitles", "speech to text", "transcribe"],
            Self::Calculator => &["calc", "math", "arithmetic", "numbers", "add", "divide"],
            Self::Clock => &[
                "alarms",
                "alarm",
                "timer",
                "stopwatch",
                "world clock",
                "focus",
            ],
            Self::Paint => &[
                "mspaint",
                "paint 3d",
                "draw",
                "drawing",
                "image editor",
                "3d",
            ],
            Self::SnippingTool => &["screenshot", "snip", "screen capture", "print screen"],
            Self::Weather => &["forecast", "temperature", "rain", "msn weather"],
            Self::Magnifier => &["magnify", "zoom", "lens", "accessibility"],
            Self::Narrator => &["screen reader", "read aloud", "speech", "accessibility"],
            Self::OnScreenKeyboard => &["osk", "keyboard", "virtual keyboard", "accessibility"],
            Self::VoiceAccess => &["voice control", "dictation", "speech", "accessibility"],
        }
    }
}
/// Desktop "Medium icons" grid (Windows 11: 48 px icons, one column), in
/// logical pixels relative to the work-area origin. Painting, hit testing,
/// hover and the guest scenarios all derive positions from here.
pub mod desktop_grid {
    use core::ops::Range;
    /// Desktop slots: the Recycle Bin first (as Windows arranges a new
    /// desktop), then the apps in `desktop_order`.
    pub const SLOTS: usize = super::COUNT + 1;
    pub const BIN_SLOT: usize = 0;
    /// The slot of the app at `index` of the desktop order.
    pub const fn app_slot(index: usize) -> usize {
        index + 1
    }
    pub const ICON: u32 = 48;
    pub const PITCH: i32 = 88;
    /// Icon top-left for slot `index`.
    pub const fn icon(index: usize) -> (i32, i32) {
        (18, 12 + index as i32 * PITCH)
    }
    /// Label horizontal center and line-box top (12 px text).
    pub const fn label(index: usize) -> (i32, i32) {
        (42, 62 + index as i32 * PITCH)
    }
    /// Selection/hover plate (x, y, width, height).
    pub const fn plate(index: usize) -> (i32, i32, u32, u32) {
        (2, 8 + index as i32 * PITCH, 80, 76)
    }
    /// Vertical hit band; the horizontal band is `HIT_X`.
    pub const fn band(index: usize) -> Range<i32> {
        8 + index as i32 * PITCH..84 + index as i32 * PITCH
    }
    pub const HIT_X: Range<i32> = 2..82;
}
/// All apps sorted by display name, as in the Start List and Grid views.
pub fn alphabetical() -> [App; COUNT] {
    let mut apps = App::ALL;
    apps.sort_by(|a, b| {
        a.label()
            .bytes()
            .map(|c| c.to_ascii_lowercase())
            .cmp(b.label().bytes().map(|c| c.to_ascii_lowercase()))
    });
    apps
}
/// One program in Start's All list: a desktop app or an installed tool
/// (the inbox apps and Live captions; the component gallery is search-only).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Entry {
    App(App),
    Tool(Tool),
}
impl Entry {
    pub const fn label(self) -> &'static str {
        match self {
            Self::App(a) => a.label(),
            Self::Tool(t) => t.label(),
        }
    }
    pub const fn category(self) -> Category {
        match self {
            Self::App(a) => a.category(),
            Self::Tool(t) => t.category(),
        }
    }
    /// Stable order: the apps, then the tools.
    pub const fn order(self) -> usize {
        match self {
            Self::App(a) => a.index(),
            Self::Tool(t) => COUNT + t.index(),
        }
    }
    pub fn app(self) -> Option<App> {
        match self {
            Self::App(a) => Some(a),
            Self::Tool(_) => None,
        }
    }
}
/// Every program Start's All list shows, in the stable order.
pub fn entries() -> alloc::vec::Vec<Entry> {
    App::ALL
        .into_iter()
        .map(Entry::App)
        .chain(
            Tool::ALL
                .into_iter()
                .filter(|t| *t != Tool::Gallery)
                .map(Entry::Tool),
        )
        .collect()
}
/// The All list by name (Grid and List views).
pub fn entries_alphabetical() -> alloc::vec::Vec<Entry> {
    let mut v = entries();
    v.sort_by(|a, b| {
        a.label()
            .bytes()
            .map(|c| c.to_ascii_lowercase())
            .cmp(b.label().bytes().map(|c| c.to_ascii_lowercase()))
    });
    v
}
/// Draw an entry's icon like `draw_icon`.
pub fn draw_entry_icon(canvas: &mut Canvas<'_>, x: i32, y: i32, entry: Entry, size: u32) {
    match entry {
        Entry::App(a) => draw_icon(canvas, x, y, a, size),
        Entry::Tool(t) => draw_tool_icon(canvas, x, y, t, size),
    }
}
/// Categories that actually form (at least three programs) followed by
/// Other. Each lists its programs in the stable order.
pub fn categories() -> alloc::vec::Vec<(Category, alloc::vec::Vec<Entry>)> {
    use alloc::vec::Vec;
    let mut formed: Vec<(Category, Vec<Entry>)> = Vec::new();
    let mut other = Vec::new();
    for category in [
        Category::Creativity,
        Category::DeveloperTools,
        Category::Productivity,
        Category::UtilitiesTools,
    ] {
        let apps: Vec<Entry> = entries()
            .into_iter()
            .filter(|e| e.category() == category)
            .collect();
        if apps.len() >= 3 {
            formed.push((category, apps));
        } else {
            other.extend(apps);
        }
    }
    other.extend(
        entries()
            .into_iter()
            .filter(|e| e.category() == Category::Other),
    );
    other.sort_by_key(|e| e.order());
    if !other.is_empty() {
        formed.push((Category::Other, other));
    }
    formed
}
extern crate alloc;
macro_rules! windows_icon {
    ($name:literal, $physical:expr) => {
        match $physical {
            16 => Some(
                &include_bytes!(concat!("../../../assets/icons/windows/", $name, ".16.bgra"))[..],
            ),
            24 => Some(
                &include_bytes!(concat!("../../../assets/icons/windows/", $name, ".24.bgra"))[..],
            ),
            28 => Some(
                &include_bytes!(concat!("../../../assets/icons/windows/", $name, ".28.bgra"))[..],
            ),
            32 => Some(
                &include_bytes!(concat!("../../../assets/icons/windows/", $name, ".32.bgra"))[..],
            ),
            40 => Some(
                &include_bytes!(concat!("../../../assets/icons/windows/", $name, ".40.bgra"))[..],
            ),
            48 => Some(
                &include_bytes!(concat!("../../../assets/icons/windows/", $name, ".48.bgra"))[..],
            ),
            56 => Some(
                &include_bytes!(concat!("../../../assets/icons/windows/", $name, ".56.bgra"))[..],
            ),
            64 => Some(
                &include_bytes!(concat!("../../../assets/icons/windows/", $name, ".64.bgra"))[..],
            ),
            72 => Some(
                &include_bytes!(concat!("../../../assets/icons/windows/", $name, ".72.bgra"))[..],
            ),
            80 => Some(
                &include_bytes!(concat!("../../../assets/icons/windows/", $name, ".80.bgra"))[..],
            ),
            96 => Some(
                &include_bytes!(concat!("../../../assets/icons/windows/", $name, ".96.bgra"))[..],
            ),
            120 => Some(
                &include_bytes!(concat!(
                    "../../../assets/icons/windows/",
                    $name,
                    ".120.bgra"
                ))[..],
            ),
            _ => None,
        }
    };
}
macro_rules! branding_icon {
    ($name:literal, $physical:expr) => {
        match $physical {
            16 => {
                Some(&include_bytes!(concat!("../../../assets/branding/", $name, ".16.bgra"))[..])
            }
            20 => {
                Some(&include_bytes!(concat!("../../../assets/branding/", $name, ".20.bgra"))[..])
            }
            24 => {
                Some(&include_bytes!(concat!("../../../assets/branding/", $name, ".24.bgra"))[..])
            }
            28 => {
                Some(&include_bytes!(concat!("../../../assets/branding/", $name, ".28.bgra"))[..])
            }
            32 => {
                Some(&include_bytes!(concat!("../../../assets/branding/", $name, ".32.bgra"))[..])
            }
            40 => {
                Some(&include_bytes!(concat!("../../../assets/branding/", $name, ".40.bgra"))[..])
            }
            48 => {
                Some(&include_bytes!(concat!("../../../assets/branding/", $name, ".48.bgra"))[..])
            }
            56 => {
                Some(&include_bytes!(concat!("../../../assets/branding/", $name, ".56.bgra"))[..])
            }
            60 => {
                Some(&include_bytes!(concat!("../../../assets/branding/", $name, ".60.bgra"))[..])
            }
            64 => {
                Some(&include_bytes!(concat!("../../../assets/branding/", $name, ".64.bgra"))[..])
            }
            72 => {
                Some(&include_bytes!(concat!("../../../assets/branding/", $name, ".72.bgra"))[..])
            }
            80 => {
                Some(&include_bytes!(concat!("../../../assets/branding/", $name, ".80.bgra"))[..])
            }
            84 => {
                Some(&include_bytes!(concat!("../../../assets/branding/", $name, ".84.bgra"))[..])
            }
            96 => {
                Some(&include_bytes!(concat!("../../../assets/branding/", $name, ".96.bgra"))[..])
            }
            120 => {
                Some(&include_bytes!(concat!("../../../assets/branding/", $name, ".120.bgra"))[..])
            }
            128 => {
                Some(&include_bytes!(concat!("../../../assets/branding/", $name, ".128.bgra"))[..])
            }
            160 => {
                Some(&include_bytes!(concat!("../../../assets/branding/", $name, ".160.bgra"))[..])
            }
            192 => {
                Some(&include_bytes!(concat!("../../../assets/branding/", $name, ".192.bgra"))[..])
            }
            _ => None,
        }
    };
}
/// A Terminal profile opened directly (search, Run, Start): Terminal on a
/// tab of that shell, as Windows 11 opens cmd.exe and PowerShell in
/// Windows Terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shell {
    Doorsh,
    CommandPrompt,
    PowerShell,
    Dsl,
}
impl Shell {
    pub const ALL: [Self; 4] = [
        Self::Doorsh,
        Self::CommandPrompt,
        Self::PowerShell,
        Self::Dsl,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Doorsh => "Doorsh",
            Self::CommandPrompt => "Command Prompt",
            Self::PowerShell => "PowerShell",
            Self::Dsl => "DSL",
        }
    }
    /// Search keywords: the Windows names a user may type for each.
    pub fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::Doorsh => &["doors shell", "dsh", "shell", "terminal", "console"],
            Self::CommandPrompt => &[
                "cmd",
                "cmd.exe",
                "command prompt",
                "dos",
                "batch",
                "console",
            ],
            Self::PowerShell => &[
                "powershell",
                "pwsh",
                "windows powershell",
                "ps1",
                "powershell ise",
            ],
            Self::Dsl => &[
                "dsl",
                "doors subsystem for linux",
                "linux",
                "wsl",
                "bash",
                "ubuntu",
                "debian",
                "shell",
            ],
        }
    }
    /// The document Terminal is asked to open (`Terminal::open_path`).
    pub fn payload(self) -> &'static [u8] {
        match self {
            Self::Doorsh => b"doorsh",
            Self::CommandPrompt => b"cmd",
            Self::PowerShell => b"powershell",
            Self::Dsl => b"dsl",
        }
    }
    pub fn index(self) -> usize {
        self as usize
    }
    /// The profile's own icon (Doorsh project artwork, Terminal's cmd and
    /// PowerShell marks, the Windows 11 WSL logo for DSL).
    pub fn icon_pixels(self, physical: u32) -> Option<&'static [u8]> {
        match self {
            Self::Doorsh => branding_icon!("doorsh", physical),
            Self::CommandPrompt => windows_icon!("terminal-cmd", physical),
            Self::PowerShell => windows_icon!("terminal-powershell", physical),
            Self::Dsl => windows_icon!("terminal-wsl", physical),
        }
    }
}
pub fn draw_shell_icon(canvas: &mut Canvas<'_>, x: i32, y: i32, shell: Shell, size: u32) {
    blit_native(canvas, x, y, size, |physical| shell.icon_pixels(physical));
}
/// Native premultiplied BGRA frame for `app` at a physical pixel size.
pub fn icon_pixels(app: App, physical: u32) -> Option<&'static [u8]> {
    match app {
        App::Files => windows_icon!("explorer", physical),
        App::Notes => windows_icon!("notepad", physical),
        App::Settings => windows_icon!("settings", physical),
        App::Terminal => windows_icon!("terminal-current", physical),
        App::Preview => branding_icon!("preview", physical),
        // Task Manager's own icon (installed reference, Taskmgr.exe.mun group
        // 30651; `taskmgr.source.json`).
        App::Monitor => windows_icon!("taskmgr", physical),
    }
}
/// File-type icon for a search result (Windows file icons, 16 and 32 px).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileIcon {
    Folder,
    Text,
    Image,
    Code,
    Pdf,
    Archive,
    /// A program without its own icon (.exe/.com), as Windows 11 draws it.
    Application,
    /// .bat/.cmd.
    Batch,
    /// .msi/.msp.
    Installer,
    Generic,
}
impl FileIcon {
    /// Icon and Windows type label for a name.
    pub fn for_name(name: &str, folder: bool) -> Self {
        if folder {
            return Self::Folder;
        }
        let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
        let is = |list: &[&str]| list.iter().any(|e| e.eq_ignore_ascii_case(ext));
        if is(&["exe", "com", "scr"]) {
            Self::Application
        } else if is(&["bat", "cmd"]) {
            Self::Batch
        } else if is(&["msi", "msp"]) {
            Self::Installer
        } else if is(&[
            "txt", "md", "log", "ini", "cfg", "conf", "json", "csv", "xml",
        ]) {
            Self::Text
        } else if is(&[
            "png", "jpg", "jpeg", "bmp", "gif", "qoi", "webp", "ico", "tif", "tiff",
        ]) {
            Self::Image
        } else if is(&[
            "rs", "c", "h", "cpp", "hpp", "py", "js", "ts", "toml", "html", "css", "sh", "ps1",
        ]) {
            Self::Code
        } else if is(&["pdf"]) {
            Self::Pdf
        } else if is(&["zip", "7z", "tar", "gz", "rar", "cab"]) {
            Self::Archive
        } else {
            Self::Generic
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Folder => "File folder",
            Self::Text => "Text Document",
            Self::Image => "Image",
            Self::Code => "Source file",
            Self::Pdf => "PDF Document",
            Self::Archive => "Compressed (zipped) Folder",
            Self::Application => "Application",
            Self::Batch => "Doors Batch File",
            Self::Installer => "Doors Installer Package",
            Self::Generic => "File",
        }
    }
}
macro_rules! file_icon {
    ($name:literal, $physical:expr) => {
        match $physical {
            16 => Some(
                &include_bytes!(concat!("../../../assets/icons/windows/", $name, ".16.bgra"))[..],
            ),
            32 => Some(
                &include_bytes!(concat!("../../../assets/icons/windows/", $name, ".32.bgra"))[..],
            ),
            48 => Some(
                &include_bytes!(concat!("../../../assets/icons/windows/", $name, ".48.bgra"))[..],
            ),
            64 => Some(
                &include_bytes!(concat!("../../../assets/icons/windows/", $name, ".64.bgra"))[..],
            ),
            96 => Some(
                &include_bytes!(concat!("../../../assets/icons/windows/", $name, ".96.bgra"))[..],
            ),
            _ => None,
        }
    };
}
pub fn file_icon_pixels(icon: FileIcon, physical: u32) -> Option<&'static [u8]> {
    match icon {
        FileIcon::Folder => file_icon!("folder", physical),
        FileIcon::Text => file_icon!("text-file", physical),
        FileIcon::Image => file_icon!("file-photo", physical),
        FileIcon::Code => file_icon!("file-code", physical),
        FileIcon::Pdf => file_icon!("file-pdf", physical),
        FileIcon::Archive => file_icon!("file-zip", physical),
        FileIcon::Application => file_icon!("file-application", physical),
        FileIcon::Batch => file_icon!("file-batch", physical),
        FileIcon::Installer => file_icon!("file-installer", physical),
        FileIcon::Generic => file_icon!("file-generic", physical),
    }
}
/// Draw a 16 or 32 px file icon (other sizes fall back to the nearest).
pub fn draw_file_icon(canvas: &mut Canvas<'_>, x: i32, y: i32, icon: FileIcon, size: u32) {
    let size = if size >= 32 { 32 } else { 16 };
    blit_native(canvas, x, y, size, |physical| {
        file_icon_pixels(icon, physical)
    });
}
/// Blit the native frame for logical `size` at logical (`x`, `y`), placed
/// at the snapped physical origin with its exact physical size. Integer
/// scales use native frames only; a fractional scale without a native frame
/// area-filters the next whole density's frame down (never stretches up).
fn blit_native(
    canvas: &mut Canvas<'_>,
    x: i32,
    y: i32,
    size: u32,
    frame: impl Fn(u32) -> Option<&'static [u8]>,
) {
    let scale = canvas.scale();
    let n = scale.len(size);
    let Ok(rect) = Rect::new(scale.px(x), scale.px(y), n, n) else {
        return;
    };
    if let Some(pixels) = frame(n) {
        let _ = canvas.blit_bgra_physical(rect, pixels);
    } else if scale.factor().is_none() {
        let source = size * scale.asset_factor();
        if let Some(pixels) = frame(source) {
            let pixels =
                door_graphics_core::area_resample_bgra(pixels, source as usize, n as usize);
            let _ = canvas.blit_bgra_physical(rect, &pixels);
        }
    }
}
/// A decoded square icon (premultiplied BGRA) at logical `size`: area
/// filtered down to the physical size, or repeated up when smaller.
pub fn draw_image_icon(
    canvas: &mut Canvas<'_>,
    x: i32,
    y: i32,
    size: u32,
    image: &door_pe_icon::Image,
) {
    let scale = canvas.scale();
    let n = scale.len(size);
    let Ok(rect) = Rect::new(scale.px(x), scale.px(y), n, n) else {
        return;
    };
    let source = image.width as usize;
    if image.width != image.height || image.bgra.len() != source * source * 4 {
        return;
    }
    let pixels = if source >= n as usize {
        door_graphics_core::area_resample_bgra(&image.bgra, source, n as usize)
    } else {
        let n = n as usize;
        let mut up = alloc::vec![0u8; n * n * 4];
        for y in 0..n {
            for x in 0..n {
                let from = ((y * source / n) * source + x * source / n) * 4;
                up[(y * n + x) * 4..][..4].copy_from_slice(&image.bgra[from..from + 4]);
            }
        }
        up
    };
    let _ = canvas.blit_bgra_physical(rect, &pixels);
}
/// The Doors mark used by the Start button.
pub fn logo_pixels(physical: u32) -> Option<&'static [u8]> {
    branding_icon!("door", physical)
}
/// Largest native icon size not above `size` that exists at this density.
pub fn icon_size(app: App, size: u32, scale: Scale) -> u32 {
    let mut s = size;
    while s > 8 {
        if icon_pixels(app, scale.len(s)).is_some() {
            return s;
        }
        s -= 1;
    }
    size
}
/// An inbox app's installed Windows icon (its package's unplated
/// taskbar/Start artwork, `calculator.source.json`); other tools have none.
pub fn tool_icon_pixels(tool: Tool, physical: u32) -> Option<&'static [u8]> {
    match tool {
        Tool::Calculator => windows_icon!("calculator", physical),
        Tool::Clock => windows_icon!("clock", physical),
        Tool::Paint => windows_icon!("paint", physical),
        Tool::SnippingTool => windows_icon!("snipping-tool", physical),
        Tool::Weather => windows_icon!("weather", physical),
        Tool::Captions => windows_icon!("live-captions", physical),
        Tool::Magnifier => windows_icon!("magnifier", physical),
        Tool::Narrator => windows_icon!("narrator", physical),
        Tool::OnScreenKeyboard => windows_icon!("on-screen-keyboard", physical),
        Tool::VoiceAccess => windows_icon!("voice-access", physical),
        Tool::Gallery => None,
    }
}
/// The Run dialog's icon: the installed Windows Run artwork (shell32.dll
/// group 25, `assets/icons/windows/run.source.json`).
pub fn run_icon_pixels(physical: u32) -> Option<&'static [u8]> {
    windows_icon!("run", physical)
}
pub fn draw_run_icon(canvas: &mut Canvas<'_>, x: i32, y: i32, size: u32) {
    blit_native(canvas, x, y, size, run_icon_pixels);
}
/// Draw an inbox app's icon like `draw_icon` (largest native frame not
/// above `size`, never stretched).
pub fn draw_tool_icon(canvas: &mut Canvas<'_>, x: i32, y: i32, tool: Tool, size: u32) {
    let scale = canvas.scale();
    let size = (9..=size)
        .rev()
        .find(|s| tool_icon_pixels(tool, scale.len(*s)).is_some())
        .unwrap_or(size);
    blit_native(canvas, x, y, size, |physical| {
        tool_icon_pixels(tool, physical)
    });
}
/// Draw the app's native frame at a logical position. Sizes never stretch a
/// bitmap; a missing density frame draws nothing rather than a blurred one.
pub fn draw_icon(canvas: &mut Canvas<'_>, x: i32, y: i32, app: App, size: u32) {
    let size = icon_size(app, size, canvas.scale());
    blit_native(canvas, x, y, size, |physical| icon_pixels(app, physical));
}
pub fn draw_logo(canvas: &mut Canvas<'_>, x: i32, y: i32, size: u32) {
    blit_native(canvas, x, y, size, logo_pixels);
}
/// `draw_logo` at `permille` of its size, centred in the same box (the
/// taskbar's press feedback).
pub fn draw_logo_scaled(canvas: &mut Canvas<'_>, x: i32, y: i32, size: u32, permille: u32) {
    blit_scaled(canvas, x, y, size, permille, logo_pixels);
}
/// `draw_icon` at `permille` of its size, centred in the same box.
pub fn draw_icon_scaled(
    canvas: &mut Canvas<'_>,
    x: i32,
    y: i32,
    app: App,
    size: u32,
    permille: u32,
) {
    let size = icon_size(app, size, canvas.scale());
    blit_scaled(canvas, x, y, size, permille, |physical| {
        icon_pixels(app, physical)
    });
}
/// The native frame for logical `size` area-filtered to `permille` of its
/// physical size and centred in its box; full size blits it unchanged.
fn blit_scaled(
    canvas: &mut Canvas<'_>,
    x: i32,
    y: i32,
    size: u32,
    permille: u32,
    frame: impl Fn(u32) -> Option<&'static [u8]>,
) {
    if permille >= 1000 {
        blit_native(canvas, x, y, size, frame);
        return;
    }
    let scale = canvas.scale();
    let n = scale.len(size);
    let source = match frame(n) {
        Some(pixels) => alloc::borrow::Cow::Borrowed(pixels),
        None => {
            let larger = size * scale.asset_factor();
            let Some(pixels) = frame(larger) else {
                return;
            };
            alloc::borrow::Cow::Owned(door_graphics_core::area_resample_bgra(
                pixels,
                larger as usize,
                n as usize,
            ))
        }
    };
    let m = ((n * permille + 500) / 1000).max(1);
    let pixels = door_graphics_core::area_resample_bgra(&source, n as usize, m as usize);
    let offset = (n - m) as i32 / 2;
    if let Ok(rect) = Rect::new(scale.px(x) + offset, scale.px(y) + offset, m, m) {
        let _ = canvas.blit_bgra_physical(rect, &pixels);
    }
}
/// The Recycle Bin's installed Windows icon (imageres.dll groups 55 empty
/// and 54 full, `assets/icons/windows/recycle-bin-*.source.json`).
pub fn recycle_bin_pixels(full: bool, physical: u32) -> Option<&'static [u8]> {
    if full {
        windows_icon!("recycle-bin-full", physical)
    } else {
        windows_icon!("recycle-bin-empty", physical)
    }
}
/// Draw the Recycle Bin icon (empty or full art) at a logical position.
pub fn draw_recycle_bin(canvas: &mut Canvas<'_>, x: i32, y: i32, full: bool, size: u32) {
    // Whole densities: the largest native size not above `size` (3x has no
    // 144 px frame). Fractional densities area-filter the next larger
    // whole-density frame (`blit_native`).
    let scale = canvas.scale();
    let size = if scale.factor().is_some() {
        (9..=size)
            .rev()
            .find(|s| recycle_bin_pixels(full, scale.len(*s)).is_some())
            .unwrap_or(size)
    } else {
        size
    };
    blit_native(canvas, x, y, size, |physical| {
        recycle_bin_pixels(full, physical)
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    /// The desktop Recycle Bin has native frames at the desktop icon size
    /// for every whole and common fractional density, distinct empty and
    /// full art, and the first slot before the apps.
    #[test]
    fn recycle_bin_has_empty_and_full_native_frames_in_the_first_slot() {
        for physical in [48, 72, 96, 120] {
            let empty = recycle_bin_pixels(false, physical).expect("empty frame");
            let full = recycle_bin_pixels(true, physical).expect("full frame");
            assert_eq!(empty.len(), (physical * physical * 4) as usize);
            assert_ne!(empty, full, "{physical}");
        }
        assert_eq!(desktop_grid::BIN_SLOT, 0);
        assert_eq!(desktop_grid::app_slot(0), 1);
        assert_eq!(desktop_grid::SLOTS, COUNT + 1);
    }
    #[test]
    fn categories_follow_the_three_app_rule_and_cover_every_app_once() {
        let groups = categories();
        let mut seen = alloc::vec::Vec::new();
        for (category, apps) in &groups {
            if *category != Category::Other {
                assert!(apps.len() >= 3);
            }
            seen.extend(apps.iter().copied());
        }
        seen.sort();
        let mut all = entries();
        all.sort();
        assert_eq!(seen, all);
        assert_eq!(groups.last().map(|g| g.0), Some(Category::Other));
        assert_eq!(groups[0].0, Category::UtilitiesTools);
    }
    #[test]
    fn every_app_has_native_frames_for_shell_sizes_at_each_density() {
        for app in App::ALL {
            assert_eq!(App::from_command(app.command()), Some(app));
            for scale in 1..=3 {
                for size in [16, 24, 32, 40] {
                    let pixels = icon_pixels(app, size * scale).expect("native frame");
                    assert_eq!(pixels.len(), (size * size * scale * scale * 4) as usize);
                }
            }
        }
        for scale in 1..=3 {
            assert!(logo_pixels(24 * scale).is_some());
            assert!(logo_pixels(16 * scale).is_some());
        }
        let names: alloc::vec::Vec<_> = alphabetical().iter().map(|a| a.label()).collect();
        assert_eq!(
            names,
            [
                "File Explorer",
                "Notes",
                "Preview",
                "Settings",
                "Task Manager",
                "Terminal"
            ]
        );
    }
}

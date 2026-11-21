//! Original Windows color artwork and Segoe Fluent/Microsoft Fluent masks.
//! Exact offline source identities live in the three asset manifests; the
//! existing `cargo xtask assets check` verifies native-density exports.
#[derive(Clone, Copy)]
pub enum Icon {
    Folder,
    FolderBack,
    FolderFront,
    FolderFilled,
    PC,
    AddCircle,
    Explorer,
    Document,
    TextFile,
    Add,
    Refresh,
    Up,
    Power,
    Save,
    Open,
    Notepad,
    Back,
    Forward,
    Desktop,
    Documents,
    Downloads,
    Music,
    Pictures,
    Videos,
    Computer,
    Drive,
    Cut,
    Copy,
    Paste,
    Rename,
    Share,
    Delete,
    Sort,
    View,
    Details,
    LargeIcons,
    More,
    Search,
    Chevron,
    /// "Pin to Quick access" action: Segoe Fluent E718, outline.
    Pin,
    /// The pinned state (navigation pane, Quick access tiles): Windows draws
    /// it filled, E840 Pinned + E841 PinnedFill, in an 11 px box.
    Pinned,
    Star,
    /// E734 FavoriteStar: "Add to Favorites" in the 26H2 context menu.
    StarOutline,
    /// "Run as administrator": the User Account Control shield (Segoe
    /// Fluent EA18).
    Shield,
    ChevronRight,
    Settings,
    Home,
    SidebarDocuments,
    SidebarDownloads,
    SidebarMusic,
    SidebarPictures,
    SidebarVideos,
    Network,
    /// NoneDrive (the generic cloud artwork, imageres group 1404).
    NoneDrive,
    /// Linux (the Windows Subsystem for Linux artwork, wsl.exe group 1).
    Linux,
    Pdf,
    Sheet,
    Word,
    CodeFile,
    VideoFile,
    AudioFile,
    ArchiveFile,
    /// Stock Windows 11 26H2 file-type icons, from the default association's
    /// DefaultIcon on the reference PC (`assets/icons/windows/file-*.source.json`):
    /// a program without its own icon (imageres 15).
    Application,
    /// .bat/.cmd (imageres 68).
    BatchFile,
    /// .dll/.sys/.drv/.ocx (imageres 67).
    SystemFile,
    /// .msi/.msp (msiexec.exe).
    InstallerFile,
    /// .ini/.inf/.cfg (Notepad's inifile).
    SettingsFile,
    /// .ps1/.psm1 (Notepad's ps1file).
    PowerShellFile,
    /// .reg (regedit.exe).
    RegistryFile,
    /// .url (url.dll).
    InternetShortcut,
    /// .ttf/.otf/.fon (fontext.dll).
    FontFile,
    /// .iso/.img (imageres 5205).
    DiscImage,
    /// .vhd/.vhdx (imageres 32).
    VirtualDisk,
    /// .cab (cabview.dll).
    CabinetFile,
    /// .cer/.crt/.pfx (cryptui.dll).
    CertificateFile,
    ModelFile,
    ModelFileDark,
    ImageFile,
    PresentationFile,
    NewTab,
    Tag,
    Close,
    Preview,
    Info,
    DetailsPane,
    NewFolder,
    Terminal,
    // Explorer monochrome location/command glyphs (Segoe Fluent Icons).
    GlyphDocument,
    GlyphFolder,
    GlyphMusic,
    GlyphVideo,
    Zip,
    Link,
    Properties,
    NewWindow,
    Mail,
    Convert,
    Check,
    /// Recycle Bin, empty and holding items (drawn glyphs,
    /// `door_workspace::file_operations::bin_mask`).
    RecycleBin,
    RecycleBinFull,
}
/// See `Icon::folder_layers`: where folder contents go, relative to the
/// icon's top-left, and where the front plate starts covering them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FolderLayers {
    /// (x, y, width, height) of the contents window.
    pub content: (i32, i32, u32, u32),
    pub front_top: i32,
}
impl Icon {
    /// Only the explicitly acquired association is specialized. Unregistered
    /// types keep the generic document until an actual association is supplied.
    pub fn for_file_name(name: &str) -> Self {
        use door_workspace::explorer_preview::Kind;
        let ext = name
            .rsplit_once('.')
            .map_or("", |(_, e)| e)
            .to_ascii_lowercase();
        let by_type = match ext.as_str() {
            "exe" | "com" | "scr" => Some(Self::Application),
            "bat" | "cmd" => Some(Self::BatchFile),
            "dll" | "sys" | "drv" | "ocx" | "cpl" => Some(Self::SystemFile),
            "msi" | "msp" => Some(Self::InstallerFile),
            "ini" | "inf" | "cfg" => Some(Self::SettingsFile),
            "ps1" | "psm1" | "psd1" | "ps1xml" | "psc1" => Some(Self::PowerShellFile),
            "reg" => Some(Self::RegistryFile),
            "url" => Some(Self::InternetShortcut),
            "ttf" | "otf" | "fon" | "ttc" => Some(Self::FontFile),
            "iso" | "img" => Some(Self::DiscImage),
            "vhd" | "vhdx" => Some(Self::VirtualDisk),
            "cab" => Some(Self::CabinetFile),
            "cer" | "crt" | "pfx" => Some(Self::CertificateFile),
            _ => None,
        };
        if let Some(icon) = by_type {
            return icon;
        }
        match Kind::of(name, false) {
            Kind::Pdf => Self::Pdf,
            Kind::Spreadsheet => Self::Sheet,
            Kind::Document => Self::Word,
            Kind::Code => Self::CodeFile,
            Kind::Video => Self::VideoFile,
            Kind::Audio => Self::AudioFile,
            Kind::Archive => Self::ArchiveFile,
            Kind::Model => Self::ModelFile,
            Kind::Image => Self::ImageFile,
            Kind::Presentation => Self::PresentationFile,
            Kind::Text => Self::TextFile,
            _ => Self::Document,
        }
    }
    /// Content layering of a `size` px folder (Medium icons and larger),
    /// drawn in this order: `Icon::FolderBack`, the contents (an image
    /// thumbnail with a 1-2 px white border, or child type icons) inside
    /// `content`, then `Icon::FolderFront`, which covers everything below
    /// `front_top` so the contents peek out above it, as in Windows 11. A
    /// folder holding files without a thumbnail is `Icon::FolderFilled`.
    /// Mirrors the grid of `tools/xtask/src/assets/folders.rs`.
    pub fn folder_layers(size: u32) -> FolderLayers {
        // round(k * size / 48) in integers (core has no `f32::round`).
        let at = |k: u32| ((2 * k * size + 48) / 96) as i32;
        let left = at(3).max(1);
        let right = size as i32 - left;
        let back_top = at(11).max(at(7) + 2);
        let front_top = at(22);
        let inset = at(2).max(1);
        let top = back_top + at(1).max(1);
        FolderLayers {
            content: (
                left + inset,
                top,
                (right - left - 2 * inset).max(0) as u32,
                (front_top + at(6) - top).max(0) as u32,
            ),
            front_top,
        }
    }
    pub fn themed(self, dark: bool) -> Self {
        if dark && matches!(self, Self::ModelFile) {
            Self::ModelFileDark
        } else {
            self
        }
    }
    pub fn color_scaled(self, size: u32, scale: u32) -> Option<&'static [u8]> {
        // `color!(name)` loads `assets/icons/windows/`; `color!(dir, name)`
        // another directory (Door's own Explorer folder art in `explorer`).
        macro_rules! color {
            ($name:literal) => {
                color!("windows", $name)
            };
            ($dir:literal, $name:literal) => {
                Some(match (size, scale) {
                    (16, 1) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/",
                        $dir,
                        "/",
                        $name,
                        ".16.qoi"
                    ))[..],
                    (16, 3) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/",
                        $dir,
                        "/",
                        $name,
                        ".48.qoi"
                    ))[..],
                    (16, 2) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/",
                        $dir,
                        "/",
                        $name,
                        ".32.qoi"
                    ))[..],
                    (20, 1) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/",
                        $dir,
                        "/",
                        $name,
                        ".20.qoi"
                    ))[..],
                    (20, 3) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/",
                        $dir,
                        "/",
                        $name,
                        ".60.qoi"
                    ))[..],
                    (20, 2) | (40, 1) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/",
                        $dir,
                        "/",
                        $name,
                        ".40.qoi"
                    ))[..],
                    (40, 3) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/",
                        $dir,
                        "/",
                        $name,
                        ".120.qoi"
                    ))[..],
                    (80, 1) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/",
                        $dir,
                        "/",
                        $name,
                        ".80.qoi"
                    ))[..],
                    (80, 2) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/",
                        $dir,
                        "/",
                        $name,
                        ".160.qoi"
                    ))[..],
                    (80, 3) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/",
                        $dir,
                        "/",
                        $name,
                        ".240.qoi"
                    ))[..],
                    (40, 2) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/",
                        $dir,
                        "/",
                        $name,
                        ".80.qoi"
                    ))[..],
                    _ => return None,
                })
            };
        }
        match self {
            Self::Pdf => color!("file-pdf"),
            Self::Sheet => color!("file-excel"),
            Self::Word => color!("file-word"),
            Self::CodeFile => color!("file-code"),
            // Media Player's file icon covers audio and video, Photos'
            // covers images, the compressed folder every archive.
            Self::VideoFile | Self::AudioFile => color!("file-media"),
            Self::ArchiveFile => color!("file-zip"),
            Self::Application => color!("file-application"),
            Self::BatchFile => color!("file-batch"),
            Self::SystemFile => color!("file-system"),
            Self::InstallerFile => color!("file-installer"),
            Self::SettingsFile => color!("file-settings"),
            Self::PowerShellFile => color!("file-powershell"),
            Self::RegistryFile => color!("file-registry"),
            Self::InternetShortcut => color!("file-internet-shortcut"),
            Self::FontFile => color!("file-font"),
            Self::DiscImage => color!("file-disc-image"),
            Self::VirtualDisk => color!("file-virtual-disk"),
            Self::CabinetFile => color!("file-cabinet"),
            Self::CertificateFile => color!("file-certificate"),
            Self::ModelFile => color!("file-model-light"),
            Self::ModelFileDark => color!("file-model"),
            Self::ImageFile => color!("file-photo"),
            Self::PresentationFile => color!("file-presentation"),
            Self::Document => color!("file-generic"),
            Self::Settings => color!("settings"),
            Self::Explorer => color!("explorer"),
            Self::Home => color!("explorer-home"),
            Self::TextFile => color!("text-file"),
            // Door-original folder art (`cargo xtask assets folders build`).
            Self::Folder => color!("explorer", "folder"),
            Self::FolderBack => color!("explorer", "folder-back"),
            Self::FolderFront => color!("explorer", "folder-front"),
            Self::FolderFilled => color!("explorer", "folder-files"),
            Self::Notepad => color!("notepad"),
            Self::Desktop => color!("explorer", "desktop"),
            Self::Documents => color!("explorer", "documents"),
            Self::Downloads => color!("explorer", "downloads"),
            Self::Music => color!("explorer", "music"),
            Self::Pictures => color!("explorer", "pictures"),
            Self::Videos => color!("explorer", "videos"),
            Self::Computer => color!("computer"),
            Self::Drive => color!("drive"),
            Self::SidebarDocuments => color!("explorer", "documents"),
            Self::SidebarMusic => color!("explorer", "music"),
            Self::SidebarPictures => color!("explorer", "pictures"),
            Self::SidebarVideos => color!("explorer", "videos"),
            Self::Network => color!("network"),
            Self::NoneDrive => color!("nonedrive"),
            Self::Linux => color!("linux"),
            // Windows' own Recycle Bin art (installed reference, imageres).
            Self::RecycleBin => color!("recycle-bin-empty"),
            Self::RecycleBinFull => color!("recycle-bin-full"),
            _ => None,
        }
    }
    pub fn mask_scaled(self, size: u32, scale: u32) -> &'static [u8] {
        macro_rules! windows {
            ($name:literal) => {
                match (size, scale) {
                    (12, 1) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/windows-shell/",
                        $name,
                        ".12.mask"
                    ))[..],
                    (12, 3) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/windows-shell/",
                        $name,
                        ".36.mask"
                    ))[..],
                    (12, 2) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/windows-shell/",
                        $name,
                        ".24.mask"
                    ))[..],
                    (16, 1) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/windows-shell/",
                        $name,
                        ".16.mask"
                    ))[..],
                    (16, 3) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/windows-shell/",
                        $name,
                        ".48.mask"
                    ))[..],
                    (16, 2) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/windows-shell/",
                        $name,
                        ".32.mask"
                    ))[..],
                    (20, 3) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/windows-shell/",
                        $name,
                        ".60.mask"
                    ))[..],
                    (20, 2) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/windows-shell/",
                        $name,
                        ".40.mask"
                    ))[..],
                    _ => &door_workspace::asset!(concat!(
                        "../../../assets/icons/windows-shell/",
                        $name,
                        ".20.mask"
                    ))[..],
                }
            };
        }
        macro_rules! mask {
            ($name:literal) => {
                match (size, scale) {
                    (12, 1) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/fluent/ic_fluent_",
                        $name,
                        "_20_regular.12.a8"
                    ))[..],
                    (12, 3) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/fluent/ic_fluent_",
                        $name,
                        "_20_regular.36.a8"
                    ))[..],
                    (12, 2) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/fluent/ic_fluent_",
                        $name,
                        "_20_regular.24.a8"
                    ))[..],
                    (16, 1) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/fluent/ic_fluent_",
                        $name,
                        "_20_regular.16.a8"
                    ))[..],
                    (16, 3) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/fluent/ic_fluent_",
                        $name,
                        "_20_regular.48.a8"
                    ))[..],
                    (16, 2) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/fluent/ic_fluent_",
                        $name,
                        "_20_regular.32.a8"
                    ))[..],
                    (20, 3) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/fluent/ic_fluent_",
                        $name,
                        "_20_regular.60.a8"
                    ))[..],
                    (20, 2) => &door_workspace::asset!(concat!(
                        "../../../assets/icons/fluent/ic_fluent_",
                        $name,
                        "_20_regular.40.a8"
                    ))[..],
                    _ => &door_workspace::asset!(concat!(
                        "../../../assets/icons/fluent/ic_fluent_",
                        $name,
                        "_20_regular.a8"
                    ))[..],
                }
            };
        }
        match self {
            Self::Folder
            | Self::FolderBack
            | Self::FolderFront
            | Self::FolderFilled
            | Self::Explorer => mask!("folder"),
            Self::Pdf
            | Self::Sheet
            | Self::Word
            | Self::CodeFile
            | Self::VideoFile
            | Self::AudioFile
            | Self::ArchiveFile
            | Self::ModelFile
            | Self::ModelFileDark
            | Self::ImageFile
            | Self::PresentationFile
            | Self::Document
            | Self::TextFile
            | Self::Application
            | Self::BatchFile
            | Self::SystemFile
            | Self::InstallerFile
            | Self::SettingsFile
            | Self::PowerShellFile
            | Self::RegistryFile
            | Self::InternetShortcut
            | Self::FontFile
            | Self::DiscImage
            | Self::VirtualDisk
            | Self::CabinetFile
            | Self::CertificateFile
            | Self::Notepad => mask!("document"),
            Self::PC => windows!("explorer-pc"),
            Self::AddCircle => windows!("explorer-add-circle"),
            Self::Add => windows!("explorer-add"),
            Self::NewTab => windows!("explorer-new-tab"),
            Self::Tag => windows!("explorer-tag"),
            Self::Close => windows!("explorer-close"),
            Self::Preview => windows!("explorer-preview"),
            Self::Info => windows!("explorer-info"),
            Self::DetailsPane => windows!("explorer-pane"),
            Self::NewFolder => windows!("explorer-new-folder"),
            Self::Terminal => windows!("explorer-terminal"),
            Self::Refresh => windows!("explorer-refresh"),
            Self::Up => windows!("explorer-up"),
            Self::Power => mask!("power"),
            Self::Save => mask!("save"),
            Self::Open => windows!("explorer-open"),
            Self::Back => windows!("explorer-back"),
            Self::Forward => windows!("explorer-forward"),
            Self::Cut => windows!("explorer-cut"),
            Self::Copy => windows!("explorer-copy"),
            Self::Paste => windows!("explorer-paste"),
            Self::Rename => windows!("explorer-rename"),
            Self::Share => windows!("explorer-share"),
            Self::Delete => windows!("explorer-delete"),
            Self::Sort => windows!("explorer-sort"),
            Self::View => windows!("explorer-view"),
            Self::Details => windows!("explorer-details"),
            Self::LargeIcons => windows!("explorer-icons"),
            Self::More => windows!("explorer-more"),
            Self::Search => windows!("explorer-search"),
            Self::Chevron => windows!("explorer-chevron"),
            Self::Pin => windows!("explorer-pin"),
            // Only drawn in its 11 px box (Windows' 16 px at 150 %): every
            // scale has a native mask, whatever `size` asks for.
            Self::Pinned => match scale {
                3 => door_workspace::asset!(
                    "../../../assets/icons/windows-shell/explorer-pinned.33.mask"
                ),
                2 => door_workspace::asset!(
                    "../../../assets/icons/windows-shell/explorer-pinned.22.mask"
                ),
                _ => door_workspace::asset!(
                    "../../../assets/icons/windows-shell/explorer-pinned.11.mask"
                ),
            },
            Self::Star => windows!("explorer-star"),
            Self::StarOutline => windows!("explorer-star-outline"),
            Self::Shield => windows!("shell-shield"),
            Self::SidebarDownloads => windows!("explorer-download"),
            Self::ChevronRight => windows!("explorer-chevron-right"),
            Self::Settings => mask!("settings"),
            Self::Home => mask!("home"),
            Self::Desktop
            | Self::Documents
            | Self::Downloads
            | Self::Music
            | Self::Pictures
            | Self::Videos
            | Self::Computer
            | Self::Drive => mask!("folder"),
            Self::SidebarDocuments
            | Self::SidebarMusic
            | Self::SidebarPictures
            | Self::SidebarVideos
            | Self::Network
            | Self::NoneDrive
            | Self::Linux => mask!("folder"),
            Self::GlyphDocument => windows!("explorer-document"),
            Self::GlyphFolder => windows!("explorer-folder"),
            Self::GlyphMusic => windows!("explorer-music"),
            Self::GlyphVideo => windows!("explorer-video"),
            Self::Zip => windows!("explorer-zip"),
            Self::Link => windows!("explorer-link"),
            Self::Properties => windows!("explorer-properties"),
            Self::NewWindow => windows!("explorer-window"),
            Self::Mail => windows!("explorer-mail"),
            Self::Convert => windows!("explorer-convert"),
            Self::Check => windows!("explorer-check"),
            Self::RecycleBin | Self::RecycleBinFull => {
                let px = match (size, scale) {
                    (12 | 16 | 20, 1..=3) => (size * scale) as usize,
                    _ => 20,
                };
                door_workspace::file_operations::bin_mask(matches!(self, Self::RecycleBinFull), px)
            }
        }
    }
}
#[cfg(test)]
mod folder_tests {
    use super::*;
    #[test]
    fn folder_contents_sit_between_the_back_and_front_layers() {
        // The exporter's grid at 48 and 96 px (left 3/6, back top 11/22,
        // front top 22/44): the window starts under the back plate's top
        // and runs below the front plate's top, which hides the rest.
        assert_eq!(
            Icon::folder_layers(48),
            FolderLayers {
                content: (5, 12, 38, 16),
                front_top: 22
            }
        );
        assert_eq!(Icon::folder_layers(96).content, (10, 24, 76, 32));
        for size in [16, 20, 32, 40, 48, 60, 80, 96, 120, 160, 192, 240] {
            let l = Icon::folder_layers(size);
            let (x, y, w, h) = l.content;
            assert!(x > 0 && x as u32 + w < size && w > size / 2, "{size}");
            assert!(y < l.front_top && y + h as i32 > l.front_top, "{size}");
            assert!((y + h as i32) < size as i32, "{size}");
        }
    }
}

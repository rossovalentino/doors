//! Shared logical-pixel Settings window geometry and the route model.
//! Geometry follows the Windows 11 26H2 Settings window (owner reference
//! `7.webp`): back button and caption in a 48 px title bar, the search box
//! centred in the client caption control, an account header and the category
//! rail. Paint, hit testing and keyboard focus all derive from these values.
pub const INCLUDED_APPS: [(&str, u8); 6] = [
    ("File Explorer", 1),
    ("Notes", 2),
    ("Settings", 3),
    ("Terminal", 9),
    ("Preview", 10),
    ("Task Manager", 11),
];
pub fn installed_app_command(index: usize) -> Option<u8> {
    INCLUDED_APPS.get(index).map(|app| app.1)
}
/// A built-in theme, as `C:\Windows\Resources\Themes\*.theme` defines it on
/// Windows 11 26H2: picture (the first of its album), app and system mode,
/// accent (`ColorizationColor`, `AutoColorization=0`) and, for the four
/// picture sets, a slideshow of the album every 30 minutes
/// (`[Slideshow] Interval=1800000`). Windows spotlight is left out: its
/// pictures come from Microsoft's online service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeDef {
    pub name: &'static str,
    pub dark: bool,
    /// Pack id (`door_initfs::desktop_assets::WALLPAPERS`).
    pub background: u32,
    /// Accent RGB.
    pub accent: u32,
    pub slideshow: bool,
    /// Pictures, as Windows' theme tiles name them ("4 images").
    pub images: u32,
}
impl ThemeDef {
    /// The appearance word: bit 0 dark, bits 1.. the background.
    pub const fn appearance(self) -> u32 {
        self.background << 1 | self.dark as u32
    }
    /// The accent setting the theme applies: Windows' default blue is the
    /// default accent (a clean install is "Doors (light)"), any other
    /// colour a manual one.
    pub const fn accent_bits(self) -> u32 {
        use door_service_protocol::desktop::Accent;
        if self.accent == 0x0078d4 {
            Accent::DEFAULT.bits()
        } else {
            Accent::manual(self.accent).bits()
        }
    }
}
const fn theme(
    name: &'static str,
    dark: bool,
    background: u32,
    accent: u32,
    slideshow: bool,
) -> ThemeDef {
    ThemeDef {
        name,
        dark,
        background,
        accent,
        slideshow,
        images: if slideshow { 4 } else { 1 },
    }
}
/// Personalization > Themes order (aero, dark, themeA..D).
pub const THEMES: [ThemeDef; 6] = [
    theme("Doors (light)", false, 0, 0x0078d4, false),
    theme("Doors (dark)", true, 1, 0x0078d4, false),
    theme("Glow", true, 2, 0x680081, true),
    theme("Captured Motion", true, 7, 0xd0000c, true),
    theme("Sunrise", false, 3, 0x459bac, true),
    theme("Flow", false, 14, 0x506773, true),
];
/// "Select a theme to apply" on Home and Personalization (Windows caps that
/// gallery at six, in this order): indices into `THEMES`.
pub const GALLERY: [usize; 6] = [1, 2, 0, 3, 4, 5];
/// Built-in picture names by pack id.
pub const BACKGROUNDS: [&str; door_initfs::desktop_assets::COUNT] = {
    let mut names = [""; door_initfs::desktop_assets::COUNT];
    let mut i = 0;
    while i < names.len() {
        names[i] = door_initfs::desktop_assets::WALLPAPERS[i].name;
        i += 1;
    }
    names
};
use door_design_tokens as tokens;
/// The route table lives in `door-settings-map` (shared with the shell's
/// Run, Start search and launcher, which resolve legacy commands to it).
pub use door_settings_map::Page;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scrollbar {
    pub top: u32,
    pub thumb: u32,
    pub travel: u32,
}
impl Scrollbar {
    pub fn position(self, y: i32, grab: i32, max_scroll: u32) -> u32 {
        (((i64::from(y) - i64::from(Layout::CONTENT_TOP) - i64::from(grab))
            .clamp(0, self.travel as i64) as u64
            * max_scroll as u64
            + self.travel as u64 / 2)
            / self.travel.max(1) as u64) as u32
    }
}
/// Title-bar search box rectangle (inside the client caption control).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchBox {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}
impl SearchBox {
    pub fn contains(self, x: i32, y: i32) -> bool {
        (self.x..self.x + self.w as i32).contains(&x)
            && (self.y..self.y + self.h as i32).contains(&y)
    }
}
impl Layout {
    /// Left edge of page content (heading and cards).
    pub const MAIN: i32 = 310;
    /// Title bar height painted by Settings (trusted caption buttons are 32).
    pub const TITLE: i32 = 48;
    /// Top of the scrolling content viewport, below the fixed page heading
    /// (26H2 at 150 %: scrolled content clips at 158 px = 105 epx).
    pub const CONTENT_TOP: i32 = 105;
    /// Category rail rows (Windows NavigationView: 36 px items, 4 px gaps).
    pub const NAV_PITCH: i32 = 40;
    /// Navigation rail pitch and row at the applied text size (14 px labels).
    /// Growth of the page heading's line box (28 px semibold) at the applied
    /// text size; the content below moves down by it. Zero at 100%.
    pub fn heading_growth() -> i32 {
        crate::palette::text_grow_i(0, door_graphics_text::TextStyle::Semibold28)
    }
    /// Top of the scrolling content, below the (grown) page heading.
    pub fn content_top() -> i32 {
        Self::CONTENT_TOP + Self::heading_growth()
    }
    pub fn nav_pitch() -> i32 {
        crate::palette::text_grow_i(Self::NAV_PITCH, door_graphics_text::TextStyle::Regular14)
    }
    pub fn nav_row() -> i32 {
        crate::palette::text_grow_i(Self::NAV_ROW, door_graphics_text::TextStyle::Regular14)
    }
    /// Navigation items: `size.nav_item_height` (36), inset 17 and 279 wide
    /// (26H2 measured: x 16.7, width 279.3 epx; docs/design/
    /// fidelity-26h2-diff.md G2). The icon sits 10 px into the item, the
    /// label 44 px, and the selection pill is 3 x 14 (`radius.bar`).
    pub const NAV_ROW: i32 = tokens::spacing(tokens::SpacingToken::NavItemHeight) as i32;
    pub const NAV_X: i32 = 17;
    pub const NAV_W: u32 = 279;
    pub const NAV_ICON: i32 = 10;
    pub const NAV_LABEL: i32 = 44;
    pub const NAV_PILL: (u32, u32) = (3, 14);
    /// Rail overlay scrollbar column (shown only while the rail is hovered
    /// or dragged: the 26H2 scroll indicator is hidden at rest).
    pub const RAIL_BAR: i32 = 296;
    /// Window server client-caption policy (`create_client_caption`): the
    /// 26H2 "Find a setting" pill is centred in the title bar, at most 516
    /// wide, and keeps 190 px from both window edges (owner capture
    /// 2026-10-27: x 190..663 in an 856 px window). Back and "Settings" sit
    /// left of it, the trusted caption buttons right of it.
    pub const CAPTION_CONTROL: u32 = 516;
    pub const CAPTION_MARGIN: u32 = 190;
    /// Narrowest usable title-bar search (magnifier plus a few characters).
    /// A narrower window has no search box (26H2 keeps search out of the
    /// rail), and Ctrl+F does nothing there.
    pub const SEARCH_MIN: u32 = 120;
    /// First rail row: the rail starts right below the account block.
    pub const RAIL_TOP: i32 = 144;
    /// Generic Windows pages stop widening here; Advanced pages use `main_width`.
    pub const CONTENT_MAX: u32 = 1000;
    pub fn back(self) -> (i32, i32, u32, u32) {
        (4, 8, 36, 32)
    }
    pub fn back_contains(self, x: i32, y: i32) -> bool {
        let (bx, by, bw, bh) = self.back();
        self.width >= 48 && (bx..bx + bw as i32).contains(&x) && (by..by + bh as i32).contains(&y)
    }
    /// The trusted caption delivers input only inside this centred control.
    pub fn caption_control(self) -> Option<(i32, u32)> {
        let width = Self::CAPTION_CONTROL.min(self.width.saturating_sub(2 * Self::CAPTION_MARGIN));
        (width > 0).then(|| (((self.width - width) / 2) as i32, width))
    }
    /// The title-bar search pill: the whole caption control, 32 tall at y 8.
    pub fn search(self) -> Option<SearchBox> {
        if self.height < 200 {
            return None;
        }
        let (x, w) = self.caption_control()?;
        (w >= Self::SEARCH_MIN).then_some(SearchBox { x, y: 8, w, h: 32 })
    }
    pub fn rail_top(self) -> i32 {
        Self::RAIL_TOP
    }
    pub fn scrollbar(self, scroll: u32, max_scroll: u32) -> Option<Scrollbar> {
        if !self.has_main() || max_scroll == 0 || self.height <= 144 {
            return None;
        }
        let track = self.height - 144;
        let thumb = ((track as u64 * track as u64 / (track as u64 + max_scroll as u64)).max(24)
            as u32)
            .min(track);
        let travel = track - thumb;
        Some(Scrollbar {
            top: Self::CONTENT_TOP as u32
                + (travel as u64 * scroll.min(max_scroll) as u64 / max_scroll as u64) as u32,
            thumb,
            travel,
        })
    }
    pub fn main_width(self) -> u32 {
        self.width.saturating_sub(332).min(1600)
    }
    /// Card width of ordinary (Windows) pages.
    pub fn content_width(self) -> u32 {
        self.main_width().min(Self::CONTENT_MAX)
    }
    pub fn has_main(self) -> bool {
        self.main_width() >= 280
    }
    pub fn main_contains(self, x: i32, y: i32) -> bool {
        self.has_main()
            && x >= Self::MAIN
            && x < Self::MAIN + self.main_width() as i32
            && y >= Self::CONTENT_TOP
            && y < self.height as i32
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installed_apps_keep_terminal_launch_identity() {
        assert_eq!(installed_app_command(3), Some(9));
        assert_eq!(installed_app_command(4), Some(10));
        assert_eq!(installed_app_command(5), Some(11));
        assert_eq!(installed_app_command(6), None);
    }
    #[test]
    fn scroll_thumb_preserves_grab_offset() {
        let full = Layout {
            width: 1129,
            height: 640,
        };
        let thumb = full.scrollbar(100, 250).unwrap();
        assert!(thumb.position(thumb.top as i32 + 8, 8, 250).abs_diff(100) <= 1);
        assert_eq!(thumb.position(i32::MAX, 8, 250), 250);
        assert_eq!(thumb.position(-100, 8, 250), 0);
    }
    #[test]
    fn narrow_or_clipped_pages_have_no_invisible_main_targets() {
        for width in 160..612 {
            let layout = Layout { width, height: 720 };
            for y in [132, 390, 600, 678] {
                assert!(!layout.main_contains(310, y));
            }
        }
        let layout = Layout {
            width: 1129,
            height: 600,
        };
        assert!(layout.main_contains(334, 599));
        assert!(!layout.main_contains(334, 600));
    }
    #[test]
    fn search_lives_in_the_title_bar_and_never_in_the_rail() {
        // Reference window: 516 px search centred in the title bar (7.webp).
        let l = Layout {
            width: 1129,
            height: 787,
        };
        let s = l.search().unwrap();
        assert_eq!((s.x, s.y, s.w, s.h), (306, 8, 516, 32));
        assert_eq!(s.x + s.w as i32 / 2, 1129 / 2);
        assert_eq!(l.rail_top(), 144);
        assert_eq!(l.main_width(), 797);
        assert_eq!(l.content_width(), 797);
        // Owner capture (26H2, 856 px window): the pill spans x 190..663.
        let s = Layout {
            width: 856,
            height: 1018,
        }
        .search()
        .unwrap();
        assert_eq!((s.x, s.x + s.w as i32), (190, 666));
        // The caption control shrinks with the window and keeps both margins;
        // the rail always starts below the account block.
        for width in [500, 640, 717, 760, 896, 1129, 1920] {
            let l = Layout { width, height: 700 };
            let s = l.search().unwrap();
            assert!(s.x >= 190 && s.x + s.w as i32 <= width as i32 - 190);
            assert_eq!(s.x + s.w as i32 / 2, width as i32 / 2);
            assert_eq!(l.caption_control(), Some((s.x, s.w)));
            assert_eq!(l.rail_top(), 144);
        }
        // Too narrow for a usable pill: no search box at all.
        for (width, height) in [(499, 700), (240, 700), (1129, 199)] {
            let l = Layout { width, height };
            assert_eq!(l.search(), None, "{width}x{height}");
            assert_eq!(l.rail_top(), 144);
        }
        let l = Layout {
            width: 700,
            height: 700,
        };
        // Back is inside the caption's client strip (x < 32 is client input).
        assert!(l.back_contains(10, 20));
        assert!(!l.back_contains(10, 44));
        assert!(
            Layout {
                width: 1920,
                height: 1032
            }
            .content_width()
                == Layout::CONTENT_MAX
        );
    }
    #[test]
    fn titles_keywords_and_stable_identities() {
        assert!(Page::Personalization.name_matches("PERSON"));
        assert!(!Page::System.name_matches("network"));
        assert!(Page::Display.name_matches("DISPLAY"));
        assert!(Page::InstalledApps.name_matches("installed"));
        assert!(Page::Colors.matches_keywords("dark mode"));
        assert!(!Page::Colors.matches_keywords(""));
        for (index, page) in Page::ALL.iter().enumerate() {
            assert_eq!(*page as usize, index);
            assert_eq!(Page::from_index(index), Some(*page));
            // Every page is reachable from a rail category.
            assert!(Page::NAV.contains(&page.category()));
        }
        // Identities reported to existing guest scenarios never move.
        assert_eq!(Page::About as usize, 12);
        assert_eq!(Page::Display as usize, 13);
        assert_eq!(Page::InstalledApps as usize, 14);
        assert_eq!(Page::Taskbar as usize, 15);
        assert_eq!(Page::ContextMenu as usize, 16);
        assert_eq!(Page::Registry as usize, 17);
        assert_eq!(Page::DeveloperTools as usize, 32);
    }
}

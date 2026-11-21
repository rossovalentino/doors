//! The Settings category rail, the route-derived breadcrumb heading and the
//! title-bar search suggestions. The rail holds Windows 11's twelve
//! categories only: every other page sits under one of them (at most two
//! levels down) and search reaches all of them, by name, section, keyword
//! or legacy tool name (`door_settings_map::search`).
use crate::settings_layout::{Layout, Page};
use door_settings_map::Hit;
#[cfg(test)]
extern crate alloc;
/// Rail rows: the categories, in Windows 11's order.
pub fn items() -> impl Iterator<Item = Page> {
    Page::NAV.into_iter()
}
/// Search suggestions: page names first, then sections, keywords and legacy
/// names (one per destination).
pub fn suggestions(query: &str) -> impl Iterator<Item = Hit> + '_ {
    door_settings_map::search(query).take(SUGGESTIONS)
}
pub const SUGGESTIONS: usize = 8;
/// What a suggestion row shows: its title and a secondary line.
pub fn suggestion_text(hit: Hit) -> (&'static str, &'static str) {
    match hit {
        Hit::Page(page) | Hit::Keyword(page) => (
            page.nav_title(),
            if page.depth() > 0 {
                page.parent().nav_title()
            } else {
                ""
            },
        ),
        Hit::Section(s) => (s.title, s.page.nav_title()),
        Hit::Legacy(r) => (
            r.section
                .and_then(|anchor| door_settings_map::section(r.page, anchor))
                .map_or(r.page.nav_title(), |s| s.title),
            r.name,
        ),
    }
}
/// Suggestion list geometry below the search box: (x, y, w, row pitch).
pub fn suggestion_box(layout: Layout) -> Option<(i32, i32, u32, i32)> {
    let s = layout.search()?;
    Some((
        s.x,
        s.y + s.h as i32 + 4,
        s.w.max(280)
            .min(layout.width.saturating_sub(s.x as u32 + 8)),
        36,
    ))
}
#[derive(Clone, Copy, Debug)]
pub struct Rail {
    pub height: u32,
    pub scroll: u32,
    pub top: i32,
}
impl Rail {
    pub fn new(layout: Layout, scroll: u32) -> Self {
        Self {
            height: layout.height,
            scroll,
            top: layout.rail_top(),
        }
    }
    pub fn bottom(self) -> i32 {
        self.height.saturating_sub(12) as i32
    }
    pub fn max_scroll(self, count: usize) -> u32 {
        (count as u32 * Layout::nav_pitch() as u32)
            .saturating_sub(self.height.saturating_sub(self.top as u32 + 12))
    }
    pub fn row(self, index: usize) -> i32 {
        self.top + index as i32 * Layout::nav_pitch() - self.scroll as i32
    }
    pub fn hit(self, x: i32, y: i32, count: usize) -> Option<usize> {
        if !(Layout::NAV_X..Layout::NAV_X + Layout::NAV_W as i32).contains(&x)
            || y < self.top
            || y >= self.bottom()
        {
            return None;
        }
        let offset = (y - self.top) as u32 + self.scroll;
        let row = (offset / Layout::nav_pitch() as u32) as usize;
        (row < count && offset % (Layout::nav_pitch() as u32) < Layout::nav_row() as u32)
            .then_some(row)
    }
    pub fn reveal(self, index: usize, count: usize) -> u32 {
        let top = self.top + index as i32 * Layout::nav_pitch();
        let mut scroll = self.scroll;
        if top - (scroll as i32) < self.top {
            scroll = (top - self.top) as u32;
        }
        if top + Layout::nav_row() - scroll as i32 > self.bottom() {
            scroll = (top + Layout::nav_row() - self.bottom()).max(0) as u32;
        }
        scroll.min(self.max_scroll(count))
    }
    /// Overlay thumb (top, height, travel) for `count` rows.
    pub fn thumb(self, count: usize) -> (i32, i32, i32) {
        let track = (self.bottom() - self.top).max(1);
        let max = self.max_scroll(count);
        let thumb = (i64::from(track) * i64::from(track) / (i64::from(track) + i64::from(max)))
            .max(24)
            .min(i64::from(track)) as i32;
        let travel = track - thumb;
        (
            self.top
                + (i64::from(travel) * i64::from(self.scroll.min(max)) / i64::from(max.max(1)))
                    as i32,
            thumb,
            travel,
        )
    }
}
/// The ancestors shown before a page's heading, nearest last. They come from
/// the route model (`Page::parent`), not from chronological Back history.
pub fn ancestors(page: Page) -> impl Iterator<Item = Page> {
    page.crumbs().take(page.depth())
}
/// Measured breadcrumb heading (System > Storage > Disks & volumes): muted
/// ancestors, chevrons, then the current page. Too narrow for everything,
/// whole ancestors are dropped from the left before the current title is
/// ever clipped, as Windows' breadcrumb collapses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Heading {
    /// Shown ancestors (page, x, width), left to right.
    pub ancestors: [Option<(Page, i32, u32)>; 2],
    /// Chevron x after each shown ancestor.
    pub chevrons: [i32; 2],
    pub current_x: i32,
    pub current_width: u32,
}
pub const HEADING_GAP: i32 = 12;
pub const HEADING_CHEVRON: i32 = 16;
pub fn heading(page: Page, x: i32, available: u32, measure: impl Fn(&str) -> u32) -> Heading {
    let current = measure(page.nav_title());
    let separator = (2 * HEADING_GAP + HEADING_CHEVRON) as u32;
    let all: [Option<Page>; 2] = {
        let mut list = [None; 2];
        for (i, p) in ancestors(page).enumerate() {
            list[i] = Some(p);
        }
        list
    };
    let count = all.iter().flatten().count();
    // Keep as many trailing ancestors as fit in full.
    let mut skip = 0;
    while skip < count {
        let need: u32 = all[skip..count]
            .iter()
            .flatten()
            .map(|p| measure(p.nav_title()) + separator)
            .sum();
        if need + current <= available {
            break;
        }
        skip += 1;
    }
    let mut heading = Heading {
        ancestors: [None; 2],
        chevrons: [x; 2],
        current_x: x,
        current_width: current.min(available),
    };
    let mut cursor = x;
    for (slot, p) in all[skip..count].iter().flatten().enumerate() {
        let w = measure(p.nav_title());
        heading.ancestors[slot] = Some((*p, cursor, w));
        heading.chevrons[slot] = cursor + w as i32 + HEADING_GAP;
        cursor = heading.chevrons[slot] + HEADING_CHEVRON + HEADING_GAP;
    }
    heading.current_x = cursor;
    heading.current_width = current.min(available.saturating_sub((cursor - x) as u32));
    heading
}
#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;
    #[test]
    fn heading_hierarchy_comes_from_routes_and_degrades_without_overlap() {
        let measure = |s: &str| s.len() as u32 * 14;
        let h = heading(Page::Taskbar, 310, 900, measure);
        let (crumb, x, w) = h.ancestors[0].unwrap();
        assert_eq!(crumb, Page::Personalization);
        assert_eq!(x, 310);
        assert!(x + w as i32 + HEADING_GAP <= h.chevrons[0]);
        assert_eq!(h.current_x, h.chevrons[0] + HEADING_CHEVRON + HEADING_GAP);
        assert_eq!(h.current_width, measure("Taskbar"));
        assert_eq!(
            heading(Page::Personalization, 310, 900, measure).ancestors,
            [None; 2]
        );
        // Three levels: System > Storage > Disks & volumes.
        let h = heading(Page::DiskManagement, 310, 900, measure);
        assert_eq!(h.ancestors[0].map(|a| a.0), Some(Page::System));
        assert_eq!(h.ancestors[1].map(|a| a.0), Some(Page::Storage));
        assert!(h.ancestors[0].unwrap().1 + h.ancestors[0].unwrap().2 as i32 <= h.chevrons[0]);
        assert!(h.chevrons[0] < h.ancestors[1].unwrap().1);
        // Narrow: ancestors are whole or gone (leftmost first); the page
        // title stays.
        for available in [0, 60, 120, 160, 200, 260, 400, 520, 700] {
            let h = heading(Page::Registry, 310, available, measure);
            assert!(
                h.current_x + h.current_width as i32 <= 310 + available as i32 || available == 0
            );
            let shown: Vec<_> = h.ancestors.iter().flatten().map(|a| a.0).collect();
            assert!(shown.is_empty() || *shown.last().unwrap() == Page::SystemAdvanced);
            for (p, _, w) in h.ancestors.iter().flatten() {
                assert_eq!(*w, measure(p.nav_title()));
            }
        }
    }
    #[test]
    fn the_rail_holds_the_twelve_windows_categories() {
        assert_eq!(items().collect::<Vec<_>>(), Page::NAV);
        assert_eq!(items().count(), 12);
    }
    /// docs/design/fidelity-26h2-diff.md G2 (settings-home.uia.tsv at
    /// 150 %): items at x 25 px (16.7 epx), 419 px (279.3) wide, 54 px (36)
    /// tall at a 60 px (40) pitch from y 215 px (143.3); the icon 10 epx
    /// into the item and a 3 x 14 accent pill.
    #[test]
    fn rail_items_follow_the_measured_26h2_geometry() {
        assert_eq!((Layout::NAV_X, Layout::NAV_W), (17, 279));
        assert_eq!((Layout::NAV_ROW, Layout::NAV_PITCH), (36, 40));
        assert_eq!(
            Layout::NAV_ROW,
            door_design_tokens::spacing(door_design_tokens::SpacingToken::NavItemHeight) as i32
        );
        assert_eq!((Layout::NAV_ICON, Layout::NAV_LABEL), (10, 44));
        assert_eq!(Layout::NAV_PILL, (3, 14));
        let rail = Rail::new(
            Layout {
                width: 1129,
                height: 787,
            },
            0,
        );
        assert_eq!(rail.row(0), 144);
        assert_eq!(rail.hit(17, 144, 12), Some(0));
        assert_eq!(rail.hit(16, 144, 12), None);
        assert_eq!(rail.hit(295, 144, 12), Some(0));
        assert_eq!(rail.hit(296, 144, 12), None);
    }
    #[test]
    fn suggestions_rank_titles_before_keywords_and_keep_scenario_first_matches() {
        // Guest scenarios type these and press Return (first suggestion).
        for (query, page) in [
            ("drivers", Page::DevicesDrivers),
            ("disk management", Page::DiskManagement),
            ("services", Page::Services),
            ("registry", Page::Registry),
            ("group policy", Page::GroupPolicy),
            ("task scheduler", Page::TaskScheduler),
            ("personalization", Page::Personalization),
            ("installed apps", Page::InstalledApps),
            ("taskbar", Page::Taskbar),
            ("context menu", Page::ContextMenu),
            ("colors", Page::Colors),
            ("environment", Page::Environment),
            ("typing", Page::Typing),
            ("visual effects", Page::VisualEffects),
        ] {
            assert_eq!(
                suggestions(query).next().map(Hit::target),
                Some((page, None)),
                "{query}"
            );
        }
        let dark: Vec<_> = suggestions("dark mode").map(Hit::target).collect();
        assert_eq!(dark[0], (Page::Colors, None));
        assert_eq!(suggestions("").count(), 0);
        assert_eq!(suggestions("   ").count(), 0);
        assert!(suggestions("e").count() <= SUGGESTIONS);
        let titles: Vec<_> = suggestions("display").map(Hit::target).collect();
        assert_eq!(titles[0], (Page::Display, None));
        // A legacy name shows its replacement and the name it was found by.
        let hit = suggestions("Device Manager").next().unwrap();
        assert_eq!(suggestion_text(hit), ("Devices", "Device Manager"));
    }
    #[test]
    fn independent_rail_clips_hits_and_reveals_rows_at_short_heights() {
        for height in [240, 400, 632, 752, 900] {
            for top in [Layout::RAIL_TOP, 176] {
                let mut rail = Rail {
                    height,
                    scroll: 0,
                    top,
                };
                let count = 12;
                rail.scroll = rail.reveal(11, count);
                assert!(rail.scroll <= rail.max_scroll(count));
                let row = rail.row(11);
                assert!(row >= rail.top);
                assert!(row + Layout::nav_row() <= rail.bottom());
                assert_eq!(rail.hit(20, row, count), Some(11));
                assert_eq!(rail.hit(20, row + 35, count), Some(11));
                assert_eq!(rail.hit(20, row + 36, count), None);
                assert_eq!(rail.hit(20, rail.top - 1, count), None);
                assert_eq!(rail.hit(20, rail.bottom(), count), None);
                assert_eq!(rail.hit(296, row, count), None);
                assert_eq!(rail.hit(16, row, count), None);
                let (thumb_top, thumb, travel) = rail.thumb(count);
                assert!(thumb_top >= rail.top && thumb_top + thumb <= rail.bottom());
                assert!(travel >= 0);
            }
        }
        // Reference geometry: Home row at 144, 40 px pitch (7.webp).
        let rail = Rail::new(
            Layout {
                width: 1129,
                height: 787,
            },
            0,
        );
        assert_eq!(rail.row(0), 144);
        assert_eq!(rail.row(1), 184);
        assert_eq!(rail.hit(20, 144, 12), Some(0));
        assert_eq!(rail.max_scroll(12), 0);
    }
}

//! Weather (MSN Weather 4.54), measured at 150%: the accent title bar, a
//! 48 px navigation rail (menu; Forecast, Maps, Historical Weather, Monthly
//! Forecast, Health & Activities, Air Quality, Weather Trends, Favorites,
//! Send Feedback; account and Settings at the bottom), the 48 px header (the
//! page title; Refresh, Add to favorites, Pin to Start, units; "Search for
//! location") over the content. The forecast comes from the network, which
//! Doors OS does not have yet: every forecast page says so plainly instead
//! of inventing weather. Settings (units, theme) and favorites work.
use crate::paint::view::{Rect, button};
use alloc::{format, string::String, vec, vec::Vec};
use door_app_ui::{Application, View};
use door_graphics_text::{FontAtlas, TextStyle};
use door_service_client::{Error, Storage};

pub const TITLE: i32 = 32;
const RAIL: u32 = 48;
const HEADER: i32 = 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Forecast,
    Maps,
    History,
    Monthly,
    Health,
    Air,
    Trends,
    Favorites,
    Feedback,
    Account,
    Settings,
}
impl Page {
    const RAIL: [Self; 9] = [
        Self::Forecast,
        Self::Maps,
        Self::History,
        Self::Monthly,
        Self::Health,
        Self::Air,
        Self::Trends,
        Self::Favorites,
        Self::Feedback,
    ];
    pub const fn title(self) -> &'static str {
        match self {
            Self::Forecast => "Forecast",
            Self::Maps => "Maps",
            Self::History => "Historical Weather",
            Self::Monthly => "Monthly Forecast",
            Self::Health => "Health & Activities",
            Self::Air => "Air Quality",
            Self::Trends => "Weather Trends",
            Self::Favorites => "Favorites",
            Self::Feedback => "Send Feedback",
            Self::Account => "Account",
            Self::Settings => "Settings",
        }
    }
    const fn glyph(self) -> &'static str {
        match self {
            Self::Forecast => "weather-home",
            Self::Maps => "weather-maps",
            Self::History => "weather-history",
            Self::Monthly => "weather-calendar",
            Self::Health => "weather-health",
            Self::Air => "weather-air",
            Self::Trends => "weather-trends",
            Self::Favorites => "weather-favorites",
            Self::Feedback => "weather-feedback",
            Self::Account => "weather-account",
            Self::Settings => "weather-settings",
        }
    }
    /// Pages whose content is fetched from the forecast service.
    const fn online(self) -> bool {
        !matches!(
            self,
            Self::Favorites | Self::Settings | Self::Feedback | Self::Account
        )
    }
}

mod id {
    pub const MENU: u32 = 1;
    pub const PAGE: u32 = 10; // + rail index; ACCOUNT 30, SETTINGS 31
    pub const ACCOUNT: u32 = 30;
    pub const SETTINGS: u32 = 31;
    pub const REFRESH: u32 = 40;
    pub const FAVORITE: u32 = 41;
    pub const PIN: u32 = 42;
    pub const UNITS: u32 = 43;
    pub const SEARCH: u32 = 44;
    pub const RETRY: u32 = 50;
    pub const OPTION: u32 = 60; // settings radios
    pub const REMOVE: u32 = 80; // favorites
}

struct Ink {
    accent: [u8; 4],
    rail: [u8; 4],
    header: [u8; 4],
    content: [u8; 4],
    field: [u8; 4],
    divider: [u8; 4],
    text: [u8; 4],
    secondary: [u8; 4],
    hover: [u8; 4],
}
/// Round to the nearest integer (no libm in the guest).
fn rnd(x: f64) -> i32 {
    if x < 0.0 {
        (x - 0.5) as i32
    } else {
        (x + 0.5) as i32
    }
}
const fn rgb(r: u8, g: u8, b: u8) -> [u8; 4] {
    [b, g, r, 255]
}
fn ink(dark: bool) -> Ink {
    if dark {
        Ink {
            accent: rgb(0, 99, 177),
            rail: rgb(31, 31, 31),
            header: rgb(23, 23, 23),
            content: rgb(32, 32, 32),
            field: rgb(46, 46, 46),
            divider: rgb(76, 76, 76),
            text: rgb(255, 255, 255),
            secondary: rgb(186, 186, 186),
            hover: rgb(51, 51, 51),
        }
    } else {
        Ink {
            accent: rgb(0, 99, 177),
            rail: rgb(230, 230, 230),
            header: rgb(242, 242, 242),
            content: rgb(255, 255, 255),
            field: rgb(255, 255, 255),
            divider: rgb(204, 204, 204),
            text: rgb(0, 0, 0),
            secondary: rgb(96, 96, 96),
            hover: rgb(214, 214, 214),
        }
    }
}

pub struct Weather {
    pub width: u32,
    pub height: u32,
    ready: bool,
    pub page: Page,
    pub expanded: bool,
    pub celsius: bool,
    /// 0 light, 1 dark, 2 Windows mode.
    pub theme: u8,
    pub query: String,
    pub searching: bool,
    pub favorites: Vec<String>,
    pub notice: Option<&'static str>,
    pub retries: u32,
    /// The service registry (door.network); 0 in tests.
    registry: u64,
    /// The place shown, its forecast, the request in flight and the last
    /// failure.
    pub place: Option<crate::weather_data::Place>,
    pub forecast: Option<crate::weather_data::Forecast>,
    fetch: Option<(Want, door_workspace::https::Fetch)>,
    pub error: Option<&'static str>,
}
/// What a request in flight is for.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Want {
    Places(String),
    Forecast,
}

impl Default for Weather {
    fn default() -> Self {
        Self::new()
    }
}
impl Weather {
    /// Give Weather the session's service registry (forecast requests).
    pub fn with_registry(mut self, registry: u64) -> Self {
        self.registry = registry;
        self
    }
    /// Start a request (one at a time; a newer one replaces it).
    fn request(&mut self, want: Want, url: &str) {
        let now = door_sys::time().unwrap_or(0);
        if self.registry == 0 {
            self.error = Some("You're offline.");
            return;
        }
        match door_workspace::https::Fetch::start(self.registry, url, now) {
            Ok(f) => {
                self.fetch = Some((want, f));
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }
    fn search(&mut self, query: &str) {
        let q = String::from(query.trim());
        if !q.is_empty() {
            let url = crate::weather_data::geocode_url(&q);
            self.request(Want::Places(q), &url);
        }
    }
    fn refresh(&mut self) {
        if let Some(p) = self.place.clone() {
            let url = crate::weather_data::forecast_url(&p, self.celsius);
            self.request(Want::Forecast, &url);
        }
    }
    /// A request is in flight.
    pub fn loading(&self) -> bool {
        self.fetch.is_some()
    }
    /// Drive the request in flight; true when something changed.
    fn poll(&mut self) -> bool {
        use door_workspace::https::Poll;
        let Some((want, fetch)) = self.fetch.as_mut() else {
            return false;
        };
        let now = door_sys::time().unwrap_or(0);
        match fetch.poll(now) {
            Poll::Pending => false,
            Poll::Failed(e) => {
                self.fetch = None;
                self.error = Some(e);
                door_rt::diagnostic(format_args!("DOOR_P15 WEATHER FAILED reason={e}\n"));
                true
            }
            Poll::Done(r) => {
                let want = want.clone();
                self.fetch = None;
                let json = (r.status == 200)
                    .then(|| crate::weather_data::parse(&r.body))
                    .flatten();
                let Some(json) = json else {
                    self.error = Some("The forecast service sent an answer Weather couldn't read.");
                    return true;
                };
                match want {
                    Want::Places(q) => {
                        match crate::weather_data::places(&json).into_iter().next() {
                            Some(p) => {
                                door_rt::diagnostic(format_args!(
                                    "DOOR_P15 WEATHER PLACE query={q} name={}\n",
                                    p.name
                                ));
                                self.place = Some(p);
                                self.searching = false;
                                self.query.clear();
                                self.page = Page::Forecast;
                                self.refresh();
                            }
                            None => self.error = Some("No places match that search."),
                        }
                    }
                    Want::Forecast => {
                        self.forecast = crate::weather_data::forecast(&json, self.celsius);
                        if let Some(f) = &self.forecast {
                            door_rt::diagnostic(format_args!(
                                "DOOR_P15 WEATHER FORECAST temperature={} code={} days={}\n",
                                f.current.temperature as i32,
                                f.current.code,
                                f.days.len()
                            ));
                        } else {
                            self.error =
                                Some("The forecast service sent an answer Weather couldn't read.");
                        }
                    }
                }
                true
            }
        }
    }
    pub fn new() -> Self {
        Self {
            width: 1000,
            height: 680,
            ready: false,
            page: Page::Forecast,
            expanded: false,
            celsius: false,
            theme: 2,
            query: String::new(),
            searching: false,
            favorites: Vec::new(),
            notice: None,
            retries: 0,
            registry: 0,
            place: None,
            forecast: None,
            fetch: None,
            error: None,
        }
    }
    fn rail_w(&self) -> u32 {
        if self.expanded { 240 } else { RAIL }
    }
    /// Rail rows that fit above the account/settings pair (UWP rails clip).
    fn rail_rows(&self) -> usize {
        let room = self.height as i32 - 96 - 1 - (TITLE + HEADER + 9);
        (room.max(0) / 48).min(Page::RAIL.len() as i32) as usize
    }
    fn rail_item(&self, i: usize) -> Rect {
        Rect::new(0, TITLE + HEADER + 9 + i as i32 * 48, self.rail_w(), 48)
    }
    fn bottom_item(&self, i: usize) -> Rect {
        Rect::new(
            0,
            self.height as i32 - 96 + i as i32 * 48,
            self.rail_w(),
            48,
        )
    }
    fn header_buttons(&self) -> [(Rect, u32); 4] {
        let s = self.search_box();
        let x = s.x - 270;
        [
            (Rect::new(x, TITLE + 4, 52, 40), id::REFRESH),
            (Rect::new(x + 68, TITLE + 4, 52, 40), id::FAVORITE),
            (Rect::new(x + 136, TITLE + 4, 52, 40), id::PIN),
            (Rect::new(x + 204, TITLE + 4, 52, 40), id::UNITS),
        ]
    }
    fn search_box(&self) -> Rect {
        let w = 300.min(self.width.saturating_sub(RAIL + 380));
        Rect::new(self.width as i32 - w as i32 - 6, TITLE + 6, w, 38)
    }
    fn content(&self) -> Rect {
        Rect::new(
            RAIL as i32,
            TITLE + HEADER,
            self.width - RAIL,
            (self.height as i32 - TITLE - HEADER).max(1) as u32,
        )
    }
    fn retry(&self) -> Rect {
        let c = self.content();
        Rect::new(
            c.x + c.w as i32 / 2 - 60,
            c.y + c.h as i32 / 2 + 70,
            120,
            32,
        )
    }
    fn options(&self) -> Vec<(Rect, u32)> {
        let c = self.content();
        let mut v = Vec::new();
        for i in 0..2 {
            v.push((
                Rect::new(c.x + 32, c.y + 84 + i * 32, 220, 32),
                id::OPTION + i as u32,
            ));
        }
        for i in 0..3 {
            v.push((
                Rect::new(c.x + 32, c.y + 196 + i * 32, 220, 32),
                id::OPTION + 2 + i as u32,
            ));
        }
        v
    }
    fn hits(&self) -> Vec<(Rect, u32)> {
        let mut v = vec![(Rect::new(0, TITLE, RAIL, HEADER as u32), id::MENU)];
        for i in 0..self.rail_rows() {
            v.push((self.rail_item(i), id::PAGE + i as u32));
        }
        v.push((self.bottom_item(0), id::ACCOUNT));
        v.push((self.bottom_item(1), id::SETTINGS));
        v.extend(self.header_buttons());
        v.push((self.search_box(), id::SEARCH));
        if self.page.online() {
            v.push((self.retry(), id::RETRY));
        }
        if self.page == Page::Settings {
            v.extend(self.options());
        }
        if self.page == Page::Favorites {
            let c = self.content();
            for i in 0..self.favorites.len().min(8) {
                v.push((
                    Rect::new(c.x + c.w as i32 - 140, c.y + 80 + i as i32 * 48, 100, 32),
                    id::REMOVE + i as u32,
                ));
            }
        }
        v
    }
    fn activate(&mut self, target: u32) -> bool {
        self.notice = None;
        match target {
            id::MENU => self.expanded = !self.expanded,
            t if (id::PAGE..id::PAGE + 9).contains(&t) => {
                self.page = Page::RAIL[(t - id::PAGE) as usize];
                self.expanded = false;
                self.searching = false;
                if self.page == Page::Feedback {
                    self.notice = Some("Feedback Hub isn't available on this device.");
                }
            }
            id::ACCOUNT => {
                self.page = Page::Account;
                self.expanded = false;
            }
            id::SETTINGS => {
                self.page = Page::Settings;
                self.expanded = false;
            }
            id::REFRESH | id::RETRY => {
                self.retries += 1;
                if self.place.is_some() {
                    self.refresh();
                } else if !self.query.trim().is_empty() {
                    let q = self.query.clone();
                    self.search(&q);
                } else {
                    self.searching = true;
                    self.notice = Some("Search for a location to see its forecast.");
                }
            }
            id::FAVORITE => match self.place.as_ref().map(crate::weather_data::Place::label) {
                Some(label) if !self.favorites.contains(&label) && self.favorites.len() < 8 => {
                    self.favorites.push(label);
                    self.notice = Some("Added to your favorite places.");
                }
                Some(_) => self.notice = Some("This place is already in your favorites."),
                None => self.notice = Some("Search for a location to add it to your favorites."),
            },
            id::PIN => {
                self.notice =
                    Some("Weather is already in Start. Pinning a place needs its forecast.")
            }
            id::UNITS => {
                self.celsius = !self.celsius;
                self.refresh();
            }
            id::SEARCH => self.searching = true,
            t if (id::OPTION..id::OPTION + 2).contains(&t) => {
                let celsius = t == id::OPTION + 1;
                if celsius != self.celsius {
                    self.celsius = celsius;
                    self.refresh();
                }
            }
            t if (id::OPTION + 2..id::OPTION + 5).contains(&t) => {
                self.theme = (t - id::OPTION - 2) as u8
            }
            t if (id::REMOVE..id::REMOVE + 8).contains(&t) => {
                let i = (t - id::REMOVE) as usize;
                if i < self.favorites.len() {
                    self.favorites.remove(i);
                }
            }
            _ => return false,
        }
        true
    }
    pub fn paint(&self, v: &mut View<'_>) {
        let dark = match self.theme {
            0 => false,
            1 => true,
            _ => v.palette.dark,
        };
        v.palette.dark = dark;
        let k = ink(dark);
        let (w, h) = (self.width, self.height);
        v.rect(0, 0, w, h, k.content);
        // Title bar in the accent colour (UWP).
        v.rect(0, 0, w, TITLE as u32, k.accent);
        v.text_clip_centered(
            12,
            (0, TITLE as u32),
            200,
            "Weather",
            TextStyle::Regular12,
            rgb(255, 255, 255),
        );
        // Header.
        v.rect(RAIL as i32, TITLE, w - RAIL, HEADER as u32, k.header);
        let title = if self.searching && !self.query.is_empty() {
            "Search results"
        } else {
            self.page.title()
        };
        v.text_clip_centered(
            RAIL as i32 + 23,
            (TITLE, HEADER as u32),
            360,
            title,
            TextStyle::Semibold16,
            k.text,
        );
        let hover = v.hover;
        let hovered = |r: Rect| hover.is_some_and(|(x, y)| r.contains(x, y));
        for (r, t) in self.header_buttons() {
            if hovered(r) {
                v.rect(r.x, r.y, r.w, r.h, k.hover);
            }
            let name = match t {
                id::REFRESH => "weather-refresh",
                id::FAVORITE => "weather-star",
                id::PIN => "weather-pin",
                _ => "weather-units",
            };
            crate::paint::view::glyph(v, name, r.x + 16, r.y + 10, 20, k.text);
        }
        let s = self.search_box();
        v.rect(s.x, s.y, s.w, s.h, k.field);
        if self.searching {
            v.rect(s.x, s.y + s.h as i32 - 2, s.w, 2, k.accent);
        }
        let shown = if self.query.is_empty() {
            "Search for location"
        } else {
            self.query.as_str()
        };
        v.text_clip_centered(
            s.x + 12,
            (s.y, s.h),
            s.w - 56,
            shown,
            TextStyle::Regular16,
            if self.query.is_empty() {
                k.secondary
            } else {
                k.text
            },
        );
        crate::paint::view::glyph(
            v,
            "weather-search",
            s.x + s.w as i32 - 34,
            s.y + 9,
            20,
            k.text,
        );
        // Content.
        self.paint_content(v, &k);
        // Rail (drawn last: expanded it overlays the content).
        let rw = self.rail_w();
        v.rect(0, TITLE, rw, h - TITLE as u32, k.rail);
        if hovered(Rect::new(0, TITLE, RAIL, HEADER as u32)) {
            v.rect(0, TITLE, RAIL, HEADER as u32, k.hover);
        }
        crate::paint::view::glyph(v, "weather-menu", 14, TITLE + 14, 20, k.text);
        let row = |v: &mut View<'_>, r: Rect, page: Page| {
            if hovered(r) {
                v.rect(r.x, r.y, r.w, r.h, k.hover);
            }
            if self.page == page {
                v.rect(r.x, r.y + 12, 4, 24, k.accent);
            }
            if page == Page::Account {
                // The signed-out avatar: a person in a grey disc.
                v.rounded_asset(
                    r.x + 12,
                    r.y + 12,
                    24,
                    24,
                    12,
                    if k.text[0] > 128 {
                        rgb(77, 77, 77)
                    } else {
                        rgb(204, 204, 204)
                    },
                );
                crate::paint::view::glyph(v, page.glyph(), r.x + 16, r.y + 16, 16, k.text);
            } else {
                crate::paint::view::glyph(v, page.glyph(), r.x + 16, r.y + 16, 16, k.text);
            }
            if self.expanded {
                v.text_clip_centered(
                    r.x + 48,
                    (r.y, r.h),
                    r.w - 56,
                    page.title(),
                    TextStyle::Regular14,
                    k.text,
                );
            }
        };
        for (i, p) in Page::RAIL.iter().enumerate().take(self.rail_rows()) {
            row(v, self.rail_item(i), *p);
        }
        v.rect(0, self.bottom_item(0).y - 1, rw, 1, k.divider);
        row(v, self.bottom_item(0), Page::Account);
        row(v, self.bottom_item(1), Page::Settings);
    }
    /// The Forecast page (MSN Weather): the place, the current conditions
    /// with their details, the next hours and the ten-day strip.
    fn paint_forecast(
        &self,
        v: &mut View<'_>,
        k: &Ink,
        f: &crate::weather_data::Forecast,
        p: &crate::weather_data::Place,
    ) {
        use crate::weather_data::{condition, day_label, glyph};
        let c = self.content();
        let x = c.x + 32;
        let unit = if f.celsius { "C" } else { "F" };
        let deg = |t: f64| format!("{}\u{b0}", rnd(t));
        v.text_clip(
            x,
            c.y + 20,
            c.w.saturating_sub(64),
            &p.label(),
            TextStyle::Semibold20,
            k.text,
        );
        if !p.country.is_empty() {
            v.text_clip(
                x,
                c.y + 50,
                400,
                &p.country,
                TextStyle::Regular14,
                k.secondary,
            );
        }
        let cur = &f.current;
        let top = c.y + 84;
        crate::paint::view::glyph(v, glyph(cur.code, cur.day), x, top, 96, k.text);
        v.text_clip(
            x + 112,
            top + 8,
            200,
            &format!("{}{unit}", deg(cur.temperature)),
            TextStyle::Semibold28,
            k.text,
        );
        v.text_clip(
            x + 112,
            top + 50,
            300,
            condition(cur.code, cur.day),
            TextStyle::Regular20,
            k.text,
        );
        v.text_clip(
            x + 112,
            top + 78,
            300,
            &format!("Feels like {}", deg(cur.feels)),
            TextStyle::Regular14,
            k.secondary,
        );
        // Details: wind, humidity, visibility, pressure.
        let (wind, vis, pressure) = if f.celsius {
            (
                format!("{} km/h", rnd(cur.wind)),
                format!("{} km", rnd(cur.visibility / 1000.0)),
                format!("{} mb", rnd(cur.pressure)),
            )
        } else {
            (
                format!("{} mph", rnd(cur.wind)),
                format!("{} mi", rnd(cur.visibility / 1609.34)),
                format!("{:.2} in", cur.pressure * 0.02953),
            )
        };
        let details = [
            ("weather-wind", "Wind", wind),
            (
                "weather-drop",
                "Humidity",
                format!("{}%", rnd(cur.humidity)),
            ),
            ("weather-fog", "Visibility", vis),
            ("weather-thermometer", "Pressure", pressure),
        ];
        let dx = x + 440;
        for (i, (g, label, value)) in details.iter().enumerate() {
            let y = top + 4 + i as i32 * 26;
            if dx + 260 > (c.x + c.w as i32) {
                break;
            }
            crate::paint::view::glyph(v, g, dx, y + 2, 16, k.secondary);
            v.text_clip(dx + 26, y, 110, label, TextStyle::Regular14, k.secondary);
            v.text_clip(dx + 140, y, 120, value, TextStyle::Semibold14, k.text);
        }
        // Next hours.
        let hy = top + 124;
        v.text_clip(x, hy, 300, "Hourly", TextStyle::Semibold14, k.text);
        let hours = ((c.w as i32 - 64) / 72).clamp(0, 12) as usize;
        for (i, (time, t, code)) in f.hours.iter().take(hours).enumerate() {
            let cx = x + i as i32 * 72;
            v.text_clip(cx, hy + 26, 64, time, TextStyle::Regular12, k.secondary);
            let hour: u32 = time.get(..2).and_then(|h| h.parse().ok()).unwrap_or(12);
            crate::paint::view::glyph(
                v,
                glyph(*code, (7..19).contains(&hour)),
                cx + 8,
                hy + 48,
                24,
                k.text,
            );
            v.text_clip(cx + 4, hy + 78, 64, &deg(*t), TextStyle::Regular14, k.text);
        }
        // Daily cards.
        let dy = hy + 112;
        v.text_clip(x, dy, 300, "Daily", TextStyle::Semibold14, k.text);
        let cards = ((c.w as i32 - 64) / 104).clamp(0, 10) as usize;
        for (i, d) in f.days.iter().take(cards).enumerate() {
            let r = Rect::new(x + i as i32 * 104, dy + 26, 96, 132);
            v.rounded_asset(
                r.x,
                r.y,
                r.w,
                r.h,
                6,
                if i == 0 { k.hover } else { k.field },
            );
            let label = if i == 0 {
                String::from("Today")
            } else {
                day_label(&d.date)
            };
            v.text_clip_centered(
                r.x + 8,
                (r.y + 4, 24),
                r.w - 16,
                &label,
                TextStyle::Regular14,
                k.text,
            );
            crate::paint::view::glyph(v, glyph(d.code, true), r.x + 28, r.y + 34, 40, k.text);
            v.text_clip(
                r.x + 12,
                r.y + 82,
                40,
                &deg(d.high),
                TextStyle::Semibold14,
                k.text,
            );
            v.text_clip(
                r.x + 52,
                r.y + 82,
                40,
                &deg(d.low),
                TextStyle::Regular14,
                k.secondary,
            );
            v.text_clip(
                r.x + 12,
                r.y + 106,
                80,
                &format!("{}%", rnd(d.rain)),
                TextStyle::Regular12,
                k.secondary,
            );
        }
        v.text_clip(
            x,
            c.y + c.h as i32 - 28,
            600,
            "Data: Open-Meteo.com (CC BY 4.0)",
            TextStyle::Regular12,
            k.secondary,
        );
    }
    fn paint_content(&self, v: &mut View<'_>, k: &Ink) {
        let c = self.content();
        let hover = v.hover;
        let hovered = |r: Rect| hover.is_some_and(|(x, y)| r.contains(x, y));
        let centre = |v: &mut View<'_>, y: i32, text: &str, style: TextStyle, color: [u8; 4]| {
            let tw = v.measure_style(text, style) as i32;
            v.text_clip(
                c.x + (c.w as i32 - tw) / 2,
                y,
                tw as u32 + 4,
                text,
                style,
                color,
            );
        };
        if let Some(n) = self.notice {
            let tw = v.measure_style(n, TextStyle::Regular14) + 32;
            let r = Rect::new(c.x + (c.w as i32 - tw as i32) / 2, c.y + 16, tw, 36);
            v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.field);
            v.text_clip_centered(
                r.x + 16,
                (r.y, r.h),
                r.w - 32,
                n,
                TextStyle::Regular14,
                k.text,
            );
        }
        if self.page == Page::Forecast && self.error.is_none() {
            if let (Some(f), Some(p)) = (&self.forecast, &self.place) {
                self.paint_forecast(v, k, f, p);
                return;
            }
            let cy = c.y + c.h as i32 / 2;
            if self.loading() {
                crate::paint::view::glyph(
                    v,
                    "weather-partly-day",
                    c.x + c.w as i32 / 2 - 48,
                    cy - 124,
                    96,
                    k.secondary,
                );
                centre(
                    v,
                    cy - 10,
                    "Getting the forecast\u{2026}",
                    TextStyle::Semibold20,
                    k.text,
                );
            } else {
                crate::paint::view::glyph(
                    v,
                    "weather-search",
                    c.x + c.w as i32 / 2 - 24,
                    cy - 76,
                    48,
                    k.secondary,
                );
                centre(
                    v,
                    cy - 10,
                    "Search for a location",
                    TextStyle::Semibold20,
                    k.text,
                );
                centre(
                    v,
                    cy + 24,
                    "Type a city in the search box, then press Enter.",
                    TextStyle::Regular14,
                    k.secondary,
                );
            }
            return;
        }
        if self.page.online() || self.error.is_some() {
            // No forecast could be fetched: say why, never invent weather.
            let cy = c.y + c.h as i32 / 2;
            crate::paint::view::glyph(
                v,
                "weather-cloud",
                c.x + c.w as i32 / 2 - 48,
                cy - 124,
                96,
                k.secondary,
            );
            let heading = match self.error {
                Some("You're offline.") | None => "You're offline",
                Some(_) => "Something went wrong",
            };
            centre(v, cy - 10, heading, TextStyle::Semibold20, k.text);
            let line = match self.error {
                Some(e) if e != "You're offline." => String::from(e),
                _ if self.page != Page::Forecast => format!(
                    "{} isn't available in Weather for Doors yet.",
                    self.page.title()
                ),
                _ => String::from("Weather needs an internet connection to show the forecast."),
            };
            centre(v, cy + 24, &line, TextStyle::Regular14, k.secondary);
            centre(
                v,
                cy + 44,
                "Connect to the internet, then try again.",
                TextStyle::Regular14,
                k.secondary,
            );
            let r = self.retry();
            let pk = crate::paint::view::ink(k.text[0] > 128);
            button(v, &pk, r, "Try again", true, hovered(r));
            return;
        }
        let x = c.x + 32;
        match self.page {
            Page::Settings => {
                v.text_clip(x, c.y + 24, 400, "Settings", TextStyle::Semibold20, k.text);
                v.text_clip(
                    x,
                    c.y + 62,
                    300,
                    "Show temperature in",
                    TextStyle::Semibold14,
                    k.text,
                );
                v.text_clip(x, c.y + 174, 300, "Mode", TextStyle::Semibold14, k.text);
                let labels = ["Fahrenheit", "Celsius", "Light", "Dark", "Use Doors mode"];
                for (i, (r, _)) in self.options().iter().enumerate() {
                    let on = if i < 2 {
                        self.celsius == (i == 1)
                    } else {
                        self.theme as usize == i - 2
                    };
                    let cy = r.y + r.h as i32 / 2;
                    v.rounded_asset(
                        r.x,
                        cy - 10,
                        20,
                        20,
                        10,
                        if on { k.accent } else { k.secondary },
                    );
                    v.rounded_asset(r.x + 2, cy - 8, 16, 16, 8, k.content);
                    if on {
                        v.rounded_asset(r.x + 5, cy - 5, 10, 10, 5, k.accent);
                    }
                    v.text_clip_centered(
                        r.x + 30,
                        (r.y, r.h),
                        r.w - 30,
                        labels[i],
                        TextStyle::Regular14,
                        k.text,
                    );
                }
                v.text_clip(x, c.y + 300, 600, "About", TextStyle::Semibold14, k.text);
                v.text_clip(
                    x,
                    c.y + 326,
                    600,
                    "Weather for Doors. Forecasts need an internet connection.",
                    TextStyle::Regular14,
                    k.secondary,
                );
            }
            Page::Favorites => {
                v.text_clip(
                    x,
                    c.y + 24,
                    400,
                    "Favorite Places",
                    TextStyle::Semibold20,
                    k.text,
                );
                if self.favorites.is_empty() {
                    v.text_clip(
                        x,
                        c.y + 70,
                        700,
                        "You haven't added any favorite places yet.",
                        TextStyle::Regular14,
                        k.secondary,
                    );
                }
                for (i, f) in self.favorites.iter().take(8).enumerate() {
                    let y = c.y + 80 + i as i32 * 48;
                    v.text_clip_centered(x, (y, 32), 400, f, TextStyle::Regular14, k.text);
                    let r = Rect::new(c.x + c.w as i32 - 140, y, 100, 32);
                    let pk = crate::paint::view::ink(k.text[0] > 128);
                    button(v, &pk, r, "Remove", false, hovered(r));
                }
            }
            Page::Account => {
                v.text_clip(x, c.y + 24, 400, "Account", TextStyle::Semibold20, k.text);
                v.text_clip(
                    x,
                    c.y + 70,
                    700,
                    "You're using Weather with your Doors account. No Macrohard account is needed.",
                    TextStyle::Regular14,
                    k.secondary,
                );
            }
            _ => {
                v.text_clip(
                    x,
                    c.y + 24,
                    400,
                    "Send Feedback",
                    TextStyle::Semibold20,
                    k.text,
                );
                v.text_clip(
                    x,
                    c.y + 70,
                    700,
                    "Feedback Hub isn't available on this device.",
                    TextStyle::Regular14,
                    k.secondary,
                );
            }
        }
    }
}

impl Application for Weather {
    fn title(&self) -> &str {
        "Weather"
    }
    fn options(&self) -> door_workspace::runtime::WindowOptions {
        door_workspace::runtime::WindowOptions {
            width: 1000,
            height: 680,
            caption_height: TITLE as u32,
            caption_left: 120,
            centered: true,
        }
    }
    fn caption(&self) -> (u32, u32) {
        (120, 1)
    }
    fn configure(&mut self, width: u32, height: u32) {
        self.width = width.max(500);
        self.height = height.max(400);
        if !self.ready {
            self.ready = true;
            door_rt::diagnostic(format_args!("DOOR_INBOX READY app=weather\n"));
        }
    }
    fn draw(&self, v: &mut View<'_>) {
        self.paint(v);
    }
    fn target(&self, x: i32, y: i32) -> Option<u32> {
        self.hits()
            .into_iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, t)| t)
    }
    fn click(&mut self, id: u32, _s: &mut Storage) -> Result<bool, Error> {
        Ok(self.activate(id))
    }
    fn key(&mut self, key: u32, _s: &mut Storage) -> Result<bool, Error> {
        let scan = key & 511;
        let text = (key >> 16) as u8;
        if !self.searching {
            return Ok(false);
        }
        match scan {
            1 => {
                self.searching = false;
                self.query.clear();
            }
            14 => {
                self.query.pop();
            }
            28 | 0x11c => {
                if self.page == Page::Favorites {
                    if !self.query.trim().is_empty()
                        && !self.favorites.contains(&self.query)
                        && self.favorites.len() < 8
                    {
                        self.favorites.push(self.query.clone());
                    }
                } else {
                    let q = self.query.clone();
                    self.search(&q);
                }
            }
            _ if (32..127).contains(&text) && self.query.len() < 48 => {
                self.query.push(text as char)
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn text_hit(&self, x: i32, y: i32) -> bool {
        self.search_box().contains(x, y)
    }
    fn wheel(&mut self, _x: i32, _y: i32, _n: i32, _f: &FontAtlas) -> bool {
        false
    }
    fn tick(&mut self, _s: &mut Storage, _now: u64) -> Result<bool, Error> {
        Ok(self.poll() || self.loading())
    }
}

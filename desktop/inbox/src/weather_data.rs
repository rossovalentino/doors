//! Weather's forecast data (P15): Open-Meteo's free geocoding and forecast
//! APIs over HTTPS (`door_workspace::https`), a small JSON reader, and the
//! WMO weather codes as the condition names MSN Weather shows.
use alloc::{string::String, vec::Vec};

/// A JSON value (numbers as f64).
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}
impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Self::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn num(&self) -> Option<f64> {
        match self {
            Self::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn arr(&self) -> &[Json] {
        match self {
            Self::Arr(a) => a,
            _ => &[],
        }
    }
}
/// Parse JSON text (depth-limited).
pub fn parse(text: &[u8]) -> Option<Json> {
    let mut p = Parser { b: text, at: 0 };
    let v = p.value(0)?;
    p.ws();
    (p.at == p.b.len()).then_some(v)
}
struct Parser<'a> {
    b: &'a [u8],
    at: usize,
}
impl Parser<'_> {
    fn ws(&mut self) {
        while self.b.get(self.at).is_some_and(|c| c.is_ascii_whitespace()) {
            self.at += 1;
        }
    }
    fn eat(&mut self, s: &[u8]) -> bool {
        if self.b[self.at..].starts_with(s) {
            self.at += s.len();
            true
        } else {
            false
        }
    }
    fn value(&mut self, depth: u32) -> Option<Json> {
        if depth > 32 {
            return None;
        }
        self.ws();
        match *self.b.get(self.at)? {
            b'{' => {
                self.at += 1;
                let mut fields = Vec::new();
                self.ws();
                if self.eat(b"}") {
                    return Some(Json::Obj(fields));
                }
                loop {
                    self.ws();
                    let Json::Str(k) = self.string()? else {
                        return None;
                    };
                    self.ws();
                    if !self.eat(b":") {
                        return None;
                    }
                    fields.push((k, self.value(depth + 1)?));
                    self.ws();
                    if self.eat(b"}") {
                        return Some(Json::Obj(fields));
                    }
                    if !self.eat(b",") {
                        return None;
                    }
                }
            }
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                self.ws();
                if self.eat(b"]") {
                    return Some(Json::Arr(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    if self.eat(b"]") {
                        return Some(Json::Arr(items));
                    }
                    if !self.eat(b",") {
                        return None;
                    }
                }
            }
            b'"' => self.string(),
            b't' => self.eat(b"true").then_some(Json::Bool(true)),
            b'f' => self.eat(b"false").then_some(Json::Bool(false)),
            b'n' => self.eat(b"null").then_some(Json::Null),
            _ => self.number(),
        }
    }
    fn string(&mut self) -> Option<Json> {
        if !self.eat(b"\"") {
            return None;
        }
        let mut out = String::new();
        loop {
            let c = *self.b.get(self.at)?;
            self.at += 1;
            match c {
                b'"' => return Some(Json::Str(out)),
                b'\\' => {
                    let e = *self.b.get(self.at)?;
                    self.at += 1;
                    match e {
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'b' | b'f' => (),
                        b'u' => {
                            let hex =
                                core::str::from_utf8(self.b.get(self.at..self.at + 4)?).ok()?;
                            self.at += 4;
                            let unit = u32::from_str_radix(hex, 16).ok()?;
                            out.push(char::from_u32(unit).unwrap_or('\u{FFFD}'));
                        }
                        other => out.push(other as char),
                    }
                }
                _ => {
                    // Copy the UTF-8 sequence that starts here.
                    let start = self.at - 1;
                    let len = match c {
                        0x00..=0x7F => 1,
                        0xC0..=0xDF => 2,
                        0xE0..=0xEF => 3,
                        _ => 4,
                    };
                    self.at = start + len;
                    out.push_str(core::str::from_utf8(self.b.get(start..start + len)?).ok()?);
                }
            }
        }
    }
    fn number(&mut self) -> Option<Json> {
        let start = self.at;
        while self
            .b
            .get(self.at)
            .is_some_and(|c| matches!(c, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'))
        {
            self.at += 1;
        }
        let s = core::str::from_utf8(&self.b[start..self.at]).ok()?;
        s.parse::<f64>().ok().map(Json::Num)
    }
}

/// A place from the geocoding API.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub name: String,
    pub region: String,
    pub country: String,
    pub latitude: f64,
    pub longitude: f64,
}
impl Place {
    pub fn label(&self) -> String {
        match (self.region.is_empty(), self.country.is_empty()) {
            (false, _) => alloc::format!("{}, {}", self.name, self.region),
            (true, false) => alloc::format!("{}, {}", self.name, self.country),
            _ => self.name.clone(),
        }
    }
}
fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.trim().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' => out.push(b as char),
            b' ' => out.push_str("%20"),
            _ => out.push_str(&alloc::format!("%{b:02X}")),
        }
    }
    out
}
pub fn geocode_url(query: &str) -> String {
    alloc::format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=5&language=en&format=json",
        encode(query)
    )
}
pub fn places(json: &Json) -> Vec<Place> {
    json.get("results")
        .map(Json::arr)
        .unwrap_or(&[])
        .iter()
        .filter_map(|r| {
            Some(Place {
                name: r.get("name")?.str()?.into(),
                region: r.get("admin1").and_then(Json::str).unwrap_or("").into(),
                country: r.get("country").and_then(Json::str).unwrap_or("").into(),
                latitude: r.get("latitude")?.num()?,
                longitude: r.get("longitude")?.num()?,
            })
        })
        .collect()
}
pub fn forecast_url(p: &Place, celsius: bool) -> String {
    alloc::format!(
        "https://api.open-meteo.com/v1/forecast?latitude={:.4}&longitude={:.4}\
&current=temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m,wind_direction_10m,pressure_msl,visibility,is_day\
&hourly=temperature_2m,weather_code&daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max\
&timezone=auto&forecast_days=10&forecast_hours=24&temperature_unit={}&wind_speed_unit={}",
        p.latitude,
        p.longitude,
        if celsius { "celsius" } else { "fahrenheit" },
        if celsius { "kmh" } else { "mph" }
    )
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Current {
    pub temperature: f64,
    pub feels: f64,
    pub humidity: f64,
    pub code: u8,
    pub wind: f64,
    pub wind_direction: f64,
    pub pressure: f64,
    pub visibility: f64,
    pub day: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    /// "2026-11-02"
    pub date: String,
    pub code: u8,
    pub high: f64,
    pub low: f64,
    pub rain: f64,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Forecast {
    pub current: Current,
    pub days: Vec<Day>,
    /// Next hours: ("14:00", temperature, code).
    pub hours: Vec<(String, f64, u8)>,
    pub celsius: bool,
}
pub fn forecast(json: &Json, celsius: bool) -> Option<Forecast> {
    let c = json.get("current")?;
    let n = |o: &Json, k: &str| o.get(k).and_then(Json::num).unwrap_or(0.0);
    let current = Current {
        temperature: c.get("temperature_2m")?.num()?,
        feels: n(c, "apparent_temperature"),
        humidity: n(c, "relative_humidity_2m"),
        code: n(c, "weather_code") as u8,
        wind: n(c, "wind_speed_10m"),
        wind_direction: n(c, "wind_direction_10m"),
        pressure: n(c, "pressure_msl"),
        visibility: n(c, "visibility"),
        day: n(c, "is_day") != 0.0,
    };
    let d = json.get("daily")?;
    let col = |o: &Json, k: &str, i: usize| {
        o.get(k)
            .map(Json::arr)
            .and_then(|a| a.get(i))
            .and_then(Json::num)
            .unwrap_or(0.0)
    };
    let dates = d.get("time").map(Json::arr).unwrap_or(&[]);
    let days = dates
        .iter()
        .enumerate()
        .map(|(i, t)| Day {
            date: t.str().unwrap_or("").into(),
            code: col(d, "weather_code", i) as u8,
            high: col(d, "temperature_2m_max", i),
            low: col(d, "temperature_2m_min", i),
            rain: col(d, "precipitation_probability_max", i),
        })
        .collect();
    let h = json.get("hourly");
    let hours = h
        .and_then(|h| h.get("time"))
        .map(Json::arr)
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let time = t.str().unwrap_or("");
            let hh = time.get(11..16).unwrap_or(time);
            (
                String::from(hh),
                h.map_or(0.0, |h| col(h, "temperature_2m", i)),
                h.map_or(0, |h| col(h, "weather_code", i) as u8),
            )
        })
        .collect();
    Some(Forecast {
        current,
        days,
        hours,
        celsius,
    })
}
/// MSN Weather's condition name for a WMO code.
pub fn condition(code: u8, day: bool) -> &'static str {
    match code {
        0 if day => "Sunny",
        0 => "Clear",
        1 if day => "Mostly sunny",
        1 => "Mostly clear",
        2 if day => "Partly sunny",
        2 => "Partly cloudy",
        3 => "Cloudy",
        45 | 48 => "Fog",
        51 | 53 | 55 => "Drizzle",
        56 | 57 => "Freezing drizzle",
        61 => "Light rain",
        63 => "Rain",
        65 => "Heavy rain",
        66 | 67 => "Freezing rain",
        71 => "Light snow",
        73 => "Snow",
        75 => "Heavy snow",
        77 => "Snow grains",
        80..=82 => "Rain showers",
        85 | 86 => "Snow showers",
        95 => "Thunderstorms",
        96 | 99 => "Thunderstorms with hail",
        _ => "Unknown",
    }
}
/// The MSN Weather icon for a WMO code.
pub fn glyph(code: u8, day: bool) -> &'static str {
    match code {
        0 if day => "weather-sunny",
        0 => "weather-clear-night",
        1 | 2 if day => "weather-partly-day",
        1 | 2 => "weather-partly-night",
        3 => "weather-cloudy",
        45 | 48 => "weather-fog",
        51..=57 => "weather-drizzle",
        61..=67 => "weather-rain",
        71..=77 => "weather-snow",
        80..=82 if day => "weather-showers-day",
        80..=82 => "weather-showers-night",
        85 | 86 => "weather-snow-showers",
        95..=99 => "weather-thunder",
        _ => "weather-cloud",
    }
}
/// "Mon 2" style day label from "YYYY-MM-DD".
pub fn day_label(date: &str) -> String {
    let y: i32 = date.get(..4).and_then(|s| s.parse().ok()).unwrap_or(1970);
    let m: u8 = date.get(5..7).and_then(|s| s.parse().ok()).unwrap_or(1);
    let d: u8 = date.get(8..10).and_then(|s| s.parse().ok()).unwrap_or(1);
    let days = door_abi::clock::days_from_civil(y as u16, m, d);
    // Day 0 is 1601-01-01, a Monday.
    const NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    alloc::format!("{} {}", NAMES[(days % 7) as usize], d)
}

#[cfg(test)]
mod tests {
    use super::*;
    const SAMPLE: &str = r#"{"latitude":45.46,"current":{"time":"2026-11-02T14:00","temperature_2m":12.4,"apparent_temperature":10.9,
"relative_humidity_2m":71,"weather_code":2,"wind_speed_10m":8.3,"wind_direction_10m":240,"pressure_msl":1018.2,"visibility":24140.0,"is_day":1},
"hourly":{"time":["2026-11-02T14:00","2026-11-02T15:00"],"temperature_2m":[12.4,12.9],"weather_code":[2,3]},
"daily":{"time":["2026-11-02","2026-11-03"],"weather_code":[2,61],"temperature_2m_max":[14.1,11.0],"temperature_2m_min":[7.5,6.2],"precipitation_probability_max":[10,80]}}"#;
    #[test]
    fn parses_forecasts_and_places() {
        let f = forecast(&parse(SAMPLE.as_bytes()).unwrap(), true).unwrap();
        assert_eq!(
            (f.current.temperature, f.current.code, f.current.day),
            (12.4, 2, true)
        );
        assert_eq!(f.days.len(), 2);
        assert_eq!(f.days[1].code, 61);
        assert_eq!(f.hours[1], (String::from("15:00"), 12.9, 3));
        assert_eq!(condition(2, true), "Partly sunny");
        assert_eq!(day_label("2026-11-02"), "Mon 2");
        let g = parse(br#"{"results":[{"name":"Milan","latitude":45.46,"longitude":9.19,"country":"Italy","admin1":"Lombardy"}]}"#).unwrap();
        let p = places(&g);
        assert_eq!(p[0].label(), "Milan, Lombardy");
        assert!(geocode_url("San José").contains("San%20Jos%C3%A9"));
        assert!(parse(b"{\"a\":}").is_none());
        assert_eq!(
            parse(b"\"\\u00e9t\\u00e9\"").unwrap(),
            Json::Str("\u{e9}t\u{e9}".into())
        );
    }
}

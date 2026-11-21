//! A package's dialogs as Windows Installer lays them out: the Dialog,
//! Control, ControlEvent, ControlCondition, EventMapping, TextStyle,
//! RadioButton and UIText tables read into a model the UI draws. Positions
//! are installer units: at 96 DPI a unit is 4/3 of a pixel (a 370 x 270
//! dialog is 493 x 360 pixels of client area).
use alloc::{string::String, vec::Vec};
use door_msi::{Database, Table};

pub fn pixels(units: i32) -> i32 {
    (units * 4 + 1) / 3
}

/// Dialog attribute bits.
pub mod dialog_attr {
    pub const VISIBLE: i32 = 0x1;
    pub const MODAL: i32 = 0x2;
    pub const MINIMIZE: i32 = 0x4;
    pub const KEEP_MODELESS: i32 = 0x10;
    pub const TRACK_DISK_SPACE: i32 = 0x20;
    pub const ERROR: i32 = 0x10000;
}
/// Control attribute bits.
pub mod control_attr {
    pub const VISIBLE: i32 = 0x1;
    pub const ENABLED: i32 = 0x2;
    pub const SUNKEN: i32 = 0x4;
    pub const INDIRECT: i32 = 0x8;
    pub const INTEGER: i32 = 0x10;
    pub const TRANSPARENT: i32 = 0x10000;
    pub const NO_PREFIX: i32 = 0x20000;
    pub const NO_WRAP: i32 = 0x40000;
    pub const PROGRESS95: i32 = 0x10000;
    pub const PASSWORD: i32 = 0x200000;
    pub const MULTILINE: i32 = 0x10000;
    pub const PUSH_LIKE: i32 = 0x20000;
    pub const BITMAP: i32 = 0x40000;
    pub const ICON: i32 = 0x80000;
    pub const ELEVATION_SHIELD: i32 = 0x800000;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextStyle {
    pub name: String,
    pub face: String,
    pub size: i32,
    /// 0x00BBGGRR, or None for the system color.
    pub color: Option<u32>,
    /// Bold 1, italic 2, underline 4, strike 8.
    pub bits: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Event {
    pub event: String,
    pub argument: String,
    pub condition: String,
    pub ordering: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Control {
    pub name: String,
    pub kind: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub attributes: i32,
    pub property: String,
    pub text: String,
    pub next: String,
    pub help: String,
    pub events: Vec<Event>,
    /// (Action: Show, Hide, Enable, Disable, Default; condition).
    pub conditions: Vec<(String, String)>,
    /// EventMapping: (event, attribute) the control subscribes to.
    pub subscriptions: Vec<(String, String)>,
    /// RadioButtonGroup: (value, x, y, width, height, text).
    pub radio: Vec<(String, i32, i32, i32, i32, String)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dialog {
    pub name: String,
    /// Horizontal and vertical centering, 0..100.
    pub h_center: i32,
    pub v_center: i32,
    pub width: i32,
    pub height: i32,
    pub attributes: i32,
    pub title: String,
    pub first: String,
    pub default: String,
    pub cancel: String,
    pub controls: Vec<Control>,
}
impl Dialog {
    pub fn control(&self, name: &str) -> Option<&Control> {
        self.controls.iter().find(|c| c.name == name)
    }
    pub fn modal(&self) -> bool {
        self.attributes & dialog_attr::MODAL != 0
    }
}

fn opt(db: &Database, name: &str) -> Option<Table> {
    db.table(name).ok()
}
fn s(t: &Table, r: usize, c: &str) -> String {
    String::from(t.get(r, c).str().unwrap_or(""))
}
fn i(t: &Table, r: usize, c: &str) -> i32 {
    t.get(r, c).int().unwrap_or(0)
}

/// The dialog `name` with its controls, events, conditions and
/// subscriptions.
pub fn dialog(db: &Database, name: &str) -> Option<Dialog> {
    let t = opt(db, "Dialog")?;
    let r = t.find(name)?;
    let mut d = Dialog {
        name: String::from(name),
        h_center: i(&t, r, "HCentering"),
        v_center: i(&t, r, "VCentering"),
        width: i(&t, r, "Width"),
        height: i(&t, r, "Height"),
        attributes: i(&t, r, "Attributes"),
        title: s(&t, r, "Title"),
        first: s(&t, r, "Control_First"),
        default: s(&t, r, "Control_Default"),
        cancel: s(&t, r, "Control_Cancel"),
        controls: Vec::new(),
    };
    if let Some(c) = opt(db, "Control") {
        for r in 0..c.rows.len() {
            if c.get(r, "Dialog_").str() != Some(name) {
                continue;
            }
            d.controls.push(Control {
                name: s(&c, r, "Control"),
                kind: s(&c, r, "Type"),
                x: i(&c, r, "X"),
                y: i(&c, r, "Y"),
                width: i(&c, r, "Width"),
                height: i(&c, r, "Height"),
                attributes: i(&c, r, "Attributes"),
                property: s(&c, r, "Property"),
                text: s(&c, r, "Text"),
                next: s(&c, r, "Control_Next"),
                help: s(&c, r, "Help"),
                ..Control::default()
            });
        }
    }
    if let Some(e) = opt(db, "ControlEvent") {
        for r in 0..e.rows.len() {
            if e.get(r, "Dialog_").str() != Some(name) {
                continue;
            }
            let control = s(&e, r, "Control_");
            if let Some(c) = d.controls.iter_mut().find(|c| c.name == control) {
                c.events.push(Event {
                    event: s(&e, r, "Event"),
                    argument: s(&e, r, "Argument"),
                    condition: s(&e, r, "Condition"),
                    ordering: i(&e, r, "Ordering"),
                });
            }
        }
        for c in &mut d.controls {
            c.events.sort_by_key(|e| e.ordering);
        }
    }
    if let Some(e) = opt(db, "ControlCondition") {
        for r in 0..e.rows.len() {
            if e.get(r, "Dialog_").str() != Some(name) {
                continue;
            }
            let control = s(&e, r, "Control_");
            if let Some(c) = d.controls.iter_mut().find(|c| c.name == control) {
                c.conditions
                    .push((s(&e, r, "Action"), s(&e, r, "Condition")));
            }
        }
    }
    if let Some(e) = opt(db, "EventMapping") {
        for r in 0..e.rows.len() {
            if e.get(r, "Dialog_").str() != Some(name) {
                continue;
            }
            let control = s(&e, r, "Control_");
            if let Some(c) = d.controls.iter_mut().find(|c| c.name == control) {
                c.subscriptions
                    .push((s(&e, r, "Event"), s(&e, r, "Attribute")));
            }
        }
    }
    if let Some(rb) = opt(db, "RadioButton") {
        for c in &mut d.controls {
            if c.kind != "RadioButtonGroup" {
                continue;
            }
            #[allow(clippy::type_complexity)]
            let mut list: Vec<(i32, (String, i32, i32, i32, i32, String))> = (0..rb.rows.len())
                .filter(|&r| rb.get(r, "Property").str() == Some(c.property.as_str()))
                .map(|r| {
                    (
                        i(&rb, r, "Order"),
                        (
                            s(&rb, r, "Value"),
                            i(&rb, r, "X"),
                            i(&rb, r, "Y"),
                            i(&rb, r, "Width"),
                            i(&rb, r, "Height"),
                            s(&rb, r, "Text"),
                        ),
                    )
                })
                .collect();
            list.sort_by_key(|x| x.0);
            c.radio = list.into_iter().map(|x| x.1).collect();
        }
    }
    Some(d)
}

/// The TextStyle table.
pub fn text_styles(db: &Database) -> Vec<TextStyle> {
    let Some(t) = opt(db, "TextStyle") else {
        return Vec::new();
    };
    (0..t.rows.len())
        .map(|r| TextStyle {
            name: s(&t, r, "TextStyle"),
            face: s(&t, r, "FaceName"),
            size: i(&t, r, "Size"),
            color: t.get(r, "Color").int().map(|c| c as u32),
            bits: i(&t, r, "StyleBits"),
        })
        .collect()
}

/// A control text's leading style references (`{\Style}` or `{&Style}`)
/// and the text after them.
pub fn split_style(text: &str) -> (Option<String>, &str) {
    let mut rest = text;
    let mut style = None;
    while let Some(after) = rest.strip_prefix("{\\").or_else(|| rest.strip_prefix("{&")) {
        let Some(end) = after.find('}') else { break };
        style = Some(String::from(&after[..end]));
        rest = &after[end + 1..];
    }
    (style, rest)
}

/// A UIText table entry (Windows Installer's strings: "bytes", "KB",
/// "Absent", ...).
pub fn ui_text(db: &Database, key: &str) -> Option<String> {
    let t = opt(db, "UIText")?;
    let r = t.find(key)?;
    t.get(r, "Text").str().map(String::from)
}

/// The plain text of an RTF document (a ScrollableText control's): groups
/// that are destinations (`{\*...}`, font and color tables) are skipped,
/// `\par` breaks lines, `\'hh` is a Windows-1252 byte, `\tab` a tab.
pub fn rtf_text(rtf: &str) -> String {
    let b: Vec<char> = rtf.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    // Depths at which a skipped destination began.
    let mut skip: Vec<usize> = Vec::new();
    let mut depth = 0usize;
    while i < b.len() {
        let c = b[i];
        match c {
            '{' => {
                depth += 1;
                i += 1;
                // {\* ...}, {\fonttbl ...}, {\colortbl ...}, {\stylesheet ...}, {\info ...}
                let rest: String = b[i..(i + 12).min(b.len())].iter().collect();
                if rest.starts_with("\\*")
                    || rest.starts_with("\\fonttbl")
                    || rest.starts_with("\\colortbl")
                    || rest.starts_with("\\stylesheet")
                    || rest.starts_with("\\info")
                    || rest.starts_with("\\pict")
                {
                    skip.push(depth);
                }
            }
            '}' => {
                if skip.last() == Some(&depth) {
                    skip.pop();
                }
                depth = depth.saturating_sub(1);
                i += 1;
            }
            '\\' => {
                i += 1;
                let Some(&n) = b.get(i) else { break };
                if n == '\'' {
                    let hex: String = b[i + 1..(i + 3).min(b.len())].iter().collect();
                    if skip.is_empty()
                        && let Ok(v) = u8::from_str_radix(&hex, 16)
                    {
                        out.push(char::from(v));
                    }
                    i += 3;
                    continue;
                }
                if !n.is_ascii_alphabetic() {
                    if skip.is_empty() && matches!(n, '\\' | '{' | '}') {
                        out.push(n);
                    }
                    i += 1;
                    continue;
                }
                let start = i;
                while i < b.len() && b[i].is_ascii_alphabetic() {
                    i += 1;
                }
                let word: String = b[start..i].iter().collect();
                if i < b.len() && (b[i] == '-' || b[i].is_ascii_digit()) {
                    i += 1;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                if i < b.len() && b[i] == ' ' {
                    i += 1;
                }
                if skip.is_empty() {
                    match word.as_str() {
                        "par" | "line" => out.push_str("\r\n"),
                        "tab" => out.push('\t'),
                        "bullet" => out.push('\u{2022}'),
                        "emdash" => out.push('\u{2014}'),
                        "endash" => out.push('\u{2013}'),
                        "lquote" => out.push('\u{2018}'),
                        "rquote" => out.push('\u{2019}'),
                        "ldblquote" => out.push('\u{201c}'),
                        "rdblquote" => out.push('\u{201d}'),
                        _ => (),
                    }
                }
            }
            '\r' | '\n' => i += 1,
            _ => {
                if skip.is_empty() {
                    out.push(c);
                }
                i += 1;
            }
        }
    }
    String::from(out.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn styles_split_and_rtf_reads_as_text() {
        assert_eq!(
            split_style("{\\Title}Welcome"),
            (Some(String::from("Title")), "Welcome")
        );
        assert_eq!(
            split_style("{&Bold}{\\Big}x"),
            (Some(String::from("Big")), "x")
        );
        assert_eq!(split_style("plain"), (None, "plain"));
        let rtf = "{\\rtf1\\ansi{\\fonttbl{\\f0 Segoe UI;}}{\\colortbl;\\red0\\green0\\blue0;}\\f0\\fs20 Terms\\par\\pard Caf\\'e9 \\{ok\\}\\par}";
        assert_eq!(rtf_text(rtf), "Terms\r\nCaf\u{e9} {ok}");
        assert_eq!(pixels(370), 493);
        assert_eq!(pixels(270), 360);
    }
}

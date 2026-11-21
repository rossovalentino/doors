//! Doorsh values: commands pass structured data down a pipeline (strings,
//! numbers, sizes, durations, lists, records and tables) and the console
//! renders whatever reaches its end. Text crosses to external programs as
//! lines.
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::fmt::Write;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// Bytes.
    Size(u64),
    /// Milliseconds.
    Duration(u64),
    List(Vec<Value>),
    /// Ordered fields.
    Record(Vec<(String, Value)>),
}
impl Value {
    pub fn str(s: &str) -> Self {
        Self::Str(String::from(s))
    }
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "bool",
            Self::Int(_) => "int",
            Self::Float(_) => "float",
            Self::Str(_) => "string",
            Self::Size(_) => "size",
            Self::Duration(_) => "duration",
            Self::List(l) if l.iter().all(|v| matches!(v, Value::Record(_))) && !l.is_empty() => {
                "table"
            }
            Self::List(_) => "list",
            Self::Record(_) => "record",
        }
    }
    pub fn truthy(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(b) => *b,
            Self::Int(i) => *i != 0,
            Self::Float(f) => *f != 0.0,
            Self::Str(s) => !s.is_empty(),
            Self::Size(s) => *s != 0,
            Self::Duration(d) => *d != 0,
            Self::List(l) => !l.is_empty(),
            Self::Record(r) => !r.is_empty(),
        }
    }
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            Self::Float(f) => Some(*f as i64),
            Self::Size(s) => i64::try_from(*s).ok(),
            Self::Duration(d) => i64::try_from(*d).ok(),
            Self::Bool(b) => Some(i64::from(*b)),
            Self::Str(s) => parse_number(s.trim()).and_then(|v| v.as_int()),
            _ => None,
        }
    }
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            Self::Str(s) => parse_number(s.trim()).and_then(|v| v.as_float()),
            other => other.as_int().map(|i| i as f64),
        }
    }
    pub fn field(&self, name: &str) -> Option<&Value> {
        match self {
            Self::Record(fields) => fields
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v),
            _ => None,
        }
    }
    /// Items for `for`, `each` and list commands: a list's items, a
    /// string's lines, anything else alone.
    pub fn items(self) -> Vec<Value> {
        match self {
            Self::List(l) => l,
            Self::Null => Vec::new(),
            Self::Str(s) => s.lines().map(Value::str).collect(),
            other => alloc::vec![other],
        }
    }
    /// Plain text: what an external program or a file receives.
    pub fn to_text(&self) -> String {
        match self {
            Self::Null => String::new(),
            Self::Str(s) => s.clone(),
            Self::List(l) if !self.is_table() => {
                let mut out = String::new();
                for v in l {
                    out.push_str(&v.to_text());
                    out.push('\n');
                }
                out
            }
            other => render(other, usize::MAX),
        }
    }
    pub fn is_table(&self) -> bool {
        matches!(self, Self::List(l) if !l.is_empty() && l.iter().all(|v| matches!(v, Value::Record(_))))
    }
}
/// A value written back as Doorsh source (`where`, `calc` and `time`
/// re-read their arguments): sizes and durations keep their units.
pub fn source(v: &Value) -> String {
    match v {
        Value::Size(b) => format!("{b}b"),
        Value::Duration(ms) => format!("{ms}ms"),
        Value::Float(f) => format!("{f}"),
        Value::Str(s) if s.contains([' ', '"']) && !s.starts_with('"') => format!("'{s}'"),
        other => inline(other),
    }
}
/// A value as a text file holds it: text ends with a line break.
pub fn file_text(v: &Value) -> String {
    let mut text = v.to_text();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text
}
/// One value on one line (a table cell, `echo`, interpolation).
pub fn inline(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => float(*f),
        Value::Str(s) => s.clone(),
        Value::Size(s) => size(*s),
        Value::Duration(d) => duration(*d),
        Value::List(l) => {
            if l.len() > 6 {
                format!("[list {} items]", l.len())
            } else {
                let parts: Vec<String> = l.iter().map(inline).collect();
                format!("[{}]", parts.join(", "))
            }
        }
        Value::Record(r) => format!("{{record {} fields}}", r.len()),
    }
}
pub fn float(f: f64) -> String {
    if f.is_finite() && f == (f as i64) as f64 && f.abs() < 1e15 {
        format!("{}", f as i64)
    } else {
        let s = format!("{f:.6}");
        let s = s.trim_end_matches('0');
        String::from(s.trim_end_matches('.'))
    }
}
/// 1.5 KB, 23.4 MB (powers of 1024, as File Explorer shows sizes).
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!(
            "{} {}",
            float((value * 10.0 + 0.5) as u64 as f64 / 10.0),
            UNITS[unit]
        )
    }
}
pub fn duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{}s", float((ms as f64 / 100.0 + 0.5) as u64 as f64 / 10.0))
    } else if ms < 3_600_000 {
        format!("{}min {}s", ms / 60_000, (ms % 60_000) / 1000)
    } else {
        format!("{}h {}min", ms / 3_600_000, (ms % 3_600_000) / 60_000)
    }
}
/// A number literal with an optional size or duration unit: `42`, `-3.5`,
/// `0x1F`, `10kb`, `1.5mb`, `2gb`, `500ms`, `5s`, `2min`, `1h`, `3d`.
pub fn parse_number(s: &str) -> Option<Value> {
    if s.is_empty() {
        return None;
    }
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return i64::from_str_radix(hex, 16).ok().map(Value::Int);
    }
    let split = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    if num.is_empty() || num == "-" || num == "." {
        return None;
    }
    let unit = unit.to_ascii_lowercase();
    let n: f64 = num.parse().ok()?;
    let scaled = |m: f64| (n * m + 0.5) as u64;
    Some(match unit.as_str() {
        "" => {
            if num.contains('.') || num.contains('e') {
                Value::Float(n)
            } else {
                Value::Int(num.parse().ok()?)
            }
        }
        "b" => Value::Size(scaled(1.0)),
        "kb" => Value::Size(scaled(1024.0)),
        "mb" => Value::Size(scaled(1024.0 * 1024.0)),
        "gb" => Value::Size(scaled(1024.0 * 1024.0 * 1024.0)),
        "tb" => Value::Size(scaled(1024.0 * 1024.0 * 1024.0 * 1024.0)),
        "ms" => Value::Duration(scaled(1.0)),
        "s" | "sec" => Value::Duration(scaled(1000.0)),
        "min" | "m" => Value::Duration(scaled(60_000.0)),
        "h" | "hr" => Value::Duration(scaled(3_600_000.0)),
        "d" | "day" | "days" => Value::Duration(scaled(86_400_000.0)),
        _ => return None,
    })
}
/// Ordering for sort, min/max and comparisons (numbers by value, text
/// case-insensitively, mixed kinds by kind).
pub fn compare(a: &Value, b: &Value) -> core::cmp::Ordering {
    use core::cmp::Ordering;
    match (a, b) {
        (Value::Str(x), Value::Str(y)) => {
            let (x, y) = (x.to_lowercase(), y.to_lowercase());
            x.cmp(&y)
        }
        (Value::Size(x), Value::Size(y)) | (Value::Duration(x), Value::Duration(y)) => x.cmp(y),
        _ => match (a.as_float(), b.as_float()) {
            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
            _ => inline(a).cmp(&inline(b)),
        },
    }
}
/// Console rendering, at most `width` columns wide: tables as aligned
/// columns under a header, records as `key  value` lines, lists one item
/// per line.
pub fn render(v: &Value, width: usize) -> String {
    match v {
        Value::Null => String::new(),
        Value::List(rows) if v.is_table() => {
            let mut columns: Vec<String> = Vec::new();
            for row in rows {
                if let Value::Record(fields) = row {
                    for (k, _) in fields {
                        if !columns.iter().any(|c| c == k) {
                            columns.push(k.clone());
                        }
                    }
                }
            }
            let cell = |row: &Value, c: &str| row.field(c).map(inline).unwrap_or_default();
            let numeric = |c: &str| {
                rows.iter().all(|r| {
                    matches!(
                        r.field(c),
                        None | Some(
                            Value::Int(_) | Value::Float(_) | Value::Size(_) | Value::Duration(_)
                        )
                    )
                })
            };
            let mut widths: Vec<usize> = columns.iter().map(|c| c.chars().count()).collect();
            for row in rows {
                for (i, c) in columns.iter().enumerate() {
                    widths[i] = widths[i].max(cell(row, c).chars().count().min(48));
                }
            }
            // Drop trailing columns that do not fit.
            let mut shown = columns.len();
            while shown > 1 && widths[..shown].iter().sum::<usize>() + 2 * (shown - 1) > width {
                shown -= 1;
            }
            let mut out = String::new();
            let line = |out: &mut String, cells: &[String], right: &[bool]| {
                for (i, text) in cells.iter().enumerate().take(shown) {
                    let text: String = text.chars().take(48).collect();
                    let pad = widths[i].saturating_sub(text.chars().count());
                    if i > 0 {
                        out.push_str("  ");
                    }
                    if right[i] {
                        out.extend(core::iter::repeat_n(' ', pad));
                        out.push_str(&text);
                    } else {
                        out.push_str(&text);
                        if i + 1 < shown {
                            out.extend(core::iter::repeat_n(' ', pad));
                        }
                    }
                }
                out.push('\n');
            };
            let right: Vec<bool> = columns.iter().map(|c| numeric(c)).collect();
            line(&mut out, &columns, &right);
            let rules: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
            line(&mut out, &rules, &right);
            for row in rows {
                let cells: Vec<String> = columns.iter().map(|c| cell(row, c)).collect();
                line(&mut out, &cells, &right);
            }
            out
        }
        Value::List(items) => {
            let mut out = String::new();
            for item in items {
                match item {
                    Value::List(_) | Value::Record(_) => out.push_str(&render(item, width)),
                    other => {
                        out.push_str(&inline(other));
                        out.push('\n');
                    }
                }
            }
            out
        }
        Value::Record(fields) => {
            let key = fields
                .iter()
                .map(|(k, _)| k.chars().count())
                .max()
                .unwrap_or(0);
            let mut out = String::new();
            for (k, v) in fields {
                let _ = writeln!(out, "{k:key$}  {}", inline(v));
            }
            out
        }
        other => {
            let mut s = inline(other);
            if !s.ends_with('\n') {
                s.push('\n');
            }
            s
        }
    }
}
/// JSON text (`to json`).
pub fn to_json(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(f) => out.push_str(&float(*f)),
        Value::Size(s) | Value::Duration(s) => out.push_str(&s.to_string()),
        Value::Str(s) => {
            out.push('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c if (c as u32) < 0x20 => {
                        let _ = write!(out, "\\u{:04x}", c as u32);
                    }
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        Value::List(l) => {
            out.push('[');
            for (i, item) in l.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                to_json(item, out);
            }
            out.push(']');
        }
        Value::Record(r) => {
            out.push('{');
            for (i, (k, item)) in r.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                to_json(&Value::Str(k.clone()), out);
                out.push_str(": ");
                to_json(item, out);
            }
            out.push('}');
        }
    }
}
/// Parse JSON (`from json`); None on malformed input.
pub fn from_json(text: &str) -> Option<Value> {
    let mut p = Json {
        s: text.as_bytes(),
        i: 0,
        depth: 0,
    };
    let v = p.value()?;
    p.ws();
    (p.i == p.s.len()).then_some(v)
}
struct Json<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}
impl Json<'_> {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(|c| c.is_ascii_whitespace()) {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn value(&mut self) -> Option<Value> {
        self.ws();
        self.depth += 1;
        if self.depth > 64 {
            return None;
        }
        let v = match *self.s.get(self.i)? {
            b'{' => {
                self.i += 1;
                let mut fields = Vec::new();
                if !self.eat(b'}') {
                    loop {
                        self.ws();
                        let Value::Str(k) = self.string()? else {
                            return None;
                        };
                        if !self.eat(b':') {
                            return None;
                        }
                        fields.push((k, self.value()?));
                        if self.eat(b'}') {
                            break;
                        }
                        if !self.eat(b',') {
                            return None;
                        }
                    }
                }
                Value::Record(fields)
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                if !self.eat(b']') {
                    loop {
                        items.push(self.value()?);
                        if self.eat(b']') {
                            break;
                        }
                        if !self.eat(b',') {
                            return None;
                        }
                    }
                }
                Value::List(items)
            }
            b'"' => self.string()?,
            b't' if self.s[self.i..].starts_with(b"true") => {
                self.i += 4;
                Value::Bool(true)
            }
            b'f' if self.s[self.i..].starts_with(b"false") => {
                self.i += 5;
                Value::Bool(false)
            }
            b'n' if self.s[self.i..].starts_with(b"null") => {
                self.i += 4;
                Value::Null
            }
            _ => {
                let start = self.i;
                while self
                    .s
                    .get(self.i)
                    .is_some_and(|c| c.is_ascii_digit() || b"+-.eE".contains(c))
                {
                    self.i += 1;
                }
                let text = core::str::from_utf8(&self.s[start..self.i]).ok()?;
                if text.contains(['.', 'e', 'E']) {
                    Value::Float(text.parse().ok()?)
                } else {
                    Value::Int(text.parse().ok()?)
                }
            }
        };
        self.depth -= 1;
        Some(v)
    }
    fn string(&mut self) -> Option<Value> {
        if self.s.get(self.i) != Some(&b'"') {
            return None;
        }
        self.i += 1;
        let mut out = String::new();
        loop {
            let c = *self.s.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => return Some(Value::Str(out)),
                b'\\' => {
                    let e = *self.s.get(self.i)?;
                    self.i += 1;
                    match e {
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'u' => {
                            let hex = core::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
                            self.i += 4;
                            out.push(
                                char::from_u32(u32::from_str_radix(hex, 16).ok()?)
                                    .unwrap_or('\u{fffd}'),
                            );
                        }
                        other => out.push(other as char),
                    }
                }
                _ => {
                    // Copy the UTF-8 sequence starting at this byte.
                    let start = self.i - 1;
                    let len = match c {
                        0x00..=0x7f => 1,
                        0xc0..=0xdf => 2,
                        0xe0..=0xef => 3,
                        _ => 4,
                    };
                    let text = core::str::from_utf8(self.s.get(start..start + len)?).ok()?;
                    out.push_str(text);
                    self.i = start + len;
                }
            }
        }
    }
}

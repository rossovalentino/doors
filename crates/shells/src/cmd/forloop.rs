//! FOR: the sets each form iterates (files and literals, /D directories,
//! /R directory trees, /L numbers, /F lines split into tokens), built
//! when the loop starts. A /F `'command'` set first runs the command with
//! its output captured (`Built::Command`), then builds from that text.
use super::Cmd;
use super::parse::{For, ForKind};
use crate::host::{Host, attr};
use crate::{wild, wpath};
use alloc::{collections::VecDeque, format, string::String, vec::Vec};

pub(crate) struct Loop {
    /// FOR variables of enclosing loops below this index stay bound.
    pub base: usize,
    kind: Kind,
}
enum Kind {
    Items(VecDeque<Vec<(char, String)>>),
    Range {
        var: char,
        next: i64,
        step: i64,
        end: i64,
    },
}
impl Loop {
    fn items(items: VecDeque<Vec<(char, String)>>) -> Self {
        Self {
            base: 0,
            kind: Kind::Items(items),
        }
    }
    /// The next iteration's bindings, None when done.
    pub fn next(&mut self) -> Option<Vec<(char, String)>> {
        match &mut self.kind {
            Kind::Items(q) => q.pop_front(),
            Kind::Range {
                var,
                next,
                step,
                end,
            } => {
                let done = if *step >= 0 {
                    *next > *end
                } else {
                    *next < *end
                };
                if done || *step == 0 && *next != *end {
                    return None;
                }
                let v = *next;
                *next = next.wrapping_add(*step);
                if *step == 0 {
                    // FOR /L (1,0,1) runs forever in cmd.exe; Doors stops it.
                    *next = end.wrapping_add(1);
                }
                Some(alloc::vec![(*var, format!("{v}"))])
            }
        }
    }
}
pub(crate) enum Built {
    Loop(Loop),
    /// Run this command line with output captured, then build again.
    Command(String),
    /// An error message (may be empty): the loop does not run.
    Error(String),
}

/// Split a set into items: spaces, tabs, `,`, `;`, `=` outside quotes.
fn items(set: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in set.chars() {
        if c == '"' {
            quoted = !quoted;
            cur.push(c);
        } else if !quoted && matches!(c, ' ' | '\t' | ',' | ';' | '=') {
            if !cur.is_empty() {
                out.push(core::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}
pub(crate) fn build(
    cmd: &mut Cmd,
    host: &mut dyn Host,
    spec: &For,
    vars: &[(char, String)],
    source: Option<String>,
) -> Built {
    let set = cmd.substitute(host, &spec.set, vars);
    let var = spec.var;
    let one = |v: String| alloc::vec![(var, v)];
    match spec.kind {
        ForKind::Range => {
            let n: Vec<i64> = items(&set)
                .iter()
                .map(|s| s.parse::<i64>().unwrap_or(0))
                .collect();
            let get = |i: usize| n.get(i).copied().unwrap_or(0);
            Built::Loop(Loop {
                base: 0,
                kind: Kind::Range {
                    var,
                    next: get(0),
                    step: get(1),
                    end: get(2),
                },
            })
        }
        ForKind::Files | ForKind::Dirs => {
            let dirs = spec.kind == ForKind::Dirs;
            let mut q = VecDeque::new();
            for item in items(&set) {
                if wpath::wild(&item) {
                    let pattern = item.replace('"', "");
                    let relative = !pattern.contains(['\\', '/', ':']);
                    for (path, _) in cmd.glob(host, &pattern, dirs) {
                        // Relative patterns give names, as cmd.exe does.
                        let value = if relative {
                            String::from(wpath::name(&path))
                        } else {
                            full_display(&pattern, &path)
                        };
                        q.push_back(one(value));
                    }
                } else {
                    q.push_back(one(item));
                }
            }
            Built::Loop(Loop::items(q))
        }
        ForKind::Recursive => {
            let root = match &spec.root {
                Some(r) => {
                    let r = cmd.substitute(host, r, vars);
                    cmd.full(&r).unwrap_or_else(|_| cmd.cwd.clone())
                }
                None => cmd.cwd.clone(),
            };
            let mut dirs = Vec::new();
            walk(host, &root, &mut dirs, 0);
            let mut q = VecDeque::new();
            let list = items(&set);
            for d in dirs {
                for item in &list {
                    let item = item.replace('"', "");
                    if wpath::wild(&item) {
                        let Some(native) = host.native(&d) else {
                            continue;
                        };
                        let Ok(mut entries) = host.list(&native) else {
                            continue;
                        };
                        entries.sort_by_key(|e| e.name.to_lowercase());
                        for e in entries {
                            if !e.meta.dir
                                && e.meta.attrs & (attr::HIDDEN | attr::SYSTEM) == 0
                                && wild::matches(&item, &e.name)
                            {
                                q.push_back(one(wpath::join(&d, &e.name)));
                            }
                        }
                    } else {
                        q.push_back(one(wpath::join(&d, &item)));
                    }
                }
            }
            Built::Loop(Loop::items(q))
        }
        ForKind::Lines => {
            let options = match Options::parse(&cmd.substitute(host, &spec.options, vars)) {
                Some(o) => o,
                None => {
                    return Built::Error(format!("{} was unexpected at this time.", spec.options));
                }
            };
            let text = match source {
                Some(t) => t,
                None => {
                    let s = set.trim();
                    let (cmd_quote, string_quote) = if options.backq {
                        ('`', '\'')
                    } else {
                        ('\'', '"')
                    };
                    if s.len() >= 2 && s.starts_with(cmd_quote) && s.ends_with(cmd_quote) {
                        return Built::Command(String::from(&s[1..s.len() - 1]));
                    }
                    if s.len() >= 2 && s.starts_with(string_quote) && s.ends_with(string_quote) {
                        String::from(&s[1..s.len() - 1])
                    } else {
                        // Files, in order; a missing one stops the loop.
                        let mut all = String::new();
                        for f in items(s) {
                            let f = f.replace('"', "");
                            let Ok(native) = cmd.native(host, &f) else {
                                return Built::Error(format!(
                                    "The system cannot find the file {f}."
                                ));
                            };
                            match host.read(&native, 16 << 20) {
                                Ok(b) => {
                                    all.push_str(&super::builtin::decode_text(&b));
                                    if !all.ends_with('\n') {
                                        all.push('\n');
                                    }
                                }
                                Err(_) => {
                                    return Built::Error(format!(
                                        "The system cannot find the file {f}."
                                    ));
                                }
                            }
                        }
                        all
                    }
                }
            };
            let mut q = VecDeque::new();
            for line in text.lines().filter(|l| !l.is_empty()).skip(options.skip) {
                if let Some(b) = options.split(line, var) {
                    q.push_back(b);
                }
            }
            Built::Loop(Loop::items(q))
        }
    }
}
/// A wildcard FOR item with a path gives the path as typed plus the name.
fn full_display(pattern: &str, found: &str) -> String {
    match pattern.rfind(['\\', '/']) {
        Some(i) => format!("{}{}", &pattern[..=i], wpath::name(found)),
        None => String::from(wpath::name(found)),
    }
}
fn walk(host: &mut dyn Host, dir: &str, out: &mut Vec<String>, depth: usize) {
    out.push(String::from(dir));
    if depth > 64 {
        return;
    }
    let Some(native) = host.native(dir) else {
        return;
    };
    let Ok(mut entries) = host.list(&native) else {
        return;
    };
    entries.sort_by_key(|e| e.name.to_lowercase());
    for e in entries {
        if e.meta.dir && e.meta.attrs & (attr::HIDDEN | attr::SYSTEM) == 0 {
            walk(host, &wpath::join(dir, &e.name), out, depth + 1);
        }
    }
}
/// FOR /F's options string.
struct Options {
    eol: Option<char>,
    skip: usize,
    delims: Vec<char>,
    /// Token numbers (1-based) and whether `*` takes the rest.
    tokens: Vec<usize>,
    rest: bool,
    backq: bool,
}
impl Options {
    fn parse(text: &str) -> Option<Self> {
        let mut o = Options {
            eol: Some(';'),
            skip: 0,
            delims: alloc::vec![' ', '\t'],
            tokens: alloc::vec![1],
            rest: false,
            backq: false,
        };
        let mut t = text.trim();
        while !t.is_empty() {
            let lower = t.to_ascii_lowercase();
            if let Some(r) = lower.strip_prefix("usebackq") {
                o.backq = true;
                t = t[t.len() - r.len()..].trim_start();
            } else if lower.starts_with("eol=") {
                let c = t[4..].chars().next();
                o.eol = c;
                t = t[4 + c.map_or(0, char::len_utf8)..].trim_start();
            } else if lower.starts_with("skip=") {
                let end = t[5..].find(' ').map_or(t.len(), |e| 5 + e);
                o.skip = t[5..end].parse().ok()?;
                t = t[end..].trim_start();
            } else if lower.starts_with("delims=") {
                // Delims run to the end of the options or the next space,
                // unless a space is itself a delimiter (it must come last).
                let body = &t[7..];
                let end = body.find(' ').map_or(body.len(), |e| e);
                let (d, rest) = if body[end..].trim().is_empty() || body.ends_with(' ') {
                    (body, "")
                } else {
                    (&body[..end], &body[end..])
                };
                o.delims = d.chars().collect();
                t = rest.trim_start();
            } else if lower.starts_with("tokens=") {
                let end = t[7..].find(' ').map_or(t.len(), |e| 7 + e);
                let spec = &t[7..end];
                o.tokens.clear();
                for part in spec.split(',') {
                    let part = part.trim();
                    if part == "*" {
                        o.rest = true;
                        continue;
                    }
                    let (body, star) = match part.strip_suffix('*') {
                        Some(b) => (b, true),
                        None => (part, false),
                    };
                    if let Some((a, b)) = body.split_once('-') {
                        let (a, b): (usize, usize) = (a.parse().ok()?, b.parse().ok()?);
                        o.tokens.extend(a..=b);
                    } else if !body.is_empty() {
                        o.tokens.push(body.parse().ok()?);
                    }
                    if star {
                        o.rest = true;
                    }
                }
                o.tokens.sort_unstable();
                o.tokens.dedup();
                t = t[end..].trim_start();
            } else {
                return None;
            }
        }
        Some(o)
    }
    /// Bindings for one line, None when it is skipped.
    fn split(&self, line: &str, first: char) -> Option<Vec<(char, String)>> {
        let chars: Vec<char> = line.chars().collect();
        // Tokens: (start, end) character ranges.
        let mut spans = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            while i < chars.len() && self.delims.contains(&chars[i]) {
                i += 1;
            }
            if i >= chars.len() {
                break;
            }
            let start = i;
            while i < chars.len() && !self.delims.contains(&chars[i]) {
                i += 1;
            }
            spans.push((start, i));
        }
        if spans.is_empty() {
            return None;
        }
        if let Some(eol) = self.eol
            && chars[spans[0].0] == eol
        {
            return None;
        }
        let mut out = Vec::new();
        let mut var = first;
        let mut last_end = 0usize;
        let mut any = false;
        for &n in &self.tokens {
            if let Some(&(s, e)) = spans.get(n.wrapping_sub(1)) {
                out.push((var, chars[s..e].iter().collect()));
                last_end = last_end.max(n);
                any = true;
            }
            var = next_var(var);
        }
        if self.rest {
            let after = if self.tokens.is_empty() {
                0
            } else {
                *self.tokens.last().unwrap()
            };
            if let Some(&(s, _)) = spans.get(after) {
                out.push((var, chars[s..].iter().collect()));
                any = true;
            }
        }
        let _ = last_end;
        any.then_some(out)
    }
}
fn next_var(c: char) -> char {
    match c {
        'z' => 'A',
        'Z' => 'a',
        _ => char::from_u32(c as u32 + 1).unwrap_or(c),
    }
}

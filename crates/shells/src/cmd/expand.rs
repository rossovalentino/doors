//! cmd.exe's expansion phases: percent expansion when a line is read
//! (`%VAR%`, `%VAR:~s,n%`, `%VAR:a=b%`, batch parameters with `%~`
//! modifiers, `%%` in batch files), FOR variable substitution and delayed
//! `!VAR!` expansion when a command executes.
use crate::host::{Meta, attr};
use alloc::{format, string::String, vec::Vec};

/// Environment and file facts the expansions read.
pub(crate) trait Ctx {
    /// A variable (environment, then dynamic ones such as `%CD%`).
    fn var(&mut self, name: &str) -> Option<String>;
    /// Full Windows path and metadata of a (possibly relative) path.
    fn file(&mut self, path: &str) -> (String, Option<Meta>);
    /// `%~$PATH:1`: first directory in the variable holding the file.
    fn search(&mut self, variable: &str, name: &str) -> Option<String>;
    fn local_time(&mut self, filetime: u64) -> crate::host::LocalTime;
}

/// Phase 1. `batch` resolves `%0`..`%9` and `%*`; None at the prompt, where
/// undefined variables stay as typed.
pub(crate) fn percent(line: &str, ctx: &mut dyn Ctx, batch: Option<&[String]>) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '%' {
            out.push(c);
            i += 1;
            continue;
        }
        if let Some(args) = batch {
            match chars.get(i + 1) {
                Some('%') => {
                    out.push('%');
                    i += 2;
                    continue;
                }
                Some(&d) if d.is_ascii_digit() => {
                    out.push_str(
                        args.get(d as usize - '0' as usize)
                            .map_or("", String::as_str),
                    );
                    i += 2;
                    continue;
                }
                Some('*') => {
                    out.push_str(&args.get(1..).unwrap_or(&[]).join(" "));
                    i += 2;
                    continue;
                }
                Some('~') => {
                    if let Some((mods, end)) = modifiers(&chars, i + 2, |c| c.is_ascii_digit()) {
                        let index = chars[end] as usize - '0' as usize;
                        let value = args.get(index).cloned().unwrap_or_default();
                        out.push_str(&apply_modifiers(&mods, &value, ctx));
                        i = end + 1;
                        continue;
                    }
                }
                _ => (),
            }
        }
        // %NAME[:op]%
        let close = chars[i + 1..]
            .iter()
            .position(|&ch| ch == '%')
            .map(|p| i + 1 + p);
        let Some(close) = close else {
            // A lone % vanishes in a batch file and stays at the prompt.
            if batch.is_none() {
                out.push('%');
            }
            i += 1;
            continue;
        };
        let body: String = chars[i + 1..close].iter().collect();
        let (name, op) = match body.find(':') {
            Some(p) => (&body[..p], Some(&body[p + 1..])),
            None => (body.as_str(), None),
        };
        if name.is_empty() {
            if batch.is_none() {
                out.push('%');
            }
            i += 1;
            continue;
        }
        match ctx.var(name) {
            Some(v) => match op.map(|o| apply_op(&v, o)) {
                Some(Some(r)) => {
                    out.push_str(&r);
                    i = close + 1;
                }
                Some(None) => {
                    // Malformed operation: cmd prints the text as typed.
                    out.push('%');
                    i += 1;
                }
                None => {
                    out.push_str(&v);
                    i = close + 1;
                }
            },
            None if batch.is_some() => {
                // Undefined: removed, but only up to the operation's own
                // colon form, as cmd.exe does.
                i = close + 1;
            }
            None => {
                out.push('%');
                i += 1;
            }
        }
    }
    out
}
/// `~start[,len]` substrings and `old=new` / `*old=new` replacement. None
/// for a malformed operation.
pub(crate) fn apply_op(value: &str, op: &str) -> Option<String> {
    let v: Vec<char> = value.chars().collect();
    if let Some(range) = op.strip_prefix('~') {
        let (a, b) = match range.split_once(',') {
            Some((a, b)) => (a.trim(), Some(b.trim())),
            None => (range.trim(), None),
        };
        let n = v.len() as i64;
        let start = parse_int(a).unwrap_or(0);
        let start = if start < 0 {
            (n + start).max(0)
        } else {
            start.min(n)
        };
        let end = match b {
            Some(b) => match parse_int(b) {
                Some(len) if len < 0 => (n + len).max(start),
                Some(len) => (start + len).min(n),
                None => n,
            },
            None => n,
        };
        return Some(v[start as usize..end.max(start) as usize].iter().collect());
    }
    let (old, new) = op.split_once('=')?;
    if let Some(old) = old.strip_prefix('*') {
        if old.is_empty() {
            return Some(value.into());
        }
        let lower = value.to_lowercase();
        return Some(match lower.find(&old.to_lowercase()) {
            Some(at) => format!("{new}{}", &value[at + old.len()..]),
            None => value.into(),
        });
    }
    if old.is_empty() {
        return Some(value.into());
    }
    Some(replace_ignore_case(value, old, new))
}
fn parse_int(s: &str) -> Option<i64> {
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let v = if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        i64::from_str_radix(h, 16).ok()?
    } else if s.len() > 1 && s.starts_with('0') {
        i64::from_str_radix(&s[1..], 8).ok()?
    } else {
        s.parse().ok()?
    };
    Some(if neg { -v } else { v })
}
pub(crate) fn replace_ignore_case(value: &str, old: &str, new: &str) -> String {
    let lower = value.to_lowercase();
    let needle = old.to_lowercase();
    if lower.len() != value.len() {
        return value.replace(old, new);
    }
    let mut out = String::new();
    let mut at = 0;
    while let Some(p) = lower[at..].find(&needle) {
        out.push_str(&value[at..at + p]);
        out.push_str(new);
        at += p + needle.len();
    }
    out.push_str(&value[at..]);
    out
}
/// Phase 5: `!NAME!` (and its operation forms); `^!` is a literal `!` and a
/// caret before any other character escapes it, as with delayed expansion
/// enabled.
pub(crate) fn delayed(text: &str, ctx: &mut dyn Ctx) -> String {
    if !text.contains('!') {
        return text.into();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '^' if i + 1 < chars.len() => {
                out.push(chars[i + 1]);
                i += 2;
            }
            '!' => match chars[i + 1..].iter().position(|&c| c == '!') {
                Some(p) => {
                    let body: String = chars[i + 1..i + 1 + p].iter().collect();
                    let (name, op) = match body.find(':') {
                        Some(q) => (&body[..q], Some(&body[q + 1..])),
                        None => (body.as_str(), None),
                    };
                    if let Some(v) = ctx.var(name) {
                        match op {
                            Some(o) => out.push_str(&apply_op(&v, o).unwrap_or_default()),
                            None => out.push_str(&v),
                        }
                    }
                    i += p + 2;
                }
                None => i += 1,
            },
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}
/// Modifier letters after `%~` and the index of the variable character:
/// the longest run of modifiers (`$VAR:` is the PATH-search form) that
/// still leaves a valid variable, as cmd.exe resolves `%~nf` with FOR %f.
fn modifiers(chars: &[char], j: usize, valid: impl Fn(char) -> bool) -> Option<(String, usize)> {
    let mut stops = alloc::vec![j];
    let mut k = j;
    while k < chars.len() {
        let c = chars[k];
        if c == '$' {
            let close = chars[k + 1..].iter().position(|&ch| ch == ':')? + k + 1;
            k = close + 1;
            stops.push(k);
            continue;
        }
        if "fdpnxsatz".contains(c.to_ascii_lowercase()) {
            k += 1;
            stops.push(k);
            continue;
        }
        break;
    }
    for &end in stops.iter().rev() {
        if end < chars.len() && valid(chars[end]) {
            return Some((
                chars[j..end]
                    .iter()
                    .map(|c| c.to_ascii_lowercase())
                    .collect(),
                end,
            ));
        }
    }
    None
}
/// Apply `%~` modifiers to a parameter or FOR value.
pub(crate) fn apply_modifiers(mods: &str, value: &str, ctx: &mut dyn Ctx) -> String {
    let unquoted = value.trim_matches('"');
    if mods.is_empty() {
        return unquoted.into();
    }
    let (search, letters) = match mods.find('$') {
        Some(p) => {
            let close = mods[p..].find(':').map_or(mods.len(), |c| p + c);
            (
                Some(&mods[p + 1..close]),
                format!("{}{}", &mods[..p], &mods[close + 1..]),
            )
        }
        None => (None, String::from(mods)),
    };
    let full = match search {
        Some(var) => match ctx.search(var, unquoted) {
            Some(f) => f,
            None => return String::new(),
        },
        None => ctx.file(unquoted).0,
    };
    if letters.is_empty() {
        return full;
    }
    let (_, meta) = ctx.file(&full);
    let has = |c| letters.contains(c);
    let mut path = full.clone();
    if has('s') {
        let mut short = String::from(&full[..full.len().min(3)]);
        for (i, part) in full[3.min(full.len())..]
            .split('\\')
            .filter(|p| !p.is_empty())
            .enumerate()
        {
            if i > 0 {
                short.push('\\');
            }
            short.push_str(&crate::wpath::short_name(part));
        }
        path = short;
    }
    let (dir, name) = crate::wpath::split(&path);
    let mut out = String::new();
    if has('a') {
        out.push_str(&meta.map_or(String::new(), |m| attr_string(m.attrs, m.dir)));
    }
    if has('t') {
        if !out.is_empty() {
            out.push(' ');
        }
        if let Some(m) = meta {
            let t = ctx.local_time(m.modified);
            out.push_str(&format!(
                "{} {}",
                crate::format::date(&t),
                crate::format::time_short(&t)
            ));
        }
    }
    if has('z') {
        if !out.is_empty() {
            out.push(' ');
        }
        if let Some(m) = meta.filter(|m| !m.dir) {
            out.push_str(&format!("{}", m.size));
        }
    }
    let pathy = has('f') || has('d') || has('p') || has('n') || has('x');
    if pathy && !out.is_empty() {
        out.push(' ');
    }
    if has('f') && !has('d') && !has('p') && !has('n') && !has('x') {
        out.push_str(&path);
        return out;
    }
    if has('d') {
        out.push_str(&path[..2.min(path.len())]);
    }
    if has('p') {
        let p = &dir[2.min(dir.len())..];
        out.push_str(p);
        if !p.ends_with('\\') {
            out.push('\\');
        }
    }
    if has('n') {
        out.push_str(crate::wpath::stem(name));
    }
    if has('x') {
        out.push_str(crate::wpath::extension(name));
    }
    if has('f') && !(has('d') || has('p') || has('n') || has('x')) {
        out.push_str(&path);
    }
    out
}
/// `%~a`: `drahscotl-` flag letters, one per position.
pub(crate) fn attr_string(attrs: u32, dir: bool) -> String {
    let mut s = String::new();
    let flags = [
        (dir, 'd'),
        (attrs & attr::READONLY != 0, 'r'),
        (attrs & attr::ARCHIVE != 0, 'a'),
        (attrs & attr::HIDDEN != 0, 'h'),
        (attrs & attr::SYSTEM != 0, 's'),
        (false, 'c'),
        (attrs & attr::OFFLINE != 0, 'o'),
        (false, 't'),
        (attrs & attr::REPARSE != 0, 'l'),
    ];
    for (on, c) in flags {
        s.push(if on { c } else { '-' });
    }
    s
}
/// Phase 4: substitute `%v` / `%~mods v` of the active FOR variables.
pub(crate) fn for_vars(text: &str, vars: &[(char, String)], ctx: &mut dyn Ctx) -> String {
    if vars.is_empty() || !text.contains('%') {
        return text.into();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let bound = |c: char| {
        vars.iter()
            .rev()
            .find(|(v, _)| *v == c)
            .map(|(_, s)| s.clone())
    };
    while i < chars.len() {
        if chars[i] == '%'
            && let Some(&n) = chars.get(i + 1)
        {
            if n != '~'
                && let Some(v) = bound(n)
            {
                out.push_str(&v);
                i += 2;
                continue;
            }
            if n == '~'
                && let Some((mods, end)) =
                    modifiers(&chars, i + 2, |c| vars.iter().any(|(v, _)| *v == c))
            {
                let v = bound(chars[end]).unwrap_or_default();
                out.push_str(&apply_modifiers(&mods, &v, ctx));
                i = end + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    struct T(BTreeMap<String, String>);
    impl Ctx for T {
        fn var(&mut self, name: &str) -> Option<String> {
            self.0.get(&name.to_ascii_uppercase()).cloned()
        }
        fn file(&mut self, path: &str) -> (String, Option<Meta>) {
            let full = if path.contains(':') {
                String::from(path)
            } else {
                format!("C:\\work\\{path}")
            };
            (
                full,
                Some(Meta {
                    size: 26,
                    attrs: attr::ARCHIVE,
                    ..Meta::default()
                }),
            )
        }
        fn search(&mut self, _: &str, name: &str) -> Option<String> {
            Some(format!("C:\\Windows\\System32\\{name}"))
        }
        fn local_time(&mut self, _: u64) -> crate::host::LocalTime {
            crate::host::LocalTime {
                year: 2026,
                month: 11,
                day: 2,
                hour: 12,
                minute: 16,
                ..Default::default()
            }
        }
    }
    fn ctx() -> T {
        let mut m = BTreeMap::new();
        m.insert("NAME".into(), "Hello World".into());
        m.insert("PATH".into(), "C:\\Windows".into());
        T(m)
    }
    #[test]
    fn percent_expansion() {
        let mut c = ctx();
        assert_eq!(percent("echo %name%", &mut c, None), "echo Hello World");
        assert_eq!(percent("echo %missing%", &mut c, None), "echo %missing%");
        assert_eq!(percent("echo %missing%x", &mut c, Some(&[])), "echo x");
        assert_eq!(percent("%name:~6%", &mut c, None), "World");
        assert_eq!(percent("%name:~0,5%", &mut c, None), "Hello");
        assert_eq!(percent("%name:~-5,3%", &mut c, None), "Wor");
        assert_eq!(percent("%name:o=0%", &mut c, None), "Hell0 W0rld");
        assert_eq!(percent("%name:*o=%", &mut c, None), " World");
        assert_eq!(percent("100%%", &mut c, Some(&[])), "100%");
        let args = [
            String::from("t.bat"),
            String::from("\"a b.txt\""),
            String::from("x"),
        ];
        assert_eq!(
            percent("[%1] [%~1] [%*] [%2]", &mut c, Some(&args)),
            "[\"a b.txt\"] [a b.txt] [\"a b.txt\" x] [x]"
        );
        assert_eq!(
            percent("%~nx1|%~dp1|%~x1|%~z1", &mut c, Some(&args)),
            "a b.txt|C:\\work\\|.txt|26"
        );
        assert_eq!(
            percent("%~$PATH:2", &mut c, Some(&args)),
            "C:\\Windows\\System32\\x"
        );
        assert_eq!(
            percent("for %%i in (a) do echo %%i", &mut c, Some(&args)),
            "for %i in (a) do echo %i"
        );
    }
    #[test]
    fn delayed_and_for_variables() {
        let mut c = ctx();
        assert_eq!(
            delayed("!name! ^!x !name:~0,1!", &mut c),
            "Hello World !x H"
        );
        let vars = [('i', String::from("a.txt")), ('j', String::from("two"))];
        assert_eq!(
            for_vars("%i-%j-%~ni-%~xi-%k", &vars, &mut c),
            "a.txt-two-a-.txt-%k"
        );
        assert_eq!(for_vars("%~fi", &vars, &mut c), "C:\\work\\a.txt");
        assert_eq!(
            attr_string(attr::ARCHIVE | attr::READONLY, false),
            "-ra------"
        );
    }
}

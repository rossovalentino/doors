//! Doorsh's own commands. Each takes its arguments (already expanded
//! values, never re-split) and the value piped in, and returns a value or
//! yields a step (a key, a line, a pause, an app to open).
use super::value::{self, Value};
use super::{Doorsh, Fail, Res, fail};
use crate::host::{Host, LocalTime, attr};
use crate::{InputKind, Launch, Resume, Step};
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

pub enum Outcome {
    Value(Value),
    Yield(Step, Pending),
    /// Run a script (`run`: its own scope; `source`: this one).
    Script(String, Vec<String>, bool),
    Exit(i32),
}
/// What a yielded built-in does when resumed.
#[derive(Clone, Debug, PartialEq)]
pub enum Pending {
    Nothing,
    Line,
    Key,
    Confirm(String),
    /// Delete these native paths (folders recursively) on 'y'.
    Remove(Vec<(String, bool)>),
}
pub const NAMES: &[&str] = &[
    "alias",
    "append",
    "avg",
    "base64",
    "bookmark",
    "calc",
    "cat",
    "cd",
    "chars",
    "clear",
    "clip",
    "columns",
    "compact",
    "confirm",
    "count",
    "cp",
    "date",
    "default",
    "describe",
    "df",
    "diff",
    "dirs",
    "du",
    "each-line",
    "echo",
    "edit",
    "enumerate",
    "env",
    "exit",
    "explain",
    "explorer",
    "find",
    "first",
    "flatten",
    "fns",
    "format",
    "from",
    "get",
    "go",
    "grep",
    "group-by",
    "hash",
    "head",
    "help",
    "history",
    "hostname",
    "input",
    "ip",
    "is-empty",
    "join",
    "kill",
    "last",
    "lines",
    "ls",
    "lower",
    "max",
    "min",
    "mkdir",
    "mv",
    "open",
    "paste",
    "pause",
    "ping",
    "popd",
    "print",
    "ps",
    "pushd",
    "pwd",
    "random",
    "reject",
    "rename",
    "replace",
    "resolve",
    "reverse",
    "rm",
    "rmdir",
    "run",
    "save",
    "select",
    "seq",
    "set-env",
    "settings",
    "skip",
    "sleep",
    "sort",
    "sort-by",
    "source",
    "split",
    "stat",
    "sum",
    "sysinfo",
    "tail",
    "take",
    "time",
    "title",
    "to",
    "touch",
    "tree",
    "trim",
    "type-of",
    "uniq",
    "unalias",
    "unset",
    "upper",
    "uptime",
    "uuid",
    "vars",
    "version",
    "wc",
    "where",
    "which",
    "whoami",
    "words",
    "write",
];
/// Windows and Unix habits map onto Doorsh commands.
pub const DEFAULT_ALIASES: &[(&str, &str)] = &[
    ("dir", "ls"),
    ("ll", "ls -l"),
    ("la", "ls -a"),
    ("type", "cat"),
    ("del", "rm"),
    ("erase", "rm"),
    ("copy", "cp"),
    ("move", "mv"),
    ("md", "mkdir"),
    ("rd", "rmdir"),
    ("ren", "rename"),
    ("cls", "clear"),
    ("more", "lines"),
    ("findstr", "grep"),
    ("man", "help"),
    ("..", "cd .."),
    ("...", "cd ..\\.."),
];
pub fn is_builtin(name: &str) -> bool {
    NAMES.binary_search(&name).is_ok() || NAMES.contains(&name)
}
/// Flags (`-r`, `--long`), options that take a value, and positionals.
struct Args {
    flags: Vec<String>,
    options: Vec<(String, Value)>,
    pos: Vec<Value>,
}
fn args(raw: &[Value], with_value: &[&str]) -> Args {
    let mut a = Args {
        flags: Vec::new(),
        options: Vec::new(),
        pos: Vec::new(),
    };
    let mut i = 0;
    while i < raw.len() {
        match &raw[i] {
            Value::Str(s)
                if s.len() > 1
                    && s.starts_with('-')
                    && !s[1..].starts_with(|c: char| c.is_ascii_digit()) =>
            {
                let name = s.trim_start_matches('-').to_string();
                if with_value.contains(&name.as_str()) && i + 1 < raw.len() {
                    a.options.push((name, raw[i + 1].clone()));
                    i += 1;
                } else if s.starts_with("--") || name.len() == 1 {
                    a.flags.push(name);
                } else {
                    // -rf: several one-letter flags.
                    a.flags.extend(name.chars().map(|c| c.to_string()));
                }
            }
            v => a.pos.push(v.clone()),
        }
        i += 1;
    }
    a
}
impl Args {
    fn has(&self, names: &[&str]) -> bool {
        self.flags.iter().any(|f| names.contains(&f.as_str()))
    }
    fn opt(&self, name: &str) -> Option<&Value> {
        self.options.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }
    fn text(&self, i: usize) -> Option<String> {
        self.pos.get(i).map(value::inline)
    }
    fn int(&self, i: usize) -> Res<Option<i64>> {
        match self.pos.get(i) {
            None => Ok(None),
            Some(v) => v
                .as_int()
                .map(Some)
                .ok_or_else(|| fail(format!("'{}' is not a number", value::inline(v)))),
        }
    }
}
fn ok(v: Value) -> Res<Outcome> {
    Ok(Outcome::Value(v))
}
fn none() -> Res<Outcome> {
    Ok(Outcome::Value(Value::Null))
}
fn usage(cmd: &str) -> Fail {
    fail(format!("usage: {}", super::help::usage(cmd)))
}
fn io(path: &str, e: crate::host::FsError) -> Fail {
    fail(format!("{path}: {}", e.message().trim_end()))
}
fn time_text(t: &LocalTime) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        t.year, t.month, t.day, t.hour, t.minute
    )
}
/// Rows of a value treated as a list (a string's lines).
fn rows(input: Value) -> Vec<Value> {
    input.items()
}
fn text_of(input: &Value) -> String {
    input.to_text()
}
/// Locations deleting never touches.
fn protected(full: &str) -> bool {
    let lower = full.to_ascii_lowercase();
    let lower = lower.trim_end_matches('\\');
    lower.len() <= 3
        || lower == "c:\\windows"
        || lower.starts_with("c:\\windows\\")
        || lower == "c:\\program files"
        || lower == "c:\\program files (x86)"
        || lower == "c:\\users"
        || lower == "c:\\programdata"
}
pub fn run(
    sh: &mut Doorsh,
    host: &mut dyn Host,
    name: &str,
    raw: &[Value],
    input: Value,
) -> Res<Outcome> {
    sh.status = 0;
    match name {
        // ---- navigation ----
        "cd" => {
            let a = args(raw, &[]);
            let target = match a.text(0).as_deref() {
                None | Some("~") => sh.home.clone(),
                Some("-") => sh
                    .prev_dir
                    .clone()
                    .ok_or_else(|| fail("no previous folder"))?,
                Some(t) => {
                    // A bookmark name works too.
                    if let Some(m) = sh.marks.get(t).filter(|_| {
                        sh.native(host, t).is_err()
                            || host
                                .native(&sh.full(t).unwrap_or_default())
                                .is_none_or(|n| host.stat(&n).is_err())
                    }) {
                        m.clone()
                    } else {
                        sh.full(t)?
                    }
                }
            };
            let native = host
                .native(&target)
                .ok_or_else(|| fail(format!("{target}: not on a Doors drive")))?;
            match host.stat(&native) {
                Ok(m) if m.dir => {}
                Ok(_) => return Err(fail(format!("{target} is a file, not a folder"))),
                Err(e) => return Err(io(&target, e)),
            }
            let target = host.windows(&native);
            sh.prev_dir = Some(core::mem::replace(&mut sh.cwd, target));
            none()
        }
        "pwd" => ok(Value::Str(sh.cwd.clone())),
        "ls" => ls(sh, host, raw),
        "tree" => {
            let a = args(raw, &["depth"]);
            let depth = a
                .opt("depth")
                .and_then(Value::as_int)
                .unwrap_or(3)
                .clamp(1, 12) as usize;
            let (full, native) =
                sh.native(host, &a.text(0).unwrap_or_else(|| String::from(".")))?;
            let mut out = format!("{full}\n");
            let mut count = 0;
            tree(
                host,
                &native,
                "",
                depth,
                a.has(&["f", "files"]),
                &mut out,
                &mut count,
            );
            ok(Value::Str(out))
        }
        "pushd" => {
            let a = args(raw, &[]);
            let target = sh.full(&a.text(0).ok_or_else(|| usage(name))?)?;
            let native = host
                .native(&target)
                .ok_or_else(|| fail("not on a Doors drive"))?;
            if !host.stat(&native).is_ok_and(|m| m.dir) {
                return Err(fail(format!("{target}: no such folder")));
            }
            let old = core::mem::replace(&mut sh.cwd, host.windows(&native));
            sh.dir_stack.push(old);
            none()
        }
        "popd" => {
            let d = sh
                .dir_stack
                .pop()
                .ok_or_else(|| fail("the folder stack is empty"))?;
            sh.prev_dir = Some(core::mem::replace(&mut sh.cwd, d));
            none()
        }
        "dirs" => ok(Value::List(
            sh.dir_stack.iter().rev().map(|d| Value::str(d)).collect(),
        )),
        "bookmark" => {
            let a = args(raw, &[]);
            match (a.text(0), a.text(1)) {
                (None, _) => ok(Value::List(
                    sh.marks
                        .iter()
                        .map(|(k, v)| {
                            Value::Record(alloc::vec![
                                (String::from("name"), Value::str(k)),
                                (String::from("folder"), Value::str(v))
                            ])
                        })
                        .collect(),
                )),
                (Some(n), _) if a.has(&["d", "delete"]) => {
                    sh.marks.remove(&n);
                    none()
                }
                (Some(n), Some(p)) => {
                    let full = sh.full(&p)?;
                    sh.marks.insert(n, full);
                    none()
                }
                (Some(n), None) => {
                    let cwd = sh.cwd.clone();
                    sh.marks.insert(n, cwd);
                    none()
                }
            }
        }
        "go" => {
            let a = args(raw, &[]);
            let n = a.text(0).ok_or_else(|| usage(name))?;
            let target = sh
                .marks
                .get(&n)
                .cloned()
                .ok_or_else(|| fail(format!("no bookmark '{n}' (see 'bookmark')")))?;
            sh.prev_dir = Some(core::mem::replace(&mut sh.cwd, target));
            none()
        }
        // ---- files ----
        "cat" => {
            let a = args(raw, &[]);
            if a.pos.is_empty() {
                return ok(input);
            }
            let mut out = String::new();
            for p in &a.pos {
                out.push_str(&sh.read_text(host, &value::inline(p))?);
            }
            ok(Value::Str(out))
        }
        "head" | "tail" => {
            let a = args(raw, &["n"]);
            let n = a.opt("n").and_then(Value::as_int).unwrap_or(10).max(0) as usize;
            let text = match a.text(0) {
                Some(p) => sh.read_text(host, &p)?,
                None => text_of(&input),
            };
            if a.pos.is_empty() && matches!(input, Value::List(_)) {
                let items = rows(input);
                let len = items.len();
                return ok(Value::List(if name == "head" {
                    items.into_iter().take(n).collect()
                } else {
                    items.into_iter().skip(len.saturating_sub(n)).collect()
                }));
            }
            let lines: Vec<&str> = text.lines().collect();
            let picked: Vec<&str> = if name == "head" {
                lines.iter().take(n).copied().collect()
            } else {
                lines[lines.len().saturating_sub(n)..].to_vec()
            };
            ok(Value::Str(picked.join("\n") + "\n"))
        }
        "wc" => {
            let a = args(raw, &[]);
            let text = match a.text(0) {
                Some(p) => sh.read_text(host, &p)?,
                None => text_of(&input),
            };
            ok(Value::Record(alloc::vec![
                (
                    String::from("lines"),
                    Value::Int(text.lines().count() as i64)
                ),
                (
                    String::from("words"),
                    Value::Int(text.split_whitespace().count() as i64)
                ),
                (
                    String::from("chars"),
                    Value::Int(text.chars().count() as i64)
                ),
                (String::from("bytes"), Value::Size(text.len() as u64)),
            ]))
        }
        "stat" => {
            let a = args(raw, &[]);
            let p = a.text(0).ok_or_else(|| usage(name))?;
            let (full, native) = sh.native(host, &p)?;
            let m = host.stat(&native).map_err(|e| io(&p, e))?;
            ok(Value::Record(alloc::vec![
                (String::from("name"), Value::str(crate::wpath::name(&full))),
                (String::from("path"), Value::Str(full.clone())),
                (
                    String::from("type"),
                    Value::str(if m.dir { "folder" } else { "file" })
                ),
                (String::from("size"), Value::Size(m.size)),
                (
                    String::from("modified"),
                    Value::Str(time_text(&host.local(m.modified)))
                ),
                (
                    String::from("created"),
                    Value::Str(time_text(&host.local(m.created)))
                ),
                (
                    String::from("readonly"),
                    Value::Bool(m.attrs & attr::READONLY != 0)
                ),
                (
                    String::from("hidden"),
                    Value::Bool(m.attrs & attr::HIDDEN != 0)
                ),
            ]))
        }
        "du" => {
            let a = args(raw, &[]);
            let (full, native) =
                sh.native(host, &a.text(0).unwrap_or_else(|| String::from(".")))?;
            let (size, files, folders) = du(host, &native, 0);
            ok(Value::Record(alloc::vec![
                (String::from("path"), Value::Str(full)),
                (String::from("size"), Value::Size(size)),
                (String::from("files"), Value::Int(files as i64)),
                (String::from("folders"), Value::Int(folders as i64)),
            ]))
        }
        "df" => {
            let v = host
                .volume('C')
                .ok_or_else(|| fail("the C: drive is not available"))?;
            ok(Value::List(alloc::vec![Value::Record(alloc::vec![
                (String::from("drive"), Value::str("C:")),
                (
                    String::from("label"),
                    Value::Str(if v.label.is_empty() {
                        String::from("Local Disk")
                    } else {
                        v.label
                    })
                ),
                (String::from("filesystem"), Value::Str(v.filesystem)),
                (String::from("size"), Value::Size(v.total)),
                (String::from("free"), Value::Size(v.free)),
                (
                    String::from("used"),
                    Value::Size(v.total.saturating_sub(v.free))
                ),
            ])]))
        }
        "find" => {
            let a = args(raw, &["name", "type", "depth"]);
            let (full, native) =
                sh.native(host, &a.text(0).unwrap_or_else(|| String::from(".")))?;
            let pattern = a
                .opt("name")
                .map(value::inline)
                .or_else(|| a.text(1))
                .unwrap_or_else(|| String::from("*"));
            let pattern = if pattern.contains(['*', '?']) {
                pattern
            } else {
                format!("*{pattern}*")
            };
            let want = a.opt("type").map(value::inline);
            let depth = a.opt("depth").and_then(Value::as_int).unwrap_or(32) as usize;
            let mut out = Vec::new();
            find(
                host,
                &native,
                &full,
                &pattern,
                want.as_deref(),
                depth,
                &mut out,
            );
            ok(Value::List(out))
        }
        "mkdir" => {
            let a = args(raw, &[]);
            if a.pos.is_empty() {
                return Err(usage(name));
            }
            for p in &a.pos {
                let (full, _) = sh.native(host, &value::inline(p))?;
                // Every missing parent too (mkdir -p is the default).
                let mut path = String::new();
                for part in full.split('\\') {
                    if path.is_empty() {
                        path = format!("{part}\\");
                        continue;
                    }
                    path = crate::wpath::join(&path, part);
                    let native = host
                        .native(&path)
                        .ok_or_else(|| fail("not on a Doors drive"))?;
                    match host.stat(&native) {
                        Ok(m) if m.dir => {}
                        Ok(_) => return Err(fail(format!("{path} is a file"))),
                        Err(_) => host.mkdir(&native).map_err(|e| io(&path, e))?,
                    }
                }
            }
            none()
        }
        "touch" => {
            let a = args(raw, &[]);
            for p in &a.pos {
                let p = value::inline(p);
                let (_, native) = sh.native(host, &p)?;
                if host.stat(&native).is_err() {
                    host.write(&native, b"", false).map_err(|e| io(&p, e))?;
                } else {
                    let data = host
                        .read(&native, 64 * 1024 * 1024)
                        .map_err(|e| io(&p, e))?;
                    host.write(&native, &data, false).map_err(|e| io(&p, e))?;
                }
            }
            none()
        }
        "cp" => {
            let a = args(raw, &[]);
            if a.pos.len() < 2 {
                return Err(usage(name));
            }
            let dest = a.text(a.pos.len() - 1).unwrap();
            let (dest_full, dest_native) = sh.native(host, &dest)?;
            let dest_dir = host.stat(&dest_native).is_ok_and(|m| m.dir);
            for p in &a.pos[..a.pos.len() - 1] {
                let p = value::inline(p);
                let (full, native) = sh.native(host, &p)?;
                let m = host.stat(&native).map_err(|e| io(&p, e))?;
                let target = if dest_dir {
                    crate::wpath::join(&dest_full, crate::wpath::name(&full))
                } else {
                    dest_full.clone()
                };
                let target_native = host
                    .native(&target)
                    .ok_or_else(|| fail("not on a Doors drive"))?;
                if m.dir {
                    if !a.has(&["r", "recursive"]) {
                        return Err(fail(format!("{p} is a folder: use cp -r")));
                    }
                    copy_tree(host, &native, &target_native)?;
                } else {
                    host.copy_file(&native, &target_native)
                        .map_err(|e| io(&p, e))?;
                }
            }
            none()
        }
        "mv" | "rename" => {
            let a = args(raw, &[]);
            if a.pos.len() < 2 {
                return Err(usage(name));
            }
            let dest = a.text(a.pos.len() - 1).unwrap();
            let (dest_full, dest_native) = sh.native(host, &dest)?;
            let dest_dir = host.stat(&dest_native).is_ok_and(|m| m.dir);
            for p in &a.pos[..a.pos.len() - 1] {
                let p = value::inline(p);
                let (full, native) = sh.native(host, &p)?;
                if protected(&full) {
                    return Err(fail(format!(
                        "{full} is a system location and can't be moved"
                    )));
                }
                let target = if dest_dir {
                    crate::wpath::join(&dest_full, crate::wpath::name(&full))
                } else {
                    dest_full.clone()
                };
                let target_native = host
                    .native(&target)
                    .ok_or_else(|| fail("not on a Doors drive"))?;
                host.rename(&native, &target_native)
                    .map_err(|e| io(&p, e))?;
            }
            none()
        }
        "rm" | "rmdir" => {
            let a = args(raw, &[]);
            if a.pos.is_empty() {
                return Err(usage(name));
            }
            let recursive = a.has(&["r", "recursive"]) || name == "rmdir" && a.has(&["s"]);
            let mut targets = Vec::new();
            for p in &a.pos {
                let p = value::inline(p);
                let (full, native) = sh.native(host, &p)?;
                if protected(&full) || full.eq_ignore_ascii_case(&sh.home) {
                    return Err(fail(format!(
                        "{full} is a protected location and can't be deleted"
                    )));
                }
                let m = host.stat(&native).map_err(|e| io(&p, e))?;
                if m.dir && name == "rm" && !recursive {
                    return Err(fail(format!(
                        "{p} is a folder: use rm -r (or rmdir for an empty folder)"
                    )));
                }
                targets.push((native, m.dir && recursive));
            }
            // Deleting several items or a folder tree asks first.
            if !a.has(&["f", "force", "y"]) && (targets.len() > 1 || targets.iter().any(|t| t.1)) {
                let what = if targets.len() == 1 {
                    String::from("this folder and everything in it")
                } else {
                    format!("these {} items", targets.len())
                };
                sh.out(&format!("Delete {what} permanently? (y/n) "));
                return Ok(Outcome::Yield(
                    Step::Input(InputKind::Key),
                    Pending::Remove(targets),
                ));
            }
            remove(host, &targets)?;
            none()
        }
        "hash" => {
            let a = args(raw, &["algorithm", "a"]);
            let algorithm = a
                .opt("algorithm")
                .or(a.opt("a"))
                .map(value::inline)
                .unwrap_or_else(|| String::from("SHA256"))
                .to_ascii_uppercase();
            let mut out = Vec::new();
            for p in &a.pos {
                let p = value::inline(p);
                let (_, native) = sh.native(host, &p)?;
                let digest = host.digest(&native, &algorithm).map_err(|e| io(&p, e))?;
                let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
                out.push(Value::Record(alloc::vec![
                    (String::from("algorithm"), Value::Str(algorithm.clone())),
                    (String::from("hash"), Value::Str(hex)),
                    (String::from("file"), Value::Str(p)),
                ]));
            }
            if out.is_empty() {
                return Err(usage(name));
            }
            ok(Value::List(out))
        }
        "diff" => {
            let a = args(raw, &[]);
            let (Some(x), Some(y)) = (a.text(0), a.text(1)) else {
                return Err(usage(name));
            };
            let (l, r) = (sh.read_text(host, &x)?, sh.read_text(host, &y)?);
            let out = diff(&l, &r);
            sh.status = i32::from(!out.is_empty());
            ok(Value::Str(if out.is_empty() {
                String::from("The files are identical.\n")
            } else {
                out
            }))
        }
        "save" | "append" | "write" => {
            let a = args(raw, &[]);
            let p = a.text(0).ok_or_else(|| usage(name))?;
            // `append file text...` / `write file text...` write their words.
            let text = if a.pos.len() > 1 {
                a.pos[1..]
                    .iter()
                    .map(value::inline)
                    .collect::<Vec<_>>()
                    .join(" ")
                    + "\n"
            } else {
                value::file_text(&input)
            };
            sh.write_text(host, &p, &text, name == "append" || a.has(&["a", "append"]))?;
            none()
        }
        "open" | "explorer" | "edit" => {
            let a = args(raw, &[]);
            let target = a.text(0).unwrap_or_else(|| String::from("."));
            let (_, native) = sh.native(host, &target)?;
            let launch = match name {
                "edit" => Launch::App {
                    app: crate::legacy::App::Notes,
                    argument: Some(native),
                },
                "explorer" => Launch::App {
                    app: crate::legacy::App::Files,
                    argument: Some(native),
                },
                _ => Launch::Open(native),
            };
            Ok(Outcome::Yield(Step::Launch(launch), Pending::Nothing))
        }
        "settings" => {
            let a = args(raw, &[]);
            let query = a
                .pos
                .iter()
                .map(value::inline)
                .collect::<Vec<_>>()
                .join(" ");
            let nav = if query.is_empty() {
                door_settings_map::navigate_arg(door_settings_map::Page::Home, None)
            } else if let Some((page, section)) = door_settings_map::resolve(&query) {
                door_settings_map::navigate_arg(page, section)
            } else if let Some(hit) = door_settings_map::search(&query).next() {
                door_settings_map::navigate_arg(hit.page(), hit.anchor())
            } else {
                return Err(fail(format!("no Settings page matches '{query}'")));
            };
            sh.out_line(&crate::legacy::settings_note(nav));
            Ok(Outcome::Yield(
                Step::Launch(Launch::Settings(nav)),
                Pending::Nothing,
            ))
        }
        // ---- text and data ----
        "echo" | "print" => {
            let a = args(raw, &[]);
            if a.pos.len() == 1 && !a.has(&["n"]) {
                return ok(a.pos[0].clone());
            }
            let text = a
                .pos
                .iter()
                .map(value::inline)
                .collect::<Vec<_>>()
                .join(" ");
            ok(Value::Str(if a.has(&["n"]) { text } else { text + "\n" }))
        }
        "grep" => {
            let a = args(raw, &["n"]);
            let pattern = a.text(0).ok_or_else(|| usage(name))?;
            let insensitive =
                a.has(&["i", "ignore-case"]) || !pattern.chars().any(char::is_uppercase);
            let invert = a.has(&["v", "invert"]);
            let numbers = a.has(&["n", "line-numbers"]);
            let mut out = Vec::new();
            let mut sources: Vec<(Option<String>, String)> = Vec::new();
            if a.pos.len() > 1 {
                for f in &a.pos[1..] {
                    let f = value::inline(f);
                    sources.push((Some(f.clone()), sh.read_text(host, &f)?));
                }
            } else if matches!(input, Value::List(_)) {
                // Lists and tables: keep the items whose text matches.
                let items: Vec<Value> = rows(input)
                    .into_iter()
                    .filter(|item| {
                        let t = match item {
                            Value::Record(f) => f
                                .iter()
                                .map(|(_, v)| value::inline(v))
                                .collect::<Vec<_>>()
                                .join(" "),
                            other => value::inline(other),
                        };
                        matches(&t, &pattern, insensitive) != invert
                    })
                    .collect();
                sh.status = i32::from(items.is_empty());
                return ok(Value::List(items));
            } else {
                sources.push((None, text_of(&input)));
            }
            let several = sources.len() > 1;
            for (file, text) in sources {
                for (n, line) in text.lines().enumerate() {
                    if matches(line, &pattern, insensitive) != invert {
                        let mut s = String::new();
                        if several {
                            s.push_str(file.as_deref().unwrap_or(""));
                            s.push(':');
                        }
                        if numbers {
                            s.push_str(&format!("{}:", n + 1));
                        }
                        s.push_str(line);
                        out.push(Value::Str(s));
                    }
                }
            }
            sh.status = i32::from(out.is_empty());
            ok(Value::List(out))
        }
        "lines" => ok(Value::List(
            text_of(&input).lines().map(Value::str).collect(),
        )),
        "words" => ok(Value::List(
            text_of(&input).split_whitespace().map(Value::str).collect(),
        )),
        "chars" => ok(Value::List(
            text_of(&input)
                .chars()
                .filter(|c| *c != '\n')
                .map(|c| Value::Str(c.to_string()))
                .collect(),
        )),
        "split" => {
            let a = args(raw, &[]);
            let sep = a.text(0).unwrap_or_else(|| String::from(","));
            let text = text_of(&input);
            ok(Value::List(
                text.trim_end_matches('\n')
                    .split(sep.as_str())
                    .map(Value::str)
                    .collect(),
            ))
        }
        "join" => {
            let a = args(raw, &[]);
            let sep = a.text(0).unwrap_or_default();
            ok(Value::Str(
                rows(input)
                    .iter()
                    .map(value::inline)
                    .collect::<Vec<_>>()
                    .join(&sep),
            ))
        }
        "replace" => {
            let a = args(raw, &[]);
            let (Some(from), Some(to)) = (a.text(0), a.text(1)) else {
                return Err(usage(name));
            };
            map_text(input, |s| s.replace(from.as_str(), &to))
        }
        "trim" => map_text(input, |s| String::from(s.trim())),
        "upper" => map_text(input, |s| s.to_uppercase()),
        "lower" => map_text(input, |s| s.to_lowercase()),
        "sort" => {
            let a = args(raw, &[]);
            let mut items = rows(input);
            items.sort_by(value::compare);
            if a.has(&["r", "reverse"]) {
                items.reverse();
            }
            ok(Value::List(items))
        }
        "sort-by" => {
            let a = args(raw, &[]);
            let field = a.text(0).ok_or_else(|| usage(name))?;
            let mut items = rows(input);
            items.sort_by(|x, y| {
                value::compare(
                    x.field(&field).unwrap_or(&Value::Null),
                    y.field(&field).unwrap_or(&Value::Null),
                )
            });
            if a.has(&["r", "reverse"]) {
                items.reverse();
            }
            ok(Value::List(items))
        }
        "uniq" => {
            let mut out: Vec<Value> = Vec::new();
            for v in rows(input) {
                if !out.iter().any(|o| super::equal(o, &v)) {
                    out.push(v);
                }
            }
            ok(Value::List(out))
        }
        "reverse" => {
            let mut items = rows(input);
            items.reverse();
            ok(Value::List(items))
        }
        "where" => {
            let text = raw.iter().map(value::inline).collect::<Vec<_>>().join(" ");
            if text.is_empty() {
                return Err(usage(name));
            }
            let expr = super::parse::parse_expr(&text)
                .map_err(|e| fail(format!("where: {}", e.message)))?;
            let mut out = Vec::new();
            for row in rows(input) {
                if sh.eval_row(host, &expr, &row)?.truthy() {
                    out.push(row);
                }
            }
            ok(Value::List(out))
        }
        "select" | "reject" => {
            let fields: Vec<String> = raw.iter().map(value::inline).collect();
            let keep =
                |k: &str| fields.iter().any(|f| f.eq_ignore_ascii_case(k)) == (name == "select");
            let pick = |v: Value| match v {
                Value::Record(f) => {
                    if name == "select" {
                        Value::Record(
                            fields
                                .iter()
                                .filter_map(|n| {
                                    f.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).cloned()
                                })
                                .collect(),
                        )
                    } else {
                        Value::Record(f.into_iter().filter(|(k, _)| keep(k)).collect())
                    }
                }
                other => other,
            };
            ok(match input {
                Value::List(items) => Value::List(items.into_iter().map(pick).collect()),
                other => pick(other),
            })
        }
        "get" => {
            let a = args(raw, &[]);
            let path = a.text(0).ok_or_else(|| usage(name))?;
            let mut v = input;
            for part in path.split('.') {
                v = super::index(&v, &Value::str(part))?;
            }
            ok(v)
        }
        "first" | "last" | "take" | "skip" => {
            let a = args(raw, &[]);
            let n = a.int(0)?.unwrap_or(1).max(0) as usize;
            if let Value::Str(s) = &input
                && !s.contains('\n')
            {
                let chars: Vec<char> = s.chars().collect();
                let picked: String = match name {
                    "first" | "take" => chars.iter().take(n).collect(),
                    "last" => chars[chars.len().saturating_sub(n)..].iter().collect(),
                    _ => chars.iter().skip(n).collect(),
                };
                return ok(Value::Str(picked));
            }
            let items = rows(input);
            let len = items.len();
            let picked: Vec<Value> = match name {
                "first" | "take" => items.into_iter().take(n).collect(),
                "last" => items.into_iter().skip(len.saturating_sub(n)).collect(),
                _ => items.into_iter().skip(n).collect(),
            };
            // `first` / `last` without a count give the item itself.
            if a.pos.is_empty() && matches!(name, "first" | "last") {
                return ok(picked.into_iter().next().unwrap_or(Value::Null));
            }
            ok(Value::List(picked))
        }
        "count" => ok(Value::Int(match &input {
            Value::Null => 0,
            Value::Str(s) if !s.contains('\n') => s.chars().count() as i64,
            other => other.clone().items().len() as i64,
        })),
        "sum" | "avg" | "min" | "max" => {
            let a = args(raw, &[]);
            let items: Vec<Value> = match a.text(0) {
                Some(field) => rows(input)
                    .into_iter()
                    .filter_map(|r| r.field(&field).cloned())
                    .collect(),
                None => rows(input),
            };
            if items.is_empty() {
                return ok(Value::Null);
            }
            match name {
                "min" => ok(items
                    .iter()
                    .min_by(|x, y| value::compare(x, y))
                    .cloned()
                    .unwrap()),
                "max" => ok(items
                    .iter()
                    .max_by(|x, y| value::compare(x, y))
                    .cloned()
                    .unwrap()),
                _ => {
                    let mut total = items[0].clone();
                    for v in &items[1..] {
                        total = super::binary(&total, super::parse::Op::Add, v)?;
                    }
                    if name == "sum" {
                        ok(total)
                    } else {
                        super::binary(
                            &total,
                            super::parse::Op::Div,
                            &Value::Int(items.len() as i64),
                        )
                        .map(Outcome::Value)
                    }
                }
            }
        }
        "enumerate" => ok(Value::List(
            rows(input)
                .into_iter()
                .enumerate()
                .map(|(i, v)| {
                    Value::Record(alloc::vec![
                        (String::from("index"), Value::Int(i as i64)),
                        (String::from("item"), v)
                    ])
                })
                .collect(),
        )),
        "flatten" => {
            let mut out = Vec::new();
            for v in rows(input) {
                match v {
                    Value::List(inner) => out.extend(inner),
                    other => out.push(other),
                }
            }
            ok(Value::List(out))
        }
        "group-by" => {
            let a = args(raw, &[]);
            let field = a.text(0).ok_or_else(|| usage(name))?;
            let mut groups: Vec<(String, Vec<Value>)> = Vec::new();
            for row in rows(input) {
                let key = row.field(&field).map(value::inline).unwrap_or_default();
                match groups.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, g)) => g.push(row),
                    None => groups.push((key, alloc::vec![row])),
                }
            }
            ok(Value::List(
                groups
                    .into_iter()
                    .map(|(k, g)| {
                        Value::Record(alloc::vec![
                            (field.clone(), Value::Str(k)),
                            (String::from("count"), Value::Int(g.len() as i64)),
                            (String::from("items"), Value::List(g))
                        ])
                    })
                    .collect(),
            ))
        }
        "columns" => match input {
            Value::Record(f) => ok(Value::List(
                f.into_iter().map(|(k, _)| Value::Str(k)).collect(),
            )),
            Value::List(rows) => {
                let mut names: Vec<String> = Vec::new();
                for r in rows {
                    if let Value::Record(f) = r {
                        for (k, _) in f {
                            if !names.contains(&k) {
                                names.push(k);
                            }
                        }
                    }
                }
                ok(Value::List(names.into_iter().map(Value::Str).collect()))
            }
            _ => ok(Value::List(Vec::new())),
        },
        "compact" => ok(Value::List(
            rows(input)
                .into_iter()
                .filter(|v| *v != Value::Null && !matches!(v, Value::Str(s) if s.trim().is_empty()))
                .collect(),
        )),
        "default" => {
            let a = args(raw, &[]);
            let d = a.pos.first().cloned().ok_or_else(|| usage(name))?;
            ok(
                if input == Value::Null || matches!(&input, Value::Str(s) if s.is_empty()) {
                    d
                } else {
                    input
                },
            )
        }
        "is-empty" => ok(Value::Bool(!input.truthy())),
        "describe" | "type-of" => ok(Value::str(input.type_name())),
        "from" => {
            let a = args(raw, &[]);
            let text = text_of(&input);
            match a.text(0).as_deref() {
                Some("json") => value::from_json(&text)
                    .map(Outcome::Value)
                    .ok_or_else(|| fail("from json: the text is not valid JSON")),
                Some("csv") => ok(from_csv(&text)),
                Some("lines") => ok(Value::List(text.lines().map(Value::str).collect())),
                _ => Err(usage(name)),
            }
        }
        "to" => {
            let a = args(raw, &[]);
            match a.text(0).as_deref() {
                Some("json") => {
                    let mut s = String::new();
                    value::to_json(&input, &mut s);
                    ok(Value::Str(s + "\n"))
                }
                Some("csv") => ok(Value::Str(to_csv(&input))),
                Some("text") => ok(Value::Str(value::render(&input, usize::MAX))),
                _ => Err(usage(name)),
            }
        }
        "format" => {
            let a = args(raw, &[]);
            let template = a.text(0).ok_or_else(|| usage(name))?;
            let fill = |row: &Value| {
                let mut out = String::new();
                let mut rest = template.as_str();
                while let Some(open) = rest.find('{') {
                    out.push_str(&rest[..open]);
                    let Some(close) = rest[open..].find('}') else {
                        break;
                    };
                    let key = &rest[open + 1..open + close];
                    out.push_str(&row.field(key).map(value::inline).unwrap_or_default());
                    rest = &rest[open + close + 1..];
                }
                out.push_str(rest);
                Value::Str(out)
            };
            ok(match input {
                Value::List(items) => Value::List(items.iter().map(fill).collect()),
                other => fill(&other),
            })
        }
        "base64" => {
            let a = args(raw, &[]);
            let text = match a.text(0) {
                Some(t) if !a.has(&["d", "decode"]) || input == Value::Null => t,
                _ => text_of(&input),
            };
            if a.has(&["d", "decode"]) {
                let bytes =
                    base64_decode(text.trim()).ok_or_else(|| fail("base64: not valid base64"))?;
                ok(Value::Str(String::from_utf8_lossy(&bytes).into_owned()))
            } else {
                ok(Value::Str(base64_encode(text.as_bytes())))
            }
        }
        "uuid" => {
            let mut b = [0u8; 16];
            for chunk in b.chunks_mut(4) {
                chunk.copy_from_slice(&host.random().to_le_bytes());
            }
            b[6] = (b[6] & 0x0f) | 0x40;
            b[8] = (b[8] & 0x3f) | 0x80;
            let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
            ok(Value::Str(format!(
                "{}-{}-{}-{}-{}",
                &h[..8],
                &h[8..12],
                &h[12..16],
                &h[16..20],
                &h[20..]
            )))
        }
        "random" => {
            let a = args(raw, &[]);
            let (lo, hi) = match (a.int(0)?, a.int(1)?) {
                (Some(a), Some(b)) => (a, b),
                (Some(b), None) => (0, b),
                _ => (0, 100),
            };
            if hi < lo {
                return Err(fail("random: the maximum is below the minimum"));
            }
            let span = (hi - lo) as u64 + 1;
            ok(Value::Int(
                lo + ((u64::from(host.random()) << 16 | u64::from(host.random() & 0xffff)) % span)
                    as i64,
            ))
        }
        "seq" => {
            let a = args(raw, &[]);
            let (lo, hi) = match (a.int(0)?, a.int(1)?) {
                (Some(a), Some(b)) => (a, b),
                (Some(b), None) => (1, b),
                _ => return Err(usage(name)),
            };
            super::range(&Value::Int(lo), &Value::Int(hi)).map(Outcome::Value)
        }
        "calc" => {
            let text = raw.iter().map(value::inline).collect::<Vec<_>>().join(" ");
            let e = super::parse::parse_expr(&text)
                .map_err(|e| fail(format!("calc: {}", e.message)))?;
            sh.eval(host, &e).map(Outcome::Value)
        }
        "each-line" => {
            let a = args(raw, &[]);
            let template = a.text(0).ok_or_else(|| usage(name))?;
            ok(Value::List(
                rows(input)
                    .iter()
                    .map(|v| Value::Str(template.replace("{}", &value::inline(v))))
                    .collect(),
            ))
        }
        // ---- system ----
        "ps" => {
            let mut list: Vec<Value> = host
                .processes()
                .into_iter()
                .map(|p| {
                    Value::Record(alloc::vec![
                        (String::from("pid"), Value::Int(i64::from(p.pid))),
                        (String::from("name"), Value::Str(p.name)),
                        (String::from("memory"), Value::Size(p.memory_kb * 1024)),
                        (String::from("session"), Value::Str(p.session_name)),
                    ])
                })
                .collect();
            list.sort_by(|a, b| {
                value::compare(
                    a.field("pid").unwrap_or(&Value::Null),
                    b.field("pid").unwrap_or(&Value::Null),
                )
            });
            ok(Value::List(list))
        }
        "kill" => {
            let a = args(raw, &[]);
            if a.pos.is_empty() {
                return Err(usage(name));
            }
            for p in &a.pos {
                let pid = match p.as_int() {
                    Some(pid) => pid as u32,
                    None => {
                        let n = value::inline(p).to_ascii_lowercase();
                        host.processes()
                            .into_iter()
                            .find(|x| {
                                x.name.to_ascii_lowercase() == n
                                    || x.name.to_ascii_lowercase() == format!("{n}.exe")
                            })
                            .map(|x| x.pid)
                            .ok_or_else(|| fail(format!("no process named {n}")))?
                    }
                };
                host.terminate(pid, a.has(&["f", "force"]))
                    .map_err(|e| fail(format!("process {pid}: {}", e.message().trim_end())))?;
            }
            none()
        }
        "sysinfo" => {
            let s = host.system();
            let v = host.volume('C');
            ok(Value::Record(alloc::vec![
                (String::from("os"), Value::Str(s.os_name)),
                (String::from("version"), Value::Str(s.os_version)),
                (String::from("computer"), Value::Str(host.computer_name())),
                (String::from("user"), Value::Str(host.user_name())),
                (
                    String::from("processor"),
                    Value::Str(s.processors.first().cloned().unwrap_or_default())
                ),
                (
                    String::from("processors"),
                    Value::Int(i64::from(host.processors()))
                ),
                (
                    String::from("memory"),
                    Value::Size(s.memory_total_mb * 1024 * 1024)
                ),
                (
                    String::from("memory free"),
                    Value::Size(s.memory_free_mb * 1024 * 1024)
                ),
                (
                    String::from("disk"),
                    Value::Size(v.as_ref().map_or(0, |v| v.total))
                ),
                (
                    String::from("disk free"),
                    Value::Size(v.as_ref().map_or(0, |v| v.free))
                ),
                (
                    String::from("uptime"),
                    Value::Duration(host.ticks().saturating_mul(10))
                ),
            ]))
        }
        "uptime" => ok(Value::Duration(host.ticks().saturating_mul(10))),
        "whoami" => {
            let a = args(raw, &[]);
            if a.has(&["all", "a"]) {
                return ok(Value::Record(alloc::vec![
                    (
                        String::from("user"),
                        Value::Str(format!("{}\\{}", host.domain(), host.user_name()))
                    ),
                    (String::from("sid"), Value::Str(host.user_sid())),
                    (String::from("administrator"), Value::Bool(host.elevated())),
                ]));
            }
            ok(Value::Str(format!(
                "{}\\{}",
                host.domain().to_lowercase(),
                host.user_name().to_lowercase()
            )))
        }
        "hostname" => ok(Value::Str(host.computer_name())),
        "env" => ok(Value::List(
            sh.env
                .iter()
                .map(|(k, v)| {
                    Value::Record(alloc::vec![
                        (String::from("name"), Value::str(k)),
                        (String::from("value"), Value::str(v))
                    ])
                })
                .collect(),
        )),
        "set-env" => {
            let a = args(raw, &[]);
            let (Some(n), v) = (a.text(0), a.text(1)) else {
                return Err(usage(name));
            };
            sh.set_env(&n, &v.unwrap_or_default());
            if a.has(&["persist", "p"]) {
                host.set_user_variable(&n, &a.text(1).unwrap_or_default())
                    .map_err(|e| fail(format!("set-env: {}", e.message().trim_end())))?;
            }
            none()
        }
        "unset" => {
            for n in raw.iter().map(value::inline) {
                let n = n.trim_start_matches('$').to_string();
                if !sh.unset_var(&n) && !sh.remove_function(&n) {
                    return Err(fail(format!("${n} is not set")));
                }
            }
            none()
        }
        "vars" => ok(Value::List(
            sh.variables()
                .into_iter()
                .map(|(k, v)| {
                    Value::Record(alloc::vec![
                        (String::from("name"), Value::Str(k)),
                        (String::from("type"), Value::str(v.type_name())),
                        (String::from("value"), Value::Str(value::inline(&v)))
                    ])
                })
                .collect(),
        )),
        "fns" => ok(Value::List(
            sh.function_list()
                .into_iter()
                .map(|f| {
                    let params = f
                        .params
                        .iter()
                        .map(|(p, d)| {
                            if d.is_some() {
                                format!("{p}?")
                            } else {
                                p.clone()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    Value::Record(alloc::vec![
                        (String::from("name"), Value::Str(f.name.clone())),
                        (String::from("parameters"), Value::Str(params))
                    ])
                })
                .collect(),
        )),
        "which" => {
            let a = args(raw, &[]);
            let n = a.text(0).ok_or_else(|| usage(name))?;
            let kind = if sh.has_function(&n) {
                String::from("function")
            } else if let Some(t) = sh.aliases.get(&n) {
                format!("alias for '{t}'")
            } else if is_builtin(&n) {
                String::from("Doorsh command")
            } else {
                // Ask cmd's resolver through `where`-style lookup.
                match crate::legacy::resolve(&n, &n) {
                    Some(crate::legacy::Target::App(app)) => format!("opens {}", app.label()),
                    Some(crate::legacy::Target::Settings(nav)) => {
                        crate::legacy::settings_note(nav).replace("Opening ", "opens ")
                    }
                    None => String::from(
                        "a program or System32 tool (run through cmd.exe), if one exists",
                    ),
                }
            };
            ok(Value::Record(alloc::vec![
                (String::from("name"), Value::Str(n)),
                (String::from("is"), Value::Str(kind))
            ]))
        }
        "alias" => {
            let a = args(raw, &[]);
            match a.text(0) {
                None => ok(Value::List(
                    sh.aliases
                        .iter()
                        .map(|(k, v)| {
                            Value::Record(alloc::vec![
                                (String::from("name"), Value::str(k)),
                                (String::from("command"), Value::str(v))
                            ])
                        })
                        .collect(),
                )),
                Some(n) => {
                    // alias name = command words  |  alias name command words
                    let start = if a.text(1).as_deref() == Some("=") {
                        2
                    } else {
                        1
                    };
                    let body = a
                        .pos
                        .iter()
                        .skip(start)
                        .map(value::inline)
                        .collect::<Vec<_>>()
                        .join(" ");
                    if body.is_empty() {
                        return ok(sh
                            .aliases
                            .get(&n)
                            .map_or(Value::Null, |t| Value::Str(t.clone())));
                    }
                    if is_builtin(&n) && body.split_whitespace().next() != Some(n.as_str()) {
                        return Err(fail(format!(
                            "'{n}' is a Doorsh command and can't be redefined"
                        )));
                    }
                    sh.aliases.insert(n, body);
                    none()
                }
            }
        }
        "unalias" => {
            for n in raw.iter().map(value::inline) {
                if sh.aliases.remove(&n).is_none() {
                    return Err(fail(format!("no alias '{n}'")));
                }
            }
            none()
        }
        "history" => {
            let a = args(raw, &[]);
            let filter = a.text(0).unwrap_or_default();
            ok(Value::List(
                sh.history
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| l.contains(filter.as_str()))
                    .map(|(i, l)| {
                        Value::Record(alloc::vec![
                            (String::from("index"), Value::Int(i as i64 + 1)),
                            (String::from("command"), Value::Str(l.clone()))
                        ])
                    })
                    .collect(),
            ))
        }
        "time" => {
            let line = raw.iter().map(value::inline).collect::<Vec<_>>().join(" ");
            if line.is_empty() {
                return Err(usage(name));
            }
            let stmts =
                super::parse::parse(&line).map_err(|e| fail(format!("time: {}", e.message)))?;
            let Some(super::parse::Stmt::Chain(chain)) = stmts.into_iter().next() else {
                return Err(usage(name));
            };
            let start = host.ticks();
            let v = sh.capture(host, chain.into_iter().next().unwrap().1)?;
            let elapsed = host.ticks().saturating_sub(start).saturating_mul(10);
            let text = value::render(&v, sh.width);
            sh.out(&text);
            ok(Value::Str(format!(
                "Elapsed: {}\n",
                value::duration(elapsed)
            )))
        }
        "sleep" => {
            let a = args(raw, &[]);
            let d = a.pos.first().cloned().ok_or_else(|| usage(name))?;
            let ms = match d {
                Value::Duration(ms) => ms,
                other => (other.as_float().ok_or_else(|| usage(name))? * 1000.0) as u64,
            };
            Ok(Outcome::Yield(
                Step::Sleep {
                    ms: ms.min(24 * 3_600_000),
                    keys: false,
                },
                Pending::Nothing,
            ))
        }
        "pause" => {
            sh.out("Press any key to continue . . . ");
            Ok(Outcome::Yield(Step::Input(InputKind::Key), Pending::Key))
        }
        "input" => {
            let a = args(raw, &[]);
            if let Some(p) = a.text(0) {
                sh.out(&p);
            }
            Ok(Outcome::Yield(Step::Input(InputKind::Line), Pending::Line))
        }
        "confirm" => {
            let a = args(raw, &[]);
            let q = a.text(0).unwrap_or_else(|| String::from("Continue?"));
            sh.out(&format!("{q} (y/n) "));
            Ok(Outcome::Yield(
                Step::Input(InputKind::Key),
                Pending::Confirm(q),
            ))
        }
        "date" => {
            let t = host.now();
            let a = args(raw, &[]);
            if a.has(&["r", "record"]) {
                return ok(Value::Record(alloc::vec![
                    (String::from("year"), Value::Int(i64::from(t.year))),
                    (String::from("month"), Value::Int(i64::from(t.month))),
                    (String::from("day"), Value::Int(i64::from(t.day))),
                    (String::from("hour"), Value::Int(i64::from(t.hour))),
                    (String::from("minute"), Value::Int(i64::from(t.minute))),
                    (String::from("second"), Value::Int(i64::from(t.second))),
                    (
                        String::from("weekday"),
                        Value::str(
                            [
                                "Sunday",
                                "Monday",
                                "Tuesday",
                                "Wednesday",
                                "Thursday",
                                "Friday",
                                "Saturday"
                            ][t.weekday as usize % 7]
                        )
                    ),
                ]));
            }
            ok(Value::Str(format!(
                "{} {}",
                crate::format::date_long(&t),
                crate::format::time_seconds(&t)
            )))
        }
        "clear" => {
            sh.clear();
            none()
        }
        "title" => {
            let t = raw.iter().map(value::inline).collect::<Vec<_>>().join(" ");
            sh.set_title(if t.is_empty() { "Doorsh" } else { &t });
            none()
        }
        "clip" => {
            let text = if raw.is_empty() {
                text_of(&input)
            } else {
                raw.iter().map(value::inline).collect::<Vec<_>>().join(" ")
            };
            host.set_clipboard(&text)
                .map_err(|e| fail(format!("clip: {}", e.message().trim_end())))?;
            none()
        }
        "paste" => ok(host.clipboard().map_or(Value::Null, Value::Str)),
        "exit" => {
            let a = args(raw, &[]);
            Ok(Outcome::Exit(a.int(0)?.unwrap_or(0) as i32))
        }
        "version" => ok(Value::Record(alloc::vec![
            (String::from("doorsh"), Value::str(super::VERSION)),
            (String::from("doors"), {
                let (a, b, c, d) = crate::WINDOWS_VERSION;
                Value::Str(format!("{} {a}.{b}.{c}.{d}", crate::PRODUCT))
            }),
        ])),
        "help" => {
            let a = args(raw, &[]);
            ok(Value::Str(match a.text(0) {
                Some(cmd) => super::help::topic(&cmd),
                None => super::help::overview(),
            }))
        }
        "explain" => {
            let line = raw.iter().map(value::inline).collect::<Vec<_>>().join(" ");
            let stmts = super::parse::parse(&line)
                .map_err(|e| fail(format!("explain: {} (at column {})", e.message, e.at + 1)))?;
            let mut out = String::new();
            for s in &stmts {
                super::help::explain(sh, s, 0, &mut out);
            }
            ok(Value::Str(out))
        }
        "run" | "source" => {
            let a = args(raw, &[]);
            let script = a.text(0).ok_or_else(|| usage(name))?;
            let rest: Vec<String> = a.pos.iter().skip(1).map(value::inline).collect();
            Ok(Outcome::Script(script, rest, name == "run"))
        }
        // ---- network ----
        "ping" => {
            let a = args(raw, &["n", "count"]);
            let target = a.text(0).ok_or_else(|| usage(name))?;
            let count = a
                .opt("n")
                .or(a.opt("count"))
                .and_then(Value::as_int)
                .unwrap_or(4)
                .clamp(1, 20);
            let address = host
                .resolve(&target)
                .ok_or_else(|| fail(format!("ping: can't find {target}")))?;
            let ip = format!(
                "{}.{}.{}.{}",
                address[0], address[1], address[2], address[3]
            );
            let mut out = Vec::new();
            for seq in 1..=count {
                let reply = host
                    .ping(address, 32, 4000)
                    .map_err(|e| fail(format!("ping: {}", e.message().trim_end())))?;
                out.push(Value::Record(alloc::vec![
                    (String::from("seq"), Value::Int(seq)),
                    (String::from("address"), Value::Str(ip.clone())),
                    (
                        String::from("time"),
                        reply.map_or(Value::str("timed out"), Value::Duration)
                    ),
                ]));
            }
            sh.status = i32::from(
                out.iter()
                    .all(|r| matches!(r.field("time"), Some(Value::Str(_)))),
            );
            ok(Value::List(out))
        }
        "resolve" => {
            let a = args(raw, &[]);
            let target = a.text(0).ok_or_else(|| usage(name))?;
            let address = host
                .resolve(&target)
                .ok_or_else(|| fail(format!("can't find {target}")))?;
            ok(Value::Record(alloc::vec![
                (String::from("name"), Value::Str(target)),
                (
                    String::from("address"),
                    Value::Str(format!(
                        "{}.{}.{}.{}",
                        address[0], address[1], address[2], address[3]
                    ))
                ),
            ]))
        }
        "ip" => {
            let ip4 = |a: [u8; 4]| format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3]);
            ok(Value::List(
                host.adapters()
                    .into_iter()
                    .map(|a| {
                        Value::Record(alloc::vec![
                            (String::from("adapter"), Value::Str(a.name)),
                            (
                                String::from("address"),
                                Value::Str(a.ipv4.map(ip4).unwrap_or_default())
                            ),
                            (
                                String::from("gateway"),
                                Value::Str(a.gateway.map(ip4).unwrap_or_default())
                            ),
                            (
                                String::from("dns"),
                                Value::Str(
                                    a.dns.iter().map(|d| ip4(*d)).collect::<Vec<_>>().join(", ")
                                )
                            ),
                            (
                                String::from("mac"),
                                Value::Str(
                                    a.mac
                                        .iter()
                                        .map(|b| format!("{b:02X}"))
                                        .collect::<Vec<_>>()
                                        .join("-")
                                )
                            ),
                            (String::from("connected"), Value::Bool(a.connected)),
                        ])
                    })
                    .collect(),
            ))
        }
        _ => Err(fail(format!("'{name}' is not implemented"))),
    }
}
/// A yielded built-in continues.
pub fn resumed(sh: &mut Doorsh, host: &mut dyn Host, pending: Pending, r: Resume) -> Res<Value> {
    match (pending, r) {
        (Pending::Line, Resume::Line(text)) => Ok(Value::Str(text)),
        (Pending::Key, _) => {
            sh.out("\n");
            Ok(Value::Null)
        }
        (Pending::Confirm(_), Resume::Key(c)) => {
            sh.out_line(&c.to_string());
            Ok(Value::Bool(c.eq_ignore_ascii_case(&'y')))
        }
        (Pending::Remove(targets), Resume::Key(c)) => {
            sh.out_line(&c.to_string());
            if c.eq_ignore_ascii_case(&'y') {
                remove(host, &targets)?;
            } else {
                sh.status = 1;
            }
            Ok(Value::Null)
        }
        (_, Resume::Done(false)) => Err(fail("that couldn't be opened")),
        _ => Ok(Value::Null),
    }
}
fn remove(host: &mut dyn Host, targets: &[(String, bool)]) -> Res<()> {
    for (native, tree) in targets {
        if *tree {
            remove_tree(host, native, 0)?;
        } else {
            let r = match host.stat(native) {
                Ok(m) if m.dir => host.remove_dir(native),
                _ => host.remove_file(native),
            };
            r.map_err(|e| io(&host.windows(native), e))?;
        }
    }
    Ok(())
}
fn remove_tree(host: &mut dyn Host, native: &str, depth: usize) -> Res<()> {
    if depth > 64 {
        return Err(fail("the folder tree is too deep"));
    }
    let entries = host
        .list(native)
        .map_err(|e| io(&host.windows(native), e))?;
    for e in entries {
        let child = format!("{native}/{}", e.name);
        if e.meta.dir {
            remove_tree(host, &child, depth + 1)?;
        } else {
            host.remove_file(&child)
                .map_err(|err| io(&host.windows(&child), err))?;
        }
    }
    host.remove_dir(native)
        .map_err(|e| io(&host.windows(native), e))
}
fn copy_tree(host: &mut dyn Host, from: &str, to: &str) -> Res<()> {
    if host.stat(to).is_err() {
        host.mkdir(to).map_err(|e| io(&host.windows(to), e))?;
    }
    for e in host.list(from).map_err(|e| io(&host.windows(from), e))? {
        let (src, dst) = (format!("{from}/{}", e.name), format!("{to}/{}", e.name));
        if e.meta.dir {
            copy_tree(host, &src, &dst)?;
        } else {
            host.copy_file(&src, &dst)
                .map_err(|err| io(&host.windows(&src), err))?;
        }
    }
    Ok(())
}
fn ls(sh: &mut Doorsh, host: &mut dyn Host, raw: &[Value]) -> Res<Outcome> {
    let a = args(raw, &[]);
    let all = a.has(&["a", "all"]);
    let long = a.has(&["l", "long"]);
    let targets: Vec<String> = if a.pos.is_empty() {
        alloc::vec![String::from(".")]
    } else {
        a.pos.iter().map(value::inline).collect()
    };
    let mut out = Vec::new();
    for t in targets {
        let (full, native) = sh.native(host, &t)?;
        let m = host.stat(&native).map_err(|e| io(&t, e))?;
        let entries = if m.dir {
            host.list(&native).map_err(|e| io(&t, e))?
        } else {
            alloc::vec![crate::host::Entry {
                name: String::from(crate::wpath::name(&full)),
                meta: m
            }]
        };
        let mut entries: Vec<_> = entries
            .into_iter()
            .filter(|e| all || (e.meta.attrs & attr::HIDDEN == 0 && !e.name.starts_with('.')))
            .collect();
        // Folders first, then by name, as File Explorer lists.
        entries.sort_by(|x, y| {
            y.meta
                .dir
                .cmp(&x.meta.dir)
                .then_with(|| x.name.to_lowercase().cmp(&y.name.to_lowercase()))
        });
        for e in entries {
            let mut fields = alloc::vec![
                (String::from("name"), Value::Str(e.name.clone())),
                (
                    String::from("type"),
                    Value::str(if e.meta.dir { "folder" } else { "file" })
                ),
                (
                    String::from("size"),
                    if e.meta.dir {
                        Value::Null
                    } else {
                        Value::Size(e.meta.size)
                    }
                ),
                (
                    String::from("modified"),
                    Value::Str(time_text(&host.local(e.meta.modified)))
                ),
            ];
            if long {
                let mut flags = String::new();
                for (bit, c) in [
                    (attr::READONLY, 'r'),
                    (attr::HIDDEN, 'h'),
                    (attr::SYSTEM, 's'),
                    (attr::ARCHIVE, 'a'),
                ] {
                    flags.push(if e.meta.attrs & bit != 0 { c } else { '-' });
                }
                fields.push((String::from("attributes"), Value::Str(flags)));
                fields.push((
                    String::from("created"),
                    Value::Str(time_text(&host.local(e.meta.created))),
                ));
            }
            out.push(Value::Record(fields));
        }
    }
    Ok(Outcome::Value(Value::List(out)))
}
fn tree(
    host: &mut dyn Host,
    native: &str,
    prefix: &str,
    depth: usize,
    files: bool,
    out: &mut String,
    count: &mut usize,
) {
    if depth == 0 || *count > 5000 {
        return;
    }
    let Ok(mut entries) = host.list(native) else {
        return;
    };
    entries.retain(|e| files || e.meta.dir);
    entries.sort_by_key(|e| e.name.to_lowercase());
    let n = entries.len();
    for (i, e) in entries.into_iter().enumerate() {
        *count += 1;
        let last = i + 1 == n;
        out.push_str(&format!(
            "{prefix}{}{}\n",
            if last { "\\---" } else { "+---" },
            e.name
        ));
        if e.meta.dir {
            let child = format!("{native}/{}", e.name);
            let next = format!("{prefix}{}", if last { "    " } else { "|   " });
            tree(host, &child, &next, depth - 1, files, out, count);
        }
    }
}
fn du(host: &mut dyn Host, native: &str, depth: usize) -> (u64, u64, u64) {
    if depth > 64 {
        return (0, 0, 0);
    }
    let Ok(entries) = host.list(native) else {
        return (0, 0, 0);
    };
    let mut total = (0, 0, 0);
    for e in entries {
        if e.meta.dir {
            let (s, f, d) = du(host, &format!("{native}/{}", e.name), depth + 1);
            total = (total.0 + s, total.1 + f, total.2 + d + 1);
        } else {
            total = (total.0 + e.meta.size, total.1 + 1, total.2);
        }
    }
    total
}
#[allow(clippy::too_many_arguments)]
fn find(
    host: &mut dyn Host,
    native: &str,
    shown: &str,
    pattern: &str,
    want: Option<&str>,
    depth: usize,
    out: &mut Vec<Value>,
) {
    if depth == 0 || out.len() >= 10_000 {
        return;
    }
    let Ok(entries) = host.list(native) else {
        return;
    };
    for e in entries {
        let path = crate::wpath::join(shown, &e.name);
        let kind_ok = match want {
            Some("f" | "file") => !e.meta.dir,
            Some("d" | "folder" | "dir") => e.meta.dir,
            _ => true,
        };
        if kind_ok && crate::wild::matches(pattern, &e.name) {
            out.push(Value::Record(alloc::vec![
                (String::from("path"), Value::Str(path.clone())),
                (
                    String::from("type"),
                    Value::str(if e.meta.dir { "folder" } else { "file" })
                ),
                (
                    String::from("size"),
                    if e.meta.dir {
                        Value::Null
                    } else {
                        Value::Size(e.meta.size)
                    }
                ),
            ]));
        }
        if e.meta.dir {
            find(
                host,
                &format!("{native}/{}", e.name),
                &path,
                pattern,
                want,
                depth - 1,
                out,
            );
        }
    }
}
/// Text search: `*`/`?` wildcards, `^` and `$` anchors, else a substring.
fn matches(line: &str, pattern: &str, insensitive: bool) -> bool {
    let (l, p) = if insensitive {
        (line.to_lowercase(), pattern.to_lowercase())
    } else {
        (String::from(line), String::from(pattern))
    };
    let anchored_start = p.starts_with('^');
    let anchored_end = p.ends_with('$') && p.len() > 1;
    let core = p.trim_start_matches('^').trim_end_matches('$');
    if core.contains(['*', '?']) {
        let wrapped = format!(
            "{}{core}{}",
            if anchored_start { "" } else { "*" },
            if anchored_end { "" } else { "*" }
        );
        return crate::wild::matches(&wrapped, &l);
    }
    match (anchored_start, anchored_end) {
        (true, true) => l == core,
        (true, false) => l.starts_with(core),
        (false, true) => l.ends_with(core),
        _ => l.contains(core),
    }
}
fn map_text(input: Value, f: impl Fn(&str) -> String) -> Res<Outcome> {
    ok(match input {
        Value::List(items) => Value::List(
            items
                .into_iter()
                .map(|v| Value::Str(f(&value::inline(&v))))
                .collect(),
        ),
        Value::Null => Value::Null,
        other => Value::Str(f(&other.to_text())),
    })
}
fn diff(a: &str, b: &str) -> String {
    let (x, y): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    if x.len() * y.len() > 4_000_000 {
        return String::from("The files differ (too large to compare line by line).\n");
    }
    // Longest common subsequence, then the edit script.
    let mut lcs = alloc::vec![alloc::vec![0u32; y.len() + 1]; x.len() + 1];
    for i in (0..x.len()).rev() {
        for j in (0..y.len()).rev() {
            lcs[i][j] = if x[i] == y[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = String::new();
    while i < x.len() || j < y.len() {
        if i < x.len() && j < y.len() && x[i] == y[j] {
            i += 1;
            j += 1;
        } else if j < y.len() && (i == x.len() || lcs[i][j + 1] >= lcs[i + 1][j]) {
            out.push_str(&format!("+ {}\n", y[j]));
            j += 1;
        } else {
            out.push_str(&format!("- {}\n", x[i]));
            i += 1;
        }
    }
    out
}
fn from_csv(text: &str) -> Value {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let Some(header) = lines.next() else {
        return Value::List(Vec::new());
    };
    let names = csv_fields(header);
    Value::List(
        lines
            .map(|l| {
                let cells = csv_fields(l);
                Value::Record(
                    names
                        .iter()
                        .enumerate()
                        .map(|(i, n)| {
                            (
                                n.clone(),
                                cells
                                    .get(i)
                                    .map(|c| {
                                        value::parse_number(c)
                                            .unwrap_or_else(|| Value::Str(c.clone()))
                                    })
                                    .unwrap_or(Value::Null),
                            )
                        })
                        .collect(),
                )
            })
            .collect(),
    )
}
fn csv_fields(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(core::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out
}
fn to_csv(v: &Value) -> String {
    let cell = |v: &Value| {
        let s = value::inline(v);
        if s.contains([',', '"', '\n']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s
        }
    };
    match v {
        Value::List(rows) if v.is_table() => {
            let mut columns: Vec<String> = Vec::new();
            for r in rows {
                if let Value::Record(f) = r {
                    for (k, _) in f {
                        if !columns.contains(k) {
                            columns.push(k.clone());
                        }
                    }
                }
            }
            let mut out = columns.join(",") + "\n";
            for r in rows {
                out.push_str(
                    &columns
                        .iter()
                        .map(|c| r.field(c).map(cell).unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join(","),
                );
                out.push('\n');
            }
            out
        }
        Value::List(items) => items.iter().map(cell).collect::<Vec<_>>().join("\n") + "\n",
        other => cell(other) + "\n",
    }
}
const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
fn base64_encode(data: &[u8]) -> String {
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0;
    for c in text.bytes().filter(|c| !c.is_ascii_whitespace()) {
        if c == b'=' {
            break;
        }
        let v = B64.iter().position(|b| *b == c)? as u32;
        buf = buf << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

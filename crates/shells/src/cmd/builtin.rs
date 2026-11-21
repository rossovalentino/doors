//! cmd.exe's internal commands, with the messages and formats Windows 11
//! prints (captured from the owner's reference installation). Each takes
//! its raw argument text, as cmd.exe's do.
use super::{Cmd, Event, Frame, crlf_to_lf, help};
use crate::format::{date, date_long, grouped, time_long, time_short};
use crate::host::{FsError, Host, Meta, attr};
use crate::{InputKind, Launch, Resume, Step, wild, wpath};
use alloc::{format, string::String, vec, vec::Vec};

/// Internal command names (cmd.exe's own table).
pub(crate) const NAMES: [&str; 44] = [
    "assoc", "break", "call", "cd", "chdir", "cls", "color", "copy", "date", "del", "dir", "dpath",
    "echo", "endlocal", "erase", "exit", "ftype", "goto", "md", "mkdir", "mklink", "move", "path",
    "pause", "popd", "prompt", "pushd", "rd", "rem", "ren", "rename", "rmdir", "set", "setlocal",
    "shift", "start", "time", "title", "type", "ver", "verify", "vol", "keys", "for",
];

pub(crate) enum Outcome {
    /// Finished; the flag is the command's success for `&&`/`||`.
    Done(bool),
    /// Waiting for input or time; `resume` continues `Pending`.
    Wait(Pending, Step),
    /// A program, launch or exit for the caller.
    Step(Step),
    /// Frames changed (a batch file started, GOTO, CALL): keep stepping.
    Continue,
}
pub(crate) enum Pending {
    Pause,
    SetPrompt(String),
    /// "Are you sure (Y/N)?" before deleting these files.
    Delete {
        files: Vec<(String, String)>,
        force: bool,
        subdirs: bool,
    },
    /// RD /S without /Q: the tree to remove after the answer.
    RemoveTree {
        native: String,
        display: String,
    },
    /// COPY/MOVE asked "Overwrite ...? (Yes/No/All)".
    Transfer(Transfer),
    /// DATE/TIME without /T asked for a new value.
    SetClock,
    Tool(super::tools::Pending),
}
pub(crate) struct Transfer {
    pub moving: bool,
    pub items: Vec<(String, String, String)>,
    pub index: usize,
    pub all: bool,
    pub done: usize,
    pub dirs: bool,
}

/// Split arguments: delimiters are spaces, tabs, `,`, `;`, `=`; quotes
/// group and stay.
pub(crate) fn tokens(tail: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in tail.chars() {
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
pub(crate) fn unq(s: &str) -> String {
    s.replace('"', "")
}
/// The tail without its first delimiter character and outer spaces.
fn rest(tail: &str) -> &str {
    let t = tail
        .strip_prefix([' ', '\t', ',', ';', '='])
        .unwrap_or(tail);
    t.trim()
}
fn syntax(cmd: &mut Cmd, host: &mut dyn Host) -> Outcome {
    cmd.err_line(host, "The syntax of the command is incorrect.");
    cmd.errorlevel = 1;
    Outcome::Done(false)
}
fn fail(cmd: &mut Cmd, host: &mut dyn Host, e: FsError) -> Outcome {
    cmd.err_line(host, e.message());
    cmd.errorlevel = 1;
    Outcome::Done(false)
}
pub(crate) fn change_drive(cmd: &mut Cmd, host: &mut dyn Host, drive: char) -> Outcome {
    let d = drive.to_ascii_uppercase();
    let target = cmd
        .drives
        .iter()
        .find(|(x, _)| *x == d)
        .map(|(_, p)| p.clone())
        .unwrap_or_else(|| format!("{d}:\\"));
    match host.native(&target).map(|n| host.stat(&n)) {
        Some(Ok(m)) if m.dir => {
            remember_drive(cmd);
            cmd.cwd = target;
            Outcome::Done(true)
        }
        _ => {
            cmd.err_line(host, "The system cannot find the drive specified.");
            cmd.errorlevel = 1;
            Outcome::Done(false)
        }
    }
}
fn remember_drive(cmd: &mut Cmd) {
    if let Some(d) = wpath::drive(&cmd.cwd) {
        let cwd = cmd.cwd.clone();
        match cmd.drives.iter_mut().find(|(x, _)| *x == d) {
            Some(e) => e.1 = cwd,
            None => cmd.drives.push((d, cwd)),
        }
    }
}

pub(crate) fn run(cmd: &mut Cmd, host: &mut dyn Host, name: &str, tail: &str) -> Outcome {
    match name {
        "rem" => Outcome::Done(true),
        "echo" => echo(cmd, host, tail),
        "set" => set(cmd, host, tail),
        "cd" | "chdir" => cd(cmd, host, tail),
        "cls" => {
            cmd.events.push(Event::Clear);
            Outcome::Done(true)
        }
        "dir" => dir(cmd, host, tail),
        "type" => type_(cmd, host, tail),
        "md" | "mkdir" => mkdir(cmd, host, tail),
        "rd" | "rmdir" => rmdir(cmd, host, tail),
        "del" | "erase" => del(cmd, host, tail),
        "ren" | "rename" => ren(cmd, host, tail),
        "copy" => transfer(cmd, host, tail, false),
        "move" => transfer(cmd, host, tail, true),
        "pushd" => pushd(cmd, host, tail),
        "popd" => {
            if let Some(d) = cmd.pushd.pop() {
                remember_drive(cmd);
                cmd.cwd = d;
            }
            Outcome::Done(true)
        }
        "setlocal" => setlocal(cmd, host, tail),
        "endlocal" => {
            cmd.endlocal();
            Outcome::Done(true)
        }
        "shift" => shift(cmd, host, tail),
        "goto" => {
            let label = rest(tail);
            if label.is_empty() {
                cmd.err_line(host, "No batch label specified to GOTO command.");
                return Outcome::Done(false);
            }
            if cmd.batch_depth() == 0 && !label.eq_ignore_ascii_case(":eof") {
                return Outcome::Done(true);
            }
            cmd.goto(host, label);
            Outcome::Continue
        }
        "call" => call(cmd, host, tail),
        "exit" => exit(cmd, host, tail),
        "pause" => {
            cmd.out(host, "Press any key to continue . . . ");
            if let Some(input) = cmd.input() {
                let _ = input;
                cmd.out(host, "\n");
                return Outcome::Done(true);
            }
            Outcome::Wait(Pending::Pause, Step::Input(InputKind::Key))
        }
        "title" => {
            cmd.title = String::from(rest(tail));
            cmd.events.push(Event::Title(cmd.title.clone()));
            Outcome::Done(true)
        }
        "color" => color(cmd, host, tail),
        "ver" => {
            let (a, b, c, d) = crate::WINDOWS_VERSION;
            cmd.out(
                host,
                &format!("\n{} [Version {a}.{b}.{c}.{d}]\n", crate::PRODUCT),
            );
            Outcome::Done(true)
        }
        "vol" => vol(cmd, host, tail),
        "verify" => {
            match rest(tail).to_ascii_lowercase().as_str() {
                "" => {
                    let state = if cmd.verify { "on" } else { "off" };
                    cmd.out_line(host, &format!("VERIFY is {state}."));
                }
                "on" => cmd.verify = true,
                "off" => cmd.verify = false,
                _ => {
                    cmd.err_line(host, "Must specify ON or OFF.");
                    return Outcome::Done(false);
                }
            }
            Outcome::Done(true)
        }
        "break" => {
            match rest(tail).to_ascii_lowercase().as_str() {
                "" => {
                    let state = if cmd.brk { "on" } else { "off" };
                    cmd.out_line(host, &format!("BREAK is {state}"));
                }
                "on" => cmd.brk = true,
                "off" => cmd.brk = false,
                _ => {
                    cmd.err_line(host, "Must specify ON or OFF.");
                    return Outcome::Done(false);
                }
            }
            Outcome::Done(true)
        }
        "keys" => {
            cmd.out_line(host, "KEYS is on");
            Outcome::Done(true)
        }
        "path" | "dpath" => path(cmd, host, name, tail),
        "prompt" => {
            let p = rest(tail);
            if p.is_empty() {
                cmd.env.set("PROMPT", "$P$G");
            } else {
                cmd.env.set("PROMPT", p);
            }
            Outcome::Done(true)
        }
        "date" | "time" => clock(cmd, host, name, tail),
        "start" => start(cmd, host, tail),
        "assoc" | "ftype" => assoc(cmd, host, name, tail),
        "mklink" => {
            let t = tokens(tail);
            if t.len() < 2 {
                return syntax(cmd, host);
            }
            cmd.err_line(
                host,
                "You do not have sufficient privilege to perform this operation.",
            );
            cmd.errorlevel = 1;
            Outcome::Done(false)
        }
        "for" => syntax(cmd, host),
        _ => syntax(cmd, host),
    }
}

/// Continue a command that waited for input or time.
pub(crate) fn resume(cmd: &mut Cmd, host: &mut dyn Host, pending: Pending, r: Resume) -> Outcome {
    match pending {
        Pending::Pause => {
            cmd.out(host, "\n");
            Outcome::Done(true)
        }
        Pending::SetPrompt(var) => {
            let line = match r {
                Resume::Line(l) => l,
                Resume::Key(c) => String::from(c),
                _ => String::new(),
            };
            if line.is_empty() {
                cmd.errorlevel = 1;
                return Outcome::Done(false);
            }
            cmd.env.set(&var, &line);
            Outcome::Done(true)
        }
        Pending::Delete {
            files,
            force,
            subdirs,
        } => {
            let yes = answer(&r) == Some('y');
            if yes {
                delete_files(cmd, host, &files, force, subdirs);
            }
            Outcome::Done(true)
        }
        Pending::RemoveTree { native, display } => {
            let _ = display;
            if answer(&r) == Some('y')
                && let Err(e) = remove_tree(host, &native)
            {
                return fail(cmd, host, e);
            }
            Outcome::Done(true)
        }
        Pending::Transfer(mut t) => {
            let choice = match &r {
                Resume::Line(l) => l.trim().chars().next().map(|c| c.to_ascii_lowercase()),
                Resume::Key(c) => Some(c.to_ascii_lowercase()),
                _ => None,
            };
            match choice {
                Some('y') => {
                    let (from, to, _) = t.items[t.index].clone();
                    if do_transfer(host, &from, &to, t.moving, true).is_ok() {
                        t.done += 1;
                    }
                    t.index += 1;
                }
                Some('a') => {
                    t.all = true;
                }
                Some('n') => t.index += 1,
                _ => {
                    // Asked again, as cmd.exe does for other answers.
                    let display = t.items[t.index].2.clone();
                    cmd.out(host, &format!("Overwrite {display}? (Yes/No/All): "));
                    return Outcome::Wait(Pending::Transfer(t), Step::Input(InputKind::Line));
                }
            }
            continue_transfer(cmd, host, t)
        }
        Pending::SetClock => {
            if let Resume::Line(l) = r
                && !l.trim().is_empty()
            {
                cmd.err_line(host, "A required privilege is not held by the client.");
                cmd.errorlevel = 1;
                return Outcome::Done(false);
            }
            Outcome::Done(true)
        }
        Pending::Tool(p) => super::tools::resume(cmd, host, p, r),
    }
}
fn answer(r: &Resume) -> Option<char> {
    match r {
        Resume::Key(c) => Some(c.to_ascii_lowercase()),
        Resume::Line(l) => l.trim().chars().next().map(|c| c.to_ascii_lowercase()),
        _ => None,
    }
}

fn echo(cmd: &mut Cmd, host: &mut dyn Host, tail: &str) -> Outcome {
    if tail.trim().is_empty() {
        let state = if cmd.echo { "on" } else { "off" };
        cmd.out_line(host, &format!("ECHO is {state}."));
        return Outcome::Done(true);
    }
    let first = tail.chars().next().unwrap();
    let text = if matches!(
        first,
        ' ' | '\t' | '.' | ',' | ';' | '=' | ':' | '/' | '\\' | '(' | '[' | ']' | '+'
    ) {
        &tail[first.len_utf8()..]
    } else {
        tail
    };
    let word = text.trim().to_ascii_lowercase();
    if first == ' ' || first == '\t' {
        if word == "on" {
            cmd.echo = true;
            return Outcome::Done(true);
        }
        if word == "off" {
            cmd.echo = false;
            return Outcome::Done(true);
        }
        if text.trim() == "/?" {
            cmd.out(host, &help::text("echo"));
            return Outcome::Done(true);
        }
    }
    cmd.out_line(host, text);
    Outcome::Done(true)
}

fn set(cmd: &mut Cmd, host: &mut dyn Host, tail: &str) -> Outcome {
    let text = rest(tail);
    let lower = text.to_ascii_lowercase();
    if let Some(expr) = lower.strip_prefix("/a").map(|_| text[2..].trim()) {
        if expr.is_empty() {
            return syntax(cmd, host);
        }
        struct V<'a>(&'a mut Cmd, &'a dyn Host);
        impl super::arith::Vars for V<'_> {
            fn get(&mut self, name: &str) -> Option<String> {
                self.0.var(self.1, name)
            }
            fn set(&mut self, name: &str, value: i32) {
                self.0.env.set(name, &format!("{value}"));
            }
        }
        let interactive = cmd.batch_depth() == 0;
        let result = super::arith::eval(expr, &mut V(cmd, host));
        return match result {
            Ok(v) => {
                if interactive {
                    cmd.out(host, &format!("{v}"));
                }
                Outcome::Done(true)
            }
            Err(e) => {
                cmd.err_line(host, &e.message());
                cmd.errorlevel = if matches!(e, super::arith::Error::DivideByZero) {
                    1073750993
                } else {
                    1073750988
                };
                Outcome::Done(false)
            }
        };
    }
    if lower.starts_with("/p") {
        let spec = text[2..].trim_start();
        let spec = unquote_set(spec);
        let Some((name, prompt)) = spec.split_once('=') else {
            return syntax(cmd, host);
        };
        if name.is_empty() {
            return syntax(cmd, host);
        }
        cmd.out(host, prompt);
        if let Some(input) = cmd.input() {
            let text = crlf_to_lf(&input);
            let line = text.lines().next().unwrap_or("");
            if line.is_empty() {
                cmd.errorlevel = 1;
                return Outcome::Done(false);
            }
            cmd.env.set(name, line);
            return Outcome::Done(true);
        }
        return Outcome::Wait(
            Pending::SetPrompt(String::from(name)),
            Step::Input(InputKind::Line),
        );
    }
    if text.is_empty() {
        let pairs: Vec<String> = cmd
            .env
            .iter()
            .filter(|(n, _)| !n.starts_with('='))
            .map(|(n, v)| format!("{n}={v}"))
            .collect();
        for p in pairs {
            cmd.out_line(host, &p);
        }
        return Outcome::Done(true);
    }
    let spec = unquote_set(text);
    match spec.split_once('=') {
        None => {
            // A prefix listing.
            let prefix = spec.trim().to_uppercase();
            let pairs: Vec<String> = cmd
                .env
                .iter()
                .filter(|(n, _)| n.to_uppercase().starts_with(&prefix))
                .map(|(n, v)| format!("{n}={v}"))
                .collect();
            if pairs.is_empty() {
                cmd.err_line(
                    host,
                    &format!("Environment variable {} not defined", spec.trim()),
                );
                cmd.errorlevel = 1;
                return Outcome::Done(false);
            }
            for p in pairs {
                cmd.out_line(host, &p);
            }
            Outcome::Done(true)
        }
        Some(("", _)) => syntax(cmd, host),
        Some((name, value)) => {
            if value.is_empty() {
                cmd.env.remove(name);
            } else {
                cmd.env.set(name, value);
            }
            Outcome::Done(true)
        }
    }
}
/// `set "name=value"`: the text between the first quote and the last.
fn unquote_set(text: &str) -> &str {
    if let Some(inner) = text.strip_prefix('"')
        && let Some(end) = inner.rfind('"')
    {
        return &inner[..end];
    }
    text
}

fn cd(cmd: &mut Cmd, host: &mut dyn Host, tail: &str) -> Outcome {
    let mut text = rest(tail).to_owned_s();
    let mut switch_drive = false;
    if text.len() >= 2 && text[..2].eq_ignore_ascii_case("/d") {
        switch_drive = true;
        text = text[2..].trim().to_owned_s();
    }
    let text = unq(&text);
    let text = text.trim_end();
    if text.is_empty() {
        let cwd = cmd.cwd.clone();
        cmd.out_line(host, &cwd);
        return Outcome::Done(true);
    }
    // `cd D:` prints that drive's directory.
    if text.len() == 2 && text.ends_with(':') {
        let d = text.as_bytes()[0].to_ascii_uppercase() as char;
        let dir = if Some(d) == wpath::drive(&cmd.cwd) {
            cmd.cwd.clone()
        } else {
            cmd.drives
                .iter()
                .find(|(x, _)| *x == d)
                .map(|(_, p)| p.clone())
                .unwrap_or_else(|| format!("{d}:\\"))
        };
        cmd.out_line(host, &dir);
        return Outcome::Done(true);
    }
    let full = match cmd.full(text) {
        Ok(f) => f,
        Err(_) => return fail(cmd, host, FsError::PathNotFound),
    };
    let Some(native) = host.native(&full) else {
        return fail(cmd, host, FsError::PathNotFound);
    };
    match host.stat(&native) {
        Ok(m) if m.dir => (),
        Ok(_) => {
            cmd.err_line(host, "The directory name is invalid.");
            cmd.errorlevel = 1;
            return Outcome::Done(false);
        }
        Err(_) => return fail(cmd, host, FsError::PathNotFound),
    }
    // The on-disk case of each component, as cmd.exe shows it.
    let display = host.windows(&native);
    let target = if display.len() == full.len() && display.eq_ignore_ascii_case(&full) {
        display
    } else {
        full
    };
    let drive = wpath::drive(&target);
    if drive == wpath::drive(&cmd.cwd) || switch_drive {
        remember_drive(cmd);
        cmd.cwd = target;
    } else if let Some(d) = drive {
        match cmd.drives.iter_mut().find(|(x, _)| *x == d) {
            Some(e) => e.1 = target,
            None => cmd.drives.push((d, target)),
        }
    }
    Outcome::Done(true)
}
trait OwnedS {
    fn to_owned_s(&self) -> String;
}
impl OwnedS for str {
    fn to_owned_s(&self) -> String {
        String::from(self)
    }
}
fn pushd(cmd: &mut Cmd, host: &mut dyn Host, tail: &str) -> Outcome {
    let target = unq(rest(tail));
    if target.is_empty() {
        let list: Vec<String> = cmd.pushd.iter().rev().cloned().collect();
        for d in list {
            cmd.out_line(host, &d);
        }
        return Outcome::Done(true);
    }
    let saved = cmd.cwd.clone();
    let ok = matches!(cd(cmd, host, &format!(" /d {target}")), Outcome::Done(true));
    if ok {
        cmd.pushd.push(saved);
    }
    Outcome::Done(ok)
}
fn setlocal(cmd: &mut Cmd, host: &mut dyn Host, tail: &str) -> Outcome {
    if cmd.batch_depth() == 0 {
        return Outcome::Done(true);
    }
    let mut delayed = cmd.delayed;
    let mut extensions = cmd.extensions;
    for t in tokens(tail) {
        match t.to_ascii_lowercase().as_str() {
            "enabledelayedexpansion" => delayed = true,
            "disabledelayedexpansion" => delayed = false,
            "enableextensions" => extensions = true,
            "disableextensions" => extensions = false,
            _ => {
                cmd.err_line(host, "Invalid parameter to SETLOCAL command");
                cmd.errorlevel = 1;
                return Outcome::Done(false);
            }
        }
    }
    cmd.setlocal();
    cmd.delayed = delayed;
    cmd.extensions = extensions;
    cmd.errorlevel = 0;
    Outcome::Done(true)
}
fn shift(cmd: &mut Cmd, host: &mut dyn Host, tail: &str) -> Outcome {
    let arg = rest(tail);
    let from = match arg.strip_prefix('/') {
        Some(n) => match n.parse::<usize>() {
            Ok(n) if n <= 8 => n,
            _ => {
                cmd.err_line(host, "Invalid parameter to SHIFT command");
                return Outcome::Done(false);
            }
        },
        None => 0,
    };
    if let Some(b) = cmd.current_batch() {
        if from == 0 {
            b.shift += 1;
        } else if b.shift + from < b.args.len() {
            b.args.remove(b.shift + from);
        }
    }
    Outcome::Done(true)
}
fn call(cmd: &mut Cmd, host: &mut dyn Host, tail: &str) -> Outcome {
    let text = rest(tail);
    if text.is_empty() {
        return Outcome::Done(true);
    }
    // CALL :label args -> a new batch context in the same file.
    if text.starts_with(':') {
        if cmd.batch_depth() == 0 {
            cmd.err_line(
                host,
                "Invalid attempt to call batch label outside of batch script.",
            );
            return Outcome::Done(false);
        }
        let mut words = super::batch_words(text);
        let label = words.remove(0);
        let (path, file) = match cmd.current_batch() {
            Some(b) => (b.path.clone(), b.text.clone()),
            None => return Outcome::Done(false),
        };
        let mut args = vec![label.clone()];
        args.extend(words);
        cmd.advance_pc();
        let depth = cmd.redirs.len();
        let redir_depth = cmd.redirs.len();
        cmd.frames.push(Frame::Batch(super::Batch {
            path,
            text: file,
            pos: 0,
            args,
            shift: 0,
            redir_depth,
            prompt_echo: None,
        }));
        let _ = depth;
        if !cmd.goto(host, &label) {
            return Outcome::Continue;
        }
        return Outcome::Continue;
    }
    // A second percent expansion, then the command (a batch file returns).
    let args = cmd.batch_args();
    let expanded = {
        let mut c = super::Ctx { cmd, host };
        super::expand::percent(
            text,
            &mut c,
            if args.is_empty() { None } else { Some(&args) },
        )
    };
    let node = match super::parse::parse(&expanded) {
        Ok(n) => n,
        Err(_) => return syntax(cmd, host),
    };
    let super::parse::Node::Simple(s) = node else {
        // CALL does not run IF, FOR or blocks.
        return syntax(cmd, host);
    };
    let head = unq(&s.head);
    let lower = head.to_ascii_lowercase();
    if NAMES.contains(&lower.as_str()) {
        return run(cmd, host, &lower, &s.tail);
    }
    if let Some(super::Found::Batch(path)) = cmd.resolve(host, &head) {
        let words = super::batch_words(&s.tail);
        return match cmd.open_batch(host, &path, &words) {
            Ok(b) => {
                cmd.advance_pc();
                cmd.frames.push(Frame::Batch(b));
                Outcome::Continue
            }
            Err(e) => fail(cmd, host, e),
        };
    }
    cmd.external(host, &s.head, &s.tail)
}
fn exit(cmd: &mut Cmd, host: &mut dyn Host, tail: &str) -> Outcome {
    let t = tokens(tail);
    let batch = t.first().is_some_and(|a| a.eq_ignore_ascii_case("/b"));
    let code_text = if batch { t.get(1) } else { t.first() };
    let code = code_text.and_then(|c| c.parse::<i32>().ok());
    if let Some(c) = code {
        cmd.errorlevel = c;
    }
    if cmd.exit_child(code.unwrap_or(cmd.errorlevel)) {
        return Outcome::Continue;
    }
    if batch && cmd.batch_depth() > 0 {
        cmd.exit_batch();
        return Outcome::Continue;
    }
    let _ = host;
    Outcome::Step(Step::Exit(code.unwrap_or(cmd.errorlevel)))
}
fn color(cmd: &mut Cmd, host: &mut dyn Host, tail: &str) -> Outcome {
    let arg = rest(tail);
    let value = if arg.is_empty() {
        Some(0x07)
    } else {
        u8::from_str_radix(arg, 16).ok().filter(|_| arg.len() <= 2)
    };
    match value {
        Some(v) if v >> 4 != v & 0xF => {
            cmd.events.push(Event::Color(v));
            Outcome::Done(true)
        }
        _ => {
            let _ = host;
            cmd.errorlevel = 1;
            Outcome::Done(false)
        }
    }
}
fn vol(cmd: &mut Cmd, host: &mut dyn Host, tail: &str) -> Outcome {
    let arg = unq(rest(tail));
    let drive = if arg.is_empty() {
        wpath::drive(&cmd.cwd).unwrap_or('C')
    } else {
        arg.chars().next().unwrap().to_ascii_uppercase()
    };
    let Some(v) = host.volume(drive) else {
        cmd.err_line(host, "The system cannot find the drive specified.");
        return Outcome::Done(false);
    };
    let label = if v.label.is_empty() {
        format!(" Volume in drive {drive} has no label.")
    } else {
        format!(" Volume in drive {drive} is {}", v.label)
    };
    cmd.out_line(host, &label);
    cmd.out_line(
        host,
        &format!(
            " Volume Serial Number is {:04X}-{:04X}",
            v.serial >> 16,
            v.serial & 0xFFFF
        ),
    );
    Outcome::Done(true)
}
fn path(cmd: &mut Cmd, host: &mut dyn Host, name: &str, tail: &str) -> Outcome {
    let var = if name == "path" { "PATH" } else { "DPATH" };
    let arg = rest(tail);
    if arg.is_empty() {
        match cmd.env.get(var) {
            Some(v) => {
                let line = format!("{var}={v}");
                cmd.out_line(host, &line);
            }
            None => cmd.out_line(host, if var == "PATH" { "No Path" } else { "No DPath" }),
        }
    } else if arg == ";" {
        cmd.env.remove(var);
    } else {
        cmd.env.set(var, arg);
    }
    Outcome::Done(true)
}
fn clock(cmd: &mut Cmd, host: &mut dyn Host, name: &str, tail: &str) -> Outcome {
    let now = host.now();
    let t = rest(tail);
    if t.eq_ignore_ascii_case("/t") {
        let text = if name == "date" {
            format!("{} ", date_long(&now))
        } else {
            time_short(&now)
        };
        cmd.out_line(host, &text);
        return Outcome::Done(true);
    }
    if !t.is_empty() {
        cmd.err_line(host, "A required privilege is not held by the client.");
        cmd.errorlevel = 1;
        return Outcome::Done(false);
    }
    if name == "date" {
        cmd.out_line(host, &format!("The current date is: {}", date_long(&now)));
        cmd.out(host, "Enter the new date: (mm-dd-yy) ");
    } else {
        cmd.out_line(
            host,
            &format!("The current time is: {}", time_long(&now).trim_start()),
        );
        cmd.out(host, "Enter the new time: ");
    }
    Outcome::Wait(Pending::SetClock, Step::Input(InputKind::Line))
}
fn assoc(cmd: &mut Cmd, host: &mut dyn Host, name: &str, tail: &str) -> Outcome {
    let arg = rest(tail);
    let assoc = name == "assoc";
    if let Some((key, _)) = arg.split_once('=') {
        let _ = key;
        cmd.err_line(host, "Access is denied.");
        cmd.err_line(
            host,
            &format!(
                "Error occurred while processing: {}.",
                arg.split('=').next().unwrap_or("")
            ),
        );
        cmd.errorlevel = 1;
        return Outcome::Done(false);
    }
    let lookup = |host: &mut dyn Host, key: &str| -> Option<String> {
        let base = if assoc {
            format!("HKCR\\{key}")
        } else {
            format!("HKCR\\{key}\\shell\\open\\command")
        };
        host.reg_values(&base)
            .ok()?
            .into_iter()
            .find(|(n, _)| n.is_empty())
            .and_then(|(_, v)| match v {
                crate::host::RegValue::Sz(s) | crate::host::RegValue::ExpandSz(s) => Some(s),
                _ => None,
            })
    };
    if arg.is_empty() {
        let keys = host.reg_subkeys("HKCR").unwrap_or_default();
        for k in keys {
            if assoc != k.starts_with('.') {
                continue;
            }
            if let Some(v) = lookup(host, &k) {
                cmd.out_line(host, &format!("{k}={v}"));
            }
        }
        return Outcome::Done(true);
    }
    match lookup(host, arg) {
        Some(v) => {
            cmd.out_line(host, &format!("{arg}={v}"));
            Outcome::Done(true)
        }
        None => {
            if assoc {
                cmd.err_line(
                    host,
                    &format!("File association not found for extension {arg}"),
                );
            } else {
                cmd.err_line(
                    host,
                    &format!("File type '{arg}' not found or no open command associated with it."),
                );
            }
            cmd.errorlevel = if assoc { 1 } else { 2 };
            Outcome::Done(false)
        }
    }
}
fn start(cmd: &mut Cmd, host: &mut dyn Host, tail: &str) -> Outcome {
    let mut t = tokens(tail);
    // The first quoted argument is the window title.
    if t.first().is_some_and(|a| a.starts_with('"')) {
        t.remove(0);
    }
    let mut wait = false;
    let mut dir: Option<String> = None;
    while let Some(first) = t.first() {
        let lower = first.to_ascii_lowercase();
        if !lower.starts_with('/') {
            break;
        }
        match lower.as_str() {
            "/wait" => wait = true,
            "/d" => {
                t.remove(0);
                dir = t.first().map(|d| unq(d));
            }
            "/b" | "/i" | "/min" | "/max" | "/low" | "/normal" | "/high" | "/realtime"
            | "/abovenormal" | "/belownormal" | "/shared" | "/separate" => (),
            _ if lower.starts_with("/d") => dir = Some(unq(&first[2..])),
            _ => (),
        }
        t.remove(0);
    }
    let cwd = match dir {
        Some(d) => cmd.native(host, &d).unwrap_or_default(),
        None => host.native(&cmd.cwd).unwrap_or_default(),
    };
    let Some(target) = t.first().cloned() else {
        // `start` alone opens a new Command Prompt.
        return Outcome::Step(Step::Launch(Launch::Terminal {
            profile: crate::Profile::Cmd,
            cwd,
            script: None,
        }));
    };
    let args = t[1..].join(" ");
    let name = unq(&target);
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www.") {
        return Outcome::Step(Step::Launch(Launch::Url(name)));
    }
    if matches!(lower.as_str(), "cmd" | "cmd.exe") {
        return Outcome::Step(Step::Launch(Launch::Terminal {
            profile: crate::Profile::Cmd,
            cwd,
            script: None,
        }));
    }
    if let Some(target) = crate::legacy::resolve(&name, &format!("{name} {args}")) {
        return match target {
            crate::legacy::Target::App(app) => cmd.launch_app(host, app, &format!(" {args}")),
            crate::legacy::Target::Settings(nav) => {
                Outcome::Step(Step::Launch(Launch::Settings(nav)))
            }
        };
    }
    match cmd.resolve(host, &name) {
        Some(super::Found::Batch(path)) => {
            let native = host.native(&path).unwrap_or_default();
            Outcome::Step(Step::Launch(Launch::Terminal {
                profile: crate::Profile::Cmd,
                cwd,
                script: Some(native),
            }))
        }
        Some(super::Found::Program(_, native)) => {
            let command_line = format!("{target} {args}");
            Outcome::Step(Step::Spawn(crate::Spawn {
                program: native,
                argv: crate::split_command_line(&command_line),
                command_line,
                cwd,
                environment: cmd.env.pairs(),
                input: None,
                capture: false,
                detached: !wait,
            }))
        }
        Some(super::Found::Document(native)) => Outcome::Step(Step::Launch(Launch::Open(native))),
        Some(super::Found::Tool(tool)) => super::tools::run(cmd, host, tool, &format!(" {args}")),
        None => {
            // A folder opens in File Explorer.
            if let Ok(native) = cmd.native(host, &name)
                && host.stat(&native).is_ok_and(|m| m.dir)
            {
                return Outcome::Step(Step::Launch(Launch::Open(native)));
            }
            cmd.err_line(host, &format!("The system cannot find the file {name}."));
            cmd.errorlevel = 9059;
            Outcome::Done(false)
        }
    }
}

// ----- Files -----
fn type_(cmd: &mut Cmd, host: &mut dyn Host, tail: &str) -> Outcome {
    let args: Vec<String> = tokens(tail)
        .into_iter()
        .filter(|a| !a.starts_with('/'))
        .collect();
    if args.is_empty() {
        return syntax(cmd, host);
    }
    let mut files: Vec<(String, String)> = Vec::new();
    let mut ok = true;
    for a in &args {
        let name = unq(a);
        if wpath::wild(&name) {
            let found = cmd.glob(host, &name, false);
            if found.is_empty() {
                cmd.err_line(host, FsError::NotFound.message());
                ok = false;
            }
            for (p, _) in found {
                files.push((p.clone(), String::from(wpath::name(&p))));
            }
        } else {
            match cmd.full(&name) {
                Ok(p) => files.push((p, name.clone())),
                Err(e) => {
                    cmd.err_line(host, e.message());
                    ok = false;
                }
            }
        }
    }
    let headers = files.len() > 1 || args.iter().any(|a| wpath::wild(a));
    for (path, display) in files {
        let Some(native) = host.native(&path) else {
            cmd.err_line(host, FsError::PathNotFound.message());
            ok = false;
            continue;
        };
        match host.stat(&native) {
            Ok(m) if m.dir => {
                cmd.err_line(host, FsError::Denied.message());
                ok = false;
                continue;
            }
            Err(_) => {
                cmd.err_line(host, FsError::NotFound.message());
                ok = false;
                continue;
            }
            _ => (),
        }
        if headers {
            cmd.err(host, &format!("\n{display}\n\n\n"));
        }
        match host.read(&native, 16 << 20) {
            Ok(bytes) => {
                let text = decode_text(&bytes);
                cmd.out(host, &text);
            }
            Err(e) => {
                cmd.err_line(host, e.message());
                ok = false;
            }
        }
    }
    if !ok {
        cmd.errorlevel = 1;
    }
    Outcome::Done(ok)
}
/// File bytes as text: UTF-16 with a BOM, UTF-8 (BOM dropped), CRLF to LF.
pub(crate) fn decode_text(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units).replace("\r\n", "\n");
    }
    let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    crlf_to_lf(b)
}
fn mkdir(cmd: &mut Cmd, host: &mut dyn Host, tail: &str) -> Outcome {
    let args = tokens(tail);
    if args.is_empty() {
        return syntax(cmd, host);
    }
    let mut ok = true;
    for a in args {
        let name = unq(&a);
        let Ok(full) = cmd.full(&name) else {
            cmd.err_line(host, FsError::Invalid.message());
            ok = false;
            continue;
        };
        let Some(native) = host.native(&full) else {
            cmd.err_line(host, FsError::PathNotFound.message());
            ok = false;
            continue;
        };
        if host.stat(&native).is_ok() {
            cmd.err_line(
                host,
                &format!("A subdirectory or file {name} already exists."),
            );
            ok = false;
            continue;
        }
        // Create missing parents too (command extensions).
        let mut chain = Vec::new();
        let mut cur = full.clone();
        while let Some(n) = host.native(&cur) {
            if host.stat(&n).is_ok() {
                break;
            }
            chain.push(n);
            if wpath::is_root(&cur) {
                break;
            }
            cur = String::from(wpath::parent(&cur));
        }
        for n in chain.iter().rev() {
            if let Err(e) = host.mkdir(n) {
                cmd.err_line(host, e.message());
                ok = false;
                break;
            }
        }
    }
    if !ok {
        cmd.errorlevel = 1;
    }
    Outcome::Done(ok)
}
fn remove_tree(host: &mut dyn Host, native: &str) -> Result<(), FsError> {
    for e in host.list(native)? {
        let child = format!("{}/{}", native.trim_end_matches('/'), e.name);
        if e.meta.dir {
            remove_tree(host, &child)?;
        } else {
            host.remove_file(&child)?;
        }
    }
    host.remove_dir(native)
}
fn rmdir(cmd: &mut Cmd, host: &mut dyn Host, tail: &str) -> Outcome {
    let mut tree = false;
    let mut quiet = false;
    let mut paths = Vec::new();
    for t in tokens(tail) {
        match t.to_ascii_lowercase().as_str() {
            "/s" => tree = true,
            "/q" => quiet = true,
            s if s.starts_with('/') => {
                cmd.err_line(host, &format!("Invalid switch - \"{}\".", &t[1..]));
                return Outcome::Done(false);
            }
            _ => paths.push(unq(&t)),
        }
    }
    if paths.is_empty() {
        return syntax(cmd, host);
    }
    let mut ok = true;
    for name in paths {
        let native = match cmd.native(host, &name) {
            Ok(n) => n,
            Err(e) => return fail(cmd, host, e),
        };
        match host.stat(&native) {
            Ok(m) if m.dir => (),
            Ok(_) => {
                cmd.err_line(host, "The directory name is invalid.");
                ok = false;
                continue;
            }
            Err(_) => {
                cmd.err_line(host, FsError::NotFound.message());
                ok = false;
                continue;
            }
        }
        if cmd.full(&name).is_ok_and(|f| {
            cmd.cwd
                .to_ascii_lowercase()
                .starts_with(&f.to_ascii_lowercase())
        }) {
            cmd.err_line(host, FsError::InUse.message());
            ok = false;
            continue;
        }
        if tree {
            if !quiet {
                cmd.out(host, &format!("{name}, Are you sure (Y/N)? "));
                return Outcome::Wait(
                    Pending::RemoveTree {
                        native,
                        display: name,
                    },
                    Step::Input(InputKind::Line),
                );
            }
            if let Err(e) = remove_tree(host, &native) {
                cmd.err_line(host, e.message());
                ok = false;
            }
        } else if let Err(e) = host.remove_dir(&native) {
            cmd.err_line(host, e.message());
            ok = false;
        }
    }
    if !ok {
        cmd.errorlevel = 1;
    }
    Outcome::Done(ok)
}
/// DEL's attribute filter (`/A:-R`, `/AH`).
fn attr_filter(spec: &str) -> impl Fn(&Meta) -> bool + '_ {
    move |m: &Meta| {
        let mut chars = spec.trim_start_matches(':').chars().peekable();
        while let Some(c) = chars.next() {
            let neg = c == '-';
            let k = if neg { chars.next().unwrap_or(' ') } else { c };
            let has = match k.to_ascii_lowercase() {
                'd' => m.dir,
                'r' => m.attrs & attr::READONLY != 0,
                'h' => m.attrs & attr::HIDDEN != 0,
                's' => m.attrs & attr::SYSTEM != 0,
                'a' => m.attrs & attr::ARCHIVE != 0,
                'i' => m.attrs & attr::NOT_CONTENT_INDEXED != 0,
                'l' => m.attrs & attr::REPARSE != 0,
                'o' => m.attrs & attr::OFFLINE != 0,
                _ => true,
            };
            if has == neg {
                return false;
            }
        }
        true
    }
}
fn del(cmd: &mut Cmd, host: &mut dyn Host, tail: &str) -> Outcome {
    let mut force = false;
    let mut quiet = false;
    let mut subdirs = false;
    let mut prompt_each = false;
    let mut attrs: Option<String> = None;
    let mut names = Vec::new();
    for t in tokens(tail) {
        let lower = t.to_ascii_lowercase();
        match lower.as_str() {
            "/f" => force = true,
            "/q" => quiet = true,
            "/s" => subdirs = true,
            "/p" => prompt_each = true,
            s if s.starts_with("/a") => attrs = Some(String::from(&t[2..])),
            s if s.starts_with('/') => {
                cmd.err_line(host, &format!("Invalid switch - \"{}\".", &t[1..]));
                return Outcome::Done(false);
            }
            _ => names.push(unq(&t)),
        }
    }
    let _ = prompt_each;
    if names.is_empty() {
        return syntax(cmd, host);
    }
    let mut files: Vec<(String, String)> = Vec::new();
    let mut ask = false;
    let mut pattern_display = String::new();
    for name in names {
        let Ok(mut full) = cmd.full(&name) else {
            cmd.err_line(host, FsError::Invalid.message());
            continue;
        };
        // A directory means everything in it.
        if !wpath::wild(&full)
            && let Some(n) = host.native(&full)
            && host.stat(&n).is_ok_and(|m| m.dir)
        {
            full = wpath::join(&full, "*");
        }
        let (dir, pattern) = wpath::split(&full);
        let (dir, pattern) = (String::from(dir), String::from(pattern));
        let mut matched = Vec::new();
        collect(host, &dir, &pattern, subdirs, &attrs, &mut matched);
        if matched.is_empty() {
            cmd.err_line(host, &format!("Could Not Find {full}"));
            continue;
        }
        if (pattern == "*" || pattern == "*.*") && !quiet {
            ask = true;
            pattern_display = full.clone();
        }
        files.extend(matched);
    }
    if files.is_empty() {
        return Outcome::Done(true);
    }
    if ask {
        cmd.out(host, &format!("{pattern_display}, Are you sure (Y/N)? "));
        return Outcome::Wait(
            Pending::Delete {
                files,
                force,
                subdirs,
            },
            Step::Input(InputKind::Line),
        );
    }
    delete_files(cmd, host, &files, force, subdirs);
    Outcome::Done(true)
}
/// Files matching `pattern` in `dir` (and below with `subdirs`).
fn collect(
    host: &mut dyn Host,
    dir: &str,
    pattern: &str,
    subdirs: bool,
    attrs: &Option<String>,
    out: &mut Vec<(String, String)>,
) {
    let Some(native) = host.native(dir) else {
        return;
    };
    let Ok(mut entries) = host.list(&native) else {
        return;
    };
    entries.sort_by_key(|e| e.name.to_lowercase());
    for e in &entries {
        if e.meta.dir {
            continue;
        }
        let visible = match attrs {
            Some(spec) => attr_filter(spec)(&e.meta),
            None => e.meta.attrs & (attr::HIDDEN | attr::SYSTEM) == 0,
        };
        if visible && wild::matches(pattern, &e.name) {
            out.push((
                format!("{}/{}", native.trim_end_matches('/'), e.name),
                wpath::join(dir, &e.name),
            ));
        }
    }
    if subdirs {
        for e in entries.iter().filter(|e| e.meta.dir) {
            collect(host, &wpath::join(dir, &e.name), pattern, true, attrs, out);
        }
    }
}
fn delete_files(
    cmd: &mut Cmd,
    host: &mut dyn Host,
    files: &[(String, String)],
    force: bool,
    subdirs: bool,
) {
    for (native, display) in files {
        if !force
            && host
                .stat(native)
                .is_ok_and(|m| m.attrs & attr::READONLY != 0)
        {
            cmd.err_line(host, &format!("{display}\nAccess is denied."));
            continue;
        }
        match host.remove_file(native) {
            Ok(()) => {
                if subdirs {
                    cmd.out_line(host, &format!("Deleted file - {display}"));
                }
            }
            Err(e) => cmd.err_line(host, &format!("{display}\n{}", e.message())),
        }
    }
}
fn ren(cmd: &mut Cmd, host: &mut dyn Host, tail: &str) -> Outcome {
    let args = tokens(tail);
    if args.len() != 2 {
        return syntax(cmd, host);
    }
    let (from, to) = (unq(&args[0]), unq(&args[1]));
    if to.contains(['\\', '/', ':']) {
        return syntax(cmd, host);
    }
    let Ok(full) = cmd.full(&from) else {
        return fail(cmd, host, FsError::Invalid);
    };
    let (dir, pattern) = wpath::split(&full);
    let dir = String::from(dir);
    let targets: Vec<(String, Meta)> = if wpath::wild(pattern) {
        let mut v = cmd.glob(host, &full, false);
        v.extend(cmd.glob(host, &full, true));
        v
    } else {
        match host.native(&full).map(|n| host.stat(&n)) {
            Some(Ok(m)) => vec![(full.clone(), m)],
            _ => Vec::new(),
        }
    };
    if targets.is_empty() {
        return fail(cmd, host, FsError::NotFound);
    }
    let mut ok = true;
    for (path, _) in targets {
        let name = wpath::name(&path);
        let new_name = wild::template(&to, name);
        let dest = wpath::join(&dir, &new_name);
        let (Some(a), Some(b)) = (host.native(&path), host.native(&dest)) else {
            continue;
        };
        if host.stat(&b).is_ok() && !a.eq_ignore_ascii_case(&b) {
            cmd.err_line(
                host,
                "A duplicate file name exists, or the file\ncannot be found.",
            );
            ok = false;
            continue;
        }
        if let Err(e) = host.rename(&a, &b) {
            cmd.err_line(host, e.message());
            ok = false;
        }
    }
    if !ok {
        cmd.errorlevel = 1;
    }
    Outcome::Done(ok)
}
/// COPY and MOVE: plan (source native, destination native, display), then
/// run, asking before overwriting at the prompt.
fn transfer(cmd: &mut Cmd, host: &mut dyn Host, tail: &str, moving: bool) -> Outcome {
    let mut confirm = cmd.batch_depth() == 0;
    if let Some(c) = cmd.env.get("COPYCMD") {
        if c.eq_ignore_ascii_case("/y") {
            confirm = false;
        }
        if c.eq_ignore_ascii_case("/-y") {
            confirm = true;
        }
    }
    let mut args = Vec::new();
    // `a+b` concatenation keeps `+` as its own word.
    let spaced = tail.replace('+', " + ");
    for t in tokens(&spaced) {
        match t.to_ascii_lowercase().as_str() {
            "/y" => confirm = false,
            "/-y" => confirm = true,
            "/a" | "/b" | "/v" | "/n" | "/z" | "/l" | "/d" | "/u" => (),
            s if s.starts_with('/') => {
                cmd.err_line(host, &format!("Invalid switch - {}", &t[1..]));
                return Outcome::Done(false);
            }
            _ => args.push(unq(&t)),
        }
    }
    if args.is_empty() {
        return syntax(cmd, host);
    }
    // Sources joined by '+', then an optional destination.
    let mut sources = vec![args[0].clone()];
    let mut i = 1;
    while i + 1 < args.len() + 1 && args.get(i).is_some_and(|a| a == "+") {
        if let Some(s) = args.get(i + 1) {
            sources.push(s.clone());
        }
        i += 2;
    }
    let dest = args.get(i).cloned();
    if args.len() > i + 1 {
        return syntax(cmd, host);
    }
    let concat = sources.len() > 1;
    // Expand sources.
    let mut files: Vec<(String, Meta)> = Vec::new();
    let mut wildcard = concat;
    for s in &sources {
        let Ok(full) = cmd.full(s) else {
            return fail(cmd, host, FsError::Invalid);
        };
        if wpath::wild(&full) {
            wildcard = true;
            files.extend(cmd.glob(host, &full, false));
            if moving {
                files.extend(cmd.glob(host, &full, true));
            }
        } else {
            match host.native(&full).map(|n| host.stat(&n)) {
                Some(Ok(m)) if m.dir && !moving => {
                    // COPY dir copies the files in it.
                    wildcard = true;
                    files.extend(cmd.glob(host, &wpath::join(&full, "*"), false));
                }
                Some(Ok(m)) => files.push((full, m)),
                _ => {
                    cmd.err_line(host, FsError::NotFound.message());
                    if !moving {
                        cmd.out_line(host, "        0 file(s) copied.");
                    }
                    cmd.errorlevel = 1;
                    return Outcome::Done(false);
                }
            }
        }
    }
    if files.is_empty() {
        cmd.err_line(host, FsError::NotFound.message());
        if !moving {
            cmd.out_line(host, "        0 file(s) copied.");
        }
        cmd.errorlevel = 1;
        return Outcome::Done(false);
    }
    let dest_full = match &dest {
        Some(d) => match cmd.full(d) {
            Ok(f) => f,
            Err(e) => return fail(cmd, host, e),
        },
        None => cmd.cwd.clone(),
    };
    let dest_native = host.native(&dest_full);
    let dest_is_dir = dest_native
        .as_ref()
        .is_some_and(|n| host.stat(n).is_ok_and(|m| m.dir));
    // Concatenation (or several files onto one file name) writes one file.
    if !moving && (concat || (files.len() > 1 && !dest_is_dir && dest.is_some())) {
        let target = if dest_is_dir || dest.is_none() {
            wpath::join(&dest_full, wpath::name(&files[0].0))
        } else {
            dest_full.clone()
        };
        let Some(t) = host.native(&target) else {
            return fail(cmd, host, FsError::PathNotFound);
        };
        let mut data = Vec::new();
        for (p, _) in &files {
            cmd.out_line(host, wpath::name(p));
            if let Some(n) = host.native(p)
                && let Ok(mut b) = host.read(&n, 64 << 20)
            {
                // ASCII mode stops at Ctrl+Z, as COPY /A does.
                if let Some(z) = b.iter().position(|c| *c == 0x1a) {
                    b.truncate(z);
                }
                data.extend(b);
            }
        }
        return match host.write(&t, &data, false) {
            Ok(()) => {
                cmd.out_line(host, "        1 file(s) copied.");
                Outcome::Done(true)
            }
            Err(e) => fail(cmd, host, e),
        };
    }
    let mut items = Vec::new();
    for (p, m) in &files {
        let target = if dest_is_dir {
            wpath::join(&dest_full, wpath::name(p))
        } else if files.len() == 1 {
            dest_full.clone()
        } else {
            wpath::join(&dest_full, wpath::name(p))
        };
        let (Some(a), Some(b)) = (host.native(p), host.native(&target)) else {
            continue;
        };
        if a.eq_ignore_ascii_case(&b) {
            cmd.err_line(host, "The file cannot be copied onto itself.");
            if !moving {
                cmd.out_line(host, "        0 file(s) copied.");
            }
            cmd.errorlevel = 1;
            return Outcome::Done(false);
        }
        let _ = m;
        items.push((a, b, target));
    }
    if wildcard {
        // Names are listed when wildcards are used.
        let names: Vec<String> = files
            .iter()
            .map(|(p, _)| String::from(wpath::name(p)))
            .collect();
        for n in names {
            cmd.out_line(host, &n);
        }
    }
    let dirs = moving && files.len() == 1 && files[0].1.dir;
    continue_transfer(
        cmd,
        host,
        Transfer {
            moving,
            items,
            index: 0,
            all: !confirm,
            done: 0,
            dirs,
        },
    )
}
fn continue_transfer(cmd: &mut Cmd, host: &mut dyn Host, mut t: Transfer) -> Outcome {
    while t.index < t.items.len() {
        let (from, to, display) = t.items[t.index].clone();
        if !t.all && host.stat(&to).is_ok() {
            cmd.out(host, &format!("Overwrite {display}? (Yes/No/All): "));
            return Outcome::Wait(Pending::Transfer(t), Step::Input(InputKind::Line));
        }
        match do_transfer(host, &from, &to, t.moving, true) {
            Ok(()) => t.done += 1,
            Err(e) => cmd.err_line(host, e.message()),
        }
        t.index += 1;
    }
    let what = if t.dirs { "dir(s)" } else { "file(s)" };
    let verb = if t.moving { "moved" } else { "copied" };
    cmd.out_line(host, &format!("{:>9} {what} {verb}.", t.done));
    let ok = t.done > 0 || t.items.is_empty();
    if !ok {
        cmd.errorlevel = 1;
    }
    Outcome::Done(ok)
}
fn do_transfer(
    host: &mut dyn Host,
    from: &str,
    to: &str,
    moving: bool,
    overwrite: bool,
) -> Result<(), FsError> {
    if overwrite
        && let Ok(m) = host.stat(to)
        && !m.dir
        && !from.eq_ignore_ascii_case(to)
    {
        host.remove_file(to)?;
    }
    if moving {
        host.rename(from, to)
    } else {
        host.copy_file(from, to)
    }
}

// ----- DIR -----
#[derive(Default)]
struct DirOptions {
    bare: bool,
    wide: bool,
    recurse: bool,
    lower: bool,
    owner: bool,
    short: bool,
    separators: bool,
    attrs: Option<String>,
    order: Option<String>,
    time: char,
}
fn dir(cmd: &mut Cmd, host: &mut dyn Host, tail: &str) -> Outcome {
    let env_switches = cmd.env.get("DIRCMD").map(String::from).unwrap_or_default();
    let mut o = DirOptions {
        separators: true,
        time: 'w',
        ..DirOptions::default()
    };
    let mut paths = Vec::new();
    for t in tokens(&format!("{env_switches} {tail}")) {
        let lower = t.to_ascii_lowercase();
        if let Some(sw) = lower.strip_prefix('/') {
            let (neg, sw) = match sw.strip_prefix('-') {
                Some(s) => (true, s),
                None => (false, sw),
            };
            match sw.chars().next() {
                Some('b') => o.bare = !neg,
                Some('w') | Some('d') => o.wide = !neg,
                Some('s') => o.recurse = !neg,
                Some('l') => o.lower = !neg,
                Some('q') => o.owner = !neg,
                Some('x') => o.short = !neg,
                Some('c') => o.separators = !neg,
                Some('a') => {
                    o.attrs = if neg {
                        None
                    } else {
                        Some(String::from(sw[1..].trim_start_matches(':')))
                    }
                }
                Some('o') => {
                    o.order = if neg {
                        None
                    } else {
                        Some(String::from(sw[1..].trim_start_matches(':')))
                    }
                }
                Some('t') => {
                    o.time = sw[1..]
                        .trim_start_matches(':')
                        .chars()
                        .next()
                        .unwrap_or('w')
                }
                Some('p' | 'n' | '4' | 'r') => (),
                _ => {
                    cmd.err_line(host, &format!("Invalid switch - \"{}\".", &t[1..]));
                    return Outcome::Done(false);
                }
            }
        } else {
            paths.push(unq(&t));
        }
    }
    if paths.is_empty() {
        paths.push(String::from("."));
    }
    let drive = wpath::drive(&cmd.cwd).unwrap_or('C');
    let volume = host.volume(drive).unwrap_or_default();
    if !o.bare {
        let label = if volume.label.is_empty() {
            format!(" Volume in drive {drive} has no label.")
        } else {
            format!(" Volume in drive {drive} is {}", volume.label)
        };
        cmd.out_line(host, &label);
        cmd.out_line(
            host,
            &format!(
                " Volume Serial Number is {:04X}-{:04X}",
                volume.serial >> 16,
                volume.serial & 0xFFFF
            ),
        );
    }
    let mut total = (0u64, 0u64, 0u64);
    let mut any = false;
    for p in paths {
        let Ok(full) = cmd.full(&p) else {
            cmd.err_line(host, FsError::Invalid.message());
            continue;
        };
        let (dir, pattern) = match host.native(&full).map(|n| host.stat(&n)) {
            Some(Ok(m)) if m.dir => (full.clone(), String::from("*")),
            _ => {
                let (d, n) = wpath::split(&full);
                (String::from(d), String::from(n))
            }
        };
        if host.native(&dir).is_none_or(|n| host.stat(&n).is_err()) {
            cmd.err_line(host, FsError::PathNotFound.message());
            cmd.errorlevel = 1;
            return Outcome::Done(false);
        }
        any |= dir_one(cmd, host, &dir, &pattern, &o, &mut total, true);
    }
    if !any {
        cmd.err_line(host, "File Not Found");
        cmd.errorlevel = 1;
        return Outcome::Done(false);
    }
    let free = volume.free;
    if !o.bare {
        let sep = |n: u64| {
            if o.separators {
                grouped(n)
            } else {
                format!("{n}")
            }
        };
        if o.recurse {
            cmd.out_line(host, "\n     Total Files Listed:");
            cmd.out_line(
                host,
                &format!("{:>16} File(s){:>15} bytes", total.0, sep(total.1)),
            );
        }
        cmd.out_line(
            host,
            &format!("{:>16} Dir(s){:>16} bytes free", total.2, sep(free)),
        );
    }
    Outcome::Done(true)
}
/// One directory's listing; true when anything matched. Totals:
/// (files, bytes, dirs).
fn dir_one(
    cmd: &mut Cmd,
    host: &mut dyn Host,
    dir: &str,
    pattern: &str,
    o: &DirOptions,
    total: &mut (u64, u64, u64),
    top: bool,
) -> bool {
    let Some(native) = host.native(dir) else {
        return false;
    };
    let Ok(mut entries) = host.list(&native) else {
        return false;
    };
    let root = wpath::is_root(dir);
    if !root {
        let me = host.stat(&native).unwrap_or_default();
        let parent = host
            .native(wpath::parent(dir))
            .and_then(|n| host.stat(&n).ok())
            .unwrap_or(me);
        entries.insert(
            0,
            crate::host::Entry {
                name: String::from(".."),
                meta: parent,
            },
        );
        entries.insert(
            0,
            crate::host::Entry {
                name: String::from("."),
                meta: me,
            },
        );
    }
    let shown: Vec<&crate::host::Entry> = entries
        .iter()
        .filter(|e| match &o.attrs {
            Some(spec) if !spec.is_empty() => attr_filter(spec)(&e.meta),
            Some(_) => true,
            None => e.meta.attrs & (attr::HIDDEN | attr::SYSTEM) == 0,
        })
        .filter(|e| {
            wild::matches(pattern, &e.name) || (pattern == "*" && (e.name == "." || e.name == ".."))
        })
        .collect();
    let mut shown: Vec<crate::host::Entry> = shown.into_iter().cloned().collect();
    if let Some(order) = &o.order {
        sort_entries(&mut shown, if order.is_empty() { "gn" } else { order });
    }
    let mut matched = !shown.is_empty();
    if !shown.is_empty() {
        if !o.bare {
            cmd.out_line(host, &format!("\n Directory of {dir}\n"));
        }
        let (mut files, mut bytes, mut dirs) = (0u64, 0u64, 0u64);
        let width = shown
            .iter()
            .map(|e| e.name.chars().count() + if e.meta.dir { 2 } else { 0 })
            .max()
            .unwrap_or(0)
            + 3;
        let per_row = (cmd.columns.max(20) / width.max(1)).max(1);
        let mut row = String::new();
        let mut col = 0;
        for e in &shown {
            let name = if o.lower {
                e.name.to_lowercase()
            } else {
                e.name.clone()
            };
            if e.meta.dir {
                dirs += 1;
            } else {
                files += 1;
                bytes += e.meta.size;
            }
            if o.bare {
                if e.name == "." || e.name == ".." {
                    continue;
                }
                let line = if o.recurse {
                    wpath::join(dir, &name)
                } else {
                    name
                };
                cmd.out_line(host, &line);
                continue;
            }
            if o.wide {
                let cell = if e.meta.dir {
                    format!("[{name}]")
                } else {
                    name
                };
                row.push_str(&format!("{cell:<width$}"));
                col += 1;
                if col == per_row {
                    cmd.out_line(host, &row);
                    row.clear();
                    col = 0;
                }
                continue;
            }
            let ft = match o.time {
                'c' => e.meta.created,
                'a' => e.meta.accessed,
                _ => e.meta.modified,
            };
            let t = host.local(ft);
            let size = if e.meta.dir {
                String::from("    <DIR>         ")
            } else if o.separators {
                format!("{:>18}", grouped(e.meta.size))
            } else {
                format!("{:>18}", e.meta.size)
            };
            let mut line = format!("{}  {}{size} ", date(&t), time_short(&t));
            if o.short {
                let s = if e.name == "." || e.name == ".." {
                    String::new()
                } else {
                    wpath::short_name(&e.name)
                };
                let s = if s.eq_ignore_ascii_case(&e.name) {
                    String::new()
                } else {
                    s
                };
                line.push_str(&format!("{s:<13}"));
            }
            if o.owner {
                let owner = format!("{}\\{}", host.domain(), host.user_name());
                line.push_str(&format!("{owner:<23}"));
            }
            line.push_str(&name);
            cmd.out_line(host, &line);
        }
        if !row.is_empty() {
            cmd.out_line(host, &row);
        }
        if !o.bare {
            let sep = |n: u64| {
                if o.separators {
                    grouped(n)
                } else {
                    format!("{n}")
                }
            };
            cmd.out_line(
                host,
                &format!("{:>16} File(s){:>15} bytes", files, sep(bytes)),
            );
        }
        total.0 += files;
        total.1 += bytes;
        total.2 += dirs;
    }
    if o.recurse {
        let mut subdirs: Vec<String> = entries
            .iter()
            .filter(|e| e.meta.dir && e.name != "." && e.name != "..")
            .map(|e| e.name.clone())
            .collect();
        subdirs.sort_by_key(|n| n.to_lowercase());
        for s in subdirs {
            matched |= dir_one(cmd, host, &wpath::join(dir, &s), pattern, o, total, false);
        }
    }
    let _ = top;
    matched
}
fn sort_entries(entries: &mut [crate::host::Entry], order: &str) {
    let keys: Vec<(char, bool)> = {
        let mut v = Vec::new();
        let mut chars = order.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '-' {
                if let Some(k) = chars.next() {
                    v.push((k.to_ascii_lowercase(), true));
                }
            } else {
                v.push((c.to_ascii_lowercase(), false));
            }
        }
        v
    };
    entries.sort_by(|a, b| {
        use core::cmp::Ordering;
        for &(k, rev) in &keys {
            let o = match k {
                'n' => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                'e' => wpath::extension(&a.name)
                    .to_lowercase()
                    .cmp(&wpath::extension(&b.name).to_lowercase()),
                's' => a.meta.size.cmp(&b.meta.size),
                'd' => a.meta.modified.cmp(&b.meta.modified),
                'g' => b.meta.dir.cmp(&a.meta.dir),
                _ => Ordering::Equal,
            };
            let o = if rev { o.reverse() } else { o };
            if o != Ordering::Equal {
                return o;
            }
        }
        Ordering::Equal
    });
}

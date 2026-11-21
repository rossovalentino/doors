//! cmd.exe (P16): the Windows command interpreter as Terminal's CMD
//! profile and the engine `.bat`/`.cmd` files run in.
//!
//! A batch file is read one logical line at a time as cmd.exe reads it
//! (carets join lines, an open parenthesis continues the block); each line
//! is percent-expanded, parsed (`parse`) and compiled to a small program
//! that the VM runs with FOR variables, delayed expansion, `&&`/`||`
//! status, pipes (the left side's output feeds the right side) and
//! redirections. Internal commands are in `builtin`, the System32 console
//! tools batch files rely on in `tools`. Programs, input and waits yield a
//! [`Step`] instead of blocking.
pub mod arith;
mod builtin;
mod events;
pub mod expand;
mod forloop;
mod help;
mod netsh;
pub mod parse;
mod sc;
mod schtasks;
mod tools;
mod vpn;

use crate::host::{FsError, Host, Meta};
use crate::{Launch, Resume, Spawn, Step, wpath};
use alloc::{format, rc::Rc, string::String, vec, vec::Vec};
use parse::{CmpOp, Cond, Node, Redirect, Simple};

/// Console events for Terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Text(String),
    Clear,
    Title(String),
    /// `color` attribute (background << 4 | foreground).
    Color(u8),
}
/// The environment: case-insensitive names, kept sorted as Windows does.
#[derive(Clone, Debug, Default)]
pub struct Env {
    vars: Vec<(String, String)>,
}
impl Env {
    fn key(name: &str) -> String {
        name.to_uppercase()
    }
    fn find(&self, name: &str) -> Result<usize, usize> {
        let k = Self::key(name);
        self.vars.binary_search_by(|(n, _)| Self::key(n).cmp(&k))
    }
    pub fn get(&self, name: &str) -> Option<&str> {
        self.find(name).ok().map(|i| self.vars[i].1.as_str())
    }
    pub fn set(&mut self, name: &str, value: &str) {
        match self.find(name) {
            Ok(i) => self.vars[i].1 = value.into(),
            Err(i) => self.vars.insert(i, (name.into(), value.into())),
        }
    }
    pub fn remove(&mut self, name: &str) -> bool {
        match self.find(name) {
            Ok(i) => {
                self.vars.remove(i);
                true
            }
            Err(_) => false,
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.vars.iter().map(|(n, v)| (n.as_str(), v.as_str()))
    }
    pub fn pairs(&self) -> Vec<String> {
        self.vars.iter().map(|(n, v)| format!("{n}={v}")).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Sink {
    Console,
    Null,
    /// Native path, opened for appending.
    File(String),
    Capture(usize),
}
#[derive(Clone, Debug)]
pub(crate) struct Redir {
    pub out: Sink,
    pub err: Sink,
    pub input: Option<Rc<Vec<u8>>>,
}
#[derive(Clone, Debug)]
pub(crate) enum Op {
    Run(Rc<Simple>),
    If {
        not: bool,
        insensitive: bool,
        cond: Rc<Cond>,
        otherwise: usize,
    },
    Jump(usize),
    /// `||`: skip the right side after success.
    JumpIfOk(usize),
    /// `&&`: skip the right side after failure.
    JumpIfFail(usize),
    /// Block redirections; on failure continue at `skip`.
    PushRedirect {
        redirects: Rc<Vec<Redirect>>,
        skip: usize,
    },
    PopRedirect,
    PipeBegin,
    PipeMid,
    PipeEnd,
    ForBegin {
        spec: Rc<parse::For>,
        end: usize,
    },
    ForNext {
        end: usize,
    },
    /// End of a `cmd /c` child: its environment changes end with it.
    RestoreChild,
}
pub(crate) struct Code {
    ops: Rc<Vec<Op>>,
    pc: usize,
    vars: Vec<(char, String)>,
    loops: Vec<forloop::Loop>,
    redir_depth: usize,
    /// A FOR /F command's capture that finished (`ForBegin` resumes).
    for_capture: Option<usize>,
    /// The frame of a `cmd /c` child context.
    child: bool,
}
pub(crate) struct Batch {
    /// Full Windows path (`%0`, `%~f0`).
    pub path: String,
    pub text: Rc<String>,
    pub pos: usize,
    pub args: Vec<String>,
    /// `SHIFT` offset (`%0` = args[shift]).
    pub shift: usize,
    /// Redirections in force when it started (its end drops later ones).
    pub redir_depth: usize,
    /// ECHO at the prompt when the outermost batch file started: restored
    /// when it returns to the command line, as cmd.exe does.
    pub prompt_echo: Option<bool>,
}
pub(crate) enum Frame {
    Batch(Batch),
    Code(Code),
}
struct Local {
    env: Env,
    cwd: String,
    delayed: bool,
    extensions: bool,
    depth: usize,
}
/// What the engine waits for.
pub(crate) enum Wait {
    None,
    Program {
        capture: Option<usize>,
        pop: bool,
        quiet_status: bool,
    },
    Builtin {
        pending: builtin::Pending,
        pop: bool,
    },
    Terminate,
    Exit(i32),
}

pub struct Cmd {
    pub(crate) env: Env,
    pub(crate) cwd: String,
    pub(crate) drives: Vec<(char, String)>,
    pub(crate) echo: bool,
    pub(crate) delayed: bool,
    pub(crate) extensions: bool,
    pub(crate) errorlevel: i32,
    pub(crate) ok: bool,
    locals: Vec<Local>,
    pub(crate) pushd: Vec<String>,
    pub(crate) frames: Vec<Frame>,
    pub(crate) redirs: Vec<Redir>,
    pub(crate) captures: Vec<Option<Vec<u8>>>,
    pub(crate) events: Vec<Event>,
    pub(crate) wait: Wait,
    pending: String,
    pub(crate) columns: usize,
    pub(crate) rows: usize,
    pub(crate) codepage: u32,
    pub(crate) title: String,
    /// The command line cmd.exe was started with (`%CMDCMDLINE%`).
    pub(crate) cmdline: String,
    /// `/c`: exit after the first command.
    pub(crate) once: bool,
    pub(crate) verify: bool,
    pub(crate) brk: bool,
    /// Saved contexts of running `cmd /c` children.
    children: Vec<Local>,
}

/// Default environment of a Windows 11 session for `user` on `computer`
/// with the profile at `profile` (`C:\Users\name`).
pub fn default_environment(user: &str, computer: &str, profile: &str, processors: u32) -> Env {
    let mut e = Env::default();
    let local = format!("{profile}\\AppData\\Local");
    let pairs: [(&str, String); 37] = [
        ("ALLUSERSPROFILE", "C:\\ProgramData".into()),
        ("APPDATA", format!("{profile}\\AppData\\Roaming")),
        ("CommonProgramFiles", "C:\\Program Files\\Common Files".into()),
        ("CommonProgramFiles(x86)", "C:\\Program Files (x86)\\Common Files".into()),
        ("CommonProgramW6432", "C:\\Program Files\\Common Files".into()),
        ("COMPUTERNAME", computer.into()),
        ("ComSpec", "C:\\Windows\\system32\\cmd.exe".into()),
        ("DriverData", "C:\\Windows\\System32\\Drivers\\DriverData".into()),
        ("HOMEDRIVE", "C:".into()),
        ("HOMEPATH", profile[2..].into()),
        ("LOCALAPPDATA", local.clone()),
        ("LOGONSERVER", format!("\\\\{computer}")),
        ("NUMBER_OF_PROCESSORS", format!("{processors}")),
        ("OS", "Windows_NT".into()),
        (
            "Path",
            format!(
                "C:\\Windows\\system32;C:\\Windows;C:\\Windows\\System32\\Wbem;C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\;C:\\Windows\\System32\\OpenSSH\\;{local}\\Microsoft\\WindowsApps;"
            ),
        ),
        ("PATHEXT", ".COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC".into()),
        ("PROCESSOR_ARCHITECTURE", "AMD64".into()),
        ("PROCESSOR_IDENTIFIER", "Intel64 Family 6 Model 85 Stepping 7, GenuineIntel".into()),
        ("PROCESSOR_LEVEL", "6".into()),
        ("PROCESSOR_REVISION", "5507".into()),
        ("ProgramData", "C:\\ProgramData".into()),
        ("ProgramFiles", "C:\\Program Files".into()),
        ("ProgramFiles(x86)", "C:\\Program Files (x86)".into()),
        ("ProgramW6432", "C:\\Program Files".into()),
        ("PROMPT", "$P$G".into()),
        ("PSModulePath", "C:\\Program Files\\WindowsPowerShell\\Modules;C:\\Windows\\system32\\WindowsPowerShell\\v1.0\\Modules".into()),
        ("PUBLIC", "C:\\Users\\Public".into()),
        ("SESSIONNAME", "Console".into()),
        ("SystemDrive", "C:".into()),
        ("SystemRoot", "C:\\Windows".into()),
        ("TEMP", format!("{local}\\Temp")),
        ("TMP", format!("{local}\\Temp")),
        ("USERDOMAIN", computer.into()),
        ("USERDOMAIN_ROAMINGPROFILE", computer.into()),
        ("USERNAME", user.into()),
        ("USERPROFILE", profile.into()),
        ("windir", "C:\\Windows".into()),
    ];
    for (n, v) in pairs {
        e.set(n, &v);
    }
    e
}

impl Cmd {
    /// A new interpreter in `cwd` (a full Windows path).
    pub fn new(env: Env, cwd: &str) -> Self {
        Self {
            env,
            cwd: cwd.into(),
            drives: Vec::new(),
            echo: true,
            delayed: false,
            extensions: true,
            errorlevel: 0,
            ok: true,
            locals: Vec::new(),
            pushd: Vec::new(),
            frames: Vec::new(),
            redirs: vec![Redir {
                out: Sink::Console,
                err: Sink::Console,
                input: None,
            }],
            captures: Vec::new(),
            events: Vec::new(),
            wait: Wait::None,
            pending: String::new(),
            columns: 120,
            rows: 30,
            codepage: 437,
            title: String::from("Command Prompt"),
            cmdline: String::from("\"C:\\Windows\\system32\\cmd.exe\" "),
            once: false,
            verify: false,
            brk: false,
            children: Vec::new(),
        }
    }
    /// The banner cmd.exe prints when a console starts.
    pub fn banner() -> String {
        let (a, b, c, d) = crate::WINDOWS_VERSION;
        format!(
            "{} [Version {a}.{b}.{c}.{d}]\n(c) {}. All rights reserved.\n",
            crate::PRODUCT,
            crate::COMPANY
        )
    }
    /// ECHO is on (cmd.exe prints a blank line after each prompt command).
    pub fn errorlevel(&self) -> i32 {
        self.errorlevel
    }
    pub fn echo_on(&self) -> bool {
        self.echo
    }
    pub fn cwd(&self) -> &str {
        &self.cwd
    }
    /// Another engine (Doorsh) moved the current folder.
    pub fn set_cwd(&mut self, cwd: &str) {
        self.cwd = String::from(cwd);
    }
    /// Another engine (Doorsh) changed an environment variable.
    pub fn set_env(&mut self, name: &str, value: &str) {
        if value.is_empty() {
            self.env.remove(name);
        } else {
            self.env.set(name, value);
        }
    }
    pub fn env(&self) -> &Env {
        &self.env
    }
    pub fn set_size(&mut self, columns: usize, rows: usize) {
        self.columns = columns.max(20);
        self.rows = rows.max(5);
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn take_events(&mut self) -> Vec<Event> {
        core::mem::take(&mut self.events)
    }
    pub fn busy(&self) -> bool {
        !self.frames.is_empty() || !matches!(self.wait, Wait::None)
    }
    /// The prompt for the next line (`More? ` inside an open block).
    pub fn prompt(&self, host: &dyn Host) -> String {
        if !self.pending.is_empty() {
            return String::from("More? ");
        }
        self.expand_prompt(host)
    }
    pub(crate) fn expand_prompt(&self, host: &dyn Host) -> String {
        let spec = self.env.get("PROMPT").unwrap_or("$P$G");
        let mut out = String::new();
        let mut chars = spec.chars();
        while let Some(c) = chars.next() {
            if c != '$' {
                out.push(c);
                continue;
            }
            match chars.next().map(|c| c.to_ascii_uppercase()) {
                Some('P') => out.push_str(&self.cwd),
                Some('G') => out.push('>'),
                Some('L') => out.push('<'),
                Some('B') => out.push('|'),
                Some('Q') => out.push('='),
                Some('S') => out.push(' '),
                Some('A') => out.push('&'),
                Some('C') => out.push('('),
                Some('F') => out.push(')'),
                Some('N') => out.push(wpath::drive(&self.cwd).unwrap_or('C')),
                Some('D') => out.push_str(&crate::format::date_long(&host.now())),
                Some('T') => out.push_str(crate::format::time_long(&host.now()).trim_start()),
                Some('V') => out.push_str(
                    &Self::banner()
                        .lines()
                        .next()
                        .unwrap_or("")
                        .to_owned_string(),
                ),
                Some('_') => out.push('\n'),
                Some('$') => out.push('$'),
                Some('E') => out.push('\u{1b}'),
                Some('H') => {
                    out.pop();
                }
                Some('+') => out.extend(core::iter::repeat_n('+', self.pushd.len())),
                Some('M') => (),
                _ => (),
            }
        }
        out
    }
    /// Run `cmd /c line` semantics: exit after this line.
    pub fn run_once(&mut self, host: &mut dyn Host, line: &str) -> Step {
        self.once = true;
        self.submit(host, line)
    }
    /// A line typed at the prompt.
    pub fn submit(&mut self, host: &mut dyn Host, line: &str) -> Step {
        let text = if self.pending.is_empty() {
            String::from(line)
        } else {
            format!("{}\n{line}", self.pending)
        };
        let expanded = {
            let mut c = Ctx { cmd: self, host };
            expand::percent(&text, &mut c, None)
        };
        match parse::parse(&expanded) {
            Err(parse::ParseError::More) => {
                self.pending = text;
                Step::Idle
            }
            Err(parse::ParseError::Syntax(m)) => {
                self.pending.clear();
                self.err_line(host, &m);
                self.errorlevel = 255;
                self.after_command(host)
            }
            Ok(node) => {
                self.pending.clear();
                let ops = compile(&node);
                let depth = self.redirs.len();
                self.frames.push(Frame::Code(Code {
                    ops: Rc::new(ops),
                    pc: 0,
                    vars: Vec::new(),
                    loops: Vec::new(),
                    redir_depth: depth,
                    for_capture: None,
                    child: false,
                }));
                self.step(host)
            }
        }
    }
    /// Run a batch file (Explorer, `start x.bat`, Terminal argument).
    pub fn start_batch(&mut self, host: &mut dyn Host, path: &str, args: &[String]) -> Step {
        match self.open_batch(host, path, args) {
            Ok(b) => {
                self.frames.push(Frame::Batch(b));
                self.step(host)
            }
            Err(e) => {
                self.err_line(host, e.message());
                Step::Idle
            }
        }
    }
    pub(crate) fn open_batch(
        &mut self,
        host: &mut dyn Host,
        path: &str,
        args: &[String],
    ) -> Result<Batch, FsError> {
        let native = host.native(path).ok_or(FsError::PathNotFound)?;
        let bytes = host.read(&native, 4 << 20)?;
        let mut text = String::from_utf8(bytes.clone())
            .unwrap_or_else(|_| bytes.iter().map(|&b| char::from(b)).collect());
        if let Some(p) = text.find('\u{1a}') {
            text.truncate(p);
        }
        let mut all = vec![String::from(path)];
        all.extend(args.iter().cloned());
        Ok(Batch {
            path: path.into(),
            text: Rc::new(text),
            pos: 0,
            args: all,
            shift: 0,
            redir_depth: self.redirs.len(),
            prompt_echo: (self.batch_depth() == 0).then_some(self.echo),
        })
    }
    /// Ctrl+C: stop the running batch (asks as cmd.exe does).
    pub fn interrupt(&mut self, host: &mut dyn Host) -> Step {
        self.pending.clear();
        let in_batch = self.frames.iter().any(|f| matches!(f, Frame::Batch(_)));
        self.write_console("^C\n");
        if in_batch {
            self.write_console("Terminate batch job (Y/N)? ");
            self.wait = Wait::Terminate;
            return Step::Input(crate::InputKind::Key);
        }
        self.abort_all();
        let _ = host;
        Step::Idle
    }
    fn abort_all(&mut self) {
        self.frames.clear();
        self.redirs.truncate(1);
        self.wait = Wait::None;
        // A cancelled script's SETLOCALs end with it.
        while let Some(l) = self.locals.pop() {
            self.restore(l);
        }
    }
    fn restore(&mut self, l: Local) {
        self.env = l.env;
        self.cwd = l.cwd;
        self.delayed = l.delayed;
        self.extensions = l.extensions;
    }

    /// Continue after a [`Step`].
    pub fn resume(&mut self, host: &mut dyn Host, r: Resume) -> Step {
        match core::mem::replace(&mut self.wait, Wait::None) {
            Wait::None => self.step(host),
            Wait::Exit(code) => Step::Exit(code),
            Wait::Terminate => {
                let yes = matches!(&r, Resume::Key(c) if c.eq_ignore_ascii_case(&'y'))
                    || matches!(&r, Resume::Line(l) if l.trim().eq_ignore_ascii_case("y"));
                self.write_console("\n");
                if yes {
                    self.abort_all();
                    return self.after_command(host);
                }
                self.step(host)
            }
            Wait::Program {
                capture,
                pop,
                quiet_status,
            } => {
                match r {
                    Resume::Exited { code, output } => {
                        if let Some(id) = capture {
                            let _ = id;
                            let text = crlf_to_lf(&output);
                            self.out(host, &text);
                        }
                        if !quiet_status {
                            self.errorlevel = code;
                        }
                        self.ok = code == 0;
                    }
                    Resume::Failed(e) => {
                        self.err_line(host, e.message());
                        self.errorlevel = e.code();
                        self.ok = false;
                    }
                    _ => {
                        self.ok = true;
                    }
                }
                if pop {
                    self.redirs.pop();
                }
                self.advance();
                self.step(host)
            }
            Wait::Builtin { pending, pop } => match builtin::resume(self, host, pending, r) {
                builtin::Outcome::Done(ok) => {
                    self.ok = ok;
                    if pop {
                        self.redirs.pop();
                    }
                    self.advance();
                    self.step(host)
                }
                builtin::Outcome::Wait(pending, step) => {
                    self.wait = Wait::Builtin { pending, pop };
                    step
                }
                builtin::Outcome::Step(step) => {
                    if pop {
                        self.redirs.pop();
                    }
                    step
                }
                builtin::Outcome::Continue => {
                    self.continued(pop);
                    self.step(host)
                }
            },
        }
    }
    /// A command that changed frames: a redirection it opened now belongs to
    /// the batch file or child it started (and ends with it).
    fn continued(&mut self, pop: bool) {
        if !pop {
            return;
        }
        let depth = self.redirs.len().saturating_sub(1);
        match self.frames.last_mut() {
            Some(Frame::Batch(b)) => b.redir_depth = depth,
            Some(Frame::Code(c)) if c.child => c.redir_depth = depth,
            _ => {
                self.redirs.pop();
            }
        }
    }
    /// Move the innermost code frame past the command that just finished.
    fn advance(&mut self) {
        if let Some(Frame::Code(c)) = self.frames.last_mut() {
            c.pc += 1;
        }
    }
    /// After an interactive command: `/c` exits, otherwise idle.
    fn after_command(&mut self, _host: &mut dyn Host) -> Step {
        if self.once && self.frames.is_empty() {
            return Step::Exit(self.errorlevel);
        }
        Step::Idle
    }

    /// Run until a program, input or the prompt is needed.
    pub fn step(&mut self, host: &mut dyn Host) -> Step {
        let mut budget = 200_000u32;
        loop {
            budget -= 1;
            if budget == 0 {
                // A runaway loop still yields to the UI.
                return Step::Sleep { ms: 0, keys: false };
            }
            let Some(frame) = self.frames.last_mut() else {
                return self.after_command(host);
            };
            match frame {
                Frame::Batch(_) => {
                    if let Some(step) = self.batch_line(host) {
                        return step;
                    }
                }
                Frame::Code(code) => {
                    if code.pc >= code.ops.len() {
                        let depth = code.redir_depth;
                        self.frames.pop();
                        self.redirs.truncate(depth.max(1));
                        continue;
                    }
                    let op = code.ops[code.pc].clone();
                    if let Some(step) = self.exec(host, op) {
                        return step;
                    }
                }
            }
        }
    }
    /// Read, expand, echo, parse and start the next line of the innermost
    /// batch file; pops it at the end.
    fn batch_line(&mut self, host: &mut dyn Host) -> Option<Step> {
        let Some(Frame::Batch(b)) = self.frames.last_mut() else {
            return None;
        };
        let text = b.text.clone();
        let mut pos = b.pos;
        let mut logical = String::new();
        loop {
            if pos >= text.len() {
                if logical.is_empty() {
                    self.end_batch();
                    return None;
                }
                break;
            }
            let end = text[pos..].find('\n').map_or(text.len(), |e| pos + e);
            let mut line = &text[pos..end];
            pos = (end + 1).min(text.len().max(end + 1));
            if line.ends_with('\r') {
                line = &line[..line.len() - 1];
            }
            // A caret at the end escapes the line break.
            if let Some(stripped) = line.strip_suffix('^')
                && !stripped.ends_with('^')
            {
                logical.push_str(stripped);
                continue;
            }
            logical.push_str(line);
            // Percent-expand and see whether a block stays open.
            let args = self.batch_args();
            let expanded = {
                let mut c = Ctx { cmd: self, host };
                expand::percent(&logical, &mut c, Some(&args))
            };
            match parse::parse(&expanded) {
                Err(parse::ParseError::More) if pos < text.len() => {
                    logical.push('\n');
                    continue;
                }
                _ => break,
            }
        }
        if let Some(Frame::Batch(b)) = self.frames.last_mut() {
            b.pos = pos;
        }
        let args = self.batch_args();
        let expanded = {
            let mut c = Ctx { cmd: self, host };
            expand::percent(&logical, &mut c, Some(&args))
        };
        let node = match parse::parse(&expanded) {
            Ok(n) => n,
            Err(parse::ParseError::More) => {
                self.err_line(host, "The syntax of the command is incorrect.");
                self.end_batch();
                return None;
            }
            Err(parse::ParseError::Syntax(m)) => {
                // cmd.exe stops the batch file at a syntax error.
                if self.echo {
                    self.write_console(&format!(
                        "\n{}{}\n",
                        self.expand_prompt(host),
                        logical.trim()
                    ));
                }
                self.err_line(host, &m);
                self.errorlevel = 255;
                while let Some(f) = self.frames.pop() {
                    if matches!(f, Frame::Batch(_)) {
                        self.end_batch_locals();
                        break;
                    }
                }
                return None;
            }
        };
        if node == Node::Empty {
            return None;
        }
        if self.echo && !quiet(&node) {
            let prompt = self.expand_prompt(host);
            self.write_console(&format!("\n{prompt}{}\n", parse::render(&node)));
        }
        let ops = compile(&node);
        let depth = self.redirs.len();
        self.frames.push(Frame::Code(Code {
            ops: Rc::new(ops),
            pc: 0,
            vars: Vec::new(),
            loops: Vec::new(),
            redir_depth: depth,
            for_capture: None,
            child: false,
        }));
        None
    }
    pub(crate) fn batch_args(&self) -> Vec<String> {
        for f in self.frames.iter().rev() {
            if let Frame::Batch(b) = f {
                let mut a: Vec<String> = b.args.iter().skip(b.shift).cloned().collect();
                // %0 stays the batch name when SHIFT /n starts later.
                if a.is_empty() {
                    a.push(String::new());
                }
                return a;
            }
        }
        Vec::new()
    }
    pub(crate) fn current_batch(&mut self) -> Option<&mut Batch> {
        self.frames.iter_mut().rev().find_map(|f| match f {
            Frame::Batch(b) => Some(b),
            _ => None,
        })
    }
    fn end_batch(&mut self) {
        if let Some(Frame::Batch(b)) = self.frames.last() {
            let depth = b.redir_depth;
            if let Some(echo) = b.prompt_echo {
                self.echo = echo;
            }
            self.frames.pop();
            self.redirs.truncate(depth.max(1));
            self.end_batch_locals();
        }
    }
    /// Move the innermost code frame past its current command.
    pub(crate) fn advance_pc(&mut self) {
        self.advance();
    }
    /// `cmd /c line`: a child context whose SET/CD changes end with it.
    pub(crate) fn push_child(
        &mut self,
        host: &mut dyn Host,
        line: &str,
        delayed: bool,
    ) -> builtin::Outcome {
        let expanded = {
            let mut c = Ctx { cmd: self, host };
            expand::percent(line, &mut c, None)
        };
        let node = match parse::parse(&expanded) {
            Ok(n) => n,
            Err(parse::ParseError::More) | Err(parse::ParseError::Syntax(_)) => {
                self.err_line(host, "The syntax of the command is incorrect.");
                return builtin::Outcome::Done(false);
            }
        };
        self.children.push(Local {
            env: self.env.clone(),
            cwd: self.cwd.clone(),
            delayed: self.delayed,
            extensions: self.extensions,
            depth: usize::MAX,
        });
        self.delayed = delayed;
        let mut ops = compile(&node);
        ops.push(Op::RestoreChild);
        self.advance();
        let depth = self.redirs.len();
        self.frames.push(Frame::Code(Code {
            ops: Rc::new(ops),
            pc: 0,
            vars: Vec::new(),
            loops: Vec::new(),
            redir_depth: depth,
            for_capture: None,
            child: true,
        }));
        builtin::Outcome::Continue
    }
    /// EXIT inside a `cmd /c` child ends only the child.
    pub(crate) fn exit_child(&mut self, code: i32) -> bool {
        let Some(at) = self
            .frames
            .iter()
            .rposition(|f| matches!(f, Frame::Code(c) if c.child))
        else {
            return false;
        };
        if self.frames[at..]
            .iter()
            .any(|f| matches!(f, Frame::Batch(_)))
        {
            return false;
        }
        let depth = match &self.frames[at] {
            Frame::Code(c) => c.redir_depth,
            Frame::Batch(_) => unreachable!(),
        };
        self.frames.truncate(at);
        self.redirs.truncate(depth.max(1));
        if let Some(l) = self.children.pop() {
            self.restore(l);
        }
        self.errorlevel = code;
        true
    }
    fn end_batch_locals(&mut self) {
        let depth = self.batch_depth();
        while self.locals.last().is_some_and(|l| l.depth > depth) {
            let l = self.locals.pop().unwrap();
            self.restore(l);
        }
    }
    pub(crate) fn batch_depth(&self) -> usize {
        self.frames
            .iter()
            .filter(|f| matches!(f, Frame::Batch(_)))
            .count()
    }
    pub(crate) fn setlocal(&mut self) {
        let depth = self.batch_depth();
        self.locals.push(Local {
            env: self.env.clone(),
            cwd: self.cwd.clone(),
            delayed: self.delayed,
            extensions: self.extensions,
            depth,
        });
    }
    pub(crate) fn endlocal(&mut self) {
        let depth = self.batch_depth();
        if self
            .locals
            .last()
            .is_some_and(|l| l.depth == depth && depth > 0)
        {
            let l = self.locals.pop().unwrap();
            self.restore(l);
        }
    }
    /// GOTO: continue the innermost batch file after `label`; `:eof` ends
    /// it. Blocks and loops in progress end.
    pub(crate) fn goto(&mut self, host: &mut dyn Host, label: &str) -> bool {
        let label = label.trim().trim_start_matches(':');
        let label = label
            .split([' ', '\t', '+', ':', ',', ';', '='])
            .next()
            .unwrap_or("");
        while let Some(Frame::Code(_)) = self.frames.last() {
            let Some(Frame::Code(c)) = self.frames.pop() else {
                unreachable!()
            };
            self.redirs.truncate(c.redir_depth.max(1));
        }
        if label.eq_ignore_ascii_case("eof") {
            self.end_batch();
            return true;
        }
        let Some(Frame::Batch(b)) = self.frames.last_mut() else {
            return false;
        };
        let text = b.text.clone();
        // Search forward from the current line, then from the top.
        let find = |from: usize, to: usize| -> Option<usize> {
            let mut pos = from;
            while pos < to {
                let end = text[pos..].find('\n').map_or(text.len(), |e| pos + e);
                let line = text[pos..end].trim_start_matches([' ', '\t', '@']);
                if let Some(rest) = line.strip_prefix(':') {
                    let name = rest
                        .split([' ', '\t', '+', ':', ',', ';', '=', '\r'])
                        .next()
                        .unwrap_or("");
                    if name.eq_ignore_ascii_case(label) {
                        return Some((end + 1).min(text.len()));
                    }
                }
                pos = end + 1;
            }
            None
        };
        match find(b.pos, text.len()).or_else(|| find(0, b.pos)) {
            Some(p) => {
                b.pos = p;
                true
            }
            None => {
                self.err_line(host, "The system cannot find the batch label specified - ");
                if let Some(Event::Text(t)) = self.events.last_mut() {
                    t.pop();
                    t.push_str(label);
                    t.push('\n');
                }
                // cmd.exe ends the batch file.
                self.end_batch();
                false
            }
        }
    }
    /// EXIT /B: leave the innermost batch file (or label CALL).
    pub(crate) fn exit_batch(&mut self) -> bool {
        while let Some(Frame::Code(_)) = self.frames.last() {
            let Some(Frame::Code(c)) = self.frames.pop() else {
                unreachable!()
            };
            self.redirs.truncate(c.redir_depth.max(1));
        }
        if matches!(self.frames.last(), Some(Frame::Batch(_))) {
            self.end_batch();
            true
        } else {
            false
        }
    }

    // ----- Output -----
    pub(crate) fn write_console(&mut self, text: &str) {
        if let Some(Event::Text(t)) = self.events.last_mut() {
            t.push_str(text);
        } else {
            self.events.push(Event::Text(text.into()));
        }
    }
    fn write_sink(&mut self, host: &mut dyn Host, sink: Sink, text: &str) {
        match sink {
            Sink::Console => self.write_console(text),
            Sink::Null => (),
            Sink::File(path) => {
                let bytes = lf_to_crlf(text);
                let _ = host.write(&path, bytes.as_bytes(), true);
            }
            Sink::Capture(id) => {
                if let Some(Some(buf)) = self.captures.get_mut(id) {
                    buf.extend_from_slice(text.as_bytes());
                }
            }
        }
    }
    pub(crate) fn out(&mut self, host: &mut dyn Host, text: &str) {
        let sink = self.redirs.last().map_or(Sink::Console, |r| r.out.clone());
        self.write_sink(host, sink, text);
    }
    pub(crate) fn out_line(&mut self, host: &mut dyn Host, text: &str) {
        self.out(host, &format!("{text}\n"));
    }
    pub(crate) fn err(&mut self, host: &mut dyn Host, text: &str) {
        let sink = self.redirs.last().map_or(Sink::Console, |r| r.err.clone());
        self.write_sink(host, sink, text);
    }
    pub(crate) fn err_line(&mut self, host: &mut dyn Host, text: &str) {
        self.err(host, &format!("{text}\n"));
    }
    /// Standard input of the current command, when redirected or piped.
    pub(crate) fn input(&self) -> Option<Rc<Vec<u8>>> {
        self.redirs.last().and_then(|r| r.input.clone())
    }
    pub(crate) fn stdout_is_console(&self) -> bool {
        self.redirs.last().is_none_or(|r| r.out == Sink::Console)
    }
    fn new_capture(&mut self) -> usize {
        if let Some(i) = self.captures.iter().position(Option::is_none) {
            self.captures[i] = Some(Vec::new());
            i
        } else {
            self.captures.push(Some(Vec::new()));
            self.captures.len() - 1
        }
    }
    /// Open redirections over the current ones (files relative to cwd).
    pub(crate) fn open_redirects(
        &mut self,
        host: &mut dyn Host,
        redirects: &[Redirect],
        vars: &[(char, String)],
    ) -> Result<Redir, String> {
        let mut r = self.redirs.last().cloned().unwrap_or(Redir {
            out: Sink::Console,
            err: Sink::Console,
            input: None,
        });
        for red in redirects {
            match red {
                Redirect::Out { fd, target, append } => {
                    let target = self.substitute(host, target, vars);
                    let sink = self.file_sink(host, &target, *append)?;
                    if *fd == 2 {
                        r.err = sink;
                    } else {
                        r.out = sink;
                    }
                }
                Redirect::In(target) => {
                    let target = self.substitute(host, target, vars);
                    if wpath::device(&target) == Some("NUL") {
                        r.input = Some(Rc::new(Vec::new()));
                        continue;
                    }
                    let path = self.full(&target).map_err(|e| String::from(e.message()))?;
                    let native = host
                        .native(&path)
                        .ok_or_else(|| String::from(FsError::PathNotFound.message()))?;
                    let data = host
                        .read(&native, 16 << 20)
                        .map_err(|e| String::from(e.message()))?;
                    r.input = Some(Rc::new(data));
                }
                Redirect::Dup { fd, to } => {
                    let src = if *to == 2 {
                        r.err.clone()
                    } else {
                        r.out.clone()
                    };
                    if *fd == 2 {
                        r.err = src;
                    } else {
                        r.out = src;
                    }
                }
            }
        }
        Ok(r)
    }
    fn file_sink(
        &mut self,
        host: &mut dyn Host,
        target: &str,
        append: bool,
    ) -> Result<Sink, String> {
        match wpath::device(target) {
            Some("NUL") => return Ok(Sink::Null),
            Some("CON" | "CONOUT$") => return Ok(Sink::Console),
            Some(_) => return Err(String::from(FsError::Device.message())),
            None => (),
        }
        let path = self.full(target).map_err(|e| String::from(e.message()))?;
        let native = host
            .native(&path)
            .ok_or_else(|| String::from(FsError::PathNotFound.message()))?;
        if let Ok(m) = host.stat(&native)
            && m.dir
        {
            return Err(String::from(FsError::Denied.message()));
        }
        host.write(&native, b"", append)
            .map_err(|e| String::from(e.message()))?;
        Ok(Sink::File(native))
    }
    /// FOR variables then delayed expansion.
    pub(crate) fn substitute(
        &mut self,
        host: &mut dyn Host,
        text: &str,
        vars: &[(char, String)],
    ) -> String {
        let delayed = self.delayed;
        let mut c = Ctx { cmd: self, host };
        let t = expand::for_vars(text, vars, &mut c);
        if delayed {
            expand::delayed(&t, &mut c)
        } else {
            t
        }
    }
    /// A path typed by the user resolved against the current directory.
    pub(crate) fn full(&self, path: &str) -> Result<String, FsError> {
        let p = path.trim().trim_matches('"');
        if p.is_empty() {
            return Err(FsError::Invalid);
        }
        wpath::full(
            &self.cwd,
            &|d| {
                self.drives
                    .iter()
                    .find(|(x, _)| *x == d)
                    .map(|(_, p)| p.clone())
            },
            p,
        )
    }
    pub(crate) fn native(&self, host: &dyn Host, path: &str) -> Result<String, FsError> {
        let full = self.full(path)?;
        host.native(&full).ok_or(FsError::PathNotFound)
    }
    pub(crate) fn var(&self, host: &dyn Host, name: &str) -> Option<String> {
        if let Some(v) = self.env.get(name) {
            return Some(v.into());
        }
        let upper = name.to_ascii_uppercase();
        Some(match upper.as_str() {
            "CD" => self.cwd.clone(),
            "__CD__" => format!(
                "{}{}",
                self.cwd,
                if self.cwd.ends_with('\\') { "" } else { "\\" }
            ),
            "DATE" => crate::format::date_long(&host.now()),
            "TIME" => crate::format::time_long(&host.now()),
            "ERRORLEVEL" => format!("{}", self.errorlevel),
            "CMDEXTVERSION" => String::from("2"),
            "CMDCMDLINE" => self.cmdline.clone(),
            "HIGHESTNUMANODENUMBER" => String::from("0"),
            "__APPDIR__" => String::from("C:\\Windows\\system32\\"),
            _ => return None,
        })
    }

    // ----- The VM -----
    /// Execute one op; Some when the engine must yield.
    fn exec(&mut self, host: &mut dyn Host, op: Op) -> Option<Step> {
        match op {
            Op::Run(simple) => self.run_simple(host, &simple),
            Op::Jump(to) => {
                self.set_pc(to);
                None
            }
            Op::JumpIfOk(to) => {
                if self.ok {
                    self.set_pc(to);
                } else {
                    self.advance();
                }
                None
            }
            Op::JumpIfFail(to) => {
                if self.ok {
                    self.advance();
                } else {
                    self.set_pc(to);
                }
                None
            }
            Op::If {
                not,
                insensitive,
                cond,
                otherwise,
            } => {
                let vars = self.vars();
                match self.condition(host, &cond, insensitive, &vars) {
                    Ok(v) => {
                        if v != not {
                            self.advance();
                        } else {
                            self.set_pc(otherwise);
                        }
                    }
                    Err(m) => {
                        self.err_line(host, &m);
                        self.set_pc(usize::MAX);
                    }
                }
                None
            }
            Op::PushRedirect { redirects, skip } => {
                let vars = self.vars();
                match self.open_redirects(host, &redirects, &vars) {
                    Ok(r) => {
                        self.redirs.push(r);
                        self.advance();
                    }
                    Err(m) => {
                        self.err_line(host, &m);
                        self.errorlevel = 1;
                        self.ok = false;
                        self.set_pc(skip);
                    }
                }
                None
            }
            Op::PopRedirect => {
                if self.redirs.len() > 1 {
                    self.redirs.pop();
                }
                self.advance();
                None
            }
            Op::PipeBegin => {
                let id = self.new_capture();
                let mut r = self.redirs.last().cloned().unwrap();
                r.out = Sink::Capture(id);
                self.redirs.push(r);
                self.advance();
                None
            }
            Op::PipeMid => {
                let r = self.redirs.pop().unwrap();
                let data = match r.out {
                    Sink::Capture(id) => self
                        .captures
                        .get_mut(id)
                        .and_then(Option::take)
                        .unwrap_or_default(),
                    _ => Vec::new(),
                };
                let mut next = self.redirs.last().cloned().unwrap();
                next.input = Some(Rc::new(data));
                self.redirs.push(next);
                self.advance();
                None
            }
            Op::PipeEnd => {
                if self.redirs.len() > 1 {
                    self.redirs.pop();
                }
                self.advance();
                None
            }
            Op::ForBegin { spec, end } => self.for_begin(host, &spec, end),
            Op::RestoreChild => {
                // The child's exit code is its last command's.
                let level = self.errorlevel;
                if let Some(l) = self.children.pop() {
                    self.restore(l);
                }
                self.errorlevel = level;
                self.advance();
                None
            }
            Op::ForNext { end } => {
                let Some(Frame::Code(c)) = self.frames.last_mut() else {
                    return None;
                };
                match c.loops.last_mut().and_then(forloop::Loop::next) {
                    Some(binding) => {
                        let base = c.loops.last().map_or(0, |l| l.base);
                        c.vars.truncate(base);
                        c.vars.extend(binding);
                        c.pc += 1;
                    }
                    None => {
                        if let Some(l) = c.loops.pop() {
                            c.vars.truncate(l.base);
                        }
                        c.pc = end;
                    }
                }
                None
            }
        }
    }
    fn set_pc(&mut self, to: usize) {
        if let Some(Frame::Code(c)) = self.frames.last_mut() {
            c.pc = to;
        }
    }
    pub(crate) fn vars(&self) -> Vec<(char, String)> {
        match self.frames.last() {
            Some(Frame::Code(c)) => c.vars.clone(),
            _ => Vec::new(),
        }
    }
    fn in_loop(&self) -> bool {
        matches!(self.frames.last(), Some(Frame::Code(c)) if !c.loops.is_empty())
    }
    fn condition(
        &mut self,
        host: &mut dyn Host,
        cond: &Cond,
        insensitive: bool,
        vars: &[(char, String)],
    ) -> Result<bool, String> {
        Ok(match cond {
            Cond::ErrorLevel(n) => {
                let n = self.substitute(host, n, vars);
                let level: i32 = n
                    .trim()
                    .parse()
                    .map_err(|_| String::from("The syntax of the command is incorrect."))?;
                self.errorlevel >= level
            }
            Cond::Exist(p) => {
                let p = self.substitute(host, p, vars);
                let p = p.trim_matches('"');
                if wpath::device(p).is_some() {
                    true
                } else if wpath::wild(p) {
                    !self.glob(host, p, false).is_empty()
                } else {
                    let trailing = p.ends_with('\\');
                    match self.native(host, p) {
                        Ok(n) => host.stat(&n).is_ok_and(|m| !trailing || m.dir),
                        Err(_) => false,
                    }
                }
            }
            Cond::Defined(v) => {
                let v = self.substitute(host, v, vars);
                self.var(host, &v).is_some()
            }
            Cond::CmdExtVersion(n) => {
                let n = self.substitute(host, n, vars);
                n.trim().parse::<i32>().is_ok_and(|n| n <= 2)
            }
            Cond::Compare { lhs, op, rhs } => {
                let a = self.substitute(host, lhs, vars);
                let b = self.substitute(host, rhs, vars);
                compare(&a, *op, &b, insensitive)
            }
        })
    }
    /// Files (or directories) matching a wildcard path, sorted as NTFS
    /// lists them; hidden and system entries excluded, as FOR and IF.
    pub(crate) fn glob(
        &mut self,
        host: &mut dyn Host,
        pattern: &str,
        dirs: bool,
    ) -> Vec<(String, Meta)> {
        let Ok(full) = self.full(pattern) else {
            return Vec::new();
        };
        let (dir, name) = wpath::split(&full);
        let Some(native) = host.native(dir) else {
            return Vec::new();
        };
        let Ok(mut entries) = host.list(&native) else {
            return Vec::new();
        };
        entries.sort_by_key(|e| e.name.to_lowercase());
        entries
            .into_iter()
            .filter(|e| {
                e.meta.dir == dirs
                    && e.meta.attrs & (crate::host::attr::HIDDEN | crate::host::attr::SYSTEM) == 0
            })
            .filter(|e| crate::wild::matches(name, &e.name))
            .map(|e| (wpath::join(dir, &e.name), e.meta))
            .collect()
    }
    fn for_begin(&mut self, host: &mut dyn Host, spec: &parse::For, end: usize) -> Option<Step> {
        let vars = self.vars();
        let finished = match self.frames.last_mut() {
            Some(Frame::Code(c)) => c.for_capture.take(),
            _ => None,
        };
        let source = match finished {
            Some(id) => {
                // Our capture redirect is the innermost; drop it.
                self.redirs.pop();
                let data = self
                    .captures
                    .get_mut(id)
                    .and_then(Option::take)
                    .unwrap_or_default();
                Some(crlf_to_lf(&data))
            }
            None => None,
        };
        let built = forloop::build(self, host, spec, &vars, source);
        match built {
            forloop::Built::Loop(mut l) => {
                let Some(Frame::Code(c)) = self.frames.last_mut() else {
                    return None;
                };
                l.base = c.vars.len();
                c.loops.push(l);
                c.pc += 1;
                None
            }
            forloop::Built::Command(line) => {
                // Run the command with its output captured, then come back.
                let id = self.new_capture();
                let mut r = self.redirs.last().cloned().unwrap();
                r.out = Sink::Capture(id);
                self.redirs.push(r);
                if let Some(Frame::Code(c)) = self.frames.last_mut() {
                    c.for_capture = Some(id);
                }
                match parse::parse(&line) {
                    Ok(node) => {
                        let ops = compile(&node);
                        let depth = self.redirs.len();
                        self.frames.push(Frame::Code(Code {
                            ops: Rc::new(ops),
                            pc: 0,
                            vars,
                            loops: Vec::new(),
                            redir_depth: depth,
                            for_capture: None,
                            child: false,
                        }));
                    }
                    Err(_) => {
                        self.err_line(host, "The syntax of the command is incorrect.");
                    }
                }
                None
            }
            forloop::Built::Error(m) => {
                if !m.is_empty() {
                    self.err_line(host, &m);
                }
                self.set_pc(end);
                None
            }
        }
    }
    /// One simple command: substitute, echo inside loops, redirect, run.
    fn run_simple(&mut self, host: &mut dyn Host, s: &Simple) -> Option<Step> {
        let vars = self.vars();
        let head = self.substitute(host, &s.head, &vars);
        let tail = self.substitute(host, &s.tail, &vars);
        if self.echo && !s.quiet && self.in_loop() && self.batch_depth() > 0 {
            let prompt = self.expand_prompt(host);
            self.write_console(&format!("\n{prompt}{head}{tail}\n"));
        }
        let pop = !s.redirects.is_empty();
        if pop {
            match self.open_redirects(host, &s.redirects, &vars) {
                Ok(r) => self.redirs.push(r),
                Err(m) => {
                    self.err_line(host, &m);
                    self.errorlevel = 1;
                    self.ok = false;
                    self.advance();
                    return None;
                }
            }
        }
        let outcome = self.dispatch(host, &head, &tail);
        match outcome {
            builtin::Outcome::Done(ok) => {
                self.ok = ok;
                if pop {
                    self.redirs.pop();
                }
                self.advance();
                None
            }
            builtin::Outcome::Wait(pending, step) => {
                self.wait = Wait::Builtin { pending, pop };
                Some(step)
            }
            builtin::Outcome::Continue => {
                self.continued(pop);
                None
            }
            builtin::Outcome::Step(step) => {
                if let Step::Spawn(sp) = &step {
                    let capture = (sp.capture).then_some(0);
                    self.wait = Wait::Program {
                        capture,
                        pop,
                        quiet_status: sp.detached,
                    };
                } else if let Step::Launch(_) = &step {
                    self.wait = Wait::Program {
                        capture: None,
                        pop,
                        quiet_status: true,
                    };
                } else if let Step::Exit(code) = &step {
                    self.wait = Wait::Exit(*code);
                } else if pop {
                    self.redirs.pop();
                }
                Some(step)
            }
        }
    }
    /// Internal command, drive change, batch file, tool or program.
    fn dispatch(&mut self, host: &mut dyn Host, head: &str, tail: &str) -> builtin::Outcome {
        let name = head.trim_matches('"');
        let lower = name.to_ascii_lowercase();
        if lower.len() == 2
            && lower.ends_with(':')
            && lower.as_bytes()[0].is_ascii_alphabetic()
            && tail.trim().is_empty()
        {
            return builtin::change_drive(self, host, lower.as_bytes()[0] as char);
        }
        if !head.starts_with('"') && builtin::NAMES.contains(&lower.as_str()) {
            if tail.trim() == "/?" {
                let text = help::text(&lower);
                self.out(host, &text);
                return builtin::Outcome::Done(true);
            }
            return builtin::run(self, host, &lower, tail);
        }
        self.external(host, head, tail)
    }
    /// A batch file, built-in System32 tool, program, document or legacy
    /// Windows name.
    pub(crate) fn external(
        &mut self,
        host: &mut dyn Host,
        head: &str,
        tail: &str,
    ) -> builtin::Outcome {
        let name = head.trim_matches('"');
        // `pwsh` and `powershell` run PowerShell 7 in this console where it
        // is installed, as typing them in a Windows Terminal tab does
        // (Windows PowerShell 5.1 has no Doors counterpart).
        let lower = name.to_ascii_lowercase();
        let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
        if matches!(stem, "pwsh" | "powershell")
            && matches!(self.resolve(host, POWERSHELL_7), Some(Found::Program(..)))
        {
            return self.external(host, &format!("\"{POWERSHELL_7}\""), tail);
        }
        match self.resolve(host, name) {
            Some(Found::Batch(path)) => {
                let args = batch_words(tail);
                match self.open_batch(host, &path, &args) {
                    Ok(mut b) => {
                        // Without CALL a batch file replaces the running one
                        // (and keeps its echo state).
                        if self.batch_depth() > 0 {
                            let echo = self.echo;
                            let saved = self.current_batch().and_then(|c| c.prompt_echo.take());
                            self.exit_batch();
                            b.prompt_echo = saved;
                            self.echo = echo;
                        } else {
                            self.advance();
                        }
                        self.frames.push(Frame::Batch(b));
                        builtin::Outcome::Continue
                    }
                    Err(e) => {
                        self.err_line(host, e.message());
                        builtin::Outcome::Done(false)
                    }
                }
            }
            Some(Found::Tool(tool)) => {
                if tail.trim() == "/?" {
                    let t = help::text(tool);
                    self.out(host, &t);
                    return builtin::Outcome::Done(true);
                }
                tools::run(self, host, tool, tail)
            }
            Some(Found::Program(path, native)) => {
                // Windows' own program names (C:\Windows\...\notepad.exe,
                // powershell.exe) open the Doors app that stands for them; a
                // program installed elsewhere under such a name (PowerShell 7
                // in C:\Program Files) is that program and runs.
                let system = path.to_ascii_lowercase().starts_with("c:\\windows\\");
                if system
                    && let Some(crate::legacy::Target::App(app)) =
                        crate::legacy::resolve(name, &format!("{name}{tail}"))
                {
                    return self.launch_app(host, app, tail);
                }
                let command_line = format!("{head}{tail}");
                let argv = crate::split_command_line(&command_line);
                let cwd = host.native(&self.cwd).unwrap_or_default();
                builtin::Outcome::Step(Step::Spawn(Spawn {
                    program: native,
                    command_line,
                    argv,
                    cwd,
                    environment: self.env.pairs(),
                    input: self.input().map(|i| (*i).clone()),
                    capture: !self.stdout_is_console(),
                    detached: false,
                }))
            }
            Some(Found::Document(native)) => {
                self.ok = true;
                builtin::Outcome::Step(Step::Launch(Launch::Open(native)))
            }
            None => {
                if let Some(target) = crate::legacy::resolve(name, &format!("{name}{tail}")) {
                    return match target {
                        crate::legacy::Target::App(app) => self.launch_app(host, app, tail),
                        crate::legacy::Target::Settings(nav) => {
                            builtin::Outcome::Step(Step::Launch(Launch::Settings(nav)))
                        }
                    };
                }
                self.err_line(host, &format!("'{name}' is not recognized as an internal or external command,\noperable program or batch file."));
                self.errorlevel = 9009;
                builtin::Outcome::Done(false)
            }
        }
    }
    pub(crate) fn launch_app(
        &mut self,
        host: &mut dyn Host,
        app: crate::legacy::App,
        tail: &str,
    ) -> builtin::Outcome {
        let arg = tail.trim();
        let argument = if arg.is_empty() || !app.opens_files() {
            None
        } else {
            let unq = arg.trim_matches('"');
            Some(self.native(host, unq).unwrap_or_else(|_| String::from(unq)))
        };
        builtin::Outcome::Step(Step::Launch(Launch::App { app, argument }))
    }
    /// PATH/PATHEXT search: the current directory first, then each PATH
    /// directory; built-in tools live in System32.
    pub(crate) fn resolve(&mut self, host: &mut dyn Host, name: &str) -> Option<Found> {
        if name.is_empty() {
            return None;
        }
        let pathext: Vec<String> = self
            .env
            .get("PATHEXT")
            .unwrap_or(".COM;.EXE;.BAT;.CMD")
            .split(';')
            .filter(|e| !e.is_empty())
            .map(|e| e.to_ascii_lowercase())
            .collect();
        let ext = wpath::extension(name).to_ascii_lowercase();
        let has_path = name.contains(['\\', '/', ':']);
        let candidates = |base: &str| -> Vec<String> {
            let mut v = Vec::new();
            if !ext.is_empty() {
                v.push(String::from(base));
            }
            for e in &pathext {
                v.push(format!("{base}{e}"));
            }
            v
        };
        let try_dir = |cmd: &mut Self, host: &mut dyn Host, dir: Option<&str>| -> Option<Found> {
            let base = match dir {
                Some(d) => wpath::join(d, name),
                None => cmd.full(name).ok()?,
            };
            for c in candidates(&base) {
                let Some(native) = host.native(&c).and_then(|n| host.exact(&n)) else {
                    continue;
                };
                if let Ok(m) = host.stat(&native)
                    && !m.dir
                {
                    return Some(classify(&c, native, &pathext));
                }
            }
            None
        };
        if has_path {
            let found = try_dir(self, host, None);
            if found.is_none() {
                // C:\Windows\System32\find.exe and friends are built in.
                let full = self.full(name).ok()?;
                let (dir, file) = wpath::split(&full);
                if dir.eq_ignore_ascii_case("C:\\Windows\\System32") {
                    let stem = file
                        .trim_end_matches(".exe")
                        .trim_end_matches(".EXE")
                        .to_ascii_lowercase();
                    return tools::NAMES
                        .iter()
                        .find(|t| **t == stem)
                        .map(|t| Found::Tool(t));
                }
            }
            return found;
        }
        if let Some(f) = try_dir(self, host, Some(&self.cwd.clone())) {
            return Some(f);
        }
        let stem = name
            .strip_suffix(".exe")
            .or_else(|| name.strip_suffix(".EXE"))
            .unwrap_or(name)
            .to_ascii_lowercase();
        if let Some(t) = tools::NAMES.iter().find(|t| **t == stem) {
            return Some(Found::Tool(t));
        }
        let path = self.env.get("PATH").unwrap_or("").to_owned_string();
        for dir in path
            .split(';')
            .map(|d| d.trim().trim_matches('"'))
            .filter(|d| !d.is_empty())
        {
            if let Some(f) = try_dir(self, host, Some(dir)) {
                return Some(f);
            }
        }
        None
    }
}
/// A dotted IPv4 address.
pub fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = s.split('.');
    for b in &mut out {
        *b = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}
/// PowerShell 7, where its installer puts it.
const POWERSHELL_7: &str = "C:\\Program Files\\PowerShell\\7\\pwsh.exe";
pub(crate) enum Found {
    Batch(String),
    Tool(&'static str),
    Program(String, String),
    Document(String),
}
fn classify(windows: &str, native: String, pathext: &[String]) -> Found {
    let ext = wpath::extension(windows).to_ascii_lowercase();
    match ext.as_str() {
        ".bat" | ".cmd" => Found::Batch(windows.into()),
        ".exe" | ".com" => Found::Program(windows.into(), native),
        _ if ext.is_empty() => Found::Program(windows.into(), native),
        _ if pathext.contains(&ext)
            && !matches!(
                ext.as_str(),
                ".vbs" | ".vbe" | ".js" | ".jse" | ".wsf" | ".wsh" | ".msc"
            ) =>
        {
            Found::Program(windows.into(), native)
        }
        _ => Found::Document(native),
    }
}
/// Batch arguments: delimiters are spaces, tabs, `,`, `;`, `=`; quotes kept.
pub(crate) fn batch_words(tail: &str) -> Vec<String> {
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
fn quiet(node: &Node) -> bool {
    match node {
        Node::Simple(s) => s.quiet,
        Node::Seq(a, _) | Node::And(a, _) | Node::Or(a, _) | Node::Pipe(a, _) => quiet(a),
        _ => false,
    }
}
fn number(s: &str) -> Option<i64> {
    let t = s.trim();
    let (neg, t) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    if t.is_empty() {
        return None;
    }
    let v = if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        i64::from_str_radix(h, 16).ok()?
    } else if t.len() > 1 && t.starts_with('0') {
        i64::from_str_radix(&t[1..], 8).ok()?
    } else {
        t.parse().ok()?
    };
    Some(if neg { -v } else { v })
}
fn compare(a: &str, op: CmpOp, b: &str, insensitive: bool) -> bool {
    if op == CmpOp::Eq {
        return if insensitive {
            a.to_lowercase() == b.to_lowercase()
        } else {
            a == b
        };
    }
    let ord = match (number(a), number(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        _ => {
            if insensitive {
                a.to_lowercase().cmp(&b.to_lowercase())
            } else {
                a.cmp(b)
            }
        }
    };
    use core::cmp::Ordering::*;
    match op {
        CmpOp::Equ => ord == Equal,
        CmpOp::Neq => ord != Equal,
        CmpOp::Lss => ord == Less,
        CmpOp::Leq => ord != Greater,
        CmpOp::Gtr => ord == Greater,
        CmpOp::Geq => ord != Less,
        CmpOp::Eq => unreachable!(),
    }
}
/// Compile a parsed line into ops.
pub(crate) fn compile(node: &Node) -> Vec<Op> {
    let mut ops = Vec::new();
    emit(node, &mut ops);
    ops
}
fn emit(node: &Node, ops: &mut Vec<Op>) {
    match node {
        Node::Empty => (),
        Node::Simple(s) => ops.push(Op::Run(Rc::new(s.clone()))),
        Node::Seq(a, b) => {
            emit(a, ops);
            emit(b, ops);
        }
        Node::And(a, b) | Node::Or(a, b) => {
            emit(a, ops);
            let at = ops.len();
            ops.push(Op::Jump(0));
            emit(b, ops);
            let end = ops.len();
            ops[at] = if matches!(node, Node::And(..)) {
                Op::JumpIfFail(end)
            } else {
                Op::JumpIfOk(end)
            };
        }
        Node::Pipe(a, b) => {
            ops.push(Op::PipeBegin);
            emit(a, ops);
            ops.push(Op::PipeMid);
            emit(b, ops);
            ops.push(Op::PipeEnd);
        }
        Node::Block { body, redirects } => {
            if redirects.is_empty() {
                emit(body, ops);
            } else {
                let at = ops.len();
                ops.push(Op::Jump(0));
                emit(body, ops);
                ops.push(Op::PopRedirect);
                let end = ops.len();
                ops[at] = Op::PushRedirect {
                    redirects: Rc::new(redirects.clone()),
                    skip: end,
                };
            }
        }
        Node::If {
            not,
            insensitive,
            cond,
            then,
            otherwise,
        } => {
            let at = ops.len();
            ops.push(Op::Jump(0));
            emit(then, ops);
            match otherwise {
                Some(e) => {
                    let jump = ops.len();
                    ops.push(Op::Jump(0));
                    let else_at = ops.len();
                    emit(e, ops);
                    let end = ops.len();
                    ops[jump] = Op::Jump(end);
                    ops[at] = Op::If {
                        not: *not,
                        insensitive: *insensitive,
                        cond: Rc::new(cond.clone()),
                        otherwise: else_at,
                    };
                }
                None => {
                    let end = ops.len();
                    ops[at] = Op::If {
                        not: *not,
                        insensitive: *insensitive,
                        cond: Rc::new(cond.clone()),
                        otherwise: end,
                    };
                }
            }
        }
        Node::For(f) => {
            let begin = ops.len();
            ops.push(Op::Jump(0));
            let next = ops.len();
            ops.push(Op::Jump(0));
            emit(&f.body, ops);
            ops.push(Op::Jump(next));
            let end = ops.len();
            ops[begin] = Op::ForBegin {
                spec: Rc::new((**f).clone()),
                end,
            };
            ops[next] = Op::ForNext { end };
        }
    }
}
/// The expansion context over an interpreter and its host.
pub(crate) struct Ctx<'a> {
    pub cmd: &'a Cmd,
    pub host: &'a mut dyn Host,
}
impl expand::Ctx for Ctx<'_> {
    fn var(&mut self, name: &str) -> Option<String> {
        if name.eq_ignore_ascii_case("RANDOM") && self.cmd.env.get(name).is_none() {
            return Some(format!("{}", self.host.random() % 32768));
        }
        self.cmd.var(self.host, name)
    }
    fn file(&mut self, path: &str) -> (String, Option<Meta>) {
        match self.cmd.full(path) {
            Ok(full) => {
                let meta = self
                    .host
                    .native(&full)
                    .and_then(|n| self.host.stat(&n).ok());
                (full, meta)
            }
            Err(_) => (String::from(path), None),
        }
    }
    fn search(&mut self, variable: &str, name: &str) -> Option<String> {
        let dirs = self.cmd.var(self.host, variable)?;
        let mut all = vec![self.cmd.cwd.clone()];
        all.extend(dirs.split(';').filter(|d| !d.is_empty()).map(String::from));
        for d in all {
            let p = wpath::join(&d, name);
            if self
                .host
                .native(&p)
                .is_some_and(|n| self.host.stat(&n).is_ok())
            {
                return Some(p);
            }
        }
        None
    }
    fn local_time(&mut self, filetime: u64) -> crate::host::LocalTime {
        self.host.local(filetime)
    }
}
pub(crate) fn crlf_to_lf(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.replace("\r\n", "\n")
}
fn lf_to_crlf(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut prev = '\0';
    for c in text.chars() {
        if c == '\n' && prev != '\r' {
            out.push('\r');
        }
        out.push(c);
        prev = c;
    }
    out
}
trait OwnedString {
    fn to_owned_string(&self) -> String;
}
impl OwnedString for str {
    fn to_owned_string(&self) -> String {
        String::from(self)
    }
}

#[cfg(test)]
mod tests;

//! Doorsh, the Doors shell: a structured, scriptable native shell.
//!
//! Commands pass values (text, numbers, sizes, lists, records, tables)
//! down a pipeline and the console renders what reaches its end. Scripts
//! (`.dsh`) have variables, expressions, functions, `if`/`for`/`while`,
//! `try`/`catch`. Anything that is not a Doorsh command, function or alias
//! runs through cmd.exe's engine (the System32 tools, Windows programs,
//! batch files and legacy names), its output flowing into the pipeline as
//! text. Like cmd it is a resumable machine: running a program, waiting for
//! a key or opening an app yields a [`Step`] and the caller resumes it.
//!
//! Safety: Doorsh holds only the signed-in user's rights (every operation
//! goes through the same Host the cmd engine uses), never re-splits
//! arguments, asks before deleting more than one item or a folder tree,
//! and refuses to delete system locations.
mod builtins;
mod help;
pub mod parse;
#[cfg(test)]
mod tests;
pub mod value;

use crate::cmd::{Cmd, Event};
use crate::host::Host;
use crate::{Resume, Step};
use alloc::{
    collections::BTreeMap,
    format,
    rc::Rc,
    string::{String, ToString},
    vec::Vec,
};
use parse::{
    Command, Cond, Expr, Function, Iter, Joint, Op, Part, Pipeline, RedirectKind, Stmt, Word,
};
use value::Value;

/// A failure: its message and the status it leaves in `$?`.
#[derive(Clone, Debug, PartialEq)]
pub struct Fail {
    pub message: String,
    pub code: i32,
}
pub fn fail(message: impl Into<String>) -> Fail {
    Fail {
        message: message.into(),
        code: 1,
    }
}
pub type Res<T> = Result<T, Fail>;

/// Where a finished pipeline's value goes.
#[derive(Clone, Debug, PartialEq)]
enum Sink {
    /// The console (and redirections).
    Console,
    /// Kept for the caller (`$( )`, `for`, `let`, conditions).
    Keep,
}
#[derive(Clone, Debug)]
struct Pipe {
    commands: Rc<Vec<Command>>,
    index: usize,
    value: Value,
    sink: Sink,
    /// What the current stage waits for.
    waiting: Waiting,
}
#[derive(Clone, Debug, PartialEq)]
enum Waiting {
    Nothing,
    /// A function call's frames run above this pipe.
    Function,
    /// The cmd engine runs a program or asks for input.
    Interop {
        capture: bool,
    },
    /// A built-in yielded (sleep, a key, a line, an app).
    Builtin(builtins::Pending),
    /// A Doors program runs.
    Native {
        capture: bool,
    },
}
#[derive(Clone, Debug)]
enum Frame {
    Block {
        stmts: Rc<Vec<Stmt>>,
        pc: usize,
    },
    Chain {
        items: Rc<Vec<(Joint, Pipeline)>>,
        index: usize,
    },
    If {
        arms: Rc<Vec<(Cond, Vec<Stmt>)>>,
        otherwise: Option<Rc<Vec<Stmt>>>,
        index: usize,
        checking: bool,
    },
    For {
        var: String,
        items: Vec<Value>,
        index: usize,
        body: Rc<Vec<Stmt>>,
    },
    While {
        cond: Rc<Cond>,
        body: Rc<Vec<Stmt>>,
        forever: bool,
        checking: bool,
    },
    /// A function or script boundary: its scope is popped with it.
    Call {
        scoped: bool,
        script: bool,
    },
    Try {
        var: Option<String>,
        catch: Rc<Vec<Stmt>>,
    },
    Pipe(Pipe),
}
/// What a loop-control or return statement unwinds to.
enum Unwind {
    Break,
    Continue,
    Return(Value),
}
pub struct Doorsh {
    /// Current directory (a Windows path, as File Explorer shows it).
    pub cwd: String,
    pub home: String,
    scopes: Vec<BTreeMap<String, Value>>,
    pub(crate) env: Vec<(String, String)>,
    functions: BTreeMap<String, Rc<Function>>,
    pub(crate) aliases: BTreeMap<String, String>,
    pub status: i32,
    last_arg: String,
    events: Vec<Event>,
    frames: Vec<Frame>,
    /// A value a finished nested run (function, `$( )`) handed back.
    result: Option<Value>,
    pub(crate) prev_dir: Option<String>,
    pub(crate) dir_stack: Vec<String>,
    pub(crate) marks: BTreeMap<String, String>,
    pub(crate) history: Vec<String>,
    /// Console width for tables.
    pub width: usize,
    exit: Option<i32>,
    /// cmd.exe's engine for everything Doorsh does not do itself.
    interop: Cmd,
    /// Text the interop engine printed for the current stage.
    captured: String,
    pub title: String,
    /// Ticks when the current line started (`time`).
    started: u64,
}
pub const VERSION: &str = "1.0";
pub const MAX_SCRIPT: usize = 512 * 1024;

impl Doorsh {
    /// `env`: the session environment (cmd.exe's, shared with the tools it
    /// runs); `cwd` and `home` Windows paths.
    pub fn new(env: crate::cmd::Env, cwd: &str, home: &str) -> Self {
        let pairs: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| (String::from(k), String::from(v)))
            .collect();
        let interop = Cmd::new(env, cwd);
        let mut aliases = BTreeMap::new();
        for (a, b) in builtins::DEFAULT_ALIASES {
            aliases.insert(String::from(*a), String::from(*b));
        }
        Self {
            cwd: String::from(cwd),
            home: String::from(home),
            scopes: alloc::vec![BTreeMap::new()],
            env: pairs,
            functions: BTreeMap::new(),
            aliases,
            status: 0,
            last_arg: String::new(),
            events: Vec::new(),
            frames: Vec::new(),
            result: None,
            prev_dir: None,
            dir_stack: Vec::new(),
            marks: BTreeMap::new(),
            history: Vec::new(),
            width: 100,
            exit: None,
            interop,
            captured: String::new(),
            title: String::from("Doorsh"),
            started: 0,
        }
    }
    pub fn banner() -> String {
        format!(
            "Doorsh {VERSION}, the Doors shell\nType 'help' for commands, 'help <command>' for details and examples.\n\n"
        )
    }
    /// The prompt: the folder (the profile as ~) and `>`.
    pub fn prompt(&self) -> String {
        format!("{}> ", self.short_cwd())
    }
    pub fn short_cwd(&self) -> String {
        if self.cwd.eq_ignore_ascii_case(&self.home) {
            return String::from("~");
        }
        let prefix = format!("{}\\", self.home);
        if self.cwd.len() > prefix.len() && self.cwd[..prefix.len()].eq_ignore_ascii_case(&prefix) {
            return format!("~\\{}", &self.cwd[prefix.len()..]);
        }
        self.cwd.clone()
    }
    /// Ctrl+C: stop the running line or script.
    pub fn interrupt(&mut self) -> Step {
        if !self.frames.is_empty() {
            self.frames.clear();
            self.out_line("^C");
            self.status = 130;
        }
        Step::Idle
    }
    pub fn take_events(&mut self) -> Vec<Event> {
        core::mem::take(&mut self.events)
    }
    pub fn busy(&self) -> bool {
        !self.frames.is_empty()
    }
    pub(crate) fn out(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        match self.events.last_mut() {
            Some(Event::Text(t)) => t.push_str(text),
            _ => self.events.push(Event::Text(String::from(text))),
        }
    }
    pub(crate) fn out_line(&mut self, text: &str) {
        self.out(text);
        self.out("\n");
    }
    pub(crate) fn clear(&mut self) {
        self.events.push(Event::Clear);
    }
    pub(crate) fn set_title(&mut self, title: &str) {
        self.title = String::from(title);
        self.events.push(Event::Title(String::from(title)));
    }
    /// Run a typed line.
    pub fn submit(&mut self, host: &mut dyn Host, line: &str) -> Step {
        self.frames.clear();
        self.started = host.ticks();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return Step::Idle;
        }
        // History expansion: !! and !prefix.
        let line = match self.expand_history(trimmed) {
            Some(expanded) => {
                self.out_line(&expanded);
                expanded
            }
            None => String::from(trimmed),
        };
        if !line.starts_with(' ') {
            self.history.push(line.clone());
            if self.history.len() > 1000 {
                self.history.remove(0);
            }
        }
        match parse::parse(&line) {
            Ok(stmts) => {
                self.frames.push(Frame::Block {
                    stmts: Rc::new(stmts),
                    pc: 0,
                });
                self.run(host)
            }
            Err(e) => {
                self.syntax_error(&line, &e);
                self.status = 2;
                Step::Idle
            }
        }
    }
    fn expand_history(&self, line: &str) -> Option<String> {
        if line == "!!" {
            return self.history.last().cloned();
        }
        let prefix = line.strip_prefix('!')?;
        if prefix.is_empty() || prefix.contains(' ') || prefix.starts_with('=') {
            return None;
        }
        self.history
            .iter()
            .rev()
            .find(|h| h.starts_with(prefix))
            .cloned()
    }
    fn syntax_error(&mut self, line: &str, e: &parse::Error) {
        let column = line[..e.at.min(line.len())].chars().count();
        self.out_line(&format!(
            "Syntax error: {}\n  {line}\n  {}^",
            e.message,
            " ".repeat(column)
        ));
    }
    /// Run a script file (`.dsh`) with arguments, as `run` does.
    pub fn start_script(&mut self, host: &mut dyn Host, path: &str, args: &[String]) -> Step {
        self.frames.clear();
        match self.push_script(host, path, args, true) {
            Ok(()) => self.run(host),
            Err(f) => {
                self.report(&f);
                Step::Idle
            }
        }
    }
    pub(crate) fn push_script(
        &mut self,
        host: &mut dyn Host,
        path: &str,
        args: &[String],
        scoped: bool,
    ) -> Res<()> {
        let full = self.full(path)?;
        let native = host
            .native(&full)
            .ok_or_else(|| fail(format!("{path}: not a Doors path")))?;
        let bytes = host
            .read(&native, MAX_SCRIPT + 1)
            .map_err(|e| fail(format!("{path}: {}", e.message())))?;
        if bytes.len() > MAX_SCRIPT {
            return Err(fail(format!("{path}: scripts are limited to 512 KB")));
        }
        let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let stmts = parse::parse(text).map_err(|e| {
            let line = text[..e.at.min(text.len())].matches('\n').count() + 1;
            fail(format!("{path}, line {line}: {}", e.message))
        })?;
        if self
            .frames
            .iter()
            .filter(|f| matches!(f, Frame::Call { script: true, .. }))
            .count()
            >= 16
        {
            return Err(fail("scripts are nested too deeply"));
        }
        self.frames.push(Frame::Call {
            scoped,
            script: true,
        });
        if !scoped {
            self.set_var(
                "args",
                Value::List(args.iter().map(|a| Value::str(a)).collect()),
            );
        }
        if scoped {
            let mut scope = BTreeMap::new();
            scope.insert(
                String::from("args"),
                Value::List(args.iter().map(|a| Value::str(a)).collect()),
            );
            scope.insert(String::from("script"), Value::Str(full));
            self.scopes.push(scope);
        }
        self.frames.push(Frame::Block {
            stmts: Rc::new(stmts),
            pc: 0,
        });
        Ok(())
    }
    /// Continue after a yielded [`Step`].
    pub fn resume(&mut self, host: &mut dyn Host, r: Resume) -> Step {
        let Some(Frame::Pipe(pipe)) = self.frames.last_mut() else {
            return self.run(host);
        };
        let waiting = core::mem::replace(&mut pipe.waiting, Waiting::Nothing);
        match waiting {
            Waiting::Interop { capture } => {
                let r = match r {
                    Resume::Exited { code, output } => {
                        if capture {
                            self.captured.push_str(&String::from_utf8_lossy(&output));
                        }
                        Resume::Exited {
                            code,
                            output: if capture { Vec::new() } else { output },
                        }
                    }
                    other => other,
                };
                let step = self.interop.resume(host, r);
                let Some(Frame::Pipe(pipe)) = self.frames.pop() else {
                    return self.run(host);
                };
                match self.interop_step(pipe, step, capture, None) {
                    Ok(Some(step)) => return step,
                    Ok(None) => {}
                    Err(f) => {
                        if let Err(f) = self.catch(f, 0) {
                            self.report(&f);
                            self.frames.clear();
                            return Step::Idle;
                        }
                    }
                }
            }
            Waiting::Builtin(pending) => match builtins::resumed(self, host, pending, r) {
                Ok(v) => self.stage_done(v),
                Err(f) => {
                    if let Err(f) = self.catch(f, 0) {
                        self.report(&f);
                        self.frames.clear();
                        return Step::Idle;
                    }
                }
            },
            Waiting::Native { capture } => {
                let v = match r {
                    Resume::Exited { code, output } => {
                        self.status = code;
                        if capture {
                            Value::Str(String::from_utf8_lossy(&output).into_owned())
                        } else {
                            Value::Null
                        }
                    }
                    Resume::Failed(e) => {
                        self.report(&fail(e.message().trim_end()));
                        Value::Null
                    }
                    _ => Value::Null,
                };
                self.stage_done(v);
            }
            Waiting::Function | Waiting::Nothing => {}
        }
        self.run(host)
    }
    /// Drive the frames until they yield or finish.
    fn run(&mut self, host: &mut dyn Host) -> Step {
        match self.run_until(host, 0, true) {
            Ok(Some(step)) => step,
            Ok(None) => {
                if let Some(code) = self.exit.take() {
                    return Step::Exit(code);
                }
                Step::Idle
            }
            Err(f) => {
                self.report(&f);
                self.frames.clear();
                Step::Idle
            }
        }
    }
    pub(crate) fn report(&mut self, f: &Fail) {
        self.status = f.code;
        let message = f.message.trim_end();
        if !message.is_empty() {
            self.out_line(&format!("Error: {message}"));
        }
    }
    /// Run frames above `base`. `yields`: a program or prompt may suspend
    /// (top level); nested runs (`$( )`, conditions) refuse them.
    fn run_until(&mut self, host: &mut dyn Host, base: usize, yields: bool) -> Res<Option<Step>> {
        let mut budget = 1_000_000u32;
        while self.frames.len() > base {
            budget -= 1;
            if budget == 0 {
                return Err(fail(
                    "the script ran for too long without finishing (a loop that never ends?)",
                ));
            }
            if self.exit.is_some() {
                self.frames.truncate(base);
                return Ok(None);
            }
            match self.step(host, base) {
                Ok(None) => {}
                Ok(Some(step)) => {
                    if !yields {
                        return Err(fail(
                            "programs and prompts can't run inside an expression; run them as a command",
                        ));
                    }
                    return Ok(Some(step));
                }
                Err(f) => {
                    if !self.recoverable(&f) {
                        self.catch(f, base)?;
                    }
                }
            }
        }
        Ok(None)
    }
    /// A failure that only sets `$?`: before `&&` / `||` (the chain decides
    /// what runs next; the message is shown) and in an `if` / `while`
    /// condition (false). Anything else stops the line or script, so a
    /// failed `cd` never leaves the next command running in the wrong
    /// folder.
    fn recoverable(&mut self, f: &Fail) -> bool {
        let quiet = match self.frames.last() {
            Some(Frame::Chain { items, index }) if *index < items.len() => false,
            Some(Frame::If { checking: true, .. } | Frame::While { checking: true, .. }) => true,
            _ => return false,
        };
        if !quiet {
            let message = f.message.trim_end();
            self.out_line(&format!("Error: {message}"));
        }
        self.status = if f.code == 0 { 1 } else { f.code };
        true
    }
    /// An error unwinds to the nearest `try` above `base`.
    fn catch(&mut self, f: Fail, base: usize) -> Res<()> {
        while self.frames.len() > base {
            match self.frames.pop() {
                Some(Frame::Try { var, catch }) => {
                    self.status = f.code;
                    if let Some(var) = var {
                        self.set_var(&var, Value::Str(f.message.clone()));
                    }
                    self.frames.push(Frame::Block {
                        stmts: catch,
                        pc: 0,
                    });
                    return Ok(());
                }
                Some(Frame::Call { scoped: true, .. }) => {
                    self.scopes.pop();
                }
                _ => {}
            }
        }
        Err(f)
    }
    /// One unit of work on the top frame (the frame is taken off and put
    /// back when it continues).
    fn step(&mut self, host: &mut dyn Host, base: usize) -> Res<Option<Step>> {
        let Some(frame) = self.frames.pop() else {
            return Ok(None);
        };
        match frame {
            Frame::Block { stmts, pc } => {
                if pc >= stmts.len() {
                    return Ok(None);
                }
                let stmt = stmts[pc].clone();
                self.frames.push(Frame::Block { stmts, pc: pc + 1 });
                self.exec(host, stmt, base).map(|()| None)
            }
            Frame::Chain { items, index } => {
                if index >= items.len() {
                    return Ok(None);
                }
                let (joint, pipeline) = items[index].clone();
                self.frames.push(Frame::Chain {
                    items,
                    index: index + 1,
                });
                let run = match joint {
                    Joint::First => true,
                    Joint::And => self.status == 0,
                    Joint::Or => self.status != 0,
                };
                if run {
                    self.push_pipe(pipeline, Sink::Console);
                }
                Ok(None)
            }
            Frame::If {
                arms,
                otherwise,
                index,
                checking,
            } => {
                let decided = if checking {
                    self.result = None;
                    self.status == 0
                } else if index >= arms.len() {
                    if let Some(body) = otherwise {
                        self.frames.push(Frame::Block { stmts: body, pc: 0 });
                    }
                    return Ok(None);
                } else {
                    match &arms[index].0 {
                        Cond::Expr(e) => {
                            let e = e.clone();
                            self.eval(host, &e)?.truthy()
                        }
                        Cond::Run(p) => {
                            let p = p.clone();
                            self.frames.push(Frame::If {
                                arms,
                                otherwise,
                                index,
                                checking: true,
                            });
                            self.push_pipe(p, Sink::Keep);
                            return Ok(None);
                        }
                    }
                };
                if decided {
                    let body = Rc::new(arms[index].1.clone());
                    self.frames.push(Frame::Block { stmts: body, pc: 0 });
                } else {
                    self.frames.push(Frame::If {
                        arms,
                        otherwise,
                        index: index + 1,
                        checking: false,
                    });
                }
                Ok(None)
            }
            Frame::For {
                var,
                items,
                index,
                body,
            } => {
                if index >= items.len() {
                    return Ok(None);
                }
                self.set_var(&var, items[index].clone());
                self.frames.push(Frame::For {
                    var,
                    items,
                    index: index + 1,
                    body: body.clone(),
                });
                self.frames.push(Frame::Block { stmts: body, pc: 0 });
                Ok(None)
            }
            Frame::While {
                cond,
                body,
                forever,
                checking,
            } => {
                let go = if forever {
                    true
                } else if checking {
                    self.result = None;
                    self.status == 0
                } else {
                    match cond.as_ref() {
                        Cond::Expr(e) => {
                            let e = e.clone();
                            self.eval(host, &e)?.truthy()
                        }
                        Cond::Run(p) => {
                            let p = p.clone();
                            self.frames.push(Frame::While {
                                cond,
                                body,
                                forever,
                                checking: true,
                            });
                            self.push_pipe(p, Sink::Keep);
                            return Ok(None);
                        }
                    }
                };
                if go {
                    self.frames.push(Frame::While {
                        cond,
                        body: body.clone(),
                        forever,
                        checking: false,
                    });
                    self.frames.push(Frame::Block { stmts: body, pc: 0 });
                }
                Ok(None)
            }
            Frame::Call { scoped, .. } => {
                // The body ended without `return`.
                if scoped {
                    self.scopes.pop();
                }
                self.result.get_or_insert(Value::Null);
                Ok(None)
            }
            Frame::Try { .. } => Ok(None),
            Frame::Pipe(pipe) => self.pipe_step(host, pipe),
        }
    }
    fn exec(&mut self, host: &mut dyn Host, stmt: Stmt, base: usize) -> Res<()> {
        match stmt {
            Stmt::Chain(items) => {
                if items.len() == 1 {
                    self.push_pipe(items[0].1.clone(), Sink::Console);
                } else {
                    self.frames.push(Frame::Chain {
                        items: Rc::new(items),
                        index: 0,
                    });
                }
            }
            Stmt::Let(name, e) => {
                let v = self.eval(host, &e)?;
                self.set_var(&name, v);
                self.status = 0;
            }
            Stmt::SetEnv(name, e) => {
                let v = self.eval(host, &e)?;
                self.set_env(&name, &value::inline(&v));
                self.status = 0;
            }
            Stmt::If(arms, otherwise) => self.frames.push(Frame::If {
                arms: Rc::new(arms),
                otherwise: otherwise.map(Rc::new),
                index: 0,
                checking: false,
            }),
            Stmt::For(var, iter, body) => {
                let items = match iter {
                    Iter::Expr(e) => self.eval(host, &e)?.items(),
                    Iter::Run(p) => self.capture(host, p)?.items(),
                };
                if items.len() > 1_000_000 {
                    return Err(fail("too many items to loop over"));
                }
                self.frames.push(Frame::For {
                    var,
                    items,
                    index: 0,
                    body: Rc::new(body),
                });
            }
            Stmt::While(cond, body) => self.frames.push(Frame::While {
                cond: Rc::new(cond),
                body: Rc::new(body),
                forever: false,
                checking: false,
            }),
            Stmt::Loop(body) => self.frames.push(Frame::While {
                cond: Rc::new(Cond::Expr(Expr::Lit(Value::Bool(true)))),
                body: Rc::new(body),
                forever: true,
                checking: false,
            }),
            Stmt::Fn(f) => {
                if builtins::is_builtin(&f.name) {
                    return Err(fail(format!(
                        "'{}' is a Doorsh command; choose another name",
                        f.name
                    )));
                }
                self.functions.insert(f.name.clone(), Rc::new(f));
                self.status = 0;
            }
            Stmt::Return(e) => {
                let v = match e {
                    Some(e) => self.eval(host, &e)?,
                    None => Value::Null,
                };
                self.unwind(Unwind::Return(v), base)?;
            }
            Stmt::Break => self.unwind(Unwind::Break, base)?,
            Stmt::Continue => self.unwind(Unwind::Continue, base)?,
            Stmt::Try(body, var, catch) => {
                self.frames.push(Frame::Try {
                    var,
                    catch: Rc::new(catch),
                });
                self.frames.push(Frame::Block {
                    stmts: Rc::new(body),
                    pc: 0,
                });
            }
        }
        Ok(())
    }
    fn unwind(&mut self, how: Unwind, base: usize) -> Res<()> {
        while self.frames.len() > base {
            let frame = self.frames.pop().unwrap();
            match (&how, frame) {
                (Unwind::Break, Frame::For { .. } | Frame::While { .. }) => return Ok(()),
                (Unwind::Continue, f @ (Frame::For { .. } | Frame::While { .. })) => {
                    // The loop continues with its next round.
                    self.frames.push(f);
                    return Ok(());
                }
                (Unwind::Return(v), Frame::Call { scoped, .. }) => {
                    if scoped {
                        self.scopes.pop();
                    }
                    self.result = Some(v.clone());
                    return Ok(());
                }
                (_, Frame::Call { scoped: true, .. }) => {
                    self.scopes.pop();
                }
                _ => {}
            }
        }
        match how {
            // `return` at the prompt ends the line.
            Unwind::Return(_) => Ok(()),
            Unwind::Break => Err(fail("'break' outside a loop")),
            Unwind::Continue => Err(fail("'continue' outside a loop")),
        }
    }
    fn push_pipe(&mut self, p: Pipeline, sink: Sink) {
        self.frames.push(Frame::Pipe(Pipe {
            commands: Rc::new(p.commands),
            index: 0,
            value: Value::Null,
            sink,
            waiting: Waiting::Nothing,
        }));
    }
    /// A pipeline's value, run to completion now (`$( )`, `for x in cmd`).
    pub(crate) fn capture(&mut self, host: &mut dyn Host, p: Pipeline) -> Res<Value> {
        let base = self.frames.len();
        let saved = self.result.take();
        self.push_pipe(p, Sink::Keep);
        let r = self.run_until(host, base, false);
        let v = self.result.take().unwrap_or(Value::Null);
        self.result = saved;
        r?;
        Ok(v)
    }
    /// The top pipe finished its current stage with `v`.
    fn stage_done(&mut self, v: Value) {
        if let Some(Frame::Pipe(pipe)) = self.frames.last_mut() {
            pipe.value = v;
            pipe.index += 1;
            pipe.waiting = Waiting::Nothing;
        }
    }
    fn pipe_step(&mut self, host: &mut dyn Host, mut pipe: Pipe) -> Res<Option<Step>> {
        if pipe.waiting == Waiting::Function {
            pipe.value = self.result.take().unwrap_or(Value::Null);
            pipe.index += 1;
            pipe.waiting = Waiting::Nothing;
        }
        if pipe.index >= pipe.commands.len() {
            self.finish(host, pipe)?;
            return Ok(None);
        }
        let command = pipe.commands[pipe.index].clone();
        // `< file` feeds the first stage.
        if pipe.index == 0 {
            for r in &command.redirects {
                if r.kind == RedirectKind::In {
                    let name = self.word_text(host, &r.target)?;
                    pipe.value = Value::Str(self.read_text(host, &name)?);
                }
            }
        }
        let mut args = Vec::new();
        for w in &command.words {
            args.extend(self.expand(host, w)?);
        }
        let Some(head) = args.first().map(value::inline) else {
            return Err(fail("empty command"));
        };
        let rest: Vec<Value> = args[1..].to_vec();
        if let Some(last) = rest.last() {
            self.last_arg = value::inline(last);
        }
        let input = core::mem::replace(&mut pipe.value, Value::Null);
        let last = pipe.index + 1 == pipe.commands.len();
        let redirected = command.redirects.iter().any(|r| {
            matches!(
                r.kind,
                RedirectKind::Out | RedirectKind::Append | RedirectKind::Both
            )
        });
        // A function.
        if let Some(f) = self.functions.get(&head).cloned() {
            let mut scope = BTreeMap::new();
            let mut positional = rest.iter();
            for (name, default) in &f.params {
                let v = match positional.next() {
                    Some(v) => v.clone(),
                    None => match default {
                        Some(e) => self.eval(host, e)?,
                        None => return Err(fail(format!("{}: missing argument '{name}'", f.name))),
                    },
                };
                scope.insert(name.clone(), v);
            }
            scope.insert(String::from("in"), input);
            scope.insert(String::from("args"), Value::List(rest.clone()));
            if self.scopes.len() > 200 {
                return Err(fail(format!("{}: too much recursion", f.name)));
            }
            pipe.waiting = Waiting::Function;
            self.frames.push(Frame::Pipe(pipe));
            self.frames.push(Frame::Call {
                scoped: true,
                script: false,
            });
            self.scopes.push(scope);
            self.result = None;
            self.frames.push(Frame::Block {
                stmts: Rc::new(f.body.clone()),
                pc: 0,
            });
            return Ok(None);
        }
        // An alias: its words replace the name (not when it names itself).
        if let Some(text) = self.aliases.get(&head).cloned()
            && text.split_whitespace().next() != Some(head.as_str())
        {
            let parsed =
                parse::parse(&text).map_err(|e| fail(format!("alias {head}: {}", e.message)))?;
            if let Some(Stmt::Chain(items)) = parsed.into_iter().next()
                && let Some((_, p)) = items.into_iter().next()
                && let Some(mut first) = p.commands.into_iter().next()
            {
                first.words.extend(command.words.into_iter().skip(1));
                first.redirects.extend(command.redirects);
                let mut commands = (*pipe.commands).clone();
                commands[pipe.index] = first;
                pipe.commands = Rc::new(commands);
                pipe.value = input;
                self.frames.push(Frame::Pipe(pipe));
                return Ok(None);
            }
        }
        if builtins::is_builtin(&head) {
            let outcome = builtins::run(self, host, &head, &rest, input)?;
            return match outcome {
                builtins::Outcome::Value(v) => {
                    pipe.value = v;
                    pipe.index += 1;
                    self.frames.push(Frame::Pipe(pipe));
                    Ok(None)
                }
                builtins::Outcome::Yield(step, pending) => {
                    pipe.waiting = Waiting::Builtin(pending);
                    self.frames.push(Frame::Pipe(pipe));
                    Ok(Some(step))
                }
                builtins::Outcome::Script(path, args, scoped) => {
                    pipe.waiting = Waiting::Function;
                    self.frames.push(Frame::Pipe(pipe));
                    self.result = None;
                    self.push_script(host, &path, &args, scoped)?;
                    Ok(None)
                }
                builtins::Outcome::Exit(code) => {
                    self.exit = Some(code);
                    Ok(None)
                }
            };
        }
        // A Doors program (/system/bin/<name>).
        let native = format!("/system/bin/{head}");
        if !head.contains(['\\', '/', ':']) && host.stat(&native).is_ok_and(|m| !m.dir) {
            let capture = !last || redirected || pipe.sink == Sink::Keep;
            let mut argv = alloc::vec![head.clone()];
            argv.extend(rest.iter().map(value::inline));
            let spawn = crate::Spawn {
                program: native,
                command_line: argv.join(" "),
                argv,
                cwd: host.native(&self.cwd).unwrap_or_default(),
                environment: self.env.iter().map(|(k, v)| format!("{k}={v}")).collect(),
                input: (pipe.index > 0).then(|| input.to_text().into_bytes()),
                capture,
                detached: false,
            };
            pipe.waiting = Waiting::Native { capture };
            self.frames.push(Frame::Pipe(pipe));
            return Ok(Some(Step::Spawn(spawn)));
        }
        // Everything else: cmd.exe's engine (System32 tools, programs,
        // batch files, legacy Windows names).
        let capture = !last || redirected || pipe.sink == Sink::Keep;
        let mut line = quote_word(&head);
        for a in &rest {
            line.push(' ');
            line.push_str(&quote_word(&value::inline(a)));
        }
        self.interop.set_cwd(&self.cwd);
        self.captured.clear();
        let piped = (pipe.index > 0).then(|| input.to_text());
        let step = self.interop.submit(host, &line);
        if matches!(step, Step::Idle) && self.interop.errorlevel() == 9009 && !capture {
            // Not a program either: Doorsh suggests a near command.
            let _ = self.interop.take_events();
            return Err(Fail {
                message: self.unknown(&head),
                code: 9009,
            });
        }
        pipe.value = Value::Null;
        self.interop_step(pipe, step, capture, piped)
    }
    fn unknown(&self, name: &str) -> String {
        let names = self.command_names();
        let best = names
            .iter()
            .map(|n| (help::distance(n, name), n))
            .filter(|(d, _)| *d <= 2)
            .min_by_key(|(d, _)| *d);
        match best {
            Some((_, n)) => {
                format!("'{name}' is not a command, program or script. Did you mean '{n}'?")
            }
            None => format!(
                "'{name}' is not a command, program or script. Type 'help' for the list of commands."
            ),
        }
    }
    /// Handle what the cmd engine asked for.
    fn interop_step(
        &mut self,
        mut pipe: Pipe,
        step: Step,
        capture: bool,
        piped: Option<String>,
    ) -> Res<Option<Step>> {
        for e in self.interop.take_events() {
            match e {
                Event::Text(t) if capture => self.captured.push_str(&t),
                Event::Text(t) => self.out(&t),
                Event::Clear => self.clear(),
                Event::Title(_) | Event::Color(_) => {}
            }
        }
        match step {
            Step::Idle | Step::Exit(_) => {
                self.status = self.interop.errorlevel();
                pipe.value = if capture {
                    Value::Str(core::mem::take(&mut self.captured))
                } else {
                    Value::Null
                };
                pipe.index += 1;
                self.cwd = String::from(self.interop.cwd());
                self.frames.push(Frame::Pipe(pipe));
                Ok(None)
            }
            Step::Spawn(mut spawn) => {
                spawn.capture = spawn.capture || capture;
                if let Some(text) = piped {
                    spawn.input = Some(text.into_bytes());
                }
                pipe.waiting = Waiting::Interop { capture };
                self.frames.push(Frame::Pipe(pipe));
                Ok(Some(Step::Spawn(spawn)))
            }
            other => {
                pipe.waiting = Waiting::Interop { capture };
                self.frames.push(Frame::Pipe(pipe));
                Ok(Some(other))
            }
        }
    }
    fn finish(&mut self, host: &mut dyn Host, pipe: Pipe) -> Res<()> {
        let v = pipe.value;
        if pipe.sink == Sink::Keep {
            self.result = Some(v);
            return Ok(());
        }
        let last = pipe.commands.last().cloned().unwrap_or_default();
        for r in &last.redirects {
            if matches!(
                r.kind,
                RedirectKind::Out | RedirectKind::Append | RedirectKind::Both
            ) {
                let name = self.word_text(host, &r.target)?;
                let text = value::file_text(&v);
                self.write_text(host, &name, &text, r.kind == RedirectKind::Append)?;
                return Ok(());
            }
        }
        if v != Value::Null {
            let text = value::render(&v, self.width);
            self.out(&text);
        }
        Ok(())
    }
    // ---- words and expressions ----
    /// A word's values: one typed value for `$x`, `(expr)` and `$(cmd)`;
    /// text otherwise, which globs and braces may turn into several.
    fn expand(&mut self, host: &mut dyn Host, w: &Word) -> Res<Vec<Value>> {
        if let [single] = w.parts.as_slice() {
            match single {
                Part::Var(name, path) => return Ok(alloc::vec![self.var_path(name, path)?]),
                Part::Sub(p) => return Ok(alloc::vec![self.capture(host, p.clone())?]),
                Part::Expr(e) => return Ok(alloc::vec![self.eval(host, e)?]),
                Part::Quoted(s) => return Ok(alloc::vec![Value::Str(s.clone())]),
                Part::Bare(_) => {}
            }
        }
        let mut text = String::new();
        let mut globbing = false;
        let all_bare = w.parts.iter().all(|p| matches!(p, Part::Bare(_)));
        for (i, part) in w.parts.iter().enumerate() {
            match part {
                Part::Bare(s) => {
                    let s = if i == 0 && (s == "~" || s.starts_with("~\\") || s.starts_with("~/")) {
                        format!("{}{}", self.home, &s[1..])
                    } else {
                        s.clone()
                    };
                    globbing |= s.contains(['*', '?']);
                    text.push_str(&s);
                }
                Part::Quoted(s) => text.push_str(s),
                Part::Var(name, path) => text.push_str(&value::inline(&self.var_path(name, path)?)),
                Part::Sub(p) => {
                    text.push_str(value::inline(&self.capture(host, p.clone())?).trim_end())
                }
                Part::Expr(e) => text.push_str(&value::inline(&self.eval(host, e)?)),
            }
        }
        let mut out = Vec::new();
        let words = if all_bare {
            braces(&text)
        } else {
            alloc::vec![text]
        };
        for t in words {
            if globbing && t.contains(['*', '?']) {
                let matches = self.glob(host, &t)?;
                if matches.is_empty() {
                    out.push(Value::Str(t));
                } else {
                    out.extend(matches.into_iter().map(Value::Str));
                }
            } else if all_bare
                && t.chars().next().is_some_and(|c| {
                    c.is_ascii_digit()
                        || (c == '-' && t.len() > 1 && t.as_bytes()[1].is_ascii_digit())
                })
            {
                out.push(value::parse_number(&t).unwrap_or(Value::Str(t)));
            } else {
                out.push(Value::Str(t));
            }
        }
        Ok(out)
    }
    fn word_text(&mut self, host: &mut dyn Host, w: &Word) -> Res<String> {
        let values = self.expand(host, w)?;
        Ok(values
            .iter()
            .map(value::inline)
            .collect::<Vec<_>>()
            .join(" "))
    }
    /// Matching names for a pattern (`*.txt`, `src\*.rs`, `**\*.dsh`).
    pub(crate) fn glob(&mut self, host: &mut dyn Host, pattern: &str) -> Res<Vec<String>> {
        let pattern = pattern.replace('/', "\\");
        let (dir, name) = match pattern.rfind('\\') {
            Some(i) => (String::from(&pattern[..i]), String::from(&pattern[i + 1..])),
            None => (String::new(), pattern.clone()),
        };
        let recursive = dir == "**" || dir.ends_with("\\**");
        let base = if recursive {
            String::from(dir.trim_end_matches("**").trim_end_matches('\\'))
        } else {
            dir
        };
        let full = self.full(if base.is_empty() { "." } else { &base })?;
        let mut out = Vec::new();
        let mut stack = alloc::vec![(full, base)];
        let mut visited = 0;
        while let Some((d, shown)) = stack.pop() {
            let Some(native) = host.native(&d) else {
                continue;
            };
            let Ok(mut entries) = host.list(&native) else {
                continue;
            };
            entries.sort_by_key(|e| e.name.to_lowercase());
            for e in entries {
                visited += 1;
                if visited > 20_000 {
                    return Err(fail("the pattern matches too many files; narrow it"));
                }
                let display = if shown.is_empty() {
                    e.name.clone()
                } else {
                    format!("{shown}\\{}", e.name)
                };
                if crate::wild::matches(&name, &e.name) {
                    out.push(display.clone());
                }
                if recursive && e.meta.dir {
                    stack.push((crate::wpath::join(&d, &e.name), display));
                }
            }
        }
        out.sort_by_key(|s| s.to_lowercase());
        Ok(out)
    }
    pub(crate) fn var_path(&self, name: &str, path: &[String]) -> Res<Value> {
        let mut v = self.var(name)?;
        for p in path {
            v = index(&v, &Value::str(p))?;
        }
        Ok(v)
    }
    pub(crate) fn var(&self, name: &str) -> Res<Value> {
        match name {
            "?" => return Ok(Value::Int(i64::from(self.status))),
            "_" => return Ok(Value::Str(self.last_arg.clone())),
            "env" => {
                return Ok(Value::Record(
                    self.env
                        .iter()
                        .map(|(k, v)| (k.clone(), Value::str(v)))
                        .collect(),
                ));
            }
            "HOME" => return Ok(Value::Str(self.home.clone())),
            "PWD" => return Ok(Value::Str(self.cwd.clone())),
            _ => {}
        }
        // The innermost scope, then the script's and the session's.
        if let Some(v) = self.scopes.iter().rev().find_map(|s| s.get(name)) {
            return Ok(v.clone());
        }
        if let Some((_, v)) = self.env.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
            return Ok(Value::str(v));
        }
        Err(fail(format!("${name} is not set (use 'let {name} = ...')")))
    }
    pub(crate) fn set_var(&mut self, name: &str, v: Value) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(String::from(name), v);
        }
    }
    pub(crate) fn unset_var(&mut self, name: &str) -> bool {
        self.scopes
            .iter_mut()
            .rev()
            .any(|s| s.remove(name).is_some())
    }
    pub(crate) fn set_env(&mut self, name: &str, value: &str) {
        self.env.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
        if !value.is_empty() {
            self.env.push((String::from(name), String::from(value)));
        }
        self.interop.set_env(name, value);
    }
    pub(crate) fn variables(&self) -> Vec<(String, Value)> {
        let mut out: Vec<(String, Value)> = Vec::new();
        for scope in &self.scopes {
            for (k, v) in scope {
                out.retain(|(n, _)| n != k);
                out.push((k.clone(), v.clone()));
            }
        }
        out
    }
    pub(crate) fn function_list(&self) -> Vec<Rc<Function>> {
        self.functions.values().cloned().collect()
    }
    pub(crate) fn has_function(&self, name: &str) -> bool {
        self.functions.contains_key(name)
    }
    pub(crate) fn remove_function(&mut self, name: &str) -> bool {
        self.functions.remove(name).is_some()
    }
    pub(crate) fn eval(&mut self, host: &mut dyn Host, e: &Expr) -> Res<Value> {
        Ok(match e {
            Expr::Lit(v) => v.clone(),
            Expr::Interp(parts) => {
                let mut s = String::new();
                for p in parts {
                    match p {
                        Part::Quoted(t) | Part::Bare(t) => s.push_str(t),
                        Part::Var(name, path) => {
                            s.push_str(&value::inline(&self.var_path(name, path)?))
                        }
                        Part::Sub(p) => {
                            s.push_str(value::inline(&self.capture(host, p.clone())?).trim_end())
                        }
                        Part::Expr(e) => s.push_str(&value::inline(&self.eval(host, e)?)),
                    }
                }
                Value::Str(s)
            }
            Expr::Var(name) => self.var(name)?,
            Expr::Sub(p) => self.capture(host, p.clone())?,
            Expr::List(items) => Value::List(
                items
                    .iter()
                    .map(|i| self.eval(host, i))
                    .collect::<Res<_>>()?,
            ),
            Expr::Record(fields) => Value::Record(
                fields
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), self.eval(host, v)?)))
                    .collect::<Res<_>>()?,
            ),
            Expr::Unary('!', e) => Value::Bool(!self.eval(host, e)?.truthy()),
            Expr::Unary(_, e) => match self.eval(host, e)? {
                Value::Int(i) => {
                    Value::Int(i.checked_neg().ok_or_else(|| fail("number too large"))?)
                }
                Value::Float(f) => Value::Float(-f),
                other => return Err(fail(format!("can't negate a {}", other.type_name()))),
            },
            Expr::Binary(a, Op::And, b) => {
                let left = self.eval(host, a)?;
                Value::Bool(left.truthy() && self.eval(host, b)?.truthy())
            }
            Expr::Binary(a, Op::Or, b) => {
                let left = self.eval(host, a)?;
                Value::Bool(left.truthy() || self.eval(host, b)?.truthy())
            }
            Expr::Binary(a, op, b) => {
                let (l, r) = (self.eval(host, a)?, self.eval(host, b)?);
                binary(&l, *op, &r)?
            }
            Expr::Field(e, name) => {
                let v = self.eval(host, e)?;
                index(&v, &Value::Str(name.clone()))?
            }
            Expr::Index(e, i) => {
                let (v, i) = (self.eval(host, e)?, self.eval(host, i)?);
                index(&v, &i)?
            }
            Expr::Range(a, b) => {
                let (a, b) = (self.eval(host, a)?, self.eval(host, b)?);
                range(&a, &b)?
            }
        })
    }
    /// An expression evaluated against a table row (`where`, `sort-by`):
    /// bare words naming a field are that field.
    pub(crate) fn eval_row(&mut self, host: &mut dyn Host, e: &Expr, row: &Value) -> Res<Value> {
        match e {
            Expr::Lit(Value::Str(s)) => Ok(row
                .field(s)
                .cloned()
                .unwrap_or_else(|| Value::Str(s.clone()))),
            Expr::Binary(a, Op::And, b) => Ok(Value::Bool(
                self.eval_row(host, a, row)?.truthy() && self.eval_row(host, b, row)?.truthy(),
            )),
            Expr::Binary(a, Op::Or, b) => Ok(Value::Bool(
                self.eval_row(host, a, row)?.truthy() || self.eval_row(host, b, row)?.truthy(),
            )),
            Expr::Binary(a, op, b) => {
                let l = self.eval_row(host, a, row)?;
                // The right side is a literal unless it names a field.
                let r = match b.as_ref() {
                    Expr::Lit(Value::Str(s))
                        if row.field(s).is_some()
                            && !matches!(a.as_ref(), Expr::Lit(Value::Str(_))) =>
                    {
                        row.field(s).cloned().unwrap()
                    }
                    Expr::Lit(v) => v.clone(),
                    other => self.eval_row(host, other, row)?,
                };
                binary(&l, *op, &r)
            }
            Expr::Unary('!', inner) => Ok(Value::Bool(!self.eval_row(host, inner, row)?.truthy())),
            Expr::Var(name) if name == "it" => Ok(row.clone()),
            Expr::Field(inner, name) => {
                let v = self.eval_row(host, inner, row)?;
                index(&v, &Value::Str(name.clone()))
            }
            other => {
                self.set_var("it", row.clone());
                self.eval(host, other)
            }
        }
    }
    // ---- paths and files ----
    /// A full Windows path for a typed one (`~`, `/` separators and
    /// relative names allowed).
    pub(crate) fn full(&self, path: &str) -> Res<String> {
        let path = path.replace('/', "\\");
        let path = if path == "~" || path.starts_with("~\\") {
            format!("{}{}", self.home, &path[1..])
        } else {
            path
        };
        crate::wpath::full(&self.cwd, &|_| None, &path)
            .map_err(|e| fail(format!("{path}: {}", e.message().trim_end())))
    }
    pub(crate) fn native(&self, host: &dyn Host, path: &str) -> Res<(String, String)> {
        let full = self.full(path)?;
        let native = host
            .native(&full)
            .ok_or_else(|| fail(format!("{path}: not on a Doors drive")))?;
        Ok((full, native))
    }
    pub(crate) fn read_text(&self, host: &mut dyn Host, path: &str) -> Res<String> {
        let (_, native) = self.native(host, path)?;
        let bytes = host
            .read(&native, 16 * 1024 * 1024)
            .map_err(|e| fail(format!("{path}: {}", e.message().trim_end())))?;
        Ok(String::from_utf8_lossy(&bytes).replace("\r\n", "\n"))
    }
    pub(crate) fn write_text(
        &self,
        host: &mut dyn Host,
        path: &str,
        text: &str,
        append: bool,
    ) -> Res<()> {
        let (_, native) = self.native(host, path)?;
        host.write(&native, text.as_bytes(), append)
            .map_err(|e| fail(format!("{path}: {}", e.message().trim_end())))
    }
    /// Command names Doorsh can complete or suggest.
    pub fn command_names(&self) -> Vec<String> {
        let mut names: Vec<String> = builtins::NAMES.iter().map(|n| String::from(*n)).collect();
        names.extend(self.functions.keys().cloned());
        names.extend(self.aliases.keys().cloned());
        names.sort();
        names.dedup();
        names
    }
    /// Tab completion for the last word of `line`: commands for the first
    /// word, file and folder names after it, variables after `$`.
    pub fn complete(&self, host: &mut dyn Host, line: &str) -> Vec<String> {
        let start = line.rfind([' ', '|', '(']).map_or(0, |i| i + 1);
        let word = &line[start..];
        let before = line[..start].trim_end();
        let first = before.is_empty() || before.ends_with('|') || before.ends_with('(');
        if let Some(v) = word.strip_prefix('$') {
            let mut out: Vec<String> = self
                .variables()
                .into_iter()
                .map(|(k, _)| k)
                .filter(|k| k.starts_with(v))
                .map(|k| format!("${k}"))
                .collect();
            out.sort();
            return out;
        }
        if first && !word.contains(['\\', '/', '.']) {
            return self
                .command_names()
                .into_iter()
                .filter(|n| n.starts_with(word))
                .collect();
        }
        let (dir, prefix) = match word.rfind(['\\', '/']) {
            Some(i) => (&word[..=i], &word[i + 1..]),
            None => ("", word),
        };
        let Ok((_, native)) = self.native(host, if dir.is_empty() { "." } else { dir }) else {
            return Vec::new();
        };
        let Ok(entries) = host.list(&native) else {
            return Vec::new();
        };
        let lower = prefix.to_lowercase();
        let mut out: Vec<String> = entries
            .into_iter()
            .filter(|e| e.name.to_lowercase().starts_with(&lower))
            .map(|e| {
                let mut s = format!("{dir}{}", e.name);
                if e.meta.dir {
                    s.push('\\');
                }
                if s.contains(' ') {
                    s = format!("'{s}'");
                }
                s
            })
            .collect();
        out.sort_by_key(|s| s.to_lowercase());
        out
    }
}
/// Quote a word for cmd.exe when it holds spaces or specials.
fn quote_word(w: &str) -> String {
    if w.is_empty() || w.contains([' ', '\t', '&', '|', '<', '>', '^', '(', ')']) {
        format!("\"{}\"", w.replace('"', "\\\""))
    } else {
        String::from(w)
    }
}
/// `{a,b}` and `{1..3}` brace expansion of one word.
pub(crate) fn braces(text: &str) -> Vec<String> {
    let Some(open) = text.find('{') else {
        return alloc::vec![String::from(text)];
    };
    let Some(close) = text[open..].find('}').map(|i| open + i) else {
        return alloc::vec![String::from(text)];
    };
    let inner = &text[open + 1..close];
    let (pre, post) = (&text[..open], &text[close + 1..]);
    let options: Vec<String> = if let Some((a, b)) = inner.split_once("..") {
        match (a.parse::<i64>(), b.parse::<i64>()) {
            (Ok(a), Ok(b)) if (b - a).abs() <= 1000 => {
                let step = if b >= a { 1 } else { -1 };
                let mut v = Vec::new();
                let mut i = a;
                loop {
                    v.push(i.to_string());
                    if i == b {
                        break;
                    }
                    i += step;
                }
                v
            }
            _ => return alloc::vec![String::from(text)],
        }
    } else if inner.contains(',') {
        inner.split(',').map(String::from).collect()
    } else {
        return alloc::vec![String::from(text)];
    };
    let mut out = Vec::new();
    for o in options {
        for rest in braces(post) {
            out.push(format!("{pre}{o}{rest}"));
        }
    }
    out
}
pub(crate) fn range(a: &Value, b: &Value) -> Res<Value> {
    let (Some(a), Some(b)) = (a.as_int(), b.as_int()) else {
        return Err(fail("a range needs whole numbers: 1..10"));
    };
    if (b - a).unsigned_abs() > 1_000_000 {
        return Err(fail("the range is too large"));
    }
    let step = if b >= a { 1 } else { -1 };
    let mut out = Vec::new();
    let mut i = a;
    loop {
        out.push(Value::Int(i));
        if i == b {
            break;
        }
        i += step;
    }
    Ok(Value::List(out))
}
/// `v.key`, `v.0`, `v[i]`: a record's field, a list's item (negative from
/// the end), a table's column, a string's character, `.length`.
pub(crate) fn index(v: &Value, i: &Value) -> Res<Value> {
    if let Value::Str(key) = i {
        if matches!(key.as_str(), "length" | "len" | "count") && v.field(key).is_none() {
            return Ok(Value::Int(match v {
                Value::List(l) => l.len() as i64,
                Value::Str(s) => s.chars().count() as i64,
                Value::Record(r) => r.len() as i64,
                _ => 0,
            }));
        }
        if let Ok(n) = key.parse::<i64>() {
            return index(v, &Value::Int(n));
        }
        return match v {
            Value::Record(_) => v
                .field(key)
                .cloned()
                .ok_or_else(|| fail(format!("no field '{key}'"))),
            Value::List(rows) if v.is_table() => Ok(Value::List(
                rows.iter()
                    .map(|r| r.field(key).cloned().unwrap_or(Value::Null))
                    .collect(),
            )),
            _ => Err(fail(format!("a {} has no field '{key}'", v.type_name()))),
        };
    }
    let n = i
        .as_int()
        .ok_or_else(|| fail("an index must be a number"))?;
    let pick = |len: usize| -> Option<usize> {
        let at = if n < 0 { len as i64 + n } else { n };
        (0..len as i64).contains(&at).then_some(at as usize)
    };
    match v {
        Value::List(l) => pick(l.len())
            .map(|at| l[at].clone())
            .ok_or_else(|| fail(format!("index {n} is outside the list ({} items)", l.len()))),
        Value::Str(s) => {
            let chars: Vec<char> = s.chars().collect();
            pick(chars.len())
                .map(|at| Value::Str(chars[at].to_string()))
                .ok_or_else(|| fail(format!("index {n} is outside the text")))
        }
        Value::Record(r) => pick(r.len())
            .map(|at| r[at].1.clone())
            .ok_or_else(|| fail(format!("index {n} is outside the record"))),
        other => Err(fail(format!("can't index a {}", other.type_name()))),
    }
}
pub(crate) fn binary(l: &Value, op: Op, r: &Value) -> Res<Value> {
    use core::cmp::Ordering::*;
    let num_err = || {
        fail(format!(
            "can't apply that to a {} and a {}",
            l.type_name(),
            r.type_name()
        ))
    };
    Ok(match op {
        Op::Eq => Value::Bool(equal(l, r)),
        Op::Ne => Value::Bool(!equal(l, r)),
        Op::Lt => Value::Bool(value::compare(l, r) == Less),
        Op::Le => Value::Bool(value::compare(l, r) != Greater),
        Op::Gt => Value::Bool(value::compare(l, r) == Greater),
        Op::Ge => Value::Bool(value::compare(l, r) != Less),
        Op::In => Value::Bool(match r {
            Value::List(items) => items.iter().any(|i| equal(i, l)),
            Value::Str(s) => s.to_lowercase().contains(&value::inline(l).to_lowercase()),
            Value::Record(_) => r.field(&value::inline(l)).is_some(),
            _ => false,
        }),
        Op::Like => {
            let (text, pattern) = (value::inline(l), value::inline(r));
            Value::Bool(if pattern.contains(['*', '?']) {
                crate::wild::matches(&pattern, &text)
            } else {
                text.to_lowercase().contains(&pattern.to_lowercase())
            })
        }
        Op::Concat => match (l, r) {
            (Value::List(a), Value::List(b)) => Value::List(a.iter().chain(b).cloned().collect()),
            (Value::List(a), b) => Value::List(a.iter().cloned().chain([b.clone()]).collect()),
            _ => Value::Str(format!("{}{}", value::inline(l), value::inline(r))),
        },
        Op::Add => match (l, r) {
            (Value::Int(a), Value::Int(b)) => {
                Value::Int(a.checked_add(*b).ok_or_else(|| fail("number too large"))?)
            }
            (Value::Size(a), Value::Size(b)) => Value::Size(a.saturating_add(*b)),
            (Value::Duration(a), Value::Duration(b)) => Value::Duration(a.saturating_add(*b)),
            (Value::Str(a), b) => Value::Str(format!("{a}{}", value::inline(b))),
            (Value::List(a), Value::List(b)) => Value::List(a.iter().chain(b).cloned().collect()),
            _ => {
                Value::Float(l.as_float().ok_or_else(num_err)? + r.as_float().ok_or_else(num_err)?)
            }
        },
        Op::Sub => match (l, r) {
            (Value::Int(a), Value::Int(b)) => {
                Value::Int(a.checked_sub(*b).ok_or_else(|| fail("number too large"))?)
            }
            (Value::Size(a), Value::Size(b)) => Value::Size(a.saturating_sub(*b)),
            (Value::Duration(a), Value::Duration(b)) => Value::Duration(a.saturating_sub(*b)),
            _ => {
                Value::Float(l.as_float().ok_or_else(num_err)? - r.as_float().ok_or_else(num_err)?)
            }
        },
        Op::Mul => match (l, r) {
            (Value::Int(a), Value::Int(b)) => {
                Value::Int(a.checked_mul(*b).ok_or_else(|| fail("number too large"))?)
            }
            (Value::Size(a), Value::Int(b)) | (Value::Int(b), Value::Size(a)) if *b >= 0 => {
                Value::Size(a.saturating_mul(*b as u64))
            }
            (Value::Duration(a), Value::Int(b)) | (Value::Int(b), Value::Duration(a))
                if *b >= 0 =>
            {
                Value::Duration(a.saturating_mul(*b as u64))
            }
            (Value::Str(s), Value::Int(n)) if (0..=10_000).contains(n) => {
                Value::Str(s.repeat(*n as usize))
            }
            _ => {
                Value::Float(l.as_float().ok_or_else(num_err)? * r.as_float().ok_or_else(num_err)?)
            }
        },
        Op::Div => {
            let d = r.as_float().ok_or_else(num_err)?;
            if d == 0.0 {
                return Err(fail("division by zero"));
            }
            match (l, r) {
                (Value::Int(a), Value::Int(b)) if a % b == 0 => Value::Int(a / b),
                (Value::Size(a), Value::Int(b)) if *b > 0 => Value::Size(a / *b as u64),
                _ => Value::Float(l.as_float().ok_or_else(num_err)? / d),
            }
        }
        Op::Rem => match (l, r) {
            (Value::Int(_), Value::Int(0)) => return Err(fail("division by zero")),
            (Value::Int(a), Value::Int(b)) => Value::Int(a.rem_euclid(*b)),
            _ => {
                let d = r.as_float().ok_or_else(num_err)?;
                if d == 0.0 {
                    return Err(fail("division by zero"));
                }
                let a = l.as_float().ok_or_else(num_err)?;
                Value::Float(a - d * ((a / d) as i64) as f64)
            }
        },
        Op::Pow => match (l, r) {
            (Value::Int(a), Value::Int(b)) if (0..64).contains(b) => Value::Int(
                a.checked_pow(*b as u32)
                    .ok_or_else(|| fail("number too large"))?,
            ),
            _ => {
                let (a, b) = (
                    l.as_float().ok_or_else(num_err)?,
                    r.as_int().ok_or_else(num_err)?,
                );
                let mut out = 1.0;
                for _ in 0..b.unsigned_abs().min(1024) {
                    out *= a;
                }
                Value::Float(if b < 0 { 1.0 / out } else { out })
            }
        },
        Op::And | Op::Or => Value::Bool(if op == Op::And {
            l.truthy() && r.truthy()
        } else {
            l.truthy() || r.truthy()
        }),
    })
}
/// Equality: text exactly; numbers by value (a number written as text
/// equals the number); lists and records item by item.
pub(crate) fn equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Null, Value::Null) => true,
        (Value::Null, _) | (_, Value::Null) => false,
        (Value::List(x), Value::List(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| equal(a, b))
        }
        (Value::Record(_), Value::Record(_)) => a == b,
        (Value::Str(s), other) | (other, Value::Str(s)) => {
            *s == value::inline(other)
                || value::parse_number(s).is_some_and(|n| n.as_float() == other.as_float())
        }
        _ => a.as_float().is_some() && a.as_float() == b.as_float(),
    }
}

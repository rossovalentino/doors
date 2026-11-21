//! Terminal tabs running a shell engine: cmd.exe (`door_shells::cmd`) on
//! Command Prompt tabs and Doorsh (`door_shells::doorsh`) on Doorsh tabs.
//! Each tab owns an interpreter; Enter submits a line, and the engine's steps are carried
//! out here: programs start through Init EXECUTE (output mirrored into the
//! tab, or captured for pipes and redirections), Doors apps and Settings
//! pages open through the window server, keys and lines answer prompts, and
//! timed waits resume from `tick`.
use super::*;
use door_shells::cmd::{Cmd, Event};
use door_shells::doorsh::Doorsh;
use door_shells::host::Host as _;
use door_shells::{InputKind, Launch, Resume, Step};

pub(super) enum Wait {
    Ready,
    Key,
    Line,
    Sleep {
        until: u64,
        keys: bool,
    },
    Program,
    /// A launch for `window_update`; then resume with its outcome.
    Launch(Option<bool>),
}
/// A tab's interpreter.
pub(super) enum Engine {
    Cmd(alloc::boxed::Box<Cmd>),
    Doorsh(alloc::boxed::Box<Doorsh>),
}
impl Engine {
    pub fn submit(&mut self, host: &mut dyn door_shells::host::Host, line: &str) -> Step {
        match self {
            Self::Cmd(c) => c.submit(host, line),
            Self::Doorsh(d) => d.submit(host, line),
        }
    }
    pub fn resume(&mut self, host: &mut dyn door_shells::host::Host, r: Resume) -> Step {
        match self {
            Self::Cmd(c) => c.resume(host, r),
            Self::Doorsh(d) => d.resume(host, r),
        }
    }
    pub fn interrupt(&mut self, host: &mut dyn door_shells::host::Host) -> Step {
        match self {
            Self::Cmd(c) => c.interrupt(host),
            Self::Doorsh(d) => d.interrupt(),
        }
    }
    pub fn take_events(&mut self) -> Vec<Event> {
        match self {
            Self::Cmd(c) => c.take_events(),
            Self::Doorsh(d) => d.take_events(),
        }
    }
    pub fn errorlevel(&self) -> i32 {
        match self {
            Self::Cmd(c) => c.errorlevel(),
            Self::Doorsh(d) => d.status,
        }
    }
    /// cmd.exe prints a blank line after a prompt command's output.
    pub fn echo_on(&self) -> bool {
        match self {
            Self::Cmd(c) => c.echo_on(),
            Self::Doorsh(_) => false,
        }
    }
    pub fn prompt(&self, host: &dyn door_shells::host::Host) -> String {
        match self {
            Self::Cmd(c) => c.prompt(host),
            Self::Doorsh(d) => d.prompt(),
        }
    }
    pub fn is_doorsh(&self) -> bool {
        matches!(self, Self::Doorsh(_))
    }
}
pub(super) struct Session {
    pub tab: u64,
    pub engine: Engine,
    pub wait: Wait,
    /// The line came from the prompt (a blank line follows its output).
    pub interactive: bool,
    /// Ctrl+C arrived while a program ran.
    pub interrupted: bool,
    /// The line being run and the bytes it printed (Doorsh's completion
    /// diagnostic for guest scenarios).
    pub line: String,
    pub bytes: usize,
}
impl Terminal {
    pub(super) fn host<'a>(&self, storage: &'a mut Storage) -> host::SessionHost<'a> {
        host::SessionHost {
            storage,
            registry: self.registry,
            uid: self.uid,
            administrator: self.administrator,
            user: self.user.clone(),
            computer: self.computer.clone(),
            offset: self.utc_offset,
            clipboard: None,
            processors: 1,
            elevated: self
                .state
                .tabs
                .get(self.state.active)
                .is_some_and(|t| t.elevated),
            directory: self.directory,
        }
    }
    fn session(&self, tab: u64) -> Option<usize> {
        self.sessions.iter().position(|s| s.tab == tab)
    }
    pub(super) fn tab_index(&self, id: u64) -> Option<usize> {
        self.state.tabs.iter().position(|t| t.id == id)
    }
    /// The interpreter of a CMD tab, created on first use in the tab's
    /// directory.
    fn session_for(&mut self, index: usize, storage: &mut Storage) -> usize {
        let id = self.state.tabs[index].id;
        if let Some(s) = self.session(id) {
            return s;
        }
        let profile_dir = format!("C:\\Users\\{}", self.user);
        let mut env =
            door_shells::cmd::default_environment(&self.user, &self.computer, &profile_dir, 1);
        for pair in &self.environment {
            if let Some((n, v)) = pair.split_once('=')
                && !n.is_empty()
                && !v.starts_with('/')
            {
                env.set(n, v);
            }
        }
        let cwd = {
            let host = self.host(storage);
            let native = self.state.tabs[index].cwd.clone();
            let w = host.windows(&native);
            if w.starts_with("C:") {
                w
            } else {
                profile_dir.clone()
            }
        };
        let engine = if self.state.tabs[index].profile == Profile::Native {
            let mut sh = Doorsh::new(env, &cwd, &profile_dir);
            sh.width = self.columns().max(20);
            Engine::Doorsh(alloc::boxed::Box::new(sh))
        } else {
            let mut cmd = alloc::boxed::Box::new(Cmd::new(env, &cwd));
            cmd.set_size(self.columns().max(20), self.grid().rows.max(5));
            Engine::Cmd(cmd)
        };
        let prompt = engine.prompt(&self.host(storage));
        self.state.tabs[index].prompt = Some(prompt);
        self.sessions.push(Session {
            tab: id,
            engine,
            wait: Wait::Ready,
            interactive: false,
            interrupted: false,
            line: String::new(),
            bytes: 0,
        });
        self.sessions.len() - 1
    }
    /// Prompt of a CMD tab before its first command.
    pub(super) fn prepare_cmd_tab(&mut self, storage: &mut Storage) {
        let index = self.state.active;
        let tab = &self.state.tabs[index];
        if tab.profile == Profile::Cmd && tab.prompt.is_none() && !tab.hosted {
            self.session_for(index, storage);
        }
    }
    /// Whether PowerShell 7 is installed (its program file is there).
    pub(super) fn refresh_powershell(&mut self, storage: &mut Storage) {
        let installed = storage.metadata(PWSH).is_ok_and(|i| !i.directory);
        Profile::set_powershell_installed(installed);
        self.powershell_checked = true;
    }
    /// A PowerShell tab not yet started runs pwsh.exe in its console grid
    /// (interactive, with its banner, in the tab's folder), as Windows
    /// Terminal's PowerShell profile starts it.
    pub(super) fn prepare_powershell_tab(&mut self) {
        let index = self.state.active;
        let tab = &self.state.tabs[index];
        if tab.profile != Profile::PowerShell
            || !Profile::PowerShell.available()
            || tab.hosted
            || self.running_in(tab.id)
            || self.registry == 0
        {
            return;
        }
        let cwd = tab.cwd.clone();
        let arguments = commands::argument_block(PWSH_WINDOWS, &[]);
        self.state.tabs[index].hosted = true;
        self.state.tabs[index].hide_input = true;
        if let Err(error) = self.start_program(index, PWSH, &cwd, &arguments, false, false) {
            self.state.tabs[index].output(&format!("[PowerShell could not start: {error:?}]"));
        }
    }
    /// A PowerShell script File Explorer opened (.ps1): a PowerShell tab
    /// runs it in its folder (`pwsh -NoExit -File`, as PowerShell 7's "Run
    /// with PowerShell 7") and stays at the prompt afterwards, replacing a
    /// fresh first tab when Terminal was just started for it.
    pub(super) fn run_powershell_script(&mut self, native: &str, storage: &mut Storage) {
        self.refresh_powershell(storage);
        if !Profile::PowerShell.available() {
            self.message =
                "PowerShell 7 is not installed. Install it to run PowerShell scripts.".into();
            return;
        }
        let fresh = self.state.tabs.len() == 1
            && self.state.tabs[0].profile == Profile::Native
            && self.state.tabs[0].history_is_empty()
            && !self.running_in(self.state.tabs[0].id);
        self.state.new_tab(Profile::PowerShell);
        if fresh {
            self.state.close(0);
        }
        let index = self.state.active;
        let parent = native
            .rsplit_once('/')
            .map_or("/", |(p, _)| if p.is_empty() { "/" } else { p });
        let windows = self.host(storage).windows(native);
        let tab = &mut self.state.tabs[index];
        tab.cwd = String::from(parent);
        tab.hosted = true;
        tab.hide_input = true;
        let arguments =
            commands::argument_block(PWSH_WINDOWS, &["-NoExit".into(), "-File".into(), windows]);
        if let Err(error) = self.start_program(index, PWSH, parent, &arguments, false, false) {
            self.state.tabs[index].output(&format!("[PowerShell could not start: {error:?}]"));
        }
        self.state.page = door_workspace::terminal_model::Page::Console;
    }
    /// A console program File Explorer (or Run) handed over, as Windows 11
    /// hands console programs to Windows Terminal: a tab of its own, titled
    /// with its name, running it in its folder; the tab closes with it.
    pub(super) fn run_hosted(&mut self, native: &str, storage: &mut Storage) {
        self.state.new_tab(Profile::Cmd);
        let index = self.state.active;
        let parent = native
            .rsplit_once('/')
            .map_or("/", |(p, _)| if p.is_empty() { "/" } else { p });
        let name = native.rsplit('/').next().unwrap_or(native);
        let windows = self.host(storage).windows(native);
        let tab = &mut self.state.tabs[index];
        tab.cwd = String::from(parent);
        tab.title = String::from(name.strip_suffix(".exe").unwrap_or(name));
        tab.hosted = true;
        tab.hide_input = true;
        let arguments = commands::argument_block(&windows, &[]);
        if let Err(error) = self.start_program(index, native, parent, &arguments, false, false) {
            self.state.tabs[index].output(&format!("[{name} could not start: {error:?}]"));
        }
    }
    /// Enter on a CMD tab.
    pub(super) fn cmd_submit(&mut self, line: &str, storage: &mut Storage) {
        let index = self.state.active;
        let s = self.session_for(index, storage);
        if !matches!(self.sessions[s].wait, Wait::Ready) {
            return;
        }
        let prompt = self.state.tabs[index].prompt.clone().unwrap_or_default();
        self.state.tabs[index].output(&format!("{prompt}{line}"));
        self.sessions[s].interactive = !line.trim().is_empty();
        self.sessions[s].line = String::from(line.split_whitespace().next().unwrap_or(""));
        self.sessions[s].bytes = 0;
        let step = {
            let mut host = self.host(storage);
            self.sessions[s].engine.submit(&mut host, line)
        };
        self.apply(s, step, storage);
    }
    /// Run a batch file in a new CMD tab (Explorer's Open on .bat/.cmd,
    /// `start x.bat`); `elevated` after User Account Control approved Run as
    /// administrator.
    pub fn run_batch(&mut self, native: &str, elevated: bool, storage: &mut Storage) {
        self.state.new_tab(Profile::Cmd);
        let index = self.state.active;
        if elevated {
            let tab = &mut self.state.tabs[index];
            tab.elevated = true;
            tab.title = alloc::format!("Administrator: {}", Profile::Cmd.name());
        }
        let parent = native
            .rsplit_once('/')
            .map_or("/", |(p, _)| if p.is_empty() { "/" } else { p });
        self.state.tabs[index].cwd = String::from(parent);
        let s = self.session_for(index, storage);
        let (windows, cwd) = {
            let host = self.host(storage);
            (host.windows(native), host.windows(parent))
        };
        let step = {
            let mut host = self.host(storage);
            match &mut self.sessions[s].engine {
                Engine::Cmd(cmd) => {
                    cmd.submit(&mut host, &format!("cd /d \"{cwd}\""));
                    cmd.start_batch(&mut host, &windows, &[])
                }
                Engine::Doorsh(sh) => sh.start_script(&mut host, &windows, &[]),
            }
        };
        self.apply(s, step, storage);
    }
    /// A document from the window server: run a batch file, or open a tab
    /// in a folder.
    pub fn open_path(&mut self, path: &str, storage: &mut Storage) {
        // "admin:<shell>": User Account Control approved an administrator
        // session of that shell.
        let (elevated, path) = match path.strip_prefix("admin:") {
            Some(rest) => (true, rest),
            None => (false, path),
        };
        let lower = path.to_ascii_lowercase();
        // "run:<path>": a console program File Explorer hands over.
        if let Some(native) = path.strip_prefix("run:") {
            self.run_hosted(native, storage);
            self.state.page = door_workspace::terminal_model::Page::Console;
            return;
        }
        if lower == "powershell" {
            self.refresh_powershell(storage);
        }
        let profile = match lower.as_str() {
            "cmd" => Some(Profile::Cmd),
            "doorsh" => Some(Profile::Native),
            "powershell" => Some(Profile::PowerShell),
            "dsl" => Some(Profile::Dsl),
            _ => None,
        };
        if let Some(profile) = profile {
            // Search or Run named a shell: its tab, replacing a fresh first
            // Doorsh tab when Terminal was just started for it.
            let fresh = self.state.tabs.len() == 1
                && self.state.tabs[0].profile == Profile::Native
                && self.state.tabs[0].history_is_empty();
            if profile == Profile::Native && fresh && (!elevated || self.state.elevated) {
                self.state.page = door_workspace::terminal_model::Page::Console;
                return;
            }
            if self.state.new_tab(profile) && fresh {
                self.state.close(0);
            }
            if elevated {
                // An elevated console starts in C:\Windows\System32 and says
                // so in its title, as Windows does.
                let tab = self.state.tab();
                tab.elevated = true;
                tab.title = alloc::format!("Administrator: {}", profile.name());
                if profile == Profile::Cmd {
                    tab.cwd = String::from("/compat/Windows/System32");
                }
            }
            self.state.page = door_workspace::terminal_model::Page::Console;
            if profile == Profile::Cmd {
                self.prepare_cmd_tab(storage);
            }
            return;
        }
        if lower.ends_with(".ps1") {
            self.run_powershell_script(path, storage);
        } else if lower.ends_with(".bat") || lower.ends_with(".cmd") {
            self.run_batch(path, elevated, storage);
        } else if storage.metadata(path).is_ok_and(|i| i.directory)
            && self.state.new_tab(Profile::Native)
        {
            let t = self.state.active;
            self.state.tabs[t].cwd = String::from(path);
        }
    }
    /// A key while the active CMD tab waits for one (true when used).
    pub(super) fn cmd_key(&mut self, scan: u32, text: u8, storage: &mut Storage) -> bool {
        let index = self.state.active;
        let Some(s) = self.session(self.state.tabs[index].id) else {
            return false;
        };
        let key = match (scan, text) {
            (28, _) => Some('\r'),
            (_, t) if t >= 0x20 => Some(t as char),
            _ => None,
        };
        let resume = match self.sessions[s].wait {
            Wait::Key | Wait::Sleep { keys: true, .. } => match key {
                Some(c) => Resume::Key(c),
                None => return true,
            },
            Wait::Line if scan == 28 => {
                let line = self.state.tabs[index].submit(false);
                let tab = &mut self.state.tabs[index];
                tab.stream(&line);
                tab.stream("\n");
                Resume::Line(line)
            }
            Wait::Line => return false,
            Wait::Ready => return false,
            _ => return true,
        };
        if let Wait::Key = self.sessions[s].wait
            && let Resume::Key(c) = resume
            && c != '\r'
        {
            // A Y/N answer shows the key, as the console echoes it.
            self.state.tabs[index].stream(&String::from(c));
        }
        let step = {
            let mut host = self.host(storage);
            self.sessions[s].engine.resume(&mut host, resume)
        };
        self.apply(s, step, storage);
        true
    }
    /// Ctrl+C on a CMD tab.
    pub(super) fn cmd_interrupt(&mut self, storage: &mut Storage) -> bool {
        let index = self.state.active;
        let Some(s) = self.session(self.state.tabs[index].id) else {
            return false;
        };
        if let Wait::Program = self.sessions[s].wait {
            self.sessions[s].interrupted = true;
            self.cancel_program();
            return true;
        }
        let step = {
            let mut host = self.host(storage);
            self.sessions[s].engine.interrupt(&mut host)
        };
        self.state.tabs[index].input.set(String::new(), false);
        self.apply(s, step, storage);
        true
    }
    /// Carry out engine steps until it waits.
    pub(super) fn apply(&mut self, s: usize, mut step: Step, storage: &mut Storage) {
        for _ in 0..64 {
            let id = self.sessions[s].tab;
            let Some(index) = self.tab_index(id) else {
                self.sessions.remove(s);
                return;
            };
            let events = self.sessions[s].engine.take_events();
            for e in events {
                let tab = &mut self.state.tabs[index];
                match e {
                    Event::Text(t) => {
                        // Guest scenarios read what the console shows.
                        let kind = if self.sessions[s].engine.is_doorsh() {
                            "DSH"
                        } else {
                            "CMD"
                        };
                        // A long line goes in pieces of at most 160
                        // bytes: TEXT, then MORE lines continuing it.
                        for line in t.split('\n').filter(|l| !l.trim().is_empty()) {
                            let shown: String = line.chars().filter(|c| !c.is_control()).collect();
                            let mut rest = shown.as_str();
                            let mut word = "TEXT";
                            loop {
                                let mut end = rest.len().min(160);
                                while !rest.is_char_boundary(end) {
                                    end -= 1;
                                }
                                door_rt::diagnostic(format_args!(
                                    "DOOR_P16 {kind} {word} {}\n",
                                    &rest[..end]
                                ));
                                rest = &rest[end..];
                                word = "MORE";
                                if rest.is_empty() {
                                    break;
                                }
                            }
                        }
                        self.sessions[s].bytes += t.len();
                        tab.stream(&t)
                    }
                    Event::Clear => tab.clear(),
                    Event::Title(t) => {
                        let t: String = t.chars().filter(|c| !c.is_control()).take(80).collect();
                        let t = if t.trim().is_empty() {
                            String::from(tab.profile.name())
                        } else {
                            t
                        };
                        tab.title = if tab.elevated {
                            format!("Administrator: {t}")
                        } else {
                            t
                        };
                    }
                    Event::Color(_) => (),
                }
            }
            let now = door_sys::time().unwrap_or(0);
            match step {
                Step::Idle => {
                    let level = self.sessions[s].engine.errorlevel();
                    if self.sessions[s].engine.is_doorsh() {
                        door_rt::diagnostic(format_args!("DOOR_P16 DSH IDLE status={level}\n"));
                        // The native shell's completion record (P12 scenarios).
                        if !self.sessions[s].line.is_empty() {
                            door_rt::diagnostic(format_args!(
                                "DOOR_P12 TERMINAL COMMAND name={} ok={} bytes={}\n",
                                self.sessions[s].line,
                                u8::from(level == 0),
                                self.sessions[s].bytes
                            ));
                            self.sessions[s].line.clear();
                        }
                    } else {
                        door_rt::diagnostic(format_args!("DOOR_P16 CMD IDLE errorlevel={level}\n"));
                    }
                    let session = &mut self.sessions[s];
                    session.wait = Wait::Ready;
                    let tab = &mut self.state.tabs[index];
                    tab.end_stream();
                    if session.interactive && session.engine.echo_on() {
                        tab.output("");
                    }
                    session.interactive = false;
                    let prompt = {
                        let host = self.host(storage);
                        self.sessions[s].engine.prompt(&host)
                    };
                    let tab = &mut self.state.tabs[index];
                    tab.prompt = Some(prompt);
                    tab.hide_input = false;
                    return;
                }
                Step::Input(kind) => {
                    let tab = &mut self.state.tabs[index];
                    tab.prompt = Some(String::new());
                    tab.hide_input = kind == InputKind::Key;
                    self.sessions[s].wait = if kind == InputKind::Key {
                        Wait::Key
                    } else {
                        Wait::Line
                    };
                    return;
                }
                Step::Sleep { ms, keys } => {
                    self.state.tabs[index].hide_input = true;
                    let ticks = ms / door_abi::platform::MONOTONIC_TICK_MILLIS;
                    self.sessions[s].wait = Wait::Sleep {
                        until: now + ticks,
                        keys,
                    };
                    return;
                }
                Step::Spawn(sp) => {
                    self.state.tabs[index].hide_input = true;
                    match self.spawn_for_engine(index, &sp) {
                        Ok(true) => {
                            self.sessions[s].wait = Wait::Program;
                            return;
                        }
                        Ok(false) => {
                            let mut host = self.host(storage);
                            step = self.sessions[s].engine.resume(
                                &mut host,
                                Resume::Exited {
                                    code: 0,
                                    output: Vec::new(),
                                },
                            );
                        }
                        Err(e) => {
                            let err = match e {
                                Error::Service(Code::NotFound) => {
                                    door_shells::host::FsError::NotFound
                                }
                                Error::Service(Code::Denied) => door_shells::host::FsError::Denied,
                                _ => door_shells::host::FsError::Unsupported,
                            };
                            let mut host = self.host(storage);
                            step = self.sessions[s]
                                .engine
                                .resume(&mut host, Resume::Failed(err));
                        }
                    }
                }
                Step::Launch(Launch::Terminal {
                    profile,
                    cwd,
                    script,
                }) => {
                    let mut host = self.host(storage);
                    step = self.sessions[s]
                        .engine
                        .resume(&mut host, Resume::Done(true));
                    drop(host);
                    match script {
                        Some(path) => self.pending_batches.push_back(path),
                        None => {
                            let p = if profile == door_shells::Profile::Cmd {
                                Profile::Cmd
                            } else {
                                Profile::Native
                            };
                            if self.state.new_tab(p) {
                                let t = self.state.active;
                                self.state.tabs[t].cwd = cwd;
                                self.state.active = index;
                            }
                        }
                    }
                }
                Step::Launch(Launch::Power(p)) => {
                    let ok = Applications::connect(self.registry)
                        .and_then(|mut a| a.power(p == door_shells::Power::Restart))
                        .is_ok();
                    let mut host = self.host(storage);
                    step = self.sessions[s].engine.resume(&mut host, Resume::Done(ok));
                }
                Step::Launch(l) => {
                    if let Launch::Settings(nav) = &l {
                        let note = door_shells::legacy::settings_note(*nav);
                        self.state.tabs[index].output(&note);
                    }
                    door_rt::diagnostic(format_args!("DOOR_P16 CMD LAUNCH {l:?}\n"));
                    self.launches.push_back((id, l));
                    self.sessions[s].wait = Wait::Launch(None);
                    self.state.tabs[index].hide_input = true;
                    return;
                }
                Step::Exit(_) => {
                    self.sessions.remove(s);
                    self.state.close(index);
                    return;
                }
            }
        }
    }
    /// Start the program a CMD step asked for; Ok(true) while it runs,
    /// Ok(false) when it was started detached (`start`).
    fn spawn_for_engine(&mut self, index: usize, sp: &door_shells::Spawn) -> Result<bool, Error> {
        if self.registry == 0 {
            return Err(Error::Service(Code::Unsupported));
        }
        let arguments = commands::argument_block(&sp.program, sp.argv.get(1..).unwrap_or(&[]));
        let mut applications = Applications::connect(self.registry)?;
        let cwd = if sp.cwd.is_empty() {
            self.state.tabs[index].cwd.clone()
        } else {
            sp.cwd.clone()
        };
        if sp.detached {
            applications.execute_detached(&sp.program, &cwd, &arguments, self.directory)?;
            return Ok(false);
        }
        if self.running_in(self.state.tabs[index].id) {
            return Err(Error::Service(Code::Resources));
        }
        self.start_program(index, &sp.program, &cwd, &arguments, sp.capture, true)?;
        Ok(true)
    }
    /// The engine's program retired (from `poll_program`).
    pub(super) fn engine_program_done(
        &mut self,
        tab: u64,
        code: i32,
        output: Vec<u8>,
        storage: &mut Storage,
    ) {
        let Some(s) = self.session(tab) else { return };
        let mut step = {
            let mut host = self.host(storage);
            self.sessions[s]
                .engine
                .resume(&mut host, Resume::Exited { code, output })
        };
        if core::mem::take(&mut self.sessions[s].interrupted) {
            let mut host = self.host(storage);
            step = self.sessions[s].engine.interrupt(&mut host);
        }
        self.apply(s, step, storage);
    }
    /// Idle work: timed waits, finished launches and batch files queued
    /// for new tabs. True when a tab changed.
    pub(crate) fn poll_engines(&mut self, storage: &mut Storage, now: u64) -> bool {
        let mut changed = false;
        if !self.powershell_checked {
            self.refresh_powershell(storage);
        }
        self.prepare_cmd_tab(storage);
        let before = self.state.tabs[self.state.active].hosted;
        self.prepare_powershell_tab();
        changed |= before != self.state.tabs[self.state.active].hosted;
        if let Some(path) = self.pending_batches.pop_front() {
            self.run_batch(&path, false, storage);
            changed = true;
        }
        for s in (0..self.sessions.len()).rev() {
            let resume = match self.sessions[s].wait {
                Wait::Sleep { until, .. } if now >= until => Resume::Tick,
                Wait::Launch(Some(ok)) => Resume::Done(ok),
                _ => continue,
            };
            let step = {
                let mut host = self.host(storage);
                self.sessions[s].engine.resume(&mut host, resume)
            };
            self.apply(s, step, storage);
            changed = true;
        }
        changed
    }
    /// The launches engines asked for, carried out with the window client.
    pub(super) fn run_launches(&mut self, client: &mut WindowClient) -> Result<bool, Error> {
        let mut any = false;
        while let Some((tab, launch)) = self.launches.pop_front() {
            any = true;
            let result = match &launch {
                Launch::App { app, argument } => match argument {
                    Some(path) if matches!(app.command(), 2 | 10) => {
                        client.open_document_in_app(app.command(), path.as_bytes())
                    }
                    _ => client.launch_app(app.command()),
                },
                Launch::Settings(nav) => client.open_settings_page(*nav),
                Launch::Open(path) => {
                    let app = document_app(path);
                    if app == 1 {
                        client.launch_app(1)
                    } else {
                        client.open_document_in_app(app, path.as_bytes())
                    }
                }
                Launch::Url(_) => Err(Error::Service(Code::Unsupported)),
                Launch::Terminal { .. } | Launch::Power(_) => Ok(()),
            };
            let ok = result.is_ok();
            if let Launch::Url(url) = &launch
                && let Some(index) = self.tab_index(tab)
            {
                self.state.tabs[index].output(&format!(
                    "Doors has no web browser yet: {url} could not be opened."
                ));
            }
            if let Some(s) = self.session(tab) {
                self.sessions[s].wait = Wait::Launch(Some(ok));
            }
        }
        Ok(any)
    }
}
/// The Doors app a document opens in: text in Notes, media and archives in
/// Preview, folders in File Explorer.
fn document_app(path: &str) -> u32 {
    let name = path.rsplit('/').next().unwrap_or("");
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "" => 1,
        "txt" | "log" | "md" | "ini" | "inf" | "cfg" | "conf" | "bat" | "cmd" | "dsh" | "csv"
        | "json" | "xml" | "reg" | "ps1" | "c" | "h" | "cpp" | "rs" | "py" | "js" | "css"
        | "html" | "htm" | "yaml" | "yml" | "toml" => 2,
        _ => 10,
    }
}

impl Terminal {
    /// The tab under a press in the caption strip that can be dragged (not
    /// its close button): the window runtime drags it along the strip, to
    /// another Terminal window or out into a new one.
    pub(crate) fn dragged_tab(&self, x: i32, y: i32) -> Option<usize> {
        let (count, active) = (self.state.tabs.len(), self.state.active);
        if y < 0 || y >= door_workspace::terminal_model::CAPTION as i32 {
            return None;
        }
        if self.layout.close_at(x, y, count, active).is_some() {
            return None;
        }
        self.layout.tab_at(x, y, count, active)
    }
    pub(crate) fn tab_identity(&self, index: usize) -> Option<u64> {
        self.state.tabs.get(index).map(|t| t.id)
    }
    pub(crate) fn tab_position(&self, id: u64) -> Option<usize> {
        self.tab_index(id)
    }
    /// Move tab `index` to `target` (another Terminal window): its running
    /// program, its shell engine and its queued launches go with it, as a
    /// Windows Terminal tab keeps its session when dragged to another
    /// window. `pristine`: the target is a window just made for the tab,
    /// whose own startup tab (and anything it started) gives way.
    pub(crate) fn transfer_tab_to(
        &mut self,
        target: &mut Terminal,
        index: usize,
        pristine: bool,
    ) -> bool {
        let Some(id) = self.tab_identity(index) else {
            return false;
        };
        let last = self.state.tabs.len() == 1;
        if pristine {
            for tab in &target.state.tabs {
                let gone = tab.id;
                for running in target.running.iter_mut().filter(|r| r.tab_id() == gone) {
                    running.end();
                }
                target.sessions.retain(|s| s.tab != gone);
                target.launches.retain(|(t, _)| *t != gone);
            }
        }
        let Some(tab) = self.state.take(index) else {
            return false;
        };
        let mut i = 0;
        while i < self.running.len() {
            if self.running[i].tab_id() == id {
                target.running.push(self.running.remove(i));
            } else {
                i += 1;
            }
        }
        let mut i = 0;
        while i < self.sessions.len() {
            if self.sessions[i].tab == id {
                target.sessions.push(self.sessions.remove(i));
            } else {
                i += 1;
            }
        }
        let (moved, kept): (VecDeque<_>, VecDeque<_>) =
            self.launches.drain(..).partition(|(t, _)| *t == id);
        self.launches = kept;
        target.launches.extend(moved);
        target.state.adopt(tab, pristine);
        if last {
            self.emptied = true;
        }
        true
    }
    /// Tab `from` dragged to position `to` of the strip.
    pub(crate) fn move_tab(&mut self, from: usize, to: usize) -> bool {
        self.state.reorder(from, to)
    }
    /// The window gave its last tab away.
    pub(crate) fn emptied(&self) -> bool {
        self.emptied
    }
}

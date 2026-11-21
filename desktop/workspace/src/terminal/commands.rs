//! Programs started from a Terminal tab (by cmd.exe, Doorsh or the tab's
//! own PowerShell): Init EXECUTE runs them in the tab's folder, each in a
//! console of its own (`console::Command::Open`), as Windows Terminal gives
//! each tab its own pseudoconsole; their children use the same console
//! (Init passes it on). The console's output is mirrored into the tab (or
//! captured for the engine's pipes and redirections) and the engine
//! resumes when the process retires. Every tab can run a program at once.
//!
//! A program whose output the tab shows runs in a console grid
//! (`Tab::open_console`), as in Windows Terminal: its output drives the
//! grid, the keys typed in the tab are its input (win32-input-mode
//! sequences through the console service, which also follows the grid's
//! size and the program's cursor), and the grid's rows join the
//! scrollback when it ends. Ctrl+C is a signal for the console's programs
//! while their input is processed (each program's console control
//! handlers take it); when none took it within `SIGNAL_TICKS`, Init ends
//! the console's programs, as Windows' default handler would have.
//!
//! Mirrored output is bounded per frame (`Ingest`): at most
//! `MIRROR_CHUNKS` mirror reads, only while a chunk fits the pending queue
//! (a flood is left to the console service's bounded ring), and at most
//! `INGEST_BYTES` streamed into the scrollback, so input and painting keep
//! running however fast a program writes.
use super::*;
use door_service_client::{Applications, Console};
use door_service_protocol::{CHUNK_BYTES, console as wire};
/// Mirror reads per frame at most.
const MIRROR_CHUNKS: usize = 32;
/// Mirror reads after the program retired at most (its children may keep
/// writing to the console).
const AFTER_EXIT_CHUNKS: usize = 512;
/// ENABLE_PROCESSED_INPUT: Ctrl+C is a signal, not a key.
const PROCESSED_INPUT: u32 = 1;
/// How long the console's programs have to take a Ctrl+C signal (1 s).
const SIGNAL_TICKS: u64 = 100;
/// A program started from a tab.
pub(super) struct Running {
    name: String,
    /// The tab's identity (it may move when others close).
    tab_id: u64,
    /// Output kept for the engine (pipes, redirections) instead of shown.
    capture: Option<Vec<u8>>,
    /// A CMD tab's engine waits for the exit.
    engine: bool,
    child: u64,
    applications: Applications,
    /// The console service connection, attached to the program's console.
    console: Option<Console>,
    /// The program's console (0: the session's, when none was free).
    id: u8,
    /// The tab shows the program in a console grid.
    grid: bool,
    /// Mirrored bytes not yet in the scrollback (bounded).
    ingest: Ingest,
    bytes: usize,
    /// The retired program's exit, once observed.
    exit: Option<Result<i64, door_abi::Status>>,
    /// Mirror reads left after the exit.
    after_exit: usize,
    /// A Ctrl+C signal sent (the console's count after it) and when the
    /// programs must have taken it.
    signal: Option<(u32, u64)>,
    /// Its tab closed: its programs were ended.
    orphaned: bool,
}
/// Program argument block: the resolved path, then each word, NUL-terminated.
pub(super) fn argument_block(path: &str, words: &[String]) -> Vec<u8> {
    let mut arguments = Vec::new();
    for word in core::iter::once(path).chain(words.iter().map(String::as_str)) {
        arguments.extend_from_slice(word.as_bytes());
        arguments.push(0);
    }
    arguments
}
impl Terminal {
    /// The program running in the tab with identity `tab_id`.
    pub(super) fn running_in(&self, tab_id: u64) -> bool {
        self.running
            .iter()
            .any(|r| r.tab_id == tab_id && r.exit.is_none())
    }
    /// Start a program for tab number tab (its output mirrored there, or
    /// captured for the engine).
    pub(super) fn start_program(
        &mut self,
        tab: usize,
        path: &str,
        cwd: &str,
        arguments: &[u8],
        capture: bool,
        engine: bool,
    ) -> Result<(), Error> {
        let tab_id = self.state.tabs[tab].id;
        if self.running.iter().any(|r| r.tab_id == tab_id) {
            return Err(Error::Service(Code::Resources));
        }
        let mut console = Console::connect(self.registry).ok();
        // A shown program gets the console grid, sized as the tab, before
        // it writes anything; a captured one a console of the same size.
        let size = self.grid();
        let grid = !capture && console.is_some();
        let row = if grid {
            self.state.tabs[tab].open_console(size.columns, size.rows)
        } else {
            0
        };
        let mut id = 0;
        if let Some(c) = console.as_mut() {
            let opened = c
                .command(wire::Command::Open {
                    columns: size.columns as u32,
                    rows: size.rows as u32,
                    row: row as u32,
                })
                .and_then(|s| {
                    let n = s.console as u8;
                    c.command(wire::Command::Attach(n)).map(|_| n)
                });
            match opened {
                Ok(n) => id = n,
                Err(error) => {
                    self.state.tabs[tab].close_console();
                    return Err(error);
                }
            }
        }
        let close = |c: &mut Option<Console>| {
            if let Some(c) = c.as_mut()
                && id != 0
            {
                let _ = c.command(wire::Command::Close(id));
            }
        };
        let mut applications = match Applications::connect(self.registry) {
            Ok(a) => a,
            Err(error) => {
                close(&mut console);
                self.state.tabs[tab].close_console();
                return Err(error);
            }
        };
        let launched = if id != 0 {
            applications.execute_in_console(path, cwd, arguments, self.directory, id)
        } else {
            applications.execute(path, cwd, arguments, self.directory)
        };
        let child = match launched {
            Ok(child) => child,
            Err(error) => {
                close(&mut console);
                self.state.tabs[tab].close_console();
                return Err(error);
            }
        };
        let name = String::from(path.rsplit('/').next().unwrap_or(path));
        self.running.push(Running {
            name,
            tab_id,
            capture: capture.then(Vec::new),
            engine,
            child,
            applications,
            console,
            id,
            grid,
            ingest: Ingest::default(),
            bytes: 0,
            exit: None,
            after_exit: AFTER_EXIT_CHUNKS,
            signal: None,
            orphaned: false,
        });
        Ok(())
    }
    /// The active tab's program: Ctrl+C to its console (a signal for its
    /// programs), or, without a console of its own, Init ends the programs
    /// this Terminal started.
    pub(super) fn cancel_program(&mut self) {
        let id = self.state.tabs[self.state.active].id;
        let Some(running) = self.running.iter_mut().find(|r| r.tab_id == id) else {
            return;
        };
        if running.id == 0 || !running.interrupt() {
            let _ = running.applications.cancel_children();
        }
    }
    /// The running program's console when the active tab shows it in a grid.
    fn grid_running(&mut self) -> Option<&mut Running> {
        let id = self.state.tabs[self.state.active].id;
        self.running
            .iter_mut()
            .find(|r| r.grid && r.tab_id == id && r.exit.is_none() && r.console.is_some())
    }
    /// A key typed while the active tab shows a program's grid goes to the
    /// program, as Windows Terminal passes keys to the console: false
    /// leaves it to the Terminal (its own shortcuts: Ctrl+Shift+...,
    /// Ctrl+Tab, Ctrl+comma, Shift+PageUp/PageDown scrolling). Ctrl+C is a
    /// signal for the console's programs while their input is processed.
    pub(super) fn program_key(&mut self, scan: u32, text: u8, ctrl: bool, shift: bool) -> bool {
        let alt = self.alt;
        if (ctrl && (shift || matches!(scan, 15 | 51))) || (shift && matches!(scan, 0x149 | 0x151))
        {
            return false;
        }
        let Some(running) = self.grid_running() else {
            return false;
        };
        let delivered = if ctrl && scan == 46 {
            running.interrupt()
        } else {
            let typed = if text != 0 { char::from(text) } else { '\0' };
            match door_vt::input::from_scan(scan as u16, typed, ctrl, shift, alt) {
                None => true,
                Some(key) => {
                    let mut sequence = [0u8; 48];
                    let n = door_vt::input::encode_win32(&key, &mut sequence);
                    let console = running.console.as_mut().expect("grid console");
                    matches!(console.input(&sequence[..n]), Ok(true))
                }
            }
        };
        if !delivered {
            self.message = "The program is not reading input; the key was dropped.".into();
        }
        self.state.tab().scroll = 0;
        true
    }
    /// Pasted text for the program in the active tab's grid (line breaks
    /// as Enter); false when no program takes it.
    pub(super) fn program_paste(&mut self, text: &str) -> bool {
        let Some(running) = self.grid_running() else {
            return false;
        };
        let console = running.console.as_mut().expect("grid console");
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        let mut sent = true;
        let mut rest = text.as_str();
        while !rest.is_empty() && sent {
            let mut cut = rest.len().min(CHUNK_BYTES);
            while !rest.is_char_boundary(cut) {
                cut -= 1;
            }
            sent = matches!(console.input(&rest.as_bytes()[..cut]), Ok(true));
            rest = &rest[cut..];
        }
        let _ = console;
        if !sent {
            self.message = "Part of the paste did not fit the program's input.".into();
        }
        self.state.tab().scroll = 0;
        true
    }
    /// The Terminal closes: every program its tabs run ends with it, as
    /// closing Windows Terminal ends its tabs' programs.
    pub(crate) fn end_programs(&mut self) {
        for running in &mut self.running {
            if running.exit.is_none() {
                let _ = if running.id != 0 {
                    running.applications.cancel_console(running.id)
                } else {
                    running.applications.cancel_children()
                };
            }
        }
    }
    /// Idle work, once per frame: mirror each program's console output
    /// into its tab (bounded, see the module notes) and complete its
    /// command once it has retired and its output is in. True when a tab
    /// changed.
    pub(crate) fn poll_program(&mut self, now: u64, storage: &mut Storage) -> bool {
        let mut changed = false;
        let mut i = 0;
        while i < self.running.len() {
            match self.poll_one(i, now) {
                Poll::Running(c) => {
                    changed |= c;
                    i += 1;
                }
                Poll::Finished => {
                    let running = self.running.remove(i);
                    self.finish(running, storage);
                    changed = true;
                }
            }
        }
        changed
    }
    fn poll_one(&mut self, i: usize, now: u64) -> Poll {
        let size = self.grid();
        let running = &mut self.running[i];
        if running.exit.is_none() {
            running.exit = match door_sys::process_wait(running.child, now) {
                Ok(code) => Some(Ok(code)),
                Err(door_abi::Status::Timeout | door_abi::Status::WouldBlock) => None,
                Err(e) => Some(Err(e)),
            };
        }
        let exited = running.exit.is_some();
        let index = self.state.tabs.iter().position(|t| t.id == running.tab_id);
        if index.is_none() && !running.orphaned && !exited {
            // Its tab closed: Windows Terminal ends the tab's programs.
            running.orphaned = true;
            let _ = if running.id != 0 {
                running.applications.cancel_console(running.id)
            } else {
                running.applications.cancel_children()
            };
        }
        // Ctrl+C no program took in time: end the console's programs.
        if let Some((count, deadline)) = running.signal
            && now >= deadline
            && !exited
        {
            running.signal = None;
            let taken = running
                .console
                .as_mut()
                .and_then(|c| c.command(wire::Command::Query(running.id)).ok())
                .is_some_and(|s| s.seen >= count);
            if !taken {
                let _ = running.applications.cancel_console(running.id);
            }
        }
        // After retirement every write has completed: drain to the end,
        // still a bounded amount per frame.
        let mut drained = running.console.is_none() || (exited && running.after_exit == 0);
        let mut chunk = [0; CHUNK_BYTES];
        if let Some(console) = running.console.as_mut() {
            for _ in 0..MIRROR_CHUNKS {
                // Backpressure: a full queue leaves the rest in the service.
                if running.ingest.room() < CHUNK_BYTES || drained {
                    break;
                }
                match console.mirror(&mut chunk) {
                    Ok(0) | Err(_) => drained = true,
                    Ok(n) => {
                        if let Some(c) = running.capture.as_mut() {
                            if c.len() < 4 << 20 {
                                c.extend_from_slice(&chunk[..n]);
                            }
                        } else if index.is_some() {
                            running.ingest.push(&chunk[..n]);
                        }
                        running.bytes += n;
                        if exited {
                            running.after_exit -= 1;
                            drained = running.after_exit == 0;
                        }
                    }
                }
            }
        }
        let finished = exited && drained;
        let Some(index) = index else {
            return if finished {
                Poll::Finished
            } else {
                Poll::Running(false)
            };
        };
        let moved = running
            .ingest
            .drain_into(&mut self.state.tabs[index], INGEST_BYTES, finished);
        // The grid follows the window; the console service's cursor too.
        let mut resized = false;
        if running.grid
            && !finished
            && self.state.tabs[index].resize_console(size.columns, size.rows)
        {
            resized = true;
            if let Some(console) = running.console.as_mut() {
                let _ = console.command(wire::Command::Resize {
                    console: running.id,
                    columns: size.columns as u32,
                    rows: size.rows as u32,
                });
            }
        }
        if !finished || !running.ingest.is_empty() {
            return Poll::Running(moved > 0 || resized);
        }
        Poll::Finished
    }
    /// A program retired and its output is in: its console closes and its
    /// tab (or engine) learns how it ended.
    fn finish(&mut self, mut running: Running, storage: &mut Storage) {
        let exit = running.exit.expect("retired program");
        let _ = door_sys::close(running.child);
        if let Some(console) = running.console.as_mut()
            && running.id != 0
        {
            let _ = console.command(wire::Command::Close(running.id));
        }
        let Some(index) = self.tab_index(running.tab_id) else {
            return;
        };
        self.state.tabs[index].close_console();
        if running.engine {
            let output = running.capture.take().unwrap_or_default();
            let code = match exit {
                Ok(c) => c as i32,
                Err(_) => -1,
            };
            self.state.tabs[index].end_stream();
            self.engine_program_done(running.tab_id, code, output, storage);
            return;
        }
        self.state.tabs[index].end_stream();
        if self.state.tabs[index].hosted {
            // A tab's own program ended: the tab closes when it ended
            // cleanly (or by Ctrl+C), else it says how, as Windows
            // Terminal's "automatic" close-on-exit does.
            let code = match exit {
                Ok(code) => code as u32,
                Err(_) => u32::MAX,
            };
            if matches!(code, 0 | 0xc000_013a) {
                if self.state.tabs.len() == 1 {
                    // The last tab: the window closes with it.
                    self.pending_close = true;
                } else {
                    self.state.close(index);
                }
            } else {
                self.state.tabs[index].output(&format!(
                    "\n[process exited with code {code} (0x{code:08x})]\nYou can now close this tab."
                ));
            }
            door_rt::diagnostic(format_args!(
                "DOOR_P18 TERMINAL HOSTED name={} exit={code:#x}\n",
                running.name
            ));
            return;
        }
        let tab = &mut self.state.tabs[index];
        match exit {
            Ok(0) => (),
            Ok(code) => tab.output(&format!("Process exited {code}")),
            Err(e) => tab.output(&format!("Process wait failed: {e:?}")),
        }
        door_rt::diagnostic(format_args!(
            "DOOR_P12 TERMINAL COMMAND name={} ok={} bytes={} exit={}\n",
            running.name,
            u8::from(exit.is_ok()),
            running.bytes,
            exit.unwrap_or(-1)
        ));
    }
}
enum Poll {
    Running(bool),
    Finished,
}
impl Running {
    /// The tab the program runs in.
    pub(super) fn tab_id(&self) -> u64 {
        self.tab_id
    }
    /// Its tab closed without moving: its programs end.
    pub(super) fn end(&mut self) {
        if self.exit.is_none() && !self.orphaned {
            self.orphaned = true;
            let _ = if self.id != 0 {
                self.applications.cancel_console(self.id)
            } else {
                self.applications.cancel_children()
            };
        }
    }
    /// Ctrl+C to the program's console: a signal for its programs while
    /// their input is processed (they have `SIGNAL_TICKS` to take it),
    /// else the program's key. False when the console took nothing.
    fn interrupt(&mut self) -> bool {
        let Some(console) = self.console.as_mut() else {
            return false;
        };
        let Some(key) = door_vt::input::from_scan(46, 'c', true, false, false) else {
            return false;
        };
        let mut sequence = [0u8; 48];
        let n = door_vt::input::encode_win32(&key, &mut sequence);
        if !matches!(console.input(&sequence[..n]), Ok(true)) {
            return false;
        }
        if let Ok(s) = console.state()
            && s.input_mode & PROCESSED_INPUT != 0
            && s.seen < s.signals
            && let Ok(now) = door_sys::time()
        {
            self.signal = Some((s.signals, now + SIGNAL_TICKS));
        }
        true
    }
}

#![no_std]
#![no_main]
extern crate alloc;
use alloc::{format, string::String, vec::Vec};
use door_abi::{
    Handle, Status,
    platform::{StartupV2, inherited},
};
use door_service_client::{
    Applications, Console, Disposition, Error, READ, SHARE_ALL, Setting, Storage, WRITE,
};
use door_service_protocol::Code;
use door_userland::{Command, cwd, parse, print, resolve};

#[global_allocator]
static HEAP: door_rt::HeapAllocator = door_rt::HeapAllocator::new();
door_rt::entry_v2!(main);
struct Shell {
    console: Console,
    storage: Storage,
    applications: Applications,
    cwd: String,
    directory: Handle,
    last_status: i64,
}
impl Shell {
    fn path(&self, value: &str) -> Result<String, Error> {
        resolve(&self.cwd, value).map_err(|_| Error::Service(Code::InvalidRequest))
    }
    fn write_file(&mut self, path: &str, data: &[u8], append: bool) -> Result<(), Error> {
        let file = self.storage.open(
            path,
            if append {
                Disposition::OpenAlways
            } else {
                Disposition::CreateAlways
            },
            WRITE,
            SHARE_ALL,
        )?;
        let result = if append {
            self.storage.append(&file, data)
        } else {
            self.storage.write_all(&file, 0, data)
        };
        let closed = self.storage.close(file);
        result.and(closed)?;
        self.storage.flush()
    }
    fn output(&mut self, command: &Command, text: &str) -> Result<(), Error> {
        if let Some(destination) = &command.output {
            let path = self.path(&destination.path)?;
            self.write_file(&path, text.as_bytes(), destination.append)
        } else {
            print(&mut self.console, text)
        }
    }
    fn cat(&mut self, command: &Command) -> Result<(), Error> {
        if command.words.len() < 2 {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let mut data = Vec::new();
        for name in &command.words[1..] {
            let path = self.path(name)?;
            let file = self
                .storage
                .open(&path, Disposition::OpenExisting, READ, SHARE_ALL)?;
            let result = (|| {
                let mut offset = 0;
                let mut buffer = [0; 192];
                loop {
                    let count = self.storage.read(&file, offset, &mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    if data.len() + count > 512 * 1024 {
                        return Err(Error::Service(Code::Resources));
                    }
                    data.extend_from_slice(&buffer[..count]);
                    offset += count as u64;
                }
                Ok(())
            })();
            let closed = self.storage.close(file);
            result.and(closed)?;
        }
        if let Some(destination) = &command.output {
            let path = self.path(&destination.path)?;
            self.write_file(&path, &data, destination.append)
        } else if let Ok(text) = core::str::from_utf8(&data) {
            print(&mut self.console, text)
        } else {
            // Conversion stays bounded even for a large non-text file.
            for chunk in data.chunks(192) {
                print(&mut self.console, &String::from_utf8_lossy(chunk))?;
            }
            Ok(())
        }
    }
    fn copy(&mut self, source: &str, destination: &str) -> Result<(), Error> {
        let source = self.path(source)?;
        let destination = self.path(destination)?;
        if source == destination {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let input = self
            .storage
            .open(&source, Disposition::OpenExisting, READ, SHARE_ALL)?;
        let result = (|| {
            let output =
                self.storage
                    .open(&destination, Disposition::CreateAlways, WRITE, SHARE_ALL)?;
            let copied = (|| {
                let mut buffer = [0; 4096];
                let mut offset = 0;
                loop {
                    let mut count = 0;
                    while count < buffer.len() {
                        let end = (count + 192).min(buffer.len());
                        let n = self.storage.read(
                            &input,
                            offset + count as u64,
                            &mut buffer[count..end],
                        )?;
                        count += n;
                        if n < end - (count - n) {
                            break;
                        }
                    }
                    if count == 0 {
                        break;
                    }
                    self.storage.write_all(&output, offset, &buffer[..count])?;
                    offset += count as u64;
                }
                Ok(())
            })();
            let closed = self.storage.close(output);
            copied.and(closed)
        })();
        let closed = self.storage.close(input);
        result.and(closed)
    }
    fn execute(&mut self, command: &Command, first: usize) -> Result<(), Error> {
        let name = command
            .words
            .get(first)
            .ok_or(Error::Service(Code::InvalidRequest))?;
        let path = if name.contains('/') {
            self.path(name)?
        } else {
            format!("/system/bin/{name}")
        };
        let mut arguments = Vec::new();
        for word in core::iter::once(&path).chain(command.words[first + 1..].iter()) {
            arguments.extend_from_slice(word.as_bytes());
            arguments.push(0);
        }
        if command.output.is_some() {
            return Err(Error::Service(Code::Unsupported));
        }
        let child = self
            .applications
            .execute(&path, &self.cwd, &arguments, self.directory)?;
        let result = loop {
            if self.console.interrupted()? {
                self.applications.cancel_children()?;
            }
            match door_sys::process_wait(child, door_sys::time()?.saturating_add(10)) {
                Ok(code) => break Ok(code),
                Err(Status::Timeout | Status::WouldBlock) => (),
                Err(error) => break Err(error),
            }
        };
        let closed = door_sys::close(child);
        self.last_status = result?;
        closed?;
        if self.last_status != 0 {
            print(
                &mut self.console,
                &format!("Process exited {}\n", self.last_status),
            )?;
        }
        Ok(())
    }
    fn command(&mut self, command: Command) -> Result<bool, Error> {
        let Some(name) = command.words.first().map(String::as_str) else {
            return Ok(true);
        };
        match name {
            "help" => self.output(&command, "Commands:\n  pwd  cd PATH  ls [PATH]  stat PATH\n  cat FILE...  write FILE TEXT...  append FILE TEXT...\n  echo TEXT... [> FILE | >> FILE]  touch FILE  mkdir PATH\n  cp SOURCE DEST  mv SOURCE DEST  rm PATH  clear\n  run PATH [ARGS...]  tools help  theme [0|1|2]\n  status  shutdown  reboot\nOther names launch /system/bin/NAME. Quotes preserve spaces.\n")?,
            "pwd" => self.output(&command, &format!("{}\n", self.cwd))?,
            "cd" => {
                let path = self.path(command.words.get(1).map(String::as_str).unwrap_or("/users/1001"))?;
                if !self.storage.metadata(&path)?.directory { return Err(Error::Service(Code::InvalidRequest)); }
                self.cwd = path;
            }
            "ls" => {
                let path = self.path(command.words.get(1).map(String::as_str).unwrap_or("."))?;
                let mut cookie = 0;
                let mut listing = String::new();
                while let Some(entry) = self.storage.list(&path, cookie)? {
                    if entry.cookie <= cookie { return Err(Error::Malformed); }
                    cookie = entry.cookie;
                    let name = String::from_utf16_lossy(&entry.name[..entry.name_len]);
                    listing.push_str(&format!("{} {:>8} {name}\n", if entry.info.directory {"d"} else {"f"}, entry.info.bytes));
                }
                self.output(&command, &listing)?;
            }
            "stat" => {
                let path = self.path(command.words.get(1).ok_or(Error::Service(Code::InvalidRequest))?)?;
                let info = self.storage.metadata(&path)?;
                self.output(&command, &format!("{path}: {} bytes, {}, id={}, version={}\n",info.bytes,
                    if info.directory {"directory"} else {"file"},info.object,info.version))?;
            }
            "cat" => self.cat(&command)?,
            "echo" => self.output(&command, &format!("{}\n", command.words[1..].join(" ")))? ,
            "write" | "append" => {
                let path = self.path(command.words.get(1).ok_or(Error::Service(Code::InvalidRequest))?)?;
                let text = format!("{}\n",command.words[2..].join(" "));
                self.write_file(&path, text.as_bytes(), name == "append")?;
            }
            "touch" => {
                let path = self.path(command.words.get(1).ok_or(Error::Service(Code::InvalidRequest))?)?;
                let file = self.storage.open(&path, Disposition::OpenAlways, WRITE, SHARE_ALL)?;
                self.storage.close(file)?; self.storage.flush()?;
            }
            "mkdir" | "rm" => {
                if command.words.len() < 2 { return Err(Error::Service(Code::InvalidRequest)); }
                for path in &command.words[1..] {
                    let path = self.path(path)?;
                    if name == "mkdir" { self.storage.mkdir(&path)?; } else { self.storage.remove(&path)?; }
                    self.storage.flush()?;
                }
            }
            "cp" | "mv" => {
                if command.words.len() != 3 { return Err(Error::Service(Code::InvalidRequest)); }
                if name == "cp" { self.copy(&command.words[1], &command.words[2])?; }
                else {
                    let old = self.path(&command.words[1])?; let new = self.path(&command.words[2])?;
                    self.storage.rename(&old, &new)?; self.storage.flush()?;
                }
            }
            "clear" => self.console.clear()?,
            "theme" => {
                if let Some(value) = command.words.get(1) {
                    let value = value.parse::<u32>().ok().filter(|v| *v <= 2).ok_or(Error::Service(Code::InvalidRequest))?;
                    self.storage.set_setting(Setting::Theme,value)?; self.storage.flush()?; self.console.set_theme(value)?;
                }
                let value = self.storage.get_setting(Setting::Theme)?;
                self.output(&command,&format!("Theme {value}\n"))?;
            }
            "status" => {
                let id = door_sys::identity(0)?;
                self.output(&command,&format!("PID {} UID {} session {}\nWorking directory {}\nLast process exit {}\n",id.pid,id.uid,id.session,self.cwd,self.last_status))?;
            }
            "shutdown" | "reboot" | "exit" => {
                self.storage.flush()?; print(&mut self.console,"Flushing user data and stopping services.\n")?;
                self.applications.power(name == "reboot")?; return Ok(false);
            }
            "run" => self.execute(&command,1)?,
            _ => self.execute(&command,0)?,
        }
        Ok(true)
    }
    fn run(&mut self) -> Result<(), Error> {
        if let Ok(theme) = self.storage.get_setting(Setting::Theme) {
            self.console.set_theme(theme)?;
        }
        print(
            &mut self.console,
            "Doors native shell\nYour files persist across shutdown and restart. Type help.\n",
        )?;
        let mut input = Vec::new();
        loop {
            print(&mut self.console, &format!("{}> ", self.cwd))?;
            input.clear();
            loop {
                let byte = match self.console.read() {
                    Ok(byte) => byte,
                    Err(Error::Service(Code::WouldBlock))
                    | Err(Error::Transport(Status::WouldBlock)) => {
                        door_sys::sleep(door_sys::time()?.saturating_add(1))?;
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                match byte {
                    b'\n' | b'\r' => {
                        print(&mut self.console, "\n")?;
                        break;
                    }
                    8 | 127 if !input.is_empty() => {
                        input.pop();
                        print(&mut self.console, "\x08")?;
                    }
                    3 | 27 => {
                        input.clear();
                        print(&mut self.console, "\n")?;
                        break;
                    }
                    32..=126 if input.len() < door_userland::MAX_LINE_BYTES => {
                        input.push(byte);
                        print(
                            &mut self.console,
                            core::str::from_utf8(&[byte]).map_err(|_| Error::Malformed)?,
                        )?;
                    }
                    _ => (),
                }
            }
            let line = core::str::from_utf8(&input).map_err(|_| Error::Malformed)?;
            match parse(line) {
                Ok(command) => match self.command(command) {
                    Ok(true) => (),
                    Ok(false) => return Ok(()),
                    Err(error) => {
                        self.last_status = 1;
                        let message = match error {
                            Error::Service(Code::Denied) => "Access denied.",
                            Error::Service(Code::NotFound) => "No such file or program.",
                            Error::Service(Code::Exists) => "The destination already exists.",
                            Error::Service(Code::InvalidRequest) => {
                                "Invalid path/arguments or malformed executable."
                            }
                            Error::Service(Code::Unsupported) => {
                                "This executable format, dependency or operation is unsupported."
                            }
                            Error::Service(Code::Resources) => {
                                "Resource or disk capacity limit reached."
                            }
                            Error::Service(Code::SharingViolation) => {
                                "The file is in use with incompatible sharing."
                            }
                            _ => "The operation failed.",
                        };
                        print(&mut self.console, &format!("{message} ({error:?})\n"))?;
                    }
                },
                Err(error) => print(&mut self.console, &format!("Command syntax: {error:?}\n"))?,
            }
        }
    }
}
fn main(startup: StartupV2) -> i64 {
    if HEAP.initialize().is_err() {
        return 70;
    }
    let registry = startup.base.inherited[inherited::REGISTRY].handle;
    let connected = (|| -> Result<Shell, Error> {
        Ok(Shell {
            console: Console::connect(registry)?,
            storage: Storage::connect(registry)?,
            applications: Applications::connect(registry)?,
            cwd: cwd(&startup),
            directory: startup.base.inherited[inherited::DIRECTORY].handle,
            last_status: 0,
        })
    })();
    match connected {
        Ok(mut shell) => match shell.run() {
            Ok(()) => 0,
            Err(error) => {
                let _ = print(&mut shell.console, &format!("Shell stopped: {error:?}\n"));
                1
            }
        },
        Err(_) => 71,
    }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    door_rt::panic()
}

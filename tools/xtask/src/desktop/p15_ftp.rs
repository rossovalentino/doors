//! P15 CONNECT-A's FTP server on this host: RFC 959 with EPSV (RFC 2428),
//! passive data connections only (QEMU's user network cannot reach the
//! guest), over a fresh directory holding `hello.txt` ("hello over ftp")
//! and `pub/readme.txt`. It logs every command, so a run records what
//! WinINet (Windows' or Doors') actually sent.
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, UNIX_EPOCH},
};

pub(super) struct Ftp {
    pub port: u16,
    pub log: Arc<Mutex<Vec<String>>>,
    pub stop: Arc<AtomicBool>,
}
impl Drop for Ftp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
/// The contents the probe reads back.
pub(super) const HELLO: &[u8] = b"hello over ftp";

pub(super) fn serve(root: &Path) -> Result<Ftp, String> {
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root.join("pub")).map_err(|e| e.to_string())?;
    fs::write(root.join("hello.txt"), HELLO).map_err(|e| e.to_string())?;
    fs::write(root.join("pub").join("readme.txt"), b"readme").map_err(|e| e.to_string())?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let log = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let (l, s, r) = (log.clone(), stop.clone(), root.to_path_buf());
    thread::spawn(move || {
        while !s.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let (l, r, s) = (l.clone(), r.clone(), s.clone());
                    thread::spawn(move || {
                        let _ = session(stream, &r, &l, &s);
                    });
                }
                Err(_) => thread::sleep(Duration::from_millis(20)),
            }
        }
    });
    Ok(Ftp { port, log, stop })
}

struct State {
    root: PathBuf,
    /// The current directory, "/"-rooted.
    cwd: String,
    passive: Option<TcpListener>,
    rename_from: Option<PathBuf>,
}
impl State {
    /// A client path inside the root, or None when it climbs out.
    fn resolve(&self, path: &str) -> Option<(String, PathBuf)> {
        let joined = if path.starts_with('/') {
            String::from(path)
        } else {
            format!("{}/{}", self.cwd.trim_end_matches('/'), path)
        };
        let mut parts: Vec<&str> = Vec::new();
        for p in joined.split('/') {
            match p {
                "" | "." => (),
                ".." => {
                    parts.pop()?;
                }
                p => parts.push(p),
            }
        }
        let virtual_path = format!("/{}", parts.join("/"));
        let mut real = self.root.clone();
        for p in &parts {
            real.push(p);
        }
        Some((virtual_path, real))
    }
}
fn reply(stream: &mut TcpStream, text: &str) -> std::io::Result<()> {
    stream.write_all(format!("{text}\r\n").as_bytes())
}
fn session(
    mut control: TcpStream,
    root: &Path,
    log: &Mutex<Vec<String>>,
    stop: &AtomicBool,
) -> std::io::Result<()> {
    // Accepted sockets inherit the listener's nonblocking mode on Windows.
    control.set_nonblocking(false)?;
    control.set_read_timeout(Some(Duration::from_secs(60)))?;
    let mut reader = BufReader::new(control.try_clone()?);
    let mut state = State {
        root: root.to_path_buf(),
        cwd: String::from("/"),
        passive: None,
        rename_from: None,
    };
    reply(&mut control, "220 Doors test FTP server ready.")?;
    let mut line = String::new();
    while !stop.load(Ordering::Relaxed) {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let command = line.trim_end();
        let (verb, argument) = match command.split_once(' ') {
            Some((v, a)) => (v.to_ascii_uppercase(), a),
            None => (command.to_ascii_uppercase(), ""),
        };
        if let Ok(mut l) = log.lock() {
            l.push(if verb == "PASS" {
                String::from("PASS ****")
            } else {
                String::from(command)
            });
        }
        match verb.as_str() {
            "USER" => reply(&mut control, "331 Password required.")?,
            "PASS" => reply(&mut control, "230 User logged in.")?,
            "SYST" => reply(&mut control, "215 UNIX Type: L8")?,
            "FEAT" => {
                reply(&mut control, "211-Features:")?;
                reply(&mut control, " EPSV")?;
                reply(&mut control, " SIZE")?;
                reply(&mut control, " MDTM")?;
                reply(&mut control, "211 End")?;
            }
            "OPTS" | "MODE" | "STRU" | "ALLO" => reply(&mut control, "200 OK.")?,
            "TYPE" => reply(&mut control, &format!("200 Type set to {argument}."))?,
            "NOOP" => reply(&mut control, "200 NOOP ok.")?,
            "PWD" | "XPWD" => {
                let q = state.cwd.replace('"', "\"\"");
                reply(
                    &mut control,
                    &format!("257 \"{q}\" is the current directory."),
                )?
            }
            "CWD" | "XCWD" | "CDUP" | "XCUP" => {
                let target = if matches!(verb.as_str(), "CDUP" | "XCUP") {
                    ".."
                } else {
                    argument
                };
                match state.resolve(target) {
                    Some((v, real)) if real.is_dir() => {
                        state.cwd = v;
                        reply(&mut control, "250 Directory changed.")?
                    }
                    _ => reply(&mut control, "550 No such directory.")?,
                }
            }
            "PASV" | "EPSV" => {
                let listener = TcpListener::bind("127.0.0.1:0")?;
                let port = listener.local_addr()?.port();
                state.passive = Some(listener);
                if verb == "PASV" {
                    reply(
                        &mut control,
                        &format!(
                            "227 Entering Passive Mode (127,0,0,1,{},{}).",
                            port >> 8,
                            port & 0xff
                        ),
                    )?
                } else {
                    reply(
                        &mut control,
                        &format!("229 Entering Extended Passive Mode (|||{port}|)"),
                    )?
                }
            }
            "PORT" | "EPRT" => reply(&mut control, "502 Active mode is not offered.")?,
            "LIST" | "NLST" | "RETR" | "STOR" => {
                let Some(listener) = state.passive.take() else {
                    reply(&mut control, "425 Use PASV first.")?;
                    continue;
                };
                // LIST's argument may carry ls flags ("-a").
                let argument = argument
                    .split_whitespace()
                    .filter(|a| !a.starts_with('-'))
                    .collect::<Vec<_>>()
                    .join(" ");
                let target = state.resolve(if argument.is_empty() { "." } else { &argument });
                let Some((_, real)) = target else {
                    reply(&mut control, "550 Permission denied.")?;
                    continue;
                };
                if matches!(verb.as_str(), "RETR") && !real.is_file() {
                    reply(&mut control, "550 No such file.")?;
                    continue;
                }
                reply(&mut control, "150 Opening data connection.")?;
                listener.set_nonblocking(false)?;
                let (mut data, _) = listener.accept()?;
                data.set_read_timeout(Some(Duration::from_secs(30)))?;
                match verb.as_str() {
                    "LIST" | "NLST" => {
                        let mut text = String::new();
                        let mut names: Vec<_> = fs::read_dir(&real)?.flatten().collect();
                        names.sort_by_key(|e| e.file_name());
                        for e in names {
                            let name = e.file_name().to_string_lossy().into_owned();
                            if verb == "NLST" {
                                text.push_str(&format!("{name}\r\n"));
                                continue;
                            }
                            let meta = e.metadata()?;
                            let (kind, size) = if meta.is_dir() {
                                ('d', 4096)
                            } else {
                                ('-', meta.len())
                            };
                            text.push_str(&format!(
                                "{kind}rw-r--r--    1 ftp      ftp      {size:>8} Nov  8 14:30 {name}\r\n"
                            ));
                        }
                        data.write_all(text.as_bytes())?;
                    }
                    "RETR" => data.write_all(&fs::read(&real)?)?,
                    _ => {
                        let mut bytes = Vec::new();
                        data.read_to_end(&mut bytes)?;
                        fs::write(&real, bytes)?;
                    }
                }
                drop(data);
                reply(&mut control, "226 Transfer complete.")?;
            }
            "SIZE" => match state.resolve(argument) {
                Some((_, real)) if real.is_file() => {
                    let size = fs::metadata(&real)?.len();
                    reply(&mut control, &format!("213 {size}"))?
                }
                _ => reply(&mut control, "550 No such file.")?,
            },
            "MDTM" => match state.resolve(argument) {
                Some((_, real)) if real.is_file() => {
                    let _ = fs::metadata(&real)?.modified()?.duration_since(UNIX_EPOCH);
                    reply(&mut control, "213 20261108143000")?
                }
                _ => reply(&mut control, "550 No such file.")?,
            },
            "DELE" => match state.resolve(argument) {
                Some((_, real)) if real.is_file() => {
                    fs::remove_file(real)?;
                    reply(&mut control, "250 Deleted.")?
                }
                _ => reply(&mut control, "550 No such file.")?,
            },
            "MKD" | "XMKD" => match state.resolve(argument) {
                Some((v, real)) if !real.exists() => {
                    fs::create_dir(real)?;
                    let q = v.replace('"', "\"\"");
                    reply(&mut control, &format!("257 \"{q}\" created."))?
                }
                _ => reply(&mut control, "550 Cannot create.")?,
            },
            "RMD" | "XRMD" => match state.resolve(argument) {
                Some((_, real)) if real.is_dir() && real != state.root => {
                    match fs::remove_dir(real) {
                        Ok(()) => reply(&mut control, "250 Removed.")?,
                        Err(_) => reply(&mut control, "550 Directory not empty.")?,
                    }
                }
                _ => reply(&mut control, "550 No such directory.")?,
            },
            "RNFR" => match state.resolve(argument) {
                Some((_, real)) if real.exists() => {
                    state.rename_from = Some(real);
                    reply(&mut control, "350 Ready for RNTO.")?
                }
                _ => reply(&mut control, "550 No such file.")?,
            },
            "RNTO" => match (state.rename_from.take(), state.resolve(argument)) {
                (Some(from), Some((_, to))) if !to.exists() => {
                    fs::rename(from, to)?;
                    reply(&mut control, "250 Renamed.")?
                }
                _ => reply(&mut control, "503 Bad sequence of commands.")?,
            },
            "QUIT" => {
                reply(&mut control, "221 Goodbye.")?;
                break;
            }
            _ => reply(&mut control, "502 Command not implemented.")?,
        }
    }
    Ok(())
}

//! P15 CONNECT-A: ordinary Windows console programs (`netprobe.exe` for
//! Winsock, `dnsprobe.exe` for IP Helper and the DNS client; built with the
//! pinned MSVC toolchain and SDK import libraries, no C runtime) run from
//! Terminal against endpoints on this host, reached through QEMU's user
//! network at 10.0.2.2: TCP and UDP echo servers, a closed port and the
//! `doors.test` DNS zone (`p15_names`); the host also connects to
//! netprobe's own listening socket through a forwarded port. `httpprobe`
//! and `inetprobe` cover WinHTTP and WinINet, `diagprobe` ICMP echo and the
//! connection tables, and `iocpprobe` overlapped Winsock through a
//! completion port (AcceptEx through the same forwarded port). Each program
//! prints one line per check to its console and exits 0 only when every
//! step passed.
use super::*;
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream, UdpSocket},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// A probe program: its source directory, the SDK import libraries it
/// links and the modules it must import.
pub(super) struct Probe {
    pub name: &'static str,
    pub directory: &'static str,
    pub libraries: &'static [&'static str],
    pub modules: &'static [&'static str],
}
const NETPROBE: Probe = Probe {
    name: "netprobe",
    directory: DIRECTORY,
    libraries: &["kernel32.lib", "WS2_32.Lib"],
    modules: &["kernel32.dll", "ws2_32.dll"],
};
const DNSPROBE: Probe = Probe {
    name: "dnsprobe",
    directory: DIRECTORY,
    libraries: &["kernel32.lib", "iphlpapi.lib", "DnsAPI.Lib"],
    modules: &["dnsapi.dll", "iphlpapi.dll", "kernel32.dll"],
};
const HTTPPROBE: Probe = Probe {
    name: "httpprobe",
    directory: DIRECTORY,
    libraries: &["kernel32.lib", "winhttp.lib"],
    modules: &["kernel32.dll", "winhttp.dll"],
};
const INETPROBE: Probe = Probe {
    name: "inetprobe",
    directory: DIRECTORY,
    libraries: &["kernel32.lib", "WinInet.Lib"],
    modules: &["kernel32.dll", "wininet.dll"],
};
const DIAGPROBE: Probe = Probe {
    name: "diagprobe",
    directory: DIRECTORY,
    libraries: &["kernel32.lib", "WS2_32.Lib", "iphlpapi.lib"],
    modules: &["iphlpapi.dll", "kernel32.dll", "ws2_32.dll"],
};
const V6PROBE: Probe = Probe {
    name: "v6probe",
    directory: DIRECTORY,
    libraries: &["kernel32.lib", "WS2_32.Lib", "iphlpapi.lib"],
    modules: &["iphlpapi.dll", "kernel32.dll", "ws2_32.dll"],
};
/// Steps v6probe reports, in order.
const V6_STEPS: [&str; 8] = [
    "text",
    "string-address",
    "getaddrinfo",
    "adapters",
    "tcp6-echo",
    "udp6-echo",
    "dual-stack",
    "tables",
];
const MCASTPROBE: Probe = Probe {
    name: "mcastprobe",
    directory: DIRECTORY,
    libraries: &["kernel32.lib", "WS2_32.Lib", "AdvAPI32.Lib"],
    modules: &["advapi32.dll", "kernel32.dll", "ws2_32.dll"],
};
/// Steps mcastprobe reports, in order.
const MC_STEPS: [&str; 9] = [
    "loopback-tcp",
    "loopback-udp",
    "defaults",
    "set-options",
    "join",
    "group-delivery",
    "leave",
    "ipv6-group",
    "raw-icmp",
];
const IOCPPROBE: Probe = Probe {
    name: "iocpprobe",
    directory: DIRECTORY,
    libraries: &["kernel32.lib", "WS2_32.Lib"],
    modules: &["kernel32.dll", "ws2_32.dll"],
};
/// Steps iocpprobe reports, in order.
const IOCP_STEPS: [&str; 9] = [
    "port-post",
    "extensions",
    "connect-ex",
    "receive-event",
    "transmit-file",
    "completion-routine",
    "cancel",
    "accept-ex",
    "disconnect-ex",
];
/// Steps diagprobe reports, in order.
const DIAG_STEPS: [&str; 7] = [
    "icmp-echo",
    "icmp-unanswered",
    "tcp-listener",
    "udp-table",
    "tcp-connection",
    "statistics",
    "arp-table",
];
const DIRECTORY: &str = "tests/fixtures/windows-net";
/// The guest port netprobe listens on (step 9).
const GUEST_LISTEN: u16 = 7070;
const RESOLUTION: &str = "1280x720";
/// Steps netprobe reports, in order.
const STEPS: [&str; 11] = [
    "startup",
    "gethostname",
    "getaddrinfo",
    "localhost",
    "tcp-echo",
    "nonblocking",
    "reset",
    "udp-echo",
    "server",
    "wsapoll",
    "cleanup",
];
/// Steps dnsprobe reports, in order.
const NAME_STEPS: [&str; 11] = [
    "adapters",
    "overflow",
    "interfaces",
    "parameters",
    "notify",
    "a-cname",
    "mx-srv",
    "txt-tcp",
    "negative",
    "async",
    "local-rules",
];

/// Steps httpprobe reports, in order.
/// Steps inetprobe reports, in order.
const INET_STEPS: [&str; 14] = [
    "open-url",
    "chunked",
    "redirect",
    "post",
    "cookies",
    "not-found",
    "urls",
    "connected",
    "async",
    "secure-failure",
    "decoding",
    "digest-auth",
    "url-cache",
    "ftp",
];
const HTTP_STEPS: [&str; 19] = [
    "get",
    "chunked",
    "redirect",
    "post",
    "cookies",
    "not-found",
    "basic-auth",
    "async",
    "urls",
    "secure-failure",
    "decompression",
    "digest-auth",
    "ntlm-auth",
    "negotiate-auth",
    "proxy-auto-config",
    "https",
    "untrusted",
    "wrong-host",
    "expired",
];

/// Compiles and links one probe; the record of the build and its imports.
pub(super) fn build(run: &mut Run, target: &Path, probe: &Probe) -> Result<Value, String> {
    build_for(run, target, probe, false, false)
}
/// `build` with the hybrid C runtime (P18 M0d), as .NET's native binaries
/// are linked: compiled for the static CRT (/MT), linked with the static
/// vcruntime and C startup and the Universal CRT's import library, so the
/// program imports the `api-ms-win-crt-*` contracts and starts at
/// `mainCRTStartup`.
pub(super) fn build_hybrid_crt(
    run: &mut Run,
    target: &Path,
    probe: &Probe,
) -> Result<Value, String> {
    build_for(run, target, probe, false, true)
}
/// uild for a 32-bit (x86) probe (P17): the x86-hosted-on-x64 compiler
/// and linker, the SDK's x86 import libraries, no SafeSEH table (the probe
/// registers its own SEH handler) and the classic 0x400000 base.
pub(super) fn build_x86(run: &mut Run, target: &Path, probe: &Probe) -> Result<Value, String> {
    build_for(run, target, probe, true, false)
}
fn build_for(
    run: &mut Run,
    target: &Path,
    probe: &Probe,
    x86: bool,
    crt: bool,
) -> Result<Value, String> {
    use crate::windows::native_path;
    let (tools, architecture) = if x86 {
        ("Hostx64/x86", "x86")
    } else {
        ("Hostx64/x64", "x64")
    };
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    let profile = crate::windows::build::toolchain(&run.root)?;
    let msvc = profile["msvc_root"].as_str().ok_or("MSVC root")?.to_owned();
    let sdk = profile["sdk_root"].as_str().ok_or("SDK root")?.to_owned();
    let version = profile["sdk_version"]
        .as_str()
        .ok_or("SDK version")?
        .to_owned();
    let libraries = format!("{sdk}/Lib/{version}/um/{architecture}");
    for library in probe.libraries {
        let pinned = profile["files"]
            .as_array()
            .ok_or("toolchain files")?
            .iter()
            .any(|f| f["path"].as_str() == Some(&format!("{libraries}/{library}")));
        if !pinned {
            return Err(format!("Windows toolchain does not pin {library}"));
        }
    }
    let crt_libraries: Vec<String> = if crt {
        [
            format!("{sdk}/Lib/{version}/ucrt/{architecture}/ucrt.lib"),
            format!("{msvc}/lib/{architecture}/libvcruntime.lib"),
            format!("{msvc}/lib/{architecture}/libcmt.lib"),
        ]
        .into()
    } else {
        Vec::new()
    };
    for library in &crt_libraries {
        let pinned = profile["files"]
            .as_array()
            .ok_or("toolchain files")?
            .iter()
            .any(|f| f["path"].as_str() == Some(library));
        if !pinned {
            return Err(format!("Windows toolchain does not pin {library}"));
        }
    }
    let name = probe.name;
    let source = format!("{}/{name}.c", probe.directory);
    let object = target.join(format!("{name}.obj"));
    let exe = target.join(format!("{name}.exe"));
    let map = target.join(format!("{name}.map"));
    let mut compile: Vec<String> = [
        "/nologo",
        "/c",
        "/TC",
        "/O1",
        "/GS-",
        if crt { "/MT" } else { "/Zl" },
        "/W4",
        "/WX",
        "/X",
        "/Brepro",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    compile.push(format!("/I{msvc}/include"));
    for directory in ["um", "shared", "ucrt"] {
        compile.push(format!("/I{sdk}/Include/{version}/{directory}"));
    }
    compile.push(format!("/Fo{}", native_path(&object)?));
    compile.push(native_path(&run.root.join(&source))?);
    run.required(
        &format!("{msvc}/bin/{tools}/cl.exe"),
        &compile.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
    let mut link: Vec<String> = [
        "/NOLOGO",
        // Every library is named: a hybrid-CRT probe gets ucrt.lib (not the
        // static UCRT), libvcruntime, libcmt and kernel32.
        "/NODEFAULTLIB",
        "/DYNAMICBASE",
        "/NXCOMPAT",
        "/Brepro",
        "/DEBUG:NONE",
        "/INCREMENTAL:NO",
        "/MANIFEST:NO",
        "/OPT:REF",
        "/OPT:ICF",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    if !crt {
        link.push("/ENTRY:entry".into());
    }
    let machine: &[&str] = if x86 {
        &[
            "/SUBSYSTEM:CONSOLE,6.01",
            "/MACHINE:X86",
            "/SAFESEH:NO",
            "/BASE:0x400000",
        ]
    } else {
        &[
            "/SUBSYSTEM:CONSOLE,6.02",
            "/MACHINE:X64",
            "/BASE:0x140000000",
        ]
    };
    link.extend(machine.iter().map(|flag| (*flag).to_owned()));
    link.push(format!("/OUT:{}", native_path(&exe)?));
    link.push(format!("/MAP:{}", native_path(&map)?));
    link.push(native_path(&object)?);
    for library in probe.libraries {
        link.push(format!("{libraries}/{library}"));
    }
    link.extend(crt_libraries.iter().cloned());
    run.required(
        &format!("{msvc}/bin/{tools}/link.exe"),
        &link.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
    let pe = crate::windows::pe::inspect(&exe)?;
    let mut modules: Vec<String> = pe["imports"]
        .as_array()
        .ok_or("probe imports")?
        .iter()
        .filter_map(|m| m["module"].as_str().map(str::to_ascii_lowercase))
        .collect();
    modules.sort();
    if modules != probe.modules {
        return Err(format!(
            "{name} imports {modules:?}; expected {:?}",
            probe.modules
        ));
    }
    Ok(
        json!({"name":name,"source":source,"source_sha256":file_hash(&run.root.join(&source))?,
        "path":relative(&run.root,&exe)?,"sha256":file_hash(&exe)?,"imports":pe["imports"],
        "link_map":{"path":relative(&run.root,&map)?,"sha256":file_hash(&map)?},
        "machine":architecture,"runtime":if crt {"the hybrid CRT: /MT, libcmt and libvcruntime static, the Universal CRT from ucrt.lib (api-ms-win-crt-*); /SUBSYSTEM:CONSOLE"} else {"Windows SDK C; no CRT; /SUBSYSTEM:CONSOLE"}}),
    )
}

/// The desktop image, which must ship the `required` DLLs, with the probes
/// installed at /system/bin, staged as the `variant` image.
pub(super) fn stage(
    run: &mut Run,
    image: &str,
    fixtures: &[Value],
    target: &Path,
    variant: &str,
    required: &[&str],
) -> Result<String, String> {
    stage_with(run, image, fixtures, &[], target, variant, required)
}
/// `stage`, plus data files the probes read, each installed as
/// `/system/bin/<name>`.
pub(super) fn stage_with(
    run: &mut Run,
    image: &str,
    fixtures: &[Value],
    data: &[(&str, Vec<u8>)],
    target: &Path,
    variant: &str,
    required: &[&str],
) -> Result<String, String> {
    let (base, _) = crate::image::validate_staged(&run.root, image)?;
    let production = fs::read(base.join("esp/Door/initfs.tar")).map_err(|e| e.to_string())?;
    let archive =
        door_initfs::Archive::parse(&production).map_err(|e| format!("desktop initfs {e:?}"))?;
    for dll in required {
        if archive.find(dll).is_none() {
            return Err(format!("the desktop image does not ship {dll}; rebuild it"));
        }
    }
    let mut manifest = String::from_utf8(
        archive
            .find("system/install.manifest")
            .ok_or("desktop initfs lacks system/install.manifest")?
            .data
            .to_vec(),
    )
    .map_err(|e| e.to_string())?;
    if !manifest.ends_with('\n') {
        manifest.push('\n');
    }
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for fixture in fixtures {
        let name = fixture["name"].as_str().ok_or("probe name")?;
        manifest.push_str(&format!("/system/bin/{name}.exe seed/{name}.exe\n"));
        let exe = fs::read(paths::existing(
            &run.root,
            fixture["path"].as_str().ok_or("probe path")?,
        )?)
        .map_err(|e| e.to_string())?;
        files.push((format!("seed/{name}.exe"), exe));
    }
    for (name, bytes) in data {
        manifest.push_str(&format!("/system/bin/{name} seed/{name}\n"));
        files.push((format!("seed/{name}"), bytes.clone()));
    }
    files.push(("system/install.manifest".into(), manifest.into_bytes()));
    let edits: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    let initfs = target.join(format!("initfs-{variant}.tar"));
    fs::write(
        &initfs,
        crate::initfs::with_files(&production, &edits, &["system/install.manifest"])?,
    )
    .map_err(|e| e.to_string())?;
    session::stage_desktop_variant(
        run,
        &base.join("esp/EFI/BOOT/BOOTX64.EFI"),
        &base.join("esp/Door/kernel.elf"),
        &initfs,
        variant,
    )
}

/// Host endpoints the guest reaches at 10.0.2.2, served until `stop`.
struct Endpoints {
    tcp: u16,
    udp: u16,
    reset: u16,
    forward: u16,
    stop: Arc<AtomicBool>,
    log: Arc<std::sync::Mutex<Vec<String>>>,
}
fn free_port() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    Ok(listener.local_addr().map_err(|e| e.to_string())?.port())
}
fn serve() -> Result<Endpoints, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let log = Arc::new(std::sync::Mutex::new(Vec::new()));
    let tcp = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    tcp.set_nonblocking(true).map_err(|e| e.to_string())?;
    let udp = UdpSocket::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let reset = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    reset.set_nonblocking(true).map_err(|e| e.to_string())?;
    udp.set_read_timeout(Some(Duration::from_millis(200)))
        .map_err(|e| e.to_string())?;
    let endpoints = Endpoints {
        tcp: tcp.local_addr().map_err(|e| e.to_string())?.port(),
        udp: udp.local_addr().map_err(|e| e.to_string())?.port(),
        reset: reset.local_addr().map_err(|e| e.to_string())?.port(),
        forward: free_port()?,
        stop: stop.clone(),
        log: log.clone(),
    };
    // TCP echo: every connection echoes until the peer half-closes, then closes.
    let (tcp_stop, tcp_log) = (stop.clone(), log.clone());
    thread::spawn(move || {
        while !tcp_stop.load(Ordering::Relaxed) {
            match tcp.accept() {
                Ok((mut stream, peer)) => {
                    let log = tcp_log.clone();
                    thread::spawn(move || {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
                        let mut total = 0;
                        let mut buffer = [0u8; 4096];
                        while let Ok(n) = stream.read(&mut buffer) {
                            if n == 0 || stream.write_all(&buffer[..n]).is_err() {
                                break;
                            }
                            total += n;
                        }
                        let _ = stream.shutdown(Shutdown::Both);
                        if let Ok(mut log) = log.lock() {
                            log.push(format!("tcp {peer} echoed {total}"));
                        }
                    });
                }
                Err(_) => thread::sleep(Duration::from_millis(20)),
            }
        }
    });
    // Accepts, waits for the client's byte, then closes without reading
    // it: Windows sends RST for a socket closed with unread data.
    let (reset_stop, reset_log) = (stop.clone(), log.clone());
    thread::spawn(move || {
        while !reset_stop.load(Ordering::Relaxed) {
            match reset.accept() {
                Ok((stream, peer)) => {
                    let _ = stream.set_nonblocking(false);
                    let mut waited = 0;
                    let mut peek = [0u8; 1];
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
                    while waited < 50 && stream.peek(&mut peek).map_or(true, |n| n == 0) {
                        waited += 1;
                    }
                    drop(stream);
                    if let Ok(mut log) = reset_log.lock() {
                        log.push(format!("reset {peer} after {waited} polls"));
                    }
                }
                Err(_) => thread::sleep(Duration::from_millis(20)),
            }
        }
    });
    let (udp_stop, udp_log) = (stop, log);
    thread::spawn(move || {
        let mut buffer = [0u8; 2048];
        while !udp_stop.load(Ordering::Relaxed) {
            if let Ok((n, peer)) = udp.recv_from(&mut buffer) {
                let _ = udp.send_to(&buffer[..n], peer);
                if let Ok(mut log) = udp_log.lock() {
                    log.push(format!("udp {peer} echoed {n}"));
                }
            }
        }
    });
    Ok(endpoints)
}
/// Step 9 from the host side: connect through the forwarded port, send
/// "ping", read "pong", close.
/// IPv6 TCP and UDP echo servers on this host's `[::1]`, which QEMU's user
/// network presents to the guest as `[fec0::2]`. Serve until `stop`.
struct Endpoints6 {
    tcp: u16,
    udp: u16,
    stop: Arc<AtomicBool>,
}
impl Drop for Endpoints6 {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
fn serve6() -> Result<Endpoints6, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let tcp = TcpListener::bind("[::1]:0").map_err(|e| format!("IPv6 loopback: {e}"))?;
    tcp.set_nonblocking(true).map_err(|e| e.to_string())?;
    let udp = UdpSocket::bind("[::1]:0").map_err(|e| e.to_string())?;
    udp.set_read_timeout(Some(Duration::from_millis(200)))
        .map_err(|e| e.to_string())?;
    let endpoints = Endpoints6 {
        tcp: tcp.local_addr().map_err(|e| e.to_string())?.port(),
        udp: udp.local_addr().map_err(|e| e.to_string())?.port(),
        stop: stop.clone(),
    };
    let tcp_stop = stop.clone();
    thread::spawn(move || {
        while !tcp_stop.load(Ordering::Relaxed) {
            match tcp.accept() {
                Ok((mut stream, _)) => {
                    thread::spawn(move || {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
                        let mut buffer = [0u8; 4096];
                        while let Ok(n) = stream.read(&mut buffer) {
                            if n == 0 || stream.write_all(&buffer[..n]).is_err() {
                                break;
                            }
                        }
                        let _ = stream.shutdown(Shutdown::Both);
                    });
                }
                Err(_) => thread::sleep(Duration::from_millis(20)),
            }
        }
    });
    thread::spawn(move || {
        let mut buffer = [0u8; 2048];
        while !stop.load(Ordering::Relaxed) {
            if let Ok((n, peer)) = udp.recv_from(&mut buffer) {
                let _ = udp.send_to(&buffer[..n], peer);
            }
        }
    });
    Ok(endpoints)
}
fn ping(port: u16, seconds: u64) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        // QEMU accepts on the forwarded port before the guest does; a reset
        // means the guest was not listening yet.
        let attempt = (|| -> Result<String, String> {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .map_err(|e| e.to_string())?;
            stream.write_all(b"ping").map_err(|e| e.to_string())?;
            let mut answer = [0u8; 4];
            stream.read_exact(&mut answer).map_err(|e| e.to_string())?;
            stream.shutdown(Shutdown::Both).map_err(|e| e.to_string())?;
            Ok(String::from_utf8_lossy(&answer).into_owned())
        })();
        match attempt {
            Ok(answer) => return Ok(answer),
            Err(e) if Instant::now() >= deadline => {
                return Err(format!(
                    "host could not reach the guest's listening socket: {e}"
                ));
            }
            Err(_) => thread::sleep(Duration::from_millis(500)),
        }
    }
}
/// Waits for `needle` in the decoded console stream.
pub(super) fn console_after(
    run: &Run,
    record: &Value,
    before: usize,
    needle: &str,
    seconds: u64,
) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let text = session::transcript(&run.root, record)?;
        if let Some(rest) = text.get(before.min(text.len())..)
            && rest.contains(needle)
        {
            return Ok(rest.to_owned());
        }
        if serial(&run.root, record)?.contains("KERNEL_PANIC") {
            return Err("guest panicked; inspect serial".into());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "console output missing {needle:?}; got {:?}",
                text.get(before.min(text.len())..).unwrap_or("")
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// Types `line` in Terminal, waits for the probe's PASSED/FAILED line and
/// its exit, and checks every step reported ok.
pub(super) fn run_probe(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    fixture: &Value,
    line: &str,
    tag: &str,
    steps: &[&str],
) -> Result<Value, String> {
    run_probe_during(run, record, c, fixture, line, tag, steps, None)
}
/// What a run does once the probe printed a marker (the host connects to
/// its listener, a Security Alert is answered) while it keeps running.
type During<'a> = Option<(
    &'a str,
    &'a mut dyn FnMut(&mut qmp::Client) -> Result<(), String>,
)>;
/// The Doors Security Alert door.network raised for `exe` (a program that
/// listens where no firewall rule names it): Allow access, approved through
/// User Account Control, or Cancel (block rules). Waits until Init stored
/// the rules and handed the policy over again; answers the alert's line.
fn firewall_alert(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    offset: usize,
    exe: &str,
    allow: bool,
) -> Result<String, String> {
    let line = alert_shown(run, record, offset, exe)?;
    answer_alert(run, record, c, allow)?;
    Ok(line)
}
/// Waits for the alert for `exe` to open: its serial line.
fn alert_shown(run: &Run, record: &Value, offset: usize, exe: &str) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let line = loop {
        let text = serial(&run.root, record)?;
        if let Some(l) = text.get(offset..).unwrap_or("").lines().find(|l| {
            l.contains("DOOR_P15 FIREWALL ALERT program=")
                && l.to_ascii_lowercase().contains(&exe.to_ascii_lowercase())
        }) {
            break l.trim_end().to_owned();
        }
        if Instant::now() >= deadline {
            return Err(format!("no Doors Security Alert for {exe}"));
        }
        thread::sleep(Duration::from_millis(100));
    };
    thread::sleep(Duration::from_millis(700));
    Ok(line)
}
/// Allow access (then Yes on User Account Control) or Cancel the open
/// alert; returns once Init handed the new rules to door.network.
fn answer_alert(run: &Run, record: &Value, c: &mut qmp::Client, allow: bool) -> Result<(), String> {
    let before = serial(&run.root, record)?.len();
    if allow {
        key(c, &["alt", "a"])?;
        super::win32::wait_after(
            run,
            record,
            before,
            "DOOR_P16 UAC PROMPT program=Doors Defender Firewall",
            20,
        )?;
        thread::sleep(Duration::from_millis(800));
        key(c, &["alt", "y"])?;
    } else {
        key(c, &["esc"])?;
    }
    super::win32::wait_after(
        run,
        record,
        before,
        "DOOR_P15 FIREWALL STORED kind=2 ok=1",
        20,
    )?;
    super::win32::wait_after(run, record, before, "DOOR_P15 FIREWALL RELOAD rules=", 20)?;
    thread::sleep(Duration::from_millis(500));
    Ok(())
}
/// Win+X > Terminal (Admin) and Yes on User Account Control: an elevated
/// Terminal on a Doorsh tab. The quick-link menu opens on its first item;
/// "Terminal (Admin)" is the eleventh. Answers the prompt's serial line.
pub(super) fn open_admin_terminal(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
) -> Result<String, String> {
    let before = serial(&run.root, record)?.len();
    key(c, &["meta_l", "x"])?;
    thread::sleep(Duration::from_millis(700));
    for _ in 0..10 {
        key(c, &["down"])?;
        thread::sleep(Duration::from_millis(80));
    }
    key(c, &["ret"])?;
    super::win32::wait_after(run, record, before, "DOOR_P16 UAC PROMPT", 20)?;
    let uac = serial(&run.root, record)?
        .get(before..)
        .unwrap_or("")
        .lines()
        .find(|l| l.contains("DOOR_P16 UAC PROMPT"))
        .unwrap_or("")
        .to_owned();
    if !uac.contains("program=Terminal publisher=Macrohard Doors") {
        return Err(format!("Terminal (Admin) prompt: {uac}"));
    }
    thread::sleep(Duration::from_millis(800));
    key(c, &["alt", "y"])?;
    super::win32::wait_after(
        run,
        record,
        before,
        "DOOR_P16 ELEVATED TERMINAL launched",
        20,
    )?;
    thread::sleep(Duration::from_secs(2));
    Ok(uac)
}
/// Types `line` in a Doorsh tab and waits until it is idle again: the
/// lines it printed, which must contain each of `needles`.
pub(super) fn doorsh(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    line: &str,
    needles: &[&str],
) -> Result<Vec<String>, String> {
    let before = serial(&run.root, record)?.len();
    type_line(c, line)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let printed: Vec<String> = loop {
        let text = serial(&run.root, record)?;
        let tail = text.get(before.min(text.len())..).unwrap_or("");
        if tail.contains("DOOR_P16 DSH IDLE") {
            break tail
                .lines()
                .take_while(|l| !l.contains("DOOR_P16 DSH IDLE"))
                .filter_map(|l| {
                    l.split_once("DOOR_P16 DSH TEXT ")
                        .map(|(_, t)| t.trim_end().to_owned())
                })
                .collect();
        }
        if Instant::now() >= deadline {
            return Err(format!("{line}: Doorsh did not finish"));
        }
        thread::sleep(Duration::from_millis(100));
    };
    let joined = printed.join("\n");
    for needle in needles {
        if !joined.contains(needle) {
            return Err(format!("{line}: expected {needle:?}, printed:\n{joined}"));
        }
    }
    Ok(printed)
}
#[allow(clippy::too_many_arguments)]
fn run_probe_during(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    fixture: &Value,
    line: &str,
    tag: &str,
    steps: &[&str],
    during: During<'_>,
) -> Result<Value, String> {
    // Type only once the shell shows its prompt again: keys sent while the
    // previous program is still exiting are lost.
    let ready = Instant::now() + Duration::from_secs(15);
    while !session::transcript(&run.root, record)?
        .trim_end()
        .ends_with('>')
        && Instant::now() < ready
    {
        thread::sleep(Duration::from_millis(100));
    }
    thread::sleep(Duration::from_millis(300));
    let before = session::transcript(&run.root, record)?.len();
    let offset = serial(&run.root, record)?.len();
    type_line(c, line)?;
    let pid = super::win32::wait_launch(
        run,
        record,
        offset,
        fixture["sha256"].as_str().ok_or("probe hash")?,
        line,
    )?;
    if let Some((marker, act)) = during {
        console_after(run, record, before, marker, 60)?;
        act(c)?;
    }
    let passed = format!("{tag} PASSED");
    let output = console_after(run, record, before, &passed, 120).or_else(|e| {
        console_after(run, record, before, &format!("{tag} FAILED"), 1)
            .map(|o| format!("{o}\n({e})"))
    })?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let exit = loop {
        if let Some(exit) = super::win32::processes(&serial(&run.root, record)?, offset)
            .remove(&pid)
            .and_then(|p| p.exit)
        {
            break exit;
        }
        if Instant::now() >= deadline {
            return Err(format!("{line}: pid {pid} did not exit"));
        }
        thread::sleep(Duration::from_millis(100));
    };
    let modules = super::win32::processes(&serial(&run.root, record)?, offset)
        .remove(&pid)
        .map(|p| p.modules)
        .unwrap_or_default();
    let lines: Vec<&str> = output
        .lines()
        .filter(|l| l.starts_with(&format!("{tag} ")))
        .collect();
    let mut found = Vec::new();
    for (index, name) in steps.iter().enumerate() {
        let prefix = format!("{tag} {} {name} ", index + 1);
        let line = lines
            .iter()
            .find(|l| l.starts_with(&prefix))
            .ok_or(format!(
                "{tag} step {} {name} missing from the console",
                index + 1
            ))?;
        found.push(json!({"step":index + 1,"name":name,"line":line.trim_end()}));
    }
    if exit != 0 || !output.contains(&passed) {
        return Err(format!("{tag} failed (exit {exit}):\n{}", lines.join("\n")));
    }
    Ok(json!({"command":line,"pid":pid,"exit":exit,"modules":modules,"steps":found}))
}

/// The console line of `run_probe`'s step `step` (1-based).
fn step_line(probe: &Value, step: usize) -> String {
    probe["steps"][step - 1]["line"]
        .as_str()
        .unwrap_or("")
        .to_owned()
}
/// The host endpoints alone, for running the probes on Windows itself.
pub(super) fn serve_reference(run: &mut Run) -> Result<(), String> {
    let endpoints = serve()?;
    let endpoints6 = serve6()?;
    let dns = super::p15_names::serve()?;
    let web = super::p15_http::serve()?;
    let tls = super::p15_tls::serve()?;
    let ftp_root = run.generated_directory(&format!("out/builds/{}/ftp-root", run.id))?;
    let ftp = super::p15_ftp::serve(&ftp_root)?;
    println!("CONNECT-A host endpoints on 127.0.0.1 for 300 s:");
    println!(
        "  v6probe.exe ::1 {} {} 127.0.0.1 {}",
        endpoints6.tcp, endpoints6.udp, endpoints.tcp
    );
    println!(
        "  netprobe.exe 127.0.0.1 {} {} {GUEST_LISTEN} {}",
        endpoints.tcp, endpoints.udp, endpoints.reset
    );
    println!("  dnsprobe.exe {} 127.0.0.1", dns.port);
    println!(
        "  httpprobe.exe {} 127.0.0.1 {} {} {}",
        web.port, tls.good, tls.wrong_host, tls.expired
    );
    println!("  inetprobe.exe {} 127.0.0.1 {}", web.port, ftp.port);
    println!("  diagprobe.exe 127.0.0.1 {}", endpoints.tcp);
    println!("  mcastprobe.exe 127.0.0.1");
    println!(
        "  iocpprobe.exe 127.0.0.1 {} <listen port> 127.0.0.1",
        endpoints.tcp
    );
    thread::sleep(Duration::from_secs(300));
    endpoints.stop.store(true, Ordering::Relaxed);
    run.check("p15-connect-serve", json!({"ftp_log":ftp.log.lock().map(|l| l.clone()).unwrap_or_default(),"tcp":endpoints.tcp,"udp":endpoints.udp,"reset":endpoints.reset,"dns":dns.port,"web":web.port,
        "web_log":web.log.lock().map(|l| l.clone()).unwrap_or_default(),"dns_log":dns.log.lock().map(|l| l.clone()).unwrap_or_default(),
        "tls":{"ports":[tls.good,tls.wrong_host,tls.expired],"log":tls.log.lock().map(|l| l.clone()).unwrap_or_default()}}));
    Ok(())
}

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/connect-a", run.id))?;
    let fixture = build(run, &target, &NETPROBE)?;
    let names_fixture = build(run, &target, &DNSPROBE)?;
    let http_fixture = build(run, &target, &HTTPPROBE)?;
    let inet_fixture = build(run, &target, &INETPROBE)?;
    let diag_fixture = build(run, &target, &DIAGPROBE)?;
    let iocp_fixture = build(run, &target, &IOCPPROBE)?;
    let v6_fixture = build(run, &target, &V6PROBE)?;
    let mcast_fixture = build(run, &target, &MCASTPROBE)?;
    let required = [
        "windows/ws2_32.dll",
        "windows/iphlpapi.dll",
        "windows/dnsapi.dll",
        "windows/winhttp.dll",
        "windows/wininet.dll",
    ];
    let staged = stage(
        run,
        image,
        &[
            fixture.clone(),
            names_fixture.clone(),
            http_fixture.clone(),
            inet_fixture.clone(),
            diag_fixture.clone(),
            iocp_fixture.clone(),
            v6_fixture.clone(),
            mcast_fixture.clone(),
        ],
        &target,
        "desktop-connect",
        &required,
    )?;
    let endpoints = serve()?;
    let endpoints6 = serve6()?;
    let dns = super::p15_names::serve()?;
    let web = super::p15_http::serve()?;
    let tls = super::p15_tls::serve()?;
    let ftp_root = run.generated_directory(&format!("out/builds/{}/ftp-root", run.id))?;
    let ftp = super::p15_ftp::serve(&ftp_root)?;
    let disk = super::win32::fresh_disk(run, "connect-a")?;
    let forwarding = crate::emulator::forward_tcp(&[(endpoints.forward, GUEST_LISTEN)]);
    let launched = session::launch_desktop_accelerated(run, &staged, &disk, false, RESOLUTION);
    drop(forwarding);
    let mut record = launched?;
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        thread::sleep(Duration::from_secs(2));
        super::win32::open_terminal(run, &record, &mut c)?;
        thread::sleep(Duration::from_secs(1));
        let before = session::transcript(&run.root, &record)?.len();
        let offset = serial(&run.root, &record)?.len();
        let line = format!(
            "netprobe.exe 10.0.2.2 {} {} {GUEST_LISTEN} {}",
            endpoints.tcp, endpoints.udp, endpoints.reset
        );
        type_line(&mut c, &line)?;
        let pid = super::win32::wait_launch(
            run,
            &record,
            offset,
            fixture["sha256"].as_str().ok_or("netprobe hash")?,
            &line,
        )?;
        console_after(run, &record, before, "CONNECT-A listening", 90)?;
        // The firewall blocks the new listener: the Doors Security Alert
        // asks, Allow access goes through User Account Control, and the
        // host's connection gets in once Init stored the allow rules.
        let netprobe_alert = alert_shown(run, &record, offset, "netprobe.exe")?;
        let frame = scene::capture_settled(run, &mut c, "connect-a-firewall-alert", &[])?;
        let alert_shot = super::win32::keep(run, &frame, "connect-a-firewall-alert")?;
        answer_alert(run, &record, &mut c, true)?;
        let answer = ping(endpoints.forward, 20)?;
        let output = console_after(run, &record, before, "CONNECT-A PASSED", 60).or_else(|e| {
            console_after(run, &record, before, "CONNECT-A FAILED", 1)
                .map(|o| format!("{o}\n({e})"))
        })?;
        let deadline = Instant::now() + Duration::from_secs(30);
        let exit = loop {
            if let Some(exit) = super::win32::processes(&serial(&run.root, &record)?, offset)
                .remove(&pid)
                .and_then(|p| p.exit)
            {
                break exit;
            }
            if Instant::now() >= deadline {
                return Err(format!("netprobe (pid {pid}) did not exit"));
            }
            thread::sleep(Duration::from_millis(100));
        };
        let modules = super::win32::processes(&serial(&run.root, &record)?, offset)
            .remove(&pid)
            .map(|p| p.modules)
            .unwrap_or_default();
        let lines: Vec<&str> = output
            .lines()
            .filter(|l| l.starts_with("CONNECT-A "))
            .collect();
        let mut steps = Vec::new();
        for (index, name) in STEPS.iter().enumerate() {
            let prefix = format!("CONNECT-A {} {name} ", index + 1);
            let found = lines
                .iter()
                .find(|l| l.starts_with(&prefix))
                .ok_or(format!(
                    "step {} {name} missing from the console",
                    index + 1
                ))?;
            steps.push(json!({"step":index + 1,"name":name,"line":found.trim_end()}));
        }
        if exit != 0 || !output.contains("CONNECT-A PASSED") || answer != "pong" {
            return Err(format!(
                "CONNECT-A failed (exit {exit}, host read {answer:?}):\n{}",
                lines.join("\n")
            ));
        }
        // Names: iphlpapi and dnsapi against the test zone.
        let before = session::transcript(&run.root, &record)?.len();
        let offset = serial(&run.root, &record)?.len();
        let names_line = format!("dnsprobe.exe {}", dns.port);
        type_line(&mut c, &names_line)?;
        let names_pid = super::win32::wait_launch(
            run,
            &record,
            offset,
            names_fixture["sha256"].as_str().ok_or("dnsprobe hash")?,
            &names_line,
        )?;
        let names_output =
            console_after(run, &record, before, "NAMES-A PASSED", 90).or_else(|e| {
                console_after(run, &record, before, "NAMES-A FAILED", 1)
                    .map(|o| format!("{o}\n({e})"))
            })?;
        let deadline = Instant::now() + Duration::from_secs(30);
        let names_exit = loop {
            if let Some(exit) = super::win32::processes(&serial(&run.root, &record)?, offset)
                .remove(&names_pid)
                .and_then(|p| p.exit)
            {
                break exit;
            }
            if Instant::now() >= deadline {
                return Err(format!("dnsprobe (pid {names_pid}) did not exit"));
            }
            thread::sleep(Duration::from_millis(100));
        };
        let names_modules = super::win32::processes(&serial(&run.root, &record)?, offset)
            .remove(&names_pid)
            .map(|p| p.modules)
            .unwrap_or_default();
        let names_lines: Vec<&str> = names_output
            .lines()
            .filter(|l| l.starts_with("NAMES-A "))
            .collect();
        let mut names_steps = Vec::new();
        for (index, name) in NAME_STEPS.iter().enumerate() {
            let prefix = format!("NAMES-A {} {name} ", index + 1);
            let found = names_lines
                .iter()
                .find(|l| l.starts_with(&prefix))
                .ok_or(format!(
                    "names step {} {name} missing from the console",
                    index + 1
                ))?;
            names_steps.push(json!({"step":index + 1,"name":name,"line":found.trim_end()}));
        }
        if names_exit != 0 || !names_output.contains("NAMES-A PASSED") {
            return Err(format!(
                "CONNECT-A names failed (exit {names_exit}):\n{}",
                names_lines.join("\n")
            ));
        }
        // WinHTTP against the test web server.
        let http = run_probe(
            run,
            &record,
            &mut c,
            &http_fixture,
            &format!(
                "httpprobe.exe {} 10.0.2.2 {} {} {}",
                web.port, tls.good, tls.wrong_host, tls.expired
            ),
            "HTTP-A",
            &HTTP_STEPS,
        )?;
        let inet = run_probe(
            run,
            &record,
            &mut c,
            &inet_fixture,
            &format!("inetprobe.exe {} 10.0.2.2 {}", web.port, ftp.port),
            "INET-A",
            &INET_STEPS,
        )?;
        // ICMP echo and the connection tables (iphlpapi), then ping and
        // netstat in Terminal over the same stack. Its listener (step 3)
        // raises a Security Alert; Cancel blocks the program.
        let diag_offset = serial(&run.root, &record)?.len();
        let diag = run_probe(
            run,
            &record,
            &mut c,
            &diag_fixture,
            &format!("diagprobe.exe 10.0.2.2 {}", endpoints.tcp),
            "DIAG-A",
            &DIAG_STEPS,
        )?;
        let diag_alert = firewall_alert(run, &record, &mut c, diag_offset, "diagprobe.exe", false)?;
        // Overlapped Winsock through a completion port; the host connects
        // to its AcceptEx listener (step 8) through the forwarded port once
        // its Security Alert was allowed.
        let forward = endpoints.forward;
        let iocp_offset = serial(&run.root, &record)?.len();
        let mut iocp_answer = String::new();
        let mut iocp_alert = String::new();
        let mut serve_iocp = |c: &mut qmp::Client| -> Result<(), String> {
            iocp_alert = firewall_alert(run, &record, c, iocp_offset, "iocpprobe.exe", true)?;
            iocp_answer = ping(forward, 30)?;
            Ok(())
        };
        let iocp = run_probe_during(
            run,
            &record,
            &mut c,
            &iocp_fixture,
            &format!("iocpprobe.exe 10.0.2.2 {} {GUEST_LISTEN}", endpoints.tcp),
            "IOCP-A",
            &IOCP_STEPS,
            Some(("IOCP-A listening", &mut serve_iocp)),
        )?;
        if iocp_answer != "pong" {
            return Err(format!("IOCP-A answered {iocp_answer:?}"));
        }
        // IPv6: the stack autoconfigured from QEMU's router advertisement
        // (fec0::/64); the host's [::1] echo servers are [fec0::2] to the guest.
        let advertised = serial(&run.root, &record)?
            .lines()
            .find(|l| l.contains("DOOR_P15 NET IPV6 address=fec0::"))
            .map(str::to_owned)
            .ok_or("no IPv6 address from router discovery")?;
        let v6 = run_probe(
            run,
            &record,
            &mut c,
            &v6_fixture,
            &format!(
                "v6probe.exe fec0::2 {} {} 10.0.2.2 {}",
                endpoints6.tcp, endpoints6.udp, endpoints.tcp
            ),
            "V6-A",
            &V6_STEPS,
        )?;
        // Loopback, multicast, IP options; a raw ICMP socket echoes (every
        // user may open one) while raw IGMP is refused (not elevated).
        let multicast = run_probe(
            run,
            &record,
            &mut c,
            &mcast_fixture,
            "mcastprobe.exe 10.0.2.2",
            "MC-A",
            &MC_STEPS,
        )?;
        if !step_line(&multicast, 9).contains("raw-icmp ok 3") {
            return Err(format!("an unelevated raw socket: {multicast}"));
        }
        // Terminal's Doorsh tab: ping's table, one row per echo reply.
        let before = serial(&run.root, &record)?.len();
        type_line(&mut c, "ping -n 2 10.0.2.2")?;
        let deadline = Instant::now() + Duration::from_secs(30);
        let ping: Vec<String> = loop {
            let text = serial(&run.root, &record)?;
            let tail = text.get(before.min(text.len())..).unwrap_or("");
            if tail.contains("DOOR_P16 DSH IDLE") {
                break tail
                    .lines()
                    .filter_map(|l| {
                        l.split_once("DOOR_P16 DSH TEXT ")
                            .map(|(_, t)| t.trim_end().to_owned())
                    })
                    .collect();
            }
            if Instant::now() >= deadline {
                return Err("ping did not finish".into());
            }
            thread::sleep(Duration::from_millis(100));
        };
        let replies = ping
            .iter()
            .filter(|l| l.split_whitespace().nth(1) == Some("10.0.2.2") && l.ends_with("ms"))
            .count();
        if replies != 2 {
            return Err(format!("ping printed {replies} replies: {ping:?}"));
        }
        let frame = scene::capture_settled(run, &mut c, "connect-a-terminal", &[])?;
        let shot = super::win32::keep(run, &frame, "connect-a-terminal")?;
        // netsh advfirewall changes nothing without elevation.
        let unelevated = doorsh(
            run,
            &record,
            &mut c,
            "netsh advfirewall set allprofiles state off",
            &["The requested operation requires elevation (Run as administrator)."],
        )?;
        // Win+X > Terminal (Admin): User Account Control names Terminal; on
        // Alt+Y the elevated Terminal runs the probe again, where raw IGMP
        // sockets open too. Doors runs at most 13
        // processes, so this Terminal closes first.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["alt", "f4"])?;
        super::win32::wait_after(run, &record, before, "CLIENT_EXIT", 15)?;
        thread::sleep(Duration::from_millis(800));
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "x"])?;
        thread::sleep(Duration::from_millis(700));
        // The quick-link menu opens on its first item; "Terminal (Admin)" is
        // the eleventh.
        for _ in 0..10 {
            key(&mut c, &["down"])?;
            thread::sleep(Duration::from_millis(80));
        }
        key(&mut c, &["ret"])?;
        super::win32::wait_after(run, &record, before, "DOOR_P16 UAC PROMPT", 20)?;
        let uac = serial(&run.root, &record)?
            .get(before..)
            .unwrap_or("")
            .lines()
            .find(|l| l.contains("DOOR_P16 UAC PROMPT"))
            .unwrap_or("")
            .to_owned();
        if !uac.contains("program=Terminal publisher=Macrohard Doors") {
            return Err(format!("Terminal (Admin) prompt: {uac}"));
        }
        thread::sleep(Duration::from_millis(800));
        let uac_frame = scene::capture_settled(run, &mut c, "connect-a-uac-terminal", &[])?;
        let uac_shot = super::win32::keep(run, &uac_frame, "connect-a-uac-terminal")?;
        key(&mut c, &["alt", "y"])?;
        super::win32::wait_after(
            run,
            &record,
            before,
            "DOOR_P16 ELEVATED TERMINAL launched",
            20,
        )?;
        thread::sleep(Duration::from_secs(2));
        let elevated = run_probe(
            run,
            &record,
            &mut c,
            &mcast_fixture,
            "mcastprobe.exe 10.0.2.2",
            "MC-A",
            &MC_STEPS,
        )?;
        if !step_line(&elevated, 9).contains("raw-icmp ok 1003") {
            return Err(format!("an elevated raw socket: {elevated}"));
        }
        // netsh advfirewall as an administrator: the rules the Security
        // Alerts stored (allow for netprobe and iocpprobe, block for
        // diagprobe), a rule added and deleted (each change handed to
        // door.network), and the public profile.
        let mut netsh = Vec::new();
        for (line, needles) in [
            (
                "netsh advfirewall firewall show rule name=netprobe.exe",
                &[
                    "Rule Name:                            netprobe.exe",
                    "Profiles:                             Public",
                    "Action:                               Allow",
                    "Ok.",
                ][..],
            ),
            (
                "netsh advfirewall firewall show rule name=diagprobe.exe verbose",
                &[
                    "Protocol:                             UDP",
                    "Action:                               Block",
                    "Program:                              C:\\",
                ][..],
            ),
            (
                "netsh advfirewall firewall add rule name=DoorsProbe dir=in action=block protocol=tcp localport=9",
                &["Ok."][..],
            ),
            (
                "netsh advfirewall firewall delete rule name=DoorsProbe",
                &["Deleted 1 rule(s).", "Ok."][..],
            ),
            (
                "netsh advfirewall show currentprofile",
                &[
                    "Public Profile Settings:",
                    "State                                 ON",
                    "Firewall Policy                       BlockInbound,AllowOutbound",
                ][..],
            ),
        ] {
            let before = serial(&run.root, &record)?.len();
            let printed = doorsh(run, &record, &mut c, line, needles)?;
            let reloaded = serial(&run.root, &record)?
                .get(before..)
                .unwrap_or("")
                .lines()
                .find(|l| l.contains("DOOR_P15 FIREWALL RELOAD rules="))
                .map(|l| l.trim_end().to_owned());
            netsh.push(json!({"command":line,"printed":printed,"reload":reloaded}));
        }
        // Six rules from the three alerts; the added one made seven.
        if netsh[2]["reload"]
            .as_str()
            .is_none_or(|l| !l.ends_with("rules=7"))
            || netsh[3]["reload"]
                .as_str()
                .is_none_or(|l| !l.ends_with("rules=6"))
        {
            return Err(format!("netsh changes reached door.network as: {netsh:?}"));
        }
        Ok(
            json!({"winsock":{"command":line,"pid":pid,"exit":exit,"modules":modules,"steps":steps,"host_answer":answer},
            "names":{"command":names_line,"pid":names_pid,"exit":names_exit,"modules":names_modules,"steps":names_steps},
            "http":http,
            "inet":inet,
            "diag":diag,
            "overlapped":{"probe":iocp,"host_answer":iocp_answer},
            "ipv6":{"probe":v6,"router_discovery":advertised},
            "multicast":{"probe":multicast,"elevated":elevated,"uac":uac,"uac_screenshot":uac_shot},
            "firewall":{"alerts":{"netprobe":netprobe_alert,"diagprobe":diag_alert,"iocpprobe":iocp_alert},
                "alert_screenshot":alert_shot,"unelevated_netsh":unelevated,"netsh":netsh},
            "ping":ping,
            "screenshot":shot}),
        )
    })();
    endpoints.stop.store(true, Ordering::Relaxed);
    let host_log = endpoints.log.lock().map(|l| l.clone()).unwrap_or_default();
    let dns_log = dns.log.lock().map(|l| l.clone()).unwrap_or_default();
    let web_log = web.log.lock().map(|l| l.clone()).unwrap_or_default();
    let tls_log = tls.log.lock().map(|l| l.clone()).unwrap_or_default();
    let details = super::win32::quit_on_error(run, &record, result, "connect-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    run.check("p15-connect-a", json!({"fixtures":[fixture,names_fixture],"image":staged,"dns":{"port":dns.port,"zone":"doors.test","log":dns_log},"web":{"port":web.port,"log":web_log},"tls":{"server":"TLS 1.3, X25519, TLS_AES_128_GCM_SHA256, self-signed ECDSA P-256 (valid, wrong host, expired)","ports":[tls.good,tls.wrong_host,tls.expired],"log":tls_log},"ftp":{"port":ftp.port,"log":ftp.log.lock().map(|l| l.clone()).unwrap_or_default()},"endpoints":{"guest_view":"10.0.2.2","tcp_echo":endpoints.tcp,"udp_echo":endpoints.udp,"reset":endpoints.reset,"forwarded":{"host":endpoints.forward,"guest":GUEST_LISTEN}},
        "host_log":host_log,"run":details,"session":record,
        "scope":"An MSVC-built Windows console program using ws2_32 (Winsock 2.2, IPv4) in the guest against host endpoints through QEMU user networking"}));
    Ok(())
}

//! cmd.exe's (and Doorsh's) view of this Doors session: Windows paths over
//! authenticated Storage (C:\ is `/compat`, C:\Users is `/users`, and the
//! user's profile folder appears as C:\Users\<name>), file times, the
//! registry hives, the network service's adapter and the clock. Every call
//! runs with the signed-in user's own authority.
use alloc::{format, string::String, vec::Vec};
use door_service_client::{
    Disposition, Error, File, READ, RegistryRoot, RegistryValue, RegistryView, SHARE_ALL, Storage,
    WRITE,
};
use door_service_protocol::files::{LARGE_REPLACE_BYTES, STAGE_SECTION_BYTES};
use door_service_protocol::{CHUNK_BYTES, Code};
use door_shells::host::{
    Adapter, Entry, FsError, Host, LocalTime, Meta, RegValue, SystemFacts, Volume, attr,
};

pub(super) struct SessionHost<'a> {
    pub storage: &'a mut Storage,
    pub registry: u64,
    pub uid: u32,
    pub administrator: bool,
    pub user: String,
    pub computer: String,
    /// Minutes east of UTC (the Date & time zone).
    pub offset: i64,
    /// Text for the clipboard, taken by Terminal after the step.
    pub clipboard: Option<String>,
    pub processors: u32,
    /// The session runs as administrator (approved by User Account Control).
    pub elevated: bool,
    /// The working-folder authority (StartService hands it to Init).
    pub directory: u64,
}
/// The Service Control Manager's registry database.
const SERVICES_KEY: &str = "HKLM\\SYSTEM\\CurrentControlSet\\Services";
/// Doors' own services, which have no registry key.
const BUILTIN_SERVICES: [(&str, &str, &str); 4] = door_service_protocol::scm::BUILTIN;
fn fs_error(e: Error) -> FsError {
    match e {
        Error::Service(Code::NotFound) => FsError::NotFound,
        Error::Service(Code::Denied) => FsError::Denied,
        Error::Service(Code::Exists) => FsError::Exists,
        Error::Service(Code::SharingViolation | Code::DeletePending) => FsError::InUse,
        Error::Service(Code::Resources) => FsError::Full,
        Error::Service(Code::InvalidRequest) => FsError::Invalid,
        _ => FsError::Unsupported,
    }
}
impl SessionHost<'_> {
    fn profile(&self) -> String {
        format!("/users/{}", self.uid)
    }
    /// A service's status from the manager and its registry configuration.
    /// Whether `name` depending on `on` closes a cycle (ERROR_CIRCULAR_
    /// DEPENDENCY, as CreateService and ChangeServiceConfig answer).
    fn cycle(&mut self, name: &str, on: &[String]) -> bool {
        let mut pending: Vec<String> = on.to_vec();
        let mut seen: Vec<String> = Vec::new();
        while let Some(d) = pending.pop() {
            if d.eq_ignore_ascii_case(name) {
                return true;
            }
            if seen.iter().any(|s| s.eq_ignore_ascii_case(&d)) || seen.len() > 64 {
                continue;
            }
            seen.push(d.clone());
            let key = format!("{SERVICES_KEY}\\{d}");
            if let Ok(values) = door_shells::host::Host::reg_values(self, &key) {
                for (n, v) in values {
                    if let ("dependonservice", RegValue::MultiSz(list)) =
                        (n.to_ascii_lowercase().as_str(), v)
                    {
                        pending.extend(list);
                    }
                }
            }
        }
        false
    }
    fn service_info(
        &mut self,
        name: &str,
        status: door_service_protocol::scm::Status,
    ) -> door_shells::host::Service {
        let mut s = door_shells::host::Service {
            name: String::from(name),
            display: String::from(name),
            state: status.state,
            pid: status.pid,
            accepted: status.accepted,
            exit_code: status.exit_code,
            kind: if status.kind == 0 { 0x10 } else { status.kind },
            start: 3,
            ..Default::default()
        };
        if let Some((n, display, description)) = BUILTIN_SERVICES
            .iter()
            .find(|(n, ..)| n.eq_ignore_ascii_case(name))
        {
            s.name = String::from(*n);
            s.display = String::from(*display);
            s.description = String::from(*description);
            s.start = 2;
            s.binary = format!("C:\\Windows\\System32\\{}.exe", n.to_ascii_lowercase());
            return s;
        }
        if let Ok(values) =
            door_shells::host::Host::reg_values(self, &format!("{SERVICES_KEY}\\{name}"))
        {
            for (n, v) in values {
                match (n.to_ascii_lowercase().as_str(), v) {
                    ("displayname", RegValue::Sz(t) | RegValue::ExpandSz(t)) => s.display = t,
                    ("description", RegValue::Sz(t) | RegValue::ExpandSz(t)) => s.description = t,
                    ("imagepath", RegValue::Sz(t) | RegValue::ExpandSz(t)) => s.binary = t,
                    ("start", RegValue::Dword(d)) => s.start = d,
                    ("type", RegValue::Dword(d)) => s.kind = d,
                    ("dependonservice", RegValue::MultiSz(list)) => s.dependencies = list,
                    _ => (),
                }
            }
        }
        s
    }
    /// One Task Scheduler call by task name, its Windows error.
    fn task_call(
        &mut self,
        name: &str,
        f: impl FnOnce(&mut door_service_client::Tasks, &str) -> door_service_client::Result<()>,
    ) -> Result<(), u32> {
        let mut client = door_service_client::Tasks::connect(self.registry).map_err(|_| 1722u32)?;
        f(&mut client, name).map_err(|e| match e {
            Error::Service(Code::NotFound) => 2,
            Error::Service(Code::Denied) => 5,
            _ => 1722,
        })
    }
    fn scm(&self) -> Result<door_service_client::Services, u32> {
        door_service_client::Services::connect(self.registry).map_err(|_| 1063)
    }
    /// The manager rereads `name` from the registry.
    fn scm_reload(&mut self, name: &str) -> Result<(), u32> {
        self.scm()?.reload(name)
    }
    /// One door.network VPN call for this user (RASDIAL, WIREGUARD).
    fn vpn(
        &mut self,
        call: impl FnOnce(&mut door_service_client::Network, u32) -> Result<(), Error>,
    ) -> Result<(), FsError> {
        let mut net =
            door_service_client::Network::connect(self.registry).map_err(|_| FsError::Device)?;
        call(&mut net, self.uid).map_err(fs_error)
    }
    fn read_all(&mut self, path: &str, limit: usize) -> Result<Vec<u8>, Error> {
        let file = self
            .storage
            .open(path, Disposition::OpenExisting, READ, SHARE_ALL)?;
        let result = (|| {
            let info = self.storage.info(&file)?;
            if info.directory {
                return Err(Error::Service(Code::Denied));
            }
            let want = (info.bytes as usize).min(limit);
            let mut data = Vec::new();
            data.try_reserve(want)
                .map_err(|_| Error::Service(Code::Resources))?;
            let mut chunk = [0u8; CHUNK_BYTES];
            while data.len() < want {
                let n = self.storage.read(
                    &file,
                    data.len() as u64,
                    &mut chunk[..(want - data.len()).min(CHUNK_BYTES)],
                )?;
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&chunk[..n]);
            }
            Ok(data)
        })();
        let closed = self.storage.close(file);
        let data = result?;
        closed?;
        Ok(data)
    }
    fn key(&mut self, key: &str, write: bool, create: bool) -> Option<door_service_client::Key> {
        self.key_access(key, if write { 3 } else { 1 }, create)
    }
    /// `key` opened with `access` (1 read, 2 write, 4 delete).
    fn key_access(
        &mut self,
        key: &str,
        access: u16,
        create: bool,
    ) -> Option<door_service_client::Key> {
        let (root, rest) = key.split_once('\\').unwrap_or((key, ""));
        let root = match root {
            "HKLM" => RegistryRoot::LocalMachine,
            "HKCU" => RegistryRoot::CurrentUser,
            "HKCR" => RegistryRoot::ClassesRoot,
            _ => return None,
        };
        let units: Vec<u16> = rest.encode_utf16().collect();
        self.storage
            .registry_open(root, RegistryView::Bits64, &units, access, create)
            .ok()
    }
    /// A multi-chunk copy as one staged replacement of the new (empty)
    /// output: 64 KiB is read, handed over in a section the service
    /// snapshots, and the whole file commits in one durable transaction
    /// instead of one per 192-byte chunk (a 4 MiB copy took over five
    /// minutes that way). A failure aborts the candidate; nothing is
    /// published.
    fn copy_staged(&mut self, input: &File, output: &File, bytes: usize) -> Result<(), Error> {
        let observed = self.storage.info(output)?;
        let mut staged = self.storage.begin_large_replace(output, observed, bytes)?;
        let mut buffer = alloc::vec![0u8; STAGE_SECTION_BYTES.min(bytes)];
        let filled = (|| {
            let mut at = 0;
            while at < bytes {
                let want = (bytes - at).min(buffer.len());
                let n = self
                    .storage
                    .read_full(input, at as u64, &mut buffer[..want])?;
                if n != want {
                    return Err(Error::Service(Code::Changed));
                }
                let part = &buffer[..n];
                match door_workspace::section_from(part) {
                    Ok(section) => {
                        let sent =
                            self.storage
                                .large_replace_write_section(&mut staged, section, 0, n);
                        let _ = door_sys::close(section);
                        sent?;
                    }
                    // No free mapping granule: the same bytes by message.
                    Err(_) => self.storage.large_replace_write(&mut staged, part)?,
                }
                at += n;
            }
            Ok(())
        })();
        match filled {
            Ok(()) => self.storage.commit_large_replace(staged).map(|_| ()),
            Err(e) => {
                let _ = self.storage.abort_large_replace(staged);
                Err(e)
            }
        }
    }
}
fn reg_value(v: &RegistryValue) -> RegValue {
    let d = v.data();
    let text = |d: &[u8]| {
        let units: Vec<u16> = d
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
            .trim_end_matches('\0')
            .into()
    };
    match v.kind {
        1 => RegValue::Sz(text(d)),
        2 => RegValue::ExpandSz(text(d)),
        7 => RegValue::MultiSz(
            text(d)
                .split('\0')
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect(),
        ),
        4 if d.len() == 4 => RegValue::Dword(u32::from_le_bytes(d.try_into().unwrap())),
        11 if d.len() == 8 => RegValue::Qword(u64::from_le_bytes(d.try_into().unwrap())),
        0 => RegValue::None,
        _ => RegValue::Binary(d.to_vec()),
    }
}
fn registry_value(v: &RegValue) -> Option<RegistryValue> {
    let string = |kind: u8, s: &str| {
        let mut b: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
        b.extend_from_slice(&[0, 0]);
        RegistryValue::new(kind, &b).ok()
    };
    match v {
        RegValue::Sz(s) => string(1, s),
        RegValue::ExpandSz(s) => string(2, s),
        RegValue::MultiSz(list) => {
            let mut b: Vec<u8> = Vec::new();
            for s in list {
                b.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
                b.extend_from_slice(&[0, 0]);
            }
            b.extend_from_slice(&[0, 0]);
            RegistryValue::new(7, &b).ok()
        }
        RegValue::Dword(d) => RegistryValue::new(4, &d.to_le_bytes()).ok(),
        RegValue::Qword(q) => RegistryValue::new(11, &q.to_le_bytes()).ok(),
        RegValue::Binary(b) => RegistryValue::new(3, b).ok(),
        RegValue::None => RegistryValue::new(0, &[]).ok(),
    }
}
impl Host for SessionHost<'_> {
    fn native(&self, windows: &str) -> Option<String> {
        let w = windows.trim_end_matches('\\');
        let b = w.as_bytes();
        if b.len() < 2 || !b[0].eq_ignore_ascii_case(&b'c') || b[1] != b':' {
            return None;
        }
        let rest = &w[2..];
        if rest.is_empty() {
            return Some(String::from("/compat"));
        }
        let lower = rest.to_ascii_lowercase();
        let profile = format!("\\users\\{}", self.user.to_ascii_lowercase());
        let slash = |s: &str| s.replace('\\', "/");
        if lower == profile || lower.starts_with(&format!("{profile}\\")) {
            return Some(format!(
                "{}{}",
                self.profile(),
                slash(&rest[profile.len()..])
            ));
        }
        if lower == "\\users" || lower.starts_with("\\users\\") {
            return Some(format!("/users{}", slash(&rest[6..])));
        }
        Some(format!("/compat{}", slash(rest)))
    }
    fn windows(&self, native: &str) -> String {
        let profile = self.profile();
        if native == profile || native.starts_with(&format!("{profile}/")) {
            return format!(
                "C:\\Users\\{}{}",
                self.user,
                native[profile.len()..].replace('/', "\\")
            );
        }
        if let Some(t) = native.strip_prefix("/users") {
            return format!("C:\\Users{}", t.replace('/', "\\"));
        }
        if let Some(t) = native.strip_prefix("/compat") {
            return if t.is_empty() {
                String::from("C:\\")
            } else {
                format!("C:{}", t.replace('/', "\\"))
            };
        }
        String::from(native)
    }
    fn stat(&mut self, path: &str) -> Result<Meta, FsError> {
        let (info, times) = self.storage.stat(path).map_err(fs_error)?;
        Ok(Meta {
            dir: info.directory,
            size: info.bytes,
            modified: times.modified,
            created: times.created,
            accessed: times.accessed,
            attrs: if info.directory {
                attr::DIRECTORY
            } else {
                attr::ARCHIVE
            },
        })
    }
    fn list(&mut self, path: &str) -> Result<Vec<Entry>, FsError> {
        let info = self.storage.metadata(path).map_err(fs_error)?;
        if !info.directory {
            return Err(FsError::Invalid);
        }
        let mut out = Vec::new();
        let mut cookie = 0;
        for _ in 0..20_000 {
            let Some(e) = self.storage.list(path, cookie).map_err(fs_error)? else {
                break;
            };
            if e.cookie <= cookie {
                break;
            }
            cookie = e.cookie;
            out.push(Entry {
                name: String::from_utf16_lossy(&e.name[..e.name_len]),
                meta: Meta {
                    dir: e.info.directory,
                    size: e.info.bytes,
                    modified: e.times.modified,
                    created: e.times.created,
                    accessed: e.times.accessed,
                    attrs: if e.info.directory {
                        attr::DIRECTORY
                    } else {
                        attr::ARCHIVE
                    },
                },
            });
        }
        Ok(out)
    }
    fn read(&mut self, path: &str, limit: usize) -> Result<Vec<u8>, FsError> {
        self.read_all(path, limit).map_err(fs_error)
    }
    fn write(&mut self, path: &str, data: &[u8], append: bool) -> Result<(), FsError> {
        let disposition = if append {
            Disposition::OpenAlways
        } else {
            Disposition::CreateAlways
        };
        let file = self
            .storage
            .open(path, disposition, READ | WRITE, SHARE_ALL)
            .map_err(fs_error)?;
        let result = (|| {
            let at = if append {
                self.storage.info(&file)?.bytes
            } else {
                0
            };
            if !append {
                self.storage.truncate(&file, 0)?;
            }
            if !data.is_empty() {
                self.storage.write_all(&file, at, data)?;
            }
            Ok(())
        })();
        let closed = self.storage.close(file);
        result.map_err(fs_error)?;
        closed.map_err(fs_error)
    }
    fn mkdir(&mut self, path: &str) -> Result<(), FsError> {
        self.storage.mkdir(path).map_err(fs_error)?;
        self.storage.flush().map_err(fs_error)
    }
    fn remove_file(&mut self, path: &str) -> Result<(), FsError> {
        if self.storage.metadata(path).map_err(fs_error)?.directory {
            return Err(FsError::Denied);
        }
        self.storage.remove(path).map_err(fs_error)?;
        self.storage.flush().map_err(fs_error)
    }
    fn remove_dir(&mut self, path: &str) -> Result<(), FsError> {
        if !self.storage.metadata(path).map_err(fs_error)?.directory {
            return Err(FsError::Invalid);
        }
        if self.storage.list(path, 0).map_err(fs_error)?.is_some() {
            return Err(FsError::NotEmpty);
        }
        self.storage.remove(path).map_err(fs_error)?;
        self.storage.flush().map_err(fs_error)
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), FsError> {
        self.storage.rename(from, to).map_err(fs_error)?;
        self.storage.flush().map_err(fs_error)
    }
    fn copy_file(&mut self, from: &str, to: &str) -> Result<(), FsError> {
        let input = self
            .storage
            .open(from, Disposition::OpenExisting, READ, SHARE_ALL)
            .map_err(fs_error)?;
        let result = (|| {
            let info = self.storage.info(&input)?;
            if info.directory {
                return Err(Error::Service(Code::Denied));
            }
            let output = self
                .storage
                .open(to, Disposition::CreateAlways, WRITE, SHARE_ALL)?;
            let copied = (|| {
                if info.bytes > CHUNK_BYTES as u64 && info.bytes <= LARGE_REPLACE_BYTES as u64 {
                    return self.copy_staged(&input, &output, info.bytes as usize);
                }
                let mut chunk = [0u8; CHUNK_BYTES];
                let mut at = 0u64;
                while at < info.bytes {
                    let n = self.storage.read(&input, at, &mut chunk)?;
                    if n == 0 {
                        return Err(Error::Service(Code::Changed));
                    }
                    self.storage.write_all(&output, at, &chunk[..n])?;
                    at += n as u64;
                }
                Ok(())
            })();
            let closed = self.storage.close(output);
            copied.and(closed)?;
            self.storage.flush()
        })();
        let closed = self.storage.close(input);
        result.and(closed).map_err(fs_error)
    }
    fn volume(&mut self, drive: char) -> Option<Volume> {
        if drive != 'C' {
            return None;
        }
        let usage = self.storage.volume_usage().ok();
        Some(Volume {
            label: String::new(),
            serial: 0x5D0C_2026,
            filesystem: String::from("NTFS"),
            total: usage.map_or(0, |u| u.total_bytes),
            free: usage.map_or(0, |u| u.free_bytes),
        })
    }
    fn now(&self) -> LocalTime {
        let ft = door_sys::wall_time().unwrap_or(0);
        door_shells::format::civil(ft, self.offset)
    }
    fn local(&self, filetime: u64) -> LocalTime {
        door_shells::format::civil(filetime, self.offset)
    }
    fn ticks(&self) -> u64 {
        door_sys::time().unwrap_or(0) * door_abi::platform::MONOTONIC_TICK_MILLIS
    }
    fn random(&mut self) -> u32 {
        let mut b = [0u8; 4];
        let _ = door_sys::entropy_read(&mut b);
        u32::from_le_bytes(b) ^ (self.ticks() as u32).rotate_left(13)
    }
    fn computer_name(&self) -> String {
        self.computer.clone()
    }
    fn user_name(&self) -> String {
        self.user.clone()
    }
    fn elevated(&self) -> bool {
        self.elevated
    }
    fn administrator(&self) -> bool {
        self.administrator
    }
    fn user_sid(&self) -> String {
        format!("S-1-5-21-2026-26300-1-{}", self.uid)
    }
    fn processors(&self) -> u32 {
        self.processors
    }
    fn adapters(&mut self) -> Vec<Adapter> {
        let Ok(mut net) = door_service_client::Network::connect(self.registry) else {
            return Vec::new();
        };
        let Ok(s) = net.status() else {
            return Vec::new();
        };
        if !s.present {
            return Vec::new();
        }
        let mask = if s.prefix == 0 {
            [0; 4]
        } else {
            (u32::MAX << (32 - u32::from(s.prefix.min(32)))).to_be_bytes()
        };
        let up = s.link && s.enabled && s.address != [0; 4];
        let v6 = net.status6().ok().filter(|_| s.link && s.enabled);
        let some = |a: [u8; 16]| (a != [0; 16]).then_some(a);
        // A VPN tunnel: its own adapter while it runs, addressed from the
        // start (the tunnel interface is up before the first handshake).
        let tunnel = net.vpn_tunnel().ok().flatten().map(|t| Adapter {
            name: String::from(t.name()),
            description: String::from("WireGuard Tunnel"),
            ipv4: Some(t.address),
            mask: if t.prefix == 0 {
                [0; 4]
            } else {
                (u32::MAX << (32 - u32::from(t.prefix.min(32)))).to_be_bytes()
            },
            dns: t.dns.iter().copied().filter(|d| *d != [0; 4]).collect(),
            connected: true,
            tunnel: true,
            ..Adapter::default()
        });
        let mut list = alloc::vec![Adapter {
            name: String::from("Ethernet"),
            description: String::from("Red Hat VirtIO Ethernet Adapter"),
            mac: s.mac,
            dhcp: s.config.fixed.is_none(),
            ipv4: up.then_some(s.address),
            mask,
            gateway: (s.gateway != [0; 4]).then_some(s.gateway),
            dns: s.dns.iter().copied().filter(|d| *d != [0; 4]).collect(),
            dhcp_server: (s.config.fixed.is_none() && s.gateway != [0; 4]).then_some(s.gateway),
            lease_obtained: None,
            lease_expires: None,
            connected: s.link && s.enabled,
            suffix: String::new(),
            ipv6: v6.and_then(|v| some(v.global)),
            link_local: v6.and_then(|v| some(v.link_local)),
            gateway6: v6.and_then(|v| some(v.router)),
            dns6: v6.map_or(Vec::new(), |v| v
                .dns
                .iter()
                .copied()
                .filter(|d| *d != [0; 16])
                .collect()),
            scope: v6.map_or(0, |v| v.scope),
            tunnel: false,
        }];
        list.extend(tunnel);
        list
    }
    fn vpn_connections(&mut self) -> Vec<door_shells::host::VpnConnection> {
        use door_service_protocol::net::vpn::{Failure, State};
        use door_shells::host::{VpnConnection, VpnState};
        let Ok(mut net) = door_service_client::Network::connect(self.registry) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for index in 0..=u8::MAX {
            let Ok(Some(e)) = net.vpn_entry(self.uid, index) else {
                break;
            };
            out.push(VpnConnection {
                name: String::from(e.name()),
                server: String::from(e.server()),
                state: match e.state {
                    State::Disconnected => VpnState::Disconnected,
                    State::Connecting => VpnState::Connecting,
                    State::Connected => VpnState::Connected,
                    State::Failed => VpnState::Failed,
                },
                // As Remote Access reports them.
                error: match e.failure {
                    Failure::None => 0,
                    Failure::NoNetwork => 800,
                    Failure::Unresolved => 868,
                    Failure::NoAnswer => 809,
                },
            });
        }
        out
    }
    // Saved and removed connections are kept in the user's phonebook too,
    // where the desktop shell finds them after a restart.
    fn vpn_save(&mut self, name: &str, text: &str) -> Result<(), FsError> {
        self.vpn(|net, uid| net.vpn_save(uid, name, text))?;
        let home = self.profile();
        door_workspace::phonebook::store(self.storage, &home, name, Some(text)).map_err(fs_error)
    }
    fn vpn_remove(&mut self, name: &str) -> Result<(), FsError> {
        self.vpn(|net, uid| net.vpn_remove(uid, name))?;
        let home = self.profile();
        door_workspace::phonebook::store(self.storage, &home, name, None).map_err(fs_error)
    }
    fn vpn_connect(&mut self, name: &str) -> Result<(), FsError> {
        self.vpn(|net, uid| net.vpn_connect(uid, name))
    }
    fn vpn_disconnect(&mut self, name: &str) -> Result<(), FsError> {
        self.vpn(|net, uid| net.vpn_disconnect(uid, name))
    }
    fn resolve(&mut self, name: &str) -> Option<[u8; 4]> {
        if let Some(a) = door_shells::cmd::parse_ip(name) {
            return Some(a);
        }
        let mut net = door_service_client::Network::connect(self.registry).ok()?;
        let (query, address) = net.resolve(name).ok()?;
        if address.is_some() {
            return address;
        }
        // A bounded wait (about two seconds) for the resolver.
        let start = door_sys::time().unwrap_or(0);
        loop {
            if let Ok(Some(a)) = net.resolved(query) {
                return a.list[..a.count].first().copied();
            }
            let now = door_sys::time().unwrap_or(0);
            if now.saturating_sub(start) > 200 {
                return None;
            }
            let _ = door_sys::sleep(now + 2);
        }
    }
    fn ping(
        &mut self,
        address: [u8; 4],
        bytes: usize,
        timeout_ms: u64,
    ) -> Result<Option<u64>, FsError> {
        use door_shells::host::Echo;
        Ok(match self.echo(address, bytes, timeout_ms, 128)? {
            Echo::Reply { ms, .. } => Some(ms),
            _ => None,
        })
    }
    /// One ICMP echo through door.network (it times the request out).
    fn echo(
        &mut self,
        address: [u8; 4],
        bytes: usize,
        timeout_ms: u64,
        ttl: u8,
    ) -> Result<door_shells::host::Echo, FsError> {
        use door_service_protocol::net::{self, ip_status};
        use door_shells::host::Echo;
        let mut network = door_service_client::Network::connect(self.registry)
            .map_err(|_| FsError::Unsupported)?;
        let request = net::EchoRequest {
            address,
            ttl,
            dont_fragment: false,
            size: bytes.min(usize::from(net::MAX_ECHO)) as u16,
            timeout: timeout_ms.clamp(1, 600_000) as u32,
            pattern: [0; net::PATTERN_BYTES],
            pattern_len: 0,
        };
        let echo = network.ping(&request).map_err(|_| FsError::Device)?;
        let ms = u64::from(echo.rtt);
        Ok(match echo.status {
            ip_status::SUCCESS => Echo::Reply {
                from: echo.from,
                ms,
                ttl: echo.ttl,
                bytes: usize::from(echo.size),
            },
            ip_status::REQ_TIMED_OUT => Echo::TimedOut,
            ip_status::TTL_EXPIRED_TRANSIT => Echo::TtlExpired {
                from: echo.from,
                ms,
            },
            ip_status::DEST_NET_UNREACHABLE
            | ip_status::DEST_HOST_UNREACHABLE
            | ip_status::DEST_PROT_UNREACHABLE
            | ip_status::DEST_PORT_UNREACHABLE
            | ip_status::PACKET_TOO_BIG => Echo::Unreachable {
                from: echo.from,
                status: echo.status,
            },
            _ => Echo::Failed,
        })
    }
    /// door.network's TCP and UDP sockets.
    fn connections(&mut self) -> Vec<door_shells::host::Connection> {
        use door_service_protocol::net;
        let Ok(mut network) = door_service_client::Network::connect(self.registry) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut page = [net::Connection::default(); net::diag::PAGE];
        loop {
            let Ok((total, count)) = network.connections(out.len() as u16, &mut page) else {
                break;
            };
            out.extend(page[..count].iter().map(|c| door_shells::host::Connection {
                tcp: c.protocol == 6,
                state: c.state,
                local: c.local,
                local_port: c.local_port,
                remote: c.remote,
                remote_port: c.remote_port,
                pid: c.pid,
                v6: None,
            }));
            if count == 0 || out.len() >= total {
                break;
            }
        }
        // The AF_INET6 sockets ([::] lines).
        let first = out.len();
        let mut page = [net::Connection6::default(); net::diag::PAGE6];
        loop {
            let Ok((total, count)) = network.connections6((out.len() - first) as u16, &mut page)
            else {
                break;
            };
            out.extend(page[..count].iter().map(|c| door_shells::host::Connection {
                tcp: c.protocol == 6,
                state: c.state,
                local_port: c.local_port,
                remote_port: c.remote_port,
                pid: c.pid,
                v6: Some(door_shells::host::Ipv6Ends {
                    local: c.local,
                    local_scope: c.local_scope,
                    remote: c.remote,
                    remote_scope: c.remote_scope,
                }),
                ..Default::default()
            }));
            if count == 0 || out.len() - first >= total {
                break;
            }
        }
        out
    }
    fn statistics(
        &mut self,
    ) -> Option<(
        door_shells::host::ProtocolStats,
        door_shells::host::ProtocolStats,
    )> {
        use door_service_protocol::net::stats::Family;
        let mut network = door_service_client::Network::connect(self.registry).ok()?;
        let counters = network.statistics().ok()?;
        let convert = |f: &Family, v6: bool| {
            // The ICMP types netstat prints, by slot.
            let kinds: &[u8] = if v6 {
                &[
                    1, 2, 3, 4, 128, 129, 130, 131, 132, 133, 134, 135, 136, 137, 138,
                ]
            } else {
                &[0, 3, 4, 5, 8, 9, 10, 11, 12, 13, 14, 17, 18]
            };
            door_shells::host::ProtocolStats {
                ip: [
                    f.ip_in_receives,
                    f.ip_in_header_errors,
                    0,
                    0,
                    f.ip_in_unknown_protos,
                    0,
                    f.ip_in_delivers,
                    f.ip_out_requests,
                    0,
                    0,
                    0,
                    f.ip_reasm_reqds,
                    0,
                    0,
                    0,
                    0,
                    f.ip_frag_creates,
                ],
                icmp_messages: (f.icmp_in_msgs, f.icmp_out_msgs),
                icmp_errors: (f.icmp_in_errors, f.icmp_out_errors),
                icmp_types: kinds
                    .iter()
                    .map(|&k| (k, f.icmp(v6, false, k), f.icmp(v6, true, k)))
                    .collect(),
                tcp: [
                    f.tcp_active_opens,
                    f.tcp_passive_opens,
                    f.tcp_attempt_fails,
                    f.tcp_estab_resets,
                    f.tcp_curr_estab,
                    f.tcp_in_segs,
                    f.tcp_out_segs,
                    f.tcp_retrans_segs,
                ],
                udp: [
                    f.udp_in_datagrams,
                    f.udp_no_ports,
                    f.udp_in_errors,
                    f.udp_out_datagrams,
                ],
            }
        };
        Some((convert(&counters.v4, false), convert(&counters.v6, true)))
    }
    fn neighbors(&mut self) -> Vec<door_shells::host::Neighbor> {
        use door_service_protocol::net::stats::{NEIGHBOR_DYNAMIC, Neighbor, PAGE};
        let Ok(mut network) = door_service_client::Network::connect(self.registry) else {
            return Vec::new();
        };
        let mut all = Vec::new();
        let mut page = [Neighbor::default(); PAGE];
        loop {
            let Ok((total, count)) = network.neighbors(all.len() as u16, &mut page) else {
                break;
            };
            all.extend_from_slice(&page[..count]);
            if count == 0 || all.len() >= total {
                break;
            }
        }
        all.iter()
            .filter_map(|n| {
                n.ipv4().map(|address| door_shells::host::Neighbor {
                    address,
                    mac: n.mac,
                    dynamic: n.kind == NEIGHBOR_DYNAMIC,
                })
            })
            .collect()
    }
    fn system(&mut self) -> SystemFacts {
        let boot = self.now();
        SystemFacts {
            os_name: String::from("Macrohard Doors 11 Pro"),
            os_version: String::from("10.0.26300 N/A Build 26300"),
            manufacturer: String::from("QEMU"),
            model: String::from("Standard PC (Q35 + ICH9, 2009)"),
            system_type: String::from("x64-based PC"),
            processors: (0..self.processors)
                .map(|_| {
                    String::from("Intel64 Family 6 Model 85 Stepping 7 GenuineIntel ~2600 Mhz")
                })
                .collect(),
            bios: String::from("EDK II, 2/2/2022"),
            memory_total_mb: 2048,
            memory_free_mb: 1024,
            boot_time: boot,
            install_date: boot,
            time_zone: String::from("(UTC) Coordinated Universal Time"),
            locale: String::from("en-us;English (United States)"),
            owner: self.user.clone(),
            product_id: String::from("00330-80000-00000-AA266"),
        }
    }
    fn set_clipboard(&mut self, text: &str) -> Result<(), FsError> {
        self.clipboard = Some(String::from(text));
        Ok(())
    }
    fn reg_values(&mut self, key: &str) -> Result<Vec<(String, RegValue)>, FsError> {
        let k = self.key(key, false, false).ok_or(FsError::NotFound)?;
        let mut out = Vec::new();
        let version = self.storage.registry_version(&k).unwrap_or(0);
        for i in 0..512 {
            match self.storage.registry_enumerate_value(&k, i, version) {
                Ok(e) => out.push((
                    String::from_utf16_lossy(e.name.units()),
                    reg_value(&e.value),
                )),
                Err(_) => break,
            }
        }
        let _ = self.storage.registry_close(k);
        Ok(out)
    }
    fn reg_subkeys(&mut self, key: &str) -> Result<Vec<String>, FsError> {
        let k = self.key(key, false, false).ok_or(FsError::NotFound)?;
        let mut out = Vec::new();
        let version = self.storage.registry_version(&k).unwrap_or(0);
        for i in 0..2048 {
            match self.storage.registry_enumerate_key(&k, i, version) {
                Ok(n) => out.push(String::from_utf16_lossy(n.units())),
                Err(_) => break,
            }
        }
        let _ = self.storage.registry_close(k);
        Ok(out)
    }
    fn firewall_reload(&mut self) {
        if let Ok(mut firewall) = door_service_client::Firewall::connect(self.registry) {
            let _ = firewall.reload();
        }
    }
    fn reg_set(&mut self, key: &str, name: &str, value: &RegValue) -> Result<(), FsError> {
        let v = registry_value(value).ok_or(FsError::TooLarge)?;
        let k = self.key(key, true, true).ok_or(FsError::Denied)?;
        let units: Vec<u16> = name.encode_utf16().collect();
        let r = self.storage.registry_set(&k, &units, v).map_err(fs_error);
        let _ = self.storage.registry_close(k);
        r?;
        self.storage.flush().map_err(fs_error)
    }
    fn reg_delete(&mut self, key: &str, name: Option<&str>) -> Result<(), FsError> {
        match name {
            Some(n) => {
                let k = self.key(key, true, false).ok_or(FsError::NotFound)?;
                let units: Vec<u16> = n.encode_utf16().collect();
                let r = self
                    .storage
                    .registry_delete_value(&k, &units)
                    .map_err(fs_error);
                let _ = self.storage.registry_close(k);
                r
            }
            None => {
                // Deleting a key needs DELETE access to it.
                let k = self
                    .key_access(key, 1 | 2 | 4, false)
                    .ok_or(FsError::NotFound)?;
                let r = self.storage.registry_delete_key(&k).map_err(fs_error);
                let _ = self.storage.registry_close(k);
                r
            }
        }
    }
    fn services(&mut self) -> Result<Vec<door_shells::host::Service>, u32> {
        let mut entries = Vec::new();
        self.scm()?
            .each(|e| entries.push((String::from(e.name()), e.state, e.pid)))?;
        let mut out = Vec::new();
        for (name, state, pid) in entries {
            let status = door_service_protocol::scm::Status {
                state,
                pid,
                ..Default::default()
            };
            let mut s = self.service_info(&name, status);
            // The list carries state and process; the rest is QUERY's.
            if let Ok(full) = self.scm().and_then(|mut m| m.query(&name)) {
                s.accepted = full.accepted;
                s.exit_code = full.exit_code;
            }
            out.push(s);
        }
        out.sort_by_key(|s| s.name.to_ascii_lowercase());
        Ok(out)
    }
    fn service(&mut self, name: &str) -> Result<door_shells::host::Service, u32> {
        let status = self.scm()?.query(name)?;
        Ok(self.service_info(name, status))
    }
    fn service_start(
        &mut self,
        name: &str,
        arguments: &[String],
    ) -> Result<door_shells::host::Service, u32> {
        let status = if arguments.is_empty() {
            let directory = self.directory;
            self.scm()?.start(name, directory)?
        } else {
            // Each argument NUL-terminated, as StartService passes them.
            let mut bytes = alloc::vec::Vec::new();
            for a in arguments {
                bytes.extend_from_slice(a.as_bytes());
                bytes.push(0);
            }
            self.scm()?.start_with(name, &bytes)?
        };
        Ok(self.service_info(name, status))
    }
    fn service_control(
        &mut self,
        name: &str,
        control: u32,
    ) -> Result<door_shells::host::Service, u32> {
        let status = self.scm()?.control(name, control)?;
        Ok(self.service_info(name, status))
    }
    fn service_create(&mut self, service: &door_shells::host::Service) -> Result<(), u32> {
        let key = format!("{SERVICES_KEY}\\{}", service.name);
        if door_shells::host::Host::reg_values(self, &key).is_ok() {
            return Err(1073);
        }
        if self.cycle(&service.name, &service.dependencies) {
            return Err(1059);
        }
        let denied = |e: FsError| -> u32 { if e == FsError::Denied { 5 } else { 87 } };
        for (n, v) in [
            ("Type", RegValue::Dword(service.kind)),
            ("Start", RegValue::Dword(service.start)),
            ("ErrorControl", RegValue::Dword(1)),
            ("ImagePath", RegValue::ExpandSz(service.binary.clone())),
            ("DisplayName", RegValue::Sz(service.display.clone())),
            ("ObjectName", RegValue::Sz(String::from("LocalSystem"))),
        ] {
            door_shells::host::Host::reg_set(self, &key, n, &v).map_err(denied)?;
        }
        if !service.dependencies.is_empty() {
            door_shells::host::Host::reg_set(
                self,
                &key,
                "DependOnService",
                &RegValue::MultiSz(service.dependencies.clone()),
            )
            .map_err(denied)?;
        }
        self.scm_reload(&service.name)
    }
    fn service_set_dependencies(&mut self, name: &str, on: &[String]) -> Result<(), u32> {
        let key = format!("{SERVICES_KEY}\\{name}");
        if door_shells::host::Host::reg_values(self, &key).is_err() {
            return Err(1060);
        }
        if self.cycle(name, on) {
            return Err(1059);
        }
        let denied = |e: FsError| -> u32 { if e == FsError::Denied { 5 } else { 87 } };
        if on.is_empty() {
            let _ = door_shells::host::Host::reg_delete(self, &key, Some("DependOnService"));
        } else {
            door_shells::host::Host::reg_set(
                self,
                &key,
                "DependOnService",
                &RegValue::MultiSz(on.to_vec()),
            )
            .map_err(denied)?;
        }
        self.scm_reload(name)
    }
    fn service_delete(&mut self, name: &str) -> Result<(), u32> {
        if BUILTIN_SERVICES
            .iter()
            .any(|(n, ..)| n.eq_ignore_ascii_case(name))
        {
            return Err(5);
        }
        let key = format!("{SERVICES_KEY}\\{name}");
        door_shells::host::Host::reg_delete(self, &key, None).map_err(|e| -> u32 {
            match e {
                FsError::Denied => 5,
                _ => 1060,
            }
        })?;
        self.scm_reload(name)
    }
    fn service_configure(
        &mut self,
        name: &str,
        start: Option<u32>,
        binary: Option<&str>,
        display: Option<&str>,
        description: Option<&str>,
    ) -> Result<(), u32> {
        let key = format!("{SERVICES_KEY}\\{name}");
        if door_shells::host::Host::reg_values(self, &key).is_err() {
            return Err(1060);
        }
        let denied = |e: FsError| -> u32 { if e == FsError::Denied { 5 } else { 87 } };
        if let Some(v) = start {
            door_shells::host::Host::reg_set(self, &key, "Start", &RegValue::Dword(v))
                .map_err(denied)?;
        }
        if let Some(v) = binary {
            door_shells::host::Host::reg_set(
                self,
                &key,
                "ImagePath",
                &RegValue::ExpandSz(String::from(v)),
            )
            .map_err(denied)?;
        }
        if let Some(v) = display {
            door_shells::host::Host::reg_set(
                self,
                &key,
                "DisplayName",
                &RegValue::Sz(String::from(v)),
            )
            .map_err(denied)?;
        }
        if let Some(v) = description {
            door_shells::host::Host::reg_set(
                self,
                &key,
                "Description",
                &RegValue::Sz(String::from(v)),
            )
            .map_err(denied)?;
        }
        self.scm_reload(name)
    }
    fn report_event(
        &mut self,
        log: &str,
        kind: u16,
        id: u32,
        source: &str,
        message: &str,
    ) -> Result<(), u32> {
        use door_service_protocol::eventlog::{Event, Log};
        let log = Log::named(log).ok_or(15007u32)?;
        let mut client =
            door_service_client::EventLog::connect(self.registry).map_err(|_| 1722u32)?;
        client
            .report(&Event::new(log, kind, 0, id, source, message))
            .map(|_| ())
            .map_err(|e| match e {
                Error::Service(Code::Denied) => 5,
                _ => 1722,
            })
    }
    fn events(
        &mut self,
        log: &str,
        newest_first: bool,
        count: usize,
    ) -> Result<Vec<door_shells::host::HostEvent>, u32> {
        use door_service_protocol::eventlog::Log;
        let log = Log::named(log).ok_or(15007u32)?;
        let denied = |e: Error| match e {
            Error::Service(Code::Denied) => 5u32,
            _ => 1722,
        };
        let mut client =
            door_service_client::EventLog::connect(self.registry).map_err(|_| 1722u32)?;
        let (oldest, total) = client.info(log).map_err(denied)?;
        let mut out = Vec::new();
        if total == 0 {
            return Ok(out);
        }
        let newest = oldest + total - 1;
        let mut record = if newest_first { newest } else { oldest };
        while out.len() < count && (oldest..=newest).contains(&record) {
            if let Some(e) = client.read(log, record).map_err(denied)? {
                out.push(door_shells::host::HostEvent {
                    record: e.record,
                    time: e.time,
                    id: e.id,
                    kind: e.kind,
                    source: String::from(e.source()),
                    message: String::from(e.message()),
                });
            }
            record = if newest_first {
                record.wrapping_sub(1)
            } else {
                record + 1
            };
        }
        Ok(out)
    }
    fn clear_log(&mut self, log: &str) -> Result<(), u32> {
        use door_service_protocol::eventlog::Log;
        let log = Log::named(log).ok_or(15007u32)?;
        let mut client =
            door_service_client::EventLog::connect(self.registry).map_err(|_| 1722u32)?;
        client.clear(log).map_err(|e| match e {
            Error::Service(Code::Denied) => 5,
            _ => 1722,
        })
    }
    fn tasks(&mut self) -> Result<Vec<door_shells::host::HostTask>, u32> {
        let mut client = door_service_client::Tasks::connect(self.registry).map_err(|_| 1722u32)?;
        let mut out = Vec::new();
        for index in 0..door_service_protocol::tasks::TASKS as u16 {
            match client.query(index) {
                Ok(Some(s)) => out.push(door_shells::host::HostTask {
                    name: String::from(s.task.name()),
                    command: String::from(s.task.command()),
                    state: s.state,
                    next_run: s.next_run,
                    last_run: s.last_run,
                    last_result: s.last_result,
                }),
                Ok(None) => break,
                Err(_) => return Err(1722),
            }
        }
        Ok(out)
    }
    fn task_create(
        &mut self,
        name: &str,
        command: &str,
        kind: u8,
        modifier: u16,
        start: Option<(u32, u32)>,
        replace: bool,
    ) -> Result<(), u32> {
        use door_service_protocol::tasks::{MINUTE, Schedule, Task};
        let modifier = modifier.max(1);
        let schedule = match kind {
            0 => Schedule::Once,
            1 => Schedule::Minutes(modifier),
            2 => Schedule::Hours(modifier),
            3 => Schedule::Days(modifier),
            _ => Schedule::OnLogon,
        };
        // Today's local time of day as UTC (FILETIME); none given: the next
        // whole minute.
        let now = door_sys::wall_time().unwrap_or(0);
        let offset = self.offset * MINUTE as i64;
        let start = match start {
            Some((h, m)) => {
                let local = now as i64 + offset;
                let midnight = local - local.rem_euclid(1440 * MINUTE as i64);
                (midnight + (i64::from(h) * 60 + i64::from(m)) * MINUTE as i64 - offset) as u64
            }
            None => now.div_ceil(MINUTE) * MINUTE,
        };
        let mut task = Task::new(name, command, schedule, start).ok_or(87u32)?;
        task.replace = replace;
        let mut client = door_service_client::Tasks::connect(self.registry).map_err(|_| 1722u32)?;
        client.create(&task).map_err(|e| match e {
            Error::Service(Code::Exists) => 183,
            Error::Service(Code::Denied) => 5,
            _ => 1722,
        })
    }
    fn task_delete(&mut self, name: &str) -> Result<(), u32> {
        self.task_call(name, |c, n| c.delete(n))
    }
    fn task_run(&mut self, name: &str) -> Result<(), u32> {
        self.task_call(name, |c, n| c.run(n))
    }
    fn task_end(&mut self, name: &str) -> Result<(), u32> {
        self.task_call(name, |c, n| c.end(n))
    }
    fn task_enable(&mut self, name: &str, enabled: bool) -> Result<(), u32> {
        self.task_call(name, |c, n| c.enable(n, enabled))
    }
    fn local_time(&mut self, filetime: u64) -> Option<(i64, u32, u32, u32, u32, u32)> {
        if filetime == 0 {
            return None;
        }
        let seconds = (filetime / 10_000_000) as i64 + self.offset * 60;
        let day = seconds.div_euclid(86_400) - 134_774;
        let rest = seconds.rem_euclid(86_400);
        let (y, m, d) = door_shells::host::civil_from_days(day);
        Some((
            y,
            m,
            d,
            (rest / 3600) as u32,
            (rest / 60 % 60) as u32,
            (rest % 60) as u32,
        ))
    }
    fn sleep_ms(&mut self, ms: u64) {
        let now = door_sys::time().unwrap_or(0);
        let _ = door_sys::sleep(now + ms.div_ceil(10));
    }
    fn set_user_variable(&mut self, name: &str, value: &str) -> Result<(), FsError> {
        self.reg_set(
            "HKCU\\Environment",
            name,
            &RegValue::Sz(String::from(value)),
        )
    }
    fn digest(&mut self, path: &str, algorithm: &str) -> Result<Vec<u8>, FsError> {
        use sha2::Digest;
        let data = self.read(path, 64 << 20)?;
        Ok(match algorithm {
            "SHA256" => sha2::Sha256::digest(&data).to_vec(),
            "SHA384" => sha2::Sha384::digest(&data).to_vec(),
            "SHA512" => sha2::Sha512::digest(&data).to_vec(),
            "SHA1" => sha1::Sha1::digest(&data).to_vec(),
            _ => return Err(FsError::Unsupported),
        })
    }
}

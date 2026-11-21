//! An in-memory host for the engines' tests: a file tree with the Doors
//! namespace (`/compat` is C:\, `/users` is C:\Users), a fixed clock and a
//! few processes and adapters.
use crate::host::{
    Adapter, Entry, FsError, Host, LocalTime, Meta, Process, RegValue, SystemFacts, Volume,
    VpnConnection, VpnState, attr,
};
use alloc::{collections::BTreeMap, format, string::String, vec::Vec};

#[derive(Clone)]
pub struct Node {
    pub dir: bool,
    pub data: Vec<u8>,
    pub attrs: u32,
}
pub struct MemHost {
    /// Lower-cased native path -> (display path, node).
    pub files: BTreeMap<String, (String, Node)>,
    pub registry: BTreeMap<String, Vec<(String, RegValue)>>,
    pub clipboard: Option<String>,
    pub seed: u32,
    pub user_vars: Vec<(String, String)>,
    /// The session runs as administrator.
    pub elevated: bool,
    /// Times netsh asked for the firewall policy to be reloaded.
    pub firewall_reloads: u32,
    /// VPN connections, their saved texts, and what a dial ends in
    /// (after two status reads).
    pub vpns: Vec<(VpnConnection, String)>,
    pub vpn_outcome: (VpnState, u32),
    vpn_wait: u32,
    /// The network stack's sockets (netstat).
    pub connections: Vec<crate::host::Connection>,
    /// The Service Control Manager: services move to their target state
    /// at once (a start runs, a stop stops).
    pub services: Vec<crate::host::Service>,
    /// The arguments of the last service start.
    pub service_arguments: Vec<String>,
    /// The Event Log: (log, event), oldest first.
    pub events: Vec<(String, crate::host::HostEvent)>,
    /// The Task Scheduler.
    pub tasks: Vec<crate::host::HostTask>,
}
/// 2026-11-02 12:16:00 UTC, the reference capture's minute.
pub const NOW: LocalTime = LocalTime {
    year: 2026,
    month: 11,
    day: 2,
    hour: 12,
    minute: 16,
    second: 0,
    centis: 0,
    weekday: 1,
};
impl MemHost {
    pub fn new() -> Self {
        let mut h = Self {
            files: BTreeMap::new(),
            registry: BTreeMap::new(),
            clipboard: None,
            seed: 7,
            user_vars: Vec::new(),
            elevated: false,
            firewall_reloads: 0,
            vpns: Vec::new(),
            vpn_outcome: (VpnState::Connected, 0),
            vpn_wait: 0,
            connections: Vec::new(),
            services: Vec::new(),
            service_arguments: Vec::new(),
            events: Vec::new(),
            tasks: Vec::new(),
        };
        for d in [
            "/compat",
            "/compat/Windows",
            "/compat/Windows/System32",
            "/users",
            "/users/1001",
            "/users/1001/Documents",
            "/compat/Program Files",
        ] {
            h.dir(d);
        }
        h.registry.insert(
            String::from("HKCR\\.txt"),
            alloc::vec![(String::new(), RegValue::Sz(String::from("txtfilelegacy")))],
        );
        h
    }
    pub fn dir(&mut self, path: &str) {
        self.files.insert(
            path.to_lowercase(),
            (
                String::from(path),
                Node {
                    dir: true,
                    data: Vec::new(),
                    attrs: attr::DIRECTORY,
                },
            ),
        );
    }
    pub fn file(&mut self, path: &str, data: &str) {
        self.files.insert(
            path.to_lowercase(),
            (
                String::from(path),
                Node {
                    dir: false,
                    data: data.as_bytes().to_vec(),
                    attrs: attr::ARCHIVE,
                },
            ),
        );
    }
    pub fn bytes(&mut self, path: &str, data: &[u8]) {
        self.file(path, "");
        if let Some((_, n)) = self.files.get_mut(&path.to_lowercase()) {
            n.data = data.to_vec();
        }
    }
    pub fn data(&self, path: &str) -> Option<&[u8]> {
        self.files
            .get(&path.to_lowercase())
            .map(|(_, n)| n.data.as_slice())
    }
    pub fn text(&self, path: &str) -> Option<String> {
        self.files
            .get(&path.to_lowercase())
            .map(|(_, n)| String::from_utf8_lossy(&n.data).into_owned())
    }
    fn parent(path: &str) -> &str {
        match path.rfind('/') {
            Some(0) => "/",
            Some(i) => &path[..i],
            None => "/",
        }
    }
}
impl Default for MemHost {
    fn default() -> Self {
        Self::new()
    }
}
impl Host for MemHost {
    fn native(&self, windows: &str) -> Option<String> {
        let w = windows.trim_end_matches('\\');
        let rest = w.get(2..)?;
        if !w[..1].eq_ignore_ascii_case("c") || !w[1..].starts_with(':') {
            return None;
        }
        let rest = rest.replace('\\', "/");
        let lower = rest.to_lowercase();
        if lower == "/users" || lower.starts_with("/users/") {
            return Some(rest);
        }
        Some(format!("/compat{rest}"))
    }
    fn windows(&self, native: &str) -> String {
        let display = self
            .files
            .get(&native.to_lowercase())
            .map_or(String::from(native), |(d, _)| d.clone());
        if let Some(t) = display.strip_prefix("/compat") {
            let t = t.replace('/', "\\");
            return if t.is_empty() {
                String::from("C:\\")
            } else {
                format!("C:{t}")
            };
        }
        let t = display.replace('/', "\\");
        match t.get(..6) {
            Some(p) if p.eq_ignore_ascii_case("\\users") => format!("C:\\Users{}", &t[6..]),
            _ => format!("C:{t}"),
        }
    }
    fn stat(&mut self, path: &str) -> Result<Meta, FsError> {
        let (_, n) = self
            .files
            .get(&path.to_lowercase())
            .ok_or(FsError::NotFound)?;
        Ok(Meta {
            dir: n.dir,
            size: n.data.len() as u64,
            modified: crate::format::filetime(&NOW),
            created: crate::format::filetime(&NOW),
            accessed: 0,
            attrs: n.attrs,
        })
    }
    fn list(&mut self, path: &str) -> Result<Vec<Entry>, FsError> {
        let key = path.to_lowercase();
        match self.files.get(&key) {
            Some((_, n)) if n.dir => (),
            Some(_) => return Err(FsError::Invalid),
            None => return Err(FsError::PathNotFound),
        }
        let mut out = Vec::new();
        for (k, (display, n)) in &self.files {
            if k != &key && Self::parent(k) == key {
                let name = display.rsplit('/').next().unwrap_or("");
                out.push(Entry {
                    name: String::from(name),
                    meta: Meta {
                        dir: n.dir,
                        size: n.data.len() as u64,
                        modified: crate::format::filetime(&NOW),
                        created: crate::format::filetime(&NOW),
                        accessed: 0,
                        attrs: n.attrs,
                    },
                });
            }
        }
        Ok(out)
    }
    fn read(&mut self, path: &str, limit: usize) -> Result<Vec<u8>, FsError> {
        let (_, n) = self
            .files
            .get(&path.to_lowercase())
            .ok_or(FsError::NotFound)?;
        if n.dir {
            return Err(FsError::Denied);
        }
        Ok(n.data[..n.data.len().min(limit)].to_vec())
    }
    fn write(&mut self, path: &str, data: &[u8], append: bool) -> Result<(), FsError> {
        let parent = Self::parent(path).to_lowercase();
        if !self.files.get(&parent).is_some_and(|(_, n)| n.dir) {
            return Err(FsError::PathNotFound);
        }
        let key = path.to_lowercase();
        match self.files.get_mut(&key) {
            Some((_, n)) if n.dir => Err(FsError::Denied),
            Some((_, n)) => {
                if n.attrs & attr::READONLY != 0 {
                    return Err(FsError::Denied);
                }
                if !append {
                    n.data.clear();
                }
                n.data.extend_from_slice(data);
                Ok(())
            }
            None => {
                self.files.insert(
                    key,
                    (
                        String::from(path),
                        Node {
                            dir: false,
                            data: data.to_vec(),
                            attrs: attr::ARCHIVE,
                        },
                    ),
                );
                Ok(())
            }
        }
    }
    fn mkdir(&mut self, path: &str) -> Result<(), FsError> {
        let key = path.to_lowercase();
        if self.files.contains_key(&key) {
            return Err(FsError::Exists);
        }
        if !self
            .files
            .get(&Self::parent(path).to_lowercase())
            .is_some_and(|(_, n)| n.dir)
        {
            return Err(FsError::PathNotFound);
        }
        self.dir(path);
        Ok(())
    }
    fn remove_file(&mut self, path: &str) -> Result<(), FsError> {
        match self.files.get(&path.to_lowercase()) {
            Some((_, n)) if !n.dir => {
                self.files.remove(&path.to_lowercase());
                Ok(())
            }
            Some(_) => Err(FsError::Denied),
            None => Err(FsError::NotFound),
        }
    }
    fn remove_dir(&mut self, path: &str) -> Result<(), FsError> {
        let key = path.to_lowercase();
        if !self.files.get(&key).is_some_and(|(_, n)| n.dir) {
            return Err(FsError::NotFound);
        }
        if self
            .files
            .keys()
            .any(|k| k != &key && Self::parent(k) == key)
        {
            return Err(FsError::NotEmpty);
        }
        self.files.remove(&key);
        Ok(())
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), FsError> {
        let fk = from.to_lowercase();
        if !self.files.contains_key(&fk) {
            return Err(FsError::NotFound);
        }
        let tk = to.to_lowercase();
        if self.files.contains_key(&tk) && fk != tk {
            return Err(FsError::Exists);
        }
        let moved: Vec<(String, (String, Node))> = self
            .files
            .iter()
            .filter(|(k, _)| **k == fk || k.starts_with(&format!("{fk}/")))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (k, (display, node)) in moved {
            self.files.remove(&k);
            let suffix = &display[from.len().min(display.len())..];
            let nd = format!("{to}{suffix}");
            self.files.insert(nd.to_lowercase(), (nd, node));
        }
        Ok(())
    }
    fn set_attributes(&mut self, path: &str, attrs: u32) -> Result<(), FsError> {
        let (_, n) = self
            .files
            .get_mut(&path.to_lowercase())
            .ok_or(FsError::NotFound)?;
        n.attrs = attrs | if n.dir { attr::DIRECTORY } else { 0 };
        Ok(())
    }
    fn volume(&mut self, drive: char) -> Option<Volume> {
        (drive == 'C').then(|| Volume {
            label: String::new(),
            serial: 0xB886_8105,
            filesystem: String::from("NTFS"),
            total: 64 << 30,
            free: 33_532_461_056,
        })
    }
    fn now(&self) -> LocalTime {
        NOW
    }
    fn ticks(&self) -> u64 {
        1000
    }
    fn random(&mut self) -> u32 {
        self.seed = self.seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        self.seed >> 8
    }
    fn computer_name(&self) -> String {
        String::from("DOORS-PC")
    }
    fn user_name(&self) -> String {
        String::from("Win")
    }
    fn elevated(&self) -> bool {
        self.elevated
    }
    fn processes(&mut self) -> Vec<Process> {
        alloc::vec![
            Process {
                pid: 4,
                name: String::from("System"),
                session_name: String::from("Services"),
                session: 0,
                memory_kb: 144,
                ..Process::default()
            },
            Process {
                pid: 1204,
                name: String::from("door-files.exe"),
                session_name: String::from("Console"),
                session: 1,
                memory_kb: 25_600,
                ..Process::default()
            },
        ]
    }
    fn terminate(&mut self, pid: u32, _force: bool) -> Result<(), FsError> {
        if pid == 1204 {
            Ok(())
        } else {
            Err(FsError::Denied)
        }
    }
    fn report_event(
        &mut self,
        log: &str,
        kind: u16,
        id: u32,
        source: &str,
        message: &str,
    ) -> Result<(), u32> {
        let log = crate::host::log_name(log).ok_or(15007u32)?;
        if log == "System" && !self.elevated {
            return Err(5);
        }
        let record = self.events.iter().filter(|(l, _)| l == log).count() as u32 + 1;
        self.events.push((
            String::from(log),
            crate::host::HostEvent {
                record,
                // 2026-11-02 12:16:00 UTC.
                time: 134_380_953_600_000_000,
                id,
                kind,
                source: String::from(source),
                message: String::from(message),
            },
        ));
        Ok(())
    }
    fn events(
        &mut self,
        log: &str,
        newest_first: bool,
        count: usize,
    ) -> Result<Vec<crate::host::HostEvent>, u32> {
        let log = crate::host::log_name(log).ok_or(15007u32)?;
        let mut out: Vec<_> = self
            .events
            .iter()
            .filter(|(l, _)| l == log)
            .map(|(_, e)| e.clone())
            .collect();
        if newest_first {
            out.reverse();
        }
        out.truncate(count);
        Ok(out)
    }
    fn clear_log(&mut self, log: &str) -> Result<(), u32> {
        let log = crate::host::log_name(log).ok_or(15007u32)?;
        if !self.elevated {
            return Err(5);
        }
        self.events.retain(|(l, _)| l != log);
        Ok(())
    }
    fn tasks(&mut self) -> Result<Vec<crate::host::HostTask>, u32> {
        Ok(self.tasks.clone())
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
        if let Some(at) = self
            .tasks
            .iter()
            .position(|t| t.name.eq_ignore_ascii_case(name))
        {
            if !replace {
                return Err(183);
            }
            self.tasks.remove(at);
        }
        let _ = (kind, modifier);
        // NOW is 12:16; a start time is today's.
        let next = start.map_or(0, |(h, m)| {
            134_380_953_600_000_000
                + ((i64::from(h) * 60 + i64::from(m) - 736) * 600_000_000) as u64
        });
        self.tasks.push(crate::host::HostTask {
            name: String::from(name),
            command: String::from(command),
            state: 1,
            next_run: next,
            last_run: 0,
            last_result: 0x41303,
        });
        Ok(())
    }
    fn task_delete(&mut self, name: &str) -> Result<(), u32> {
        let before = self.tasks.len();
        self.tasks.retain(|t| !t.name.eq_ignore_ascii_case(name));
        if before == self.tasks.len() {
            Err(2)
        } else {
            Ok(())
        }
    }
    fn task_run(&mut self, name: &str) -> Result<(), u32> {
        let t = self
            .tasks
            .iter_mut()
            .find(|t| t.name.eq_ignore_ascii_case(name))
            .ok_or(2u32)?;
        t.state = 2;
        Ok(())
    }
    fn task_end(&mut self, name: &str) -> Result<(), u32> {
        let t = self
            .tasks
            .iter_mut()
            .find(|t| t.name.eq_ignore_ascii_case(name))
            .ok_or(2u32)?;
        t.state = 1;
        Ok(())
    }
    fn task_enable(&mut self, name: &str, enabled: bool) -> Result<(), u32> {
        let t = self
            .tasks
            .iter_mut()
            .find(|t| t.name.eq_ignore_ascii_case(name))
            .ok_or(2u32)?;
        t.state = if enabled { 1 } else { 3 };
        Ok(())
    }
    fn local_time(&mut self, filetime: u64) -> Option<(i64, u32, u32, u32, u32, u32)> {
        if filetime == 0 {
            return None;
        }
        // UTC in this host (NOW's zone).
        let minutes = (filetime / 600_000_000) as i64;
        let day = minutes.div_euclid(1440) - 134_774;
        let (y, m, d) = crate::host::civil_from_days(day);
        let rest = minutes.rem_euclid(1440);
        Some((y, m, d, (rest / 60) as u32, (rest % 60) as u32, 0))
    }
    fn services(&mut self) -> Result<Vec<crate::host::Service>, u32> {
        Ok(self.services.clone())
    }
    fn service_start(
        &mut self,
        name: &str,
        arguments: &[String],
    ) -> Result<crate::host::Service, u32> {
        let elevated = self.elevated;
        self.service_arguments = arguments.to_vec();
        let s = self
            .services
            .iter_mut()
            .find(|s| s.name.eq_ignore_ascii_case(name))
            .ok_or(1060u32)?;
        if !elevated {
            return Err(5);
        }
        match (s.start, s.state) {
            (4, _) => Err(1058),
            (_, 1) => {
                s.state = 4;
                s.pid = 4242;
                s.accepted = 0x7;
                Ok(s.clone())
            }
            _ => Err(1056),
        }
    }
    fn service_control(&mut self, name: &str, control: u32) -> Result<crate::host::Service, u32> {
        let elevated = self.elevated;
        let s = self
            .services
            .iter_mut()
            .find(|s| s.name.eq_ignore_ascii_case(name))
            .ok_or(1060u32)?;
        if !elevated && control != 4 {
            return Err(5);
        }
        match (control, s.state) {
            (_, 1) => Err(1062),
            (1, _) if s.accepted & 1 == 0 => Err(1052),
            (1, _) => {
                s.state = 1;
                s.pid = 0;
                s.accepted = 0;
                Ok(s.clone())
            }
            (2, 4) => {
                s.state = 7;
                Ok(s.clone())
            }
            (3, 7) => {
                s.state = 4;
                Ok(s.clone())
            }
            (4, _) => Ok(s.clone()),
            _ => Err(1052),
        }
    }
    fn service_create(&mut self, service: &crate::host::Service) -> Result<(), u32> {
        if !self.elevated {
            return Err(5);
        }
        if self
            .services
            .iter()
            .any(|s| s.name.eq_ignore_ascii_case(&service.name))
        {
            return Err(1073);
        }
        self.services.push(service.clone());
        Ok(())
    }
    fn service_set_dependencies(&mut self, name: &str, on: &[String]) -> Result<(), u32> {
        if !self.elevated {
            return Err(5);
        }
        // ERROR_CIRCULAR_DEPENDENCY.
        let mut pending = on.to_vec();
        let mut seen = 0;
        while let Some(d) = pending.pop() {
            if d.eq_ignore_ascii_case(name) {
                return Err(1059);
            }
            seen += 1;
            if seen > 64 {
                break;
            }
            if let Some(s) = self
                .services
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(&d))
            {
                pending.extend(s.dependencies.iter().cloned());
            }
        }
        let s = self
            .services
            .iter_mut()
            .find(|s| s.name.eq_ignore_ascii_case(name))
            .ok_or(1060u32)?;
        s.dependencies = on.to_vec();
        Ok(())
    }
    fn service_delete(&mut self, name: &str) -> Result<(), u32> {
        if !self.elevated {
            return Err(5);
        }
        let before = self.services.len();
        self.services.retain(|s| !s.name.eq_ignore_ascii_case(name));
        if before == self.services.len() {
            Err(1060)
        } else {
            Ok(())
        }
    }
    fn service_configure(
        &mut self,
        name: &str,
        start: Option<u32>,
        binary: Option<&str>,
        display: Option<&str>,
        description: Option<&str>,
    ) -> Result<(), u32> {
        if !self.elevated {
            return Err(5);
        }
        let s = self
            .services
            .iter_mut()
            .find(|s| s.name.eq_ignore_ascii_case(name))
            .ok_or(1060u32)?;
        if let Some(v) = start {
            s.start = v;
        }
        if let Some(v) = binary {
            s.binary = String::from(v);
        }
        if let Some(v) = display {
            s.display = String::from(v);
        }
        if let Some(v) = description {
            s.description = String::from(v);
        }
        Ok(())
    }
    fn vpn_connections(&mut self) -> Vec<VpnConnection> {
        if self.vpn_wait > 0 {
            self.vpn_wait -= 1;
            if self.vpn_wait == 0 {
                let (state, error) = self.vpn_outcome;
                for (c, _) in self
                    .vpns
                    .iter_mut()
                    .filter(|(c, _)| c.state == VpnState::Connecting)
                {
                    c.state = state;
                    c.error = error;
                }
            }
        }
        self.vpns.iter().map(|(c, _)| c.clone()).collect()
    }
    fn vpn_save(&mut self, name: &str, text: &str) -> Result<(), FsError> {
        self.vpns
            .retain(|(c, _)| !c.name.eq_ignore_ascii_case(name));
        let connection = VpnConnection {
            name: String::from(name),
            server: String::from("10.0.2.2:51820"),
            ..VpnConnection::default()
        };
        self.vpns.push((connection, String::from(text)));
        Ok(())
    }
    fn vpn_remove(&mut self, name: &str) -> Result<(), FsError> {
        let before = self.vpns.len();
        self.vpns
            .retain(|(c, _)| !c.name.eq_ignore_ascii_case(name));
        if self.vpns.len() == before {
            Err(FsError::NotFound)
        } else {
            Ok(())
        }
    }
    fn vpn_connect(&mut self, name: &str) -> Result<(), FsError> {
        for (c, _) in &mut self.vpns {
            c.state = if c.name.eq_ignore_ascii_case(name) {
                VpnState::Connecting
            } else {
                VpnState::Disconnected
            };
        }
        self.vpn_wait = 2;
        Ok(())
    }
    fn vpn_disconnect(&mut self, name: &str) -> Result<(), FsError> {
        let c = self
            .vpns
            .iter_mut()
            .find(|(c, _)| c.name.eq_ignore_ascii_case(name))
            .ok_or(FsError::NotFound)?;
        c.0.state = VpnState::Disconnected;
        Ok(())
    }
    fn adapters(&mut self) -> Vec<Adapter> {
        let mut list = alloc::vec![Adapter {
            name: String::from("Ethernet"),
            description: String::from("Red Hat VirtIO Ethernet Adapter"),
            mac: [0x52, 0x54, 0, 0x12, 0x34, 0x56],
            dhcp: true,
            ipv4: Some([10, 0, 2, 15]),
            mask: [255, 255, 255, 0],
            gateway: Some([10, 0, 2, 2]),
            dns: alloc::vec![[10, 0, 2, 3]],
            dhcp_server: Some([10, 0, 2, 2]),
            lease_obtained: Some(NOW),
            lease_expires: Some(NOW),
            connected: true,
            suffix: String::new(),
            ipv6: Some([
                0xfe, 0xc0, 0, 0, 0, 0, 0, 0, 0x50, 0x54, 0, 0xff, 0xfe, 0x12, 0x34, 0x56
            ]),
            link_local: Some([
                0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0x50, 0x54, 0, 0xff, 0xfe, 0x12, 0x34, 0x56
            ]),
            gateway6: Some([0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]),
            dns6: alloc::vec::Vec::new(),
            scope: 2,
            tunnel: false,
        }];
        // A connected VPN's tunnel adapter, addressed from its file.
        for (c, text) in &self.vpns {
            let Ok(config) = door_wireguard::Config::parse(text) else {
                continue;
            };
            if c.state == VpnState::Connected
                && let Some(a) = config.addresses.first()
            {
                list.push(Adapter {
                    name: c.name.clone(),
                    description: String::from("WireGuard Tunnel"),
                    ipv4: Some(a.address),
                    mask: (u32::MAX << (32 - u32::from(a.prefix.clamp(1, 32)))).to_be_bytes(),
                    dns: config.dns.clone(),
                    connected: true,
                    tunnel: true,
                    ..Adapter::default()
                });
            }
        }
        list
    }
    fn resolve(&mut self, name: &str) -> Option<[u8; 4]> {
        match name {
            "localhost" => Some([127, 0, 0, 1]),
            "example.com" => Some([93, 184, 215, 14]),
            _ => crate::cmd::parse_ip(name),
        }
    }
    fn ping(
        &mut self,
        address: [u8; 4],
        _bytes: usize,
        _timeout: u64,
    ) -> Result<Option<u64>, FsError> {
        Ok((address[0] == 127).then_some(0))
    }
    /// Two routers (10.0.0.1, 10.0.0.2), then 127.x answers; the rest
    /// times out.
    fn echo(
        &mut self,
        address: [u8; 4],
        bytes: usize,
        _timeout: u64,
        ttl: u8,
    ) -> Result<crate::host::Echo, FsError> {
        use crate::host::Echo;
        if address[0] != 127 {
            return Ok(Echo::TimedOut);
        }
        Ok(match ttl {
            1 | 2 => Echo::TtlExpired {
                from: [10, 0, 0, ttl],
                ms: u64::from(ttl),
            },
            _ => Echo::Reply {
                from: address,
                ms: 0,
                ttl: 128,
                bytes,
            },
        })
    }
    fn connections(&mut self) -> Vec<crate::host::Connection> {
        self.connections.clone()
    }
    fn statistics(&mut self) -> Option<(crate::host::ProtocolStats, crate::host::ProtocolStats)> {
        let mut v4 = crate::host::ProtocolStats::default();
        v4.ip[0] = 120;
        v4.ip[7] = 98;
        v4.icmp_messages = (4, 4);
        v4.icmp_types = alloc::vec![(0, 4, 0), (8, 0, 4)];
        v4.tcp = [5, 1, 2, 0, 1, 80, 70, 3];
        v4.udp = [12, 1, 0, 13];
        let v6 = crate::host::ProtocolStats {
            icmp_messages: (1, 4),
            icmp_types: alloc::vec![(133, 0, 3), (134, 1, 0), (135, 0, 1)],
            ..Default::default()
        };
        Some((v4, v6))
    }
    fn neighbors(&mut self) -> Vec<crate::host::Neighbor> {
        alloc::vec![
            crate::host::Neighbor {
                address: [10, 0, 2, 2],
                mac: [0x52, 0x55, 10, 0, 2, 2],
                dynamic: true,
            },
            crate::host::Neighbor {
                address: [10, 0, 2, 255],
                mac: [0xff; 6],
                dynamic: false,
            },
        ]
    }
    fn system(&mut self) -> SystemFacts {
        SystemFacts {
            os_name: String::from("Macrohard Doors 11 Pro"),
            os_version: String::from("10.0.26300 N/A Build 26300"),
            processors: alloc::vec![String::from(
                "Intel64 Family 6 Model 85 Stepping 7 GenuineIntel ~2600 Mhz"
            )],
            ..SystemFacts::default()
        }
    }
    fn set_clipboard(&mut self, text: &str) -> Result<(), FsError> {
        self.clipboard = Some(String::from(text));
        Ok(())
    }
    fn reg_values(&mut self, key: &str) -> Result<Vec<(String, RegValue)>, FsError> {
        self.registry.get(key).cloned().ok_or(FsError::NotFound)
    }
    fn reg_subkeys(&mut self, key: &str) -> Result<Vec<String>, FsError> {
        let prefix = format!("{key}\\");
        Ok(self
            .registry
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix))
            .filter(|k| !k.contains('\\'))
            .map(String::from)
            .collect())
    }
    fn firewall_reload(&mut self) {
        self.firewall_reloads += 1;
    }
    fn reg_set(&mut self, key: &str, name: &str, value: &RegValue) -> Result<(), FsError> {
        // HKLM takes an administrator's token.
        if key.starts_with("HKLM") && !self.elevated {
            return Err(FsError::Denied);
        }
        let values = self.registry.entry(String::from(key)).or_default();
        values.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        values.push((String::from(name), value.clone()));
        Ok(())
    }
    fn reg_delete(&mut self, key: &str, name: Option<&str>) -> Result<(), FsError> {
        if key.starts_with("HKLM") && !self.elevated {
            return Err(FsError::Denied);
        }
        match name {
            Some(name) => {
                let values = self.registry.get_mut(key).ok_or(FsError::NotFound)?;
                values.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
            }
            None => {
                let prefix = format!("{key}\\");
                self.registry
                    .retain(|k, _| k != key && !k.starts_with(&prefix));
            }
        }
        Ok(())
    }
    fn set_user_variable(&mut self, name: &str, value: &str) -> Result<(), FsError> {
        self.user_vars
            .push((String::from(name), String::from(value)));
        Ok(())
    }
}

//! What the engines need from the system. Paths here are native Doors
//! paths (`/users/1001/Documents`); the engines translate the Windows paths
//! users type with [`Host::native`] / [`Host::windows`]. Every operation is
//! performed with the caller's own authority: the engines hold none.
use alloc::{format, string::String, vec::Vec};

/// `FILE_ATTRIBUTE_*` bits.
pub mod attr {
    pub const READONLY: u32 = 0x1;
    pub const HIDDEN: u32 = 0x2;
    pub const SYSTEM: u32 = 0x4;
    pub const DIRECTORY: u32 = 0x10;
    pub const ARCHIVE: u32 = 0x20;
    pub const NORMAL: u32 = 0x80;
    pub const OFFLINE: u32 = 0x1000;
    pub const NOT_CONTENT_INDEXED: u32 = 0x2000;
    pub const REPARSE: u32 = 0x400;
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    pub dir: bool,
    pub size: u64,
    /// FILETIME (100 ns since 1601, UTC).
    pub modified: u64,
    pub created: u64,
    pub accessed: u64,
    pub attrs: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub meta: Meta,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsError {
    NotFound,
    PathNotFound,
    Exists,
    Denied,
    NotEmpty,
    InUse,
    Invalid,
    Full,
    TooLarge,
    Unsupported,
    Device,
}
impl FsError {
    /// The Windows system message cmd.exe prints for this error.
    pub fn message(self) -> &'static str {
        match self {
            Self::NotFound => "The system cannot find the file specified.",
            Self::PathNotFound => "The system cannot find the path specified.",
            Self::Exists => "Cannot create a file when that file already exists.",
            Self::Denied => "Access is denied.",
            Self::NotEmpty => "The directory is not empty.",
            Self::InUse => {
                "The process cannot access the file because it is being used by another process."
            }
            Self::Invalid => "The filename, directory name, or volume label syntax is incorrect.",
            Self::Full => "There is not enough space on the disk.",
            Self::TooLarge => "Not enough memory resources are available to process this command.",
            Self::Unsupported => "The request is not supported.",
            Self::Device => "The device is not ready.",
        }
    }
    /// The Win32 error code (`%ERRORLEVEL%` after a failed internal command
    /// is usually 1; tools report these).
    pub fn code(self) -> i32 {
        match self {
            Self::NotFound => 2,
            Self::PathNotFound => 3,
            Self::Denied => 5,
            Self::Invalid => 123,
            Self::Exists => 183,
            Self::NotEmpty => 145,
            Self::InUse => 32,
            Self::Full => 112,
            Self::TooLarge => 8,
            Self::Unsupported => 50,
            Self::Device => 21,
        }
    }
}
/// Local wall-clock time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalTime {
    pub year: u32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub centis: u32,
    /// 0 = Sunday.
    pub weekday: u32,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Volume {
    pub label: String,
    pub serial: u32,
    pub filesystem: String,
    pub total: u64,
    pub free: u64,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub name: String,
    pub session_name: String,
    pub session: u32,
    pub memory_kb: u64,
    pub user: String,
    pub cpu_seconds: u64,
    pub title: String,
}
/// What one ICMP echo came back with (ping, tracert).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Echo {
    /// An echo reply: who sent it, the round trip, its TTL and data size.
    Reply {
        from: [u8; 4],
        ms: u64,
        ttl: u8,
        bytes: usize,
    },
    TimedOut,
    /// A router's Time Exceeded for the request's TTL.
    TtlExpired {
        from: [u8; 4],
        ms: u64,
    },
    /// Destination unreachable, as `from` reported it (IP_STATUS code).
    Unreachable {
        from: [u8; 4],
        status: u32,
    },
    /// No address, no route: "General failure."
    Failed,
}
/// One TCP or UDP socket of the network stack (netstat).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Connection {
    pub tcp: bool,
    /// MIB_TCP_STATE (2 listening, 5 established, ...); 0 for UDP.
    pub state: u8,
    pub local: [u8; 4],
    pub local_port: u16,
    pub remote: [u8; 4],
    pub remote_port: u16,
    pub pid: u32,
    /// An AF_INET6 socket: its addresses and their scope ids (then
    /// `local`/`remote` are unused).
    pub v6: Option<Ipv6Ends>,
}
/// An AF_INET6 socket's addresses with their scope ids (netstat prints
/// `[fe80::1%2]:port`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ipv6Ends {
    pub local: [u8; 16],
    pub local_scope: u32,
    pub remote: [u8; 16],
    pub remote_scope: u32,
}
/// One family's protocol counters, as `netstat -s` prints them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProtocolStats {
    /// IP: received, header errors, address errors, forwarded, unknown
    /// protocols, discarded, delivered, output requests, routing discards,
    /// discarded output, no route, reassembly required, reassembled,
    /// reassembly failures, fragmented, fragmentation failures, fragments
    /// created.
    pub ip: [u32; 17],
    /// ICMP messages and errors (received, sent).
    pub icmp_messages: (u32, u32),
    pub icmp_errors: (u32, u32),
    /// ICMP messages by type: (type, received, sent).
    pub icmp_types: Vec<(u8, u32, u32)>,
    /// TCP: active opens, passive opens, failed attempts, reset
    /// connections, current connections, segments received, sent,
    /// retransmitted.
    pub tcp: [u32; 8],
    /// UDP: datagrams received, no ports, receive errors, datagrams sent.
    pub udp: [u32; 4],
}
impl ProtocolStats {
    /// (received, sent) of ICMP message type `kind`.
    pub fn icmp(&self, kind: u8) -> (u32, u32) {
        self.icmp_types
            .iter()
            .find(|t| t.0 == kind)
            .map_or((0, 0), |t| (t.1, t.2))
    }
}
/// One ARP-table row (`arp -a`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Neighbor {
    pub address: [u8; 4],
    pub mac: [u8; 6],
    pub dynamic: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Adapter {
    pub name: String,
    pub description: String,
    pub mac: [u8; 6],
    pub dhcp: bool,
    pub ipv4: Option<[u8; 4]>,
    pub mask: [u8; 4],
    pub gateway: Option<[u8; 4]>,
    pub dns: Vec<[u8; 4]>,
    pub dhcp_server: Option<[u8; 4]>,
    pub lease_obtained: Option<LocalTime>,
    pub lease_expires: Option<LocalTime>,
    pub connected: bool,
    pub suffix: String,
    /// IPv6: the autoconfigured and link-local addresses, the router and
    /// RDNSS servers, and the interface index shown after `%`.
    pub ipv6: Option<[u8; 16]>,
    pub link_local: Option<[u8; 16]>,
    pub gateway6: Option<[u8; 16]>,
    pub dns6: Vec<[u8; 16]>,
    pub scope: u32,
    /// A VPN tunnel's virtual adapter (no physical address).
    pub tunnel: bool,
}
/// A service as the Service Control Manager and its registry key describe
/// it (`sc`, `net start`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Service {
    pub name: String,
    pub display: String,
    pub description: String,
    /// SERVICE_STATUS dwCurrentState (1 stopped .. 7 paused).
    pub state: u32,
    pub pid: u32,
    /// SERVICE_ACCEPT_* bits.
    pub accepted: u32,
    pub exit_code: u32,
    /// SERVICE_WIN32_OWN_PROCESS (0x10), ...
    pub kind: u32,
    /// Start type: 2 auto, 3 demand, 4 disabled.
    pub start: u32,
    pub binary: String,
    /// The services it depends on (DependOnService).
    pub dependencies: Vec<String>,
}
/// An Event Log entry (`wevtutil qe`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostEvent {
    pub record: u32,
    /// FILETIME (UTC).
    pub time: u64,
    pub id: u32,
    /// EVENTLOG_*_TYPE.
    pub kind: u16,
    pub source: String,
    pub message: String,
}
/// A scheduled task (`schtasks /query`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostTask {
    pub name: String,
    pub command: String,
    /// 1 ready, 2 running, 3 disabled.
    pub state: u8,
    /// FILETIME (UTC); 0: none.
    pub next_run: u64,
    pub last_run: u64,
    pub last_result: u32,
}
/// The classic log a `wevtutil`/`eventcreate` name means.
pub fn log_name(name: &str) -> Option<&'static str> {
    ["Application", "System", "Security", "Setup"]
        .into_iter()
        .find(|l| l.eq_ignore_ascii_case(name))
}

/// A VPN connection (WireGuard) as RASDIAL and WIREGUARD see it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VpnConnection {
    pub name: String,
    /// The server as configured ("vpn.example.com:51820").
    pub server: String,
    pub state: VpnState,
    /// The last attempt's failure as a Remote Access error (0: none).
    pub error: u32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VpnState {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Failed,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemFacts {
    pub os_name: String,
    pub os_version: String,
    pub manufacturer: String,
    pub model: String,
    pub system_type: String,
    pub processors: Vec<String>,
    pub bios: String,
    pub memory_total_mb: u64,
    pub memory_free_mb: u64,
    pub boot_time: LocalTime,
    pub install_date: LocalTime,
    pub time_zone: String,
    pub locale: String,
    pub owner: String,
    pub product_id: String,
}
/// A registry value as `reg.exe` shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegValue {
    Sz(String),
    ExpandSz(String),
    MultiSz(Vec<String>),
    Dword(u32),
    Qword(u64),
    Binary(Vec<u8>),
    None,
}
impl RegValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Sz(_) => "REG_SZ",
            Self::ExpandSz(_) => "REG_EXPAND_SZ",
            Self::MultiSz(_) => "REG_MULTI_SZ",
            Self::Dword(_) => "REG_DWORD",
            Self::Qword(_) => "REG_QWORD",
            Self::Binary(_) => "REG_BINARY",
            Self::None => "REG_NONE",
        }
    }
}

/// The system an engine runs on. Defaults describe a host without that
/// facility; the Terminal host implements what Doors provides.
pub trait Host {
    /// Native path for a full Windows path (`C:\Users\x`), None when the
    /// drive or namespace does not exist.
    fn native(&self, windows: &str) -> Option<String>;
    /// Windows spelling of a native path (`/compat/Windows` -> `C:\Windows`).
    fn windows(&self, native: &str) -> String;

    fn stat(&mut self, path: &str) -> Result<Meta, FsError>;
    fn list(&mut self, path: &str) -> Result<Vec<Entry>, FsError>;
    /// The stored spelling of a native path: Windows names are not case
    /// sensitive (`C:\Windows\system32` is `/compat/Windows/System32`), so
    /// each component that does not exist as typed is matched without
    /// regard to ASCII case. None when a component does not exist at all.
    fn exact(&mut self, native: &str) -> Option<String> {
        if self.stat(native).is_ok() {
            return Some(String::from(native));
        }
        let mut out = String::new();
        for part in native.split('/').filter(|p| !p.is_empty()) {
            let typed = format!("{out}/{part}");
            if self.stat(&typed).is_ok() {
                out = typed;
                continue;
            }
            let parent = if out.is_empty() {
                String::from("/")
            } else {
                out.clone()
            };
            let entry = self
                .list(&parent)
                .ok()?
                .into_iter()
                .find(|e| e.name.eq_ignore_ascii_case(part))?;
            out = format!("{out}/{}", entry.name);
        }
        Some(out)
    }
    fn read(&mut self, path: &str, limit: usize) -> Result<Vec<u8>, FsError>;
    /// Create or replace (`append` false) or extend a file.
    fn write(&mut self, path: &str, data: &[u8], append: bool) -> Result<(), FsError>;
    fn mkdir(&mut self, path: &str) -> Result<(), FsError>;
    fn remove_file(&mut self, path: &str) -> Result<(), FsError>;
    /// Remove an empty directory.
    fn remove_dir(&mut self, path: &str) -> Result<(), FsError>;
    fn rename(&mut self, from: &str, to: &str) -> Result<(), FsError>;
    /// Copy one file, keeping its timestamps where the host can.
    fn copy_file(&mut self, from: &str, to: &str) -> Result<(), FsError> {
        let data = self.read(from, usize::MAX)?;
        self.write(to, &data, false)
    }
    fn set_attributes(&mut self, path: &str, attrs: u32) -> Result<(), FsError> {
        let _ = (path, attrs);
        Err(FsError::Denied)
    }
    fn volume(&mut self, drive: char) -> Option<Volume>;

    fn now(&self) -> LocalTime;
    /// A file time in local time (the default treats local as UTC).
    fn local(&self, filetime: u64) -> LocalTime {
        crate::format::civil(filetime, 0)
    }
    /// Milliseconds since boot.
    fn ticks(&self) -> u64;
    fn random(&mut self) -> u32;
    fn computer_name(&self) -> String;
    fn user_name(&self) -> String;
    fn domain(&self) -> String {
        self.computer_name()
    }
    /// The user's SID string (`whoami /user`).
    fn user_sid(&self) -> String {
        String::from("S-1-5-21-1001")
    }
    fn elevated(&self) -> bool {
        false
    }
    /// The account is an administrator (its unelevated token keeps
    /// Administrators for deny only); a standard user has no such group.
    fn administrator(&self) -> bool {
        true
    }
    fn processors(&self) -> u32 {
        1
    }

    fn processes(&mut self) -> Vec<Process> {
        Vec::new()
    }
    fn terminate(&mut self, pid: u32, force: bool) -> Result<(), FsError> {
        let _ = (pid, force);
        Err(FsError::Denied)
    }
    fn adapters(&mut self) -> Vec<Adapter> {
        Vec::new()
    }
    /// DNS lookup (IPv4).
    fn resolve(&mut self, name: &str) -> Option<[u8; 4]> {
        let _ = name;
        None
    }
    /// One ICMP echo; the round trip in ms, None on timeout. Hosts without
    /// raw sockets return Err(Unsupported).
    fn ping(
        &mut self,
        address: [u8; 4],
        bytes: usize,
        timeout_ms: u64,
    ) -> Result<Option<u64>, FsError> {
        let _ = (address, bytes, timeout_ms);
        Err(FsError::Unsupported)
    }
    /// One ICMP echo with `ttl` (ping -i, tracert). Hosts with only
    /// `ping` answer through it.
    fn echo(
        &mut self,
        address: [u8; 4],
        bytes: usize,
        timeout_ms: u64,
        ttl: u8,
    ) -> Result<Echo, FsError> {
        let _ = ttl;
        Ok(match self.ping(address, bytes, timeout_ms)? {
            Some(ms) => Echo::Reply {
                from: address,
                ms,
                ttl: 128,
                bytes,
            },
            None => Echo::TimedOut,
        })
    }
    /// The network stack's TCP and UDP sockets (netstat).
    fn connections(&mut self) -> Vec<Connection> {
        Vec::new()
    }
    /// IPv4 and IPv6 protocol counters (netstat -s); None without a stack.
    fn statistics(&mut self) -> Option<(ProtocolStats, ProtocolStats)> {
        None
    }
    /// The Ethernet adapter's ARP table (arp -a).
    fn neighbors(&mut self) -> Vec<Neighbor> {
        Vec::new()
    }
    /// The user's VPN connections.
    /// Every service the Service Control Manager knows (Win32 error when
    /// it can't be reached).
    fn services(&mut self) -> Result<Vec<Service>, u32> {
        Err(1063)
    }
    /// One service's status and configuration (1060: no such service).
    fn service(&mut self, name: &str) -> Result<Service, u32> {
        self.services()?
            .into_iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
            .ok_or(1060)
    }
    /// StartService with `arguments` (sc start's words after the name).
    fn service_start(&mut self, _name: &str, _arguments: &[String]) -> Result<Service, u32> {
        Err(1063)
    }
    /// A control (1 stop, 2 pause, 3 continue, 4 interrogate): the status
    /// after it.
    fn service_control(&mut self, _name: &str, _control: u32) -> Result<Service, u32> {
        Err(1063)
    }
    /// Wait (about 10 s at most) until `name` is in `state`.
    fn service_wait(&mut self, name: &str, state: u32) -> Result<(), u32> {
        for _ in 0..100 {
            if self.service(name)?.state == state {
                return Ok(());
            }
            self.sleep_ms(100);
        }
        Err(1053)
    }
    fn service_create(&mut self, _service: &Service) -> Result<(), u32> {
        Err(5)
    }
    fn service_delete(&mut self, _name: &str) -> Result<(), u32> {
        Err(5)
    }
    fn service_configure(
        &mut self,
        _name: &str,
        _start: Option<u32>,
        _binary: Option<&str>,
        _display: Option<&str>,
        _description: Option<&str>,
    ) -> Result<(), u32> {
        Err(5)
    }
    /// Replace a service's dependencies (DependOnService).
    fn service_set_dependencies(&mut self, _name: &str, _on: &[String]) -> Result<(), u32> {
        Err(5)
    }
    /// A short wait (polling loops).
    fn sleep_ms(&mut self, _ms: u64) {}
    /// Report an event (eventcreate): `log` APPLICATION or SYSTEM.
    fn report_event(
        &mut self,
        _log: &str,
        _kind: u16,
        _id: u32,
        _source: &str,
        _message: &str,
    ) -> Result<(), u32> {
        Err(1722)
    }
    /// Events of `log`, oldest first (or newest first), at most `count`.
    fn events(
        &mut self,
        _log: &str,
        _newest_first: bool,
        _count: usize,
    ) -> Result<Vec<HostEvent>, u32> {
        Err(15007)
    }
    fn clear_log(&mut self, _log: &str) -> Result<(), u32> {
        Err(5)
    }
    fn tasks(&mut self) -> Result<Vec<HostTask>, u32> {
        Err(1722)
    }
    /// schtasks /create: `kind` 0 once, 1 minute, 2 hourly, 3 daily, 4 on
    /// logon; `start` the local time of day (today, or tomorrow when it has
    /// passed for ONCE).
    fn task_create(
        &mut self,
        _name: &str,
        _command: &str,
        _kind: u8,
        _modifier: u16,
        _start: Option<(u32, u32)>,
        _replace: bool,
    ) -> Result<(), u32> {
        Err(5)
    }
    fn task_delete(&mut self, _name: &str) -> Result<(), u32> {
        Err(2)
    }
    fn task_run(&mut self, _name: &str) -> Result<(), u32> {
        Err(2)
    }
    fn task_end(&mut self, _name: &str) -> Result<(), u32> {
        Err(2)
    }
    fn task_enable(&mut self, _name: &str, _enabled: bool) -> Result<(), u32> {
        Err(2)
    }
    /// A FILETIME as local (year, month, day, hour, minute, second); None
    /// for 0.
    fn local_time(&mut self, _filetime: u64) -> Option<(i64, u32, u32, u32, u32, u32)> {
        None
    }

    fn vpn_connections(&mut self) -> Vec<VpnConnection> {
        Vec::new()
    }
    /// Save connection `name` from a validated wg-quick `text`.
    fn vpn_save(&mut self, name: &str, text: &str) -> Result<(), FsError> {
        let _ = (name, text);
        Err(FsError::Unsupported)
    }
    fn vpn_remove(&mut self, name: &str) -> Result<(), FsError> {
        let _ = name;
        Err(FsError::Unsupported)
    }
    /// Start connecting (`vpn_connections` reports how it goes).
    fn vpn_connect(&mut self, name: &str) -> Result<(), FsError> {
        let _ = name;
        Err(FsError::Unsupported)
    }
    fn vpn_disconnect(&mut self, name: &str) -> Result<(), FsError> {
        let _ = name;
        Err(FsError::Unsupported)
    }
    /// The firewall's rules or profiles changed in the registry: have the
    /// network stack load them again.
    fn firewall_reload(&mut self) {}
    fn system(&mut self) -> SystemFacts {
        SystemFacts::default()
    }
    /// Text for the clipboard (`clip`).
    fn set_clipboard(&mut self, text: &str) -> Result<(), FsError> {
        let _ = text;
        Err(FsError::Unsupported)
    }
    fn clipboard(&mut self) -> Option<String> {
        None
    }

    /// `HKLM\...`-style key reads for `reg query`, `assoc`, `ftype`.
    fn reg_values(&mut self, key: &str) -> Result<Vec<(String, RegValue)>, FsError> {
        let _ = key;
        Err(FsError::NotFound)
    }
    fn reg_subkeys(&mut self, key: &str) -> Result<Vec<String>, FsError> {
        let _ = key;
        Err(FsError::NotFound)
    }
    fn reg_set(&mut self, key: &str, name: &str, value: &RegValue) -> Result<(), FsError> {
        let _ = (key, name, value);
        Err(FsError::Denied)
    }
    fn reg_delete(&mut self, key: &str, name: Option<&str>) -> Result<(), FsError> {
        let _ = (key, name);
        Err(FsError::Denied)
    }
    /// Persist a user environment variable (`setx`).
    fn set_user_variable(&mut self, name: &str, value: &str) -> Result<(), FsError> {
        let _ = (name, value);
        Err(FsError::Denied)
    }
    /// SHA-256 / SHA-1 / MD5 of a file for `certutil -hashfile`, when the
    /// host links those digests.
    fn digest(&mut self, path: &str, algorithm: &str) -> Result<Vec<u8>, FsError> {
        let _ = (path, algorithm);
        Err(FsError::Unsupported)
    }
}
/// Days since 1970-01-01 to (year, month, day) (Howard Hinnant's
/// civil_from_days).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

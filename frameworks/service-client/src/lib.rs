#![no_std]
//! Native clients communicate with actual protected services. No host filesystem,
//! console, clock or identity fallback is provided.
mod apps;
mod files;
mod replacement;
pub use apps::Applications;
pub use replacement::{LargeReplacement, Replacement};
mod registry;
mod rpc;
mod storage_connect;
pub use storage_connect::StorageConnect;
pub mod accessibility;
mod clipboard;
mod credential;
pub use credential::{Credentials, Reply as CredentialReply};
mod eventlog;
pub use eventlog::EventLog;
mod tasks;
pub use tasks::Tasks;
mod services;
pub use services::{ServiceEntry, ServicePage, Services};
mod device;
pub use device::{DeviceName, device_name, stored_device_name};
mod com;
mod firewall;
pub use com::{ComBroker, Event as ComEvent};
pub use firewall::Firewall;
pub mod audio;
mod dnd;
pub mod network;
pub use network::{Addresses, Network};
pub mod notifications;
pub use notifications::Notifications;
pub mod text_input;
pub mod window;
use door_abi::{Handle, Transfer, rights};
pub use door_service_protocol::files::{DirEntry, FileInfo, FileTimes};
use door_service_protocol::{CHUNK_BYTES, Code, PATH_BYTES, manifest, opcode};
pub use registry::{Key, RegistryRoot, RegistryValue, RegistryView};
pub use registry::{RegistryEntry, RegistryName};
use rpc::Connection;
pub use rpc::{Error, Result};

pub struct Init {
    connection: Connection,
}
impl Init {
    /// Window's event loop must never wait for Init to finish loading an ELF.
    pub fn begin_desktop(&mut self, command: u8, now: u64) -> Result<()> {
        self.connection
            .begin(door_service_protocol::application::DESKTOP, &[command], now)
    }
    /// The EXECUTE_ELEVATED body for `begin_execute_elevated` and its
    /// length, or InvalidRequest when it does not fit one message: the
    /// display service checks this before it shows the prompt.
    pub fn execute_elevated_body(
        image: &[u8; 32],
        path: &str,
        cwd: &str,
        extra: &str,
    ) -> Result<([u8; door_service_protocol::BODY_BYTES], usize)> {
        use door_service_protocol::BODY_BYTES;
        // argv[0] is the program's Windows path, NUL-terminated, as
        // Explorer passes it (what GetModuleFileName answers the program);
        // then the arguments the user approved, if any.
        let mut arguments = [0; BODY_BYTES];
        let spelled =
            door_service_protocol::shell::windows_spelling(path, &mut arguments[..BODY_BYTES - 1])
                .ok_or(Error::Service(Code::InvalidRequest))?;
        let mut argument_bytes = spelled + 1;
        // `extra`: further arguments, each NUL-terminated (an empty
        // argument is a lone NUL).
        if !extra.is_empty() {
            let end = argument_bytes + extra.len();
            if end > BODY_BYTES || !extra.ends_with('\0') {
                return Err(Error::Service(Code::InvalidRequest));
            }
            arguments[argument_bytes..end].copy_from_slice(extra.as_bytes());
            argument_bytes = end;
        }
        let mut body = [0; BODY_BYTES];
        let mut execute = [0; BODY_BYTES];
        let count = door_service_protocol::application::Execute {
            path,
            cwd,
            arguments: &arguments[..argument_bytes],
            staged: false,
            options: door_service_protocol::application::ExecuteOptions::NONE,
        }
        .encode(&mut execute)
        .map_err(Error::Service)?;
        if 32 + count > BODY_BYTES {
            return Err(Error::Service(Code::InvalidRequest));
        }
        body[..32].copy_from_slice(image);
        body[32..32 + count].copy_from_slice(&execute[..count]);
        Ok((body, 32 + count))
    }
    /// Display service, after User Account Control approved Run as
    /// administrator: start `path` (whose bytes hash to `image`) as the
    /// session user in `cwd` (`directory` is its handle) with `extra`, its
    /// further arguments, each NUL-terminated. The reply, the child process
    /// handle, arrives through `poll`.
    pub fn begin_execute_elevated(
        &mut self,
        image: &[u8; 32],
        path: &str,
        cwd: &str,
        extra: &str,
        directory: Handle,
        now: u64,
    ) -> Result<()> {
        let (body, length) = Self::execute_elevated_body(image, path, cwd, extra)?;
        self.connection.begin_transfer(
            door_service_protocol::application::EXECUTE_ELEVATED,
            &body[..length],
            &[Transfer {
                handle: directory,
                rights: rights::READ | rights::QUERY | rights::DUPLICATE | rights::TRANSFER,
                mode: 0,
            }],
            now,
        )
    }
    /// Display service: ask Init to persist the session user's shell layout.
    /// Completion arrives through `poll_desktop_command`.
    pub fn begin_shell_layout(
        &mut self,
        layout: door_service_protocol::desktop::ShellLayout,
        now: u64,
    ) -> Result<()> {
        let mut body = [0u8; 8];
        body[..4].copy_from_slice(&layout.flags().to_le_bytes());
        body[4..].copy_from_slice(&layout.order().to_le_bytes());
        self.connection
            .begin(door_service_protocol::application::SHELL_LAYOUT, &body, now)
    }
    /// Display service: ask Init to persist and apply desktop controls (Init
    /// writes storage, then SET_DESKTOP_CONTROLS reaches every client).
    pub fn begin_controls_persist(
        &mut self,
        controls: door_service_protocol::desktop::Controls,
        now: u64,
    ) -> Result<()> {
        self.connection.begin(
            door_service_protocol::application::DESKTOP_CONTROLS_PERSIST,
            &controls.bits().to_le_bytes(),
            now,
        )
    }
    pub fn begin_desktop_state_v2(&mut self, now: u64) -> Result<()> {
        self.connection.begin(
            door_service_protocol::application::DESKTOP_STATE_V2,
            &[],
            now,
        )
    }
    pub fn poll_desktop_state_v2(
        &mut self,
        now: u64,
    ) -> Result<Option<door_service_protocol::application::DesktopState>> {
        self.connection
            .poll(now)?
            .map(|reply| {
                door_service_protocol::application::DesktopState::decode(reply.data()?)
                    .map_err(|_| Error::Malformed)
            })
            .transpose()
    }
    pub fn poll_desktop_state(&mut self, now: u64) -> Result<Option<[u8; 104]>> {
        self.connection
            .poll(now)?
            .map(|reply| reply.data()?.try_into().map_err(|_| Error::Malformed))
            .transpose()
    }
    /// The started child's process handle (the caller closes it).
    pub fn poll_execute_elevated(&mut self, now: u64) -> Result<Option<Handle>> {
        self.connection
            .poll(now)?
            .map(|reply| reply.one_handle())
            .transpose()
    }
    pub fn poll_desktop_command(&mut self, now: u64) -> Result<Option<()>> {
        self.connection
            .poll(now)?
            .map(|reply| reply.empty())
            .transpose()
    }
    pub fn desktop_state(&mut self) -> Result<[u8; 104]> {
        let reply = self
            .connection
            .call(door_service_protocol::application::DESKTOP, &[0], &[])?;
        reply.data()?.try_into().map_err(|_| Error::Malformed)
    }
    pub fn desktop_command(&mut self, command: u8) -> Result<()> {
        self.connection
            .call(door_service_protocol::application::DESKTOP, &[command], &[])?
            .empty()
    }
    pub fn terminate_window_client(&mut self, pid: u64) -> Result<()> {
        self.connection
            .call(
                door_service_protocol::window::TERMINATE_CLIENT,
                &pid.to_le_bytes(),
                &[],
            )?
            .empty()
    }
    pub fn connect(port: Handle) -> Result<Self> {
        Ok(Self {
            connection: Connection::connect(port, manifest::INIT)?,
        })
    }
    /// `connect` that never waits for a slot in Init's accept queue
    /// (WouldBlock when it is full): the display service's background
    /// requests, which retry. Waiting there stalled the display service
    /// while Init was calling it (a restarted server's session bind).
    pub fn connect_now(port: Handle) -> Result<Self> {
        Ok(Self {
            connection: Connection::connect_now(port, manifest::INIT)?,
        })
    }
    pub fn resolve(&mut self, service: u64) -> Result<Handle> {
        let reply = self
            .connection
            .call(opcode::RESOLVE, &service.to_le_bytes(), &[])?;
        reply.one_handle()
    }
    /// Start resolving `service` without waiting, for a caller Init itself
    /// may be calling at that moment (the display service): a blocking
    /// resolve would deadlock both until their deadlines.
    pub fn begin_resolve(&mut self, service: u64, now: u64) -> Result<()> {
        self.connection
            .begin(opcode::RESOLVE, &service.to_le_bytes(), now)
    }
    /// The service's port once Init has answered `begin_resolve`.
    pub fn poll_resolve(&mut self, now: u64) -> Result<Option<Handle>> {
        self.connection
            .poll(now)?
            .map(|reply| reply.one_handle())
            .transpose()
    }
}
fn connect(registry: Handle, service: u64) -> Result<Connection> {
    let mut init = Init::connect(registry)?;
    let port = init.resolve(service)?;
    let result = Connection::connect(port, service);
    let _ = door_sys::close(port);
    result
}
#[derive(Debug)]
pub struct File {
    token: u64,
    owner: (Handle, u64),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Disposition {
    CreateNew = 0,
    OpenExisting = 1,
    OpenAlways = 2,
    CreateAlways = 3,
    TruncateExisting = 4,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Setting {
    Theme = 0,
    /// Display scale: 0 = automatic (host hint, else integer heuristic) or
    /// 100..=300 percent in steps of 25. Applied live by the window server.
    ScalePercent = 1,
    DesktopAppearance = 2,
    DesktopControls = 3,
    /// `desktop::ShellLayout` flags, written by Init for the display service.
    ShellLayout = 4,
    /// `desktop::ShellLayout` order.
    ShellOrder = 5,
    /// Installed keyboards and input methods with the current one
    /// (`door_keyboard_layouts::Preferences` bits; default 1 = US). Written
    /// by Settings > Time & language > Typing, applied by Init at sign-in.
    InputMethods = 6,
    /// Time zone (`door_locale::tz::Zone::id`, default 16 = UTC). Written by
    /// Settings > Time & language > Date & time; the taskbar clock follows it.
    TimeZone = 7,
    /// Regional format (`door_locale::region::Region::id`, default 1 =
    /// en-US). Written by Settings > Time & language > Language & region.
    RegionalFormat = 8,
    /// System accent colour (`desktop::Accent` word; default 0 = Door's
    /// default blue). Written by Settings > Personalization > Colors, applied
    /// by Init at sign-in and by Settings live (`WindowClient::set_accent`).
    AccentColor = 9,
    /// `preferences::ACCENT_SURFACES` and the other P14 preference words
    /// (`door_service_protocol::preferences`, ids 10..=20).
    AccentSurfaces = 10,
    ThemeSchedule = 11,
    Notifications = 12,
    DndSchedule = 13,
    Focus = 14,
    SignInAway = 15,
    LockScreen = 16,
    RegionalOverrides = 17,
    Clipboard = 18,
    SearchHistory = 19,
    EndTask = 20,
    Taskbar = 21,
    StartFolders = 22,
    Background = 23,
    WindowAppearance = 24,
    Mouse = 25,
    Multitasking = 26,
    Vision = 27,
    /// Optional features (`preferences::features`).
    Features = 28,
    /// Sound output (`preferences::SOUND`).
    Sound = 29,
    /// Privacy & security > Microphone (`preferences::MICROPHONE`).
    Microphone = 30,
    /// Network & internet > VPN advanced settings (`preferences::VPN`).
    Vpn = 31,
    /// Personalization > Background > Recent images (`preferences::RecentBackgrounds`).
    RecentBackgrounds = 32,
    /// Personalization > Colors > Recent colors (`preferences::RecentAccents`).
    RecentAccents = 33,
}
impl Setting {
    /// The setting with wire id `id`.
    pub const fn from_id(id: u32) -> Option<Self> {
        Some(match id {
            0 => Self::Theme,
            1 => Self::ScalePercent,
            2 => Self::DesktopAppearance,
            3 => Self::DesktopControls,
            4 => Self::ShellLayout,
            5 => Self::ShellOrder,
            6 => Self::InputMethods,
            7 => Self::TimeZone,
            8 => Self::RegionalFormat,
            9 => Self::AccentColor,
            10 => Self::AccentSurfaces,
            11 => Self::ThemeSchedule,
            12 => Self::Notifications,
            13 => Self::DndSchedule,
            14 => Self::Focus,
            15 => Self::SignInAway,
            16 => Self::LockScreen,
            17 => Self::RegionalOverrides,
            18 => Self::Clipboard,
            19 => Self::SearchHistory,
            20 => Self::EndTask,
            21 => Self::Taskbar,
            22 => Self::StartFolders,
            23 => Self::Background,
            24 => Self::WindowAppearance,
            25 => Self::Mouse,
            26 => Self::Multitasking,
            27 => Self::Vision,
            28 => Self::Features,
            29 => Self::Sound,
            30 => Self::Microphone,
            31 => Self::Vpn,
            32 => Self::RecentBackgrounds,
            33 => Self::RecentAccents,
            _ => return None,
        })
    }
}
pub const READ: u16 = 1;
pub const WRITE: u16 = 2;
pub const DELETE: u16 = 4;
pub const SHARE_ALL: u16 = 7;
pub struct Storage {
    connection: Connection,
}
impl Storage {
    /// A fatal transport or authenticated malformed reply retires this channel.
    /// No operation is replayed. Existing File/Key/Replacement tokens stay stale.
    pub fn needs_reconnect(&self) -> bool {
        self.connection.needs_reconnect()
    }
    /// Explicitly release all connection-scoped authority before replacement.
    pub fn retire(&mut self) {
        self.connection.retire();
    }
    fn operation<T>(&mut self, operation: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let result = operation(self);
        if matches!(result, Err(Error::Malformed)) {
            self.connection.retire();
        }
        result
    }
    /// A path operation whose body may not fit one packet (a long path):
    /// LONG_PART pieces, then LONG_COMMIT runs `operation` on them.
    fn long_call(&mut self, operation: u32, body: &[u8]) -> Result<rpc::Reply> {
        use door_service_protocol::{BODY_BYTES, LONG_BYTES};
        if body.len() <= BODY_BYTES {
            return self.connection.call(operation, body, &[]);
        }
        if body.len() > LONG_BYTES {
            return Err(Error::Service(Code::InvalidRequest));
        }
        for piece in body.chunks(BODY_BYTES) {
            self.connection
                .call(opcode::LONG_PART, piece, &[])?
                .empty()?;
        }
        self.connection
            .call(opcode::LONG_COMMIT, &operation.to_le_bytes(), &[])
    }
    pub fn connect(registry: Handle) -> Result<Self> {
        Ok(Self {
            connection: connect(registry, manifest::STORAGE)?,
        })
    }
    pub fn open(
        &mut self,
        path: &str,
        disposition: Disposition,
        access: u16,
        share: u16,
    ) -> Result<File> {
        self.operation(|storage| {
            if path.is_empty() || path.len() > PATH_BYTES || path.as_bytes().contains(&0) {
                return Err(Error::Service(Code::InvalidRequest));
            }
            let mut body = [0u8; 8 + PATH_BYTES];
            body[0] = disposition as u8;
            body[2..4].copy_from_slice(&access.to_le_bytes());
            body[4..6].copy_from_slice(&share.to_le_bytes());
            body[6..8].copy_from_slice(&(path.len() as u16).to_le_bytes());
            body[8..8 + path.len()].copy_from_slice(path.as_bytes());
            let reply = storage.long_call(opcode::OPEN, &body[..8 + path.len()])?;
            Ok(File {
                token: reply.token()?,
                owner: storage.connection.key(),
            })
        })
    }
    pub fn read(&mut self, file: &File, offset: u64, output: &mut [u8]) -> Result<usize> {
        self.operation(|storage| {
            storage.check_file(file)?;
            if output.len() > CHUNK_BYTES {
                return Err(Error::Service(Code::InvalidRequest));
            }
            let mut body = [0u8; 24];
            body[..8].copy_from_slice(&file.token.to_le_bytes());
            body[8..16].copy_from_slice(&offset.to_le_bytes());
            body[16..24].copy_from_slice(&(output.len() as u64).to_le_bytes());
            let reply = storage.connection.call(opcode::READ, &body, &[])?;
            let data = reply.data()?;
            if data.len() > output.len() {
                return Err(Error::Malformed);
            }
            output[..data.len()].copy_from_slice(data);
            Ok(data.len())
        })
    }
    /// File bytes `offset..offset + bytes` into `section` from
    /// `section_offset` (READ_SECTION), `files::SECTION_IO_BYTES` per
    /// request, stopping at the end of the file: the count read. The caller
    /// keeps the handle; the service is lent QUERY|READ|WRITE|MAP.
    pub fn read_section(
        &mut self,
        file: &File,
        offset: u64,
        section: Handle,
        section_offset: u64,
        bytes: usize,
    ) -> Result<usize> {
        self.section_io(
            opcode::READ_SECTION,
            file,
            offset,
            section,
            section_offset,
            bytes,
        )
    }
    /// `section` bytes from `section_offset` written to the file at
    /// `offset` (WRITE_SECTION): the count written.
    pub fn write_section(
        &mut self,
        file: &File,
        offset: u64,
        section: Handle,
        section_offset: u64,
        bytes: usize,
    ) -> Result<usize> {
        self.section_io(
            opcode::WRITE_SECTION,
            file,
            offset,
            section,
            section_offset,
            bytes,
        )
    }
    fn section_io(
        &mut self,
        operation: u32,
        file: &File,
        offset: u64,
        section: Handle,
        section_offset: u64,
        bytes: usize,
    ) -> Result<usize> {
        use door_service_protocol::files::{SECTION_IO_BYTES, SectionIo};
        self.operation(|storage| {
            storage.check_file(file)?;
            let mut done = 0usize;
            while done < bytes {
                let want = (bytes - done).min(SECTION_IO_BYTES);
                let body = SectionIo::new(
                    file.token,
                    offset + done as u64,
                    section_offset + done as u64,
                    want,
                )
                .map_err(Error::Service)?
                .encode();
                let moved = storage
                    .connection
                    .call(
                        operation,
                        &body,
                        &[Transfer {
                            handle: section,
                            rights: rights::QUERY
                                | rights::READ
                                | rights::WRITE
                                | rights::MAP
                                | rights::TRANSFER,
                            mode: 0,
                        }],
                    )?
                    .scalar()?;
                let moved = usize::try_from(moved)
                    .ok()
                    .filter(|n| *n <= want)
                    .ok_or(Error::Malformed)?;
                done += moved;
                if moved < want {
                    break;
                }
            }
            Ok(done)
        })
    }
    /// Up to `output.len()` bytes at `offset`, a chunk (`CHUNK_BYTES`) per
    /// request, stopping at the end of the file: the count read.
    pub fn read_full(&mut self, file: &File, offset: u64, output: &mut [u8]) -> Result<usize> {
        let mut done = 0;
        while done < output.len() {
            let want = (output.len() - done).min(CHUNK_BYTES);
            let n = self.read(file, offset + done as u64, &mut output[done..done + want])?;
            done += n;
            if n < want {
                break;
            }
        }
        Ok(done)
    }
    /// Lossless Windows view. The server captures authenticated identity and
    /// selects its initial per-user directory; UTF-16 units are not UTF-8 paths.
    pub fn open_windows(
        &mut self,
        path: &[u16],
        disposition: Disposition,
        access: u16,
        share: u16,
    ) -> Result<(File, bool)> {
        self.operation(|storage| {
            if path.is_empty() || path.len() > PATH_BYTES / 2 || path.contains(&0) {
                return Err(Error::Service(Code::InvalidRequest));
            }
            let mut body = [0u8; 8 + PATH_BYTES];
            body[0] = disposition as u8;
            body[2..4].copy_from_slice(&access.to_le_bytes());
            body[4..6].copy_from_slice(&share.to_le_bytes());
            body[6..8].copy_from_slice(&((path.len() * 2) as u16).to_le_bytes());
            for (out, unit) in body[8..].chunks_exact_mut(2).zip(path) {
                out.copy_from_slice(&unit.to_le_bytes());
            }
            let reply = storage.long_call(opcode::OPEN_WINDOWS, &body[..8 + path.len() * 2])?;
            let bytes = reply.data()?;
            if bytes.len() != 16 || bytes[8..].iter().skip(1).any(|byte| *byte != 0) || bytes[8] > 1
            {
                return Err(Error::Malformed);
            }
            let token = u64::from_le_bytes(bytes[..8].try_into().unwrap());
            if token == 0 {
                return Err(Error::Malformed);
            }
            Ok((
                File {
                    token,
                    owner: storage.connection.key(),
                },
                bytes[8] == 1,
            ))
        })
    }
    pub fn write(&mut self, file: &File, offset: u64, input: &[u8]) -> Result<usize> {
        self.operation(|storage| {
            storage.check_file(file)?;
            if input.len() > CHUNK_BYTES {
                return Err(Error::Service(Code::InvalidRequest));
            }
            let mut body = [0u8; 16 + CHUNK_BYTES];
            body[..8].copy_from_slice(&file.token.to_le_bytes());
            body[8..16].copy_from_slice(&offset.to_le_bytes());
            body[16..16 + input.len()].copy_from_slice(input);
            let count = storage
                .connection
                .call(opcode::WRITE, &body[..16 + input.len()], &[])?
                .scalar()?;
            if count > input.len() as u64 {
                return Err(Error::Malformed);
            }
            Ok(count as usize)
        })
    }
    pub fn flush(&mut self) -> Result<()> {
        self.operation(|storage| storage.connection.call(opcode::FLUSH, &[], &[])?.empty())
    }
    pub fn close(&mut self, file: File) -> Result<()> {
        self.operation(|storage| {
            storage.check_file(&file)?;
            storage
                .connection
                .call(opcode::CLOSE, &file.token.to_le_bytes(), &[])?
                .empty()
        })
    }
    fn check_file(&self, file: &File) -> Result<()> {
        if file.owner != self.connection.key() {
            return Err(Error::Service(Code::InvalidHandle));
        }
        Ok(())
    }
    pub fn get_setting(&mut self, setting: Setting) -> Result<u32> {
        self.operation(|storage| {
            let value = storage
                .connection
                .call(opcode::GET_SETTING, &(setting as u32).to_le_bytes(), &[])?
                .scalar()?;
            u32::try_from(value).map_err(|_| Error::Malformed)
        })
    }
    /// Only immutable Init authority may read another user's setting.
    pub fn get_setting_for(&mut self, uid: u32, setting: Setting) -> Result<u32> {
        self.operation(|storage| {
            let mut body = [0; 8];
            body[..4].copy_from_slice(&uid.to_le_bytes());
            body[4..].copy_from_slice(&(setting as u32).to_le_bytes());
            u32::try_from(
                storage
                    .connection
                    .call(opcode::GET_SETTING_FOR, &body, &[])?
                    .scalar()?,
            )
            .map_err(|_| Error::Malformed)
        })
    }
    /// Only immutable Init authority may write another user's setting.
    pub fn set_setting_for(&mut self, uid: u32, setting: Setting, value: u32) -> Result<()> {
        self.operation(|storage| {
            let mut body = [0u8; 12];
            body[..4].copy_from_slice(&uid.to_le_bytes());
            body[4..8].copy_from_slice(&(setting as u32).to_le_bytes());
            body[8..].copy_from_slice(&value.to_le_bytes());
            storage
                .connection
                .call(opcode::SET_SETTING_FOR, &body, &[])?
                .empty()
        })
    }
    pub fn set_setting(&mut self, setting: Setting, value: u32) -> Result<()> {
        self.operation(|storage| {
            let mut body = [0u8; 8];
            body[..4].copy_from_slice(&(setting as u32).to_le_bytes());
            body[4..].copy_from_slice(&value.to_le_bytes());
            storage
                .connection
                .call(opcode::SET_SETTING, &body, &[])?
                .empty()
        })
    }
}
pub struct Console {
    connection: Connection,
}
impl Console {
    pub fn set_theme(&mut self, value: u32) -> Result<()> {
        self.control(1, value)
    }
    pub fn clear(&mut self) -> Result<()> {
        self.control(0, 0)
    }
    fn control(&mut self, command: u32, value: u32) -> Result<()> {
        let mut body = [0; 8];
        body[..4].copy_from_slice(&command.to_le_bytes());
        body[4..].copy_from_slice(&value.to_le_bytes());
        self.connection
            .call(opcode::CONSOLE_CONTROL, &body, &[])?
            .empty()
    }
    pub fn connect(registry: Handle) -> Result<Self> {
        Ok(Self {
            connection: connect(registry, manifest::CONSOLE)?,
        })
    }
    /// `connect`, then use the Terminal tab console the program's startup
    /// environment names (`DOOR_CONSOLE=<n>`, from Init), if any.
    pub fn connect_for(registry: Handle, environment: &[u8]) -> Result<Self> {
        use door_service_protocol::console;
        let mut this = Self::connect(registry)?;
        let named = environment.split(|b| *b == 0).find_map(|entry| {
            let value = entry.strip_prefix(console::ENVIRONMENT.as_bytes())?;
            core::str::from_utf8(value).ok()?.parse::<u8>().ok()
        });
        if let Some(n) = named.filter(|&n| n != 0 && usize::from(n) < console::CONSOLES) {
            this.command(console::Command::Attach(n))?;
        }
        Ok(this)
    }
    pub fn write(&mut self, text: &str) -> Result<usize> {
        if text.len() > CHUNK_BYTES {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let count = self
            .connection
            .call(opcode::CONSOLE_WRITE, text.as_bytes(), &[])?
            .scalar()?;
        if count > text.len() as u64 {
            return Err(Error::Malformed);
        }
        Ok(count as usize)
    }
    pub fn read(&mut self) -> Result<u8> {
        u8::try_from(
            self.connection
                .call(opcode::CONSOLE_READ, &[], &[])?
                .scalar()?,
        )
        .map_err(|_| Error::Malformed)
    }
    /// Session Terminal only: append the console text written since the last
    /// call (at most one chunk) into `out`; returns the number of bytes.
    pub fn mirror(&mut self, out: &mut [u8; CHUNK_BYTES]) -> Result<usize> {
        let reply = self.connection.call(opcode::CONSOLE_MIRROR, &[], &[])?;
        let data = reply.data()?;
        if data.len() > CHUNK_BYTES {
            return Err(Error::Malformed);
        }
        out[..data.len()].copy_from_slice(data);
        Ok(data.len())
    }
    /// Session Terminal only: typed input for the console's programs (key
    /// events as win32-input-mode sequences, or text). True when queued;
    /// false when the console's input is full (nothing was queued).
    pub fn input(&mut self, bytes: &[u8]) -> Result<bool> {
        if bytes.is_empty() || bytes.len() > CHUNK_BYTES {
            return Err(Error::Service(Code::InvalidRequest));
        }
        match self
            .connection
            .call(opcode::CONSOLE_INPUT, bytes, &[])?
            .scalar()?
        {
            0 => Ok(false),
            n if n == bytes.len() as u64 => Ok(true),
            _ => Err(Error::Malformed),
        }
    }
    /// The console screen's state: the grid's size, the cursor, the modes.
    pub fn state(&mut self) -> Result<door_service_protocol::console::State> {
        let reply = self.connection.call(opcode::CONSOLE_STATE, &[], &[])?;
        door_service_protocol::console::State::decode(reply.data()?).ok_or(Error::Malformed)
    }
    /// Change the console's state (the Terminal's grid, the input mode,
    /// flushing input); the state after it.
    pub fn command(
        &mut self,
        command: door_service_protocol::console::Command,
    ) -> Result<door_service_protocol::console::State> {
        let reply = self
            .connection
            .call(opcode::CONSOLE_STATE, &command.encode(), &[])?;
        door_service_protocol::console::State::decode(reply.data()?).ok_or(Error::Malformed)
    }
    /// Typed input not yet read (at most one chunk) into `out`; WouldBlock
    /// when there is none.
    pub fn read_input(&mut self, out: &mut [u8; CHUNK_BYTES]) -> Result<usize> {
        let reply = self.connection.call(opcode::CONSOLE_READ_INPUT, &[], &[])?;
        let data = reply.data()?;
        if data.len() > CHUNK_BYTES {
            return Err(Error::Malformed);
        }
        out[..data.len()].copy_from_slice(data);
        Ok(data.len())
    }
    /// Poll cancellation without consuming another foreground application's text.
    pub fn interrupted(&mut self) -> Result<bool> {
        match self
            .connection
            .call(opcode::CONSOLE_INTERRUPT, &[], &[])?
            .scalar()?
        {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::Malformed),
        }
    }
}
pub struct Session {
    connection: Connection,
}
impl Session {
    pub fn connect(registry: Handle) -> Result<Self> {
        Ok(Self {
            connection: connect(registry, manifest::SESSION)?,
        })
    }
    /// Connect using a Port already authenticated by the service supervisor.
    pub fn connect_port(port: Handle) -> Result<Self> {
        Ok(Self {
            connection: Connection::connect(port, manifest::SESSION)?,
        })
    }
    pub fn logon(&mut self, permit: Handle) -> Result<Handle> {
        self.connection
            .call(
                opcode::LOGON,
                &[],
                &[Transfer {
                    handle: permit,
                    rights: rights::QUERY | rights::TRANSFER | rights::READ,
                    mode: 1,
                }],
            )?
            .one_handle()
    }
    pub fn logoff(&mut self, session: Handle) -> Result<()> {
        self.connection
            .call(
                opcode::LOGOFF,
                &[],
                &[Transfer {
                    handle: session,
                    rights: rights::QUERY | rights::TRANSFER | rights::WRITE,
                    mode: 0,
                }],
            )?
            .empty()
    }
}

#[cfg(test)]
mod setting_tests {
    use super::Setting;
    #[test]
    fn every_preference_word_has_a_storage_setting() {
        for p in door_service_protocol::preferences::ALL {
            assert_eq!(
                Setting::from_id(p.id).map(|s| s as u32),
                Some(p.id),
                "{}",
                p.name
            );
        }
        assert!(Setting::from_id(door_service_protocol::preferences::LAST + 1).is_none());
    }
}

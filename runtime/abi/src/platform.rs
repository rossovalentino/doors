//! Versioned native platform records. Descriptions never confer authority.
use crate::{STARTUP_BYTES, Startup, Status, get16, get32, get64, put16, put32, put64};

pub const IDENTITY_BYTES: usize = 64;
pub const STARTUP_V2_BYTES: usize = 1024;
pub const STARTUP_V2_MAGIC: [u8; 8] = *b"DOORUSR2";
pub const ARGUMENT_BYTES: usize = 128;
pub const ENVIRONMENT_BYTES: usize = 256;
pub const BLOCK_INFO_BYTES: usize = 32;
pub const LAUNCH_RESULT_BYTES: usize = 24;
pub const HEAP_BASE: u64 = 0x5000_0000;
pub const HEAP_BYTES: u64 = 1024 * 1024;
pub const NATIVE_HEAP_MAX_BYTES: u64 = 256 * HEAP_BYTES;
/// Qualified APIC period. TIME/deadlines use BSP ticks, not nanoseconds.
pub const MONOTONIC_TICK_MILLIS: u64 = 10;
pub const fn ticks_from_millis(millis: u64) -> Option<u64> {
    match millis.checked_add(MONOTONIC_TICK_MILLIS - 1) {
        Some(rounded) => Some(rounded / MONOTONIC_TICK_MILLIS),
        None => None,
    }
}

pub mod capability {
    pub const INIT: u64 = 1;
    pub const BLOCK: u64 = 2;
    pub const SESSION: u64 = 4;
    pub const CONSOLE: u64 = 8;
    pub const DISPLAY: u64 = 16;
    /// The network interface (NET_INFO/NET_SEND/NET_RECEIVE): door-network.
    pub const NETWORK: u64 = 32;
    /// The sound device (AUDIO_INFO/AUDIO_WRITE/AUDIO_CONTROL): door-audio.
    pub const AUDIO: u64 = 64;
    pub const ALL: u64 = 127;
}
pub mod manifest {
    pub const INIT: u64 = 1;
    pub const STORAGE: u64 = 2;
    pub const SESSION: u64 = 3;
    pub const CONSOLE: u64 = 4;
    pub const WORKLOAD: u64 = 5;
    pub const WINDOWS: u64 = 6;
    pub const WINDOWS_BASE: u64 = 7;
    pub const SHELL: u64 = 8;
    pub const APPLICATION: u64 = 9;
    pub const WINDOWS_APPLICATION: u64 = 10;
    pub const WINDOW: u64 = 11;
    pub const WORKSPACE: u64 = 12;
    pub const PREVIEW: u64 = 13;
    pub const MONITOR: u64 = 14;
    /// Ordinary Windows x64 GUI application (PE subsystem 2) admitted by
    /// `PROCESS_EXECUTE` under the P13 personality. Session user, no
    /// capabilities; several threads, a larger heap and the app-asset fonts.
    pub const WINDOWS_GUI: u64 = 15;
    /// The session input-method process (`door-ime`; one per signed-in
    /// session, started and retired by Init). Session user, no capabilities.
    pub const IME: u64 = 16;
    /// The Captions assistive client (`door-assist`). Session user, no
    /// kernel capabilities; the window server grants the door.accessibility
    /// assistive capability to this manifest identity only.
    pub const ASSIST: u64 = 17;
    /// The Door UI component gallery (`door-gallery`), an engineering
    /// fixture. Session user, no capabilities.
    pub const GALLERY: u64 = 18;
    /// The restartable desktop shell (`door-desktop-shell`, P14): taskbar
    /// groups, the application registration database, notifications and
    /// Start file search. Session user, no capabilities; the window server
    /// grants the door.shell role to this manifest when Init reports it.
    pub const DESKTOP_SHELL: u64 = 19;
    /// Calculator, the first inbox app (`door-inbox`, P14): the image runs
    /// the app its manifest names. Session user, no capabilities.
    pub const CALCULATOR: u64 = 20;
    /// Clock (`door-inbox`): focus sessions, timers, alarms, stopwatch,
    /// world clock. Session user, no capabilities.
    pub const CLOCK: u64 = 21;
    /// Paint (`door-inbox`): Windows 11 Paint with Paint 3D's 3D shapes,
    /// one app. Session user, no capabilities.
    pub const PAINT: u64 = 22;
    /// Snipping Tool (`door-inbox`): its captures come from the window
    /// server's snip overlay (Win+Shift+S). Session user, no capabilities.
    pub const SNIPPING_TOOL: u64 = 23;
    /// Weather (`door-inbox`). Session user, no capabilities (no network
    /// before P15: it says so).
    pub const WEATHER: u64 = 24;
    /// Magnifier (`door-inbox`): the window server magnifies the screen
    /// for it. Session user, no capabilities.
    pub const MAGNIFIER: u64 = 25;
    /// Narrator (`door-inbox`). Session user, no capabilities.
    pub const NARRATOR: u64 = 26;
    /// On-Screen Keyboard (`door-inbox`): its keys are never focused and
    /// type into the foreground window. Session user, no capabilities.
    pub const ON_SCREEN_KEYBOARD: u64 = 27;
    /// Voice access (`door-inbox`). Session user, no capabilities.
    pub const VOICE_ACCESS: u64 = 28;
    /// The network service (`door-network`, P15): the only process that
    /// moves frames through the NIC; system service (session 0).
    pub const NETWORK: u64 = 29;
    /// The sound service (`door-audio`, P15): the only process that plays
    /// through the sound device; system service (session 0).
    pub const AUDIO: u64 = 30;
    /// Inbox apps (`door-inbox`), in manifest order.
    pub const INBOX: [u64; 9] = [
        CALCULATOR,
        CLOCK,
        PAINT,
        SNIPPING_TOOL,
        WEATHER,
        MAGNIFIER,
        NARRATOR,
        ON_SCREEN_KEYBOARD,
        VOICE_ACCESS,
    ];
}
/// Trusted launch catalog shared by loader enforcement and service descriptions.
/// A UID of zero selects the authenticated Session grant's UID; it is never an
/// anonymous user. These are policy limits, not user-supplied wire authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManifestPolicy {
    pub uid: u32,
    pub caps: u64,
    pub max_threads: u32,
    pub max_heap_bytes: u64,
}
/// The console service's heap (`max_heap_bytes` of manifest 4).
pub const CONSOLE_HEAP_BYTES: u64 = 4 * HEAP_BYTES;
pub const MANIFEST_POLICIES: [ManifestPolicy; 14] = [
    ManifestPolicy {
        uid: 1,
        caps: capability::INIT,
        max_threads: 1,
        max_heap_bytes: 0,
    },
    ManifestPolicy {
        uid: 1,
        caps: capability::BLOCK,
        max_threads: 1,
        max_heap_bytes: 0,
    },
    // Session service: 8 MiB heap for the Argon2id working memory of the
    // P14 credential store (ADR 0020); still one thread, no other change.
    ManifestPolicy {
        uid: 2,
        caps: capability::SESSION,
        max_threads: 1,
        max_heap_bytes: 8 * HEAP_BYTES,
    },
    // Console service: a heap for the Terminal tabs' pseudoconsoles
    // (screens made on demand, about 10 KiB each).
    ManifestPolicy {
        uid: 3,
        caps: capability::CONSOLE,
        max_threads: 1,
        max_heap_bytes: CONSOLE_HEAP_BYTES,
    },
    ManifestPolicy {
        uid: 0,
        caps: 0,
        max_threads: 2,
        max_heap_bytes: HEAP_BYTES,
    },
    ManifestPolicy {
        uid: 0,
        caps: 0,
        max_threads: 1,
        max_heap_bytes: 0,
    },
    ManifestPolicy {
        uid: 0,
        caps: 0,
        // Windows console programs (manifest 7), P18 M0h.
        max_threads: 64,
        max_heap_bytes: HEAP_BYTES,
    },
    ManifestPolicy {
        uid: 0,
        caps: 0,
        max_threads: 8,
        max_heap_bytes: HEAP_BYTES,
    },
    ManifestPolicy {
        uid: 0,
        caps: 0,
        max_threads: 8,
        max_heap_bytes: HEAP_BYTES,
    },
    ManifestPolicy {
        uid: 0,
        caps: 0,
        max_threads: 8,
        max_heap_bytes: HEAP_BYTES,
    },
    ManifestPolicy {
        uid: 3,
        caps: capability::DISPLAY,
        max_threads: 1,
        max_heap_bytes: NATIVE_HEAP_MAX_BYTES,
    },
    ManifestPolicy {
        uid: 0,
        caps: 0,
        max_threads: 1,
        max_heap_bytes: 96 * HEAP_BYTES,
    },
    ManifestPolicy {
        uid: 0,
        caps: 0,
        max_threads: 1,
        max_heap_bytes: 128 * HEAP_BYTES,
    },
    ManifestPolicy {
        uid: 0,
        caps: 0,
        max_threads: 1,
        max_heap_bytes: 96 * HEAP_BYTES,
    },
];
/// Policy of a Windows application (GUI manifest 15, console 10): a UI
/// thread plus workers (message pumps, hung-window probes, file dialogs)
/// and 192 MiB of commit (of the 256 MiB dynamic window) for the heap,
/// VirtualAlloc, back buffers, DIB sections, fonts and text caches.
const WINDOWS_GUI_POLICY: ManifestPolicy = ManifestPolicy {
    uid: 0,
    caps: 0,
    // P18 M0h: managed runtimes and browsers start dozens of threads.
    max_threads: 64,
    max_heap_bytes: 192 * HEAP_BYTES,
};
/// Policy of the session IME (manifest 16): one thread, 48 MiB heap.
const IME_POLICY: ManifestPolicy = ManifestPolicy {
    uid: 0,
    caps: 0,
    max_threads: 1,
    max_heap_bytes: 48 * HEAP_BYTES,
};
/// Policy of the Captions assistive client (manifest 17): one thread, 48 MiB.
const ASSIST_POLICY: ManifestPolicy = ManifestPolicy {
    uid: 0,
    caps: 0,
    max_threads: 1,
    max_heap_bytes: 48 * HEAP_BYTES,
};
/// Policy of the component gallery (manifest 18): one thread, 96 MiB.
const GALLERY_POLICY: ManifestPolicy = ManifestPolicy {
    uid: 0,
    caps: 0,
    max_threads: 1,
    max_heap_bytes: 96 * HEAP_BYTES,
};
/// Policy of the desktop shell (manifest 19): one thread, 48 MiB (search
/// index, registration database and notification view).
const DESKTOP_SHELL_POLICY: ManifestPolicy = ManifestPolicy {
    uid: 0,
    caps: 0,
    max_threads: 1,
    max_heap_bytes: 48 * HEAP_BYTES,
};
/// Policy of the inbox apps (`door-inbox`): one thread, 96 MiB (Paint's
/// canvas and undo history are the largest).
const INBOX_POLICY: ManifestPolicy = ManifestPolicy {
    uid: 0,
    caps: 0,
    max_threads: 1,
    max_heap_bytes: 96 * HEAP_BYTES,
};
/// Policy of the network service (manifest 29): one thread, the NETWORK
/// capability, 16 MiB for socket buffers and the stack's tables.
const NETWORK_POLICY: ManifestPolicy = ManifestPolicy {
    uid: 4,
    caps: capability::NETWORK,
    max_threads: 1,
    max_heap_bytes: 16 * HEAP_BYTES,
};
/// Policy of the sound service (manifest 30): one thread, the AUDIO
/// capability, 8 MiB for the mixer (client PCM arrives in mapped sections).
const AUDIO_POLICY: ManifestPolicy = ManifestPolicy {
    uid: 4,
    caps: capability::AUDIO,
    max_threads: 1,
    max_heap_bytes: 8 * HEAP_BYTES,
};
/// Whether `id` is an inbox app's manifest.
pub const fn is_inbox(id: u64) -> bool {
    let mut i = 0;
    while i < manifest::INBOX.len() {
        if manifest::INBOX[i] == id {
            return true;
        }
        i += 1;
    }
    false
}
pub const fn manifest_policy(id: u64) -> Option<ManifestPolicy> {
    // A Windows console program may open windows and dialogs too
    // (MessageBox, SHBrowseForFolder, the common dialogs): the same budget.
    if id == manifest::WINDOWS_GUI || id == manifest::WINDOWS_APPLICATION {
        Some(WINDOWS_GUI_POLICY)
    } else if id == manifest::IME {
        Some(IME_POLICY)
    } else if id == manifest::ASSIST {
        Some(ASSIST_POLICY)
    } else if id == manifest::GALLERY {
        Some(GALLERY_POLICY)
    } else if id == manifest::DESKTOP_SHELL {
        Some(DESKTOP_SHELL_POLICY)
    } else if is_inbox(id) {
        Some(INBOX_POLICY)
    } else if id == manifest::NETWORK {
        Some(NETWORK_POLICY)
    } else if id == manifest::AUDIO {
        Some(AUDIO_POLICY)
    } else if id >= manifest::INIT && id <= manifest::MONITOR {
        Some(MANIFEST_POLICIES[id as usize - 1])
    } else {
        None
    }
}
/// Lifetime serial-diagnostic budget of a process bound to `manifest`. The
/// Captions assistive client reports the semantic trees it reads (the
/// accessibility evidence channel), so it gets 128 KiB; every other image
/// keeps `PROCESS_DIAGNOSTIC_BUDGET`.
pub const fn diagnostic_budget(manifest: u64) -> usize {
    if manifest == manifest::ASSIST {
        128 * 1024
    } else {
        crate::PROCESS_DIAGNOSTIC_BUDGET
    }
}
pub mod mode {
    pub const DESKTOP: u64 = 11;
    pub const WINDOW_TEST_HANG: u64 = 12;
    pub const SETTINGS: u64 = 13;
    pub const TERMINAL: u64 = 14;
    /// Windowless workspace process answering Start file search for the
    /// session (one per session, started and retired by Init).
    pub const SEARCH_HOST: u64 = 15;
    /// Captions (ASSIST manifest only) on the desktop-diagnostics image:
    /// it may write the trees and captions it reads to serial diagnostics.
    pub const ASSIST_DIAGNOSTICS: u64 = 16;
    /// Terminal with the full token (BUILTIN\Administrators enabled): Init
    /// launches it only after User Account Control approved an
    /// administrator console, as Windows Terminal opens an elevated window.
    pub const TERMINAL_ELEVATED: u64 = 17;
    pub const SAVE: u64 = 1;
    pub const REOPEN: u64 = 2;
    pub const RESTART: u64 = 3;
    pub const FAILURE: u64 = 4;
    pub const DEMO: u64 = 5;
    pub const WINDOWS: u64 = 6;
    pub const WINDOWS_BASE: u64 = 7;
    pub const SHELL: u64 = 8;
    pub const APPLICATION: u64 = 9;
    pub const WINDOWS_APPLICATION: u64 = 10;
    pub const WINDOWS_REOPEN: u64 = 8;
    pub const SESSION: u64 = 9;
}
pub mod inherited {
    pub const CONTROL: usize = 0;
    pub const REGISTRY: usize = 1;
    pub const AUTHORITY: usize = 2;
    pub const LOGON_FIRST: usize = 3;
    pub const LOGON_SECOND: usize = 4;
    pub const DIRECTORY: usize = 5;
    pub const DESKTOP_ASSETS: usize = 6;
    /// Init only: the service account's (`SERVICE_ACCOUNT`) logon permit
    /// (slot 6 is the window server's DESKTOP_ASSETS).
    pub const LOGON_SERVICE: usize = 6;
    /// Desktop applications (Workspace/Preview/Monitor manifests): a
    /// QUERY|READ|MAP handle to the one kernel-retained section holding the
    /// validated `system/app-assets.pack` (fonts and icons). Never WRITE.
    pub const APP_ASSETS: usize = 7;
}
/// On-demand read-only resources (ADR 0018): the full Windows fonts kept in
/// the authenticated initfs but too large for the application asset pack.
/// `operation::RESOURCE_OPEN` with an id below grants a QUERY|READ|MAP
/// section, created and pinned on its first request, to desktop processes
/// (Window, Workspace, Preview, Monitor and IME manifests) only.
pub mod resource {
    pub const EMOJI: u64 = 1;
    pub const HISTORIC: u64 = 2;
    pub const CHINESE_SIMPLIFIED: u64 = 3;
    pub const CHINESE_TRADITIONAL: u64 = 4;
    pub const JAPANESE: u64 = 5;
    pub const KOREAN: u64 = 6;
    pub const INDIC: u64 = 7;
    /// Input-method dictionaries (optional: absent until the owner supplies
    /// them; the IME then composes without candidates).
    pub const PINYIN_DICTIONARY: u64 = 8;
    pub const JAPANESE_DICTIONARY: u64 = 9;
    /// Initfs path of resource `id` (index `id - 1`).
    pub const PATHS: [&str; 9] = [
        "fonts/seguiemj.ttf",
        "fonts/seguihis.ttf",
        "fonts/msyh.ttc",
        "fonts/msjh.ttc",
        "fonts/YuGothM.ttc",
        "fonts/malgun.ttf",
        "fonts/Nirmala.ttc",
        "ime/pinyin.dict",
        "ime/japanese.dict",
    ];
    /// User address window for read-only resource views; each process
    /// places its views inside it (writable sections stay below 0x8000_0000).
    pub const VIEW_START: u64 = 0x8000_0000;
    pub const VIEW_END: u64 = 0x9000_0000;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Identity {
    pub pid: u64,
    pub uid: u32,
    pub integrity: u32,
    pub groups: u64,
    pub session: u64,
    pub epoch: u64,
    pub manifest: u64,
    pub instance: u64,
    pub capabilities: u64,
}
impl Identity {
    pub fn validate(&self) -> Result<(), Status> {
        if self.pid == 0
            || self.uid == 0
            || self.integrity > 2
            || (self.session == 0) != (self.epoch == 0)
            || self.manifest == 0
            || self.instance == 0
            || self.capabilities & !capability::ALL != 0
        {
            return Err(Status::InvalidArgument);
        }
        Ok(())
    }
    pub fn encode(&self, bytes: &mut [u8]) -> Result<(), Status> {
        if bytes.len() != IDENTITY_BYTES {
            return Err(Status::InvalidArgument);
        }
        self.validate()?;
        put64(bytes, 0, self.pid);
        put32(bytes, 8, self.uid);
        put32(bytes, 12, self.integrity);
        for (i, value) in [
            self.groups,
            self.session,
            self.epoch,
            self.manifest,
            self.instance,
            self.capabilities,
        ]
        .iter()
        .enumerate()
        {
            put64(bytes, 16 + i * 8, *value);
        }
        Ok(())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Status> {
        if bytes.len() != IDENTITY_BYTES {
            return Err(Status::InvalidArgument);
        }
        let value = Self {
            pid: get64(bytes, 0),
            uid: get32(bytes, 8),
            integrity: get32(bytes, 12),
            groups: get64(bytes, 16),
            session: get64(bytes, 24),
            epoch: get64(bytes, 32),
            manifest: get64(bytes, 40),
            instance: get64(bytes, 48),
            capabilities: get64(bytes, 56),
        };
        value.validate()?;
        Ok(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupV2 {
    pub base: Startup,
    pub identity: Identity,
    pub executable_sha256: [u8; 32],
    pub cwd: u64,
    pub mode: u64,
    pub arguments: [u8; ARGUMENT_BYTES],
    pub argument_bytes: u16,
    pub environment: [u8; ENVIRONMENT_BYTES],
    pub environment_bytes: u16,
    /// A Windows program's standard input, output and error when its
    /// parent passed pipes (STARTF_USESTDHANDLES): each a section holding
    /// the pipe's ring buffer; none, the console.
    pub standard: [crate::Inherited; STANDARD_HANDLES],
}
/// Standard handle slots: input, output, error.
pub const STANDARD_HANDLES: usize = 3;
fn string_list(bytes: &[u8], count: usize, environment: bool) -> Result<(), Status> {
    if count > bytes.len() || bytes[count..].iter().any(|b| *b != 0) {
        return Err(Status::InvalidArgument);
    }
    if count == 0 {
        return Ok(());
    }
    if bytes[count - 1] != 0 {
        return Err(Status::InvalidArgument);
    }
    for entry in bytes[..count - 1].split(|b| *b == 0) {
        let text = core::str::from_utf8(entry).map_err(|_| Status::InvalidArgument)?;
        if text.is_empty()
            || (environment && text.split_once('=').is_none_or(|(key, _)| key.is_empty()))
        {
            return Err(Status::InvalidArgument);
        }
    }
    Ok(())
}
impl StartupV2 {
    pub fn validate(&self) -> Result<(), Status> {
        self.identity.validate()?;
        if self.identity.pid != self.base.pid
            || self.cwd == 0
            || !(1..=mode::TERMINAL_ELEVATED).contains(&self.mode)
            || (self.mode == mode::ASSIST_DIAGNOSTICS && self.identity.manifest != manifest::ASSIST)
            // The elevated Terminal carries the full token, nothing else does.
            || (self.mode == mode::TERMINAL_ELEVATED
                && (self.identity.manifest != manifest::WORKSPACE
                    || self.identity.groups & group::ADMINISTRATORS == 0))
            || self.base.inherited[inherited::DIRECTORY].handle != self.cwd
            || self.base.inherited[inherited::DIRECTORY].kind != crate::HandleKind::Directory as u64
            || self.executable_sha256.iter().all(|b| *b == 0)
        {
            return Err(Status::InvalidArgument);
        }
        for (i, grant) in self.standard.iter().enumerate() {
            if grant.handle == 0 {
                if *grant != crate::Inherited::default() {
                    return Err(Status::InvalidArgument);
                }
            } else if grant.kind != crate::HandleKind::Section as u64
                || grant.rights & !crate::rights::ALL != 0
                || self.base.inherited.iter().any(|g| g.handle == grant.handle)
                || self.standard[..i].iter().any(|g| g.handle == grant.handle)
            {
                return Err(Status::InvalidArgument);
            }
        }
        string_list(&self.arguments, self.argument_bytes as usize, false)?;
        string_list(&self.environment, self.environment_bytes as usize, true)
    }
    pub fn encode(&self, bytes: &mut [u8]) -> Result<(), Status> {
        if bytes.len() != STARTUP_V2_BYTES {
            return Err(Status::InvalidArgument);
        }
        self.validate()?;
        bytes.fill(0);
        self.base.encode(&mut bytes[..STARTUP_BYTES])?;
        bytes[..8].copy_from_slice(&STARTUP_V2_MAGIC);
        put16(bytes, 10, 1);
        put32(bytes, 12, STARTUP_V2_BYTES as u32);
        self.identity.encode(&mut bytes[256..320])?;
        bytes[320..352].copy_from_slice(&self.executable_sha256);
        put64(bytes, 352, self.cwd);
        put64(bytes, 360, self.mode);
        put16(bytes, 368, self.argument_bytes);
        put16(bytes, 370, self.environment_bytes);
        bytes[384..512].copy_from_slice(&self.arguments);
        bytes[512..768].copy_from_slice(&self.environment);
        for (i, grant) in self.standard.iter().enumerate() {
            put64(bytes, 768 + i * 24, grant.handle);
            put64(bytes, 776 + i * 24, grant.rights);
            put64(bytes, 784 + i * 24, grant.kind);
        }
        Ok(())
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Status> {
        if bytes.len() != STARTUP_V2_BYTES
            || bytes[..8] != STARTUP_V2_MAGIC
            || get32(bytes, 12) != STARTUP_V2_BYTES as u32
            || bytes[372..384].iter().chain(&bytes[840..]).any(|b| *b != 0)
        {
            return Err(Status::InvalidArgument);
        }
        if get16(bytes, 8) != crate::MAJOR || get16(bytes, 10) != 1 {
            return Err(Status::VersionMismatch);
        }
        let mut base = [0; STARTUP_BYTES];
        base.copy_from_slice(&bytes[..STARTUP_BYTES]);
        base[..8].copy_from_slice(&crate::STARTUP_MAGIC);
        put16(&mut base, 10, crate::MINOR);
        put32(&mut base, 12, STARTUP_BYTES as u32);
        let value = Self {
            base: Startup::decode(&base)?,
            identity: Identity::decode(&bytes[256..320])?,
            executable_sha256: bytes[320..352].try_into().unwrap(),
            cwd: get64(bytes, 352),
            mode: get64(bytes, 360),
            arguments: bytes[384..512].try_into().unwrap(),
            argument_bytes: get16(bytes, 368),
            environment: bytes[512..768].try_into().unwrap(),
            environment_bytes: get16(bytes, 370),
            standard: core::array::from_fn(|i| crate::Inherited {
                handle: get64(bytes, 768 + i * 24),
                rights: get64(bytes, 776 + i * 24),
                kind: get64(bytes, 784 + i * 24),
            }),
        };
        value.validate()?;
        Ok(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LaunchResult {
    pub process: u64,
    pub control: u64,
    pub pid: u64,
}
impl LaunchResult {
    pub fn encode(self, b: &mut [u8]) -> Result<(), Status> {
        if b.len() != LAUNCH_RESULT_BYTES || self.process == 0 || self.control == 0 || self.pid == 0
        {
            return Err(Status::InvalidArgument);
        }
        put64(b, 0, self.process);
        put64(b, 8, self.control);
        put64(b, 16, self.pid);
        Ok(())
    }
    pub fn decode(b: &[u8]) -> Result<Self, Status> {
        if b.len() != LAUNCH_RESULT_BYTES {
            return Err(Status::InvalidArgument);
        }
        let value = Self {
            process: get64(b, 0),
            control: get64(b, 8),
            pid: get64(b, 16),
        };
        let mut check = [0; LAUNCH_RESULT_BYTES];
        value.encode(&mut check)?;
        Ok(value)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockInfo {
    pub sector_bytes: u64,
    pub sectors: u64,
    pub writable: bool,
    pub flush: bool,
}
impl BlockInfo {
    pub fn encode(self, b: &mut [u8]) -> Result<(), Status> {
        if b.len() != BLOCK_INFO_BYTES || self.sector_bytes != 512 || self.sectors == 0 {
            return Err(Status::InvalidArgument);
        }
        for (i, v) in [
            self.sector_bytes,
            self.sectors,
            self.writable as u64,
            self.flush as u64,
        ]
        .iter()
        .enumerate()
        {
            put64(b, i * 8, *v);
        }
        Ok(())
    }
    pub fn decode(b: &[u8]) -> Result<Self, Status> {
        if b.len() != BLOCK_INFO_BYTES || get64(b, 16) > 1 || get64(b, 24) > 1 {
            return Err(Status::InvalidArgument);
        }
        let value = Self {
            sector_bytes: get64(b, 0),
            sectors: get64(b, 8),
            writable: get64(b, 16) != 0,
            flush: get64(b, 24) != 0,
        };
        let mut check = [0; BLOCK_INFO_BYTES];
        value.encode(&mut check)?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_is_a_separate_unprivileged_session_bound_image() {
        let p = manifest_policy(manifest::PREVIEW).unwrap();
        assert_eq!(
            (p.uid, p.caps, p.max_threads, p.max_heap_bytes),
            (0, 0, 1, 128 * HEAP_BYTES)
        );
        assert_eq!(manifest::MONITOR + 1, manifest::WINDOWS_GUI);
        let gui = manifest_policy(manifest::WINDOWS_GUI).unwrap();
        assert_eq!(
            (gui.uid, gui.caps, gui.max_threads, gui.max_heap_bytes),
            (0, 0, 64, 192 * HEAP_BYTES)
        );
        assert!(manifest_policy(0).is_none());
    }
    #[test]
    fn only_the_assistive_client_has_a_larger_diagnostic_budget() {
        assert_eq!(diagnostic_budget(manifest::ASSIST), 128 * 1024);
        for id in [manifest::WINDOW, manifest::GALLERY, manifest::IME, 0, 99] {
            assert_eq!(diagnostic_budget(id), crate::PROCESS_DIAGNOSTIC_BUDGET);
        }
    }
    #[test]
    fn the_ime_is_a_session_bound_single_thread_image_with_its_own_dictionaries() {
        let p = manifest_policy(manifest::IME).unwrap();
        assert_eq!(
            (p.uid, p.caps, p.max_threads, p.max_heap_bytes),
            (0, 0, 1, 48 * HEAP_BYTES)
        );
        // The accessories follow the IME; nothing has a policy past them.
        let assist = manifest_policy(manifest::ASSIST).unwrap();
        assert_eq!(
            (
                assist.uid,
                assist.caps,
                assist.max_threads,
                assist.max_heap_bytes
            ),
            (0, 0, 1, 48 * HEAP_BYTES)
        );
        let gallery = manifest_policy(manifest::GALLERY).unwrap();
        assert_eq!(
            (
                gallery.uid,
                gallery.caps,
                gallery.max_threads,
                gallery.max_heap_bytes
            ),
            (0, 0, 1, 96 * HEAP_BYTES)
        );
        assert_eq!(
            (manifest::IME + 1, manifest::IME + 2),
            (manifest::ASSIST, manifest::GALLERY)
        );
        let shell = manifest_policy(manifest::DESKTOP_SHELL).unwrap();
        assert_eq!(
            (
                shell.uid,
                shell.caps,
                shell.max_threads,
                shell.max_heap_bytes
            ),
            (0, 0, 1, 48 * HEAP_BYTES)
        );
        assert_eq!(manifest::GALLERY + 1, manifest::DESKTOP_SHELL);
        // The inbox apps follow the desktop shell; nothing past them.
        assert_eq!(manifest::INBOX[0], manifest::DESKTOP_SHELL + 1);
        for id in manifest::INBOX {
            let p = manifest_policy(id).unwrap();
            assert_eq!(
                (p.uid, p.caps, p.max_threads, p.max_heap_bytes),
                (0, 0, 1, 96 * HEAP_BYTES)
            );
        }
        // The network service follows the inbox apps; nothing past it.
        let last = manifest::INBOX[manifest::INBOX.len() - 1];
        assert_eq!(manifest::NETWORK, last + 1);
        let net = manifest_policy(manifest::NETWORK).unwrap();
        assert_eq!((net.caps, net.max_threads), (capability::NETWORK, 1));
        assert_eq!(manifest::AUDIO, manifest::NETWORK + 1);
        let audio = manifest_policy(manifest::AUDIO).unwrap();
        assert_eq!((audio.caps, audio.max_threads), (capability::AUDIO, 1));
        assert!(manifest_policy(manifest::AUDIO + 1).is_none());
        let a = AudioInfo {
            rate: 48_000,
            channels: 2,
            running: true,
            queued: 3,
            capacity: 16,
            played: 99,
            failed: 1,
            capture: Capture::Running,
        };
        let mut b = [0; AUDIO_INFO_BYTES];
        a.encode(&mut b).unwrap();
        assert_eq!(AudioInfo::decode(&b), Ok(a));
        b[20] = 3;
        assert!(AudioInfo::decode(&b).is_err(), "unknown capture state");
        b[20] = 0;
        assert_eq!(
            AudioInfo::decode(&b).map(|i| i.capture),
            Ok(Capture::Absent)
        );
        b[6] = 17;
        assert!(
            AudioInfo::decode(&b).is_err(),
            "more queued than the device holds"
        );
        let info = NetInfo {
            mac: [2, 0, 0, 0, 0, 1],
            link: true,
        };
        let mut b = [0; NET_INFO_BYTES];
        info.encode(&mut b).unwrap();
        assert_eq!(NetInfo::decode(&b), Ok(info));
        b[6] = 2;
        assert!(NetInfo::decode(&b).is_err());
        assert_eq!(
            resource::PATHS[resource::PINYIN_DICTIONARY as usize - 1],
            "ime/pinyin.dict"
        );
        assert_eq!(
            resource::PATHS[resource::JAPANESE_DICTIONARY as usize - 1],
            "ime/japanese.dict"
        );
    }
    #[test]
    fn millisecond_deadlines_round_up_to_the_declared_native_timer_period() {
        for (millis, ticks) in [
            (0, 0),
            (1, 1),
            (9, 1),
            (10, 1),
            (11, 2),
            (1000, 100),
            (5000, 500),
        ] {
            assert_eq!(ticks_from_millis(millis), Some(ticks));
        }
        assert_eq!(ticks_from_millis(u64::MAX), None);
        assert_eq!(manifest_policy(manifest::WINDOWS).unwrap().max_threads, 1);
        assert_eq!(
            manifest_policy(manifest::WINDOWS).unwrap().max_heap_bytes,
            0
        );
        assert_eq!(
            manifest_policy(manifest::WINDOWS_BASE).unwrap().max_threads,
            64
        );
        assert_eq!(
            manifest_policy(manifest::WINDOWS_BASE)
                .unwrap()
                .max_heap_bytes,
            HEAP_BYTES
        );
    }
    #[test]
    fn startup_roundtrip_requires_identity_cwd_bounds_and_canonical_padding() {
        let identity = Identity {
            pid: 7,
            uid: 1001,
            integrity: 1,
            groups: 2,
            session: 2,
            epoch: 4,
            manifest: 5,
            instance: 1,
            capabilities: 0,
        };
        let mut base = Startup {
            pid: 7,
            peer_pid: 1,
            role: 105,
            entry: 0x400000,
            tls: 0x71002000,
            stack_top: 0x70011000,
            inherited: [crate::Inherited::default(); 8],
        };
        base.inherited[5] = crate::Inherited {
            handle: 17,
            rights: crate::rights::QUERY,
            kind: crate::HandleKind::Directory as u64,
        };
        let mut value = StartupV2 {
            base,
            identity,
            executable_sha256: [1; 32],
            cwd: 17,
            mode: 1,
            arguments: [0; 128],
            argument_bytes: 4,
            environment: [0; 256],
            environment_bytes: 11,
            standard: [crate::Inherited::default(); STANDARD_HANDLES],
        };
        value.arguments[..4].copy_from_slice(b"app\0");
        value.environment[..11].copy_from_slice(b"USER=alice\0");
        let mut b = [0; 1024];
        value.encode(&mut b).unwrap();
        assert_eq!(StartupV2::decode(&b), Ok(value));
        let mut settings = value;
        settings.mode = mode::SETTINGS;
        settings.identity.manifest = manifest::WORKSPACE;
        settings.encode(&mut b).unwrap();
        assert_eq!(StartupV2::decode(&b), Ok(settings));
        settings.mode = mode::TERMINAL;
        settings.encode(&mut b).unwrap();
        assert_eq!(StartupV2::decode(&b), Ok(settings));
        settings.mode = mode::SEARCH_HOST;
        settings.encode(&mut b).unwrap();
        assert_eq!(StartupV2::decode(&b), Ok(settings));
        // The Captions diagnostic mode belongs to the ASSIST manifest only.
        settings.mode = mode::ASSIST_DIAGNOSTICS;
        assert_eq!(settings.validate(), Err(Status::InvalidArgument));
        let mut assist = settings;
        assist.identity.manifest = manifest::ASSIST;
        assist.encode(&mut b).unwrap();
        assert_eq!(StartupV2::decode(&b), Ok(assist));
        assist.mode = mode::ASSIST_DIAGNOSTICS + 1;
        assert_eq!(assist.validate(), Err(Status::InvalidArgument));
        settings.mode = mode::SEARCH_HOST + 1;
        assert_eq!(settings.validate(), Err(Status::InvalidArgument));
        // The elevated Terminal needs the full token.
        settings.mode = mode::TERMINAL_ELEVATED;
        assert_eq!(settings.validate(), Err(Status::InvalidArgument));
        settings.identity.groups |= group::ADMINISTRATORS;
        settings.encode(&mut b).unwrap();
        assert_eq!(StartupV2::decode(&b), Ok(settings));
        settings.mode = mode::TERMINAL_ELEVATED + 1;
        assert_eq!(settings.validate(), Err(Status::InvalidArgument));
        value.encode(&mut b).unwrap();

        for end in 0..1024 {
            assert!(StartupV2::decode(&b[..end]).is_err());
        }
        for offset in [372, 383, 768, 1023] {
            let mut bad = b;
            bad[offset] = 1;
            assert!(StartupV2::decode(&bad).is_err());
        }
        let mut bad = b;
        put16(&mut bad, 368, 129);
        assert!(StartupV2::decode(&bad).is_err());
        let mut bad = b;
        put64(&mut bad, 352, 18);
        assert!(StartupV2::decode(&bad).is_err());
        let mut bad = b;
        put64(&mut bad, 256, 8);
        assert!(StartupV2::decode(&bad).is_err());
        let mut bad = b;
        bad[394] = 1;
        assert!(StartupV2::decode(&bad).is_err());
    }
    #[test]
    fn untrusted_wire_fields_never_create_a_valid_malformed_record() {
        let mut b = [0; 64];
        assert!(Identity::decode(&b).is_err());
        let value = Identity {
            pid: 1,
            uid: 1,
            integrity: 2,
            groups: 0,
            session: 0,
            epoch: 0,
            manifest: 1,
            instance: 1,
            capabilities: 1,
        };
        value.encode(&mut b).unwrap();
        assert_eq!(Identity::decode(&b), Ok(value));
        put64(&mut b, 32, 1);
        assert!(Identity::decode(&b).is_err());
        let mut block = [0; 32];
        BlockInfo {
            sector_bytes: 512,
            sectors: 262144,
            writable: true,
            flush: true,
        }
        .encode(&mut block)
        .unwrap();
        put64(&mut block, 24, 2);
        assert!(BlockInfo::decode(&block).is_err());
        assert!(LaunchResult::decode(&[0; 24]).is_err());
    }
}

pub const EXECUTE_DESCRIPTOR_BYTES: usize = 448;
pub const EXECUTABLE_MAX_BYTES: usize = 1024 * 1024;
/// Group bits of an authority (`Identity::groups`): the Windows token's
/// groups as Doors records them.
pub mod group {
    /// BUILTIN\Users: every session application.
    pub const USERS: u64 = 1 << 1;
    /// Boot services.
    pub const SERVICES: u64 = 1 << 2;
    /// BUILTIN\Administrators, enabled: an elevated application (User
    /// Account Control approved it). Unelevated applications of an
    /// administrator have it for deny only, so it is absent here.
    pub const ADMINISTRATORS: u64 = 1 << 3;
    /// BUILTIN\Administrators for deny only: the session's account is an
    /// administrator (its unelevated token is the filtered one, with a
    /// linked full token). A standard user's token lacks the group.
    pub const ADMINISTRATOR_ACCOUNT: u64 = 1 << 4;
    /// The bits a session application's authority may carry.
    pub const SESSION: u64 = USERS | ADMINISTRATORS | ADMINISTRATOR_ACCOUNT;
}
/// SESSION_CREATE's `administrators`: bit `i` set when local account
/// `ACCOUNTS[i]` is an administrator (the session service's account book).
pub const ACCOUNTS: [u32; 2] = [1001, 1002];
/// LocalSystem (S-1-5-18): the account Windows services run as. Init
/// receives its logon permit at boot and keeps its session for the boot;
/// it is an administrator, its applications (services) get the full
/// token, and it has no interactive sign-in.
pub const SERVICE_ACCOUNT: u32 = 18;
/// Captured executable description; only kernel grants confer authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecuteDescriptor {
    pub image_pointer: u64,
    pub image_bytes: u64,
    /// 1 ELF, 2 PE32+.
    pub format: u64,
    pub cwd: u64,
    pub argument_bytes: u16,
    pub environment_bytes: u16,
    /// Run with the full (elevated) token: Init sets it only for a launch
    /// User Account Control approved; the kernel grants it only to holders
    /// of the Launch authority.
    pub elevated: bool,
    /// CREATE_SUSPENDED: the first thread waits for PROCESS_RESUME.
    pub suspended: bool,
    /// The launcher's pipe sections for the child's standard input,
    /// output and error (0: the console); copied with their rights.
    pub standard: [u64; STANDARD_HANDLES],
    pub arguments: [u8; ARGUMENT_BYTES],
    pub environment: [u8; ENVIRONMENT_BYTES],
}
impl ExecuteDescriptor {
    pub fn encode(&self, out: &mut [u8]) -> Result<(), Status> {
        if out.len() != EXECUTE_DESCRIPTOR_BYTES {
            return Err(Status::InvalidArgument);
        }
        self.validate()?;
        out.fill(0);
        put64(out, 0, self.image_pointer);
        put64(out, 8, self.image_bytes);
        put64(out, 16, self.format);
        put64(out, 24, self.cwd);
        put16(out, 32, self.argument_bytes);
        put16(out, 34, self.environment_bytes);
        out[36] = u8::from(self.elevated);
        out[37] = u8::from(self.suspended);
        for (i, handle) in self.standard.iter().enumerate() {
            put64(out, 40 + i * 8, *handle);
        }
        out[64..192].copy_from_slice(&self.arguments);
        out[192..448].copy_from_slice(&self.environment);
        Ok(())
    }
    pub fn decode(input: &[u8]) -> Result<Self, Status> {
        if input.len() != EXECUTE_DESCRIPTOR_BYTES
            || input[36] > 1
            || input[37] > 1
            || input[38..40].iter().any(|b| *b != 0)
        {
            return Err(Status::InvalidArgument);
        }
        let mut value = Self {
            image_pointer: get64(input, 0),
            image_bytes: get64(input, 8),
            format: get64(input, 16),
            cwd: get64(input, 24),
            argument_bytes: get16(input, 32),
            environment_bytes: get16(input, 34),
            elevated: input[36] == 1,
            suspended: input[37] == 1,
            standard: core::array::from_fn(|i| get64(input, 40 + i * 8)),
            arguments: [0; ARGUMENT_BYTES],
            environment: [0; ENVIRONMENT_BYTES],
        };
        value.arguments.copy_from_slice(&input[64..192]);
        value.environment.copy_from_slice(&input[192..448]);
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), Status> {
        if self.image_pointer == 0
            || self.image_bytes == 0
            || self.image_bytes > EXECUTABLE_MAX_BYTES as u64
            || !matches!(self.format, 1 | 2)
            // cwd 0: no working-folder grant (Init starting a service in
            // the service account's session); the kernel gives the child a
            // folder of its own.
            || self.argument_bytes == 0
            || self
                .standard
                .iter()
                .enumerate()
                .any(|(i, h)| *h != 0 && (*h == self.cwd || self.standard[..i].contains(h)))
        {
            return Err(Status::InvalidArgument);
        }
        string_list(&self.arguments, self.argument_bytes as usize, false)?;
        string_list(&self.environment, self.environment_bytes as usize, true)
    }
}

#[cfg(test)]
mod execute_tests {
    use super::*;
    fn description() -> ExecuteDescriptor {
        let mut d = ExecuteDescriptor {
            image_pointer: 0x10000,
            image_bytes: 4096,
            format: 1,
            cwd: 1,
            argument_bytes: 4,
            environment_bytes: 6,
            elevated: false,
            suspended: false,
            standard: [0; STANDARD_HANDLES],
            arguments: [0; ARGUMENT_BYTES],
            environment: [0; ENVIRONMENT_BYTES],
        };
        d.arguments[..4].copy_from_slice(b"app\0");
        d.environment[..6].copy_from_slice(b"PWD=/\0");
        d
    }
    #[test]
    fn execute_capture_rejects_oversized_images_and_malformed_lists() {
        let d = description();
        let mut bytes = [0; EXECUTE_DESCRIPTOR_BYTES];
        d.encode(&mut bytes).unwrap();
        assert_eq!(ExecuteDescriptor::decode(&bytes), Ok(d));
        // A standard pipe section may not repeat the working folder.
        let mut piped = d;
        piped.standard = [0, 9, 0];
        piped.encode(&mut bytes).unwrap();
        assert_eq!(ExecuteDescriptor::decode(&bytes), Ok(piped));
        piped.standard = [0, 1, 0];
        assert_eq!(piped.validate(), Err(Status::InvalidArgument));
        d.encode(&mut bytes).unwrap();
        bytes[39] = 1;
        assert_eq!(
            ExecuteDescriptor::decode(&bytes),
            Err(Status::InvalidArgument)
        );
        for format in [0, 3, u64::MAX] {
            let mut bad = d;
            bad.format = format;
            assert_eq!(bad.validate(), Err(Status::InvalidArgument));
        }
        let mut bad = d;
        bad.image_bytes = EXECUTABLE_MAX_BYTES as u64 + 1;
        assert_eq!(bad.validate(), Err(Status::InvalidArgument));
        let mut bad = d;
        bad.arguments[3] = b'x';
        assert_eq!(bad.validate(), Err(Status::InvalidArgument));
        let mut bad = d;
        bad.environment[0] = b'=';
        assert_eq!(bad.validate(), Err(Status::InvalidArgument));
        // No working-folder grant: the kernel gives the child its own.
        let mut own = d;
        own.cwd = 0;
        own.standard = [0, 9, 0];
        own.encode(&mut bytes).unwrap();
        assert_eq!(ExecuteDescriptor::decode(&bytes), Ok(own));
    }
    #[test]
    fn ordinary_image_policies_never_mint_service_capabilities() {
        for manifest in [
            manifest::SHELL,
            manifest::APPLICATION,
            manifest::WINDOWS_APPLICATION,
            manifest::WINDOWS_GUI,
        ] {
            let p = manifest_policy(manifest).unwrap();
            assert_eq!((p.uid, p.caps), (0, 0));
            assert!(p.max_threads > 1);
            assert!(p.max_heap_bytes > 0);
        }
    }
}
/// `NET_INFO` reply: the NIC's MAC address and link state.
pub const NET_INFO_BYTES: usize = 8;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetInfo {
    pub mac: [u8; 6],
    pub link: bool,
}
impl NetInfo {
    pub fn encode(self, b: &mut [u8]) -> Result<(), Status> {
        if b.len() != NET_INFO_BYTES {
            return Err(Status::InvalidArgument);
        }
        b[..6].copy_from_slice(&self.mac);
        b[6] = u8::from(self.link);
        b[7] = 0;
        Ok(())
    }
    pub fn decode(b: &[u8]) -> Result<Self, Status> {
        if b.len() != NET_INFO_BYTES || b[6] > 1 || b[7] != 0 {
            return Err(Status::InvalidArgument);
        }
        let mut mac = [0; 6];
        mac.copy_from_slice(&b[..6]);
        Ok(Self {
            mac,
            link: b[6] == 1,
        })
    }
}
/// Largest frame `NET_SEND`/`NET_RECEIVE` move (Ethernet II without FCS).
pub const NET_FRAME_BYTES: usize = 1514;
/// One `AUDIO_WRITE` period: 10 ms of 48 kHz stereo signed 16-bit PCM.
pub const AUDIO_PERIOD_BYTES: usize = 1920;
/// `AUDIO_CONTROL` commands: the output stream, then capture.
pub const AUDIO_START: u64 = 1;
pub const AUDIO_STOP: u64 = 2;
pub const AUDIO_CAPTURE_START: u64 = 3;
pub const AUDIO_CAPTURE_STOP: u64 = 4;
/// The device's capture input (`AudioInfo::capture`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Capture {
    /// No input stream.
    Absent = 0,
    Stopped = 1,
    Running = 2,
}
/// `AUDIO_INFO` reply: the output stream's format and queue.
pub const AUDIO_INFO_BYTES: usize = 24;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioInfo {
    pub rate: u32,
    pub channels: u8,
    /// The output stream is started.
    pub running: bool,
    /// Periods queued and not yet played, and the most the device holds.
    pub queued: u8,
    pub capacity: u8,
    /// Periods played since boot.
    pub played: u64,
    /// Periods the device returned with an error.
    pub failed: u32,
    pub capture: Capture,
}
impl AudioInfo {
    pub fn encode(self, b: &mut [u8]) -> Result<(), Status> {
        if b.len() != AUDIO_INFO_BYTES {
            return Err(Status::InvalidArgument);
        }
        b.fill(0);
        b[..4].copy_from_slice(&self.rate.to_le_bytes());
        b[4] = self.channels;
        b[5] = u8::from(self.running);
        b[6] = self.queued;
        b[7] = self.capacity;
        b[8..16].copy_from_slice(&self.played.to_le_bytes());
        b[16..20].copy_from_slice(&self.failed.to_le_bytes());
        b[20] = self.capture as u8;
        Ok(())
    }
    pub fn decode(b: &[u8]) -> Result<Self, Status> {
        if b.len() != AUDIO_INFO_BYTES
            || b[5] > 1
            || b[6] > b[7]
            || b[20] > 2
            || b[21..].iter().any(|v| *v != 0)
        {
            return Err(Status::InvalidArgument);
        }
        let capture = match b[20] {
            0 => Capture::Absent,
            1 => Capture::Stopped,
            _ => Capture::Running,
        };
        Ok(Self {
            rate: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            channels: b[4],
            running: b[5] == 1,
            queued: b[6],
            capacity: b[7],
            played: u64::from_le_bytes(b[8..16].try_into().unwrap()),
            failed: u32::from_le_bytes([b[16], b[17], b[18], b[19]]),
            capture,
        })
    }
}

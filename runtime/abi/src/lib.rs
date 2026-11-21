#![no_std]
//! Authoritative Door Native ABI 1 schema. Wire data is explicitly little endian.

pub mod clock;
pub mod entropy;
pub mod graphics;
pub mod hardware;
pub mod platform;
mod startup;
pub mod telemetry;
mod wire;
pub use startup::{Inherited, STARTUP_BYTES, STARTUP_MAGIC, Startup, role};
pub use wire::{AbiInfo, HandleInfo, Header, Metadata, Transfer};

pub const MAJOR: u16 = 1;
pub const MINOR: u16 = 0;
pub const TAG: u64 = 0xd001_0000_0000_0000;
pub const MAX_PAYLOAD: usize = 256;
pub const MAX_TRANSFERS: usize = 4;
pub const QUEUE_DEPTH: usize = 4;
pub const CANCEL_HISTORY_DEPTH: usize = 16;
pub const HEADER_BYTES: usize = 32;
pub const TRANSFER_BYTES: usize = 32;
pub const METADATA_BYTES: usize = 32;
pub const QUERY_BYTES: usize = 64;
pub const MAX_DIAGNOSTIC: usize = 1024;
pub const PROCESS_DIAGNOSTIC_BUDGET: usize = 16 * 1024;
pub const MAX_PACKET: usize = HEADER_BYTES + MAX_PAYLOAD;
pub const USER_LOW: u64 = 0x10000;
pub const USER_END: u64 = 0x0000_8000_0000_0000;

pub mod operation {
    pub const QUERY: u32 = 0;
    pub const EXIT: u32 = 1;
    pub const WRITE: u32 = 2;
    pub const HARDWARE_INFO: u32 = 3;
    pub const CLOSE: u32 = 0x100;
    pub const DUPLICATE: u32 = 0x101;
    pub const INSPECT: u32 = 0x102;
    pub const IDENTITY: u32 = 0x103;
    pub const SECTION_CREATE: u32 = 0x200;
    pub const SECTION_MAP: u32 = 0x201;
    pub const SECTION_UNMAP: u32 = 0x202;
    pub const SECTION_SNAPSHOT: u32 = 0x203;
    /// The inverse of SECTION_SNAPSHOT: a[0] a section handle with WRITE,
    /// a[1] the section offset, a[2]/a[3] the caller's source bytes (at most
    /// `graphics::SECTION_COPY_BYTES`). Both section kinds (P18 bulk file data).
    pub const SECTION_FILL: u32 = 0x204;
    pub const VM_ALLOCATE: u32 = 0x210;
    pub const VM_RELEASE: u32 = 0x211;
    pub const VM_PROTECT: u32 = 0x212;
    pub const USER_COPY: u32 = 0x213;
    pub const PROCESS_QUIESCE: u32 = 0x214;
    pub const VM_RESERVE: u32 = 0x215;
    pub const VM_COMMIT: u32 = 0x216;
    pub const VM_DECOMMIT: u32 = 0x217;
    pub const VM_WINDOWS_PROTECT: u32 = 0x218;
    pub const VM_WINDOWS_RELEASE: u32 = 0x219;
    pub const CONTEXT_RESUME: u32 = 0x21a;
    pub const WINDOWS_MODULE_LOAD: u32 = 0x21b;
    pub const WINDOWS_MODULE_UNLOAD: u32 = 0x21c;
    pub const WINDOWS_MODULE_SYMBOL: u32 = 0x21d;
    pub const WINDOWS_TLS_CALLBACK: u32 = 0x21e;
    /// LoadLibrary of a system DLL outside an application's startup closure.
    pub const WINDOWS_PROVIDER_LOAD: u32 = 0x21f;
    /// LoadLibrary of a DLL that is not a system DLL (an application's or
    /// an installer's own): a[0]/a[1] its image bytes, a[2]/a[3] the
    /// ModuleInit output. The kernel captures the bytes, plans the DLL with
    /// the process's modules at a private slot and maps it.
    pub const WINDOWS_PRIVATE_LOAD: u32 = 0x220;
    /// VirtualQuery (P18 M0b): a[0] the address, a[1] a 48-byte
    /// MEMORY_BASIC_INFORMATION (`door_windows_abi::memory::BasicInformation`).
    pub const VM_QUERY: u32 = 0x221;
    /// FlushProcessWriteBuffers: every CPU running a thread of the caller
    /// passes a serializing barrier before this returns.
    pub const VM_FLUSH_WRITE_BUFFERS: u32 = 0x222;
    /// CreateFileMapping (P18): a[0] bytes (page multiple), a[1] flags
    /// (`door_windows_abi::memory::section::RESERVE`), a[2] the PAGE_*
    /// protection, a[3] the handle rights (READ|MAP at least). A section handle.
    pub const VM_SECTION_CREATE: u32 = 0x223;
    /// MapViewOfFile: a[0] the section handle, a[1] the address (0: any),
    /// a[2] the byte offset (64 KiB multiple), a[3] the bytes (0: to the
    /// end), a[4] the PAGE_* protection, a[5] flags (RESERVE_TOP_DOWN,
    /// section::REPLACE). The view's base.
    pub const VM_SECTION_MAP: u32 = 0x224;
    /// UnmapViewOfFile: a[0] an address inside the view, a[1] flags
    /// (section::PRESERVE).
    pub const VM_SECTION_UNMAP: u32 = 0x225;
    /// The size in bytes of the section a[0] names (QUERY or MAP).
    pub const VM_SECTION_SIZE: u32 = 0x226;
    pub const PROCESS_WAIT: u32 = 0x300;
    pub const YIELD: u32 = 0x301;
    pub const PROCESS_LAUNCH: u32 = 0x302;
    pub const PROCESS_TERMINATE: u32 = 0x303;
    pub const PROCESS_EXECUTE: u32 = 0x304;
    pub const BOOT_FILE_READ: u32 = 0x305;
    /// Open an on-demand read-only resource (`platform::resource`): a
    /// QUERY|READ|MAP section handle, created on first request (ADR 0018).
    pub const RESOURCE_OPEN: u32 = 0x306;
    /// Start a process launched suspended (CREATE_SUSPENDED): a[0] a process
    /// handle with WAIT. Returns its previous suspend count (1, or 0 when it
    /// was not held).
    pub const PROCESS_RESUME: u32 = 0x307;
    /// The machine of a process (P17, IsWow64Process): a[0] a process
    /// handle with WAIT (the caller's own is accepted too). 0x14C for a
    /// 32-bit Windows program, 0x8664 for every 64-bit process.
    pub const PROCESS_MACHINE: u32 = 0x308;
    pub const THREAD_CREATE: u32 = 0x310;
    pub const THREAD_EXIT: u32 = 0x311;
    pub const THREAD_JOIN: u32 = 0x312;
    /// SuspendThread / ResumeThread of a thread of the caller's own process
    /// (a[0] its handle, WAIT): the previous suspend count.
    pub const THREAD_SUSPEND: u32 = 0x313;
    pub const THREAD_RESUME: u32 = 0x314;
    /// GetThreadContext / SetThreadContext of a suspended thread of the
    /// caller's own 64-bit process (P18): a[0] the thread handle, a[1] a
    /// 1,232-byte CONTEXT (integer, control and floating-point state).
    /// WouldBlock while the thread is not yet stopped.
    pub const THREAD_GET_CONTEXT: u32 = 0x315;
    pub const THREAD_SET_CONTEXT: u32 = 0x316;
    /// No arguments: a new handle (QUERY, DUPLICATE, TRANSFER, WAIT) to the
    /// calling thread, as DuplicateHandle of GetCurrentThread gives one
    /// (P18).
    pub const THREAD_OPEN_SELF: u32 = 0x317;
    /// No arguments: a new handle (QUERY, WAIT, DUPLICATE, TRANSFER) to the
    /// calling process, as OpenProcess of its own ID and DuplicateHandle of
    /// GetCurrentProcess give one (P18).
    pub const PROCESS_OPEN_SELF: u32 = 0x318;
    pub const TIME: u32 = 0x400;
    pub const SLEEP: u32 = 0x401;
    pub const ENTROPY_STATUS: u32 = 0x402;
    pub const SYSTEM_SNAPSHOT: u32 = 0x403;
    /// No arguments. Returns UTC wall time as FILETIME (100 ns since
    /// 1601-01-01): the validated RTC reading plus the monotonic tick offset
    /// since it was taken (`clock` module). Unsupported = no valid RTC on
    /// this profile (unavailable, never a guess); WouldBlock = the RTC stayed
    /// mid-update, retry later.
    pub const WALL_TIME: u32 = 0x404;
    /// No arguments. Time since boot in 100 ns units (QueryPerformanceCounter
    /// at 10 MHz): the invariant TSC where there is one, else the tick.
    pub const PRECISE_TIME: u32 = 0x407;
    /// a[0]: 32 bytes out: physical bytes managed, physical bytes free, the
    /// caller's private bytes (regions and committed Windows pages), and the
    /// machine's commit limit for Windows memory (GlobalMemoryStatusEx,
    /// GetProcessMemoryInfo).
    pub const MEMORY_STATUS: u32 = 0x406;
    /// Conditioned kernel entropy (`entropy` module): user buffer pointer,
    /// length 1..=`entropy::READ_MAX`; result = bytes written. Only callers
    /// whose kernel capabilities include `platform::capability::SESSION` or
    /// `INIT` (AccessDenied otherwise); Unsupported until the pool is READY.
    pub const ENTROPY_READ: u32 = 0x405;
    pub const CHANNEL_CREATE: u32 = 0x500;
    pub const CHANNEL_SEND: u32 = 0x501;
    pub const CHANNEL_RECEIVE: u32 = 0x502;
    pub const CHANNEL_PEER: u32 = 0x503;
    pub const CHANNEL_CANCEL: u32 = 0x504;
    pub const CHANNEL_SEND_DEADLINE: u32 = 0x505;
    pub const PORT_CREATE: u32 = 0x510;
    pub const PORT_CONNECT: u32 = 0x511;
    pub const PORT_ACCEPT: u32 = 0x512;
    pub const WAIT_CREATE: u32 = 0x600;
    pub const WAIT: u32 = 0x601;
    pub const SIGNAL: u32 = 0x602;
    pub const RESET: u32 = 0x603;
    pub const CONDITION_WAIT: u32 = 0x604;
    /// WaitOnAddress (P18): a[0] the address (naturally aligned), a[1] the
    /// compared value, a[2] its size (1, 2, 4 or 8), a[3] a deadline in
    /// ticks (u64::MAX: none). Returns at once when the word differs;
    /// spurious returns are allowed (the caller re-checks).
    pub const WAIT_ADDRESS: u32 = 0x605;
    /// WakeByAddressSingle/All: a[0] the address, a[1] how many (0: all).
    pub const WAKE_ADDRESS: u32 = 0x606;
    pub const SESSION_CREATE: u32 = 0x610;
    pub const SESSION_REVOKE: u32 = 0x611;
    pub const SESSION_QUERY: u32 = 0x612;
    /// After a sign-out, Init's fresh logon permit for local account a[1]
    /// (a[0]: its Launch authority), while that account has no live session.
    pub const PERMIT_RENEW: u32 = 0x613;
    pub const PROBE: u32 = 0x700;
    pub const BLOCK_INFO: u32 = 0x800;
    pub const BLOCK_READ: u32 = 0x801;
    pub const BLOCK_WRITE: u32 = 0x802;
    pub const BLOCK_FLUSH: u32 = 0x803;
    pub const CONSOLE_WRITE: u32 = 0x810;
    pub const CONSOLE_READ: u32 = 0x811;
    pub const CONSOLE_CONTROL: u32 = 0x812;
    pub const DISPLAY_INFO: u32 = 0x820;
    pub const DISPLAY_WRITE: u32 = 0x821;
    pub const DISPLAY_INPUT: u32 = 0x822;
    pub const DISPLAY_RELEASE: u32 = 0x823;
    pub const DISPLAY_CLOCK: u32 = 0x824;
    /// The NIC (door-network only): MAC, link and presence; one frame out;
    /// one frame in (its length, 0 when none waits).
    pub const NET_INFO: u32 = 0x830;
    pub const NET_SEND: u32 = 0x831;
    pub const NET_RECEIVE: u32 = 0x832;
    /// The sound device (door-audio only): presence, format and queue;
    /// one 10 ms period out; start/stop the output stream or capture; one
    /// captured 10 ms period in (1 when one was read, 0 when none waits).
    pub const AUDIO_INFO: u32 = 0x840;
    pub const AUDIO_WRITE: u32 = 0x841;
    pub const AUDIO_CONTROL: u32 = 0x842;
    pub const AUDIO_READ: u32 = 0x843;
}

/// Successful cancellation observation. A dispatched operation is never rolled back.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u64)]
pub enum CancelOutcome {
    Canceled = 0,
    Dispatched = 1,
    AlreadyCanceled = 2,
    Expired = 3,
}
impl CancelOutcome {
    pub fn from_raw(value: u64) -> Option<Self> {
        match value {
            0 => Some(Self::Canceled),
            1 => Some(Self::Dispatched),
            2 => Some(Self::AlreadyCanceled),
            3 => Some(Self::Expired),
            _ => None,
        }
    }
}

/// Zero is never a valid grant. The integer's bit allocation is private to the kernel.
pub type Handle = u64;
pub mod rights {
    pub const QUERY: u64 = 1;
    pub const DUPLICATE: u64 = 2;
    pub const TRANSFER: u64 = 4;
    pub const READ: u64 = 8;
    pub const WRITE: u64 = 16;
    pub const WAIT: u64 = 32;
    pub const MAP: u64 = 64;
    pub const ALL: u64 = 127;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u64)]
pub enum HandleKind {
    Channel = 1,
    Section = 2,
    Process = 3,
    Port = 4,
    Thread = 5,
    Event = 6,
    Mutex = 7,
    Condition = 8,
    LogonPermit = 9,
    Session = 10,
    Directory = 11,
    Block = 12,
    Console = 13,
    Launch = 14,
    Display = 15,
    /// door-network's role authority over the NIC (P15).
    Network = 16,
    /// door-audio's role authority over the sound device (P15).
    Audio = 17,
    /// A counting semaphore (P18, CreateSemaphore).
    Semaphore = 18,
}
impl HandleKind {
    pub fn from_raw(value: u64) -> Option<Self> {
        match value {
            1 => Some(Self::Channel),
            2 => Some(Self::Section),
            3 => Some(Self::Process),
            4 => Some(Self::Port),
            5 => Some(Self::Thread),
            6 => Some(Self::Event),
            7 => Some(Self::Mutex),
            8 => Some(Self::Condition),
            9 => Some(Self::LogonPermit),
            10 => Some(Self::Session),
            11 => Some(Self::Directory),
            12 => Some(Self::Block),
            13 => Some(Self::Console),
            14 => Some(Self::Launch),
            15 => Some(Self::Display),
            16 => Some(Self::Network),
            17 => Some(Self::Audio),
            18 => Some(Self::Semaphore),
            _ => None,
        }
    }
}

macro_rules! statuses {
    ($($name:ident = $value:literal),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[repr(u64)]
        pub enum Status { $($name = $value),+ }
        impl Status {
            pub fn from_raw(value: u64) -> Option<Self> {
                match value { $($value => Some(Self::$name),)+ _ => None }
            }
        }
    };
}
statuses! {
    Success = 0, InvalidOperation = 1, VersionMismatch = 2, InvalidArgument = 3,
    BadAddress = 4, BadHandle = 5, AccessDenied = 6, NotFound = 7, AlreadyExists = 8,
    Unsupported = 9, ResourceExhausted = 10, WouldBlock = 11, Timeout = 12,
    Canceled = 13, PeerClosed = 14, DeviceLost = 15, IoFailure = 16,
}

/// Namespace/version checking happens before any operation-specific pointer access.
pub fn decode_selector(selector: u64) -> Result<u32, Status> {
    if selector >> 48 != TAG >> 48 {
        return Err(Status::InvalidOperation);
    }
    if selector & 0x0000_ffff_0000_0000 != 0 {
        return Err(Status::VersionMismatch);
    }
    Ok(selector as u32)
}
pub const fn selector(operation: u32) -> u64 {
    TAG | operation as u64
}

/// Diagnostic checksum has a separately written literal assembly implementation.
pub const fn probe_checksum(args: [u64; 6]) -> u64 {
    args[0].rotate_left(1)
        ^ args[1].rotate_left(7)
        ^ args[2].rotate_left(13)
        ^ args[3].rotate_left(23)
        ^ args[4].rotate_left(37)
        ^ args[5].rotate_left(53)
        ^ 0xa5c3_6f19_d247_8be0
}

pub(crate) fn get16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}
pub(crate) fn get32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
pub(crate) fn get64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}
pub(crate) fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
pub(crate) fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
pub(crate) fn put64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Every `operation` constant names its own number: a duplicate sends
    /// one call to the other's handler (P18 found PRECISE_TIME on
    /// ENTROPY_READ's 0x405).
    #[test]
    fn operation_numbers_are_unique() {
        extern crate std;
        let source = include_str!("lib.rs");
        let start = source.find("pub mod operation {").unwrap();
        let end = start + source[start..].find("\n}\n").unwrap();
        let mut seen = std::collections::BTreeMap::new();
        for line in source[start..end].lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("pub const ") else {
                continue;
            };
            let (name, value) = rest.split_once(": u32 = ").unwrap();
            let value = value.trim_end_matches(';').replace('_', "");
            let number = match value.strip_prefix("0x") {
                Some(hex) => u32::from_str_radix(hex, 16).unwrap(),
                None => value.parse::<u32>().unwrap(),
            };
            if let Some(other) = seen.insert(number, name) {
                panic!("{name} and {other} share {number:#x}");
            }
        }
        assert!(seen.len() >= 90, "{} operations read", seen.len());
    }
    #[test]
    fn namespace_reserved_bits_and_status_values() {
        for op in [0, 1, 0x501, u32::MAX] {
            assert_eq!(decode_selector(selector(op)), Ok(op));
        }
        assert_eq!(decode_selector(1), Err(Status::InvalidOperation));
        assert_eq!(
            decode_selector(TAG | (1 << 32)),
            Err(Status::VersionMismatch)
        );
        for value in 0..=16 {
            assert_eq!(Status::from_raw(value).unwrap() as u64, value);
        }
        assert!(Status::from_raw(17).is_none());
        assert!(HandleKind::from_raw(0).is_none());
        for value in 0..=3 {
            assert_eq!(CancelOutcome::from_raw(value).unwrap() as u64, value);
        }
        assert!(CancelOutcome::from_raw(4).is_none());
    }
}

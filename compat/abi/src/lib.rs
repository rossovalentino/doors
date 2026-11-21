#![no_std]
//! Windows-facing views are separate from authoritative native process objects.
//! Bootstrap is a private Door loader/DLL contract, never an EXE import ABI.
//!
//! # Private ntdll interface of the compatibility DLLs (P13)
//!
//! Original Door DLLs (user32, gdi32, comdlg32, comctl32, kernel32) import
//! these ntdll exports; applications never see them as a Windows ABI:
//!
//! - `DoorUserCopy(destination, source, bytes) -> NTSTATUS`: one checked
//!   copy through the native boundary, both ranges validated completely
//!   before any byte moves (overlap allowed). Up to `GUI_COPY_BYTES` (64 KiB)
//!   for `WINDOWS_GUI` (manifest 15) processes, 4 KiB for console
//!   applications; larger transfers are split by the caller.
//! - `RtlCreateHeap`/`RtlDestroyHeap`/`RtlAllocateHeap`/`RtlFreeHeap`/
//!   `RtlReAllocateHeap`/`RtlSizeHeap`/`RtlValidateHeap` and
//!   `DoorGetProcessHeap() -> HANDLE` (also `PEB.ProcessHeap`): the process
//!   heap (`heap` core). `allocator::ProcessHeap` (feature `guest-heap`)
//!   wraps it as a Rust `#[global_allocator]`.
//! - `DoorGetInheritedHandle(index) -> u64`: native handles the process was
//!   started with, `door_abi::platform::inherited::REGISTRY` (the service
//!   registry: `door_service_client::Window::connect(registry)` gives the
//!   one window-server connection of a GUI process) and `APP_ASSETS` (the
//!   read-only font/icon pack section, granted to manifest 15). Any other
//!   index, or an absent handle, returns 0.
//! - `DoorGetProcessSubsystem() -> u32`: PE subsystem of the executable
//!   (2 GUI, 3 console) from the trusted DOORWIN2 record.
//! - `DoorFindResource(module, type, name, language) -> HRSRC`,
//!   `DoorLoadResource(module, hrsrc) -> address`,
//!   `DoorSizeofResource(module, hrsrc) -> u32`: `FindResourceExW`
//!   semantics over the kernel-validated resource directory of a module
//!   present in the DOORWIN2 record (`type`/`name` are `MAKEINTRESOURCE`
//!   integers or NUL-terminated UTF-16 pointers; language 0 = neutral).
/// Feature `guest-heap`: the ntdll process heap as a Rust global allocator.
#[cfg(feature = "guest-heap")]
pub mod allocator;
pub mod apc;
/// CommandLineToArgvW's splitting and CreateProcessW's program search.
pub mod arguments;
/// DOORWIN2 startup record of the P13 application path (manifests 10, 15).
pub mod bootstrap;
pub mod console;
pub mod dll_search;
pub mod environment;
pub mod exception;
pub mod file;
pub mod heap;
pub mod io;
/// Windows structures at the build's pointer width (P17).
pub mod layout;
pub mod memory;
/// Fixed-base catalog of the original compatibility DLLs.
pub mod modules;
pub mod path;
pub mod pipe;
/// Feature `guest-runtime`: panic handler and linkage symbols of a Rust-only
/// PE DLL (Windows target builds only).
#[cfg(all(feature = "guest-runtime", target_os = "windows", not(test)))]
mod runtime;
/// `STARTUPINFOW` view of the application path.
pub mod shell_icons;
pub mod startup;
/// The handle values Doors' DLLs make (files, pipes, searches).
pub mod tagged;
pub mod text;
pub mod tls;
pub mod version_info;
/// WOW64's registry, file system and environment views (P17).
pub mod wow64;
pub mod x86;

/// A pointer-width argument of a Windows export (a pointer, handle, SIZE_T,
/// WPARAM or LPARAM) as the 64-bit value Door's internals take. On x64 it
/// is the same bits. On x86 (P17) it is sign-extended: every address of a
/// 32-bit Doors process lies below 2 GB, so pointers keep their value, and
/// the pseudo-handles and INVALID_HANDLE_VALUE (-1, -2, ...) keep theirs.
pub const fn widen(value: usize) -> u64 {
    value as isize as i64 as u64
}
/// A pointer or handle a Windows export returns, at the caller's width.
pub const fn narrow(value: u64) -> usize {
    value as usize
}

pub const BOOTSTRAP_BYTES: usize = 256;
pub const MAX_MODULES: usize = 4;
pub const BOOTSTRAP_ADDRESS: u64 = 0x7100_4000;
pub const STATE_ADDRESS: u64 = 0x7100_8000;
pub const STATE_BYTES: usize = 4 * 4096;
pub const TEB_ADDRESS: u64 = STATE_ADDRESS;
pub const PEB_ADDRESS: u64 = STATE_ADDRESS + 8192;
pub const PARAMETERS_ADDRESS: u64 = STATE_ADDRESS + 12288;
/// A 32-bit process's RTL_USER_PROCESS_PARAMETERS32 (P17), in the PEB32's
/// page: what PEB32.ProcessParameters names, in the layout 32-bit programs
/// read. Its strings are the x64 parameters' (below 2 GB).
pub const PARAMETERS32_ADDRESS: u64 = PEB_ADDRESS + 0x800;
pub const BOOTSTRAP_EXPORT: &str = "DoorProcessBootstrap";
/// Private Door FS storage, beyond the native runtime's bounded TLS record.
/// This is deliberately not a claim about a reserved Windows TEB field.
pub const LAST_ERROR_FS_OFFSET: usize = 512;
pub const THREAD_ID_FS_OFFSET: usize = 520;
pub const THREAD_TEB_FS_OFFSET: usize = 2048;

/// Initialize the SDK-profile NT_TIB/public TEB prefix for one native thread.
/// The PEB remains process-owned. This writes views, never native authority.
pub fn initial_thread_teb(
    output: &mut [u8],
    teb: u64,
    stack_base: u64,
    stack_limit: u64,
) -> Result<(), Error> {
    if output.len() < 104 {
        return Err(Error::Size);
    }
    if !address(teb) || !address(stack_base) || !address(stack_limit) || stack_limit >= stack_base {
        return Err(Error::Address);
    }
    output.fill(0);
    for (offset, value) in [
        (8, stack_base),
        (16, stack_limit),
        (48, teb),
        (96, PEB_ADDRESS),
    ] {
        output[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    Ok(())
}
pub const STD_INPUT_HANDLE: u32 = (-10_i32) as u32;
pub const STD_OUTPUT_HANDLE: u32 = (-11_i32) as u32;
pub const STD_ERROR_HANDLE: u32 = (-12_i32) as u32;
pub const INPUT_HANDLE: u64 = 0x100;
pub const OUTPUT_HANDLE: u64 = 0x104;
pub const ERROR_HANDLE: u64 = 0x108;
/// The `NUL` device: reads end at once, writes are discarded, and it is a
/// character device (FILE_TYPE_CHAR) that is not a console.
pub const NUL_HANDLE: u64 = 0x10C;

/// The Windows version Doors reports (RtlGetVersion, GetVersionEx, the
/// PEB): Windows 11 26H2, as measured on the owner's PC (10.0.26300).
pub mod os_version {
    pub const MAJOR: u32 = 10;
    pub const MINOR: u32 = 0;
    pub const BUILD: u32 = 26300;
    /// VER_PLATFORM_WIN32_NT.
    pub const PLATFORM: u32 = 2;
    /// PEB offsets of OSMajorVersion, OSMinorVersion, OSBuildNumber (u16)
    /// and OSPlatformId: x64, then the PEB32 of a WOW64 process.
    pub const PEB64: [usize; 4] = [0x118, 0x11c, 0x120, 0x124];
    pub const PEB32: [usize; 4] = [0xa4, 0xa8, 0xac, 0xb0];
    /// Write the version into a PEB at `offsets`.
    pub fn write_peb(peb: &mut [u8], offsets: [usize; 4]) {
        peb[offsets[0]..offsets[0] + 4].copy_from_slice(&MAJOR.to_le_bytes());
        peb[offsets[1]..offsets[1] + 4].copy_from_slice(&MINOR.to_le_bytes());
        peb[offsets[2]..offsets[2] + 2].copy_from_slice(&(BUILD as u16).to_le_bytes());
        peb[offsets[3]..offsets[3] + 4].copy_from_slice(&PLATFORM.to_le_bytes());
    }
}

pub mod error {
    pub const INVALID_FUNCTION: u32 = 1;
    pub const FILE_NOT_FOUND: u32 = 2;
    pub const PATH_NOT_FOUND: u32 = 3;
    pub const ACCESS_DENIED: u32 = 5;
    pub const INVALID_DATA: u32 = 13;
    pub const OUTOFMEMORY: u32 = 14;
    pub const NO_MORE_FILES: u32 = 18;
    pub const SHARING_VIOLATION: u32 = 32;
    pub const HANDLE_EOF: u32 = 38;
    pub const FILE_EXISTS: u32 = 80;
    pub const INSUFFICIENT_BUFFER: u32 = 122;
    pub const INVALID_NAME: u32 = 123;
    pub const MOD_NOT_FOUND: u32 = 126;
    pub const PROC_NOT_FOUND: u32 = 127;
    pub const NEGATIVE_SEEK: u32 = 131;
    pub const DIR_NOT_EMPTY: u32 = 145;
    pub const ALREADY_EXISTS: u32 = 183;
    pub const FILENAME_EXCED_RANGE: u32 = 206;
    pub const NOT_OWNER: u32 = 288;
    pub const TOO_MANY_POSTS: u32 = 298;
    pub const DELETE_PENDING: u32 = 303;
    pub const ARITHMETIC_OVERFLOW: u32 = 534;
    pub const INVALID_FLAGS: u32 = 1004;
    pub const TIMEOUT: u32 = 1460;
    pub const RESOURCE_DATA_NOT_FOUND: u32 = 1812;
    pub const RESOURCE_TYPE_NOT_FOUND: u32 = 1813;
    pub const RESOURCE_NAME_NOT_FOUND: u32 = 1814;
    pub const RESOURCE_LANG_NOT_FOUND: u32 = 1815;
    pub const INVALID_HANDLE: u32 = 6;
    pub const NOT_ENOUGH_MEMORY: u32 = 8;
    pub const INVALID_PARAMETER: u32 = 87;
    pub const NOT_SUPPORTED: u32 = 50;
    pub const NOACCESS: u32 = 998;
    pub const IO_DEVICE: u32 = 1117;
    pub const NO_UNICODE_TRANSLATION: u32 = 1113;
    pub const OPERATION_ABORTED: u32 = 995;
    pub const INVALID_USER_BUFFER: u32 = 1784;
}

/// NT status values used by the initial synchronous adapter. Unknown statuses
/// stay failures; no fallback reports an unsupported operation as successful.
pub mod status {
    pub const SUCCESS: u32 = 0;
    pub const INVALID_HANDLE: u32 = 0xc000_0008;
    pub const INVALID_PARAMETER: u32 = 0xc000_000d;
    pub const ACCESS_VIOLATION: u32 = 0xc000_0005;
    pub const INVALID_USER_BUFFER: u32 = 0xc000_00e8;
    pub const NO_MEMORY: u32 = 0xc000_0017;
    pub const NOT_SUPPORTED: u32 = 0xc000_00bb;
    pub const IO_DEVICE_ERROR: u32 = 0xc000_0185;
    pub const CANCELLED: u32 = 0xc000_0120;
    pub const UNMAPPABLE_CHARACTER: u32 = 0xc000_0162;
    pub const PENDING: u32 = 0x103;
    pub const ACCESS_DENIED: u32 = 0xc000_0022;
    pub const BUFFER_OVERFLOW: u32 = 0x8000_0005;
    pub const IO_TIMEOUT: u32 = 0xc000_00b5;
    pub const INVALID_CONNECTION: u32 = 0xc000_0140;
    pub const CONNECTION_RESET: u32 = 0xc000_020d;
    pub const CONNECTION_REFUSED: u32 = 0xc000_0236;
    pub const NETWORK_UNREACHABLE: u32 = 0xc000_023c;
    pub const HOST_UNREACHABLE: u32 = 0xc000_023d;
    pub const CONNECTION_ABORTED: u32 = 0xc000_0241;
    pub const UNSUCCESSFUL: u32 = 0xc000_0001;
}

pub const CURRENT_PROCESS: u64 = u64::MAX;
pub const COPY_BYTES: usize = 1024;
/// Bounded checked copy per call (`DoorUserCopy`) of a WINDOWS_GUI process.
pub const GUI_COPY_BYTES: usize = 64 * 1024;
/// Bounded checked copy per call of a console application or P10 process.
pub const APPLICATION_COPY_BYTES: usize = 4096;
/// What the kernel copies per checked call for any Windows process (P18:
/// file data moves in 64 KiB steps; `APPLICATION_COPY_BYTES` stays the
/// chunk the DLLs buffer on their stacks).
pub const BULK_COPY_BYTES: usize = 64 * 1024;
pub const IO_STATUS_BYTES: usize = 16;

pub fn dos_error(status: u32) -> u32 {
    match status {
        status::SUCCESS => 0,
        status::INVALID_HANDLE => error::INVALID_HANDLE,
        status::INVALID_PARAMETER => error::INVALID_PARAMETER,
        status::ACCESS_VIOLATION => error::NOACCESS,
        status::INVALID_USER_BUFFER => error::INVALID_USER_BUFFER,
        status::NO_MEMORY => error::NOT_ENOUGH_MEMORY,
        status::NOT_SUPPORTED => error::NOT_SUPPORTED,
        status::IO_DEVICE_ERROR => error::IO_DEVICE,
        status::CANCELLED => error::OPERATION_ABORTED,
        status::UNMAPPABLE_CHARACTER => error::NO_UNICODE_TRANSLATION,
        // RtlNtStatusToDosError for the socket and network statuses.
        status::PENDING => 997,
        status::ACCESS_DENIED => 5,
        status::BUFFER_OVERFLOW => 234,
        status::IO_TIMEOUT => 121,
        status::INVALID_CONNECTION => 1229,
        status::CONNECTION_RESET => 64,
        status::CONNECTION_REFUSED => 1225,
        status::NETWORK_UNREACHABLE => 1231,
        status::HOST_UNREACHABLE => 1232,
        status::CONNECTION_ABORTED => 1236,
        status::UNSUCCESSFUL => 31,
        _ => 317, // ERROR_MR_MID_NOT_FOUND
    }
}

pub fn standard_handle(selector: u32) -> Result<u64, u32> {
    match selector {
        STD_INPUT_HANDLE => Ok(INPUT_HANDLE),
        STD_OUTPUT_HANDLE => Ok(OUTPUT_HANDLE),
        STD_ERROR_HANDLE => Ok(ERROR_HANDLE),
        _ => Err(status::INVALID_HANDLE),
    }
}

pub fn writable_console(handle: u64) -> bool {
    matches!(handle, OUTPUT_HANDLE | ERROR_HANDLE)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ModuleInit {
    pub base: u64,
    /// A DLL may have no entry point. Nonzero entries receive DLL_PROCESS_ATTACH.
    pub entry: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bootstrap {
    pub native_startup: u64,
    pub executable_entry: u64,
    pub executable_base: u64,
    pub teb: u64,
    pub peb: u64,
    pub parameters: u64,
    pub module_count: u32,
    /// Dependency initialization order, excluding the executable.
    pub modules: [ModuleInit; MAX_MODULES],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Size,
    Version,
    Reserved,
    Address,
    Limit,
}

fn address(value: u64) -> bool {
    (0x1_0000..0x8000_0000_0000).contains(&value)
}
impl Bootstrap {
    fn validate(&self) -> Result<(), Error> {
        if self.module_count as usize > MAX_MODULES {
            return Err(Error::Limit);
        }
        for value in [
            self.native_startup,
            self.executable_entry,
            self.executable_base,
            self.teb,
            self.peb,
            self.parameters,
        ] {
            if !address(value) {
                return Err(Error::Address);
            }
        }
        for (index, module) in self.modules.iter().enumerate() {
            if index < self.module_count as usize {
                if !address(module.base) || (module.entry != 0 && !address(module.entry)) {
                    return Err(Error::Address);
                }
            } else if *module != ModuleInit::default() {
                return Err(Error::Reserved);
            }
        }
        Ok(())
    }
    pub fn encode(&self, output: &mut [u8]) -> Result<(), Error> {
        if output.len() != BOOTSTRAP_BYTES {
            return Err(Error::Size);
        }
        self.validate()?;
        output.fill(0);
        output[..8].copy_from_slice(b"DOORWIN1");
        output[8..12].copy_from_slice(&1u32.to_le_bytes());
        output[12..16].copy_from_slice(&(BOOTSTRAP_BYTES as u32).to_le_bytes());
        for (index, value) in [
            self.native_startup,
            self.executable_entry,
            self.executable_base,
            self.teb,
            self.peb,
            self.parameters,
        ]
        .iter()
        .enumerate()
        {
            output[16 + index * 8..24 + index * 8].copy_from_slice(&value.to_le_bytes());
        }
        output[64..68].copy_from_slice(&self.module_count.to_le_bytes());
        for (index, module) in self.modules.iter().enumerate() {
            let at = 72 + index * 16;
            output[at..at + 8].copy_from_slice(&module.base.to_le_bytes());
            output[at + 8..at + 16].copy_from_slice(&module.entry.to_le_bytes());
        }
        Ok(())
    }
    pub fn decode(input: &[u8]) -> Result<Self, Error> {
        if input.len() != BOOTSTRAP_BYTES {
            return Err(Error::Size);
        }
        if &input[..8] != b"DOORWIN1"
            || input[8..12] != 1u32.to_le_bytes()
            || input[12..16] != (BOOTSTRAP_BYTES as u32).to_le_bytes()
        {
            return Err(Error::Version);
        }
        if input[68..72]
            .iter()
            .chain(input[136..].iter())
            .any(|b| *b != 0)
        {
            return Err(Error::Reserved);
        }
        let get = |at: usize| u64::from_le_bytes(input[at..at + 8].try_into().unwrap());
        let result = Self {
            native_startup: get(16),
            executable_entry: get(24),
            executable_base: get(32),
            teb: get(40),
            peb: get(48),
            parameters: get(56),
            module_count: u32::from_le_bytes(input[64..68].try_into().unwrap()),
            modules: core::array::from_fn(|index| ModuleInit {
                base: get(72 + index * 16),
                entry: get(80 + index * 16),
            }),
        };
        result.validate()?;
        Ok(result)
    }
}

/// Populate only pinned public SDK fields; unspecified Windows fields stay zero.
/// NT_TIB stack bounds/Self, TEB.ProcessEnvironmentBlock, x64 PEB parameters/session,
/// and RTL_USER_PROCESS_PARAMETERS Unicode strings use the acquired SDK layouts.
pub fn initial_state(
    output: &mut [u8],
    stack_base: u64,
    stack_limit: u64,
    session: u32,
    image_path: &[u16],
    command_line: &[u16],
) -> Result<(), Error> {
    if output.len() != STATE_BYTES {
        return Err(Error::Size);
    }
    if !address(stack_base) || !address(stack_limit) || stack_limit >= stack_base {
        return Err(Error::Address);
    }
    if image_path.len() > 127 || command_line.len() > 127 {
        return Err(Error::Limit);
    }
    output.fill(0);
    let put = |bytes: &mut [u8], at: usize, value: u64| {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes())
    };
    initial_thread_teb(&mut output[..8192], TEB_ADDRESS, stack_base, stack_limit)?;
    put(output, 8192 + 32, PARAMETERS_ADDRESS);
    output[8192 + 704..8192 + 708].copy_from_slice(&session.to_le_bytes());
    os_version::write_peb(&mut output[8192..12288], os_version::PEB64);
    for (descriptor, storage, text) in [(96, 256, image_path), (112, 512, command_line)] {
        let at = 12288 + descriptor;
        output[at..at + 2].copy_from_slice(&((text.len() * 2) as u16).to_le_bytes());
        output[at + 2..at + 4].copy_from_slice(&(((text.len() + 1) * 2) as u16).to_le_bytes());
        put(output, at + 8, PARAMETERS_ADDRESS + storage as u64);
        for (index, unit) in text.iter().enumerate() {
            let start = 12288 + storage + index * 2;
            output[start..start + 2].copy_from_slice(&unit.to_le_bytes());
        }
    }
    Ok(())
}

/// `initial_state` for a 32-bit (x86) process (P17): the main thread's
/// TEB32 at `TEB_ADDRESS` (FS), its implicit TLS array at `tls_array`, and
/// the PEB32 at `PEB_ADDRESS` (ImageBaseAddress, ProcessParameters,
/// SessionId). The process parameters keep the layout `initial_state`
/// writes: Doors' own DLLs read them, not the program.
#[allow(clippy::too_many_arguments)]
pub fn initial_state32(
    output: &mut [u8],
    stack_base: u64,
    stack_limit: u64,
    session: u32,
    image_path: &[u16],
    command_line: &[u16],
    image_base: u64,
    tls_array: u64,
) -> Result<(), Error> {
    initial_state(
        output,
        stack_base,
        stack_limit,
        session,
        image_path,
        command_line,
    )?;
    let low = |value: u64| u32::try_from(value).map_err(|_| Error::Address);
    output[..8192].fill(0);
    x86::initial_teb(
        &mut output[..8192],
        low(TEB_ADDRESS)?,
        low(stack_base)?,
        low(stack_limit)?,
        0,
        0,
        low(tls_array)?,
        low(PEB_ADDRESS)?,
    )?;
    // The x64 parameters' string descriptors (Length, MaximumLength, Buffer).
    let descriptor = |output: &[u8], at: usize| -> Result<[u8; 8], Error> {
        let mut d = [0u8; 8];
        d[..4].copy_from_slice(&output[12288 + at..12288 + at + 4]);
        let buffer =
            u64::from_le_bytes(output[12288 + at + 8..12288 + at + 16].try_into().unwrap());
        d[4..].copy_from_slice(&low(buffer)?.to_le_bytes());
        Ok(d)
    };
    let image = descriptor(output, 96)?;
    let command = descriptor(output, 112)?;
    let peb = &mut output[8192..12288];
    peb.fill(0);
    for (offset, value) in [
        (x86::PEB_IMAGE_BASE, low(image_base)?),
        (x86::PEB_PROCESS_PARAMETERS, low(PARAMETERS32_ADDRESS)?),
        (x86::PEB_SESSION_ID, session),
    ] {
        peb[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    os_version::write_peb(peb, os_version::PEB32);
    let parameters = &mut peb[(PARAMETERS32_ADDRESS - PEB_ADDRESS) as usize..];
    for (offset, value) in [
        (x86::PARAMETERS32_MAXIMUM_LENGTH, x86::PARAMETERS32_BYTES),
        (x86::PARAMETERS32_LENGTH, x86::PARAMETERS32_BYTES),
        // RTL_USER_PROC_PARAMS_NORMALIZED: pointers, not offsets.
        (x86::PARAMETERS32_FLAGS, 1),
    ] {
        parameters[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    parameters[x86::PARAMETERS32_IMAGE_PATH..x86::PARAMETERS32_IMAGE_PATH + 8]
        .copy_from_slice(&image);
    parameters[x86::PARAMETERS32_COMMAND_LINE..x86::PARAMETERS32_COMMAND_LINE + 8]
        .copy_from_slice(&command);
    Ok(())
}

/// The Win32 spelling of a native program path on C: (`/compat/X` is
/// `C:\X`, `/users/X` is `C:\Users\X`), written to `out`: what Windows puts in
/// ImagePathName and argv[0]. `None` for a path off C: (`/system/...`) or one
/// that does not fit.
pub fn windows_image_path(native: &[u8], out: &mut [u8]) -> Option<usize> {
    let (prefix, rest): (&[u8], &[u8]) = if let Some(rest) = native.strip_prefix(b"/compat") {
        (b"C:", rest)
    } else if let Some(rest) = native.strip_prefix(b"/users") {
        (b"C:\\Users", rest)
    } else {
        return None;
    };
    // `/compatible/...` is not `/compat`.
    if !rest.is_empty() && rest[0] != b'/' {
        return None;
    }
    let rest = if rest.is_empty() { b"/" } else { rest };
    let total = prefix.len() + rest.len();
    let out = out.get_mut(..total)?;
    out[..prefix.len()].copy_from_slice(prefix);
    for (o, &b) in out[prefix.len()..].iter_mut().zip(rest) {
        *o = if b == b'/' { b'\\' } else { b };
    }
    Some(total)
}

/// The Windows command line of a launch argument block (UTF-8 arguments, each
/// NUL-terminated). Arguments are separated by one space and quoted only when
/// empty or containing a space, tab, newline or quote; inside quotes the
/// backslashes before a quote (or the closing quote) are doubled and quotes are
/// escaped. This is what cmd.exe and Explorer pass (`app.exe --reopen`) and what
/// `CommandLineToArgvW`/the MSVC CRT invert, so a program that reads
/// `GetCommandLineW` itself sees ordinary arguments unquoted.
pub fn command_line(arguments: &[u8], output: &mut [u16]) -> Result<usize, Error> {
    let body = arguments.strip_suffix(&[0]).ok_or(Error::Limit)?;
    let mut count = 0;
    let mut push = |unit: u16| -> Result<(), Error> {
        *output.get_mut(count).ok_or(Error::Limit)? = unit;
        count += 1;
        Ok(())
    };
    for (index, argument) in body.split(|b| *b == 0).enumerate() {
        let text = core::str::from_utf8(argument).map_err(|_| Error::Limit)?;
        if index != 0 {
            push(u16::from(b' '))?;
        }
        if !text.is_empty() && !text.contains([' ', '\t', '\n', '\u{b}', '"']) {
            for unit in text.encode_utf16() {
                push(unit)?;
            }
            continue;
        }
        push(u16::from(b'"'))?;
        let mut slashes = 0;
        for unit in text.encode_utf16() {
            if unit == u16::from(b'\\') {
                slashes += 1;
            } else {
                if unit == u16::from(b'"') {
                    for _ in 0..=slashes {
                        push(u16::from(b'\\'))?;
                    }
                }
                slashes = 0;
            }
            push(unit)?;
        }
        for _ in 0..slashes {
            push(u16::from(b'\\'))?;
        }
        push(u16::from(b'"'))?;
    }
    Ok(count)
}

/// Populate ordinary RTL process parameters from captured native UTF-8 variables.
/// The environment and current directory occupy private mapped parameter storage.
pub fn initial_environment(output: &mut [u8], environment: &[u8]) -> Result<(), Error> {
    if output.len() != STATE_BYTES {
        return Err(Error::Size);
    }
    let text = core::str::from_utf8(environment).map_err(|_| Error::Limit)?;
    if !environment.is_empty() && environment.last() != Some(&0) {
        return Err(Error::Limit);
    }
    let parameters = 12288;
    let environment_offset = 1536;
    for (units, unit) in text.encode_utf16().chain(core::iter::once(0)).enumerate() {
        if units >= 512 {
            return Err(Error::Limit);
        }
        let at = parameters + environment_offset + units * 2;
        output[at..at + 2].copy_from_slice(&unit.to_le_bytes());
    }
    output[parameters + 128..parameters + 136]
        .copy_from_slice(&(PARAMETERS_ADDRESS + environment_offset as u64).to_le_bytes());
    if let Some(cwd) = text
        .split('\0')
        .find_map(|entry| entry.strip_prefix("PWD="))
    {
        if !cwd.starts_with('/') {
            return Err(Error::Limit);
        }
        let mut count = 0;
        for unit in cwd.encode_utf16() {
            if count >= 255 {
                return Err(Error::Limit);
            }
            let unit = if unit == 47 { 92 } else { unit };
            let at = parameters + 1024 + count * 2;
            output[at..at + 2].copy_from_slice(&unit.to_le_bytes());
            count += 1;
        }
        output[parameters + 56..parameters + 58]
            .copy_from_slice(&((count * 2) as u16).to_le_bytes());
        output[parameters + 58..parameters + 60]
            .copy_from_slice(&(((count + 1) * 2) as u16).to_le_bytes());
        output[parameters + 64..parameters + 72]
            .copy_from_slice(&(PARAMETERS_ADDRESS + 1024).to_le_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_program_paths_on_c_take_their_win32_spelling() {
        let mut out = [0u8; 64];
        for (native, windows) in [
            (
                &b"/compat/Program Files/dotnet/dotnet.exe"[..],
                Some(&b"C:\\Program Files\\dotnet\\dotnet.exe"[..]),
            ),
            (b"/users/1001/app.exe", Some(b"C:\\Users\\1001\\app.exe")),
            (b"/compat", Some(b"C:\\")),
            (b"/system/bin/crtprobe.exe", None),
            (b"/compatible/x.exe", None),
        ] {
            let got = windows_image_path(native, &mut out).map(|n| &out[..n]);
            assert_eq!(got, windows, "{}", core::str::from_utf8(native).unwrap());
        }
        assert_eq!(windows_image_path(b"/compat/x.exe", &mut [0u8; 4]), None);
    }
    #[test]
    fn startup_rejects_reserved_entries_limits_and_invalid_addresses() {
        let b = Bootstrap {
            native_startup: 0x7100_0000,
            executable_entry: 0x0001_4000_1000,
            executable_base: 0x0001_4000_0000,
            teb: TEB_ADDRESS,
            peb: PEB_ADDRESS,
            parameters: PARAMETERS_ADDRESS,
            module_count: 1,
            modules: [
                ModuleInit {
                    base: 0x0001_8000_0000,
                    entry: 0,
                },
                ModuleInit::default(),
                ModuleInit::default(),
                ModuleInit::default(),
            ],
        };
        let mut bytes = [0; BOOTSTRAP_BYTES];
        b.encode(&mut bytes).unwrap();
        assert_eq!(Bootstrap::decode(&bytes), Ok(b));
        for at in [68, 136, 255] {
            let mut bad = bytes;
            bad[at] = 1;
            assert_eq!(Bootstrap::decode(&bad), Err(Error::Reserved));
        }
        let mut bad = b;
        bad.module_count = 5;
        assert_eq!(bad.encode(&mut bytes), Err(Error::Limit));
        let mut bad = b;
        bad.modules[1].base = 0x0001_8000_0000;
        assert_eq!(bad.encode(&mut bytes), Err(Error::Reserved));
        let mut bad = b;
        bad.executable_entry = u64::MAX;
        assert_eq!(bad.encode(&mut bytes), Err(Error::Address));
    }
    #[test]
    fn public_views_keep_pointer_layout_and_utf16_lengths() {
        let mut bytes = [0xaa; STATE_BYTES];
        initial_state(
            &mut bytes,
            0x7004_1000,
            0x7000_1000,
            7,
            &[67, 58, 92, 0xd800],
            &[34, 65, 34],
        )
        .unwrap();
        let get = |at| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        assert_eq!(get(48), TEB_ADDRESS);
        assert_eq!(get(96), PEB_ADDRESS);
        assert_eq!(get(8192 + 32), PARAMETERS_ADDRESS);
        assert_eq!(&bytes[8192 + 704..8192 + 708], &7u32.to_le_bytes());
        // OSMajorVersion, OSMinorVersion, OSBuildNumber, OSPlatformId.
        assert_eq!(&bytes[8192 + 0x118..8192 + 0x11c], &10u32.to_le_bytes());
        assert_eq!(&bytes[8192 + 0x120..8192 + 0x122], &26300u16.to_le_bytes());
        assert_eq!(&bytes[8192 + 0x124..8192 + 0x128], &2u32.to_le_bytes());
        assert_eq!(get(12288 + 104), PARAMETERS_ADDRESS + 256);
        assert_eq!(&bytes[12288 + 96..12288 + 100], &[8, 0, 10, 0]);
        assert_eq!(&bytes[12288 + 262..12288 + 266], &[0, 0xd8, 0, 0]);
        assert_eq!(
            initial_state(&mut bytes, 1, 2, 0, &[], &[]),
            Err(Error::Address)
        );
    }
}

#[cfg(test)]
mod command_line_tests {
    extern crate std;
    use super::*;
    use std::string::String;
    fn line(arguments: &[u8]) -> Result<String, Error> {
        let mut out = [0u16; 128];
        let n = command_line(arguments, &mut out)?;
        Ok(String::from_utf16(&out[..n]).unwrap())
    }
    #[test]
    fn ordinary_arguments_stay_unquoted_and_special_ones_round_trip() {
        assert_eq!(
            line(b"/system/bin/wingui-selftest.exe\0--reopen\0").unwrap(),
            "/system/bin/wingui-selftest.exe --reopen"
        );
        assert_eq!(line(b"app.exe\0").unwrap(), "app.exe");
        assert_eq!(
            line(b"C:\\Program Files\\a.exe\0\0two words\0").unwrap(),
            "\"C:\\Program Files\\a.exe\" \"\" \"two words\""
        );
        // Backslashes are literal unless they precede a quote.
        assert_eq!(
            line(b"a\0x\\y\0say \"hi\"\0end\\ \\\0").unwrap(),
            "a x\\y \"say \\\"hi\\\"\" \"end\\ \\\\\""
        );
        assert_eq!(line(b"a\0caf\xc3\xa9\0").unwrap(), "a caf\u{e9}");
        assert_eq!(line(b"unterminated"), Err(Error::Limit));
        assert_eq!(line(b"a\0\xff\0"), Err(Error::Limit));
        let mut small = [0u16; 4];
        assert_eq!(command_line(b"abcde\0", &mut small), Err(Error::Limit));
    }
}
#[cfg(test)]
mod environment_tests {
    extern crate std;
    use super::*;
    #[test]
    fn ordinary_environment_has_owned_utf16_storage_and_cwd() {
        let mut bytes = [0; STATE_BYTES];
        initial_environment(&mut bytes, b"USER=1001\0PWD=/users/1001/work\0").unwrap();
        assert_eq!(
            u64::from_le_bytes(bytes[12416..12424].try_into().unwrap()),
            PARAMETERS_ADDRESS + 1536
        );
        let length = u16::from_le_bytes(bytes[12344..12346].try_into().unwrap()) as usize;
        let cwd: std::vec::Vec<u16> = bytes[13312..13312 + length]
            .chunks_exact(2)
            .map(|v| u16::from_le_bytes([v[0], v[1]]))
            .collect();
        assert_eq!(
            std::string::String::from_utf16(&cwd).unwrap(),
            "\\users\\1001\\work"
        );
        assert_eq!(
            initial_environment(&mut bytes, b"PWD=relative\0"),
            Err(Error::Limit)
        );
    }
}

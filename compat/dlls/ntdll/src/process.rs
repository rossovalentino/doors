//! CreateProcessW over Init's EXECUTE: the program is found the way Windows
//! looks for it (an unquoted name with spaces tries each space in turn,
//! ".exe" is added to a name without an extension, a bare name is searched
//! in the program's own directory, the current directory, System32, the
//! Windows directory and Path), the command line is split by
//! CommandLineToArgvW's rules with the program's full path as argv[0], and
//! Init starts it in the session with the caller's directory authority.
//! The child joins the caller's console unless DETACHED_PROCESS or
//! CREATE_NO_WINDOW ask for none. Arguments travel in the startup record;
//! longer ones (up to 2048 bytes) are staged with Init and fetched by the
//! child at startup; a longer command line is ERROR_FILENAME_EXCED_RANGE.
use super::files::PATH_TEXT;
use super::guest::{RtlSetLastWin32Error, copy};
use door_abi::{HandleKind, platform::inherited, rights};
use door_service_protocol::Code;

const DETACHED_PROCESS: u32 = 0x8;
const CREATE_SUSPENDED: u32 = 0x4;
const CREATE_UNICODE_ENVIRONMENT: u32 = 0x400;
/// An environment block (lpEnvironment, UTF-16 or ANSI per the flags) as
/// NUL-separated UTF-8 entries into `out`: their length.
fn environment_block(pointer: u64, unicode: bool, out: &mut [u8]) -> Result<usize, u32> {
    let width: u64 = if unicode { 2 } else { 1 };
    let unit_at = |index: u64| -> Result<u32, u32> {
        let mut raw = [0u8; 2];
        copy(
            raw.as_mut_ptr() as u64,
            pointer + width * index,
            width as usize,
        )
        .map_err(|_| 998u32)?;
        Ok(if unicode {
            u32::from(u16::from_le_bytes(raw))
        } else {
            u32::from(raw[0])
        })
    };
    let mut previous_nul = true;
    let mut written = 0usize;
    let mut index = 0u64;
    // At most 32767 characters, as Windows.
    while index < 32_768 {
        let unit = unit_at(index)?;
        index += 1;
        if unit == 0 {
            if previous_nul {
                // The block's final empty entry.
                return Ok(written);
            }
            *out.get_mut(written).ok_or(206u32)? = 0;
            written += 1;
            previous_nul = true;
            continue;
        }
        previous_nul = false;
        // UTF-16 surrogates pair up; ANSI bytes above 0x7f read as Latin-1.
        let character = if unicode && (0xd800..0xdc00).contains(&unit) {
            let low = unit_at(index)?;
            if (0xdc00..0xe000).contains(&low) {
                index += 1;
                char::from_u32(0x10000 + ((unit - 0xd800) << 10) + (low - 0xdc00))
            } else {
                None
            }
        } else {
            char::from_u32(unit)
        }
        .unwrap_or('\u{fffd}');
        let mut bytes = [0u8; 4];
        let encoded = character.encode_utf8(&mut bytes);
        out.get_mut(written..written + encoded.len())
            .ok_or(206u32)?
            .copy_from_slice(encoded.as_bytes());
        written += encoded.len();
    }
    Err(206)
}
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const STILL_ACTIVE: u32 = 259;
/// Command-line units read (a longer line could never fit the arguments).
const LINE_UNITS: usize = 2048;

fn failed(error: u32) -> i32 {
    RtlSetLastWin32Error(error);
    0
}
/// A NUL-terminated UTF-16 string at `pointer` into `out`; its length.
fn read_units(pointer: u64, out: &mut [u16]) -> Result<usize, u32> {
    for (index, slot) in out.iter_mut().enumerate() {
        let mut unit = [0u8; 2];
        copy(unit.as_mut_ptr() as u64, pointer + 2 * index as u64, 2).map_err(|_| 998u32)?;
        let unit = u16::from_le_bytes(unit);
        if unit == 0 {
            return Ok(index);
        }
        *slot = unit;
    }
    Err(206)
}
fn is_separator(unit: u16) -> bool {
    unit == u16::from(b'\\') || unit == u16::from(b'/')
}
/// The file name has an extension (a '.' after the last separator).
fn has_extension(path: &[u16]) -> bool {
    let name = path
        .iter()
        .rposition(|&u| is_separator(u))
        .map_or(path, |i| &path[i + 1..]);
    name.contains(&u16::from(b'.'))
}
/// Appends `parts` to `out` at `length`.
fn join(out: &mut [u16], mut length: usize, parts: &[&[u16]]) -> Option<usize> {
    for part in parts {
        out.get_mut(length..length + part.len())?
            .copy_from_slice(part);
        length += part.len();
    }
    Some(length)
}
fn wide(text: &str, out: &mut [u16]) -> usize {
    let mut n = 0;
    for (slot, unit) in out.iter_mut().zip(text.encode_utf16()) {
        *slot = unit;
        n += 1;
    }
    n
}
/// The native path of the program `name` names, searched like Windows.
fn find(name: &[u16], native: &mut [u8; PATH_TEXT]) -> Result<Option<usize>, u32> {
    let mut with_exe = [0u16; 520];
    let exe = [
        u16::from(b'.'),
        u16::from(b'e'),
        u16::from(b'x'),
        u16::from(b'e'),
    ];
    let length = if has_extension(name) {
        join(&mut with_exe, 0, &[name])
    } else {
        join(&mut with_exe, 0, &[name, &exe])
    }
    .ok_or(206u32)?;
    let name = &with_exe[..length];
    if name
        .iter()
        .any(|&u| is_separator(u) || u == u16::from(b':'))
    {
        return super::files::existing_file(name, native);
    }
    let mut directory = [0u16; 520];
    let backslash = [u16::from(b'\\')];
    // The caller's own directory.
    let module = super::modules::DoorGetModuleFileNameW(
        0,
        directory.as_mut_ptr() as u64,
        directory.len() as u32,
    ) as usize;
    if let Some(end) = directory[..module.min(directory.len())]
        .iter()
        .rposition(|&u| is_separator(u))
    {
        let mut path = [0u16; 1040];
        let n = join(&mut path, 0, &[&directory[..=end], name]).ok_or(206u32)?;
        if let Some(found) = super::files::existing_file(&path[..n], native)? {
            return Ok(Some(found));
        }
    }
    // The current directory.
    if let Some(found) = super::files::existing_file(name, native)? {
        return Ok(Some(found));
    }
    for fixed in [r"C:\Windows\System32", r"C:\Windows"] {
        let n = wide(fixed, &mut directory);
        let mut path = [0u16; 1040];
        let n = join(&mut path, 0, &[&directory[..n], &backslash, name]).ok_or(206u32)?;
        if let Some(found) = super::files::existing_file(&path[..n], native)? {
            return Ok(Some(found));
        }
    }
    // Path.
    let mut variable = [0u16; 2048];
    let key = [
        u16::from(b'P'),
        u16::from(b'a'),
        u16::from(b't'),
        u16::from(b'h'),
        0,
    ];
    let n = super::environment::DoorGetEnvironmentVariableW(
        key.as_ptr() as u64,
        variable.as_mut_ptr() as u64,
        variable.len() as u32,
    ) as usize;
    if n > 0 && n < variable.len() {
        for entry in variable[..n]
            .split(|&u| u == u16::from(b';'))
            .filter(|e| !e.is_empty())
        {
            let mut path = [0u16; 1040];
            let entry = if entry.last().is_some_and(|&u| is_separator(u)) {
                &entry[..entry.len() - 1]
            } else {
                entry
            };
            let Some(n) = join(&mut path, 0, &[entry, &backslash, name]) else {
                continue;
            };
            if let Some(found) = super::files::existing_file(&path[..n], native)? {
                return Ok(Some(found));
            }
        }
    }
    Ok(None)
}
/// `C:` plus the native path with backslashes: the program's Windows path.
fn windows_path(native: &str, out: &mut [u8; PATH_TEXT + 2]) -> usize {
    // ntdll's paths are already Windows paths on C:.
    let native = native.strip_prefix("C:").unwrap_or(native);
    out[0] = b'C';
    out[1] = b':';
    let mut n = 2;
    for &byte in native.as_bytes() {
        if n == out.len() {
            break;
        }
        out[n] = if byte == b'/' { b'\\' } else { byte };
        n += 1;
    }
    n
}
fn execute_error(error: door_service_client::Error) -> u32 {
    match error {
        door_service_client::Error::Service(Code::NotFound) => 2,
        door_service_client::Error::Service(Code::Denied) => 5,
        door_service_client::Error::Service(Code::Resources) => 1450,
        door_service_client::Error::Service(Code::InvalidRequest) => 193,
        _ => 31,
    }
}

/// `CreateProcessW`'s work; `info` receives PROCESS_INFORMATION.
#[unsafe(no_mangle)]
pub extern "system" fn DoorCreateProcessW(
    application: u64,
    command_line: u64,
    directory: u64,
    flags: u32,
    info: u64,
    environment: u64,
) -> i32 {
    DoorCreateProcessExW(
        application,
        command_line,
        directory,
        flags,
        info,
        environment,
        0,
        0,
    )
}
/// STARTUPINFOW's dwFlags and hStdInput/Output/Error (at the program's
/// width, P17) when bInheritHandles and STARTF_USESTDHANDLES ask for them.
fn standard_handles(inherit: i32, startup: u64) -> Result<[u64; 3], u32> {
    use super::shapes::{STARTUPINFOW, get, size, startup as field};
    const STARTF_USESTDHANDLES: u32 = 0x100;
    if inherit == 0 || startup == 0 {
        return Ok([0; 3]);
    }
    let mut record = [0u8; size(&STARTUPINFOW)];
    copy(record.as_mut_ptr() as u64, startup, record.len()).map_err(|_| 998u32)?;
    let flags = get(&record, &STARTUPINFOW, field::FLAGS) as u32;
    if flags & STARTF_USESTDHANDLES == 0 {
        return Ok([0; 3]);
    }
    Ok(core::array::from_fn(|i| {
        get(&record, &STARTUPINFOW, field::STD_INPUT + i)
    }))
}
/// Whether the image at the native `program` has a manifest whose
/// requestedExecutionLevel is requireAdministrator or highestAvailable:
/// the headers, the resource section's RT_MANIFEST entry and its data are
/// read where they are, not the whole file.
fn requires_administrator(program: &str) -> bool {
    use door_service_client::Disposition;
    super::files::with_storage(|s| {
        let file = s
            .open(program, Disposition::OpenExisting, 1, 1)
            .map_err(super::files::service)?;
        let mut read = |offset: u64, out: &mut [u8]| -> Result<(), u32> {
            match s.read_full(&file, offset, out) {
                Ok(n) if n == out.len() => Ok(()),
                _ => Err(13u32),
            }
        };
        let result = (|| -> Result<bool, u32> {
            let mut head = [0u8; 1024];
            read(0, &mut head)?;
            let u16_at = |b: &[u8], at: usize| -> Result<u32, u32> {
                Ok(u32::from(u16::from_le_bytes(
                    b.get(at..at + 2).ok_or(13u32)?.try_into().unwrap(),
                )))
            };
            let u32_at = |b: &[u8], at: usize| -> Result<u32, u32> {
                Ok(u32::from_le_bytes(
                    b.get(at..at + 4).ok_or(13u32)?.try_into().unwrap(),
                ))
            };
            if &head[..2] != b"MZ" {
                return Ok(false);
            }
            let pe = u32_at(&head, 0x3c)? as usize;
            if head.get(pe..pe + 4) != Some(b"PE\0\0") {
                return Ok(false);
            }
            let sections = u16_at(&head, pe + 6)? as usize;
            let optional = pe + 24;
            let directories = match u16_at(&head, optional)? {
                0x20b => optional + 112,
                0x10b => optional + 96,
                _ => return Ok(false),
            };
            let resource = u32_at(&head, directories + 16)?;
            if resource == 0 {
                return Ok(false);
            }
            let table = optional + u16_at(&head, pe + 20)? as usize;
            // RVA to file offset by the section table.
            let offset = |rva: u32| -> Result<u64, u32> {
                for i in 0..sections.min(32) {
                    let s = table + 40 * i;
                    let va = u32_at(&head, s + 12)?;
                    let size = u32_at(&head, s + 8)?.max(u32_at(&head, s + 16)?);
                    if rva >= va && rva < va.saturating_add(size) {
                        return Ok(u64::from(rva - va + u32_at(&head, s + 20)?));
                    }
                }
                Err(13)
            };
            let root = offset(resource)?;
            // One level: the entry with `id` (or the first) of the
            // directory at `at` (an offset from the resource root).
            let mut level = |at: u32, id: Option<u32>| -> Result<Option<u32>, u32> {
                let mut header = [0u8; 16];
                read(root + u64::from(at), &mut header)?;
                let count = (u16_at(&header, 12)? + u16_at(&header, 14)?).min(64);
                for i in 0..count {
                    let mut entry = [0u8; 8];
                    read(root + u64::from(at) + 16 + 8 * u64::from(i), &mut entry)?;
                    let name = u32_at(&entry, 0)?;
                    if id.is_none_or(|id| name == id) {
                        return Ok(Some(u32_at(&entry, 4)?));
                    }
                }
                Ok(None)
            };
            let Some(names) = level(0, Some(24))? else {
                return Ok(false);
            };
            let Some(languages) = level(names & 0x7fff_ffff, None)? else {
                return Ok(false);
            };
            let Some(data) = level(languages & 0x7fff_ffff, None)? else {
                return Ok(false);
            };
            if data & 0x8000_0000 != 0 {
                return Ok(false);
            }
            let mut entry = [0u8; 8];
            read(root + u64::from(data), &mut entry)?;
            let mut manifest = [0u8; 4096];
            let size = (u32_at(&entry, 4)? as usize).min(manifest.len());
            read(offset(u32_at(&entry, 0)?)?, &mut manifest[..size])?;
            Ok(matches!(
                door_image_format::execution_level(&manifest[..size]),
                door_image_format::ExecutionLevel::RequireAdministrator
                    | door_image_format::ExecutionLevel::HighestAvailable
            ))
        })();
        let _ = s.close(file);
        result
    })
    .unwrap_or(false)
}
/// CreateProcessW: with bInheritHandles and STARTF_USESTDHANDLES, pipe ends
/// among the standard handles become the child's (console otherwise).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn DoorCreateProcessExW(
    application: u64,
    command_line: u64,
    directory: u64,
    flags: u32,
    info: u64,
    environment: u64,
    inherit: i32,
    startup: u64,
) -> i32 {
    let standard = match standard_handles(inherit, startup) {
        Ok(s) => s,
        Err(e) => return failed(e),
    };
    if info == 0 || (application == 0 && command_line == 0) {
        return failed(87);
    }
    let suspended = flags & CREATE_SUSPENDED != 0;
    let mut line = [0u16; LINE_UNITS];
    let line_length = if command_line == 0 {
        0
    } else {
        match read_units(command_line, &mut line) {
            Ok(n) => n,
            Err(e) => return failed(e),
        }
    };
    let line = &line[..line_length];
    // The program.
    let mut native = [0u8; PATH_TEXT];
    let found = if application != 0 {
        let mut name = [0u16; 520];
        match read_units(application, &mut name) {
            Ok(n) => find(&name[..n], &mut native),
            Err(e) => Err(e),
        }
    } else {
        let mut candidates = [(0usize, 0usize); 16];
        let count = door_windows_abi::arguments::program_candidates(line, &mut candidates);
        let mut found = Ok(None);
        for &(skip, length) in &candidates[..count] {
            found = find(&line[skip..skip + length], &mut native);
            if !matches!(found, Ok(None)) {
                break;
            }
        }
        found
    };
    let native_length = match found {
        Ok(Some(n)) => n,
        Ok(None) => return failed(2),
        Err(e) => return failed(e),
    };
    let Ok(program) = core::str::from_utf8(&native[..native_length]) else {
        return failed(123);
    };
    // A program whose manifest asks for administrator rights needs them:
    // ERROR_ELEVATION_REQUIRED (ShellExecute asks User Account Control).
    if super::guest::current_groups() & door_abi::platform::group::ADMINISTRATORS == 0
        && requires_administrator(program)
    {
        return failed(740);
    }
    // The arguments: the program's Windows path, then the command line's
    // arguments after its program name.
    let mut parts = [0u16; LINE_UNITS + 64];
    let mut starts = [0usize; 64];
    let Some(argc) = door_windows_abi::arguments::split(line, &mut parts, &mut starts) else {
        return failed(206);
    };
    let mut arguments = [0u8; door_service_protocol::application::LONG_ARGUMENT_BYTES];
    let mut at = 0;
    let mut put = |bytes: &[u8]| -> Result<(), u32> {
        arguments
            .get_mut(at..at + bytes.len())
            .ok_or(206u32)?
            .copy_from_slice(bytes);
        at += bytes.len();
        Ok(())
    };
    let mut path = [0u8; PATH_TEXT + 2];
    let n = windows_path(program, &mut path);
    let mut result = put(&path[..n]).and_then(|()| put(&[0]));
    for &from in &starts[1..argc] {
        let end = parts[from..]
            .iter()
            .position(|&u| u == 0)
            .map_or(parts.len(), |i| from + i);
        for character in char::decode_utf16(parts[from..end].iter().copied()) {
            let mut bytes = [0u8; 4];
            let encoded = character.unwrap_or('\u{fffd}').encode_utf8(&mut bytes);
            result = result.and_then(|()| put(encoded.as_bytes()));
        }
        result = result.and_then(|()| put(&[0]));
    }
    if let Err(e) = result {
        return failed(e);
    }
    // The working directory.
    let mut cwd = [0u8; PATH_TEXT];
    let cwd_length = if directory == 0 {
        super::files::current_native(&mut cwd)
    } else {
        let mut units = [0u16; 520];
        read_units(directory, &mut units)
            .and_then(|n| super::files::existing_directory(&units[..n], &mut cwd)?.ok_or(267))
    };
    let cwd_length = match cwd_length {
        Ok(n) => n,
        Err(e) => return failed(e),
    };
    let Ok(cwd) = core::str::from_utf8(&cwd[..cwd_length]) else {
        return failed(123);
    };
    let registry = super::guest::DoorGetInheritedHandle(inherited::REGISTRY as u32);
    let authority = super::guest::DoorGetInheritedHandle(inherited::DIRECTORY as u32);
    // lpEnvironment replaces the session's variables in the child.
    let mut block = [0u8; door_service_protocol::application::LONG_ARGUMENT_BYTES];
    let block_length = if environment == 0 {
        None
    } else {
        match environment_block(
            environment,
            flags & CREATE_UNICODE_ENVIRONMENT != 0,
            &mut block[..door_service_protocol::application::LONG_ARGUMENT_BYTES - at],
        ) {
            Ok(n) => Some(n),
            Err(e) => return failed(e),
        }
    };
    let detached = flags & (DETACHED_PROCESS | CREATE_NO_WINDOW) != 0;
    let sections = super::pipes::for_child(standard);
    let piped = sections != [0; 3];
    let launched = door_service_client::Applications::connect(registry).and_then(|mut apps| {
        if piped {
            apps.execute_piped(
                detached,
                program,
                cwd,
                &arguments[..at],
                authority,
                block_length.map(|n| &block[..n]),
                suspended,
                sections,
            )
        } else if block_length.is_none() && !suspended {
            if detached {
                apps.execute_detached(program, cwd, &arguments[..at], authority)
            } else {
                apps.execute(program, cwd, &arguments[..at], authority)
            }
        } else {
            apps.execute_with(
                detached,
                program,
                cwd,
                &arguments[..at],
                authority,
                block_length.map(|n| &block[..n]),
                suspended,
            )
        }
    });
    if launched.is_err() {
        super::pipes::undo(standard, sections);
    }
    for section in sections.into_iter().filter(|s| *s != 0) {
        let _ = door_sys::close(section);
    }
    let process = match launched {
        Ok(handle) => handle,
        Err(e) => return failed(execute_error(e)),
    };
    // hThread: the process handle again (waiting on it waits for the
    // program, whose first thread is its life; ResumeThread on it starts a
    // CREATE_SUSPENDED child).
    let thread = door_sys::duplicate(process, rights::QUERY | rights::WAIT).unwrap_or(0);
    let id = door_sys::inspect(process).map_or(0, |i| i[2] as u32);
    // PROCESS_INFORMATION at the program's width (24 bytes x64, 16 x86).
    use super::shapes::{PROCESS_INFORMATION, set, size};
    let mut record = [0u8; size(&PROCESS_INFORMATION)];
    for (index, value) in [process, thread, u64::from(id), u64::from(id)]
        .into_iter()
        .enumerate()
    {
        set(&mut record, &PROCESS_INFORMATION, index, value);
    }
    if copy(info, record.as_ptr() as u64, record.len()).is_err() {
        let _ = door_sys::close(process);
        let _ = door_sys::close(thread);
        return failed(998);
    }
    RtlSetLastWin32Error(0);
    1
}
fn is_process(handle: u64) -> bool {
    door_sys::inspect(handle)
        .is_ok_and(|info| HandleKind::from_raw(info[0]) == Some(HandleKind::Process))
}
/// `GetExitCodeProcess`: STILL_ACTIVE while it runs.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetExitCodeProcess(handle: u64, out: u64) -> i32 {
    let code = if handle == u64::MAX {
        STILL_ACTIVE
    } else {
        if !is_process(handle) {
            return failed(6);
        }
        match door_sys::process_wait(handle, 0) {
            Ok(code) => code as u32,
            Err(door_abi::Status::Timeout | door_abi::Status::WouldBlock) => STILL_ACTIVE,
            Err(_) => return failed(5),
        }
    };
    if copy(out, code.to_le_bytes().as_ptr() as u64, 4).is_err() {
        return failed(998);
    }
    1
}
/// `GetProcessId`.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetProcessId(handle: u64) -> u32 {
    if handle == u64::MAX {
        return super::guest::DoorGetCurrentProcessId();
    }
    if !is_process(handle) {
        failed(6);
        return 0;
    }
    door_sys::inspect(handle).map_or(0, |i| i[2] as u32)
}
/// The machine of another process (IsWow64Process, P17): 0x14C for a
/// 32-bit Windows program, 0x8664 otherwise; 0 with ERROR_INVALID_HANDLE
/// for a handle that names no process.
#[unsafe(no_mangle)]
pub extern "system" fn DoorProcessMachine(handle: u64) -> u32 {
    if !is_process(handle) {
        failed(6);
        return 0;
    }
    door_sys::process_machine(handle).map_or_else(
        |_| {
            failed(6);
            0
        },
        u32::from,
    )
}
/// `TerminateProcess` of another program (the handle must allow it).
#[unsafe(no_mangle)]
pub extern "system" fn DoorTerminateProcess(handle: u64, code: u32) -> i32 {
    if !is_process(handle) {
        return failed(6);
    }
    match door_sys::process_terminate(handle, i64::from(code as i32)) {
        Ok(()) => 1,
        Err(_) => failed(5),
    }
}

//! shell32.dll exports. Paths are Windows paths throughout; ntdll maps them
//! to the native volume. Known folders come from the environment the
//! session gives every program (USERPROFILE, APPDATA, LOCALAPPDATA) and
//! the fixed system layout. ShellExecute runs programs through
//! CreateProcess (ntdll's search and Init), opens documents through the
//! registered association (HKCR\.ext -> ProgID -> shell\verb\command) and
//! sends "runas" to User Account Control; a document with no association
//! (and "openas") goes to File Explorer, which opens it with its default
//! app here or asks which app to use, as OpenWith.exe does. File and stock
//! icons come from the shell icon table and the system image list; the
//! notification area takes icons into the taskbar's hidden-icons flyout;
//! SHBrowseForFolder is a dialog over the folders, as Windows' classic one.
use crate::*;
mod items;
use alloc::{string::String, vec, vec::Vec};

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorUserCopy(destination: u64, source: u64, bytes: usize) -> u32;
    fn DoorLoadLibraryW(name: u64) -> u64;
    fn DoorGetInheritedHandle(index: u32) -> u64;
    fn DoorGetProcAddress(module: u64, name: u64) -> u64;
    fn DoorGetEnvironmentVariableW(name: u64, buffer: u64, size: u32) -> u32;
    fn DoorGetFileAttributesW(path: u64) -> u32;
    fn DoorCreateDirectoryW(path: u64) -> i32;
    fn DoorRemoveDirectoryW(path: u64) -> i32;
    fn DoorDeleteFileW(path: u64) -> i32;
    fn DoorMoveFileExW(existing: u64, new: u64, flags: u32) -> i32;
    fn DoorCopyFileW(existing: u64, new: u64, fail_if_exists: i32) -> i32;
    fn DoorFindFirstFileW(pattern: u64, destination: u64) -> u64;
    fn DoorFindNextFileW(handle: u64, destination: u64) -> i32;
    fn DoorFindClose(handle: u64) -> i32;
    fn DoorGetCurrentDirectoryW(capacity: u32, buffer: u64) -> u32;
    fn DoorGetNativePath(path: u64, out: u64, capacity: u32) -> u32;
    fn DoorCreateFileW(
        path: u64,
        access: u32,
        share: u32,
        security: u64,
        disposition: u32,
        flags: u32,
        template: u64,
    ) -> u64;
    fn DoorReadFile(handle: u64, buffer: u64, bytes: u32, read: u64, overlapped: u64) -> i32;
    fn DoorGetFileSizeEx(handle: u64, destination: u64) -> i32;
    fn DoorCloseFile(handle: u64) -> i32;
    fn DoorCloseHandle(handle: u64) -> i32;
    fn DoorCreateProcessW(
        application: u64,
        command_line: u64,
        directory: u64,
        flags: u32,
        info: u64,
        environment: u64,
    ) -> i32;
    fn DoorGetProcessHeap() -> u64;
    fn RtlAllocateHeap(heap: usize, flags: u32, bytes: usize) -> usize;
    fn RtlFreeHeap(heap: usize, flags: u32, pointer: usize) -> u8;
    fn RtlSizeHeap(heap: usize, flags: u32, pointer: usize) -> usize;
    fn RtlSetLastWin32Error(code: u32);
    fn RtlGetLastWin32Error() -> u32;
}
#[link(name = "advapi32", kind = "dylib")]
unsafe extern "system" {
    fn RegOpenKeyExW(key: usize, sub: usize, options: u32, desired: u32, out: usize) -> u32;
    fn RegQueryValueExW(
        key: usize,
        name: usize,
        reserved: usize,
        kind: usize,
        data: usize,
        size: usize,
    ) -> u32;
    fn RegCloseKey(key: usize) -> u32;
    fn CheckTokenMembership(token: usize, sid: usize, out: usize) -> i32;
}
#[link(name = "user32", kind = "dylib")]
unsafe extern "system" {
    fn MessageBoxW(window: usize, text: usize, caption: usize, kind: u32) -> i32;
    fn DoorRequestElevation(
        path: u64,
        path_len: u32,
        image: u64,
        image_len: u32,
        flags: u32,
    ) -> u32;
    fn DoorOpenWith(path: u64, path_len: u32, choose: u32) -> u32;
    fn DoorElevationResult(process: u64) -> u32;
    fn DoorNotifyIcon(
        op: u32,
        hwnd: u64,
        id: u32,
        flags: u32,
        callback: u32,
        icon: u64,
        tip: u64,
        tip_len: u32,
        version: u32,
    ) -> i32;
    fn PrivateExtractIconsW(
        file: usize,
        index: i32,
        cx: i32,
        cy: i32,
        icons: usize,
        ids: usize,
        count: u32,
        flags: u32,
    ) -> u32;
    fn DestroyIcon(icon: usize) -> i32;
    fn DoorShellIcon(stem: u64, stem_len: u32, size: u32) -> u64;
    fn GetWindowLongPtrW(window: usize, index: i32) -> isize;
    fn DialogBoxIndirectParamW(
        instance: usize,
        template: usize,
        owner: usize,
        procedure: usize,
        param: isize,
    ) -> isize;
    fn SendMessageW(hwnd: usize, message: u32, wparam: usize, lparam: isize) -> isize;
    fn GetDlgItem(dialog: usize, id: i32) -> usize;
    fn SetDlgItemTextW(dialog: usize, id: i32, text: usize) -> i32;
    fn GetDlgItemTextW(dialog: usize, id: i32, text: usize, capacity: i32) -> u32;
    fn EndDialog(dialog: usize, result: isize) -> i32;
    fn EnableWindow(hwnd: usize, enable: i32) -> i32;
    fn SetWindowLongPtrW(window: usize, index: i32, value: isize) -> isize;
}

mod hr {
    pub const S_OK: u32 = 0;
    pub const S_FALSE: u32 = 1;
    pub const E_FAIL: u32 = 0x8000_4005;
    pub const E_INVALIDARG: u32 = 0x8007_0057;
    pub const E_OUTOFMEMORY: u32 = 0x8007_000E;
    pub const NOT_FOUND: u32 = 0x8007_0002;
}
const INVALID_ATTRIBUTES: u32 = 0xFFFF_FFFF;
const DIRECTORY: u32 = 0x10;

// ---- memory and strings ---------------------------------------------------------------
const CHUNK: usize = door_windows_abi::APPLICATION_COPY_BYTES;
fn read(pointer: u64, out: &mut [u8]) -> Result<(), u32> {
    if pointer == 0 {
        return Err(87);
    }
    for (i, c) in out.chunks_mut(CHUNK).enumerate() {
        if unsafe { DoorUserCopy(c.as_mut_ptr() as u64, pointer + (i * CHUNK) as u64, c.len()) }
            != 0
        {
            return Err(998);
        }
    }
    Ok(())
}
fn write(pointer: u64, data: &[u8]) -> Result<(), u32> {
    if pointer == 0 {
        return Err(87);
    }
    for (i, c) in data.chunks(CHUNK).enumerate() {
        if unsafe { DoorUserCopy(pointer + (i * CHUNK) as u64, c.as_ptr() as u64, c.len()) } != 0 {
            return Err(998);
        }
    }
    Ok(())
}
fn read_u32(pointer: u64) -> Result<u32, u32> {
    let mut b = [0u8; 4];
    read(pointer, &mut b)?;
    Ok(u32::from_le_bytes(b))
}
/// The structures shell32 reads and writes, at the build's width and, for
/// shellapi.h's, its packing (P17).
mod shape {
    pub use door_windows_abi::layout::{self, Field, Field::*, SHELLAPI_PACK};
    /// SHELLEXECUTEINFOW: cbSize, fMask, hwnd, lpVerb, lpFile,
    /// lpParameters, lpDirectory, nShow, hInstApp, lpIDList, lpClass,
    /// hkeyClass, dwHotKey, hIcon/hMonitor, hProcess (112 bytes x64, 60 x86).
    pub const SHELLEXECUTEINFO: [Field; 15] = [
        U32, U32, Pointer, Pointer, Pointer, Pointer, Pointer, U32, Pointer, Pointer, Pointer,
        Pointer, U32, Pointer, Pointer,
    ];
    /// SHFILEOPSTRUCTW: hwnd, wFunc, pFrom, pTo, fFlags,
    /// fAnyOperationsAborted, hNameMappings, lpszProgressTitle (56 bytes
    /// x64, 30 x86 packed).
    pub const SHFILEOPSTRUCT: [Field; 8] =
        [Pointer, U32, Pointer, Pointer, U16, U32, Pointer, Pointer];
    /// NOTIFYICONDATA's head: cbSize, hWnd, uID, uFlags, uCallbackMessage,
    /// hIcon; szTip follows at the head's packed end.
    pub const NOTIFY_HEAD: [Field; 7] = [U32, Pointer, U32, U32, U32, Pointer, U16];
    /// BROWSEINFOW: hwndOwner, pidlRoot, pszDisplayName, lpszTitle,
    /// ulFlags, lpfn, lParam, iImage (64 bytes x64, 32 x86).
    pub const BROWSEINFO: [Field; 8] = [
        Pointer, Pointer, Pointer, Pointer, U32, Pointer, Pointer, U32,
    ];
    /// PROCESS_INFORMATION: hProcess, hThread, dwProcessId, dwThreadId.
    pub const PROCESS_INFORMATION: [Field; 4] = [Pointer, Pointer, U32, U32];
    pub const fn at(fields: &[Field], index: usize) -> u64 {
        layout::offset_pack(fields, index, SHELLAPI_PACK) as u64
    }
}
/// A pointer the program stored at `pointer` (P17: its width).
fn read_pointer(pointer: u64) -> Result<u64, u32> {
    let mut b = [0u8; 8];
    read(pointer, &mut b[..POINTER])?;
    Ok(if POINTER == 4 {
        i64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])) as u64
    } else {
        u64::from_le_bytes(b)
    })
}
/// A handle or pointer an out-parameter receives, at the program's width.
fn write_pointer(pointer: u64, value: u64) -> Result<(), u32> {
    write(pointer, &value.to_le_bytes()[..POINTER])
}
const POINTER: usize = door_windows_abi::layout::POINTER;
/// A NUL-terminated UTF-16 string (at most `max` units).
fn read_wide(pointer: u64, max: usize) -> Result<Vec<u16>, u32> {
    let mut out = Vec::new();
    loop {
        let mut b = [0u8; 2];
        read(pointer + 2 * out.len() as u64, &mut b)?;
        let unit = u16::from_le_bytes(b);
        if unit == 0 {
            return Ok(out);
        }
        if out.len() == max {
            return Err(206);
        }
        out.push(unit);
    }
}
fn read_text(pointer: u64) -> Result<String, u32> {
    Ok(String::from_utf16_lossy(&read_wide(pointer, 32767)?))
}
/// An ANSI (code page 1252 subset: Latin-1) string.
fn read_ansi(pointer: u64) -> Result<String, u32> {
    let mut out = String::new();
    loop {
        let mut b = [0u8];
        read(pointer + out.chars().count() as u64, &mut b)?;
        if b[0] == 0 {
            return Ok(out);
        }
        out.push(char::from(b[0]));
    }
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
fn wide_bytes(text: &str) -> Vec<u8> {
    wide(text).iter().flat_map(|u| u.to_le_bytes()).collect()
}
fn ansi_bytes(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| if (c as u32) < 256 { c as u8 } else { b'?' })
        .chain([0])
        .collect()
}
fn set_error(code: u32) {
    unsafe { RtlSetLastWin32Error(code) }
}
fn heap() -> u64 {
    unsafe { DoorGetProcessHeap() }
}
/// Task memory (CoTaskMemAlloc's heap).
fn task_alloc(bytes: usize) -> u64 {
    unsafe {
        door_windows_abi::widen(RtlAllocateHeap(
            door_windows_abi::narrow(heap()),
            0,
            bytes.max(1),
        ))
    }
}
fn task_copy(data: &[u8]) -> Result<u64, u32> {
    let block = task_alloc(data.len());
    if block == 0 {
        return Err(8);
    }
    // SAFETY: a fresh block of data.len() bytes.
    unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), block as *mut u8, data.len()) };
    Ok(block)
}
#[unsafe(no_mangle)]
pub extern "system" fn SHAlloc(bytes: usize) -> usize {
    door_windows_abi::narrow((move || -> u64 { task_alloc(bytes) })())
}
#[unsafe(no_mangle)]
pub extern "system" fn SHFree(block: usize) {
    let block = door_windows_abi::widen(block);
    {
        if block != 0 {
            unsafe {
                RtlFreeHeap(
                    door_windows_abi::narrow(heap()),
                    0,
                    door_windows_abi::narrow(block),
                )
            };
        }
    }
}

// ---- files -------------------------------------------------------------------------
fn attributes(path: &str) -> Option<u32> {
    let w = wide(path);
    let a = unsafe { DoorGetFileAttributesW(w.as_ptr() as u64) };
    (a != INVALID_ATTRIBUTES).then_some(a)
}
fn is_directory(path: &str) -> bool {
    attributes(path).is_some_and(|a| a & DIRECTORY != 0)
}
fn current_directory() -> String {
    let mut buffer = vec![0u16; 1024];
    let n = unsafe { DoorGetCurrentDirectoryW(buffer.len() as u32, buffer.as_mut_ptr() as u64) }
        as usize;
    String::from_utf16_lossy(&buffer[..n.min(buffer.len())])
}
/// `path` made absolute against `directory` (or the current directory).
fn absolute(path: &str, directory: Option<&str>) -> String {
    let p = path.trim_matches('"');
    // A drive path, or one of Doors' own folders (`/system/bin/...`).
    if (p.len() >= 2 && p.as_bytes()[1] == b':') || p.starts_with('/') {
        return String::from(p);
    }
    let base = directory.map_or_else(current_directory, String::from);
    if p.starts_with('\\') {
        return alloc::format!("{}{}", &base[..2.min(base.len())], p);
    }
    let base = base.trim_end_matches('\\');
    alloc::format!("{base}\\{p}")
}
/// Creates `path` and every missing parent.
fn create_tree(path: &str) -> u32 {
    if is_directory(path) {
        return 183;
    }
    if attributes(path).is_some() {
        return 183;
    }
    let mut at = 0;
    let parts: Vec<&str> = path.split('\\').collect();
    let mut sofar = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            sofar.push('\\');
        }
        sofar.push_str(part);
        if i == 0 || part.is_empty() || is_directory(&sofar) {
            continue;
        }
        let w = wide(&sofar);
        if unsafe { DoorCreateDirectoryW(w.as_ptr() as u64) } == 0 {
            return unsafe { RtlGetLastWin32Error() };
        }
        at = i;
    }
    let _ = at;
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn SHCreateDirectoryExW(_window: usize, path: usize, _security: usize) -> i32 {
    let path = door_windows_abi::widen(path);
    {
        match read_text(path) {
            Ok(p) if !p.is_empty() => {
                let p = absolute(&p, None);
                let r = create_tree(&p);
                set_error(r);
                r as i32
            }
            _ => 161,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SHCreateDirectoryExA(_window: usize, path: usize, _security: usize) -> i32 {
    let path = door_windows_abi::widen(path);
    {
        match read_ansi(path) {
            Ok(p) if !p.is_empty() => create_tree(&absolute(&p, None)) as i32,
            _ => 161,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SHCreateDirectory(window: usize, path: usize) -> i32 {
    let window = door_windows_abi::widen(window);
    let path = door_windows_abi::widen(path);
    {
        SHCreateDirectoryExW(
            door_windows_abi::narrow(window),
            door_windows_abi::narrow(path),
            0,
        )
    }
}

// ---- command lines ------------------------------------------------------------------
/// One LocalAlloc block: the pointer array, then the strings (LocalFree
/// releases both).
#[unsafe(no_mangle)]
pub extern "system" fn CommandLineToArgvW(line: usize, count: usize) -> usize {
    let line = door_windows_abi::widen(line);
    let count = door_windows_abi::widen(count);
    door_windows_abi::narrow((move || -> u64 {
        if count == 0 {
            set_error(87);
            return 0;
        }
        let units = if line == 0 {
            Vec::new()
        } else {
            match read_wide(line, 32767) {
                Ok(u) => u,
                Err(e) => {
                    set_error(e);
                    return 0;
                }
            }
        };
        let mut parts = vec![0u16; units.len() + 2];
        let mut starts = vec![0usize; units.len() / 2 + 2];
        let mut argc =
            door_windows_abi::arguments::split(&units, &mut parts, &mut starts).unwrap_or(0);
        if argc == 0 {
            // An empty line: the program's own path.
            let mut module = vec![0u16; 520];
            let n = unsafe { module_file_name(module.as_mut_ptr() as u64, module.len() as u32) }
                as usize;
            parts = module[..n].iter().copied().chain([0]).collect();
            starts[0] = 0;
            argc = 1;
        }
        let used = starts[argc - 1]
            + parts[starts[argc - 1]..]
                .iter()
                .position(|&u| u == 0)
                .unwrap_or(0)
            + 1;
        // argv: a pointer-sized entry per argument and a null (P17).
        const POINTER: usize = core::mem::size_of::<usize>();
        let table = (argc + 1) * POINTER;
        let block =
            unsafe { RtlAllocateHeap(door_windows_abi::narrow(heap()), 0, table + used * 2) };
        if block == 0 {
            set_error(8);
            return 0;
        }
        for (i, &start) in starts[..argc].iter().enumerate() {
            // SAFETY: the block holds the table and the strings.
            unsafe {
                *((block + door_windows_abi::narrow((POINTER * i) as u64)) as *mut usize) = (block
                    + door_windows_abi::narrow(table as u64)
                    + door_windows_abi::narrow(2 * start as u64))
                    as usize
            };
        }
        unsafe {
            *((block + door_windows_abi::narrow((POINTER * argc) as u64)) as *mut usize) = 0;
            core::ptr::copy_nonoverlapping(
                parts.as_ptr(),
                (block + door_windows_abi::narrow(table as u64)) as *mut u16,
                used,
            );
        }
        let _ = write(count, &(argc as i32).to_le_bytes());
        door_windows_abi::widen(block)
    })())
}
#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    #[link_name = "DoorGetModuleFileNameW"]
    fn module_file_name_raw(module: u64, buffer: u64, capacity: u32) -> u32;
}
unsafe fn module_file_name(buffer: u64, capacity: u32) -> u32 {
    unsafe { module_file_name_raw(0, buffer, capacity) }
}

// ---- known folders --------------------------------------------------------------------
fn variable(name: &str) -> Option<String> {
    let key = wide(name);
    let mut buffer = vec![0u16; 1024];
    let n = unsafe {
        DoorGetEnvironmentVariableW(
            key.as_ptr() as u64,
            buffer.as_mut_ptr() as u64,
            buffer.len() as u32,
        )
    } as usize;
    (n > 0 && n < buffer.len()).then(|| String::from_utf16_lossy(&buffer[..n]))
}
fn path_of(f: &Folder) -> Option<String> {
    let profile = variable("USERPROFILE")?;
    let roaming =
        variable("APPDATA").unwrap_or_else(|| alloc::format!(r"{profile}\AppData\Roaming"));
    let local =
        variable("LOCALAPPDATA").unwrap_or_else(|| alloc::format!(r"{profile}\AppData\Local"));
    Some(folder_path(f, &profile, &roaming, &local))
}
/// The folder's path, created when `create` asks; Err(HRESULT) otherwise.
fn known(f: &Folder, create: bool, verify: bool) -> Result<String, u32> {
    let path = path_of(f).ok_or(hr::E_FAIL)?;
    if create && !is_directory(&path) {
        let r = create_tree(&path);
        if r != 0 && r != 183 {
            return Err(0x8007_0000 | r);
        }
    }
    if verify && !is_directory(&path) {
        return Err(hr::NOT_FOUND);
    }
    Ok(path)
}
const KF_FLAG_CREATE: u32 = 0x8000;
const KF_FLAG_DONT_VERIFY: u32 = 0x4000;
#[unsafe(no_mangle)]
pub extern "system" fn SHGetKnownFolderPath(
    id: usize,
    flags: u32,
    _token: usize,
    out: usize,
) -> u32 {
    let id = door_windows_abi::widen(id);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_INVALIDARG;
        }
        let _ = write_pointer(out, 0);
        let mut g = [0u8; 16];
        if read(id, &mut g).is_err() {
            return hr::E_INVALIDARG;
        }
        // An unknown FOLDERID: HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND), as
        // Windows answers.
        let Some(f) = by_id(&g).and_then(|f| view(f, cfg!(target_arch = "x86"))) else {
            return hr::NOT_FOUND;
        };
        match known(
            f,
            flags & KF_FLAG_CREATE != 0,
            flags & KF_FLAG_DONT_VERIFY == 0,
        ) {
            Ok(path) => match task_copy(&wide_bytes(&path)) {
                Ok(block) => {
                    let _ = write_pointer(out, block);
                    hr::S_OK
                }
                Err(_) => hr::E_OUTOFMEMORY,
            },
            Err(e) => e,
        }
    }
}
/// SHGetFolderPathW: S_FALSE (W) or E_FAIL (A) for a folder that does not
/// exist without CSIDL_FLAG_CREATE, E_INVALIDARG for an unknown CSIDL.
fn folder_path_for(csidl: u32, wide_api: bool) -> Result<String, u32> {
    let f = by_csidl(csidl)
        .and_then(|f| view(f, cfg!(target_arch = "x86")))
        .ok_or(hr::E_INVALIDARG)?;
    let create = csidl & CSIDL_FLAG_CREATE != 0;
    let path = known(f, create, false)?;
    if !is_directory(&path) {
        return Err(if wide_api { hr::S_FALSE } else { hr::E_FAIL });
    }
    Ok(path)
}
#[unsafe(no_mangle)]
pub extern "system" fn SHGetFolderPathW(
    _window: usize,
    csidl: i32,
    _token: usize,
    _flags: u32,
    out: usize,
) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        match folder_path_for(csidl as u32, true) {
            Ok(path) => match write(out, &wide_bytes(&path)) {
                Ok(()) => hr::S_OK,
                Err(_) => hr::E_INVALIDARG,
            },
            // The buffer is left as it was, as Windows leaves it.
            Err(e) => e,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SHGetFolderPathA(
    _window: usize,
    csidl: i32,
    _token: usize,
    _flags: u32,
    out: usize,
) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        match folder_path_for(csidl as u32, false) {
            Ok(path) => match write(out, &ansi_bytes(&path)) {
                Ok(()) => hr::S_OK,
                Err(_) => hr::E_INVALIDARG,
            },
            Err(e) => e,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SHGetSpecialFolderPathW(
    _window: usize,
    out: usize,
    csidl: i32,
    create: i32,
) -> i32 {
    let out = door_windows_abi::widen(out);
    {
        let csidl = csidl as u32 | if create != 0 { CSIDL_FLAG_CREATE } else { 0 };
        match folder_path_for(csidl, true) {
            Ok(path) => i32::from(write(out, &wide_bytes(&path)).is_ok()),
            Err(_) => 0,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SHGetSpecialFolderPathA(
    _window: usize,
    out: usize,
    csidl: i32,
    create: i32,
) -> i32 {
    let out = door_windows_abi::widen(out);
    {
        let csidl = csidl as u32 | if create != 0 { CSIDL_FLAG_CREATE } else { 0 };
        match folder_path_for(csidl, false) {
            Ok(path) => i32::from(write(out, &ansi_bytes(&path)).is_ok()),
            Err(_) => 0,
        }
    }
}

// ---- item ID lists -------------------------------------------------------------------
fn list_out(list: &[u8], out: u64) -> u32 {
    match task_copy(list) {
        Ok(block) => {
            if write_pointer(out, block).is_err() {
                SHFree(door_windows_abi::narrow(block));
                return hr::E_INVALIDARG;
            }
            hr::S_OK
        }
        Err(_) => hr::E_OUTOFMEMORY,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SHGetKnownFolderIDList(
    id: usize,
    flags: u32,
    _token: usize,
    out: usize,
) -> u32 {
    let id = door_windows_abi::widen(id);
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_INVALIDARG;
        }
        let _ = write_pointer(out, 0);
        let mut g = [0u8; 16];
        if read(id, &mut g).is_err() {
            return hr::E_INVALIDARG;
        }
        // An unknown FOLDERID: HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND), as
        // Windows answers.
        let Some(f) = by_id(&g).and_then(|f| view(f, cfg!(target_arch = "x86"))) else {
            return hr::NOT_FOUND;
        };
        match known(
            f,
            flags & KF_FLAG_CREATE != 0,
            flags & KF_FLAG_DONT_VERIFY == 0,
        ) {
            Ok(path) => list_out(&pidl::from_path(&path, true).unwrap_or(vec![0, 0]), out),
            Err(e) => e,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SHGetFolderLocation(
    _window: usize,
    csidl: i32,
    _token: usize,
    _reserved: u32,
    out: usize,
) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        if out == 0 {
            return hr::E_INVALIDARG;
        }
        let _ = write_pointer(out, 0);
        let csidl = csidl as u32;
        // The desktop is the root: the empty list.
        if csidl & !CSIDL_FLAG_MASK == CSIDL_DESKTOP {
            return list_out(&[0, 0], out);
        }
        match folder_path_for(csidl, true) {
            Ok(path) => list_out(&pidl::from_path(&path, true).unwrap_or(vec![0, 0]), out),
            Err(hr::S_FALSE) => hr::NOT_FOUND,
            Err(e) => e,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SHGetSpecialFolderLocation(window: usize, csidl: i32, out: usize) -> u32 {
    let window = door_windows_abi::widen(window);
    let out = door_windows_abi::widen(out);
    {
        SHGetFolderLocation(
            door_windows_abi::narrow(window),
            csidl,
            0,
            0,
            door_windows_abi::narrow(out),
        )
    }
}
/// The bytes of an item ID list in application memory.
fn read_list(list: u64) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let mut cb = [0u8; 2];
        read(list + out.len() as u64, &mut cb).ok()?;
        let n = usize::from(u16::from_le_bytes(cb));
        if n == 0 {
            out.extend_from_slice(&cb);
            return Some(out);
        }
        if n < 3 || out.len() + n > 64 * 1024 {
            return None;
        }
        let mut item = vec![0u8; n];
        read(list + out.len() as u64, &mut item).ok()?;
        out.extend_from_slice(&item);
    }
}
fn list_path(list: u64) -> Option<String> {
    let bytes = read_list(list)?;
    if bytes == [0, 0] {
        // The desktop's file-system path.
        return path_of(by_csidl(CSIDL_DESKTOP)?);
    }
    pidl::to_path(&bytes)
}
#[unsafe(no_mangle)]
pub extern "system" fn SHGetPathFromIDListW(list: usize, out: usize) -> i32 {
    let list = door_windows_abi::widen(list);
    let out = door_windows_abi::widen(out);
    {
        match list_path(list) {
            Some(path) if path.encode_utf16().count() < 260 => {
                i32::from(write(out, &wide_bytes(&path)).is_ok())
            }
            _ => {
                let _ = write(out, &[0, 0]);
                0
            }
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SHGetPathFromIDListEx(
    list: usize,
    out: usize,
    capacity: u32,
    _flags: u32,
) -> i32 {
    let list = door_windows_abi::widen(list);
    let out = door_windows_abi::widen(out);
    {
        match list_path(list) {
            Some(path) if path.encode_utf16().count() < capacity as usize => {
                i32::from(write(out, &wide_bytes(&path)).is_ok())
            }
            _ => 0,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SHGetPathFromIDListA(list: usize, out: usize) -> i32 {
    let list = door_windows_abi::widen(list);
    let out = door_windows_abi::widen(out);
    {
        match list_path(list) {
            Some(path) if path.len() < 260 => i32::from(write(out, &ansi_bytes(&path)).is_ok()),
            _ => 0,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ILFree(list: usize) {
    let list = door_windows_abi::widen(list);
    SHFree(door_windows_abi::narrow(list))
}
#[unsafe(no_mangle)]
pub extern "system" fn ILGetSize(list: usize) -> u32 {
    let list = door_windows_abi::widen(list);
    {
        if list == 0 {
            return 0;
        }
        read_list(list).map_or(0, |l| l.len() as u32)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ILClone(list: usize) -> usize {
    let list = door_windows_abi::widen(list);
    door_windows_abi::narrow((move || -> u64 {
        read_list(list)
            .and_then(|l| task_copy(&l).ok())
            .unwrap_or(0)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn ILCombine(first: usize, second: usize) -> usize {
    let first = door_windows_abi::widen(first);
    let second = door_windows_abi::widen(second);
    door_windows_abi::narrow((move || -> u64 {
        let (Some(mut a), Some(b)) = (
            if first == 0 {
                Some(vec![0, 0])
            } else {
                read_list(first)
            },
            if second == 0 {
                Some(vec![0, 0])
            } else {
                read_list(second)
            },
        ) else {
            return 0;
        };
        a.truncate(a.len() - 2);
        a.extend_from_slice(&b);
        task_copy(&a).unwrap_or(0)
    })())
}
/// The last item (its address inside `list`); the terminator for the
/// empty list.
#[unsafe(no_mangle)]
pub extern "system" fn ILFindLastID(list: usize) -> usize {
    let list = door_windows_abi::widen(list);
    door_windows_abi::narrow((move || -> u64 {
        let Some(bytes) = read_list(list) else {
            return 0;
        };
        let mut at = 0;
        let mut last = 0;
        loop {
            let cb = usize::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]));
            if cb == 0 {
                return list + last as u64;
            }
            last = at;
            at += cb;
        }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn ILRemoveLastID(list: usize) -> i32 {
    let list = door_windows_abi::widen(list);
    {
        let last = ILFindLastID(door_windows_abi::narrow(list));
        if last == 0 || read(door_windows_abi::widen(last), &mut [0u8; 2]).is_err() {
            return 0;
        }
        let mut cb = [0u8; 2];
        let _ = read(door_windows_abi::widen(last), &mut cb);
        if cb == [0, 0] {
            return 0;
        }
        i32::from(write(door_windows_abi::widen(last), &[0, 0]).is_ok())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ILCreateFromPathW(path: usize) -> usize {
    let path = door_windows_abi::widen(path);
    door_windows_abi::narrow((move || -> u64 {
        let Ok(path) = read_text(path) else { return 0 };
        let path = absolute(&path, None);
        let Some(a) = attributes(&path) else {
            set_error(2);
            return 0;
        };
        pidl::from_path(&path, a & DIRECTORY != 0)
            .and_then(|l| task_copy(&l).ok())
            .unwrap_or(0)
    })())
}

// ---- the registry ------------------------------------------------------------------------
const HKEY_CLASSES_ROOT: u64 = 0x8000_0000;
/// The default (or `name`) string value of HKCR\`path`.
fn class_value(path: &str, name: Option<&str>) -> Option<String> {
    let path = wide(path);
    let mut key = [0u8; 8]; // an HKEY: its low POINTER bytes
    if unsafe {
        RegOpenKeyExW(
            door_windows_abi::narrow(HKEY_CLASSES_ROOT),
            door_windows_abi::narrow(path.as_ptr() as u64),
            0,
            0x2_0019,
            door_windows_abi::narrow(key.as_mut_ptr() as u64),
        )
    } != 0
    {
        return None;
    }
    let key = if POINTER == 4 {
        i64::from(i32::from_le_bytes([key[0], key[1], key[2], key[3]])) as u64
    } else {
        u64::from_le_bytes(key)
    };
    let name = name.map(wide);
    let mut data = vec![0u16; 520];
    let mut size = (data.len() * 2) as u32;
    let mut kind = 0u32;
    let r = unsafe {
        RegQueryValueExW(
            door_windows_abi::narrow(key),
            name.as_ref()
                .map_or(0, |n| door_windows_abi::narrow(n.as_ptr() as u64)),
            0,
            door_windows_abi::narrow(&mut kind as *mut u32 as u64),
            door_windows_abi::narrow(data.as_mut_ptr() as u64),
            door_windows_abi::narrow(&mut size as *mut u32 as u64),
        )
    };
    unsafe { RegCloseKey(door_windows_abi::narrow(key)) };
    if r != 0 || !matches!(kind, 1 | 2) {
        return None;
    }
    data.truncate(size as usize / 2);
    while data.last() == Some(&0) {
        data.pop();
    }
    Some(String::from_utf16_lossy(&data))
}
fn extension(path: &str) -> Option<&str> {
    let name = path.rsplit('\\').next()?;
    name.rfind('.').filter(|&i| i > 0).map(|i| &name[i..])
}
/// The registered type of a file: its ProgID (HKCR\.ext).
fn prog_id(path: &str) -> Option<String> {
    class_value(extension(path)?, None).filter(|p| !p.is_empty())
}

// ---- ShellExecute ---------------------------------------------------------------------
const SE_ERR_FNF: u32 = 2;
const SE_ERR_PNF: u32 = 3;
const SE_ERR_ACCESSDENIED: u32 = 5;
const SE_ERR_OOM: u32 = 8;
const SE_ERR_NOASSOC: u32 = 31;
const ERROR_NO_ASSOCIATION: u32 = 1155;
const ERROR_CANCELLED: u32 = 1223;
const SEE_MASK_NOCLOSEPROCESS: u32 = 0x40;
const SEE_MASK_FLAG_NO_UI: u32 = 0x400;
/// One request: what to run and how.
struct Execute {
    verb: String,
    file: String,
    parameters: String,
    directory: Option<String>,
    keep_process: bool,
    /// SEE_MASK_FLAG_NO_UI: fail instead of asking which app to use.
    no_ui: bool,
}
fn is_program(path: &str) -> bool {
    extension(path)
        .is_some_and(|e| e.eq_ignore_ascii_case(".exe") || e.eq_ignore_ascii_case(".com"))
}
fn quoted(path: &str) -> String {
    alloc::format!("\"{path}\"")
}
/// A command template (`"C:\x\app.exe" "%1" %*`) with the file put in.
fn expand_command(template: &str, file: &str, parameters: &str) -> String {
    let mut out = String::new();
    let mut chars = template.chars().peekable();
    let mut used = false;
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.peek() {
                Some('1') | Some('L') | Some('l') => {
                    chars.next();
                    out.push_str(file);
                    used = true;
                    continue;
                }
                Some('*') => {
                    chars.next();
                    out.push_str(parameters);
                    continue;
                }
                Some('%') => {
                    chars.next();
                    out.push('%');
                    continue;
                }
                _ => (),
            }
        }
        out.push(c);
    }
    if !used {
        out.push(' ');
        out.push_str(&quoted(file));
    }
    out
}
/// Starts `command_line` in `directory`; the process handle.
fn start(command_line: &str, directory: Option<&str>) -> Result<u64, u32> {
    let line = wide(command_line);
    let dir = directory.map(wide);
    let mut info = [0u8; shape::layout::size(&shape::PROCESS_INFORMATION)];
    let ok = unsafe {
        DoorCreateProcessW(
            0,
            line.as_ptr() as u64,
            dir.as_ref().map_or(0, |d| d.as_ptr() as u64),
            0,
            info.as_mut_ptr() as u64,
            0,
        )
    };
    if ok == 0 {
        return Err(unsafe { RtlGetLastWin32Error() });
    }
    let process = shape::layout::get(&info, &shape::PROCESS_INFORMATION, 0);
    let thread = shape::layout::get(&info, &shape::PROCESS_INFORMATION, 1);
    if thread != 0 {
        unsafe { DoorCloseHandle(thread) };
    }
    Ok(process)
}
/// The program at `path` (a Windows path) with `parameters` (a command
/// line's arguments) through User Account Control: the new process's
/// handle once the user approved and it started; ERROR_CANCELLED when the
/// user answered No.
fn elevate(path: &str, parameters: &str) -> Result<u64, u32> {
    let w = wide(path);
    let mut native = vec![0u8; door_service_protocol::window::elevation::MAX_PATH];
    let n = unsafe {
        DoorGetNativePath(
            w.as_ptr() as u64,
            native.as_mut_ptr() as u64,
            native.len() as u32,
        )
    } as usize;
    if n == 0 {
        return Err(SE_ERR_FNF);
    }
    // The image the prompt shows and Init hashes.
    let file = unsafe { DoorCreateFileW(w.as_ptr() as u64, 0x8000_0000, 1, 0, 3, 0, 0) };
    if file == u64::MAX || file == 0 {
        return Err(SE_ERR_FNF);
    }
    let mut size = [0u8; 8];
    let mut image = Vec::new();
    let read_ok = unsafe { DoorGetFileSizeEx(file, size.as_mut_ptr() as u64) } != 0 && {
        // Checked before anything is allocated: the display service takes
        // no larger image.
        let size = usize::try_from(u64::from_le_bytes(size))
            .ok()
            .filter(|&s| s <= door_abi::platform::EXECUTABLE_MAX_BYTES);
        size.is_some_and(|size| {
            image = vec![0u8; size];
            let mut done = [0u8; 4];
            let read = unsafe {
                DoorReadFile(
                    file,
                    image.as_mut_ptr() as u64,
                    size as u32,
                    done.as_mut_ptr() as u64,
                    0,
                )
            };
            read != 0 && u32::from_le_bytes(done) as usize == size
        })
    };
    unsafe { DoorCloseFile(file) };
    if !read_ok {
        return Err(SE_ERR_ACCESSDENIED);
    }
    // The arguments, split as CommandLineToArgvW splits them, after the
    // program's path, each NUL-terminated (`window::elevation::ARGUMENTS`).
    let mut request = native[..n].to_vec();
    let mut flags = 0;
    let units: Vec<u16> = parameters.encode_utf16().collect();
    if !units.is_empty() {
        let mut parts = vec![0u16; units.len() + 2];
        let mut starts = vec![0usize; units.len() / 2 + 2];
        let count =
            door_windows_abi::arguments::split(&units, &mut parts, &mut starts).unwrap_or(0);
        if count > 0 {
            request.push(0);
            flags = door_service_protocol::window::elevation::ARGUMENTS;
        }
        for &start in &starts[..count] {
            let end = parts[start..]
                .iter()
                .position(|&u| u == 0)
                .map_or(parts.len(), |e| start + e);
            request.extend_from_slice(String::from_utf16_lossy(&parts[start..end]).as_bytes());
            request.push(0);
        }
    }
    if request.len() > door_service_protocol::window::elevation::MAX_PATH {
        return Err(SE_ERR_ACCESSDENIED);
    }
    match unsafe {
        DoorRequestElevation(
            request.as_ptr() as u64,
            request.len() as u32,
            image.as_ptr() as u64,
            image.len() as u32,
            flags,
        )
    } {
        0 => (),
        ERROR_CANCELLED => return Err(ERROR_CANCELLED),
        _ => return Err(SE_ERR_ACCESSDENIED),
    }
    let mut process = [0u8; 8];
    match unsafe { DoorElevationResult(process.as_mut_ptr() as u64) } {
        0 => Ok(u64::from_le_bytes(process)),
        ERROR_CANCELLED => Err(ERROR_CANCELLED),
        _ => Err(SE_ERR_ACCESSDENIED),
    }
}
/// Runs a request; the process handle when there is one to give.
fn execute(request: &Execute) -> Result<Option<u64>, u32> {
    let verb = request.verb.to_ascii_lowercase();
    let directory = request.directory.as_deref();
    let file = absolute(&request.file, directory);
    let exists = attributes(&file);
    if exists.is_none() {
        // A program found on the search path ("notepad.exe").
        if is_program(&request.file) || extension(&request.file).is_none() {
            let line = alloc::format!("{} {}", quoted(&request.file), request.parameters);
            return start(line.trim_end(), directory)
                .map(Some)
                .map_err(|e| if e == 2 { SE_ERR_FNF } else { e });
        }
        return Err(
            if file.rfind('\\').is_some_and(|i| !is_directory(&file[..i])) {
                SE_ERR_PNF
            } else {
                SE_ERR_FNF
            },
        );
    }
    if is_program(&file) {
        return match verb.as_str() {
            "runas" => elevate(&file, &request.parameters).map(Some),
            "" | "open" => {
                let line = alloc::format!("{} {}", quoted(&file), request.parameters);
                match start(line.trim_end(), directory) {
                    // A program that asks for administrator rights: User
                    // Account Control, as Windows' shell does.
                    Err(740) => elevate(&file, &request.parameters).map(Some),
                    other => other.map(Some),
                }
            }
            _ => Err(SE_ERR_NOASSOC),
        };
    }
    // "openas": always ask which app to use (OpenWith.exe).
    if verb == "openas" {
        return if request.no_ui {
            Err(SE_ERR_NOASSOC)
        } else {
            open_with(&file, true).map(|()| None)
        };
    }
    // A document: its registered command for the verb ("open" by default).
    let open = verb.is_empty() || verb == "open";
    let verb = if verb.is_empty() {
        String::from("open")
    } else {
        verb
    };
    let command = prog_id(&file)
        .and_then(|id| class_value(&alloc::format!(r"{id}\shell\{verb}\command"), None))
        .or_else(|| {
            extension(&file)
                .and_then(|e| class_value(&alloc::format!(r"{e}\shell\{verb}\command"), None))
        });
    let Some(command) = command else {
        // No Windows association: Windows asks which app to use (unless
        // the caller wants no UI); File Explorer does that here, opening
        // the file with its default app on this PC when it has one.
        return if open && !request.no_ui && !is_directory(&file) {
            open_with(&file, false).map(|()| None)
        } else {
            Err(SE_ERR_NOASSOC)
        };
    };
    let line = expand_command(&command, &file, &request.parameters);
    start(&line, directory).map(Some)
}
/// The file at `path` (a Windows path) to File Explorer's Open / app
/// picker (`choose`), as Windows starts OpenWith.exe.
fn open_with(path: &str, choose: bool) -> Result<(), u32> {
    let w = wide(path);
    let mut native = vec![0u8; door_service_protocol::window::MAX_DOCUMENT];
    let n = unsafe {
        DoorGetNativePath(
            w.as_ptr() as u64,
            native.as_mut_ptr() as u64,
            native.len() as u32 - 1,
        )
    } as usize;
    if n == 0 || n >= native.len() {
        return Err(SE_ERR_FNF);
    }
    match unsafe { DoorOpenWith(native.as_ptr() as u64, n as u32, u32::from(choose)) } {
        0 => Ok(()),
        _ => Err(SE_ERR_NOASSOC),
    }
}
/// The SE_ERR code ShellExecute returns, and the Win32 error ShellExecuteEx
/// sets, for `error`.
fn codes(error: u32) -> (u32, u32) {
    match error {
        SE_ERR_NOASSOC => (SE_ERR_NOASSOC, ERROR_NO_ASSOCIATION),
        ERROR_CANCELLED => (SE_ERR_ACCESSDENIED, ERROR_CANCELLED),
        SE_ERR_FNF | SE_ERR_PNF | SE_ERR_ACCESSDENIED | SE_ERR_OOM => (error, error),
        e => (SE_ERR_FNF.max(if e < 32 { e } else { SE_ERR_FNF }), e),
    }
}
fn shell_execute(request: Execute) -> u64 {
    match execute(&request) {
        Ok(process) => {
            if let Some(p) = process {
                unsafe { DoorCloseHandle(p) };
            }
            42
        }
        Err(e) => {
            let (code, error) = codes(e);
            set_error(error);
            u64::from(code)
        }
    }
}
fn opt_text(pointer: u64, ansi: bool) -> Result<String, u32> {
    if pointer == 0 {
        Ok(String::new())
    } else if ansi {
        read_ansi(pointer)
    } else {
        read_text(pointer)
    }
}
fn request(
    verb: u64,
    file: u64,
    parameters: u64,
    directory: u64,
    ansi: bool,
) -> Result<Execute, u32> {
    let directory = opt_text(directory, ansi)?;
    Ok(Execute {
        verb: opt_text(verb, ansi)?,
        file: opt_text(file, ansi)?,
        parameters: opt_text(parameters, ansi)?,
        directory: (!directory.is_empty()).then_some(directory),
        keep_process: false,
        no_ui: false,
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn ShellExecuteW(
    _window: usize,
    verb: usize,
    file: usize,
    parameters: usize,
    directory: usize,
    _show: i32,
) -> usize {
    let verb = door_windows_abi::widen(verb);
    let file = door_windows_abi::widen(file);
    let parameters = door_windows_abi::widen(parameters);
    let directory = door_windows_abi::widen(directory);
    door_windows_abi::narrow((move || -> u64 {
        match request(verb, file, parameters, directory, false) {
            Ok(r) => shell_execute(r),
            Err(_) => u64::from(SE_ERR_FNF),
        }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn ShellExecuteA(
    _window: usize,
    verb: usize,
    file: usize,
    parameters: usize,
    directory: usize,
    _show: i32,
) -> usize {
    let verb = door_windows_abi::widen(verb);
    let file = door_windows_abi::widen(file);
    let parameters = door_windows_abi::widen(parameters);
    let directory = door_windows_abi::widen(directory);
    door_windows_abi::narrow((move || -> u64 {
        match request(verb, file, parameters, directory, true) {
            Ok(r) => shell_execute(r),
            Err(_) => u64::from(SE_ERR_FNF),
        }
    })())
}
/// SHELLEXECUTEINFO (112 bytes on x64, 60 on x86): cbSize, fMask, hwnd,
/// lpVerb, lpFile, lpParameters, lpDirectory, nShow, hInstApp, ...,
/// hProcess.
fn shell_execute_ex(info: u64, ansi: bool) -> i32 {
    use shape::{SHELLEXECUTEINFO as S, at};
    let parsed = (|| {
        if read_u32(info)? as usize != shape::layout::size_pack(&S, shape::SHELLAPI_PACK) {
            return Err(87);
        }
        let mask = read_u32(info + 4)?;
        let mut r = request(
            read_pointer(info + at(&S, 3))?,
            read_pointer(info + at(&S, 4))?,
            read_pointer(info + at(&S, 5))?,
            read_pointer(info + at(&S, 6))?,
            ansi,
        )?;
        r.keep_process = mask & SEE_MASK_NOCLOSEPROCESS != 0;
        r.no_ui = mask & SEE_MASK_FLAG_NO_UI != 0;
        Ok(r)
    })();
    let r = match parsed {
        Ok(r) => r,
        Err(e) => {
            set_error(e);
            return 0;
        }
    };
    let (process_at, instance_at) = (
        shape::at(&shape::SHELLEXECUTEINFO, 14),
        shape::at(&shape::SHELLEXECUTEINFO, 8),
    );
    let _ = write_pointer(info + process_at, 0);
    match execute(&r) {
        Ok(process) => {
            let _ = write_pointer(info + instance_at, 42);
            match process {
                Some(p) if r.keep_process => {
                    let _ = write_pointer(info + process_at, p);
                }
                Some(p) => {
                    unsafe { DoorCloseHandle(p) };
                }
                None => (),
            }
            1
        }
        Err(e) => {
            let (code, error) = codes(e);
            let _ = write_pointer(info + instance_at, u64::from(code));
            set_error(error);
            0
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ShellExecuteExW(info: usize) -> i32 {
    let info = door_windows_abi::widen(info);
    shell_execute_ex(info, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn ShellExecuteExA(info: usize) -> i32 {
    let info = door_windows_abi::widen(info);
    shell_execute_ex(info, true)
}

// ---- SHFileOperation --------------------------------------------------------------------
const FO_MOVE: u32 = 1;
const FO_COPY: u32 = 2;
const FO_DELETE: u32 = 3;
const FO_RENAME: u32 = 4;
const FOF_MULTIDESTFILES: u16 = 0x1;
const FOF_NOCONFIRMATION: u16 = 0x10;
const FOF_ALLOWUNDO: u16 = 0x40;
const FOF_FILESONLY: u16 = 0x80;
const FOF_NOCONFIRMMKDIR: u16 = 0x200;
const FOF_NORECURSION: u16 = 0x1000;
const MB_YESNO: u32 = 0x4;
const MB_ICONWARNING: u32 = 0x30;
const IDYES: i32 = 6;
/// WIN32_FIND_DATAW entries of a directory or a wildcard pattern.
fn entries(pattern: &str) -> Vec<(String, bool)> {
    let w = wide(pattern);
    let mut data = vec![0u8; 592];
    let handle = unsafe { DoorFindFirstFileW(w.as_ptr() as u64, data.as_mut_ptr() as u64) };
    if handle == u64::MAX || handle == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    loop {
        let attributes = u32::from_le_bytes(data[..4].try_into().unwrap());
        let units: Vec<u16> = data[44..564]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        let name = String::from_utf16_lossy(&units);
        if name != "." && name != ".." {
            out.push((name, attributes & DIRECTORY != 0));
        }
        if unsafe { DoorFindNextFileW(handle, data.as_mut_ptr() as u64) } == 0 {
            break;
        }
    }
    unsafe { DoorFindClose(handle) };
    out
}
fn parent(path: &str) -> &str {
    path.rfind('\\').map_or("", |i| &path[..i])
}
fn leaf(path: &str) -> &str {
    path.rsplit('\\').next().unwrap_or(path)
}
/// The sources a pFrom entry names: itself, or what its wildcard matches.
fn sources(entry: &str, files_only: bool) -> Vec<(String, bool)> {
    let path = absolute(entry, None);
    if leaf(&path).contains(['*', '?']) {
        let dir = String::from(parent(&path));
        return entries(&path)
            .into_iter()
            .filter(|(name, d)| wildcard(leaf(&path), name) && !(files_only && *d))
            .map(|(name, d)| (alloc::format!("{dir}\\{name}"), d))
            .collect();
    }
    match attributes(&path) {
        Some(a) => vec![(path, a & DIRECTORY != 0)],
        None => Vec::new(),
    }
}
fn win32(ok: i32) -> Result<(), u32> {
    if ok != 0 {
        Ok(())
    } else {
        Err(unsafe { RtlGetLastWin32Error() })
    }
}
fn remove_tree(path: &str, directory: bool, recurse: bool) -> Result<(), u32> {
    let w = wide(path);
    if !directory {
        return win32(unsafe { DoorDeleteFileW(w.as_ptr() as u64) });
    }
    if recurse {
        for (name, d) in entries(&alloc::format!("{path}\\*")) {
            remove_tree(&alloc::format!("{path}\\{name}"), d, recurse)?;
        }
    }
    win32(unsafe { DoorRemoveDirectoryW(w.as_ptr() as u64) })
}
fn copy_tree(
    from: &str,
    to: &str,
    directory: bool,
    overwrite: bool,
    recurse: bool,
) -> Result<(), u32> {
    let (a, b) = (wide(from), wide(to));
    if !directory {
        return win32(unsafe {
            DoorCopyFileW(a.as_ptr() as u64, b.as_ptr() as u64, i32::from(!overwrite))
        });
    }
    if !is_directory(to) {
        win32(unsafe { DoorCreateDirectoryW(b.as_ptr() as u64) })?;
    }
    if recurse {
        for (name, d) in entries(&alloc::format!("{from}\\*")) {
            copy_tree(
                &alloc::format!("{from}\\{name}"),
                &alloc::format!("{to}\\{name}"),
                d,
                overwrite,
                recurse,
            )?;
        }
    }
    Ok(())
}
fn confirm(text: &str, caption: &str) -> bool {
    let (t, c) = (wide(text), wide(caption));
    unsafe {
        MessageBoxW(
            0,
            door_windows_abi::narrow(t.as_ptr() as u64),
            door_windows_abi::narrow(c.as_ptr() as u64),
            MB_YESNO | MB_ICONWARNING,
        ) == IDYES
    }
}
/// An item to the caller's Recycle Bin (FOF_ALLOWUNDO): Ok(false) when it
/// is too big for the bin.
fn recycle(path: &str) -> Result<bool, u32> {
    let w = wide(path);
    let mut native = vec![0u8; door_service_protocol::files::RECYCLE_PATH_BYTES];
    let n = unsafe {
        DoorGetNativePath(
            w.as_ptr() as u64,
            native.as_mut_ptr() as u64,
            native.len() as u32,
        )
    } as usize;
    if n == 0 {
        return Err(2);
    }
    let native = core::str::from_utf8(&native[..n]).map_err(|_| 123u32)?;
    let registry =
        unsafe { DoorGetInheritedHandle(door_abi::platform::inherited::REGISTRY as u32) };
    let mut storage = door_service_client::Storage::connect(registry).map_err(|_| 1117u32)?;
    match storage.recycle(native) {
        Ok(door_service_protocol::files::RecycleOutcome::Recycled(_)) => Ok(true),
        Ok(door_service_protocol::files::RecycleOutcome::TooBig) => Ok(false),
        Err(door_service_client::Error::Service(door_service_protocol::Code::Denied)) => Err(5),
        Err(door_service_client::Error::Service(door_service_protocol::Code::NotFound)) => Err(2),
        Err(_) => Err(0x4C7), // ERROR_CANCELLED-like: the bin refused
    }
}
/// SHFILEOPSTRUCTW (56 bytes): hwnd, wFunc at 8, pFrom 16, pTo 24, fFlags
/// (u16) at 32, fAnyOperationsAborted (BOOL) at 36. Deleting with
/// FOF_ALLOWUNDO moves the items to the user's Recycle Bin (without a
/// prompt, as Windows 11's default "Display delete confirmation" is off);
/// an item too big for the bin is deleted permanently only after asking
/// (or at once with FOF_NOCONFIRMATION).
fn file_operation(op: u64, ansi: bool) -> i32 {
    let run = || -> Result<bool, u32> {
        use shape::{SHFILEOPSTRUCT as F, at};
        let function = read_u32(op + at(&F, 1))?;
        let flags = {
            let mut b = [0u8; 2];
            read(op + at(&F, 4), &mut b)?;
            u16::from_le_bytes(b)
        };
        let list = |pointer: u64| -> Result<Vec<String>, u32> {
            if pointer == 0 {
                return Ok(Vec::new());
            }
            if ansi {
                let mut units = Vec::new();
                let mut at = 0;
                loop {
                    let mut b = [0u8];
                    read(pointer + at, &mut b)?;
                    units.push(u16::from(b[0]));
                    at += 1;
                    if units.len() >= 2 && units[units.len() - 2..] == [0, 0] {
                        break;
                    }
                }
                Ok(path_list(&units))
            } else {
                let mut units = Vec::new();
                loop {
                    let mut b = [0u8; 2];
                    read(pointer + 2 * units.len() as u64, &mut b)?;
                    units.push(u16::from_le_bytes(b));
                    if units.len() >= 2 && units[units.len() - 2..] == [0, 0] {
                        break;
                    }
                    if units.len() > 65536 {
                        return Err(206);
                    }
                }
                Ok(path_list(&units))
            }
        };
        let from = list(read_pointer(op + at(&F, 2))?)?;
        let to = list(read_pointer(op + at(&F, 3))?)?;
        if from.is_empty() || !matches!(function, FO_MOVE | FO_COPY | FO_DELETE | FO_RENAME) {
            return Err(0x7C);
        }
        let ask = flags & FOF_NOCONFIRMATION == 0;
        let recurse = flags & FOF_NORECURSION == 0;
        let files_only = flags & FOF_FILESONLY != 0;
        let mut items = Vec::new();
        for entry in &from {
            let found = sources(entry, files_only);
            if found.is_empty() {
                return Err(2);
            }
            items.extend(found);
        }
        if function == FO_DELETE {
            if flags & FOF_ALLOWUNDO != 0 {
                for (path, directory) in &items {
                    if recycle(path)? {
                        continue;
                    }
                    let what = if *directory { "folder" } else { "file" };
                    if ask
                        && !confirm(
                            &alloc::format!(
                                "This {what} is too big to recycle. Do you want to permanently delete it?"
                            ),
                            if *directory {
                                "Delete Folder"
                            } else {
                                "Delete File"
                            },
                        )
                    {
                        return Ok(true);
                    }
                    remove_tree(path, *directory, recurse)?;
                }
                return Ok(false);
            }
            if ask {
                let (text, caption) = if items.len() == 1 {
                    (
                        alloc::format!(
                            "Are you sure you want to permanently delete this {}?",
                            if items[0].1 { "folder" } else { "file" }
                        ),
                        if items[0].1 {
                            "Delete Folder"
                        } else {
                            "Delete File"
                        },
                    )
                } else {
                    (
                        alloc::format!(
                            "Are you sure you want to permanently delete these {} items?",
                            items.len()
                        ),
                        "Delete Multiple Items",
                    )
                };
                if !confirm(&text, caption) {
                    return Ok(true);
                }
            }
            for (path, directory) in &items {
                remove_tree(path, *directory, recurse)?;
            }
            return Ok(false);
        }
        let destination = |i: usize, source: &str| -> Result<String, u32> {
            let target = if flags & FOF_MULTIDESTFILES != 0 {
                to.get(i)
            } else {
                to.first()
            }
            .ok_or(0x7Cu32)?;
            let target = absolute(target, None);
            Ok(
                if function != FO_RENAME && is_directory(&target) && flags & FOF_MULTIDESTFILES == 0
                {
                    alloc::format!("{target}\\{}", leaf(source))
                } else {
                    target
                },
            )
        };
        if function == FO_RENAME {
            let (source, _) = &items[0];
            let target = destination(0, source)?;
            let (a, b) = (wide(source), wide(&target));
            win32(unsafe { DoorMoveFileExW(a.as_ptr() as u64, b.as_ptr() as u64, 0) })?;
            return Ok(false);
        }
        // A missing destination folder for several sources is created
        // (asked unless FOF_NOCONFIRMMKDIR).
        if items.len() > 1 && flags & FOF_MULTIDESTFILES == 0 {
            let target = absolute(to.first().ok_or(0x7Cu32)?, None);
            if !is_directory(&target) {
                if ask
                    && flags & FOF_NOCONFIRMMKDIR == 0
                    && !confirm(
                        &alloc::format!(
                            "The folder \"{target}\" does not exist. Do you want to create it?"
                        ),
                        "Confirm Folder Create",
                    )
                {
                    return Ok(true);
                }
                let r = create_tree(&target);
                if r != 0 && r != 183 {
                    return Err(r);
                }
            }
        }
        let mut aborted = false;
        for (i, (source, directory)) in items.iter().enumerate() {
            let target = destination(i, source)?;
            let mut overwrite = true;
            if attributes(&target).is_some() && ask {
                overwrite = confirm(
                    &alloc::format!(
                        "The destination already has a file named \"{}\".\n\nReplace the file in the destination?",
                        leaf(&target)
                    ),
                    "Replace or Skip Files",
                );
                if !overwrite {
                    aborted = true;
                    continue;
                }
            }
            if function == FO_COPY {
                copy_tree(source, &target, *directory, overwrite, recurse)?;
            } else {
                let (a, b) = (wide(source), wide(&target));
                // MOVEFILE_REPLACE_EXISTING | MOVEFILE_COPY_ALLOWED
                win32(unsafe {
                    DoorMoveFileExW(
                        a.as_ptr() as u64,
                        b.as_ptr() as u64,
                        if overwrite { 3 } else { 2 },
                    )
                })?;
            }
        }
        Ok(aborted)
    };
    match run() {
        Ok(aborted) => {
            let _ = write(
                op + shape::at(&shape::SHFILEOPSTRUCT, 5),
                &i32::from(aborted).to_le_bytes(),
            );
            0
        }
        Err(e) => e as i32,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SHFileOperationW(op: usize) -> i32 {
    let op = door_windows_abi::widen(op);
    file_operation(op, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn SHFileOperationA(op: usize) -> i32 {
    let op = door_windows_abi::widen(op);
    file_operation(op, true)
}

// ---- SHGetFileInfo ----------------------------------------------------------------------
const SHGFI_ICON: u32 = 0x100;
const SHGFI_DISPLAYNAME: u32 = 0x200;
const SHGFI_TYPENAME: u32 = 0x400;
const SHGFI_ATTRIBUTES: u32 = 0x800;
const SHGFI_ICONLOCATION: u32 = 0x1000;
const SHGFI_EXETYPE: u32 = 0x2000;
const SHGFI_SYSICONINDEX: u32 = 0x4000;
const SHGFI_USEFILEATTRIBUTES: u32 = 0x10;
const SHGFI_PIDL: u32 = 0x8;
/// SFGAO_* bits a file or folder has.
fn sfgao(directory: bool) -> u32 {
    const CANCOPY: u32 = 0x1;
    const CANMOVE: u32 = 0x2;
    const CANRENAME: u32 = 0x10;
    const CANDELETE: u32 = 0x20;
    const HASPROPSHEET: u32 = 0x40;
    const STREAM: u32 = 0x40_0000;
    const FILESYSANCESTOR: u32 = 0x1000_0000;
    const FOLDER: u32 = 0x2000_0000;
    const FILESYSTEM: u32 = 0x4000_0000;
    const HASSUBFOLDER: u32 = 0x8000_0000;
    let common = CANCOPY | CANMOVE | CANRENAME | CANDELETE | HASPROPSHEET | FILESYSTEM;
    if directory {
        common | FOLDER | FILESYSANCESTOR | HASSUBFOLDER
    } else {
        common | STREAM
    }
}
fn type_name(path: &str, directory: bool) -> String {
    if directory {
        return String::from("File folder");
    }
    prog_id(path)
        .and_then(|id| class_value(&id, None))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| default_type_name(leaf(path)))
}
/// The PE kind SHGFI_EXETYPE reports: MAKELONG('PE', version) for a GUI
/// program, 'PE' (0x4550) for a console one, 0 otherwise.
fn exe_type(path: &str) -> u64 {
    let w = wide(path);
    let file = unsafe { DoorCreateFileW(w.as_ptr() as u64, 0x8000_0000, 1, 0, 3, 0, 0) };
    if file == u64::MAX || file == 0 {
        return 0;
    }
    let mut head = vec![0u8; 1024];
    let mut done = [0u8; 4];
    let ok = unsafe {
        DoorReadFile(
            file,
            head.as_mut_ptr() as u64,
            1024,
            done.as_mut_ptr() as u64,
            0,
        )
    } != 0;
    unsafe { DoorCloseFile(file) };
    if !ok || &head[..2] != b"MZ" {
        return 0;
    }
    let pe = u32::from_le_bytes(head[60..64].try_into().unwrap()) as usize;
    if pe + 0x5E > head.len() || &head[pe..pe + 4] != b"PE\0\0" {
        return 0;
    }
    let optional = pe + 24;
    let subsystem = u16::from_le_bytes([head[optional + 68], head[optional + 69]]);
    let version = u64::from(head[optional + 40]) << 8 | u64::from(head[optional + 42]);
    match subsystem {
        2 => 0x4550 | version << 16,
        3 => 0x4550,
        _ => 0,
    }
}
const SHGFI_SMALLICON: u32 = 0x1;
const SHGFI_OPENICON: u32 = 0x2;
/// Where a file's icon comes from: a file and an ExtractIcon index (its
/// own icon, or its type's DefaultIcon), or the shell's icon table.
#[derive(Clone, Debug, PartialEq)]
enum IconSource {
    File(String, i32),
    Shell(u32),
}
/// `%NAME%` variables of a registry location replaced from the
/// environment (REG_EXPAND_SZ DefaultIcon values).
fn expand_variables(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match variable(name) {
                    Some(value) => out.push_str(&value),
                    None => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}
/// A DefaultIcon value (`"path",index`, `%1` for the file itself).
fn parse_location(value: &str, file: &str) -> Option<(String, i32)> {
    let value = value.trim();
    let (path, index) = match value.rsplit_once(',') {
        Some((p, i)) if i.trim().parse::<i32>().is_ok() => (p, i.trim().parse().ok()?),
        _ => (value, 0),
    };
    let path = path.trim().trim_matches('"');
    if path.is_empty() {
        return None;
    }
    if path == "%1" {
        return Some((String::from(file), index));
    }
    Some((expand_variables(path), index))
}
/// How many icon groups ExtractIcon finds in `path`.
fn icon_count(path: &str) -> u32 {
    let w = wide(path);
    unsafe {
        PrivateExtractIconsW(
            door_windows_abi::narrow(w.as_ptr() as u64),
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        )
    }
}
fn icon_source(path: &str, directory: bool, file_attributes_only: bool) -> IconSource {
    use door_windows_abi::shell_icons as s;
    if directory {
        return IconSource::Shell(s::FOLDER);
    }
    let default = prog_id(path)
        .and_then(|id| class_value(&alloc::format!(r"{id}\DefaultIcon"), None))
        .or_else(|| {
            extension(path).and_then(|e| class_value(&alloc::format!(r"{e}\DefaultIcon"), None))
        })
        .and_then(|v| parse_location(&v, path));
    if let Some((file, index)) = default {
        return IconSource::File(file, index);
    }
    let own = extension(path).is_some_and(|e| {
        [".exe", ".dll", ".ico", ".cpl", ".scr", ".ocx"]
            .iter()
            .any(|x| e.eq_ignore_ascii_case(x))
    });
    if own && !file_attributes_only && icon_count(path) > 0 {
        return IconSource::File(String::from(path), 0);
    }
    IconSource::Shell(s::for_file(leaf(path)))
}
/// The location SHGFI_ICONLOCATION and SHGetStockIconInfo report.
fn location(source: &IconSource) -> (String, i32) {
    match source {
        IconSource::File(path, index) => (path.clone(), *index),
        IconSource::Shell(index) => (
            String::from(door_windows_abi::shell_icons::SHELL32_PATH),
            *index as i32,
        ),
    }
}
/// A new icon of `size` px for a source; 0 when it cannot be made.
fn make_icon(source: &IconSource, size: u32) -> u64 {
    let (path, index) = location(source);
    let w = wide(&path);
    let mut icon = 0u64;
    let n = unsafe {
        PrivateExtractIconsW(
            door_windows_abi::narrow(w.as_ptr() as u64),
            index,
            size as i32,
            size as i32,
            door_windows_abi::narrow(&mut icon as *mut u64 as u64),
            0,
            1,
            0,
        )
    };
    if n == 1 && icon != 0 {
        return icon;
    }
    // A broken DefaultIcon or a program without one: the type's icon.
    if let IconSource::File(..) = source {
        let fallback = IconSource::Shell(door_windows_abi::shell_icons::for_file(leaf(&path)));
        return make_icon(&fallback, size);
    }
    0
}
/// SHFILEINFOW (696 bytes): hIcon, iIcon at 8, dwAttributes at 12,
/// szDisplayName[260] at 16, szTypeName[80] at 536. SHFILEINFOA: 352 bytes,
/// names at 16 and 276. iIcon is the shell icon table's index (the system
/// image list's).
fn file_info(path: u64, file_attributes: u32, out: u64, size: u32, flags: u32, ansi: bool) -> u64 {
    let path = if flags & SHGFI_PIDL != 0 {
        match list_path(path) {
            Some(p) => p,
            None => return 0,
        }
    } else {
        match opt_text(path, ansi) {
            Ok(p) => p,
            Err(_) => return 0,
        }
    };
    if flags & SHGFI_EXETYPE != 0 {
        return exe_type(&absolute(&path, None));
    }
    let full = absolute(&path, None);
    let attributes_only = flags & SHGFI_USEFILEATTRIBUTES != 0;
    let directory = if attributes_only {
        file_attributes & DIRECTORY != 0
    } else {
        match attributes(&full) {
            Some(a) => a & DIRECTORY != 0,
            None => return 0,
        }
    };
    let (record, display_at, display_len, type_at, type_len) = if ansi {
        (352usize, 16usize, 260usize, 276usize, 80usize)
    } else {
        (696, 16, 260, 536, 80)
    };
    let icons = SHGFI_ICON | SHGFI_SYSICONINDEX | SHGFI_ICONLOCATION;
    let source = (flags & icons != 0).then(|| {
        let s = icon_source(&full, directory, attributes_only);
        if flags & SHGFI_OPENICON != 0
            && s == IconSource::Shell(door_windows_abi::shell_icons::FOLDER)
        {
            IconSource::Shell(door_windows_abi::shell_icons::FOLDER_OPEN)
        } else {
            s
        }
    });
    if flags & SHGFI_SYSICONINDEX != 0 && out == 0 {
        // Only the image list is asked for.
        return system_image_list(flags & SHGFI_SMALLICON != 0);
    }
    if out == 0 || (size as usize) < record {
        return u64::from(out == 0);
    }
    let mut b = vec![0u8; record];
    if flags & SHGFI_ATTRIBUTES != 0 {
        b[12..16].copy_from_slice(&sfgao(directory).to_le_bytes());
    }
    let put = |b: &mut [u8], at: usize, len: usize, text: &str| {
        let bytes = if ansi {
            ansi_bytes(text)
        } else {
            wide_bytes(text)
        };
        let unit = if ansi { 1 } else { 2 };
        let n = bytes.len().min(len * unit);
        b[at..at + n].copy_from_slice(&bytes[..n]);
    };
    if flags & SHGFI_DISPLAYNAME != 0 {
        put(&mut b, display_at, display_len, leaf(&full));
    }
    if flags & SHGFI_TYPENAME != 0 {
        put(&mut b, type_at, type_len, &type_name(&full, directory));
    }
    let mut result = 1u64;
    if let Some(source) = &source {
        if flags & SHGFI_ICONLOCATION != 0 {
            let (file, index) = location(source);
            put(&mut b, display_at, display_len, &file);
            b[8..12].copy_from_slice(&index.to_le_bytes());
        } else {
            b[8..12].copy_from_slice(&system_index(source).to_le_bytes());
        }
        if flags & SHGFI_ICON != 0 {
            let px = if flags & SHGFI_SMALLICON != 0 { 16 } else { 32 };
            let icon = make_icon(source, px);
            if icon == 0 {
                return 0;
            }
            b[..8].copy_from_slice(&icon.to_le_bytes());
        }
        if flags & SHGFI_SYSICONINDEX != 0 {
            result = system_image_list(flags & SHGFI_SMALLICON != 0);
        }
    }
    if write(out, &b).is_err() {
        return 0;
    }
    result
}
#[unsafe(no_mangle)]
pub extern "system" fn SHGetFileInfoW(
    path: usize,
    attributes: u32,
    out: usize,
    size: u32,
    flags: u32,
) -> usize {
    let path = door_windows_abi::widen(path);
    let out = door_windows_abi::widen(out);
    door_windows_abi::narrow((move || -> u64 {
        file_info(path, attributes, out, size, flags, false)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn SHGetFileInfoA(
    path: usize,
    attributes: u32,
    out: usize,
    size: u32,
    flags: u32,
) -> usize {
    let path = door_windows_abi::widen(path);
    let out = door_windows_abi::widen(out);
    door_windows_abi::narrow((move || -> u64 {
        file_info(path, attributes, out, size, flags, true)
    })())
}

// ---- odds and ends ------------------------------------------------------------------------
/// BUILTIN\Administrators is enabled in the caller's token (elevated).
#[unsafe(no_mangle)]
pub extern "system" fn IsUserAnAdmin() -> i32 {
    // S-1-5-32-544: revision 1, two sub-authorities, NT authority.
    let sid: [u8; 16] = [1, 2, 0, 0, 0, 0, 0, 5, 32, 0, 0, 0, 0x20, 0x02, 0, 0];
    let mut member = 0i32;
    let ok = unsafe {
        CheckTokenMembership(
            0,
            door_windows_abi::narrow(sid.as_ptr() as u64),
            door_windows_abi::narrow(&mut member as *mut i32 as u64),
        )
    };
    i32::from(ok != 0 && member != 0)
}
/// Explorer refreshes from the volume itself; nothing to tell it.
#[unsafe(no_mangle)]
pub extern "system" fn SHChangeNotify(_event: i32, _flags: u32, _item1: usize, _item2: usize) {
    {}
}
/// Shell_NotifyIcon: NOTIFYICONDATA (x64 W: hWnd at 8, uID 16, uFlags 20,
/// uCallbackMessage 24, hIcon 32, szTip at 40 (64 units before the 952-byte
/// layout, 128 since), szInfo 304, uVersion 816, szInfoTitle 820,
/// dwInfoFlags 948). The icon lives in the taskbar's hidden-icons flyout;
/// a balloon (NIF_INFO) becomes a toast, as on Windows 10 and later.
fn notify_icon(message: u32, data: u64, ansi: bool) -> i32 {
    const NIF_TIP: u32 = 0x4;
    const NIF_INFO: u32 = 0x10;
    const NIM_SETVERSION: u32 = 4;
    let Ok(size) = read_u32(data) else {
        set_error(87);
        return 0;
    };
    // The ANSI form is the same with 8-bit strings. szTip follows the head
    // (at 40 on x64, 24 on x86, P17).
    let unit = if ansi { 1 } else { 2 };
    let tip_at = shape::at(&shape::NOTIFY_HEAD, 6) as usize;
    let (minimum, full_tip) = if POINTER == 4 {
        // NOTIFYICONDATA_V1_SIZE and V2_SIZE.
        (
            tip_at + 64 * unit,
            tip_at + 128 * unit + 8 + 256 * unit + 4 + 64 * unit + 4,
        )
    } else if ansi {
        (88, 504)
    } else {
        (152, 952)
    };
    if (size as usize) < minimum || size > 976 {
        set_error(87);
        return 0;
    }
    let mut b = vec![0u8; size as usize];
    if read(data, &mut b).is_err() {
        set_error(998);
        return 0;
    }
    let u32_at = |at: usize| {
        b.get(at..at + 4)
            .map_or(0, |v| u32::from_le_bytes(v.try_into().unwrap()))
    };
    let u64_at = |at: usize| {
        b.get(at..at + 8)
            .map_or(0, |v| u64::from_le_bytes(v.try_into().unwrap()))
    };
    let head = |index| shape::at(&shape::NOTIFY_HEAD, index) as usize;
    let pointer_at = |at: usize| {
        if POINTER == 4 {
            i64::from(u32_at(at) as i32) as u64
        } else {
            u64_at(at)
        }
    };
    let (window, id, flags, callback, icon) = (
        pointer_at(head(1)),
        u32_at(head(2)),
        u32_at(head(3)),
        u32_at(head(4)),
        pointer_at(head(5)),
    );
    let text = |at: usize, units: usize| -> String {
        if ansi {
            let raw = b.get(at..(at + units).min(b.len())).unwrap_or(&[]);
            let end = raw.iter().position(|c| *c == 0).unwrap_or(raw.len());
            raw[..end].iter().map(|&c| char::from(c)).collect()
        } else {
            let raw = b.get(at..(at + 2 * units).min(b.len())).unwrap_or(&[]);
            let units: Vec<u16> = raw
                .chunks_exact(2)
                .map(|p| u16::from_le_bytes([p[0], p[1]]))
                .take_while(|u| *u != 0)
                .collect();
            String::from_utf16_lossy(&units)
        }
    };
    let tip_units = if size as usize >= full_tip { 128 } else { 64 };
    let tip = if flags & NIF_TIP != 0 {
        text(tip_at, tip_units)
    } else {
        String::new()
    };
    // uVersion follows szTip, dwState, dwStateMask and szInfo.
    let info_at = tip_at + tip_units * unit + 8;
    let version_at = info_at + 256 * unit;
    let version = if message == NIM_SETVERSION {
        u32_at(version_at)
    } else {
        0
    };
    let tip_wide = wide(&tip);
    let ok = unsafe {
        DoorNotifyIcon(
            message,
            window,
            id,
            flags,
            callback,
            icon,
            tip_wide.as_ptr() as u64,
            tip_wide.len() as u32,
            version,
        )
    };
    if ok != 0
        && flags & NIF_INFO != 0
        && message <= 1
        && size as usize >= version_at + 4 + 64 * unit
    {
        let body = text(info_at, 256);
        let title = text(version_at + 4, 64);
        if !body.is_empty() {
            balloon(&title, &body);
        }
    }
    ok
}
/// A balloon notification as a toast from this program.
fn balloon(title: &str, body: &str) {
    // Toasts take no control characters: line breaks become spaces.
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect()
    };
    let (title, body) = (clean(title), clean(body));
    let (title, body) = (title.trim(), body.trim());
    let registry =
        unsafe { DoorGetInheritedHandle(door_abi::platform::inherited::REGISTRY as u32) };
    if let Ok(mut n) = door_service_client::Notifications::connect(registry) {
        let _ = n.post(&door_service_client::notifications::Toast {
            icon: door_service_protocol::shell::icon::WINDOWS_APP,
            flags: 0,
            title: door_service_protocol::shell::clip(
                if title.is_empty() { body } else { title },
                63,
            ),
            body: door_service_protocol::shell::clip(if title.is_empty() { "" } else { body }, 127),
            actions: ["", ""],
        });
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn Shell_NotifyIconW(message: u32, data: usize) -> i32 {
    let data = door_windows_abi::widen(data);
    notify_icon(message, data, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn Shell_NotifyIconA(message: u32, data: usize) -> i32 {
    let data = door_windows_abi::widen(data);
    notify_icon(message, data, true)
}
// HDROP: a DROPFILES block (pFiles offset u32, pt, fNC, fWide u32 at 16)
// followed by a double-NUL list of paths.
fn drop_list(drop: u64) -> Option<(Vec<String>, [i32; 2])> {
    let mut head = [0u8; 20];
    read(drop, &mut head).ok()?;
    let offset = u64::from(u32::from_le_bytes(head[..4].try_into().unwrap()));
    let point = [
        i32::from_le_bytes(head[4..8].try_into().unwrap()),
        i32::from_le_bytes(head[8..12].try_into().unwrap()),
    ];
    let wide_names = u32::from_le_bytes(head[16..20].try_into().unwrap()) != 0;
    let size = unsafe {
        RtlSizeHeap(
            door_windows_abi::narrow(heap()),
            0,
            door_windows_abi::narrow(drop),
        )
    };
    let limit = if size == usize::MAX {
        65536
    } else {
        size as u64
    };
    let mut units = Vec::new();
    let mut at = offset;
    while at < limit {
        let unit = if wide_names {
            let mut b = [0u8; 2];
            read(drop + at, &mut b).ok()?;
            at += 2;
            u16::from_le_bytes(b)
        } else {
            let mut b = [0u8];
            read(drop + at, &mut b).ok()?;
            at += 1;
            u16::from(b[0])
        };
        units.push(unit);
        if units.len() >= 2 && units[units.len() - 2..] == [0, 0] {
            break;
        }
    }
    Some((path_list(&units), point))
}
fn drag_query(drop: u64, index: u32, out: u64, capacity: u32, ansi: bool) -> u32 {
    let Some((files, _)) = drop_list(drop) else {
        return 0;
    };
    if index == 0xFFFF_FFFF {
        return files.len() as u32;
    }
    let Some(name) = files.get(index as usize) else {
        return 0;
    };
    let length = if ansi {
        name.len()
    } else {
        name.encode_utf16().count()
    };
    if out == 0 {
        return length as u32;
    }
    if capacity == 0 {
        return 0;
    }
    let text: String = if ansi {
        name.chars().take(capacity as usize - 1).collect()
    } else {
        String::from_utf16_lossy(
            &name
                .encode_utf16()
                .take(capacity as usize - 1)
                .collect::<Vec<u16>>(),
        )
    };
    let bytes = if ansi {
        ansi_bytes(&text)
    } else {
        wide_bytes(&text)
    };
    if write(out, &bytes).is_err() {
        return 0;
    }
    if ansi {
        text.len() as u32
    } else {
        text.encode_utf16().count() as u32
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DragQueryFileW(drop: usize, index: u32, out: usize, capacity: u32) -> u32 {
    let drop = door_windows_abi::widen(drop);
    let out = door_windows_abi::widen(out);
    drag_query(drop, index, out, capacity, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn DragQueryFileA(drop: usize, index: u32, out: usize, capacity: u32) -> u32 {
    let drop = door_windows_abi::widen(drop);
    let out = door_windows_abi::widen(out);
    drag_query(drop, index, out, capacity, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn DragQueryPoint(drop: usize, out: usize) -> i32 {
    let drop = door_windows_abi::widen(drop);
    let out = door_windows_abi::widen(out);
    {
        match drop_list(drop) {
            Some((_, p)) => {
                i32::from(write(out, &[p[0].to_le_bytes(), p[1].to_le_bytes()].concat()).is_ok())
            }
            None => 0,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DragFinish(drop: usize) {
    let drop = door_windows_abi::widen(drop);
    {
        if drop != 0 {
            unsafe {
                RtlFreeHeap(
                    door_windows_abi::narrow(heap()),
                    0,
                    door_windows_abi::narrow(drop),
                )
            };
        }
    }
}
/// Sets or clears WS_EX_ACCEPTFILES (0x10) on the window.
#[unsafe(no_mangle)]
pub extern "system" fn DragAcceptFiles(window: usize, accept: i32) {
    let window = door_windows_abi::widen(window);
    {
        const GWL_EXSTYLE: i32 = -20;
        let style = unsafe { GetWindowLongPtrW(door_windows_abi::narrow(window), GWL_EXSTYLE) };
        let style = if accept != 0 {
            style | 0x10
        } else {
            style & !0x10
        };
        unsafe { SetWindowLongPtrW(door_windows_abi::narrow(window), GWL_EXSTYLE, style) };
    }
}

// ---- icons: the system image list, ExtractIcon, stock icons ------------------------
type ImageListCreate = unsafe extern "system" fn(i32, i32, u32, i32, i32) -> usize;
type ImageListReplaceIcon = unsafe extern "system" fn(usize, i32, usize) -> i32;
/// The process's system image lists (large 32 px, small 16 px) and the
/// file icons added to them after the shell table's.
struct SystemImages {
    large: u64,
    small: u64,
    replace: Option<ImageListReplaceIcon>,
    files: Vec<(String, i32, i32)>,
}
struct Lock<T> {
    held: core::sync::atomic::AtomicBool,
    value: core::cell::UnsafeCell<T>,
}
// SAFETY: `held` serializes every access to the value.
unsafe impl<T: Send> Sync for Lock<T> {}
impl<T> Lock<T> {
    const fn new(value: T) -> Self {
        Self {
            held: core::sync::atomic::AtomicBool::new(false),
            value: core::cell::UnsafeCell::new(value),
        }
    }
    fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        use core::sync::atomic::Ordering;
        while self.held.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
        // SAFETY: `held` gives this closure the only access.
        let r = f(unsafe { &mut *self.value.get() });
        self.held.store(false, Ordering::Release);
        r
    }
}
static SYSTEM_IMAGES: Lock<Option<SystemImages>> = Lock::new(None);
/// comctl32 at run time, as Windows' shell32 delay-loads it.
fn comctl32(name: &str) -> u64 {
    let module = wide("comctl32.dll");
    let handle = unsafe { DoorLoadLibraryW(module.as_ptr() as u64) };
    if handle == 0 {
        return 0;
    }
    let mut symbol = Vec::from(name.as_bytes());
    symbol.push(0);
    unsafe { DoorGetProcAddress(handle, symbol.as_ptr() as u64) }
}
fn with_images<R>(f: impl FnOnce(&mut SystemImages) -> R) -> Option<R> {
    SYSTEM_IMAGES.with(|guard| {
        if guard.is_none() {
            let (create, replace) = (
                comctl32("ImageList_Create"),
                comctl32("ImageList_ReplaceIcon"),
            );
            if create == 0 || replace == 0 {
                return None;
            }
            // SAFETY: comctl32's exports with these exact signatures.
            let create: ImageListCreate =
                unsafe { core::mem::transmute(door_windows_abi::narrow(create)) };
            let replace: ImageListReplaceIcon =
                unsafe { core::mem::transmute(door_windows_abi::narrow(replace)) };
            // ILC_COLOR32 | ILC_MASK
            let (large, small) =
                unsafe { (create(32, 32, 0x21, 64, 16), create(16, 16, 0x21, 64, 16)) };
            if large == 0 || small == 0 {
                return None;
            }
            for (_, stem) in door_windows_abi::shell_icons::ICONS {
                for (list, size) in [(large, 32u32), (small, 16)] {
                    let icon =
                        unsafe { DoorShellIcon(stem.as_ptr() as u64, stem.len() as u32, size) };
                    unsafe { replace(list, -1, door_windows_abi::narrow(icon)) };
                    if icon != 0 {
                        unsafe { DestroyIcon(door_windows_abi::narrow(icon)) };
                    }
                }
            }
            *guard = Some(SystemImages {
                large: door_windows_abi::widen(large),
                small: door_windows_abi::widen(small),
                replace: Some(replace),
                files: Vec::new(),
            });
        }
        guard.as_mut().map(f)
    })
}
fn system_image_list(small: bool) -> u64 {
    with_images(|s| if small { s.small } else { s.large }).unwrap_or(0)
}
/// The system image list index of an icon source: the shell table's
/// position, or a file icon added on first use.
fn system_index(source: &IconSource) -> i32 {
    if let IconSource::Shell(index) = source {
        return door_windows_abi::shell_icons::ICONS
            .iter()
            .position(|(i, _)| i == index)
            .map_or(0, |p| p as i32);
    }
    let IconSource::File(path, index) = source else {
        return 0;
    };
    let known = with_images(|s| {
        s.files
            .iter()
            .find(|(p, i, _)| p.eq_ignore_ascii_case(path) && i == index)
            .map(|(_, _, at)| *at)
    })
    .flatten();
    if let Some(at) = known {
        return at;
    }
    let (large, small) = (make_icon(source, 32), make_icon(source, 16));
    let at = with_images(|s| {
        let replace = s.replace?;
        let at = unsafe {
            replace(
                door_windows_abi::narrow(s.large),
                -1,
                door_windows_abi::narrow(large),
            )
        };
        unsafe {
            replace(
                door_windows_abi::narrow(s.small),
                -1,
                door_windows_abi::narrow(small),
            )
        };
        (at >= 0).then(|| {
            s.files.push((path.clone(), *index, at));
            at
        })
    })
    .flatten();
    for icon in [large, small] {
        if icon != 0 {
            unsafe { DestroyIcon(door_windows_abi::narrow(icon)) };
        }
    }
    at.unwrap_or(0)
}
/// Shell_GetImageLists: the large and small system image lists.
#[unsafe(no_mangle)]
pub extern "system" fn Shell_GetImageLists(large: usize, small: usize) -> i32 {
    let large = door_windows_abi::widen(large);
    let small = door_windows_abi::widen(small);
    {
        let (l, s) = (system_image_list(false), system_image_list(true));
        if l == 0 {
            return 0;
        }
        i32::from(
            (large == 0 || write_pointer(large, l).is_ok())
                && (small == 0 || write_pointer(small, s).is_ok()),
        )
    }
}
/// SHGetImageList (SHIL_LARGE 0, SHIL_SMALL 1, SHIL_EXTRALARGE 2,
/// SHIL_SYSSMALL 3, SHIL_JUMBO 4): the list as the IImageList the caller
/// asked for is the comctl32 handle itself, as Windows' is.
#[unsafe(no_mangle)]
pub extern "system" fn SHGetImageList(kind: i32, _iid: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        let list = match kind {
            0 | 2 | 4 => system_image_list(false),
            1 | 3 => system_image_list(true),
            _ => return 0x8007_0057,
        };
        if list == 0 {
            return 0x8000_4005;
        }
        if write_pointer(out, list).is_err() {
            return 0x8000_4003;
        }
        0
    }
}
/// ExtractIconEx: up to `count` icons from index `index` (negative: a
/// resource id) at 32 px into `large` and 16 px into `small`; with index
/// -1 and no outputs, the number of icons.
fn extract_icons(file: &str, index: i32, large: u64, small: u64, count: u32) -> u32 {
    let w = wide(file);
    if index == -1 && large == 0 && small == 0 {
        return unsafe {
            PrivateExtractIconsW(
                door_windows_abi::narrow(w.as_ptr() as u64),
                0,
                0,
                0,
                0,
                0,
                0,
                0,
            )
        };
    }
    let mut made = 0;
    for (out, size) in [(large, 32), (small, 16)] {
        if out == 0 {
            continue;
        }
        let n = unsafe {
            PrivateExtractIconsW(
                door_windows_abi::narrow(w.as_ptr() as u64),
                index,
                size,
                size,
                door_windows_abi::narrow(out),
                0,
                count,
                0,
            )
        };
        if made == 0 {
            made = n;
        }
    }
    made
}
#[unsafe(no_mangle)]
pub extern "system" fn ExtractIconExW(
    file: usize,
    index: i32,
    large: usize,
    small: usize,
    count: u32,
) -> u32 {
    let file = door_windows_abi::widen(file);
    let large = door_windows_abi::widen(large);
    let small = door_windows_abi::widen(small);
    {
        match read_text(file) {
            Ok(path) => extract_icons(&path, index, large, small, count),
            Err(_) => 0,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ExtractIconExA(
    file: usize,
    index: i32,
    large: usize,
    small: usize,
    count: u32,
) -> u32 {
    let file = door_windows_abi::widen(file);
    let large = door_windows_abi::widen(large);
    let small = door_windows_abi::widen(small);
    {
        match read_ansi(file) {
            Ok(path) => extract_icons(&path, index, large, small, count),
            Err(_) => 0,
        }
    }
}
/// ExtractIcon: one 32 px icon; index -1 counts them. A file that holds
/// no icon resources at all (not a program, library or .ico) gives 1.
fn extract_icon(path: &str, index: i32) -> u64 {
    let full = absolute(path, None);
    let count = icon_count(&full);
    if index == -1 {
        return u64::from(count);
    }
    if count == 0 {
        return u64::from(
            attributes(&full).is_some() && !door_windows_abi::shell_icons::is_shell_library(&full),
        );
    }
    let mut icon = 0u64;
    extract_icons(&full, index, &mut icon as *mut u64 as u64, 0, 1);
    icon
}
#[unsafe(no_mangle)]
pub extern "system" fn ExtractIconW(_instance: usize, file: usize, index: u32) -> usize {
    let file = door_windows_abi::widen(file);
    door_windows_abi::narrow((move || -> u64 {
        read_text(file).map_or(0, |p| extract_icon(&p, index as i32))
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn ExtractIconA(_instance: usize, file: usize, index: u32) -> usize {
    let file = door_windows_abi::widen(file);
    door_windows_abi::narrow((move || -> u64 {
        read_ansi(file).map_or(0, |p| extract_icon(&p, index as i32))
    })())
}
/// ExtractAssociatedIcon: the file's own icon or its type's; the path
/// buffer (MAX_PATH) and index are updated to the icon's location.
fn associated_icon(path: u64, index: u64, ansi: bool) -> u64 {
    let Ok(text) = opt_text(path, ansi) else {
        return 0;
    };
    let full = absolute(&text, None);
    let source = icon_source(&full, is_directory(&full), false);
    let icon = make_icon(&source, 32);
    if icon != 0 {
        let (file, at) = location(&source);
        let bytes = if ansi {
            ansi_bytes(&file)
        } else {
            wide_bytes(&file)
        };
        let unit = if ansi { 1 } else { 2 };
        if bytes.len() + unit <= 260 * unit {
            let mut b = bytes;
            b.extend(core::iter::repeat_n(0, unit));
            let _ = write(path, &b);
        }
        if index != 0 {
            let _ = write(index, &(at as u16).to_le_bytes());
        }
    }
    icon
}
#[unsafe(no_mangle)]
pub extern "system" fn ExtractAssociatedIconW(
    _instance: usize,
    path: usize,
    index: usize,
) -> usize {
    let path = door_windows_abi::widen(path);
    let index = door_windows_abi::widen(index);
    door_windows_abi::narrow((move || -> u64 { associated_icon(path, index, false) })())
}
#[unsafe(no_mangle)]
pub extern "system" fn ExtractAssociatedIconA(
    _instance: usize,
    path: usize,
    index: usize,
) -> usize {
    let path = door_windows_abi::widen(path);
    let index = door_windows_abi::widen(index);
    door_windows_abi::narrow((move || -> u64 { associated_icon(path, index, true) })())
}
/// SHDefExtractIcon: icons from a location; `size` LOWORD large, HIWORD
/// small.
#[unsafe(no_mangle)]
pub extern "system" fn SHDefExtractIconW(
    location: usize,
    index: i32,
    _flags: u32,
    large: usize,
    small: usize,
    size: u32,
) -> u32 {
    let location = door_windows_abi::widen(location);
    let large = door_windows_abi::widen(large);
    let small = door_windows_abi::widen(small);
    {
        let Ok(path) = read_text(location) else {
            return 0x8007_0057;
        };
        let w = wide(&path);
        let mut ok = true;
        for (out, px) in [(large, size & 0xffff), (small, size >> 16)] {
            if out == 0 {
                continue;
            }
            let px = if px == 0 { 32 } else { px as i32 };
            let n = unsafe {
                PrivateExtractIconsW(
                    door_windows_abi::narrow(w.as_ptr() as u64),
                    index,
                    px,
                    px,
                    door_windows_abi::narrow(out),
                    0,
                    1,
                    0,
                )
            };
            ok &= n == 1;
        }
        if ok { 0 } else { 1 }
    }
}
/// SHGetStockIconInfo: SHSTOCKICONINFO {cbSize 544, hIcon at 8,
/// iSysImageIndex 16, iIcon 20, szPath at 24}.
#[unsafe(no_mangle)]
pub extern "system" fn SHGetStockIconInfo(id: u32, flags: u32, info: usize) -> u32 {
    let info = door_windows_abi::widen(info);
    {
        const SHGSI_ICON: u32 = 0x100;
        const SHGSI_SYSICONINDEX: u32 = 0x4000;
        const SHGSI_SMALLICON: u32 = 0x1;
        if read_u32(info).ok() != Some(544) {
            return 0x8007_0057;
        }
        let Some(index) = door_windows_abi::shell_icons::stock(id) else {
            return 0x8007_0057;
        };
        let source = IconSource::Shell(index);
        let mut b = vec![0u8; 544];
        b[..4].copy_from_slice(&544u32.to_le_bytes());
        let (path, at) = location(&source);
        b[20..24].copy_from_slice(&at.to_le_bytes());
        let units = wide_bytes(&path);
        b[24..24 + units.len().min(518)].copy_from_slice(&units[..units.len().min(518)]);
        if flags & SHGSI_SYSICONINDEX != 0 {
            b[16..20].copy_from_slice(&system_index(&source).to_le_bytes());
        }
        if flags & SHGSI_ICON != 0 {
            let icon = make_icon(&source, if flags & SHGSI_SMALLICON != 0 { 16 } else { 32 });
            if icon == 0 {
                return 0x8000_4005;
            }
            b[8..16].copy_from_slice(&icon.to_le_bytes());
        }
        if write(info, &b).is_err() {
            return 0x8000_4003;
        }
        0
    }
}

// ---- SHBrowseForFolder -----------------------------------------------------------------
// "Browse For Folder": the caller's title, the folders of the current
// location (a list with ".." above them, "This PC" listing the drives),
// the selected path, an optional folder-name box (BIF_EDITBOX) and "Make
// New Folder" (BIF_NEWDIALOGSTYLE), OK and Cancel. The callback gets
// BFFM_INITIALIZED and BFFM_SELCHANGED and may send BFFM_SETSELECTION,
// BFFM_ENABLEOK and BFFM_SETSTATUSTEXT, as on Windows.
const BIF_RETURNONLYFSDIRS: u32 = 0x1;
const BIF_STATUSTEXT: u32 = 0x4;
const BIF_EDITBOX: u32 = 0x10;
const BIF_NEWDIALOGSTYLE: u32 = 0x40;
const BIF_NONEWFOLDERBUTTON: u32 = 0x200;
const BFFM_INITIALIZED: u32 = 1;
const BFFM_SELCHANGED: u32 = 2;
const BFFM_ENABLEOK: u32 = 0x400 + 101;
const BFFM_SETSELECTIONA: u32 = 0x400 + 102;
const BFFM_SETSELECTIONW: u32 = 0x400 + 103;
const BFFM_SETSTATUSTEXTW: u32 = 0x400 + 104;
const BFFM_SETOKTEXT: u32 = 0x400 + 105;
const BFFM_SETEXPANDED: u32 = 0x400 + 106;
const IDC_TITLE: i32 = 0x3742;
const IDC_LIST: i32 = 0x3741;
const IDC_STATUS: i32 = 0x3743;
const IDC_EDIT: i32 = 0x3744;
const IDC_NEW: i32 = 0x3746;
const THIS_PC: &str = "This PC";

struct Browse {
    flags: u32,
    title: String,
    callback: u64,
    data: i64,
    /// The folder listed ("" is This PC) and the one selected.
    folder: String,
    selected: Option<String>,
    entries: Vec<String>,
    chosen: Option<String>,
}

fn dialog_header(b: &mut Vec<u8>, style: u32, count: u16, cx: i16, cy: i16, title: &str) {
    for v in [1u16, 0xffff] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for v in [0u32, 0, style] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&count.to_le_bytes());
    for v in [0i16, 0, cx, cy] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&[0, 0, 0, 0]);
    for u in title.encode_utf16().chain(core::iter::once(0)) {
        b.extend_from_slice(&u.to_le_bytes());
    }
    // DS_SETFONT: 9 pt Segoe UI.
    b.extend_from_slice(&9u16.to_le_bytes());
    b.extend_from_slice(&400u16.to_le_bytes());
    b.extend_from_slice(&[0, 1]);
    for u in "Segoe UI\0".encode_utf16() {
        b.extend_from_slice(&u.to_le_bytes());
    }
}
fn dialog_item(b: &mut Vec<u8>, style: u32, r: [i16; 4], id: i32, class: u16, text: &str) {
    while !b.len().is_multiple_of(4) {
        b.push(0);
    }
    // WS_CHILD | WS_VISIBLE
    for v in [0u32, 0, style | 0x5000_0000] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for v in r {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&id.to_le_bytes());
    b.extend_from_slice(&[0xff, 0xff]);
    b.extend_from_slice(&class.to_le_bytes());
    for u in text.encode_utf16().chain(core::iter::once(0)) {
        b.extend_from_slice(&u.to_le_bytes());
    }
    b.extend_from_slice(&[0, 0]);
}
fn browse_template(flags: u32) -> Vec<u8> {
    let edit = flags & BIF_EDITBOX != 0;
    let new_folder = flags & BIF_NEWDIALOGSTYLE != 0 && flags & BIF_NONEWFOLDERBUTTON == 0;
    let status = flags & BIF_STATUSTEXT != 0;
    let count = 5 + u16::from(edit) * 2 + u16::from(new_folder);
    let height = 212 + if edit { 18 } else { 0 };
    let mut b = Vec::new();
    // WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME | DS_SETFONT
    dialog_header(
        &mut b,
        0x80c8_0080 | 0x40,
        count,
        228,
        height,
        "Browse For Folder",
    );
    dialog_item(&mut b, 0, [8, 6, 212, 20], IDC_TITLE, 0x82, "");
    // SS_PATHELLIPSIS
    let _ = status;
    dialog_item(&mut b, 0x8000, [8, 28, 212, 9], IDC_STATUS, 0x82, "");
    // LBS_NOTIFY | WS_VSCROLL | WS_BORDER | WS_TABSTOP
    dialog_item(
        &mut b,
        0x0001 | 0x0020_0000 | 0x0080_0000 | 0x0001_0000,
        [8, 40, 212, 140],
        IDC_LIST,
        0x83,
        "",
    );
    let mut y = 184;
    if edit {
        dialog_item(&mut b, 0, [8, y + 2, 30, 9], 0xffff, 0x82, "Fold&er:");
        // ES_AUTOHSCROLL | WS_BORDER | WS_TABSTOP
        dialog_item(
            &mut b,
            0x0080 | 0x0080_0000 | 0x0001_0000,
            [40, y, 180, 13],
            IDC_EDIT,
            0x81,
            "",
        );
        y += 18;
    }
    if new_folder {
        dialog_item(
            &mut b,
            0x0001_0000,
            [8, y + 4, 74, 15],
            IDC_NEW,
            0x80,
            "&Make New Folder",
        );
    }
    // BS_DEFPUSHBUTTON | WS_TABSTOP
    dialog_item(
        &mut b,
        0x0001 | 0x0001_0000,
        [112, y + 4, 52, 15],
        1,
        0x80,
        "OK",
    );
    dialog_item(&mut b, 0x0001_0000, [168, y + 4, 52, 15], 2, 0x80, "Cancel");
    b
}
/// Child folders of `folder` ("" lists the drives), sorted.
fn child_folders(folder: &str) -> Vec<String> {
    if folder.is_empty() {
        return (b'C'..=b'Z')
            .map(|d| alloc::format!("{}:\\", d as char))
            .filter(|d| is_directory(d))
            .collect();
    }
    let pattern = alloc::format!("{}\\*", folder.trim_end_matches('\\'));
    let mut names: Vec<String> = entries(&pattern)
        .into_iter()
        .filter(|(name, directory)| *directory && name != "." && name != "..")
        .map(|(name, _)| name)
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}
fn browse_state(hwnd: u64) -> *mut Browse {
    unsafe { GetWindowLongPtrW(door_windows_abi::narrow(hwnd), 16) as *mut Browse }
}
/// Show `folder`'s children; the folder itself becomes the selection.
fn browse_list(hwnd: u64, b: &mut Browse, folder: String) {
    const LB_RESETCONTENT: u32 = 0x0184;
    const LB_ADDSTRING: u32 = 0x0180;
    let list = unsafe { GetDlgItem(door_windows_abi::narrow(hwnd), IDC_LIST) };
    unsafe { SendMessageW(list, LB_RESETCONTENT, 0, 0) };
    b.entries.clear();
    if !folder.is_empty() {
        b.entries.push(String::from(".."));
    }
    b.entries.extend(child_folders(&folder));
    for e in &b.entries {
        let w = wide(e);
        unsafe { SendMessageW(list, LB_ADDSTRING, 0, (w.as_ptr() as i64) as isize) };
    }
    b.folder = folder;
    let selected = (!b.folder.is_empty()).then(|| b.folder.clone());
    browse_select(hwnd, b, selected);
}
fn browse_select(hwnd: u64, b: &mut Browse, path: Option<String>) {
    let shown = path.clone().unwrap_or_else(|| String::from(THIS_PC));
    let w = wide(&shown);
    unsafe {
        SetDlgItemTextW(
            door_windows_abi::narrow(hwnd),
            IDC_STATUS,
            door_windows_abi::narrow(w.as_ptr() as u64),
        )
    };
    if b.flags & BIF_EDITBOX != 0 {
        let name = path
            .as_deref()
            .map(|p| leaf(p.trim_end_matches('\\')))
            .unwrap_or("");
        let w = wide(name);
        unsafe {
            SetDlgItemTextW(
                door_windows_abi::narrow(hwnd),
                IDC_EDIT,
                door_windows_abi::narrow(w.as_ptr() as u64),
            )
        };
    }
    // OK needs a file-system folder (This PC is not one).
    let ok = unsafe { GetDlgItem(door_windows_abi::narrow(hwnd), 1) };
    unsafe {
        EnableWindow(
            ok,
            i32::from(path.is_some() || b.flags & BIF_RETURNONLYFSDIRS == 0),
        )
    };
    b.selected = path;
    if b.callback != 0 {
        let list = b
            .selected
            .as_deref()
            .and_then(|p| pidl::from_path(p, true))
            .and_then(|l| task_copy(&l).ok())
            .unwrap_or(0);
        browse_callback(hwnd, b, BFFM_SELCHANGED, list as i64);
        if list != 0 {
            ILFree(door_windows_abi::narrow(list));
        }
    }
}
fn browse_callback(hwnd: u64, b: &Browse, message: u32, lparam: i64) {
    // SAFETY: the application's BFFCALLBACK, called on its own thread as
    // Windows does.
    // BFFCALLBACK(hwnd, uMsg, lParam, lpData), pointer-sized (P17).
    let f: extern "system" fn(usize, u32, isize, isize) -> i32 =
        unsafe { core::mem::transmute(door_windows_abi::narrow(b.callback)) };
    f(
        door_windows_abi::narrow(hwnd),
        message,
        lparam as isize,
        b.data as isize,
    );
}
extern "system" fn browse_proc(hwnd: usize, message: u32, wparam: usize, lparam: isize) -> isize {
    let hwnd = door_windows_abi::widen(hwnd);
    let wparam = door_windows_abi::widen(wparam);
    let lparam = lparam as i64;
    ((move || -> i64 {
        const WM_INITDIALOG: u32 = 0x0110;
        const WM_COMMAND: u32 = 0x0111;
        const LBN_SELCHANGE: u32 = 1;
        const LBN_DBLCLK: u32 = 2;
        const LB_GETCURSEL: u32 = 0x0188;
        if message == WM_INITDIALOG {
            unsafe { SetWindowLongPtrW(door_windows_abi::narrow(hwnd), 16, (lparam) as isize) };
            // SAFETY: the Browse lives across the modal call.
            let b = unsafe { &mut *(lparam as *mut Browse) };
            let w = wide(&b.title);
            unsafe {
                SetDlgItemTextW(
                    door_windows_abi::narrow(hwnd),
                    IDC_TITLE,
                    door_windows_abi::narrow(w.as_ptr() as u64),
                )
            };
            let start = current_directory();
            browse_list(
                hwnd,
                b,
                if is_directory(&start) {
                    start
                } else {
                    String::new()
                },
            );
            if b.callback != 0 {
                browse_callback(hwnd, b, BFFM_INITIALIZED, 0);
            }
            return 1;
        }
        let state = browse_state(hwnd);
        if state.is_null() {
            return 0;
        }
        // SAFETY: set at WM_INITDIALOG; the dialog is modal on this thread.
        let b = unsafe { &mut *state };
        match message {
            BFFM_SETSELECTIONW | BFFM_SETSELECTIONA | BFFM_SETEXPANDED => {
                // wParam TRUE: lParam is a path; FALSE: an item ID list.
                let path = if wparam != 0 {
                    if message == BFFM_SETSELECTIONA {
                        read_ansi(lparam as u64).ok()
                    } else {
                        read_text(lparam as u64).ok()
                    }
                } else {
                    list_path(lparam as u64)
                };
                if let Some(path) = path.map(|p| absolute(&p, None)).filter(|p| is_directory(p)) {
                    browse_list(hwnd, b, path);
                }
                1
            }
            BFFM_ENABLEOK => {
                let ok = unsafe { GetDlgItem(door_windows_abi::narrow(hwnd), 1) };
                unsafe { EnableWindow(ok, i32::from(lparam != 0)) };
                1
            }
            BFFM_SETSTATUSTEXTW => {
                unsafe {
                    SetDlgItemTextW(
                        door_windows_abi::narrow(hwnd),
                        IDC_STATUS,
                        door_windows_abi::narrow(lparam as u64),
                    )
                };
                1
            }
            BFFM_SETOKTEXT => {
                unsafe {
                    SetDlgItemTextW(
                        door_windows_abi::narrow(hwnd),
                        1,
                        door_windows_abi::narrow(lparam as u64),
                    )
                };
                1
            }
            WM_COMMAND => {
                let (id, code) = ((wparam & 0xffff) as i32, (wparam >> 16) as u32);
                match (id, code) {
                    (IDC_LIST, LBN_SELCHANGE | LBN_DBLCLK) => {
                        let list = unsafe { GetDlgItem(door_windows_abi::narrow(hwnd), IDC_LIST) };
                        let index = unsafe { SendMessageW(list, LB_GETCURSEL, 0, 0) };
                        let Some(name) = usize::try_from(index)
                            .ok()
                            .and_then(|i| b.entries.get(i))
                            .cloned()
                        else {
                            return 1;
                        };
                        let target = if name == ".." {
                            let trimmed = b.folder.trim_end_matches('\\');
                            match trimmed.rfind('\\') {
                                Some(i) if i > 2 => String::from(&trimmed[..i]),
                                Some(_) if trimmed.len() > 3 => String::from(&trimmed[..3]),
                                _ => String::new(),
                            }
                        } else if b.folder.is_empty() {
                            name.clone()
                        } else {
                            alloc::format!("{}\\{}", b.folder.trim_end_matches('\\'), name)
                        };
                        if code == LBN_DBLCLK {
                            browse_list(hwnd, b, target);
                        } else if name == ".." {
                            browse_select(
                                hwnd,
                                b,
                                (!b.folder.is_empty()).then(|| b.folder.clone()),
                            );
                        } else {
                            browse_select(hwnd, b, Some(target));
                        }
                        1
                    }
                    (IDC_NEW, _) if !b.folder.is_empty() => {
                        // "New folder", "New folder (2)", ... as Explorer names it.
                        let base = b.folder.trim_end_matches('\\');
                        let name = (1..100)
                            .map(|n| {
                                if n == 1 {
                                    String::from("New folder")
                                } else {
                                    alloc::format!("New folder ({n})")
                                }
                            })
                            .find(|n| attributes(&alloc::format!("{base}\\{n}")).is_none());
                        if let Some(name) = name {
                            let path = alloc::format!("{base}\\{name}");
                            let w = wide(&path);
                            if unsafe { DoorCreateDirectoryW(w.as_ptr() as u64) } != 0 {
                                let folder = b.folder.clone();
                                browse_list(hwnd, b, folder);
                                browse_select(hwnd, b, Some(path));
                            }
                        }
                        1
                    }
                    (1, _) => {
                        let mut chosen = b.selected.clone();
                        if b.flags & BIF_EDITBOX != 0 {
                            let mut text = vec![0u16; 260];
                            let n = unsafe {
                                GetDlgItemTextW(
                                    door_windows_abi::narrow(hwnd),
                                    IDC_EDIT,
                                    door_windows_abi::narrow(text.as_mut_ptr() as u64),
                                    260,
                                )
                            } as usize;
                            let typed = String::from_utf16_lossy(&text[..n.min(260)]);
                            let typed = typed.trim();
                            if !typed.is_empty() {
                                let base = if b.folder.is_empty() {
                                    None
                                } else {
                                    Some(b.folder.as_str())
                                };
                                let candidate = absolute(typed, base);
                                let current_name =
                                    chosen.as_deref().map(|p| leaf(p.trim_end_matches('\\')));
                                if current_name != Some(typed) {
                                    chosen = Some(candidate);
                                }
                            }
                        }
                        if let Some(path) = chosen.filter(|p| is_directory(p)) {
                            b.chosen = Some(path);
                            unsafe { EndDialog(door_windows_abi::narrow(hwnd), 1) };
                        }
                        1
                    }
                    (2, _) => {
                        unsafe { EndDialog(door_windows_abi::narrow(hwnd), 0) };
                        1
                    }
                    _ => 0,
                }
            }
            _ => 0,
        }
    })()) as isize
}
/// BROWSEINFO (64 bytes): hwndOwner, pidlRoot 8, pszDisplayName 16,
/// lpszTitle 24, ulFlags 32, lpfn 40, lParam 48, iImage 56.
fn browse_for_folder(info: u64, ansi: bool) -> u64 {
    use shape::BROWSEINFO as B;
    let mut raw = [0u8; shape::layout::size(&B)];
    if read(info, &mut raw).is_err() {
        return 0;
    }
    // By BROWSEINFO's x64 offsets (0 hwndOwner, 8 pidlRoot, 16
    // pszDisplayName, 24 lpszTitle, 32 ulFlags, 40 lpfn, 48 lParam), at the
    // build's width.
    let field = |at: usize| shape::layout::get(&raw, &B, at / 8);
    let flags = shape::layout::get(&raw, &B, 4) as u32;
    let title = opt_text(field(24), ansi).unwrap_or_default();
    let mut state = Browse {
        flags,
        title,
        callback: field(40),
        data: field(48) as i64,
        folder: String::new(),
        selected: None,
        entries: Vec::new(),
        chosen: None,
    };
    let template = browse_template(flags);
    // SAFETY: the template and state outlive the modal call.
    let result = unsafe {
        DialogBoxIndirectParamW(
            0,
            door_windows_abi::narrow(template.as_ptr() as u64),
            door_windows_abi::narrow(field(0)),
            door_windows_abi::narrow(browse_proc as *const () as u64),
            (&mut state as *mut Browse as i64) as isize,
        )
    };
    let Some(path) = state.chosen.filter(|_| result == 1) else {
        return 0;
    };
    // pszDisplayName (MAX_PATH): the folder's display name.
    let display = field(16);
    if display != 0 {
        let name = leaf(path.trim_end_matches('\\'));
        let mut bytes = if ansi {
            ansi_bytes(name)
        } else {
            wide_bytes(name)
        };
        bytes.extend_from_slice(if ansi { &[0] } else { &[0, 0] });
        let _ = write(display, &bytes);
    }
    pidl::from_path(&path, true)
        .and_then(|l| task_copy(&l).ok())
        .unwrap_or(0)
}
#[unsafe(no_mangle)]
pub extern "system" fn SHBrowseForFolderW(info: usize) -> usize {
    let info = door_windows_abi::widen(info);
    door_windows_abi::narrow((move || -> u64 { browse_for_folder(info, false) })())
}
#[unsafe(no_mangle)]
pub extern "system" fn SHBrowseForFolderA(info: usize) -> usize {
    let info = door_windows_abi::widen(info);
    door_windows_abi::narrow((move || -> u64 { browse_for_folder(info, true) })())
}

fn recycle_storage() -> Option<door_service_client::Storage> {
    let registry =
        unsafe { DoorGetInheritedHandle(door_abi::platform::inherited::REGISTRY as u32) };
    door_service_client::Storage::connect(registry).ok()
}
/// `SHQueryRecycleBinW`: the caller's bin (every drive is C:), into
/// SHQUERYRBINFO (cbSize u32, i64Size at 8, i64NumItems at 16).
#[unsafe(no_mangle)]
pub extern "system" fn SHQueryRecycleBinW(_root: usize, info: usize) -> u32 {
    let info = door_windows_abi::widen(info);
    {
        let Ok(size) = read_u32(info) else {
            return hr::E_INVALIDARG;
        };
        if size < 24 {
            return hr::E_INVALIDARG;
        }
        let Some(usage) = recycle_storage().and_then(|mut s| s.recycle_usage().ok()) else {
            return hr::E_FAIL;
        };
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&usage.bytes.to_le_bytes());
        out[8..].copy_from_slice(&u64::from(usage.items).to_le_bytes());
        if write(info + 8, &out).is_err() {
            return hr::E_INVALIDARG;
        }
        hr::S_OK
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SHQueryRecycleBinA(root: usize, info: usize) -> u32 {
    let root = door_windows_abi::widen(root);
    let info = door_windows_abi::widen(info);
    {
        let _ = root;
        SHQueryRecycleBinW(0, door_windows_abi::narrow(info))
    }
}
/// `SHEmptyRecycleBinW`: SHERB_NOCONFIRMATION (1) skips Windows' "Are you
/// sure you want to permanently delete these N items?".
#[unsafe(no_mangle)]
pub extern "system" fn SHEmptyRecycleBinW(_window: usize, _root: usize, flags: u32) -> u32 {
    {
        let Some(mut storage) = recycle_storage() else {
            return hr::E_FAIL;
        };
        let items = storage.recycle_usage().map_or(0, |u| u.items);
        if items == 0 {
            return hr::S_OK;
        }
        if flags & 1 == 0 {
            let text = if items == 1 {
                alloc::string::String::from(
                    "Are you sure you want to permanently delete this item?",
                )
            } else {
                alloc::format!("Are you sure you want to permanently delete these {items} items?")
            };
            if !confirm(&text, "Delete Multiple Items") {
                return 0x8007_04C7; // HRESULT_FROM_WIN32(ERROR_CANCELLED)
            }
        }
        match storage.recycle_empty() {
            Ok(_) => hr::S_OK,
            Err(_) => hr::E_FAIL,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SHEmptyRecycleBinA(window: usize, root: usize, flags: u32) -> u32 {
    let window = door_windows_abi::widen(window);
    let root = door_windows_abi::widen(root);
    {
        let _ = root;
        SHEmptyRecycleBinW(door_windows_abi::narrow(window), 0, flags)
    }
}

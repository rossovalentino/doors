//! The machine an installation in this process changes: files through
//! ntdll (Windows paths, the process's token decides what it may write),
//! the registry and services through advapi32, programs and custom action
//! DLLs through ntdll, the volume's free space and file times through the
//! storage service.
use super::*;
use door_msi::execute::Target;
use door_msi::install::{RegValue, Root, ServicePlan};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorCreateDirectoryW(path: u64) -> i32;
    fn DoorRemoveDirectoryW(path: u64) -> i32;
    fn DoorDeleteFileW(path: u64) -> i32;
    fn DoorMoveFileExW(existing: u64, new: u64, flags: u32) -> i32;
    fn DoorGetFileAttributesW(path: u64) -> u32;
    fn DoorGetFileAttributesExW(path: u64, level: u32, out: u64) -> i32;
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
    fn DoorWriteFile(handle: u64, buffer: u64, bytes: u32, written: u64, overlapped: u64) -> i32;
    fn DoorGetFileSizeEx(handle: u64, destination: u64) -> i32;
    fn DoorSetFilePointerEx(handle: u64, distance: i64, new: u64, method: u32) -> i32;
    fn DoorFlushFileBuffers(handle: u64) -> i32;
    fn DoorCloseFile(handle: u64) -> i32;
    fn DoorFindFirstFileW(pattern: u64, destination: u64) -> u64;
    fn DoorFindNextFileW(handle: u64, destination: u64) -> i32;
    fn DoorFindClose(handle: u64) -> i32;
    fn DoorCreateProcessW(
        application: u64,
        command_line: u64,
        directory: u64,
        flags: u32,
        info: u64,
        environment: u64,
    ) -> i32;
    fn DoorWaitForSingleObjectEx(handle: u64, milliseconds: u32, alertable: i32) -> u32;
    fn DoorGetExitCodeProcess(handle: u64, code: u64) -> i32;
    fn DoorCloseHandle(handle: u64) -> i32;
    fn DoorLoadLibraryW(name: u64) -> u64;
    fn DoorGetProcAddress(module: u64, name: u64) -> u64;
    fn DoorFreeLibrary(module: u64) -> i32;
    fn DoorGetTickCount64() -> u64;
    fn DoorGetNativePath(path: u64, out: u64, capacity: u32) -> u32;
    fn DoorGetInheritedHandle(index: u32) -> u64;
}
#[link(name = "advapi32", kind = "dylib")]
unsafe extern "system" {
    fn RegCreateKeyExW(
        key: usize,
        sub: usize,
        reserved: u32,
        class: usize,
        options: u32,
        desired: u32,
        security: usize,
        out: usize,
        disposition: usize,
    ) -> u32;
    fn RegOpenKeyExW(key: usize, sub: usize, options: u32, desired: u32, out: usize) -> u32;
    fn RegSetValueExW(
        key: usize,
        name: usize,
        reserved: u32,
        kind: u32,
        data: usize,
        size: u32,
    ) -> u32;
    fn RegQueryValueExW(
        key: usize,
        name: usize,
        reserved: usize,
        kind: usize,
        data: usize,
        size: usize,
    ) -> u32;
    fn RegDeleteValueW(key: usize, name: usize) -> u32;
    fn RegDeleteTreeW(key: usize, sub: usize) -> u32;
    fn RegDeleteKeyExW(key: usize, sub: usize, desired: u32, reserved: u32) -> u32;
    fn RegEnumKeyExW(
        key: usize,
        index: u32,
        name: usize,
        length: usize,
        reserved: usize,
        class: usize,
        class_length: usize,
        time: usize,
    ) -> u32;
    fn RegEnumValueW(
        key: usize,
        index: u32,
        name: usize,
        length: usize,
        reserved: usize,
        kind: usize,
        data: usize,
        size: usize,
    ) -> u32;
    fn RegCloseKey(key: usize) -> u32;
    fn CheckTokenMembership(token: usize, sid: usize, out: usize) -> i32;
    fn OpenSCManagerW(machine: usize, database: usize, access: u32) -> usize;
    fn CreateServiceW(
        manager: usize,
        name: usize,
        display: usize,
        access: u32,
        kind: u32,
        start: u32,
        error: u32,
        path: usize,
        group: usize,
        tag: usize,
        dependencies: usize,
        account: usize,
        password: usize,
    ) -> usize;
    fn OpenServiceW(manager: usize, name: usize, access: u32) -> usize;
    fn DeleteService(service: usize) -> i32;
    fn StartServiceW(service: usize, count: u32, args: usize) -> i32;
    fn ControlService(service: usize, control: u32, status: usize) -> i32;
    fn QueryServiceStatus(service: usize, status: usize) -> i32;
    fn ChangeServiceConfig2W(service: usize, level: u32, info: usize) -> i32;
    fn CloseServiceHandle(handle: usize) -> i32;
}
#[link(name = "user32", kind = "dylib")]
unsafe extern "system" {
    fn SendMessageTimeoutW(
        window: usize,
        message: u32,
        wparam: usize,
        lparam: usize,
        flags: u32,
        timeout: u32,
        result: usize,
    ) -> usize;
}

const INVALID: u64 = u64::MAX;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const OPEN_ALWAYS: u32 = 4;
const FILE_SHARE_READ: u32 = 1;

/// msiexec.exe's private switch: register (`r`) or unregister (`u`) a type
/// library for the machine (`m`) or the user (`u`) in this process's
/// registry view, for an installation of the other bitness.
pub(crate) const TYPELIB_SWITCH: &str = "/DoorsTypeLib";
/// A system DLL's module (loaded on first use).
fn system_dll(name: &str) -> Result<u64, u32> {
    let w = utf16z(name);
    match unsafe { DoorLoadLibraryW(w.as_ptr() as u64) } {
        0 => Err(0x8007_0000 | last()),
        m => Ok(m),
    }
}
/// An export of `module` by name.
fn proc(module: u64, name: &str) -> Result<usize, u32> {
    let mut n = Vec::from(name.as_bytes());
    n.push(0);
    match unsafe { DoorGetProcAddress(module, n.as_ptr() as u64) } {
        0 => Err(0x8007_007f),
        f => Ok(f as usize),
    }
}
/// DllRegisterServer (`register`) or DllUnregisterServer of a loaded
/// module, with COM initialized on this thread as regsvr32 does.
pub(crate) fn call_registration(module: u64, register: bool) -> Result<(), u32> {
    let entry = proc(
        module,
        if register {
            "DllRegisterServer"
        } else {
            "DllUnregisterServer"
        },
    )?;
    let ole = system_dll("ole32.dll").ok();
    let initialized = ole
        .and_then(|m| proc(m, "CoInitializeEx").ok())
        .map(|f| {
            let f: extern "system" fn(usize, u32) -> u32 = unsafe { core::mem::transmute(f) };
            // COINIT_APARTMENTTHREADED: S_OK or S_FALSE.
            f(0, 2) <= 1
        })
        .unwrap_or(false);
    let f: extern "system" fn() -> u32 = unsafe { core::mem::transmute(entry) };
    let hr = f();
    if initialized && let Some(u) = ole.and_then(|m| proc(m, "CoUninitialize").ok()) {
        let u: extern "system" fn() = unsafe { core::mem::transmute(u) };
        u();
    }
    if (hr as i32) < 0 { Err(hr) } else { Ok(()) }
}
fn last() -> u32 {
    match unsafe { RtlGetLastWin32Error() } {
        0 => error::FUNCTION_FAILED,
        e => e,
    }
}
fn hkey(root: Root) -> u64 {
    match root.native() {
        Root::ClassesRoot => 0x8000_0000,
        Root::CurrentUser => 0x8000_0001,
        Root::Users => 0x8000_0003,
        _ => 0x8000_0002,
    }
}
/// The registry view a root names, explicitly (KEY_WOW64_32KEY or
/// KEY_WOW64_64KEY): the same keys from the 64-bit and the 32-bit
/// msiexec.exe.
fn view(root: Root) -> u32 {
    if root.view32() {
        door_windows_abi::wow64::KEY_WOW64_32KEY
    } else {
        door_windows_abi::wow64::KEY_WOW64_64KEY
    }
}

/// This process's machine, with the installation's log.
pub struct Machine {
    pub log: Option<Vec<String>>,
    /// The rollback script, held open (and unshared) while the
    /// installation runs: another session sees it in use.
    journal: Option<(String, u64)>,
}
impl Machine {
    pub fn new() -> Self {
        Self {
            log: None,
            journal: None,
        }
    }
    fn open(&mut self, path: &str, access: u32, disposition: u32, share: u32) -> Result<u64, u32> {
        let w = utf16z(path);
        let h =
            unsafe { DoorCreateFileW(w.as_ptr() as u64, access, share, 0, disposition, 0x80, 0) };
        if h == INVALID || h == 0 {
            return Err(last());
        }
        Ok(h)
    }
    fn write_handle(h: u64, data: &[u8]) -> Result<(), u32> {
        for chunk in data.chunks(1 << 20) {
            let mut done = 0u32;
            let ok = unsafe {
                DoorWriteFile(
                    h,
                    chunk.as_ptr() as u64,
                    chunk.len() as u32,
                    &mut done as *mut u32 as u64,
                    0,
                )
            };
            if ok == 0 || done as usize != chunk.len() {
                return Err(last());
            }
        }
        Ok(())
    }
    fn key(root: Root, key: &str, write: bool) -> Option<u64> {
        let w = utf16z(key);
        let mut out = 0u64;
        let r = if write {
            unsafe {
                RegCreateKeyExW(
                    door_windows_abi::narrow(hkey(root)),
                    door_windows_abi::narrow(w.as_ptr() as u64),
                    0,
                    0,
                    0,
                    0xf003f | view(root),
                    0,
                    door_windows_abi::narrow(&mut out as *mut u64 as u64),
                    0,
                )
            }
        } else {
            unsafe {
                RegOpenKeyExW(
                    door_windows_abi::narrow(hkey(root)),
                    door_windows_abi::narrow(w.as_ptr() as u64),
                    0,
                    0x20019 | view(root),
                    door_windows_abi::narrow(&mut out as *mut u64 as u64),
                )
            }
        };
        (r == 0).then_some(out)
    }
    /// Release the rollback script.
    pub fn release(&mut self) {
        if let Some((_, h)) = self.journal.take() {
            unsafe { DoorCloseFile(h) };
        }
    }
}
impl Drop for Machine {
    fn drop(&mut self) {
        self.release();
    }
}

fn decode_value(kind: u32, data: &[u8]) -> Option<RegValue> {
    let text = |d: &[u8]| {
        let u: Vec<u16> = d
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let end = u.iter().position(|x| *x == 0).unwrap_or(u.len());
        String::from_utf16_lossy(&u[..end])
    };
    Some(match kind {
        1 => RegValue::String(text(data)),
        2 => RegValue::Expand(text(data)),
        4 => RegValue::Dword(u32::from_le_bytes(data.get(..4)?.try_into().ok()?)),
        3 | 0 => RegValue::Binary(data.to_vec()),
        7 => {
            let u: Vec<u16> = data
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            RegValue::Multi(
                u.split(|x| *x == 0)
                    .filter(|s| !s.is_empty())
                    .map(String::from_utf16_lossy)
                    .collect(),
            )
        }
        _ => return None,
    })
}
fn encode_value(v: &RegValue) -> (u32, Vec<u8>) {
    let wide = |s: &str| -> Vec<u8> { utf16z(s).iter().flat_map(|u| u.to_le_bytes()).collect() };
    match v {
        RegValue::String(s) => (1, wide(s)),
        RegValue::Expand(s) => (2, wide(s)),
        RegValue::Dword(d) => (4, d.to_le_bytes().to_vec()),
        RegValue::Binary(b) => (3, b.clone()),
        RegValue::Multi(m) => {
            let mut out = Vec::new();
            for s in m {
                out.extend(wide(s));
            }
            out.extend_from_slice(&[0, 0]);
            (7, out)
        }
    }
}

impl Target for Machine {
    fn create_folder(&mut self, path: &str) -> Result<(), u32> {
        let w = utf16z(path);
        if unsafe { DoorCreateDirectoryW(w.as_ptr() as u64) } != 0 {
            return Ok(());
        }
        match last() {
            183 => Ok(()),
            e => Err(e),
        }
    }
    fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), u32> {
        let h = self.open(path, GENERIC_WRITE, CREATE_ALWAYS, 0)?;
        let r = Self::write_handle(h, data);
        unsafe { DoorCloseFile(h) };
        r
    }
    fn read_file(&mut self, path: &str) -> Result<Vec<u8>, u32> {
        let h = self.open(path, GENERIC_READ, OPEN_EXISTING, FILE_SHARE_READ)?;
        let mut size = 0u64;
        if unsafe { DoorGetFileSizeEx(h, &mut size as *mut u64 as u64) } == 0 || size > 512 << 20 {
            unsafe { DoorCloseFile(h) };
            return Err(error::NOT_ENOUGH_MEMORY);
        }
        let mut data = vec![0u8; size as usize];
        let mut at = 0usize;
        while at < data.len() {
            let mut done = 0u32;
            let n = (data.len() - at).min(1 << 20);
            let ok = unsafe {
                DoorReadFile(
                    h,
                    data[at..].as_mut_ptr() as u64,
                    n as u32,
                    &mut done as *mut u32 as u64,
                    0,
                )
            };
            if ok == 0 || done == 0 {
                unsafe { DoorCloseFile(h) };
                return Err(last());
            }
            at += done as usize;
        }
        unsafe { DoorCloseFile(h) };
        Ok(data)
    }
    fn exists(&mut self, path: &str) -> bool {
        let w = utf16z(path);
        (unsafe { DoorGetFileAttributesW(w.as_ptr() as u64) }) != u32::MAX
    }
    fn remove_file(&mut self, path: &str) -> Result<(), u32> {
        if self
            .journal
            .as_ref()
            .is_some_and(|(p, _)| p.eq_ignore_ascii_case(path))
        {
            self.release();
        }
        let w = utf16z(path);
        if unsafe { DoorDeleteFileW(w.as_ptr() as u64) } != 0 {
            Ok(())
        } else {
            Err(last())
        }
    }
    fn remove_folder(&mut self, path: &str) -> Result<(), u32> {
        let w = utf16z(path);
        if unsafe { DoorRemoveDirectoryW(w.as_ptr() as u64) } != 0 {
            Ok(())
        } else {
            Err(last())
        }
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), u32> {
        let (a, b) = (utf16z(from), utf16z(to));
        // MOVEFILE_REPLACE_EXISTING | MOVEFILE_COPY_ALLOWED.
        if unsafe { DoorMoveFileExW(a.as_ptr() as u64, b.as_ptr() as u64, 3) } != 0 {
            Ok(())
        } else {
            Err(last())
        }
    }
    fn append_file(&mut self, path: &str, data: &[u8]) -> Result<(), u32> {
        let handle = match &self.journal {
            Some((p, h)) if p.eq_ignore_ascii_case(path) => *h,
            _ => {
                self.release();
                // Unshared: while it is open, another session sees the
                // installation running.
                let h = self.open(path, GENERIC_READ | GENERIC_WRITE, OPEN_ALWAYS, 0)?;
                self.journal = Some((String::from(path), h));
                h
            }
        };
        unsafe { DoorSetFilePointerEx(handle, 0, 0, 2) };
        Self::write_handle(handle, data)?;
        unsafe { DoorFlushFileBuffers(handle) };
        Ok(())
    }
    fn in_use(&mut self, path: &str) -> Option<bool> {
        if self
            .journal
            .as_ref()
            .is_some_and(|(p, _)| p.eq_ignore_ascii_case(path))
        {
            return Some(true);
        }
        match self.open(path, GENERIC_READ | GENERIC_WRITE, OPEN_EXISTING, 0) {
            Ok(h) => {
                unsafe { DoorCloseFile(h) };
                Some(false)
            }
            // ERROR_SHARING_VIOLATION: its installation holds it.
            Err(32) => Some(true),
            Err(_) => Some(false),
        }
    }
    fn list(&mut self, folder: &str) -> Vec<String> {
        let pattern = utf16z(&alloc::format!("{folder}\\*"));
        let mut data = [0u8; 600];
        let h = unsafe { DoorFindFirstFileW(pattern.as_ptr() as u64, data.as_mut_ptr() as u64) };
        let mut out = Vec::new();
        if h == INVALID || h == 0 {
            return out;
        }
        loop {
            let attributes = u32::from_le_bytes(data[..4].try_into().unwrap());
            let name: Vec<u16> = data[44..44 + 520]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|u| *u != 0)
                .collect();
            if attributes & 0x10 == 0 {
                out.push(String::from_utf16_lossy(&name));
            }
            if unsafe { DoorFindNextFileW(h, data.as_mut_ptr() as u64) } == 0 {
                break;
            }
        }
        unsafe { DoorFindClose(h) };
        out
    }
    fn free_bytes(&mut self, _path: &str) -> Option<u64> {
        let registry =
            unsafe { DoorGetInheritedHandle(door_abi::platform::inherited::REGISTRY as u32) };
        let mut storage = door_service_client::Storage::connect(registry).ok()?;
        storage.volume_usage().ok().map(|u| u.free_bytes)
    }
    fn file_times(&mut self, path: &str) -> Option<(u64, u64)> {
        let w = utf16z(path);
        let mut data = [0u8; 36];
        if unsafe { DoorGetFileAttributesExW(w.as_ptr() as u64, 0, data.as_mut_ptr() as u64) } == 0
        {
            return None;
        }
        let t = |at: usize| u64::from_le_bytes(data[at..at + 8].try_into().unwrap()) / 10_000_000;
        Some((t(4), t(20)))
    }
    fn elevated(&mut self) -> bool {
        // BUILTIN\Administrators: S-1-5-32-544.
        let sid: [u8; 16] = [1, 2, 0, 0, 0, 0, 0, 5, 32, 0, 0, 0, 0x20, 2, 0, 0];
        let mut member = 0i32;
        (unsafe {
            CheckTokenMembership(
                0,
                door_windows_abi::narrow(sid.as_ptr() as u64),
                door_windows_abi::narrow(&mut member as *mut i32 as u64),
            )
        }) != 0
            && member != 0
    }
    fn boot(&mut self) -> u64 {
        // When this boot began, to a minute.
        let wall = door_sys::wall_time().unwrap_or(0) / 10_000_000;
        let up = unsafe { DoorGetTickCount64() } / 1000;
        (wall.saturating_sub(up) + 30) / 60
    }
    fn today(&mut self) -> Option<String> {
        let wall = door_sys::wall_time().ok()?;
        let days = (wall / 10_000_000 / 86_400) as i64 - 134_774; // days since 1970-01-01
        // Civil date from days (Howard Hinnant's algorithm).
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };
        Some(alloc::format!("{y:04}{m:02}{d:02}"))
    }
    fn set_value(
        &mut self,
        root: Root,
        key: &str,
        name: Option<&str>,
        value: &RegValue,
    ) -> Result<(), u32> {
        let k = Self::key(root, key, true).ok_or(error::ACCESS_DENIED)?;
        let n = utf16z(name.unwrap_or(""));
        let (kind, data) = encode_value(value);
        let r = unsafe {
            RegSetValueExW(
                door_windows_abi::narrow(k),
                door_windows_abi::narrow(n.as_ptr() as u64),
                0,
                kind,
                door_windows_abi::narrow(data.as_ptr() as u64),
                data.len() as u32,
            )
        };
        unsafe { RegCloseKey(door_windows_abi::narrow(k)) };
        if r == 0 { Ok(()) } else { Err(r) }
    }
    fn create_key(&mut self, root: Root, key: &str) -> Result<(), u32> {
        let k = Self::key(root, key, true).ok_or(error::ACCESS_DENIED)?;
        unsafe { RegCloseKey(door_windows_abi::narrow(k)) };
        Ok(())
    }
    fn value(&mut self, root: Root, key: &str, name: &str) -> Option<RegValue> {
        let k = Self::key(root, key, false)?;
        let n = utf16z(name);
        let mut kind = 0u32;
        let mut size = 0u32;
        let r = unsafe {
            RegQueryValueExW(
                door_windows_abi::narrow(k),
                door_windows_abi::narrow(n.as_ptr() as u64),
                0,
                door_windows_abi::narrow(&mut kind as *mut u32 as u64),
                0,
                door_windows_abi::narrow(&mut size as *mut u32 as u64),
            )
        };
        let out = if r == 0 {
            let mut data = vec![0u8; size as usize];
            let r = unsafe {
                RegQueryValueExW(
                    door_windows_abi::narrow(k),
                    door_windows_abi::narrow(n.as_ptr() as u64),
                    0,
                    door_windows_abi::narrow(&mut kind as *mut u32 as u64),
                    door_windows_abi::narrow(data.as_mut_ptr() as u64),
                    door_windows_abi::narrow(&mut size as *mut u32 as u64),
                )
            };
            if r == 0 {
                decode_value(kind, &data[..size as usize])
            } else {
                None
            }
        } else {
            None
        };
        unsafe { RegCloseKey(door_windows_abi::narrow(k)) };
        out
    }
    fn values(&mut self, root: Root, key: &str) -> Vec<(String, RegValue)> {
        let Some(k) = Self::key(root, key, false) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for index in 0..4096 {
            let mut name = [0u16; 256];
            let mut name_len = name.len() as u32;
            let mut kind = 0u32;
            let mut data = vec![0u8; 4096];
            let mut size = data.len() as u32;
            let r = unsafe {
                RegEnumValueW(
                    door_windows_abi::narrow(k),
                    index,
                    door_windows_abi::narrow(name.as_mut_ptr() as u64),
                    door_windows_abi::narrow(&mut name_len as *mut u32 as u64),
                    0,
                    door_windows_abi::narrow(&mut kind as *mut u32 as u64),
                    door_windows_abi::narrow(data.as_mut_ptr() as u64),
                    door_windows_abi::narrow(&mut size as *mut u32 as u64),
                )
            };
            if r != 0 {
                break;
            }
            let n = String::from_utf16_lossy(&name[..name_len as usize]);
            if let Some(v) = decode_value(kind, &data[..size as usize]) {
                out.push((n, v));
            }
        }
        unsafe { RegCloseKey(door_windows_abi::narrow(k)) };
        out
    }
    fn subkeys(&mut self, root: Root, key: &str) -> Vec<String> {
        let Some(k) = Self::key(root, key, false) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for index in 0..4096 {
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            let r = unsafe {
                RegEnumKeyExW(
                    door_windows_abi::narrow(k),
                    index,
                    door_windows_abi::narrow(name.as_mut_ptr() as u64),
                    door_windows_abi::narrow(&mut len as *mut u32 as u64),
                    0,
                    0,
                    0,
                    0,
                )
            };
            if r != 0 {
                break;
            }
            out.push(String::from_utf16_lossy(&name[..len as usize]));
        }
        unsafe { RegCloseKey(door_windows_abi::narrow(k)) };
        out
    }
    fn delete_value(&mut self, root: Root, key: &str, name: Option<&str>) {
        if let Some(k) = Self::key(root, key, true) {
            let n = utf16z(name.unwrap_or(""));
            unsafe {
                RegDeleteValueW(
                    door_windows_abi::narrow(k),
                    door_windows_abi::narrow(n.as_ptr() as u64),
                )
            };
            unsafe { RegCloseKey(door_windows_abi::narrow(k)) };
        }
    }
    fn delete_key(&mut self, root: Root, key: &str) {
        // Its contents through a handle in the root's view, then the key.
        let w = utf16z(key);
        let mut k = 0u64;
        if unsafe {
            RegOpenKeyExW(
                door_windows_abi::narrow(hkey(root)),
                door_windows_abi::narrow(w.as_ptr() as u64),
                0,
                0xf003f | view(root),
                door_windows_abi::narrow(&mut k as *mut u64 as u64),
            )
        } != 0
        {
            return;
        }
        unsafe {
            RegDeleteTreeW(door_windows_abi::narrow(k), 0);
            RegCloseKey(door_windows_abi::narrow(k));
            RegDeleteKeyExW(
                door_windows_abi::narrow(hkey(root)),
                door_windows_abi::narrow(w.as_ptr() as u64),
                view(root),
                0,
            );
        }
    }
    fn run(&mut self, command_line: &str, directory: &str, wait: bool) -> Result<u32, u32> {
        let mut line = utf16z(command_line);
        let dir = utf16z(directory);
        // PROCESS_INFORMATION { hProcess, hThread, dwProcessId, dwThreadId }
        // at this build's width (ntdll writes the program's).
        use door_windows_abi::layout::{self, Field::*};
        const PROCESS_INFORMATION: [layout::Field; 4] = [Pointer, Pointer, U32, U32];
        let mut info = [0u8; 24];
        // CREATE_NO_WINDOW: custom action programs get no console.
        let ok = unsafe {
            DoorCreateProcessW(
                0,
                line.as_mut_ptr() as u64,
                if directory.is_empty() {
                    0
                } else {
                    dir.as_ptr() as u64
                },
                0x0800_0000,
                info.as_mut_ptr() as u64,
                0,
            )
        };
        if ok == 0 {
            return Err(last());
        }
        let (process, thread) = (
            layout::get(&info, &PROCESS_INFORMATION, 0),
            layout::get(&info, &PROCESS_INFORMATION, 1),
        );
        let mut exit = 0u32;
        if wait {
            unsafe { DoorWaitForSingleObjectEx(process, u32::MAX, 0) };
            unsafe { DoorGetExitCodeProcess(process, &mut exit as *mut u32 as u64) };
        }
        unsafe {
            DoorCloseHandle(thread);
            DoorCloseHandle(process);
        }
        Ok(exit)
    }
    fn register_typelib(
        &mut self,
        path: &str,
        help: &str,
        per_user: bool,
        view32: bool,
        register: bool,
    ) -> Result<(), u32> {
        // A library of the other bitness's component: that msiexec.exe
        // registers it, in its registry view (/DoorsTypeLib).
        if view32 != cfg!(target_arch = "x86") {
            let program = if cfg!(target_arch = "x86") {
                "C:\\Windows\\Sysnative\\msiexec.exe"
            } else {
                "C:\\Windows\\SysWOW64\\msiexec.exe"
            };
            let line = alloc::format!(
                "\"{program}\" {} {} {} \"{path}\" \"{help}\"",
                TYPELIB_SWITCH,
                if register { "r" } else { "u" },
                if per_user { "u" } else { "m" }
            );
            return match self.run(&line, "", true) {
                Ok(0) => Ok(()),
                Ok(code) => Err(code),
                Err(e) => Err(0x8007_0000 | e),
            };
        }
        // oleaut32's own functions, as Windows Installer calls them.
        let oleaut = system_dll("oleaut32.dll")?;
        type Load = extern "system" fn(usize, u32, usize) -> u32;
        type Register = extern "system" fn(usize, usize, usize) -> u32;
        type Unregister = extern "system" fn(usize, u16, u16, u32, u32) -> u32;
        let load: Load = unsafe { core::mem::transmute(proc(oleaut, "LoadTypeLibEx")?) };
        let file = utf16z(path);
        let mut lib = 0usize;
        // REGKIND_NONE.
        let hr = load(file.as_ptr() as usize, 2, &mut lib as *mut usize as usize);
        if hr != 0 || lib == 0 {
            return Err(if hr != 0 { hr } else { 0x8002_801d });
        }
        // ITypeLib: GetLibAttr (7), ReleaseTLibAttr (12), Release (2).
        let vtable = unsafe { *(lib as *const usize) } as *const usize;
        let method = |i: usize| unsafe { *vtable.add(i) };
        let release: extern "system" fn(usize) -> u32 = unsafe { core::mem::transmute(method(2)) };
        let result = if register {
            let f: Register = unsafe {
                core::mem::transmute(proc(
                    oleaut,
                    if per_user {
                        "RegisterTypeLibForUser"
                    } else {
                        "RegisterTypeLib"
                    },
                )?)
            };
            let help = utf16z(help);
            let hr = f(
                lib,
                file.as_ptr() as usize,
                if help.len() > 1 {
                    help.as_ptr() as usize
                } else {
                    0
                },
            );
            if hr == 0 { Ok(()) } else { Err(hr) }
        } else {
            let get: extern "system" fn(usize, usize) -> u32 =
                unsafe { core::mem::transmute(method(7)) };
            let free: extern "system" fn(usize, usize) =
                unsafe { core::mem::transmute(method(12)) };
            let mut attr = 0usize;
            if get(lib, &mut attr as *mut usize as usize) != 0 || attr == 0 {
                Err(0x8002_801d)
            } else {
                // TLIBATTR: guid, lcid, syskind, major, minor.
                let a = attr as *const u8;
                let read32 =
                    |o: usize| unsafe { core::ptr::read_unaligned(a.add(o) as *const u32) };
                let read16 =
                    |o: usize| unsafe { core::ptr::read_unaligned(a.add(o) as *const u16) };
                let (lcid, syskind, major, minor) =
                    (read32(16), read32(20), read16(24), read16(26));
                let f: Unregister = unsafe {
                    core::mem::transmute(proc(
                        oleaut,
                        if per_user {
                            "UnRegisterTypeLibForUser"
                        } else {
                            "UnRegisterTypeLib"
                        },
                    )?)
                };
                let hr = f(attr, major, minor, lcid, syskind);
                free(lib, attr);
                if hr == 0 { Ok(()) } else { Err(hr) }
            }
        };
        release(lib);
        result
    }
    fn self_register(&mut self, path: &str, register: bool) -> Result<(), u32> {
        let w = utf16z(path);
        let module = unsafe { DoorLoadLibraryW(w.as_ptr() as u64) };
        if module == 0 {
            let e = last();
            // A module of the other bitness: that msiexec.exe registers it
            // (/y, /z), as Windows does it in a process of its bitness.
            if e == 193 {
                let program = if cfg!(target_arch = "x86") {
                    "C:\\Windows\\Sysnative\\msiexec.exe"
                } else {
                    "C:\\Windows\\SysWOW64\\msiexec.exe"
                };
                let line = alloc::format!(
                    "\"{program}\" /{} \"{path}\" /qn",
                    if register { "y" } else { "z" }
                );
                return match self.run(&line, "", true) {
                    Ok(0) => Ok(()),
                    Ok(code) => Err(code),
                    Err(e) => Err(0x8007_0000 | e),
                };
            }
            return Err(0x8007_0000 | e);
        }
        let result = call_registration(module, register);
        unsafe { DoorFreeLibrary(module) };
        result
    }
    fn environment_changed(&mut self) {
        // WM_SETTINGCHANGE "Environment" to the top-level windows.
        let text = utf16z("Environment");
        let mut answer = 0usize;
        unsafe {
            SendMessageTimeoutW(
                0xffff,
                0x001a,
                0,
                text.as_ptr() as usize,
                0x0002,
                5000,
                &mut answer as *mut usize as usize,
            )
        };
    }
    fn call_dll(&mut self, path: &str, entry: &str, state: &mut State) -> Result<u32, u32> {
        let w = utf16z(path);
        let module = unsafe { DoorLoadLibraryW(w.as_ptr() as u64) };
        if module == 0 {
            return Err(last());
        }
        let mut name = Vec::from(entry.as_bytes());
        name.push(0);
        let function = unsafe { DoorGetProcAddress(module, name.as_ptr() as u64) };
        if function == 0 {
            unsafe { DoorFreeLibrary(module) };
            return Err(127); // ERROR_PROC_NOT_FOUND
        }
        let handle = insert(Object::Action(state as *mut State));
        // SAFETY: the export is a custom action entry point,
        // `UINT __stdcall Entry(MSIHANDLE)`.
        let entry: extern "system" fn(u32) -> u32 =
            unsafe { core::mem::transmute(function as usize) };
        let result = entry(handle);
        close(handle);
        unsafe { DoorFreeLibrary(module) };
        Ok(result)
    }
    fn install_service(&mut self, s: &ServicePlan) -> Result<(), u32> {
        let manager = unsafe { OpenSCManagerW(0, 0, 0xf003f) };
        if manager == 0 {
            return Err(last());
        }
        let name = utf16z(&s.name);
        let display = utf16z(&s.display_name);
        let path = utf16z(&s.command_line);
        let mut deps: Vec<u16> = Vec::new();
        for d in &s.dependencies {
            deps.extend(utf16z(d));
        }
        deps.push(0);
        let account = utf16z(&s.account);
        let service = unsafe {
            CreateServiceW(
                manager,
                door_windows_abi::narrow(name.as_ptr() as u64),
                door_windows_abi::narrow(display.as_ptr() as u64),
                0xf01ff,
                s.service_type,
                s.start_type,
                s.error_control,
                door_windows_abi::narrow(path.as_ptr() as u64),
                0,
                0,
                if s.dependencies.is_empty() {
                    0
                } else {
                    door_windows_abi::narrow(deps.as_ptr() as u64)
                },
                if s.account.is_empty() {
                    0
                } else {
                    door_windows_abi::narrow(account.as_ptr() as u64)
                },
                0,
            )
        };
        let result = if service == 0 {
            Err(last())
        } else {
            if !s.description.is_empty() {
                let description = utf16z(&s.description);
                let info = [description.as_ptr() as u64];
                unsafe {
                    ChangeServiceConfig2W(
                        service,
                        1,
                        door_windows_abi::narrow(info.as_ptr() as u64),
                    )
                };
            }
            unsafe { CloseServiceHandle(service) };
            Ok(())
        };
        unsafe { CloseServiceHandle(manager) };
        result
    }
    fn delete_service(&mut self, name: &str) -> Result<(), u32> {
        self.with_service(name, |s| {
            if unsafe { DeleteService(door_windows_abi::narrow(s)) } != 0 {
                Ok(())
            } else {
                Err(last())
            }
        })
    }
    fn start_service(&mut self, name: &str, wait: bool) -> Result<(), u32> {
        self.with_service(name, |s| {
            if unsafe { StartServiceW(door_windows_abi::narrow(s), 0, 0) } == 0 {
                let e = last();
                // ERROR_SERVICE_ALREADY_RUNNING.
                return if e == 1056 { Ok(()) } else { Err(e) };
            }
            if wait {
                for _ in 0..300 {
                    let mut status = [0u32; 7];
                    if unsafe {
                        QueryServiceStatus(
                            door_windows_abi::narrow(s),
                            door_windows_abi::narrow(status.as_mut_ptr() as u64),
                        )
                    } != 0
                        && status[1] == 4
                    {
                        break;
                    }
                    unsafe { super::target::sleep(100) };
                }
            }
            Ok(())
        })
    }
    fn stop_service(&mut self, name: &str, wait: bool) -> Result<(), u32> {
        self.with_service(name, |s| {
            let mut status = [0u32; 7];
            unsafe {
                ControlService(
                    door_windows_abi::narrow(s),
                    1,
                    door_windows_abi::narrow(status.as_mut_ptr() as u64),
                )
            };
            if wait {
                for _ in 0..300 {
                    if unsafe {
                        QueryServiceStatus(
                            door_windows_abi::narrow(s),
                            door_windows_abi::narrow(status.as_mut_ptr() as u64),
                        )
                    } != 0
                        && status[1] == 1
                    {
                        break;
                    }
                    unsafe { super::target::sleep(100) };
                }
            }
            Ok(())
        })
    }
    fn service_exists(&mut self, name: &str) -> bool {
        self.with_service(name, |_| Ok(())).is_ok()
    }
    fn log(&mut self, line: &str) {
        if let Some(log) = self.log.as_mut() {
            log.push(String::from(line));
        }
        forward(line);
    }
}
/// The serial log keeps the lines of an installation INSTALL-A checks.
pub(super) fn forward(line: &str) {
    if line.starts_with("DOOR_P16")
        || line.starts_with("Product: ")
        || line.starts_with("Rollback")
        || line.starts_with("Out of disk space")
        || line.starts_with("Custom action ")
        || line.starts_with("Error ")
        || line.starts_with("RemoveExistingProducts")
        || line.starts_with("CaProbe")
    {
        let mut text = String::from(line);
        if !text.starts_with("DOOR_P16") {
            text = alloc::format!("DOOR_P16 MSI {text}");
        }
        text.push('\n');
        let _ = door_sys::write(text.as_bytes());
    }
}
impl Machine {
    fn with_service(
        &mut self,
        name: &str,
        f: impl FnOnce(u64) -> Result<(), u32>,
    ) -> Result<(), u32> {
        let manager = unsafe { OpenSCManagerW(0, 0, 0xf003f) };
        if manager == 0 {
            return Err(last());
        }
        let n = utf16z(name);
        let service = unsafe {
            OpenServiceW(
                manager,
                door_windows_abi::narrow(n.as_ptr() as u64),
                0xf01ff,
            )
        };
        let r = if service == 0 {
            Err(last())
        } else {
            let r = f(door_windows_abi::widen(service));
            unsafe { CloseServiceHandle(service) };
            r
        };
        unsafe { CloseServiceHandle(manager) };
        r
    }
}

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorSleep(milliseconds: u32);
}
pub unsafe fn sleep(ms: u32) {
    unsafe { DoorSleep(ms) }
}
#[allow(dead_code)]
pub fn native(path: &str) -> Option<String> {
    let w = utf16z(path);
    let mut out = [0u8; 1024];
    let n =
        unsafe { DoorGetNativePath(w.as_ptr() as u64, out.as_mut_ptr() as u64, out.len() as u32) }
            as usize;
    (n > 0 && n <= out.len()).then(|| String::from_utf8_lossy(&out[..n]).into_owned())
}

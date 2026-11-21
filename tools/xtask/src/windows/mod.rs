//! Original Windows compatibility artifacts and an independent reference fixture.
pub mod base;
pub mod base_verify;
pub mod build;
pub mod dlls;
pub mod gate;
pub mod gui;
pub mod pe;
pub mod session;
pub const FIXTURE: &str = "windows-basic";
pub const OUTPUT: &str = "WINDOWS_X64_OK relocation=1 invalid_handle=1 invalid_buffer=1 last_error=1 abi=1 zero=1 selector=1\r\n";
pub const EXIT: i32 = 73;
pub const PROFILE: &str = "profiles/hardware/qemu-windows-x64.json";
pub const MODE: &str = "windows-basic";
pub fn is_mode(mode: &str) -> bool {
    matches!(mode, "windows-basic" | "windows-base" | "windows-reopen")
}

pub fn native_path(path: &std::path::Path) -> Result<String, String> {
    Ok(path
        .to_str()
        .ok_or("Windows build path")?
        .trim_start_matches("\\\\?\\")
        .replace('\\', "/"))
}

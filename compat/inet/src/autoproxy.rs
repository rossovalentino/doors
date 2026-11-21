//! Automatic proxy configuration WinHTTP and WinINet share: the user's
//! Internet Settings (HKCU `...\Internet Settings`: ProxyEnable,
//! ProxyServer, ProxyOverride, AutoConfigURL, and "Automatically detect
//! settings" in `Connections\DefaultConnectionSettings`), WPAD discovery
//! by DNS (`wpad.<domain>`, devolving), downloading a proxy auto-config
//! script and running it (door-pac) with the helpers bound to this
//! machine's resolver, address and clock.
use crate::Tolerance;
use crate::exchange::{Exchange, NoCookies, Settings};
use crate::guest::{Deadline, now_unix, resolve};
use crate::proxyconfig::{civil, url_host};
use alloc::{string::String, sync::Arc, vec, vec::Vec};
use core::sync::atomic::AtomicBool;
use door_http::Headers;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorNetStatus(out: u64) -> u32;
    fn DoorNetResolve(name: u64, length: u32, out: u64, max: u32, count: u64, timeout: u32) -> u32;
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
}
const HKEY_CURRENT_USER: u64 = 0x8000_0001;
const KEY_READ: u32 = 0x2_0019;
const SETTINGS: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings";
const CONNECTIONS: &str =
    "Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings\\Connections";

/// The longest script accepted, and how long its download may take.
const MAX_SCRIPT: usize = door_pac::MAX_SCRIPT;
const DOWNLOAD_MILLIS: u32 = 15_000;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
/// One value of an HKCU key: its type and bytes.
fn value(key: &str, name: &str) -> Option<(u32, Vec<u8>)> {
    let mut handle = 0u64;
    let path = wide(key);
    if unsafe {
        RegOpenKeyExW(
            crate::narrow(HKEY_CURRENT_USER),
            crate::narrow(path.as_ptr() as u64),
            0,
            KEY_READ,
            crate::narrow(&mut handle as *mut u64 as u64),
        )
    } != 0
    {
        return None;
    }
    let name = wide(name);
    let mut kind = 0u32;
    let mut data = vec![0u8; 2048];
    let mut size = data.len() as u32;
    let status = unsafe {
        RegQueryValueExW(
            crate::narrow(handle),
            crate::narrow(name.as_ptr() as u64),
            0,
            crate::narrow(&mut kind as *mut u32 as u64),
            crate::narrow(data.as_mut_ptr() as u64),
            crate::narrow(&mut size as *mut u32 as u64),
        )
    };
    unsafe { RegCloseKey(crate::narrow(handle)) };
    if status != 0 {
        return None;
    }
    data.truncate(size as usize);
    Some((kind, data))
}
fn string_value(key: &str, name: &str) -> Option<String> {
    let (kind, data) = value(key, name)?;
    // REG_SZ / REG_EXPAND_SZ, UTF-16 with or without the NUL.
    if kind != 1 && kind != 2 {
        return None;
    }
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|u| *u != 0)
        .collect();
    let text = String::from_utf16_lossy(&units);
    (!text.is_empty()).then_some(text)
}

/// The user's proxy settings (Settings > Network & internet > Proxy, as
/// Internet Options keeps them).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserProxy {
    pub auto_detect: bool,
    pub config_url: Option<String>,
    /// ProxyServer when ProxyEnable is on.
    pub server: Option<String>,
    pub bypass: Option<String>,
}
/// DefaultConnectionSettings' flags (offset 8): PROXY_TYPE_*.
const AUTO_DETECT_FLAG: u32 = 0x8;
pub fn user_proxy() -> UserProxy {
    let enabled = value(SETTINGS, "ProxyEnable")
        .filter(|(kind, data)| *kind == 4 && data.len() >= 4)
        .is_some_and(|(_, d)| u32::from_le_bytes([d[0], d[1], d[2], d[3]]) != 0);
    // A user who never changed it has "Automatically detect settings" on.
    let auto_detect = match value(CONNECTIONS, "DefaultConnectionSettings") {
        Some((3, data)) if data.len() >= 12 => {
            u32::from_le_bytes([data[8], data[9], data[10], data[11]]) & AUTO_DETECT_FLAG != 0
        }
        _ => true,
    };
    UserProxy {
        auto_detect,
        config_url: string_value(SETTINGS, "AutoConfigURL"),
        server: enabled
            .then(|| string_value(SETTINGS, "ProxyServer"))
            .flatten(),
        bypass: enabled
            .then(|| string_value(SETTINGS, "ProxyOverride"))
            .flatten(),
    }
}

/// Why no answer came from automatic configuration (the DLLs map these to
/// ERROR_WINHTTP_* / ERROR_INTERNET_*).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoFailure {
    /// No script was found (WPAD).
    NotDetected,
    /// The script could not be downloaded.
    Download,
    /// The script is not JavaScript, has no entry point, or failed.
    Script,
}

/// A script downloaded over HTTP(S), without a proxy.
pub fn download(url: &str) -> Result<String, AutoFailure> {
    let parsed = door_http::parse_url(url).map_err(|_| AutoFailure::Download)?;
    let mut exchange = Exchange::new(
        String::from("GET"),
        parsed,
        String::from("HTTP/1.1"),
        Headers::default(),
        Arc::new(AtomicBool::new(false)),
    );
    let settings = Settings {
        timeouts: [DOWNLOAD_MILLIS; 4],
        proxy: None,
        agent: String::from("Mozilla/4.0 (compatible; Win32; WinHttp.AutoProxy.1)"),
        follow_redirects: true,
        allow_downgrade: false,
        max_redirects: 10,
        tolerance: Tolerance::default(),
        keep_alive: false,
        decode: false,
    };
    let mut quiet = |_: crate::exchange::Event<'_>| {};
    exchange
        .send(&settings, &mut NoCookies, &[], &mut quiet)
        .map_err(|_| AutoFailure::Download)?;
    exchange
        .receive(&settings, &mut NoCookies, &mut quiet)
        .map_err(|_| AutoFailure::Download)?;
    if exchange.response().map(|r| r.status) != Some(200) {
        return Err(AutoFailure::Download);
    }
    let mut body = Vec::new();
    loop {
        let part = exchange
            .read(16 * 1024, false, DOWNLOAD_MILLIS)
            .map_err(|_| AutoFailure::Download)?;
        if part.is_empty() {
            break;
        }
        body.extend_from_slice(&part);
        if body.len() > MAX_SCRIPT {
            return Err(AutoFailure::Script);
        }
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// The PAC helpers bound to this machine.
pub struct Machine;
impl door_pac::Host for Machine {
    fn resolve(&mut self, host: &str) -> Vec<[u8; 4]> {
        let mut out = [[0u8; 4]; 8];
        let mut count = 0u32;
        if let Some(a) = crate::ipv4_literal(host) {
            return vec![a];
        }
        let ok = unsafe {
            DoorNetResolve(
                host.as_ptr() as u64,
                host.len() as u32,
                out.as_mut_ptr() as u64,
                8,
                &mut count as *mut u32 as u64,
                5_000,
            )
        } == 0;
        if !ok {
            return Vec::new();
        }
        out[..(count as usize).min(8)].to_vec()
    }
    fn my_address(&mut self) -> [u8; 4] {
        let mut status = [0u8; 88];
        if unsafe { DoorNetStatus(status.as_mut_ptr() as u64) } != 0 {
            return [127, 0, 0, 1];
        }
        let a: [u8; 4] = status[8..12].try_into().unwrap_or([0; 4]);
        if a == [0; 4] { [127, 0, 0, 1] } else { a }
    }
    fn now(&mut self, _utc: bool) -> (i32, u32, u32, u32, u32, u32, u32) {
        // The Win32 layer keeps UTC as local time.
        civil(now_unix())
    }
}
/// The script's answer for `url`.
pub fn evaluate(script: &str, url: &str) -> Result<String, AutoFailure> {
    let parsed = door_pac::Script::parse(script).map_err(|_| AutoFailure::Script)?;
    parsed
        .find_proxy(url, &url_host(url), &mut Machine)
        .map_err(|_| AutoFailure::Script)
}

/// WPAD by DNS: `wpad.<suffix>` for the machine's domain, devolving to two
/// labels (never a bare "wpad", as Windows since MS16-077). Doors' adapter
/// gets no domain from DHCP yet, so this answers what `domain` names.
pub fn detect(domain: Option<&str>) -> Result<String, AutoFailure> {
    let Some(mut domain) = domain.filter(|d| d.contains('.') || !d.is_empty()) else {
        return Err(AutoFailure::NotDetected);
    };
    loop {
        let candidate = alloc::format!("wpad.{domain}");
        if resolve(&candidate, Deadline::after(3_000)).is_ok() {
            return Ok(alloc::format!("http://{candidate}/wpad.dat"));
        }
        match domain.split_once('.') {
            Some((_, rest)) if rest.contains('.') => domain = rest,
            _ => return Err(AutoFailure::NotDetected),
        }
    }
}

/// A process-wide memory of downloaded scripts (WinHTTP caches them per
/// session; one process is close enough), kept 10 minutes.
static CACHE: crate::sync::Lock<Vec<(String, String, i64)>> = crate::sync::Lock::new(Vec::new());
const CACHE_SECONDS: i64 = 600;
/// The script at `url`, downloaded or remembered.
pub fn script(url: &str) -> Result<String, AutoFailure> {
    let now = now_unix();
    if let Some(found) = CACHE.with(|c| {
        c.retain(|(_, _, at)| now - at < CACHE_SECONDS);
        c.iter()
            .find(|(u, _, _)| u == url)
            .map(|(_, s, _)| s.clone())
    }) {
        return Ok(found);
    }
    let text = download(url)?;
    CACHE.with(|c| {
        if c.len() >= 8 {
            c.remove(0);
        }
        c.push((String::from(url), text.clone(), now));
    });
    Ok(text)
}

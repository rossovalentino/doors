//! P16 SECURITY-A: advapi32's tokens and security functions against the
//! process's real token, in the token's three shapes. `secprobe.exe`
//! (MSVC-built, Windows SDK, no C runtime) checks the token (elevation
//! type, groups, the linked token, membership), well-known SIDs and account
//! names, absolute and self-relative descriptors and ACLs, SDDL both ways
//! (byte-for-byte as Windows lays descriptors out, Windows' error codes),
//! AccessCheck with an impersonation token, privileges, file security
//! (GetNamedSecurityInfo, SetNamedSecurityInfo, GetFileSecurity,
//! GetSecurityInfo/SetSecurityInfo on an open file) over storage's ACLs,
//! registry and process-object descriptors, SetEntriesInAcl, and CryptoAPI
//! (hashes, random bytes, providers, key containers). Every expectation is
//! what Windows 11 26H2 answered (`secprobe.exe limited`, 11/11 on the
//! reference PC).
//!
//! Here: a Terminal tab (the owner's filtered token, SEC-L) and an elevated
//! Doorsh (the full token, SEC-F). p16-accounts runs it as the standard
//! user (SEC-S).
use super::p15_connect::{Probe, build, run_probe, stage};
use super::*;

pub(super) const SECPROBE: Probe = Probe {
    name: "secprobe",
    directory: "tests/fixtures/windows-security",
    libraries: &["kernel32.lib", "AdvAPI32.Lib"],
    modules: &["advapi32.dll", "kernel32.dll"],
};
/// Steps secprobe reports, in order.
pub(super) const STEPS: [&str; 11] = [
    "token",
    "sids",
    "descriptors",
    "sddl",
    "access-check",
    "privileges",
    "file-security",
    "registry-security",
    "kernel-object",
    "explicit-entries",
    "crypto",
];
const RESOLUTION: &str = "1280x720";

fn wait_for(run: &Run, record: &Value, before: usize, needle: &str) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        let text = serial(&run.root, record)?;
        if let Some(line) = text
            .get(before.min(text.len())..)
            .unwrap_or("")
            .lines()
            .find(|l| l.contains(needle))
        {
            return Ok(line.to_owned());
        }
        if text.contains("KERNEL_PANIC") || text.contains("APP FAILED") {
            return Err("guest failed; inspect serial".into());
        }
        if Instant::now() >= deadline {
            return Err(format!("{needle} missing"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/security-a", run.id))?;
    let probe = build(run, &target, &SECPROBE)?;
    let staged = stage(
        run,
        image,
        std::slice::from_ref(&probe),
        &target,
        "desktop-security",
        &[
            "windows/advapi32.dll",
            "windows/ntdll.dll",
            "windows/kernel32.dll",
        ],
    )?;
    let disk = super::win32::fresh_disk(run, "security-a")?;
    let mut record = session::launch_desktop_accelerated(run, &staged, &disk, false, RESOLUTION)?;
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        thread::sleep(Duration::from_secs(3));
        // The owner's filtered token: a Terminal tab.
        super::win32::open_terminal(run, &record, &mut c)?;
        thread::sleep(Duration::from_secs(2));
        let limited = run_probe(
            run,
            &record,
            &mut c,
            &probe,
            "secprobe.exe limited guest",
            "SEC-L",
            &STEPS,
        )?;
        let frame = scene::capture_settled(run, &mut c, "security-a-limited", &[])?;
        let limited_shot = super::win32::keep(run, &frame, "security-a-limited")?;
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["alt", "f4"])?;
        super::win32::wait_after(run, &record, before, "CLIENT_EXIT", 15)?;
        thread::sleep(Duration::from_millis(800));
        // The full token: Win+R doorsh, Ctrl+Shift+Enter, User Account
        // Control, the elevated Terminal.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "r"])?;
        thread::sleep(Duration::from_millis(400));
        for ch in "doorsh".chars() {
            super::administration::type_character(&mut c, ch)?;
        }
        key(&mut c, &["ctrl", "shift", "ret"])?;
        wait_for(run, &record, before, "DOOR_P16 UAC PROMPT")?;
        key(&mut c, &["alt", "y"])?;
        wait_for(run, &record, before, "DOOR_P16 ELEVATED TERMINAL launched")?;
        thread::sleep(Duration::from_secs(3));
        let full = run_probe(
            run,
            &record,
            &mut c,
            &probe,
            "secprobe.exe full guest",
            "SEC-F",
            &STEPS,
        )?;
        let frame = scene::capture_settled(run, &mut c, "security-a-full", &[])?;
        let full_shot = super::win32::keep(run, &frame, "security-a-full")?;
        Ok(json!({"limited":limited,"limited_screenshot":limited_shot,
            "full":full,"full_screenshot":full_shot}))
    })();
    let details = super::win32::quit_on_error(run, &record, result, "security-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    run.check("p16-security-a", json!({"fixtures":[probe],"image":staged,"run":details,"session":record,
        "reference":"Windows 11 26H2 (owner's PC, filtered administrator token): secprobe.exe limited 11/11",
        "scope":"SECURITY-A: an MSVC-built probe using advapi32's token functions (elevation type, groups in Windows' order, the linked token, CheckTokenMembership, impersonation with ImpersonateSelf/RevertToSelf/DuplicateToken), well-known SIDs and account names, absolute and self-relative security descriptors and ACLs, SDDL in both directions with Windows' layouts, lengths and error codes, AccessCheck, AdjustTokenPrivileges, file security over storage's ACLs (named, by handle, GetFileSecurity), registry and process-object descriptors, SetEntriesInAcl, and CryptoAPI (MD2/MD4/MD5/SHA-1/SHA-2 hashes, random bytes, providers, key containers in the profile), with the owner's filtered token in a Terminal tab (SEC-L) and the full token in an elevated Doorsh (SEC-F)"}));
    Ok(())
}

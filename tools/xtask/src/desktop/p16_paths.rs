//! P16 PATHS-A: paths up to MAX_PATH (260 UTF-16 units). `pathprobe.exe
//! guest` (MSVC-built, Windows SDK, no C runtime) compares the errors for
//! paths of 240 to 300 characters that do not exist with what Windows 11
//! printed (`pathprobe.expected.h`, from `pathprobe.exe raw` on the owner's
//! PC, which creates nothing), then builds a folder tree in %TEMP% whose
//! deepest file's path is 255 characters long (longer than one storage
//! packet), writes, reads, finds, renames and deletes in it and removes it,
//! and a registry key whose path is about 190 units.
use super::p15_connect::{Probe, build, run_probe, stage};
use super::*;

const PATHPROBE: Probe = Probe {
    name: "pathprobe",
    directory: "tests/fixtures/windows-security",
    libraries: &["kernel32.lib", "AdvAPI32.Lib"],
    modules: &["advapi32.dll", "kernel32.dll"],
};
const RESOLUTION: &str = "1280x720";

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/paths-a", run.id))?;
    let fixture = build(run, &target, &PATHPROBE)?;
    let staged = stage(
        run,
        image,
        std::slice::from_ref(&fixture),
        &target,
        "desktop-paths",
        &[
            "windows/ntdll.dll",
            "windows/kernel32.dll",
            "windows/advapi32.dll",
        ],
    )?;
    let disk = super::win32::fresh_disk(run, "paths-a")?;
    let mut record = session::launch_desktop_accelerated(run, &staged, &disk, false, RESOLUTION)?;
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        thread::sleep(Duration::from_secs(3));
        super::win32::open_terminal(run, &record, &mut c)?;
        thread::sleep(Duration::from_secs(2));
        let probe = run_probe(
            run,
            &record,
            &mut c,
            &fixture,
            "pathprobe.exe guest",
            "PATHS-A",
            &["limits", "deep", "registry"],
        )?;
        Ok(json!({"probe":probe}))
    })();
    let details = super::win32::quit_on_error(run, &record, result, "paths-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    run.check("p16-paths-a", json!({"fixture":fixture,"image":staged,"run":details,"session":record,
        "reference":"Windows 11 26H2 (owner's PC): pathprobe.exe check 1/1 (attributes of paths that do not exist; nothing is created)",
        "scope":"PATHS-A: GetFileAttributes and CreateFile errors for 240- to 300-character paths that do not exist, then a %TEMP% tree whose deepest file's path is 255 characters: CreateDirectory, CreateFile, WriteFile, ReadFile, FindFirstFile, MoveFile, a path past MAX_PATH refused, DeleteFile and RemoveDirectory; a registry key path of about 190 units (RegCreateKeyEx, RegSetValueEx, RegGetValue, RegDeleteTree) past one storage packet"}));
    Ok(())
}

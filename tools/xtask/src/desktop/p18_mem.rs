//! P18 MEM-A: the memory calls managed runtimes are built on.
//! `memprobe.exe guest` (MSVC-built, Windows SDK, no C runtime) compares
//! with what Windows 11 printed (`memprobe.expected.h`, from `memprobe.exe
//! raw` on the owner's PC, which creates no files): CoreCLR's double-mapped
//! SEC_RESERVE section (write through one view, run through the other),
//! VirtualAlloc2 / MapViewOfFile3 placeholders split, replaced and
//! coalesced, FILE_MAP_COPY copy on write, kernel32.dll as a SEC_IMAGE
//! view; then, in the guest only, a 1 MiB file written and read in one
//! call each and changed through a flushed shared mapping.
use super::p15_connect::{Probe, build, run_probe, stage};
use super::*;

const MEMPROBE: Probe = Probe {
    name: "memprobe",
    directory: "tests/fixtures/windows-runtime",
    libraries: &["kernel32.lib"],
    modules: &["kernel32.dll"],
};
const RESOLUTION: &str = "1280x720";

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/mem-a", run.id))?;
    let fixture = build(run, &target, &MEMPROBE)?;
    let staged = stage(
        run,
        image,
        std::slice::from_ref(&fixture),
        &target,
        "desktop-mem",
        &["windows/ntdll.dll", "windows/kernel32.dll"],
    )?;
    let disk = super::win32::fresh_disk(run, "mem-a")?;
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
            "memprobe.exe guest",
            "MEM-A",
            &["sections", "placeholders", "copy", "image", "files"],
        )?;
        Ok(json!({"probe":probe}))
    })();
    let details = super::win32::quit_on_error(run, &record, result, "mem-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    run.check("p18-mem-a", json!({"fixture":fixture,"image":staged,"run":details,"session":record,
        "reference":"Windows 11 26H2 (owner's PC): memprobe.exe check 4/4 (sections, placeholders, copy, image; nothing is created)",
        "scope":"MEM-A: a 1 GiB SEC_RESERVE pagefile section double-mapped (commit through the executable view, write through the writable view, run the code), VirtualQuery of views; VirtualAlloc2 placeholders split (MEM_PRESERVE_PLACEHOLDER), replaced by a MapViewOfFile3 view and a VirtualAlloc2 allocation (MEM_REPLACE_PLACEHOLDER), returned (UnmapViewOfFile2, VirtualFree) and coalesced; FILE_MAP_COPY copy on write; kernel32.dll as a SEC_IMAGE view (MEM_IMAGE, headers read-only, code execute-read); a 1 MiB WriteFile and ReadFile and a flushed shared file mapping"}));
    Ok(())
}

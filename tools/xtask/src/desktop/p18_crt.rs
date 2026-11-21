//! P18 CRT-A: Doors' Universal CRT (`ucrtbase.dll`, M0d) under an ordinary
//! C program linked as .NET's native binaries are (MSVC, the hybrid CRT:
//! vcruntime and the C startup static, the UCRT through the
//! `api-ms-win-crt-*` contracts). `crtprobe.exe guest` compares printf and
//! scanf formatting, conversions, the library (qsort, bsearch, strtok_s,
//! ctype, math, rand, strerror), the environment and time with what
//! Windows 11 printed (`crtprobe.expected.h`, from `crtprobe.exe raw` on the
//! owner's PC), then, in the guest only, writes, reads, seeks and removes a
//! text-mode file in %TEMP%.
use super::p15_connect::{Probe, build_hybrid_crt, run_probe, stage};
use super::*;

const CRTPROBE: Probe = Probe {
    name: "crtprobe",
    directory: "tests/fixtures/windows-runtime",
    libraries: &["kernel32.lib"],
    modules: &[
        "api-ms-win-crt-convert-l1-1-0.dll",
        "api-ms-win-crt-environment-l1-1-0.dll",
        "api-ms-win-crt-filesystem-l1-1-0.dll",
        "api-ms-win-crt-heap-l1-1-0.dll",
        "api-ms-win-crt-locale-l1-1-0.dll",
        "api-ms-win-crt-math-l1-1-0.dll",
        "api-ms-win-crt-runtime-l1-1-0.dll",
        "api-ms-win-crt-stdio-l1-1-0.dll",
        "api-ms-win-crt-string-l1-1-0.dll",
        "api-ms-win-crt-time-l1-1-0.dll",
        "api-ms-win-crt-utility-l1-1-0.dll",
        "kernel32.dll",
    ],
};
const RESOLUTION: &str = "1280x720";

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/crt-a", run.id))?;
    let fixture = build_hybrid_crt(run, &target, &CRTPROBE)?;
    let staged = stage(
        run,
        image,
        std::slice::from_ref(&fixture),
        &target,
        "desktop-crt",
        &[
            "windows/ntdll.dll",
            "windows/kernel32.dll",
            "windows/ucrtbase.dll",
        ],
    )?;
    let disk = super::win32::fresh_disk(run, "crt-a")?;
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
            "crtprobe.exe guest",
            "CRT-A",
            &[
                "printf",
                "scan",
                "convert",
                "library",
                "environment",
                "time",
                "files",
            ],
        )?;
        Ok(json!({"probe":probe}))
    })();
    let details = super::win32::quit_on_error(run, &record, result, "crt-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    run.check("p18-crt-a", json!({"fixture":fixture,"image":staged,"run":details,"session":record,
        "reference":"Windows 11 26H2 (owner's PC): crtprobe.exe check 6/6 (printf, scan, convert, library, environment, time; nothing is created)",
        "scope":"CRT-A: an MSVC hybrid-CRT program (static vcruntime and mainCRTStartup, the UCRT through eleven api-ms-win-crt-* contracts) on Doors' ucrtbase.dll: printf/wprintf family formatting incl. %a and %g, snprintf truncation and the _s variants, sscanf with sets and widths, strtol/strtod edge cases and errno, qsort/bsearch/strtok_s, ctype, libm, rand, strerror, _putenv_s/getenv/_dupenv_s, gmtime_s/strftime/_mkgmtime; a text-mode file written, sized, read line by line and removed"}));
    Ok(())
}

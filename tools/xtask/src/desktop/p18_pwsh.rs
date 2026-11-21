//! P18 PWSH-A (M1): PowerShell 7, unmodified from Microsoft's release zip
//! (self-contained: its own .NET runtime beside `pwsh.exe`), staged into
//! `C:\Program Files\PowerShell\7` on a large Doors volume, running a
//! script (`tests/fixtures/pwsh/pwshprobe.ps1`, at `C:\pwshprobe`) from the
//! Terminal: the engine's start-up (module analysis, the console host,
//! execution policy from the registry), pipelines, objects and JSON,
//! strings and formatting, functions and exceptions, hashtables, culture
//! sorting, the file-system provider, .NET interop, cmdlet discovery and
//! script blocks.
use super::*;

const TAR: &str = "C:/Windows/System32/tar.exe";
const PWSH_ZIP: &str = "out/acquisitions/powershell-7.7.0/PowerShell-7.7.0-win-x64.zip";
const PROBE: &str = "tests/fixtures/pwsh/pwshprobe.ps1";
const RESOLUTION: &str = "1280x720";
const DISK_BYTES: u64 = 3 << 30;

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/pwsh-a", run.id))?;
    let pwsh = target.join("pwsh");
    let probe = target.join("probe");
    fs::create_dir_all(&pwsh).map_err(|e| e.to_string())?;
    fs::create_dir_all(&probe).map_err(|e| e.to_string())?;
    let native = |p: &std::path::Path| crate::windows::native_path(p);
    fs::copy(run.root.join(PROBE), probe.join("probe.ps1")).map_err(|e| format!("{PROBE}: {e}"))?;
    let zip = paths::existing(&run.root, PWSH_ZIP)?;
    run.required(TAR, &["-xf", &native(&zip)?, "-C", &native(&pwsh)?])?;
    let disk = run.directory.join("pwsh-a-user.raw");
    crate::persistence::large::write(&disk, DISK_BYTES)?;
    let mut items = crate::persistence::stage::tree(&pwsh, "/compat/Program Files/PowerShell/7")?;
    items.extend(crate::persistence::stage::tree(
        &probe,
        "/compat/pwshprobe",
    )?);
    let staged = crate::persistence::stage::stage(&disk, &items)?;
    for (host, on_volume) in [
        (probe.join("probe.ps1"), "/compat/pwshprobe/probe.ps1"),
        (
            pwsh.join("pwsh.exe"),
            "/compat/Program Files/PowerShell/7/pwsh.exe",
        ),
        (
            pwsh.join("System.Management.Automation.dll"),
            "/compat/Program Files/PowerShell/7/System.Management.Automation.dll",
        ),
    ] {
        let expected = fs::read(&host).map_err(|e| format!("{}: {e}", host.display()))?;
        if crate::persistence::stage::read_file(&disk, on_volume)? != expected {
            return Err(format!(
                "{on_volume} differs from {} after staging",
                host.display()
            ));
        }
    }
    let image = super::p15_connect::stage(
        run,
        image,
        &[],
        &target,
        "desktop-pwsh",
        &[
            "windows/ntdll.dll",
            "windows/kernel32.dll",
            "windows/ucrtbase.dll",
            "windows/advapi32.dll",
            "windows/ole32.dll",
            "windows/bcrypt.dll",
            "windows/version.dll",
            "windows/secur32.dll",
            "windows/amsi.dll",
            "windows/wldp.dll",
            "windows/crypt32.dll",
        ],
    )?;
    let mut record = session::launch_desktop_accelerated_v2(run, &image, &disk, false, RESOLUTION)?;
    let line = "\"C:\\Program Files\\PowerShell\\7\\pwsh.exe\" -NoLogo -NoProfile -NonInteractive -File C:\\pwshprobe\\probe.ps1";
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        thread::sleep(Duration::from_secs(3));
        super::win32::open_terminal(run, &record, &mut c)?;
        thread::sleep(Duration::from_secs(2));
        let ready = Instant::now() + Duration::from_secs(15);
        while !session::transcript(&run.root, &record)?
            .trim_end()
            .ends_with('>')
            && Instant::now() < ready
        {
            thread::sleep(Duration::from_millis(100));
        }
        let before = session::transcript(&run.root, &record)?.len();
        type_line(&mut c, line)?;
        let deadline = Instant::now() + Duration::from_secs(900);
        let mut last = (0usize, Instant::now());
        let output = loop {
            let text = session::transcript(&run.root, &record)?;
            let rest = text.get(before.min(text.len())..).unwrap_or("").to_owned();
            if rest.contains("PWSH-A PASSED") || rest.contains("PWSH-A FAILED") {
                break rest;
            }
            let log = serial(&run.root, &record)?;
            if log.contains("KERNEL_PANIC") {
                return Err(format!("guest panicked; console: {rest}"));
            }
            let progress = text.len()
                + [
                    "DOOR_LDR",
                    "DOOR_COMPAT",
                    "DOOR_DEBUG",
                    "DOOR_P09",
                    "DOOR_EXC",
                ]
                .iter()
                .map(|k| log.matches(k).count())
                .sum::<usize>();
            if progress != last.0 {
                last = (progress, Instant::now());
            }
            if Instant::now() >= deadline || last.1.elapsed() > Duration::from_secs(180) {
                let samples = super::win32::vcpu_samples(&mut c, 6);
                return Err(format!(
                    "PWSH-A did not finish; console: {rest}\nvCPU samples:\n{}",
                    samples.join("\n")
                ));
            }
            thread::sleep(Duration::from_millis(250));
        };
        let steps: Vec<&str> = output
            .lines()
            .filter(|l| l.starts_with("PWSH-A "))
            .collect();
        if !output.contains("PWSH-A PASSED") {
            return Err(format!("PWSH-A failed:\n{}", steps.join("\n")));
        }
        Ok(json!({"command":line,"steps":steps}))
    })();
    let details = super::win32::quit_on_error(run, &record, result, "pwsh-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    run.check("p18-pwsh-a", json!({"image":image,"disk":{"bytes":DISK_BYTES,"staged":staged},
        "powershell":{"zip":PWSH_ZIP,"sha256":file_hash(&zip)?},
        "run":details,"session":record,
        "reference":"the same script under PowerShell 7.7.0 on the owner's PC prints the same steps (step 1's version aside)",
        "scope":"PWSH-A: pwsh.exe (apphost) -> hostfxr -> CoreCLR from Microsoft's PowerShell 7.7.0 win-x64 zip on a 3 GiB Doors volume, an x86-64-v2 vCPU (QEMU Nehalem; .NET 11 requires it), -NonInteractive -File: engine start-up, pipelines, objects/JSON, strings, functions and exceptions, hashtables, culture sorting, the file-system provider, .NET interop, cmdlet discovery, script blocks"}));
    Ok(())
}

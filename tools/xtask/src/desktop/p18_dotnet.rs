//! P18 DOTNET-A (M1): the .NET 10 runtime (`dotnet.exe`, hostfxr,
//! hostpolicy, CoreCLR and RyuJIT, unmodified from Microsoft's release
//! zip) staged into `C:\Program Files\dotnet` on a large Doors volume, and
//! an ordinary framework-dependent console program built by the host's .NET
//! SDK (`tests/fixtures/dotnet/dotnetprobe`) in `C:\dotnetprobe`, run from
//! the Terminal: JIT-compiled code through tiering, generics and LINQ,
//! exceptions (SEH unwinding), threads and the thread pool, the GC, files,
//! text and the environment.
use super::*;

const SDK: &str = "C:/Program Files/dotnet/dotnet.exe";
const TAR: &str = "C:/Windows/System32/tar.exe";
const RUNTIME_ZIP: &str =
    "out/acquisitions/dotnet-runtime-10.0.15/dotnet-runtime-10.0.15-win-x64.zip";
const PROBE_DIR: &str = "tests/fixtures/dotnet/dotnetprobe";
const RESOLUTION: &str = "1280x720";
const DISK_BYTES: u64 = 2 << 30;

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/dotnet-a", run.id))?;
    let probe = target.join("probe");
    let runtime = target.join("dotnet");
    fs::create_dir_all(&runtime).map_err(|e| e.to_string())?;
    let native = |p: &std::path::Path| crate::windows::native_path(p);
    // Built from a copy: the SDK writes obj/ next to the project, and the
    // source tree must not change during a run.
    let source = target.join("source");
    fs::create_dir_all(&source).map_err(|e| e.to_string())?;
    for file in ["dotnetprobe.csproj", "Program.cs"] {
        fs::copy(run.root.join(PROBE_DIR).join(file), source.join(file))
            .map_err(|e| format!("{file}: {e}"))?;
    }
    run.required(
        SDK,
        &[
            "build",
            &native(&source.join("dotnetprobe.csproj"))?,
            "-c",
            "Release",
            "-o",
            &native(&probe)?,
            "--nologo",
        ],
    )?;
    let zip = paths::existing(&run.root, RUNTIME_ZIP)?;
    run.required(TAR, &["-xf", &native(&zip)?, "-C", &native(&runtime)?])?;
    let disk = run.directory.join("dotnet-a-user.raw");
    crate::persistence::large::write(&disk, DISK_BYTES)?;
    let mut items = crate::persistence::stage::tree(&runtime, "/compat/Program Files/dotnet")?;
    items.extend(crate::persistence::stage::tree(
        &probe,
        "/compat/dotnetprobe",
    )?);
    let staged = crate::persistence::stage::stage(&disk, &items)?;
    // Read back before boot: the program and the host's bytes on the volume.
    for (host, on_volume) in [
        (
            probe.join("dotnetprobe.dll"),
            "/compat/dotnetprobe/dotnetprobe.dll",
        ),
        (
            runtime.join("dotnet.exe"),
            "/compat/Program Files/dotnet/dotnet.exe",
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
        "desktop-dotnet",
        &[
            "windows/ntdll.dll",
            "windows/kernel32.dll",
            "windows/ucrtbase.dll",
            "windows/advapi32.dll",
            "windows/ole32.dll",
        ],
    )?;
    let mut record = session::launch_desktop_accelerated(run, &image, &disk, false, RESOLUTION)?;
    let line = "\"C:\\Program Files\\dotnet\\dotnet.exe\" C:\\dotnetprobe\\dotnetprobe.dll guest";
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
        let deadline = Instant::now() + Duration::from_secs(600);
        // A program that prints nothing more and leaves no serial trace for
        // two minutes has stopped: where the vCPUs are tells spin from wait.
        let mut last = (0usize, Instant::now());
        let output = loop {
            let text = session::transcript(&run.root, &record)?;
            let rest = text.get(before.min(text.len())..).unwrap_or("").to_owned();
            if rest.contains("DOTNET-A PASSED") || rest.contains("DOTNET-A FAILED") {
                break rest;
            }
            let log = serial(&run.root, &record)?;
            if log.contains("KERNEL_PANIC") {
                return Err(format!("guest panicked; console: {rest}"));
            }
            let progress = text.len()
                + ["DOOR_LDR", "DOOR_COMPAT", "DOOR_DEBUG", "DOOR_P09"]
                    .iter()
                    .map(|k| log.matches(k).count())
                    .sum::<usize>();
            if progress != last.0 {
                last = (progress, Instant::now());
            }
            if Instant::now() >= deadline || last.1.elapsed() > Duration::from_secs(120) {
                let samples = super::win32::vcpu_samples(&mut c, 6);
                return Err(format!(
                    "DOTNET-A did not finish; console: {rest}\nvCPU samples:\n{}",
                    samples.join("\n")
                ));
            }
            thread::sleep(Duration::from_millis(250));
        };
        let steps: Vec<&str> = output
            .lines()
            .filter(|l| l.starts_with("DOTNET-A "))
            .collect();
        if !output.contains("DOTNET-A PASSED") {
            return Err(format!("DOTNET-A failed:\n{}", steps.join("\n")));
        }
        Ok(json!({"command":line,"steps":steps}))
    })();
    let details = super::win32::quit_on_error(run, &record, result, "dotnet-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    run.check("p18-dotnet-a", json!({"image":image,"disk":{"bytes":DISK_BYTES,"staged":staged},
        "runtime":{"zip":RUNTIME_ZIP,"sha256":file_hash(&zip)?},
        "run":details,"session":record,
        "reference":"the same probe on the owner's PC prints the same steps (.NET 10, guest-only step 7 excluded)",
        "scope":"DOTNET-A: dotnet.exe -> hostfxr -> hostpolicy -> CoreCLR/RyuJIT from Microsoft's .NET 10.0.15 win-x64 zip on a 2 GiB Doors volume (catalog v6, ext2 at scale), running an SDK-built framework-dependent program: tiered JIT, generics/LINQ, exceptions, threads and the thread pool, GC, files, text, environment"}));
    Ok(())
}

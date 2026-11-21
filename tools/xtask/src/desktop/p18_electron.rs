//! P18 ELECTRON-A (M7): Electron, unmodified from its release zip (MIT),
//! staged into `C:\Program Files\Electron` on a large Doors volume, running
//! a small app (`tests/fixtures/electron/app`, at `C:\electronapp`) from a
//! Terminal tab: the 250 MB `electron.exe` through the image host (M0h),
//! Node in the main process (file system, platform), Chromium's browser
//! process, a window with a page rendered in software (P19 brings GPU
//! acceleration), V8 in the page, IPC through a preload bridge, and a
//! captured frame. Each step prints `ELECTRON-A <step> ok|failed`.
use super::*;

const TAR: &str = "C:/Windows/System32/tar.exe";
const ELECTRON_ZIP: &str = "out/acquisitions/electron-45.3.3/electron-v45.3.3-win32-x64.zip";
const SUMS: &str = "out/acquisitions/electron-45.3.3/SHASUMS256.txt";
const APP: &str = "tests/fixtures/electron/app";
const RESOLUTION: &str = "1280x720";
const DISK_BYTES: u64 = 3 << 30;
/// Chromium's sandbox and GPU process wait for M0k and P19: one process,
/// software compositing.
const SWITCHES: &str =
    "--no-sandbox --disable-gpu --single-process --in-process-gpu --enable-logging --log-level=0";

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/electron-a", run.id))?;
    let electron = target.join("electron");
    let app = target.join("app");
    fs::create_dir_all(&electron).map_err(|e| e.to_string())?;
    fs::create_dir_all(&app).map_err(|e| e.to_string())?;
    let native = |p: &std::path::Path| crate::windows::native_path(p);
    for name in ["main.js", "preload.js", "index.html", "package.json"] {
        let from = run.root.join(APP).join(name);
        fs::copy(&from, app.join(name)).map_err(|e| format!("{}: {e}", from.display()))?;
    }
    let zip = paths::existing(&run.root, ELECTRON_ZIP)?;
    // The release's own checksum, as fetched with it.
    let sums = fs::read_to_string(paths::existing(&run.root, SUMS)?).map_err(|e| e.to_string())?;
    let expected = sums
        .lines()
        .find(|l| l.trim_end().ends_with("electron-v45.3.3-win32-x64.zip"))
        .and_then(|l| l.split_whitespace().next())
        .ok_or("SHASUMS256.txt lacks the win32-x64 zip")?;
    let actual = file_hash(&zip)?;
    if actual != expected {
        return Err(format!(
            "{ELECTRON_ZIP}: SHA-256 {actual}, release says {expected}"
        ));
    }
    run.required(TAR, &["-xf", &native(&zip)?, "-C", &native(&electron)?])?;
    let disk = run.directory.join("electron-a-user.raw");
    crate::persistence::large::write(&disk, DISK_BYTES)?;
    let mut items = crate::persistence::stage::tree(&electron, "/compat/Program Files/Electron")?;
    items.extend(crate::persistence::stage::tree(
        &app,
        "/compat/electronapp",
    )?);
    let staged = crate::persistence::stage::stage(&disk, &items)?;
    for (host, on_volume) in [
        (app.join("main.js"), "/compat/electronapp/main.js"),
        (
            electron.join("electron.exe"),
            "/compat/Program Files/Electron/electron.exe",
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
        "desktop-electron",
        &[
            "seed/imagehost.exe",
            "windows/ntdll.dll",
            "windows/kernel32.dll",
            "windows/user32.dll",
            "windows/gdi32.dll",
            "windows/ucrtbase.dll",
            "windows/advapi32.dll",
            "windows/ole32.dll",
            "windows/ws2_32.dll",
            "windows/crypt32.dll",
            "windows/dwrite.dll",
            "windows/bcryptprimitives.dll",
            "windows/uxtheme.dll",
            "windows/dwmapi.dll",
            "windows/shlwapi.dll",
            "windows/wtsapi32.dll",
            "windows/dxgi.dll",
            "windows/d3d11.dll",
            "windows/winmm.dll",
        ],
    )?;
    // Two CPUs: Electron's single-CPU start-up path fails its own check.
    let mut record =
        session::launch_desktop_accelerated_smp2(run, &image, &disk, false, RESOLUTION)?;
    // doorsh: single quotes keep the backslashes literal (in double quotes
    // `\e` is an escape).
    let line = format!("'C:\\Program Files\\Electron\\electron.exe' {SWITCHES} C:\\electronapp");
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
        let idle_before = serial(&run.root, &record)?.matches("DSH IDLE").count();
        type_line(&mut c, &line)?;
        let deadline = Instant::now() + Duration::from_secs(1200);
        let mut last = (0usize, Instant::now());
        let output = loop {
            let text = session::transcript(&run.root, &record)?;
            let rest = text.get(before.min(text.len())..).unwrap_or("").to_owned();
            if rest.contains("ELECTRON-A PASSED") || rest.contains("ELECTRON-A FAILED") {
                break rest;
            }
            let log = serial(&run.root, &record)?;
            if log.contains("KERNEL_PANIC") {
                return Err(format!("guest panicked; console: {rest}"));
            }
            if log.contains("DOOR_LDR program failed to load") {
                return Err(format!("electron.exe did not load; console: {rest}"));
            }
            // The shell is idle again: electron.exe exited (no wait for
            // the progress timeout).
            if log.matches("DSH IDLE").count() > idle_before {
                let faults: Vec<&str> = log
                    .lines()
                    .filter(|l| {
                        [
                            "EXCEPTION pid",
                            "FAULT_AT",
                            "PROCESS manifest",
                            "DOOR_COMPAT",
                        ]
                        .iter()
                        .any(|k| l.contains(k))
                    })
                    .collect();
                return Err(format!(
                    "electron.exe exited; console: {rest}\nserial:\n{}",
                    faults[faults.len().saturating_sub(12)..].join("\n")
                ));
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
            if Instant::now() >= deadline || last.1.elapsed() > Duration::from_secs(300) {
                let samples = super::win32::vcpu_samples(&mut c, 6);
                return Err(format!(
                    "ELECTRON-A did not finish; console: {rest}\nvCPU samples:\n{}",
                    samples.join("\n")
                ));
            }
            thread::sleep(Duration::from_millis(250));
        };
        let steps: Vec<&str> = output
            .lines()
            .filter(|l| l.starts_with("ELECTRON-A "))
            .collect();
        if !output.contains("ELECTRON-A PASSED") {
            return Err(format!("ELECTRON-A failed:\n{}", steps.join("\n")));
        }
        Ok(json!({"command":line,"steps":steps}))
    })();
    let details = super::win32::quit_on_error(run, &record, result, "electron-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    run.check("p18-electron-a", json!({"image":image,"disk":{"bytes":DISK_BYTES,"staged":staged},
        "electron":{"zip":ELECTRON_ZIP,"sha256":actual,"release_sha256":expected},
        "run":details,"session":record,
        "scope":"ELECTRON-A: Electron 45.3.3 win32-x64 (MIT) unmodified on a 3 GiB Doors volume; electron.exe through the image host; main-process Node, the browser process, a BrowserWindow rendered in software (sandbox, GPU process and multi-process mode wait for M0k and P19), V8 in the page, preload IPC and a captured frame"}));
    Ok(())
}

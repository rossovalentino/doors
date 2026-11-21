//! P18: a separate persistent volume with the third-party runtimes the
//! P18 scenarios use, for trying them by hand (`cargo xtask desktop
//! --apps`): PowerShell 7 at `C:\Program Files\PowerShell\7`, the .NET
//! runtime at `C:\Program Files\dotnet`, and Electron at
//! `C:\Program Files\Electron` with the small test app at `C:\electronapp`.
//! Every archive is the unmodified release from `out/acquisitions`. The
//! owner's own volume (`user.raw`) is never touched; an existing apps
//! volume is never replaced.
use crate::{evidence::Run, paths, persistence};
use std::fs;

const TAR: &str = "C:/Windows/System32/tar.exe";
/// Large enough for the three runtimes (about 1 GB unpacked) and use.
const DISK_BYTES: u64 = 8 << 30;
/// (release archive, where it is unpacked on the volume).
const ARCHIVES: [(&str, &str); 3] = [
    (
        "out/acquisitions/powershell-7.7.0/PowerShell-7.7.0-win-x64.zip",
        "/compat/Program Files/PowerShell/7",
    ),
    (
        "out/acquisitions/dotnet-runtime-10.0.15/dotnet-runtime-10.0.15-win-x64.zip",
        "/compat/Program Files/dotnet",
    ),
    (
        "out/acquisitions/electron-45.3.3/electron-v45.3.3-win32-x64.zip",
        "/compat/Program Files/Electron",
    ),
];
const ELECTRON_APP: &str = "tests/fixtures/electron/app";

/// The apps volume's path.
pub const PATH: &str = "out/session/apps-user.raw";

pub fn initialize(run: &mut Run) -> Result<(), String> {
    let destination = run.root.join(PATH);
    paths::reject_links(&run.root, &destination)?;
    match fs::symlink_metadata(&destination) {
        Ok(_) => {
            return Err(format!(
                "{PATH} already exists; delete it yourself to make a new one (it is never replaced)"
            ));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.to_string()),
    }
    let target = run.generated_directory(&format!("out/builds/{}/apps-volume", run.id))?;
    let mut items = Vec::new();
    for (index, (archive, on_volume)) in ARCHIVES.iter().enumerate() {
        let zip = paths::existing(&run.root, archive)?;
        let unpacked = target.join(format!("archive-{index}"));
        fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
        run.required(
            TAR,
            &[
                "-xf",
                &crate::windows::native_path(&zip)?,
                "-C",
                &crate::windows::native_path(&unpacked)?,
            ],
        )?;
        items.extend(persistence::stage::tree(&unpacked, on_volume)?);
    }
    items.extend(persistence::stage::tree(
        &run.root.join(ELECTRON_APP),
        "/compat/electronapp",
    )?);
    let staged = target.join("apps-user.raw");
    persistence::large::write(&staged, DISK_BYTES)?;
    let record = persistence::stage::stage(&staged, &items)?;
    fs::create_dir_all(run.root.join("out/session")).map_err(|e| e.to_string())?;
    fs::rename(&staged, &destination).map_err(|e| format!("{PATH}: {e}"))?;
    run.check("apps-volume", record);
    println!(
        "Created {PATH}: PowerShell 7 (C:\\Program Files\\PowerShell\\7\\pwsh.exe), .NET \
         (C:\\Program Files\\dotnet\\dotnet.exe) and Electron (C:\\Program \
         Files\\Electron\\electron.exe, app at C:\\electronapp).\nRun: cargo xtask desktop --apps"
    );
    Ok(())
}

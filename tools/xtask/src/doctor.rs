use std::{env, path::PathBuf};

use serde_json::{Value, json};

use crate::evidence::{Run, file_hash, resolve_tool, write_json};

pub fn discover(run: &mut Run) -> Result<(), String> {
    let profile: Value = serde_json::from_slice(
        &std::fs::read(run.root.join("profiles/build/p00.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let compiler = run.required("rustc", &["-Vv"])?;
    if !compiler
        .stdout
        .lines()
        .any(|line| line == "release: 1.94.1")
    {
        return Err("rustc differs from pinned 1.94.1".into());
    }
    let cargo = run.required("cargo", &["-V"])?;
    let rustup = run.required("rustup", &["show"])?;
    let targets = run.required("rustup", &["target", "list", "--installed"])?;
    let git = run.required("git", &["--version"])?;
    let sysroot = run.required("rustc", &["--print", "sysroot"])?;
    let host = compiler
        .stdout
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .ok_or("rustc host unavailable")?;
    let linker_path = PathBuf::from(sysroot.stdout.trim())
        .join("lib/rustlib")
        .join(host)
        .join("bin")
        .join(if cfg!(windows) {
            "rust-lld.exe"
        } else {
            "rust-lld"
        });
    let linker = run.required(
        linker_path.to_str().ok_or("non-UTF8 linker path")?,
        &["-flavor", "link", "--version"],
    )?;
    if !compiler.stdout.contains(&format!(
        "commit-hash: {}",
        profile["rust_commit"]
            .as_str()
            .ok_or("compiler pin missing")?
    )) || !linker.stdout.starts_with(&format!(
        "LLD {} ",
        profile["llvm_version"].as_str().ok_or("LLD pin missing")?
    )) {
        return Err("compiler/linker identity differs from build profile".into());
    }
    let hardware: Value = serde_json::from_slice(
        &std::fs::read(run.root.join("profiles/hardware/qemu-diagnostic-x64.json"))
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let qemu_program = hardware["qemu"]["path"]
        .as_str()
        .unwrap_or("qemu-system-x86_64");
    let qemu = run.optional(qemu_program, &["--version"])?;
    let mut host_details = json!({"os": env::consts::OS, "architecture": env::consts::ARCH});
    #[cfg(windows)]
    {
        let observation = run.optional("powershell.exe", &["-NoProfile", "-NonInteractive", "-Command",
            "$osInfo = Get-CimInstance Win32_OperatingSystem; $cpuInfo = Get-CimInstance Win32_Processor; [pscustomobject]@{os=$osInfo.Caption;version=$osInfo.Version;architecture=$osInfo.OSArchitecture;memory_bytes=[uint64]$osInfo.TotalVisibleMemorySize*1024;logical_cpus=($cpuInfo.NumberOfLogicalProcessors | Measure-Object -Sum).Sum;virtualization_firmware_enabled=@($cpuInfo.VirtualizationFirmwareEnabled);local_volumes=@(Get-CimInstance Win32_LogicalDisk -Filter 'DriveType=3' | Select-Object DeviceID,FreeSpace)} | ConvertTo-Json -Compress -Depth 4"])?;
        if observation.success {
            host_details =
                serde_json::from_str(observation.stdout.trim()).map_err(|e| e.to_string())?;
        }
    }
    let mut host_linker = Value::Null;
    #[cfg(windows)]
    {
        let vswhere =
            PathBuf::from("C:/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe");
        if vswhere.is_file() {
            let found = run.optional(
                vswhere.to_str().ok_or("invalid vswhere path")?,
                &[
                    "-latest",
                    "-products",
                    "*",
                    "-find",
                    "VC/Tools/MSVC/**/bin/Hostx64/x64/link.exe",
                ],
            )?;
            if found.success
                && let Some(path) = found.stdout.lines().find(|line| !line.trim().is_empty())
            {
                let version = run.optional(path.trim(), &["/?"])?;
                if !version.stdout.contains(&format!(
                    "Version {}",
                    profile["host_linker_version"]
                        .as_str()
                        .ok_or("host linker pin missing")?
                )) {
                    return Err("MSVC linker differs from pinned version".into());
                }
                host_linker = json!({"path": path.trim(), "stdout": version.stdout, "stderr": version.stderr, "probe_success": version.success,
                    "note": "link /? returns nonzero on this host; version banner is discovery, actual linking is checked by host builds"});
            }
        }
    }
    let mut blockers = Vec::new();
    if !qemu.success {
        blockers.push("P01/P02: qemu-system-x86_64 was not found or its version probe failed");
    }
    for target in ["x86_64-unknown-none", "x86_64-unknown-uefi"] {
        if !targets.stdout.lines().any(|line| line == target) {
            blockers.push(if target.ends_with("uefi") {
                "firmware Rust target missing"
            } else {
                "freestanding Rust target missing"
            });
        }
    }
    let firmware_code = env::var_os("DOOR_OVMF_CODE")
        .map(PathBuf::from)
        .or_else(|| {
            hardware["firmware"]["code_path"]
                .as_str()
                .map(PathBuf::from)
        });
    let firmware_vars = env::var_os("DOOR_OVMF_VARS_TEMPLATE")
        .map(PathBuf::from)
        .or_else(|| {
            hardware["firmware"]["vars_template"]
                .as_str()
                .map(PathBuf::from)
        });
    let code_hash = firmware_code
        .as_ref()
        .map(|path| file_hash(path))
        .transpose()?;
    let vars_hash = firmware_vars
        .as_ref()
        .map(|path| file_hash(path))
        .transpose()?;
    blockers.extend(firmware_blockers(
        firmware_code.is_some(),
        firmware_vars.is_some(),
    ));
    let report = json!({
        "schema_version": 1, "record_kind": "environment", "id": "local-development",
        "run_id": run.id, "host": host_details, "workspace": run.root,
        "tools": {
            "rustc": {"path": resolve_tool("rustc"), "version": compiler.stdout},
            "cargo": {"path": resolve_tool("cargo"), "version": cargo.stdout},
            "rustup": {"path": resolve_tool("rustup"), "toolchains": rustup.stdout},
            "git": {"path": resolve_tool("git"), "version": git.stdout},
            "bundled_linker": {"path": linker_path, "version": linker.stdout},
            "host_msvc_linker": host_linker,
            "qemu_system_x86_64": {"path": resolve_tool(qemu_program), "available": qemu.success, "version": qemu.stdout},
            "rust_targets": targets.stdout.lines().collect::<Vec<_>>()
        },
        "firmware": {"code_path": firmware_code, "code_sha256": code_hash, "vars_template": firmware_vars, "vars_template_sha256": vars_hash,
            "discovery_scope": "explicit environment override or acquired hardware profile; no system-wide scan"},
        "acceleration": {"requested": null, "detected": [], "verified": null},
        "permissions": {"network": "available_in_execution_environment", "install_tools": "not_requested", "physical_disk_writes": false},
        "commands_executed": run.commands(), "blockers": blockers, "verification_state": "host_tested"
    });
    write_json(&run.root.join("reports/environment.json"), &report)?;
    println!(
        "Environment captured. {} later-phase acquisition blocker(s).",
        blockers.len()
    );
    Ok(())
}

fn firmware_blockers(has_code: bool, has_vars: bool) -> Vec<&'static str> {
    let mut blockers = Vec::new();
    if !has_code {
        blockers.push("P01/P02: exact OVMF code path/hash is not acquired");
    }
    if !has_vars {
        blockers.push("P01/P02: exact OVMF variable-template path/hash is not acquired");
    }
    blockers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn either_missing_firmware_input_is_a_separate_blocker() {
        assert_eq!(firmware_blockers(false, false).len(), 2);
        assert_eq!(firmware_blockers(true, false).len(), 1);
        assert_eq!(firmware_blockers(false, true).len(), 1);
        assert!(firmware_blockers(true, true).is_empty());
    }
}

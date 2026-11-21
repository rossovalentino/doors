use crate::{
    artifact,
    evidence::{Run, relative, write_json},
};
use serde_json::{Value, json};
use std::path::Path;
pub(crate) mod native;

pub fn plan(phase: crate::phase::Phase) -> Value {
    let mut plan = json!({"schema_version":1,"record_kind":"build_plan","profile":"dev-qemu-x64",
        "tasks":[
            {"id":"host-tools","class":"host","target":"x86_64-pc-windows-msvc","state":"available"},
            {"id":"firmware","class":"firmware","target":"x86_64-unknown-uefi","depends_on":["boot-contract","image-validator"],"state":"available","source":"boot/uefi","features":["firmware"]},
            {"id":"kernel","class":"kernel","target":"x86_64-unknown-none","depends_on":["boot-contract","image-validator"],"state":"available","source":"kernel","features":["baremetal"],"linker_script":"kernel/linker.ld"},
            {"id":"native-user","class":"native-user","target":"x86_64-unknown-none","state":"available","source":"tests/fixtures/native","runtime":"N0","linker_script":"tests/fixtures/native/linker.ld"},
            {"id":"windows-dll","class":"windows-dll","target":null,"state":"blocked","prerequisite":"P09 Windows ABI/export contract"},
            {"id":"windows-fixture","class":"windows-fixture","target":null,"state":"blocked","prerequisite":"independent permitted fixture toolchain"}],
        "image":{"kind":"qemu-readonly-vvfat-esp-tree","entries":[
            {"role":"firmware","destination":"esp/EFI/BOOT/BOOTX64.EFI","class":"firmware"},
            {"role":"kernel","destination":"esp/Door/kernel.elf","class":"kernel"},
            {"role":"initfs","destination":"esp/Door/initfs.tar","class":"initfs-archive"}],
            "state":"available","initfs_manifest":"profiles/build/initfs.json"},
        "guest_execution":"not_run"});
    if phase >= crate::phase::Phase::P08 {
        plan["image"]["initfs_manifest"] = json!("profiles/build/initfs-p08.json");
        plan["tasks"].as_array_mut().expect("build plan tasks").extend([
            json!({"id":"native-services","class":"native-user","target":"x86_64-unknown-none","state":"available","source":"services","package":"door-system-services","binaries":["door-init","door-storage","door-session","door-console"],"runtime":"N2-foundation","linker_script":"services/linker.ld"}),
            json!({"id":"native-workload","class":"native-user","target":"x86_64-unknown-none","state":"available","source":"tests/fixtures/native-workload","package":"door-native-workload","binaries":["native-workload"],"runtime":"N2-foundation","linker_script":"tests/fixtures/native-workload/linker.ld"}),
        ]);
    }
    if phase >= crate::phase::Phase::P09 {
        plan["image"]["initfs_manifest"] = json!("profiles/build/initfs-p09.json");
        plan["tasks"][4] = json!({"id":"windows-dll","class":"windows-dll","target":"x86_64-pc-windows-msvc","state":"available","packages":["door-ntdll","door-kernel32"],"runtime":"original Windows ABI adapters"});
        plan["tasks"][5] = json!({"id":"windows-fixture","class":"windows-fixture","target":"x64 MSVC/Windows SDK","state":"available","source":"tests/fixtures/windows-x64/main.c","runtime":"independent ordinary Windows SDK C; no CRT"});
    }
    if phase >= crate::phase::Phase::P10 {
        plan["image"]["initfs_manifest"] = json!("profiles/build/initfs-p10.json");
        plan["tasks"].as_array_mut().expect("build tasks").push(json!({"id":"windows-base-fixtures","class":"windows-fixture","target":"x64 MSVC/Windows SDK","state":"available","source":"tests/fixtures/windows-base","artifacts":["lifecycle.dll","windows-base.exe"],"runtime":"ordinary Windows imports; lifecycle and native-backed threads/TLS"}));
    }
    plan
}
pub fn write_plan(run: &mut Run) -> Result<(), String> {
    let ledger = crate::image::read_json(&run.root.join("project/phases.json"))?;
    let phase = crate::phase::Phase::parse(ledger["active_phase"].as_str().ok_or("active phase")?)?;
    let first = serde_json::to_vec(&plan(phase)).map_err(|e| e.to_string())?;
    let second = serde_json::to_vec(&plan(phase)).map_err(|e| e.to_string())?;
    if first != second {
        return Err("build plan is nondeterministic".into());
    }
    write_json(&run.directory.join("build-plan.json"), &plan(phase))?;
    run.check(
        "p01-build-plan",
        json!({"deterministic":true,"guest_inputs":"firmware_kernel_and_P05_native_sources_available"}),
    );
    Ok(())
}
pub fn host_build(run: &mut Run, rebuild: bool) -> Result<(), String> {
    let destination = run.generated_directory(&format!("out/builds/{}", run.id))?;
    let mut manifests = Vec::new();
    for pass in 0..if rebuild { 2 } else { 1 } {
        let target = destination.join(format!("pass-{pass}"));
        let target_name = target.to_str().ok_or("non-UTF8 build path")?;
        // CARGO_ENCODED_RUSTFLAGS preserves a path containing spaces as one flag.
        let flags = remap_flags(&run.root);
        let observed = run.bounded(
            "cargo",
            &[
                "build",
                "--locked",
                "--release",
                "--package",
                "door-xtask",
                "--bin",
                "door-xtask",
                "--target",
                "x86_64-pc-windows-msvc",
                "--target-dir",
                target_name,
            ],
            std::time::Duration::from_secs(120),
            &[("CARGO_ENCODED_RUSTFLAGS", flags.as_str())],
            true,
        )?;
        if !observed.success {
            return Err("host artifact build failed; see captured command".into());
        }
        let artifact_path = target.join("x86_64-pc-windows-msvc/release/door-xtask.exe");
        let mut manifest = artifact::inspect(&artifact_path, "host")?;
        run.required(
            artifact_path.to_str().ok_or("non-UTF8 artifact path")?,
            &["--help"],
        )?;
        manifest["execution_state"] = json!("host_smoke_tested");
        manifest["path"] = json!(relative(&run.root, &artifact_path)?);
        manifest["build_inputs"] = run.build_inputs();
        write_json(&target.join("artifact.json"), &manifest)?;
        manifests.push(manifest);
    }
    if rebuild && manifests[0]["sha256"] != manifests[1]["sha256"] {
        write_json(
            &run.directory.join("reproducibility.json"),
            &json!({"passed":false,"artifacts":manifests}),
        )?;
        return Err("controlled host rebuild hashes differ; see reproducibility.json".into());
    }
    let record = json!({"schema_version":1,"record_kind":"controlled_host_build","passed":true,"artifacts":manifests,
        "toolchain_profile":"profiles/build/p00.json","scope":"host executable on this pinned workstation; no guest artifact build"});
    write_json(&run.directory.join("reproducibility.json"), &record)?;
    if rebuild {
        run.check("p01-reproducible-host-build", record);
    }
    println!(
        "{} inspected host build(s), outputs {}",
        manifests.len(),
        destination.display()
    );
    Ok(())
}
pub fn guest_build(run: &mut Run, rebuild: bool) -> Result<String, String> {
    let ledger = crate::image::read_json(&run.root.join("project/phases.json"))?;
    let phase = crate::phase::Phase::parse(ledger["active_phase"].as_str().ok_or("active phase")?)?;
    guest_build_for(run, rebuild, phase)
}
pub fn guest_build_for(
    run: &mut Run,
    rebuild: bool,
    phase: crate::phase::Phase,
) -> Result<String, String> {
    let initfs_manifest = if phase >= crate::phase::Phase::P10 {
        "profiles/build/initfs-p10.json"
    } else if phase >= crate::phase::Phase::P09 {
        "profiles/build/initfs-p09.json"
    } else if phase >= crate::phase::Phase::P08 {
        "profiles/build/initfs-p08.json"
    } else {
        "profiles/build/initfs.json"
    };
    let destination = run.generated_directory(&format!("out/builds/{}/guest", run.id))?;
    // The controlled rebuild is an independent pass in its own target
    // directory; both passes run concurrently (`parallel::pair`).
    let (artifacts, first_inputs) = if rebuild {
        let ((first, inputs), (second, _)) = crate::parallel::pair(
            run,
            |run| guest_pass(run, &destination, 0, phase, initfs_manifest),
            |run| guest_pass(run, &destination, 1, phase, initfs_manifest),
        )?;
        (vec![first, second], inputs)
    } else {
        let (first, inputs) = guest_pass(run, &destination, 0, phase, initfs_manifest)?;
        (vec![first], inputs)
    };
    if rebuild
        && (artifacts[0].len() != artifacts[1].len()
            || artifacts[0]
                .iter()
                .zip(&artifacts[1])
                .any(|(a, b)| a["artifact"]["sha256"] != b["artifact"]["sha256"]))
    {
        write_json(
            &run.directory.join("guest-reproducibility.json"),
            &json!({"passed":false,"artifacts":artifacts,"scope":"pinned workstation firmware/kernel/initfs/independent native ELFs"}),
        )?;
        return Err("guest controlled rebuild hashes differ".into());
    }
    let comparison = json!({"passed":true,"artifacts":artifacts,"scope":"pinned workstation firmware/kernel/initfs/independent native ELFs"});
    write_json(
        &run.directory.join("guest-reproducibility.json"),
        &comparison,
    )?;
    if rebuild {
        run.check("p02-guest-rebuild", comparison);
    }
    let inputs = crate::image::validate_inputs(&run.root, &first_inputs)?;
    let image = run.generated_directory(&format!("out/images/{}", run.id))?;
    let record = crate::image::stage_inputs(&image, &inputs)?;
    write_json(&image.join("image.json"), &record)?;
    crate::initfs::record(run, &inputs[2].0, initfs_manifest)?;
    let path = relative(&run.root, &image.join("image.json"))?;
    write_json(
        &run.root.join("out/latest-dev-image.json"),
        &json!({"path":path,"build_run":run.id}),
    )?;
    println!("Guest image: {path}");
    Ok(path)
}
/// One guest build pass into `destination/pass-{pass}`: firmware, kernel,
/// native artifacts, the Windows DLLs and fixtures (the first pass also runs
/// the Windows references) and the packed initfs. Returns the pass's
/// artifact manifests and its image inputs.
fn guest_pass(
    run: &mut Run,
    destination: &Path,
    pass: usize,
    phase: crate::phase::Phase,
    initfs_manifest: &str,
) -> Result<(Vec<Value>, Value), String> {
    {
        let target = match run.dev_cache(&format!("guest-pass-{pass}"))? {
            Some(cache) => cache,
            None => destination.join(format!("pass-{pass}")),
        };
        std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        let base_flags = format!("--remap-path-prefix={}=/door-os", run.root.display());
        for (package, feature, triple, flags) in [
            (
                "door-uefi",
                "firmware",
                "x86_64-unknown-uefi",
                format!("{base_flags}\x1f-C\x1flink-arg=/Brepro\x1f-C\x1flink-arg=/debug:none"),
            ),
            (
                "door-kernel",
                "baremetal",
                "x86_64-unknown-none",
                [
                    base_flags.clone(),
                    "-C".into(),
                    "code-model=kernel".into(),
                    "-C".into(),
                    "relocation-model=static".into(),
                    "-C".into(),
                    "no-redzone=yes".into(),
                    "-C".into(),
                    format!("link-arg=-T{}", run.root.join("kernel/linker.ld").display()),
                    "-C".into(),
                    "link-arg=--build-id=none".into(),
                    "-C".into(),
                    format!("link-arg=-Map={}", target.join("kernel.map").display()),
                ]
                .join("\x1f"),
            ),
        ] {
            let observed = run.bounded(
                "cargo",
                &[
                    "build",
                    "--locked",
                    "--release",
                    "--package",
                    package,
                    "--features",
                    feature,
                    "--target",
                    triple,
                    "--target-dir",
                    target.to_str().ok_or("build path")?,
                ],
                std::time::Duration::from_secs(120),
                &[("CARGO_ENCODED_RUSTFLAGS", &flags)],
                true,
            )?;
            if !observed.success {
                return Err("guest artifact build failed".into());
            }
        }
        let firmware = target.join("x86_64-unknown-uefi/release/door-uefi.efi");
        let kernel = target.join("x86_64-unknown-none/release/door-kernel");
        let initfs = target.join("initfs.tar");
        let mut native_artifacts = native::build(run, &target, phase)?;
        if phase >= crate::phase::Phase::P09 {
            native_artifacts.extend(crate::windows::dlls::build(run, &target)?);
            let mut fixture = crate::windows::build::fixture(run, &target)?;
            let mut bundle_artifacts = native_artifacts.clone();
            bundle_artifacts.push(fixture.clone());
            fixture["bundle_plan"] = crate::windows::pe::bundle(&run.root, &bundle_artifacts)?;
            if pass == 0 {
                let reference = crate::windows::build::reference(run, &fixture)?;
                write_json(&run.directory.join("windows-reference.json"), &reference)?;
                run.check("p09-windows-reference", reference);
            }
            native_artifacts.push(fixture);
            if phase >= crate::phase::Phase::P10 {
                let base = crate::windows::base::build(run, &target)?;
                if pass == 0 {
                    let record = crate::windows::base::reference(run, &base)?;
                    write_json(&run.directory.join("windows-base-reference.json"), &record)?;
                    run.check("p10-windows-reference", record);
                }
                native_artifacts.extend(base);
            }
        }
        let generated = native_artifacts
            .iter()
            .map(|item| {
                Ok((
                    item["name"].as_str().ok_or("native name")?,
                    run.root.join(item["path"].as_str().ok_or("native path")?),
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        crate::initfs::pack(&run.root, initfs_manifest, &initfs, &generated)?;
        let entries = vec![
            json!({"role":"firmware","path":relative(&run.root,&firmware)?,"sha256":crate::evidence::file_hash(&firmware)?}),
            json!({"role":"kernel","path":relative(&run.root,&kernel)?,"sha256":crate::evidence::file_hash(&kernel)?}),
            json!({"role":"initfs","path":relative(&run.root,&initfs)?,"sha256":crate::evidence::file_hash(&initfs)?}),
        ];
        let inputs = json!({"schema_version":1,"record_kind":"image_inputs","entries":entries});
        let inspected = crate::image::validate_inputs(&run.root, &inputs)?;
        let mut manifests:Vec<_>=inspected.iter().map(|(path,_,metadata)|Ok(json!({"path":relative(&run.root,path)?,"artifact":metadata,"build_inputs":run.build_inputs(),"link_map":if metadata["class"]=="kernel" {json!({"path":relative(&run.root,&target.join("kernel.map"))?,"sha256":crate::evidence::file_hash(&target.join("kernel.map"))?})}else {Value::Null}}))).collect::<Result<_,String>>()?;
        manifests.extend(native_artifacts);
        write_json(&target.join("image-inputs.json"), &inputs)?;
        Ok((manifests, inputs))
    }
}
fn remap_flags(root: &Path) -> String {
    [
        format!("--remap-path-prefix={}=/door-os", root.display()),
        "-C".into(),
        "link-arg=/Brepro".into(),
        "-C".into(),
        "link-arg=/PDBALTPATH:door-xtask.pdb".into(),
    ]
    .join("\x1f")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plans_keep_future_classes_blocked_and_are_deterministic() {
        let legacy = plan(crate::phase::Phase::P07);
        assert_eq!(
            serde_json::to_vec(&legacy).unwrap(),
            serde_json::to_vec(&plan(crate::phase::Phase::P07)).unwrap()
        );
        assert_eq!(legacy["tasks"].as_array().unwrap().len(), 6);
        assert_eq!(legacy["image"]["state"], "available");
        assert_eq!(legacy["tasks"][3]["runtime"], "N0");
        assert_eq!(legacy["tasks"][4]["state"], "blocked");
        let current = plan(crate::phase::Phase::P08);
        assert_eq!(current["tasks"].as_array().unwrap().len(), 8);
        assert_eq!(current["tasks"][7]["runtime"], "N2-foundation");
        assert_eq!(
            current["image"]["initfs_manifest"],
            "profiles/build/initfs-p08.json"
        );
        assert_eq!(current["tasks"][4]["state"], "blocked");
    }
}

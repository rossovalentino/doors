//! Fast development relink without duplicating retained Windows reference builds.
//! Qualification always uses the complete independent rebuild path instead.
use super::*;
pub fn desktop(run: &mut Run, previous: &str) -> Result<String, String> {
    let previous_path = paths::existing(&run.root, previous)?;
    let previous = image::read_json(&previous_path)?;
    let old = previous["source_snapshot"]["files"]
        .as_array()
        .ok_or("development base source identity")?;
    let current = run.build_inputs();
    let files = current["files"].as_array().ok_or("current files")?;
    // Never reuse a kernel/firmware across changes to their transitive sources.
    let protected = |path: &str| {
        [
            "kernel/",
            "boot/uefi/",
            "runtime/abi/",
            "crates/boot-protocol/",
            "crates/image-format/",
            "fs/initfs/",
            "boot/debug/",
            "fs/storage-core/",
            "compat/pe/",
            "compat/abi/",
        ]
        .iter()
        .any(|prefix| path.starts_with(prefix))
            || [
                "Cargo.toml",
                "Cargo.lock",
                "rust-toolchain.toml",
                ".cargo/config.toml",
            ]
            .contains(&path)
    };
    let subset = |items: &[Value]| {
        items
            .iter()
            .filter(|v| v["path"].as_str().is_some_and(protected))
            .cloned()
            .collect::<Vec<_>>()
    };
    if subset(old) != subset(files) {
        return Err("Core inputs changed; run a complete desktop build".into());
    }
    let mut artifacts = previous["artifacts"]
        .as_array()
        .ok_or("development artifact inventory")?
        .clone();
    for a in &artifacts {
        let path = paths::existing(&run.root, a["path"].as_str().ok_or("artifact path")?)?;
        let expected = a["artifact"]["sha256"]
            .as_str()
            .or_else(|| a["sha256"].as_str())
            .ok_or("artifact hash")?;
        if file_hash(&path)? != expected {
            return Err("development base artifact changed".into());
        }
    }
    let target =
        paths::fresh_directory(&run.root, &format!("out/builds/{}/desktop-relink", run.id))?;
    let services: Vec<_> = build::native::inventory(crate::phase::Phase::P08)
        .filter(|a| a.package == "door-system-services" || a.package == "door-native-workload")
        .collect();
    let built =
        build::native::build_selected(run, &target, services.into_iter().chain(GUI.iter()))?;
    let pack = crate::app_assets::build(&run.root, &target)?;
    // The original Windows DLLs are user-mode adapters (no kernel input):
    // relink them too so DLL fixes reach the development image.
    let dlls = crate::windows::dlls::build(run, &target.join("windows-dlls"))?;
    for item in built.into_iter().chain([pack]).chain(dlls) {
        let name = item["name"].clone();
        artifacts.retain(|a| a["name"] != name);
        artifacts.push(item);
    }
    let generated = artifacts
        .iter()
        .filter_map(|a| a["name"].as_str().map(|n| (n, a)))
        .map(|(name, a)| {
            Ok((
                name,
                run.root.join(a["path"].as_str().ok_or("artifact path")?),
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let initfs = target.join("initfs.tar");
    crate::initfs::pack(
        &run.root,
        "profiles/build/initfs-desktop.json",
        &initfs,
        &generated,
    )?;
    let (base, _) =
        image::validate_staged(&run.root, previous["image"].as_str().ok_or("base image")?)?;
    let firmware = base.join("esp/EFI/BOOT/BOOTX64.EFI");
    let kernel = base.join("esp/Door/kernel.elf");
    let inputs = json!({"schema_version":1,"record_kind":"image_inputs","entries":[{"role":"firmware","path":relative(&run.root,&firmware)?,"sha256":file_hash(&firmware)?},{"role":"kernel","path":relative(&run.root,&kernel)?,"sha256":file_hash(&kernel)?},{"role":"initfs","path":relative(&run.root,&initfs)?,"sha256":file_hash(&initfs)?}]});
    let inspected = image::validate_inputs(&run.root, &inputs)?;
    let staged = paths::fresh_directory(&run.root, &format!("out/images/{}-desktop-dev", run.id))?;
    let record = image::stage_inputs(&staged, &inspected)?;
    write_json(&staged.join("image.json"), &record)?;
    let path = relative(&run.root, &staged.join("image.json"))?;
    let test_image = stage_desktop_test(run, &firmware, &kernel, &target, &generated)?;
    let diagnostics_image = stage_desktop_diagnostics(run, &firmware, &kernel, &initfs, &target)?;
    let build = json!({"schema_version":1,"image":path,"test_image":test_image,"diagnostics_image":diagnostics_image,"source_snapshot":current,"artifacts":artifacts,"development_relink":true,"qualification":false,"reused_from":relative(&run.root,&previous_path)?,"initfs_sha256":file_hash(&initfs)?});
    write_json(&run.directory.join("desktop-build.json"), &build)?;
    write_json(&run.root.join("out/latest-desktop-image.json"), &build)?;
    run.check(
        "desktop-development-relink",
        json!({"image":path,"qualification":false}),
    );
    println!("Desktop development image: {path}");
    Ok(path)
}

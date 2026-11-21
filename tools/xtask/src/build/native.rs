//! Independently linked native fixtures, inspected before inclusion in initfs.
use crate::{
    artifact,
    evidence::{Run, file_hash, relative},
};
use serde_json::{Value, json};
use std::{path::Path, time::Duration};

#[derive(Clone, Copy)]
pub struct NativeArtifact {
    pub name: &'static str,
    pub package: &'static str,
    pub linker: &'static str,
    pub runtime: &'static str,
}
const FIXTURES: &[NativeArtifact] = &[
    NativeArtifact {
        name: "native-runtime",
        package: "door-native-fixtures",
        linker: "tests/fixtures/native/linker.ld",
        runtime: "N0 door-rt/door-sys",
    },
    NativeArtifact {
        name: "native-probe",
        package: "door-native-fixtures",
        linker: "tests/fixtures/native/linker.ld",
        runtime: "independent raw assembly ABI literals",
    },
];
const PLATFORM: &[NativeArtifact] = &[
    NativeArtifact {
        name: "door-init",
        package: "door-system-services",
        linker: "services/linker.ld",
        runtime: "N2-foundation door-rt/door-sys",
    },
    NativeArtifact {
        name: "door-storage",
        package: "door-system-services",
        linker: "services/linker.ld",
        runtime: "N2-foundation door-rt/door-sys",
    },
    NativeArtifact {
        name: "door-session",
        package: "door-system-services",
        linker: "services/linker.ld",
        runtime: "N2-foundation door-rt/door-sys",
    },
    NativeArtifact {
        name: "door-console",
        package: "door-system-services",
        linker: "services/linker.ld",
        runtime: "N2-foundation door-rt/door-sys",
    },
    NativeArtifact {
        name: "native-workload",
        package: "door-native-workload",
        linker: "tests/fixtures/native-workload/linker.ld",
        runtime: "N2-foundation independent application",
    },
];

pub fn inventory(phase: crate::phase::Phase) -> impl Iterator<Item = &'static NativeArtifact> {
    FIXTURES.iter().chain(
        PLATFORM
            .iter()
            .filter(move |_| phase >= crate::phase::Phase::P08),
    )
}

pub fn flags(root: &Path, target: &Path, artifact: &NativeArtifact) -> String {
    let map = target.join(format!("{}.map", artifact.name));
    [
        format!("--remap-path-prefix={}=/door-os", root.display()),
        "-C".into(),
        "code-model=small".into(),
        "-C".into(),
        "lto=off".into(),
        "-C".into(),
        "relocation-model=static".into(),
        "-C".into(),
        "no-redzone=yes".into(),
        "-C".into(),
        format!("link-arg=-T{}", root.join(artifact.linker).display()),
        "-C".into(),
        "link-arg=--build-id=none".into(),
        "-C".into(),
        format!("link-arg=-Map={}", map.display()),
    ]
    .join("\x1f")
}

pub fn build(
    run: &mut Run,
    target: &Path,
    phase: crate::phase::Phase,
) -> Result<Vec<Value>, String> {
    build_selected(run, target, inventory(phase))
}

/// Codegen flags every crate of a userland application build shares (the
/// `flags` of an artifact without its link arguments).
fn shared_flags(root: &Path) -> String {
    [
        format!("--remap-path-prefix={}=/door-os", root.display()),
        "-C".into(),
        "code-model=small".into(),
        "-C".into(),
        "lto=off".into(),
        "-C".into(),
        "relocation-model=static".into(),
        "-C".into(),
        "no-redzone=yes".into(),
    ]
    .join("\x1f")
}

/// Userland applications (text and desktop images): each executable links
/// with its own linker script, build-id policy and link map exactly as in
/// `build_selected`, but the link arguments reach only the final executable
/// (`cargo rustc ... -- -C link-arg=...`) while the codegen flags reach every
/// crate. One pass therefore compiles the shared dependency graph once,
/// instead of once per executable: per-executable link maps inside
/// CARGO_ENCODED_RUSTFLAGS made every crate's flags differ. The controlled
/// rebuild in another target directory still repeats everything from scratch.
pub fn build_applications<'a>(
    run: &mut Run,
    target: &Path,
    selected: impl Iterator<Item = &'a NativeArtifact>,
) -> Result<Vec<Value>, String> {
    let mut manifests = Vec::new();
    let shared = shared_flags(&run.root);
    for artifact in selected {
        let name = artifact.name;
        let map = target.join(format!("{name}.map"));
        // The loader never reads .symtab/.strtab (up to 700 KB of an
        // application near the native file limit); the link map keeps every
        // symbol and address for diagnosis.
        let link = [
            format!("link-arg=-T{}", run.root.join(artifact.linker).display()),
            "link-arg=--build-id=none".into(),
            format!("link-arg=-Map={}", map.display()),
            "link-arg=--strip-all".into(),
        ];
        let mut args = vec![
            "rustc",
            "--locked",
            "--release",
            "--package",
            artifact.package,
            "--features",
            "native",
            "--bin",
            name,
            "--target",
            "x86_64-unknown-none",
            "--target-dir",
            target.to_str().ok_or("native build directory")?,
            "--",
        ];
        for value in &link {
            args.extend(["-C", value.as_str()]);
        }
        let observed = run.bounded(
            "cargo",
            &args,
            Duration::from_secs(120),
            &[("CARGO_ENCODED_RUSTFLAGS", shared.as_str())],
            true,
        )?;
        if !observed.success {
            return Err(format!("native application {name} build failed"));
        }
        manifests.push(manifest(run, target, artifact)?);
    }
    Ok(manifests)
}

fn manifest(run: &Run, target: &Path, artifact: &NativeArtifact) -> Result<Value, String> {
    let name = artifact.name;
    let map = target.join(format!("{name}.map"));
    let path = target.join(format!("x86_64-unknown-none/release/{name}"));
    let inspected = artifact::inspect(&path, "native-user")?;
    Ok(
        json!({"name":name,"path":relative(&run.root,&path)?,"artifact":inspected,"build_inputs":run.build_inputs(),"initfs_destination":format!("bin/{name}.elf"),"link_map":{"path":relative(&run.root,&map)?,"sha256":file_hash(&map)?},"runtime":artifact.runtime}),
    )
}

pub fn build_selected<'a>(
    run: &mut Run,
    target: &Path,
    selected: impl Iterator<Item = &'a NativeArtifact>,
) -> Result<Vec<Value>, String> {
    let mut manifests = Vec::new();
    for artifact in selected {
        let name = artifact.name;
        let flags = flags(&run.root, target, artifact);
        let observed = run.bounded(
            "cargo",
            &[
                "build",
                "--locked",
                "--release",
                "--package",
                artifact.package,
                "--features",
                "native",
                "--bin",
                name,
                "--target",
                "x86_64-unknown-none",
                "--target-dir",
                target.to_str().ok_or("native build directory")?,
            ],
            Duration::from_secs(120),
            &[("CARGO_ENCODED_RUSTFLAGS", flags.as_str())],
            true,
        )?;
        if !observed.success {
            return Err(format!("native fixture {name} build failed"));
        }
        manifests.push(manifest(run, target, artifact)?);
    }
    Ok(manifests)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn platform_inventory_preserves_independent_legacy_callers_and_adds_real_packages() {
        let legacy: Vec<_> = inventory(crate::phase::Phase::P07)
            .map(|a| a.name)
            .collect();
        assert_eq!(legacy, ["native-runtime", "native-probe"]);
        let platform: Vec<_> = inventory(crate::phase::Phase::P08).collect();
        assert_eq!(platform.len(), 7);
        for (a, b) in legacy.iter().zip(&platform) {
            assert_eq!(*a, b.name);
        }
        assert_eq!(platform[6].package, "door-native-workload");
        assert_ne!(platform[6].package, platform[2].package);
        assert_ne!(platform[6].linker, platform[2].linker);
        let names: std::collections::BTreeSet<_> = platform.iter().map(|a| a.name).collect();
        assert_eq!(names.len(), platform.len());
    }
}

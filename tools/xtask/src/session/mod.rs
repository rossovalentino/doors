//! Ordinary text-session image and persistent owner disk. Fixtures remain separate.
pub mod applications;
pub mod apps_volume;
mod desktop_options;
pub(crate) mod native_display;
pub mod owner_volume;
pub mod refresh;
pub mod verify;
use crate::{
    build, emulator,
    evidence::{Run, file_hash, relative, write_json},
    image, paths, qmp,
};
pub use desktop_options::DesktopOptions;
pub(crate) use desktop_options::NATIVE_PIXEL_ENV;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    net::TcpListener,
    path::Path,
    thread,
    time::{Duration, Instant},
};

const APPS: &[build::native::NativeArtifact] = &[
    build::native::NativeArtifact {
        name: "door-shell",
        package: "door-shell",
        linker: "runtime/linker/application.ld",
        runtime: "native interactive shell",
    },
    build::native::NativeArtifact {
        name: "door-tools",
        package: "door-tools",
        linker: "runtime/linker/application.ld",
        runtime: "file-supplied native utilities",
    },
];
const GUI: &[build::native::NativeArtifact] = &[
    build::native::NativeArtifact {
        name: "door-window",
        package: "door-window",
        linker: "runtime/linker/application.ld",
        runtime: "protected native CPU compositor",
    },
    // Workspace applications: one executable each over the shared library
    // (ADR 0018); the kernel selects by (Workspace manifest, launch mode).
    build::native::NativeArtifact {
        name: "door-files",
        package: "door-workspace",
        linker: "runtime/linker/application.ld",
        runtime: "protected persistent File Explorer client",
    },
    build::native::NativeArtifact {
        name: "door-notes",
        package: "door-workspace",
        linker: "runtime/linker/application.ld",
        runtime: "protected persistent Notes client",
    },
    build::native::NativeArtifact {
        name: "door-settings",
        package: "door-workspace",
        linker: "runtime/linker/application.ld",
        runtime: "protected persistent Settings client",
    },
    build::native::NativeArtifact {
        name: "door-terminal",
        package: "door-workspace",
        linker: "runtime/linker/application.ld",
        runtime: "protected Terminal client",
    },
    build::native::NativeArtifact {
        name: "door-search",
        package: "door-workspace",
        linker: "runtime/linker/application.ld",
        runtime: "protected windowless Start search host",
    },
    // P14 desktop shell (manifest DESKTOP_SHELL), started and restarted by
    // Init in the search-host slot; it also hosts Start file search.
    build::native::NativeArtifact {
        name: "door-desktop-shell",
        package: "door-desktop-shell",
        linker: "runtime/linker/application.ld",
        runtime: "protected restartable desktop shell (door.shell, notifications, registrations)",
    },
    // P14 inbox apps (manifests `abi::manifest::INBOX`: Calculator), each
    // launched by Init as a Start accessory; the manifest picks the app.
    build::native::NativeArtifact {
        name: "door-inbox",
        package: "door-inbox",
        linker: "runtime/linker/application.ld",
        runtime: "protected inbox apps (Calculator)",
    },
    // P15 network service (manifest NETWORK): TCP/IP over the NIC, started
    // by Init with the desktop as a system service.
    build::native::NativeArtifact {
        name: "door-network",
        package: "door-network",
        linker: "services/linker.ld",
        runtime: "protected system network service (door.network, smoltcp)",
    },
    // P15 sound service (manifest AUDIO): the mixer over virtio-sound,
    // started by Init with the desktop as a system service.
    build::native::NativeArtifact {
        name: "door-audio",
        package: "door-audio",
        linker: "services/linker.ld",
        runtime: "protected system sound service (door.audio mixer)",
    },
    // Session input method editor (manifest IME), started by Init.
    build::native::NativeArtifact {
        name: "door-ime",
        package: "door-ime",
        linker: "runtime/linker/application.ld",
        runtime: "protected windowless session input method editor",
    },
    build::native::NativeArtifact {
        name: "door-monitor",
        package: "door-monitor",
        linker: "runtime/linker/application.ld",
        runtime: "protected standalone read-only system monitor",
    },
    build::native::NativeArtifact {
        name: "door-preview",
        package: "door-preview",
        linker: "runtime/linker/application.ld",
        runtime: "protected standalone bounded file viewer",
    },
    // Start-search accessories (manifests GALLERY and ASSIST); the
    // package's `native` feature builds both with `accessibility`.
    build::native::NativeArtifact {
        name: "door-gallery",
        package: "door-gallery",
        linker: "runtime/linker/application.ld",
        runtime: "protected Door UI component gallery (engineering fixture)",
    },
    build::native::NativeArtifact {
        name: "door-assist",
        package: "door-gallery",
        linker: "runtime/linker/application.ld",
        runtime: "protected Captions assistive client (door.accessibility)",
    },
];

pub fn transcript(root: &Path, session: &Value) -> Result<String, String> {
    let serial = paths::existing(root, session["serial"].as_str().ok_or("session serial")?)?;
    decode_transcript(&fs::read(serial).map_err(|e| e.to_string())?)
}
fn decode_transcript(bytes: &[u8]) -> Result<String, String> {
    // QEMU appends the stream concurrently. Only complete newline-committed
    // records are parseable evidence; this also excludes partial UTF-8 writes.
    let end = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let text = core::str::from_utf8(&bytes[..end]).map_err(|e| e.to_string())?;
    let mut output = String::new();
    for line in text.lines() {
        if line.starts_with("DOOR_CONSOLE ")
            && let Some((prefix, quoted)) = line.split_once(" text=")
        {
            let count = prefix
                .strip_prefix("DOOR_CONSOLE bytes=")
                .ok_or("console count")?
                .parse::<usize>()
                .map_err(|e| e.to_string())?;
            let text = serde_json::from_str::<String>(quoted).map_err(|e| e.to_string())?;
            if text.len() != count || count > 1024 {
                return Err("Console byte count mismatch".into());
            }
            output.push_str(&text);
        }
    }
    Ok(output)
}
pub fn stop(run: &mut Run) -> Result<(), String> {
    let record = image::read_json(&run.root.join("out/native-session.json"))?;
    let port = u16::try_from(record["qmp_port"].as_u64().ok_or("session port")?)
        .map_err(|e| e.to_string())?;
    let mut client = qmp::Client::connect(port)?;
    if client.execute("query-uuid", json!({}))?["UUID"] != record["uuid"] {
        return Err("Session identity changed".into());
    }
    client.execute("quit", json!({}))?;
    run.check("native-session-stop", json!({"session":record}));
    Ok(())
}
pub fn send(run: &mut Run, line: &str) -> Result<(), String> {
    let record = image::read_json(&run.root.join("out/native-session.json"))?;
    let before = transcript(&run.root, &record)?;
    send_line(&run.root, &record, line)?;
    let after = transcript(&run.root, &record)?;
    if let Some(output) = after.strip_prefix(&before) {
        print!("{output}");
    }
    run.check(
        "native-session-command",
        json!({"command":line,"session":record,"transcript":transcript(&run.root,&record)?}),
    );
    Ok(())
}
pub fn send_line(root: &Path, record: &Value, line: &str) -> Result<(), String> {
    let before = transcript(root, record)?;
    type_line(record, line)?;
    if line == "shutdown" || line == "reboot" || line == "exit" {
        return Ok(());
    }
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let after = transcript(root, record)?;
        if after.len() > before.len() + line.len() && after.ends_with("> ") {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(format!(
                "Command did not return to prompt: {line}; transcript {:?}",
                after.chars().rev().take(400).collect::<String>()
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}
pub fn type_line(record: &Value, line: &str) -> Result<(), String> {
    let port = u16::try_from(record["qmp_port"].as_u64().ok_or("session port")?)
        .map_err(|e| e.to_string())?;
    let mut client = qmp::Client::connect(port)?;
    if client.execute("query-uuid", json!({}))?["UUID"] != record["uuid"] {
        return Err("Session identity changed".into());
    }
    for ch in line.chars().chain(core::iter::once('\n')) {
        let keys = keys(ch)?;
        client.execute("send-key", json!({"keys":keys,"hold-time":15}))?;
        thread::sleep(Duration::from_millis(45));
    }
    Ok(())
}
pub(crate) fn keys(ch: char) -> Result<Vec<Value>, String> {
    if ch == '\x03' {
        return Ok(vec![
            json!({"type":"qcode","data":"ctrl"}),
            json!({"type":"qcode","data":"c"}),
        ]);
    }
    let (shift, key) = match ch {
        '\n' => (false, "ret".into()),
        '\x08' => (false, "backspace".into()),
        ' ' => (false, "spc".into()),
        '\t' => (false, "tab".into()),
        'a'..='z' => (false, ch.to_string()),
        'A'..='Z' => (true, ch.to_ascii_lowercase().to_string()),
        '0'..='9' => (false, ch.to_string()),
        '/' => (false, "slash".into()),
        '.' => (false, "dot".into()),
        ',' => (false, "comma".into()),
        '-' => (false, "minus".into()),
        '=' => (false, "equal".into()),
        '\'' => (false, "apostrophe".into()),
        ';' => (false, "semicolon".into()),
        '[' => (false, "bracket_left".into()),
        ']' => (false, "bracket_right".into()),
        '\\' => (false, "backslash".into()),
        '`' => (false, "grave_accent".into()),
        '>' => (true, "dot".into()),
        '<' => (true, "comma".into()),
        '"' => (true, "apostrophe".into()),
        '_' => (true, "minus".into()),
        '+' => (true, "equal".into()),
        ':' => (true, "semicolon".into()),
        '(' => (true, "9".into()),
        ')' => (true, "0".into()),
        '!' => (true, "1".into()),
        '@' => (true, "2".into()),
        '#' => (true, "3".into()),
        '$' => (true, "4".into()),
        '%' => (true, "5".into()),
        '^' => (true, "6".into()),
        '&' => (true, "7".into()),
        '*' => (true, "8".into()),
        '?' => (true, "slash".into()),
        '|' => (true, "backslash".into()),
        '{' => (true, "bracket_left".into()),
        '}' => (true, "bracket_right".into()),
        '~' => (true, "grave_accent".into()),
        _ => return Err(format!("Unsupported keyboard character {ch:?}")),
    };
    let mut keys = Vec::new();
    if shift {
        keys.push(json!({"type":"qcode","data":"shift"}));
    }
    keys.push(json!({"type":"qcode","data":key}));
    Ok(keys)
}
pub fn build(run: &mut Run) -> Result<String, String> {
    build_profile(run, false)
}
pub fn build_desktop(run: &mut Run) -> Result<String, String> {
    build_profile(run, true)
}
fn build_profile(run: &mut Run, desktop: bool) -> Result<String, String> {
    let diagnostic = if run.directory.join("guest-reproducibility.json").is_file() {
        run.generated_path(&format!("out/images/{}/image.json", run.id))?
    } else {
        if desktop {
            build::guest_build_for(run, false, crate::phase::Phase::P11)?
        } else {
            build::guest_build(run, false)?
        }
    };
    let (base, _) = image::validate_staged(&run.root, &diagnostic)?;
    let prior = image::read_json(&run.directory.join("guest-reproducibility.json"))?;
    let mut artifacts = prior["artifacts"][0]
        .as_array()
        .ok_or("guest artifact list")?
        .clone();
    let prefix = if desktop { "desktop-" } else { "" };
    let target = match run.dev_cache(&format!("{prefix}userland"))? {
        Some(cache) => cache,
        None => run.generated_directory(&format!("out/builds/{}/{prefix}userland", run.id))?,
    };
    let repeated = match run.dev_cache(&format!("{prefix}userland-repeat"))? {
        Some(cache) => cache,
        None => {
            run.generated_directory(&format!("out/builds/{}/{prefix}userland-repeat", run.id))?
        }
    };
    // The build and its from-scratch controlled rebuild are independent
    // passes in separate target directories (`parallel::pair`).
    let selected: Vec<&build::native::NativeArtifact> =
        APPS.iter().chain(GUI.iter().filter(|_| desktop)).collect();
    let pass = |run: &mut Run, directory: &Path| -> Result<Vec<Value>, String> {
        let mut built =
            build::native::build_applications(run, directory, selected.iter().copied())?;
        built.push(applications::build(run, directory)?);
        if desktop {
            built.push(crate::app_assets::build(&run.root, directory)?);
        }
        Ok(built)
    };
    let (first, repeated_artifacts) =
        crate::parallel::pair(run, |run| pass(run, &target), |run| pass(run, &repeated))?;
    artifacts.extend(first);
    let mut reproducibility = Vec::new();
    for artifact in &repeated_artifacts {
        let name = artifact["name"].as_str().ok_or("repeated app name")?;
        let first = artifacts
            .iter()
            .find(|a| a["name"] == name)
            .ok_or("first app artifact")?;
        let a = paths::existing(&run.root, first["path"].as_str().ok_or("first app path")?)?;
        let b = paths::existing(
            &run.root,
            artifact["path"].as_str().ok_or("repeated app path")?,
        )?;
        if file_hash(&a)? != file_hash(&b)? {
            return Err(format!("Ordinary app rebuild differs: {name}"));
        }
        reproducibility.push(json!({"name":name,"sha256":file_hash(&a)?,"paths":[relative(&run.root,&a)?,relative(&run.root,&b)?]}));
    }
    let generated = artifacts
        .iter()
        .filter_map(|a| a["name"].as_str().map(|name| (name, a)))
        .map(|(name, a)| {
            Ok((
                name,
                run.root.join(a["path"].as_str().ok_or("application path")?),
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let initfs = target.join("initfs.tar");
    crate::initfs::pack(
        &run.root,
        if desktop {
            "profiles/build/initfs-desktop.json"
        } else {
            "profiles/build/initfs-text.json"
        },
        &initfs,
        &generated,
    )?;
    let firmware = base.join("esp/EFI/BOOT/BOOTX64.EFI");
    let kernel = base.join("esp/Door/kernel.elf");
    let inputs = json!({"schema_version":1,"record_kind":"image_inputs","entries":[
        {"role":"firmware","path":relative(&run.root,&firmware)?,"sha256":file_hash(&firmware)?},
        {"role":"kernel","path":relative(&run.root,&kernel)?,"sha256":file_hash(&kernel)?},
        {"role":"initfs","path":relative(&run.root,&initfs)?,"sha256":file_hash(&initfs)?}]});
    let inspected = image::validate_inputs(&run.root, &inputs)?;
    let staged = run.generated_directory(&format!(
        "out/images/{}-{}",
        run.id,
        if desktop { "desktop" } else { "session" }
    ))?;
    let record = image::stage_inputs(&staged, &inspected)?;
    write_json(&staged.join("image.json"), &record)?;
    let path = relative(&run.root, &staged.join("image.json"))?;
    let (test_image, diagnostics_image) = if desktop {
        (
            Some(stage_desktop_test(
                run, &firmware, &kernel, &target, &generated,
            )?),
            Some(stage_desktop_diagnostics(
                run, &firmware, &kernel, &initfs, &target,
            )?),
        )
    } else {
        (None, None)
    };
    // The three desktop images: what each is for and what tells them apart.
    let images = desktop.then(|| json!({
        "image": "shipping desktop image: production initfs, no test entries",
        "test_image": "adversarial window-test image: production initfs plus system/window-tests.conf and bin/native-workload.elf, without the IME dictionaries",
        "diagnostics_image": "desktop-diagnostics image: the production initfs bytes plus only system/assist-diagnostics.conf, so Captions writes the trees and captions it reads to serial (a11y-01, component-gallery)",
    }));
    let build = json!({"schema_version":1,"output_layout":run.output_layout.declaration(&run.id),"image":path,"test_image":test_image,"diagnostics_image":diagnostics_image,"images":images,"source_snapshot":run.build_inputs(),"artifacts":artifacts,"application_rebuilds":reproducibility,"initfs_sha256":file_hash(&initfs)?});
    run.check(
        if desktop {
            "desktop-application-rebuilds"
        } else {
            "ordinary-application-rebuilds"
        },
        json!(reproducibility),
    );
    let name = if desktop { "desktop" } else { "session" };
    write_json(&run.directory.join(format!("{name}-build.json")), &build)?;
    write_json(
        &run.root.join(format!("out/latest-{name}-image.json")),
        &build,
    )?;
    println!("{name} image: {path}");
    Ok(path)
}
fn stage_desktop_test(
    run: &Run,
    firmware: &Path,
    kernel: &Path,
    target: &Path,
    generated: &[(&str, std::path::PathBuf)],
) -> Result<String, String> {
    let initfs = target.join("initfs-desktop-test.tar");
    crate::initfs::pack(
        &run.root,
        "profiles/build/initfs-desktop-test.json",
        &initfs,
        generated,
    )?;
    stage_desktop_variant(run, firmware, kernel, &initfs, "desktop-test")
}
/// The Captions diagnostics flag (`door_service_protocol::accessibility::
/// ASSIST_DIAGNOSTICS_FILE`/`_FLAG`): its staged path, source and contents.
pub const ASSIST_DIAGNOSTICS: (&str, &str, &[u8]) = (
    "system/assist-diagnostics.conf",
    "boot/config/assist-diagnostics.conf",
    b"assist-diagnostics-v1\n",
);
/// The desktop-diagnostics image: the production initfs bytes plus only the
/// Captions flag file, staged without packing or building anything again.
fn stage_desktop_diagnostics(
    run: &Run,
    firmware: &Path,
    kernel: &Path,
    production: &Path,
    target: &Path,
) -> Result<String, String> {
    let (name, source, contents) = ASSIST_DIAGNOSTICS;
    let flag = fs::read(paths::existing(&run.root, source)?).map_err(|e| e.to_string())?;
    if flag != contents {
        return Err(format!("{source} must hold exactly {contents:?}"));
    }
    let base = fs::read(production).map_err(|e| e.to_string())?;
    let initfs = target.join("initfs-desktop-diagnostics.tar");
    fs::write(&initfs, crate::initfs::with_file(&base, name, &flag)?).map_err(|e| e.to_string())?;
    stage_desktop_variant(run, firmware, kernel, &initfs, "desktop-diagnostics")
}
/// Stages a desktop image variant from the base firmware and kernel plus `initfs`
/// (also used by the P13 Windows GUI suites in `desktop::win32`).
pub(crate) fn stage_desktop_variant(
    run: &Run,
    firmware: &Path,
    kernel: &Path,
    initfs: &Path,
    variant: &str,
) -> Result<String, String> {
    let inputs = json!({"schema_version":1,"record_kind":"image_inputs","entries":[{"role":"firmware","path":relative(&run.root,firmware)?,"sha256":file_hash(firmware)?},{"role":"kernel","path":relative(&run.root,kernel)?,"sha256":file_hash(kernel)?},{"role":"initfs","path":relative(&run.root,initfs)?,"sha256":file_hash(initfs)?}]});
    let inspected = image::validate_inputs(&run.root, &inputs)?;
    let directory = run.generated_directory(&format!("out/images/{}-{variant}", run.id))?;
    let record = image::stage_inputs(&directory, &inspected)?;
    write_json(&directory.join("image.json"), &record)?;
    relative(&run.root, &directory.join("image.json"))
}
/// The `desktop-build.json` of the run that staged the shipping desktop
/// `image` (`test ui --suite ... --image`, and suites that boot a sibling
/// image of the same build).
pub fn desktop_build_of(root: &Path, image: &str) -> Result<Value, String> {
    let wanted = image.replace(std::path::MAIN_SEPARATOR, "/");
    let run_directory = root.join(&wanted).ancestors().nth(4).map(Path::to_path_buf);
    let candidates = [
        run_directory.map(|d| d.join("desktop-build.json")),
        Some(root.join("out/latest-desktop-image.json")),
    ];
    for path in candidates.into_iter().flatten() {
        if let Ok(build) = image::read_json(&path)
            && build["image"].as_str() == Some(wanted.as_str())
        {
            return Ok(build);
        }
    }
    Err(format!(
        "no desktop-build.json records {image}: pass a staged reports/runs/<id>/generated/images/desktop/image.json"
    ))
}
/// The desktop-diagnostics image staged with `image` (a11y-01 and the
/// component gallery read Captions' serial trees there).
pub fn diagnostics_image(root: &Path, image: &str) -> Result<String, String> {
    desktop_build_of(root, image)?["diagnostics_image"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "desktop build has no diagnostics image; rebuild the desktop image".into())
}
pub fn start(run: &mut Run, image_override: Option<&str>, visible: bool) -> Result<Value, String> {
    let image = match image_override {
        Some(path) => path.to_owned(),
        None => image::read_json(&run.root.join("out/latest-session-image.json"))?["image"]
            .as_str()
            .ok_or("Build the text image first: cargo xtask build --profile text-qemu-x64")?
            .to_owned(),
    };
    let owner = run.root.join("out/session");
    fs::create_dir_all(&owner).map_err(|e| e.to_string())?;
    let disk = owner.join("user.raw");
    if !disk.exists() {
        let bytes = crate::persistence::format::image(false);
        fs::write(&disk, bytes).map_err(|e| e.to_string())?;
        write_json(
            &owner.join("volume.json"),
            &json!({"schema_version":1,"purpose":"persistent owner data","disk":"out/session/user.raw","created_by":run.id}),
        )?;
    }
    launch(run, &image, &disk, visible)
}
pub fn launch(run: &mut Run, image: &str, disk: &Path, visible: bool) -> Result<Value, String> {
    launch_profile(run, image, disk, visible, false, None)
}
pub fn launch_desktop(
    run: &mut Run,
    image: &str,
    disk: &Path,
    visible: bool,
) -> Result<Value, String> {
    // A predictable host window, at native pixel size. Monitor-sized output at
    // 100% DPI made the controls tiny and forced users into fullscreen.
    launch_profile(
        run,
        image,
        disk,
        visible,
        true,
        visible.then_some("1280x720"),
    )
}
pub fn launch_desktop_wide(
    run: &mut Run,
    image: &str,
    disk: &Path,
    visible: bool,
) -> Result<Value, String> {
    launch_profile(run, image, disk, visible, true, Some("1280x720"))
}
pub fn launch_desktop_resolution(
    run: &mut Run,
    image: &str,
    disk: &Path,
    visible: bool,
    resolution: &str,
) -> Result<Value, String> {
    if !supported_resolution(resolution) {
        return Err("Supported desktop modes: 1024x768, 1280x720, 1920x1080, 1920x1200, 2560x1440, 2560x1600, 3840x2160".into());
    }
    launch_profile(run, image, disk, visible, true, Some(resolution))
}
fn supported_resolution(mode: &str) -> bool {
    matches!(
        mode,
        "1024x768"
            | "1280x720"
            | "1920x1080"
            | "1920x1200"
            | "2560x1440"
            | "2560x1600"
            | "3840x2160"
    )
}
/// Hardware acceleration is explicit and has its own recorded profile. Never
/// relabel a software-emulation regression as an accelerated measurement.
/// A scenario's next sign-in as another local account: the sign-in
/// screen's user tile (physical point, reached by relative pointer moves
/// from `start`, where the pointer rests at boot) and that account's
/// password, typed once the gate shows its password box.
pub struct SignInAs {
    pub tile: (i32, i32),
    pub start: (i32, i32),
    pub password: String,
}
static SIGN_IN_AS: std::sync::Mutex<Option<SignInAs>> = std::sync::Mutex::new(None);
/// Set (or clear) the next desktop launch's sign-in account.
pub fn sign_in_as(next: Option<SignInAs>) {
    *SIGN_IN_AS.lock().unwrap_or_else(|e| e.into_inner()) = next;
}
/// Click the user tile and type the password (`sign_in_as`).
fn answer_as(login: &mut qmp::Client, serial: &Path, who: &SignInAs) -> Result<(), String> {
    let mut at = who.start;
    while at != who.tile {
        let dx = (who.tile.0 - at.0).clamp(-50, 50);
        let dy = (who.tile.1 - at.1).clamp(-50, 50);
        login.execute(
            "input-send-event",
            json!({"events":[{"type":"rel","data":{"axis":"x","value":dx}},{"type":"rel","data":{"axis":"y","value":dy}}]}),
        )?;
        at.0 += dx;
        at.1 += dy;
        thread::sleep(Duration::from_millis(70));
    }
    let before = fs::read_to_string(serial).unwrap_or_default().len();
    for down in [true, false] {
        login.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":down,"button":"left"}}]}),
        )?;
        thread::sleep(Duration::from_millis(200));
    }
    // The gate switched account: its password box.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let text = fs::read_to_string(serial).unwrap_or_default();
        if text
            .get(before..)
            .unwrap_or("")
            .contains("DOOR_P14 SIGNIN surface=signin kind=password")
        {
            break;
        }
        if Instant::now() > deadline {
            return Err("the sign-in screen never switched to the other account".into());
        }
        thread::sleep(Duration::from_millis(100));
    }
    for ch in who.password.chars().chain(core::iter::once('\n')) {
        login.execute("send-key", json!({"keys":keys(ch)?,"hold-time":60}))?;
        thread::sleep(Duration::from_millis(120));
    }
    Ok(())
}
pub fn launch_desktop_accelerated(
    run: &mut Run,
    image: &str,
    disk: &Path,
    visible: bool,
    resolution: &str,
) -> Result<Value, String> {
    if !supported_resolution(resolution) {
        return Err("Unsupported desktop resolution".into());
    }
    launch_profile_hardware(
        run,
        image,
        disk,
        visible,
        true,
        Some(resolution),
        LaunchProfile::Fixed("profiles/hardware/qemu-desktop-whpx-x64.json"),
    )
}
/// `launch_desktop_accelerated_v2` on the 2-CPU desktop profile (P18: the
/// boot CPU runs everything; programs see two CPUs).
pub fn launch_desktop_accelerated_smp2(
    run: &mut Run,
    image: &str,
    disk: &Path,
    visible: bool,
    resolution: &str,
) -> Result<Value, String> {
    if !supported_resolution(resolution) {
        return Err("Unsupported desktop resolution".into());
    }
    launch_profile_hardware(
        run,
        image,
        disk,
        visible,
        true,
        Some(resolution),
        LaunchProfile::Cpu(
            "profiles/hardware/qemu-desktop-whpx-smp2-x64.json",
            "Nehalem-v1",
        ),
    )
}
/// `launch_desktop_accelerated` on an x86-64-v2 processor (QEMU's
/// Nehalem: SSSE3, SSE4.1/4.2, POPCNT): the baseline .NET 11 requires and
/// every PC Doors targets has. The desktop profile itself keeps its
/// historical CPU model; the session record's argv names the one used.
pub fn launch_desktop_accelerated_v2(
    run: &mut Run,
    image: &str,
    disk: &Path,
    visible: bool,
    resolution: &str,
) -> Result<Value, String> {
    if !supported_resolution(resolution) {
        return Err("Unsupported desktop resolution".into());
    }
    launch_profile_hardware(
        run,
        image,
        disk,
        visible,
        true,
        Some(resolution),
        LaunchProfile::Cpu("profiles/hardware/qemu-desktop-whpx-x64.json", "Nehalem-v1"),
    )
}
fn launch_profile(
    run: &mut Run,
    image: &str,
    disk: &Path,
    visible: bool,
    desktop: bool,
    resolution: Option<&str>,
) -> Result<Value, String> {
    launch_profile_hardware(
        run,
        image,
        disk,
        visible,
        desktop,
        resolution,
        LaunchProfile::Fixed("profiles/hardware/qemu-services-x64.json"),
    )
}
enum LaunchProfile<'a> {
    Fixed(&'a str),
    Interactive(&'a DesktopOptions),
    /// A fixed profile run with another QEMU CPU model (P18: x86-64-v2).
    Cpu(&'a str, &'a str),
}
/// A free loopback port for one guest's QMP server. The system picks it (as
/// before), and this process never hands the same port to two launches, so
/// concurrent guest workers cannot both be given one port between this probe
/// and QEMU binding it.
fn reserve_qmp_port() -> Result<u16, String> {
    static GIVEN: std::sync::Mutex<std::collections::BTreeSet<u16>> =
        std::sync::Mutex::new(std::collections::BTreeSet::new());
    let mut given = GIVEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // Holding earlier probes open makes the system offer a different port.
    let mut held = Vec::new();
    for _ in 0..64 {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        if given.insert(port) {
            return Ok(port);
        }
        held.push(listener);
    }
    Err("no unused loopback port for QMP".into())
}
fn launch_profile_hardware(
    run: &mut Run,
    image: &str,
    disk: &Path,
    visible: bool,
    desktop: bool,
    resolution: Option<&str>,
    profile: LaunchProfile<'_>,
) -> Result<Value, String> {
    // A suite launches multiple independent boots under one evidence run.
    // Each needs private firmware variables, serial output and a QMP identity.
    // Concurrent guest workers share a run identity, so the stamp is unique
    // within this process as well as increasing.
    let stamp = crate::evidence::unique_stamp()?;
    let instance = format!("{}-session-{stamp}", run.id);
    let hardware = match &profile {
        LaunchProfile::Fixed(path) | LaunchProfile::Cpu(path, _) => *path,
        LaunchProfile::Interactive(options) => options.hardware_profile(),
    };
    let prepared = emulator::prepare_named(run, hardware, image, &instance)?;
    let port = reserve_qmp_port()?;
    let hash = format!("{:x}", Sha256::digest(instance.as_bytes()));
    let uuid = format!(
        "{}-{}-{}-{}-{}",
        &hash[..8],
        &hash[8..12],
        &hash[12..16],
        &hash[16..20],
        &hash[20..32]
    );
    let mut args = Vec::new();
    let mut iter = prepared.args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "-no-reboot" {
            continue;
        }
        if arg == "-cpu"
            && let LaunchProfile::Cpu(_, model) = &profile
        {
            let _ = iter.next();
            args.extend([arg, (*model).into()]);
            continue;
        }
        if arg == "-fw_cfg" {
            let _ = iter.next();
            continue;
        }
        if arg == "-device" {
            let value = iter.next().ok_or("device argument")?;
            if value.starts_with("isa-debug-exit") {
                continue;
            }
            if desktop && value == "VGA" {
                if prepared.profile["display"]["device"] == "ramfb" {
                    args.extend([arg, "ramfb".into()]);
                    continue;
                }
                let mode = resolution.unwrap_or("1024x768");
                let (width, height) = mode.split_once('x').ok_or("desktop mode")?;
                args.extend([arg, format!("VGA,vgamem_mb=64,xres={width},yres={height}")]);
                continue;
            }
            args.extend([arg, value]);
            continue;
        }
        if arg == "-display" {
            let _ = iter.next();
            args.extend([arg, if visible { "sdl" } else { "none" }.into()]);
            continue;
        }
        // P15: desktops get a NIC (below) instead of "-nic none".
        if desktop && arg == "-nic" {
            let _ = iter.next();
            continue;
        }
        args.push(arg);
    }
    if let Some(resolution) = resolution {
        args.extend([
            "-fw_cfg".into(),
            format!("name=opt/door/display,string={resolution}"),
        ]);
    }
    // Host display scale hint (W8, spec §57): interactive auto launches only.
    if let LaunchProfile::Interactive(options) = &profile
        && let Some(percent) = options.scale_hint()
    {
        args.extend([
            "-fw_cfg".into(),
            format!("name=opt/door/scale,string={percent}"),
        ]);
    }
    args.extend(emulator::storage_arguments(&prepared.profile, disk)?);
    if desktop {
        args.extend(emulator::network_arguments());
        // The user hears a visible session; tests capture what was played.
        let wav = prepared.serial.with_file_name("sound.wav");
        args.extend(emulator::audio_arguments(
            (!visible).then_some(wav.as_path()),
        )?);
    }
    args.extend([
        "-name".into(),
        if desktop {
            "Doors OS - desktop"
        } else {
            "Doors OS - native shell"
        }
        .into(),
        "-uuid".into(),
        uuid.clone(),
        "-qmp".into(),
        format!("tcp:127.0.0.1:{port},server=on,wait=off"),
    ]);
    // Bind the user's requested backend/display/mode to the actual constructed
    // process arguments before spawning or handing off any owner session.
    if let LaunchProfile::Interactive(options) = profile {
        options.validate_argv(&args)?;
    }
    let root = run.root.clone();
    let viewer_environment = if visible && desktop && cfg!(windows) {
        desktop_options::NATIVE_PIXEL_ENV
    } else {
        &[]
    };
    let output_layout = run.output_layout.declaration(&run.id);
    let layout_owner = run.id.clone();
    let record=run.controlled_expected_env(prepared.qemu.to_str().ok_or("QEMU path")?,&args,&[(&prepared.serial,8*1024*1024)],0,viewer_environment,|child|{
        let deadline=Instant::now()+Duration::from_secs(90);
        let mut signed_in=false;
        loop {
            let text=fs::read_to_string(&prepared.serial).unwrap_or_default();
            if desktop && text.contains("DOOR_P12 SIGNIN DISPLAY READY") && text.contains("DOOR_P12 SIGNIN AUTHORITY READY") && !signed_in {
                if visible {break;}
                if text.contains("SESSION_READY") || text.contains("APP_LAUNCHED") {return Err("Session admitted before trusted sign-in gesture".into());}
                // P14 credential gate: first-run enrollment or sign-in with
                // the test PIN of this private test volume (never the owner's).
                if let Some(keys)=crate::desktop::sign_in_keys(&text) {
                    let keys=keys?;
                    let mut login=qmp::Client::connect(port)?;
                    let screen=prepared.private.join("signin.png");
                    login.execute("screendump",json!({"filename":screen.to_string_lossy().trim_start_matches("\\\\?\\").replace('\\',"/"),"format":"png"}))?;
                    let other=SIGN_IN_AS.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(who)=other.as_ref() {
                        answer_as(&mut login,&prepared.serial,who)?;
                    } else {
                        for key in &keys {
                            login.execute("send-key",json!({"keys":[{"type":"qcode","data":key}],"hold-time":60}))?;
                            thread::sleep(Duration::from_millis(120));
                        }
                    }
                    signed_in=true;
                }
            }
            if if desktop{text.contains("DOOR_P11 READY")}else{text.contains("Type help.") && text.contains("/users/1001> ")} {break}
            if text.contains("KERNEL_PANIC") || text.contains("INIT FAIL") || text.contains("Shell stopped:") {return Err("Ordinary session startup failed; inspect captured serial".into())}
            if let Some(exit)=child.poll()? {return Err(format!("Session exited during startup: {exit}"))}
            if Instant::now()>deadline {return Err("Ordinary session readiness timeout".into())}
            thread::sleep(Duration::from_millis(50));
        }
        let mut client=qmp::Client::connect(port)?;
        if client.execute("query-uuid",json!({}))?["UUID"]!=uuid {return Err("Session identity mismatch".into())}
        let screen=prepared.private.join("shell.png");
        client.execute("screendump",json!({"filename":screen.to_string_lossy().trim_start_matches("\\\\?\\").replace('\\',"/"),"format":"png"}))?;
        child.handoff()?;
        Ok(json!({"schema_version":1,"record_kind":"native_session","output_layout":output_layout,"layout_owner":layout_owner,"private_instance":instance,"state":"running","visible":visible,"viewer_environment":viewer_environment,"pid":child.id(),"qmp_port":port,"uuid":uuid,"image":image,"disk":relative(&root,disk)?,"serial":relative(&root,&prepared.serial)?,"screenshot":relative(&root,&screen)?,"argv":args}))
    })?;
    if desktop && !visible {
        let signin = prepared.private.join("signin.png");
        run.check(
            "desktop-trusted-signin",
            json!({
                "screenshot": relative(&run.root, &signin)?,
                "sha256": file_hash(&signin)?,
                "session_admitted_before_gesture": false,
                "gesture": "QMP keys answering the P14 credential gate (first-run PIN enrollment or sign-in) after trusted display and authority readiness",
                "scope": "test PIN of a private test volume; Argon2id record verified by the session service (P14-LOGON)"
            }),
        );
    }
    write_json(&run.directory.join("native-session.json"), &record)?;
    let mut session = record.clone();
    session["signin_screenshot"] =
        json!(relative(&run.root, &prepared.private.join("signin.png"))?);
    run.record_session(&session);
    if disk == run.root.join("out/session/user.raw") {
        write_json(
            &run.root.join(if desktop {
                "out/native-desktop.json"
            } else {
                "out/native-session.json"
            }),
            &record,
        )?;
    }
    println!(
        "Native {} running; user disk {}",
        if desktop { "desktop" } else { "shell" },
        disk.display()
    );
    Ok(record)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_transcripts_ignore_only_unfinished_records_including_utf8() {
        let first = "DOOR_CONSOLE bytes=3 text=\"\u{e9}\\n\"\n";
        let stream = format!("{first}DOOR_CONSOLE bytes=1 text=\"x\"\n");
        for end in 0..stream.len() {
            let expected = if end < first.len() { "" } else { "\u{e9}\n" };
            assert_eq!(
                decode_transcript(&stream.as_bytes()[..end]).unwrap(),
                expected
            );
        }
        assert_eq!(decode_transcript(stream.as_bytes()).unwrap(), "\u{e9}\nx");
        assert!(decode_transcript(b"DOOR_CONSOLE bytes=1 text=\"broken\n").is_err());
        let mut record = String::new();
        let text = "\0\x08\x1b\x7f\"\\\u{e9}\n";
        door_boot_debug::console_record(&mut record, text).unwrap();
        assert_eq!(decode_transcript(record.as_bytes()).unwrap(), text);
    }
}

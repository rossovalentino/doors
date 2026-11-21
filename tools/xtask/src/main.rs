mod app_assets;
mod artifact;
mod assets;
mod boot;
mod boot_gate;
mod build;
mod cache;
mod demo;
mod desktop;
mod doctor;
mod emulator;
mod evidence;
mod fidelity;
mod fuzz;
mod image;
#[cfg(test)]
#[path = "../../../tests/support/images.rs"]
mod image_fixtures;
mod ime_dictionaries;
mod initfs;
mod layout;
mod parallel;
mod paths;
mod persistence;
mod phase;
mod qmp;
mod scenarios;
mod services;
mod session;
mod spec;
mod storage;
#[cfg(test)]
mod test_support;
mod typography;
mod windows;

use std::{env, path::PathBuf, process::ExitCode};

use evidence::Run;

fn main() -> ExitCode {
    match dispatch() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ERROR: {error}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch() -> Result<(), String> {
    let arguments: Vec<_> = env::args().skip(1).collect();
    if arguments.is_empty() || arguments == ["--help"] || arguments == ["help"] {
        println!(
            "First run (creates a new persistent disk only, never overwrites):\n  cargo xtask init-user-volume\nThen run the existing image:\n  cargo xtask desktop"
        );
        println!(
            "Door OS host tooling\n\n  cargo xtask doctor\n  cargo xtask assets build|check\n  cargo xtask build --profile desktop-qemu-x64\n  cargo xtask desktop [--headless] [--accelerator whpx|tcg] [--resolution auto|WIDTHxHEIGHT]\n  cargo xtask test desktop --image <production-image.json> --test-image <test-image.json>\n  cargo xtask check-emulator\n  cargo xtask check-spec\n  cargo xtask build --profile text-qemu-x64\n  cargo xtask session\n  cargo xtask session --headless\n  cargo xtask session send \"<line>\"\n  cargo xtask session stop\n  cargo xtask test session --image <production-image.json>\n  cargo xtask test host\n  cargo xtask test boot\n  cargo xtask test services --image <relative-image.json>\n  cargo xtask demo\n  cargo xtask demo --profile services --image <relative-image.json> --disk <saved-data.raw>\n  cargo xtask stop-demo\n  cargo xtask verify\n  cargo xtask plan --profile dev-qemu-x64\n  cargo xtask build --profile host-dev\n  cargo xtask build --profile dev-qemu-x64\n  cargo xtask inspect --class <class> <relative-file>\n  cargo xtask image --manifest <relative-file>\n  cargo xtask run --profile <relative-file> --image <relative-image.json>\n  cargo xtask disk create [--profile p07|p07-small|p08]\n  cargo xtask disk inspect --state initial|persisted|p07|p08 <relative-data.raw>\n  cargo xtask run --profile <storage-profile> --image <relative-image.json> --disk <relative-data.raw> --mode storage-write|storage-reopen\n\nEvidence: reports/runs/. Unavailable guest prerequisites fail explicitly."
        );
        println!(
            "\nInteractive desktop selects the supported host adapter mode and WHPX/RAMFB (Windows Hypervisor Platform required):\n  cargo xtask desktop --resolution 3840x2160\n  cargo xtask desktop --accelerator tcg\n  cargo xtask test desktop-default --image <production-image.json>\n  cargo xtask test desktop-software --image <production-image.json>\n  cargo xtask test desktop-accelerated --image <production-image.json>"
        );
        println!(
            "\nCurrent desktop build and all native workflows:\n  cargo xtask test desktop-current\nRetained evidence only (no new guest execution):\n  cargo xtask revalidate desktop --report reports/runs/<execution-id>/report.json"
        );
        println!(
            "\nGate composition only (builds and runs nothing):\n  cargo xtask verify --list [--phase P13|P14]\nCumulative gate for an owner-authorized parallel phase (P13 includes all of P12; P14 includes all of P13):\n  cargo xtask verify --phase P13\n  cargo xtask verify --phase P14\nGuest work runs on DOOR_GUEST_JOBS workers (default from free memory, cores and disk; 1 = sequential).\nRe-run named gate steps, desktop scenarios and suites of a staged build on the guest pool (certifies nothing):\n  cargo xtask test guest-batch --phase P14 --image <image.json> --only desktop-recovery,monitor,a11y-01\nDeclared acceptance suites (fail as not implemented until a runner is registered):\n  cargo xtask test ui --suite text-01|a11y-01|display-01|motion-01|component-gallery\n  cargo xtask test compat --suite win32-gui-f1|win32-gui-f2|win32-gui-f3|scenario-141-5|scenario-141-6\n  cargo xtask test desktop --suite desktop-a-shell|recover-01|desktop-a-files|desktop-a-apps|sec-02-trusted|vis-01\nWindows GUI fixtures: double build, import inventory and the read-only Windows reference run:\n  cargo xtask build --profile windows-gui-reference"
        );
        println!(
            "\nPrivate 4K paint measurements (paced gestures; no FPS claim):\n  cargo xtask test desktop-motion --image <production-image.json>"
        );
        println!(
            "Rebuild and run only the private motion workflow (not the full desktop gate):\n  cargo xtask test desktop-motion-current"
        );
        println!(
            "\nOut-only 32 GB budget (target/reports excluded):\n  cargo xtask cache status\n  cargo xtask cache maintain"
        );
        println!(
            "\nAcquire an installed reference explicitly (offline exports stay pinned):\n  cargo xtask assets acquire-icon <name> --module <resource-module> --id <numeric-group-id>\n  cargo xtask assets acquire-icon <name> --png-dir <directory> --stem <targetsize-stem>\n  cargo xtask assets list-icons --module <resource-module>\n  cargo xtask assets acquire-commands --dir <FileExplorerExtensions Assets/images>"
        );
        return Ok(());
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    // IME dictionaries: pinned upstream sources, converted on the host.
    if arguments.first().is_some_and(|a| a == "ime-dictionaries") {
        return ime_dictionaries::run(&root, &arguments[1..]);
    }
    if arguments == ["cache", "status"] {
        println!(
            "{}",
            serde_json::to_string_pretty(&cache::status(&root)?).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    // Host-only previews read already-rendered pixels and write only a fresh
    // report directory; they never build, launch or mutate shared output/assets.
    if arguments == ["assets", "reference-apps-preview"] {
        let mut run = Run::new(root)?;
        let result = desktop::reference_apps_preview(&mut run);
        run.finish(result.as_ref().err().map(String::as_str))?;
        return result;
    }
    if arguments == ["assets", "explorer-preview"] {
        let mut run = Run::new(root)?;
        let result = desktop::explorer_preview(&mut run);
        run.finish(result.as_ref().err().map(String::as_str))?;
        return result;
    }
    if arguments == ["assets", "terminal-preview"] {
        let mut run = Run::new(root)?;
        let result = desktop::terminal_preview(&mut run);
        run.finish(result.as_ref().err().map(String::as_str))?;
        return result;
    }
    if arguments == ["assets", "settings-preview"] {
        let mut run = Run::new(root)?;
        let result = desktop::settings_preview(&mut run);
        run.finish(result.as_ref().err().map(String::as_str))?;
        return result;
    }
    // Read-only gate composition: no build, guest run, report or output lease.
    match arguments
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["verify", "--list"] => return verify_plan(&root, None),
        ["verify", "--list", "--phase", phase] => return verify_plan(&root, Some(phase)),
        _ => (),
    }
    // A crash releases the OS lock. In-process child Run ledgers reuse this
    // lease; they do not spawn another xtask or acquire it recursively.
    let _output_lease = if cfg!(windows) {
        Some(cache::Lease::acquire(&root)?)
    } else {
        None
    };
    // Export intentionally updates versioned assets. Qualification is a separate
    // read-only `assets check`; do not label this mutation an unchanged-input run.
    if arguments == ["assets", "build"] {
        return assets::export(&root, false);
    }
    // Run by `cargo host-assets` (tools/host-assets), which first lets the
    // workspace compile without the Microsoft-owned files.
    if arguments == ["host-assets", "acquire"] {
        return assets::host::acquire(&root);
    }
    if arguments == ["host-assets", "export"] {
        return assets::host::export(&root);
    }
    if arguments == ["assets", "advanced", "build"] {
        return assets::advanced::export(&root, false);
    }
    if arguments == ["assets", "rounded", "build"] {
        return assets::rounded_coverage(&root, false);
    }
    if arguments == ["assets", "branding", "build"] {
        return assets::export_branding(&root, false);
    }
    if arguments == ["assets", "typography", "build"] {
        return typography::export(&root, false);
    }
    if arguments == ["assets", "shell", "build"] {
        return assets::shell::export(&root, false);
    }
    if arguments == ["assets", "folders", "build"] {
        return assets::folders::export(&root, false);
    }
    if arguments == ["assets", "windows", "build"] {
        return assets::export_windows(&root, false);
    }
    match arguments
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["assets", "list-icons", "--module", module] => return assets::acquire::list(module),
        ["assets", "acquire-commands", "--dir", dir] => {
            return assets::commands::acquire(&root, dir);
        }
        ["assets", "acquire-paint-icons", "--dir", dir] => {
            return assets::paint_icons::acquire(&root, dir);
        }
        [
            "assets",
            "acquire-icon",
            name,
            "--module",
            module,
            "--name",
            group,
        ] => return assets::acquire::named_resource(&root, name, module, group),
        [
            "assets",
            "acquire-icon",
            name,
            "--module",
            module,
            "--id",
            id,
        ] => {
            return assets::acquire::resource(
                &root,
                name,
                module,
                id.parse().map_err(|_| "resource ID must be16-bit")?,
            );
        }
        [
            "assets",
            "acquire-icon",
            name,
            "--png-dir",
            directory,
            "--stem",
            stem,
        ] => return assets::acquire::pngs(&root, name, directory, stem, ""),
        // Packaged apps' unplated taskbar/Start artwork, e.g. `--suffix
        // _altform-unplated` (`{stem}.targetsize-{n}{suffix}.png`).
        [
            "assets",
            "acquire-icon",
            name,
            "--png-dir",
            directory,
            "--stem",
            stem,
            "--suffix",
            suffix,
        ] => return assets::acquire::pngs(&root, name, directory, stem, suffix),
        _ => (),
    }
    let mut run = Run::new(root)?;
    run.output_layout = layout::Layout::for_command(&arguments);
    let budgeted = !matches!(
        arguments
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice(),
        ["session", "stop"] | ["session", "send", _] | ["test", "host"] | ["disk", "inspect", ..]
    ) && matches!(
        arguments.first().map(String::as_str),
        Some(
            "build" | "desktop" | "session" | "demo" | "run" | "image" | "disk" | "verify" | "test"
        )
    );
    if budgeted && let Err(error) = assets::host::require(&run.root) {
        run.finish(Some(&error))?;
        return Err(error);
    }
    if budgeted && let Err(error) = cache::admit(&mut run) {
        run.finish(Some(&error))?;
        return Err(error);
    }
    let result = match arguments
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["init-user-volume"] => session::owner_volume::initialize(&mut run),
        ["init-apps-volume"] => session::apps_volume::initialize(&mut run),
        ["doctor"] => doctor::discover(&mut run),
        ["fuzz", rest @ ..] => fuzz::run(&mut run, rest),
        ["cache", "maintain"] => cache::maintain(&mut run, true).and_then(|receipt| {
            println!(
                "{}",
                serde_json::to_string_pretty(&receipt).map_err(|e| e.to_string())?
            );
            cache::within_limit(&run.root)
        }),
        ["check-emulator"] => {
            emulator::preflight(&mut run, "profiles/hardware/qemu-diagnostic-x64.json").map(|_| ())
        }
        ["check-spec"] => spec::check(&run.root),
        ["assets", "check"] => assets::export(&run.root, true),
        ["assets", "typography-preview"] => typography::preview(&mut run),
        ["assets", "terminal-preview"] => desktop::terminal_preview(&mut run),
        ["assets", "rounded", "check"] => assets::rounded_coverage(&run.root, true),
        ["test", "host"] => host_tests(&mut run),
        ["test", "desktop-current"] => current_desktop(&mut run),
        ["test", "desktop-motion-current"] => current_motion(&mut run),
        ["test", "explorer", "--image", image] => desktop::explorer_modern_test(&mut run, image),
        ["test", "explorer-development"] => {
            let image = session::refresh::desktop(&mut run, "out/latest-desktop-image.json")?;
            desktop::explorer_modern_test(&mut run, &image)
        }
        ["test", "explorer-current"] => {
            assets::export(&run.root, true)?;
            let image = session::build_desktop(&mut run)?;
            desktop::explorer_modern_test(&mut run, &image)
        }
        ["test", "multiwindow", "--image", image] => desktop::multiwindow_test(&mut run, image),
        ["test", "multiwindow-pressure", "--image", image] => {
            desktop::multiwindow_pressure_test(&mut run, image)
        }
        ["test", "monitor", "--image", image] => desktop::monitor_test(&mut run, image),
        ["test", "preview-monitor-current"] => {
            assets::export(&run.root, true)?;
            let image = session::build_desktop(&mut run)?;
            desktop::preview_test(&mut run, &image)?;
            desktop::monitor_test(&mut run, &image)
        }
        ["test", "preview", "--image", image] => desktop::preview_test(&mut run, image),
        ["test", "preview-current"] => {
            assets::export(&run.root, true)?;
            let image = session::build_desktop(&mut run)?;
            desktop::preview_test(&mut run, &image)
        }
        ["test", "terminal-current"] => {
            assets::export(&run.root, true)?;
            let image = session::build_desktop(&mut run)?;
            desktop::terminal_test(&mut run, &image)
        }
        ["test", "terminal-development"] => {
            let image = session::refresh::desktop(&mut run, "out/latest-desktop-image.json")?;
            desktop::terminal_test(&mut run, &image)
        }
        ["test", "terminal", "--image", image] => desktop::terminal_test(&mut run, image),
        ["test", "settings-reference-current"] => {
            assets::export(&run.root, true)?;
            let image = session::build_desktop(&mut run)?;
            desktop::settings_reference_test(&mut run, &image)
        }
        ["test", "settings-reference", "--image", image] => {
            desktop::settings_reference_test(&mut run, image)
        }
        ["test", "settings-reference-development"] => {
            let image = session::refresh::desktop(&mut run, "out/latest-desktop-image.json")?;
            desktop::settings_reference_test(&mut run, &image)
        }
        ["revalidate", "desktop", "--report", report] => desktop::revalidate(&mut run, report),
        ["revalidate", "boot", "--report", report] => boot_gate::revalidate(&mut run, report),
        ["test", "integrated-desktop"] => integrated_desktop(&mut run),
        ["test", "services", "--image", image] => services::verify::run(&mut run, image),
        [
            "test",
            "storage-connect-sdk",
            "--image",
            image,
            "--baseline",
            baseline,
        ] => services::reconnect::run(&mut run, image, baseline),
        [
            "test",
            "files-sdk",
            "--image",
            image,
            "--baseline",
            baseline,
        ] => services::demo::checked_rename_test(&mut run, image, baseline),
        [
            "test",
            "document-sdk",
            "--image",
            image,
            "--baseline",
            baseline,
        ] => services::checked_replace::run(&mut run, image, baseline),
        [
            "test",
            "remove-sdk",
            "--image",
            image,
            "--baseline",
            baseline,
        ] => services::checked_remove::run(&mut run, image, baseline),
        ["test", "session", "--image", image] => session::verify::run(&mut run, image),
        ["test", "desktop", "--image", image] => desktop::verify(&mut run, image),
        ["test", "p15-desktop", "--image", image] => desktop::p15_desktop_test(&mut run, image),
        ["serve", "p15-connect"] => desktop::p15_connect_serve(&mut run),
        ["test", "p15-connect", "--image", image] => desktop::p15_connect_test(&mut run, image),
        ["test", "p15-sound", "--image", image] => desktop::p15_sound_test(&mut run, image),
        ["test", "p15-capture", "--image", image] => desktop::p15_capture_test(&mut run, image),
        ["test", "p15-vpn", "--image", image] => desktop::p15_vpn_test(&mut run, image),
        ["test", "p15-dsound", "--image", image] => desktop::p15_dsound_test(&mut run, image),
        ["test", "p15-midi", "--image", image] => desktop::p15_midi_test(&mut run, image),
        ["test", "p15-wasapi", "--image", image] => desktop::p15_wasapi_test(&mut run, image),
        ["test", "p16-com", "--image", image] => desktop::p16_com_test(&mut run, image),
        ["test", "p16-oleaut", "--image", image] => desktop::p16_oleaut_test(&mut run, image),
        ["test", "p16-tsched", "--image", image] => desktop::p16_tsched_test(&mut run, image),
        ["test", "p16-crypt", "--image", image] => desktop::p16_crypt_test(&mut run, image),
        ["test", "p16-trust", "--image", image] => desktop::p16_trust_test(&mut run, image),
        ["test", "p16-paths", "--image", image] => desktop::p16_paths_test(&mut run, image),
        ["test", "p18-mem", "--image", image] => desktop::p18_mem_test(&mut run, image),
        ["test", "p18-crt", "--image", image] => desktop::p18_crt_test(&mut run, image),
        ["test", "p18-dotnet", "--image", image] => desktop::p18_dotnet_test(&mut run, image),
        ["test", "p18-pwsh", "--image", image] => desktop::p18_pwsh_test(&mut run, image),
        ["test", "p18-pwsh-b", "--image", image] => desktop::p18_pwsh_b_test(&mut run, image),
        ["test", "p18-electron", "--image", image] => desktop::p18_electron_test(&mut run, image),
        ["test", "p17-x86", "--image", image] => desktop::p17_x86_test(&mut run, image),
        ["test", "p17-wow64", "--image", image] => desktop::p17_wow64_test(&mut run, image),
        ["test", "p16-comoop", "--image", image] => desktop::p16_comoop_test(&mut run, image),
        ["test", "p16-shell", "--image", image] => desktop::p16_shell_test(&mut run, image),
        ["test", "p16-cmd", "--image", image] => desktop::p16_cmd_test(&mut run, image),
        ["test", "p16-install", "--image", image] => desktop::p16_install_test(&mut run, image),
        ["test", "p16-accounts", "--image", image] => desktop::p16_accounts_test(&mut run, image),
        ["test", "p16-scm", "--image", image] => desktop::p16_scm_test(&mut run, image),
        ["test", "p16-security", "--image", image] => desktop::p16_security_test(&mut run, image),
        ["test", "desktop-accelerated", "--image", image] => {
            desktop::accelerated_test(&mut run, image)
        }
        ["test", "desktop-default", "--image", image] => {
            desktop::interactive_test(&mut run, image, false)
        }
        ["test", "desktop-motion", "--image", image] => desktop::motion_test(&mut run, image),
        ["test", "desktop-software", "--image", image] => {
            desktop::interactive_test(&mut run, image, true)
        }
        [
            "test",
            "desktop",
            "--image",
            image,
            "--test-image",
            test_image,
        ] => desktop::verify_all(&mut run, image, test_image),
        ["test", "desktop-recovery", "--image", image] => desktop::recovery_test(&mut run, image),
        ["test", "desktop-files", "--image", image] => desktop::file_mutation_test(&mut run, image),
        ["test", "file-drag", "--image", image] => desktop::file_drag_test(&mut run, image),
        ["test", "desktop-remove", "--image", image] => {
            desktop::file_deletion_test(&mut run, image, false)
        }
        ["test", "desktop-remove-accelerated", "--image", image] => {
            desktop::file_deletion_test(&mut run, image, true)
        }
        ["test", "desktop-files-accelerated", "--image", image] => {
            desktop::file_mutation_accelerated_test(&mut run, image)
        }
        ["test", "desktop-notes", "--image", image] => desktop::notes_test(&mut run, image, false),
        ["test", "desktop-notes-accelerated", "--image", image] => {
            desktop::notes_test(&mut run, image, true)
        }
        ["test", "desktop-wide", "--image", image] => desktop::wide_test(&mut run, image),
        ["test", "desktop-shell", "--image", image] => desktop::shell_test(&mut run, image),
        ["test", "desktop-native-default", "--image", image] => {
            desktop::native_default_test(&mut run, image)
        }
        [
            "test",
            "desktop-resolutions",
            "--image",
            image,
            "--resolution",
            mode,
        ] => desktop::resolution_test_mode(&mut run, image, mode),
        ["test", "desktop-resolutions", "--image", image] => {
            desktop::resolution_test(&mut run, image)
        }
        ["build", "--profile", "windows-reference"] => windows::build::standalone(&mut run, false),
        ["build", "--profile", "windows-base"] => windows::base::standalone(&mut run),
        ["build", "--profile", "windows-gui-reference"] => windows::gui::standalone(&mut run),
        ["build", "--profile", "windows-artifacts"] => windows::build::standalone(&mut run, true),
        ["disk", "clone", "--profile", "p09", "--from", saved] => {
            windows::session::clone_saved(&run, saved).map(|_| ())
        }
        ["test", "boot", "--through", phase] => boot::verify(&mut run, phase),
        ["test", "boot"] => {
            let ledger = image::read_json(&run.root.join("project/phases.json"))?;
            boot::verify(
                &mut run,
                ledger["active_phase"]
                    .as_str()
                    .ok_or("active phase missing")?,
            )
        }
        ["stop-demo"] => demo::stop(&run),
        ["verify"] => verify(&mut run, None),
        ["verify", "--phase", phase] => verify(&mut run, Some(*phase)),
        [
            "test",
            "guest-batch",
            "--phase",
            phase,
            "--image",
            image,
            "--only",
            names,
        ] => guest_batch(&mut run, phase, image, names),
        [
            "test",
            family @ ("ui" | "compat" | "desktop"),
            "--suite",
            name,
        ] => acceptance_command(&mut run, family, name, None),
        [
            "test",
            family @ ("ui" | "compat" | "desktop"),
            "--suite",
            name,
            "--image",
            image,
        ] => acceptance_command(&mut run, family, name, Some(image)),
        ["build", "--profile", "text-qemu-x64"] => session::build(&mut run).map(|_| ()),
        ["build", "--profile", "desktop-qemu-x64"] => session::build_desktop(&mut run).map(|_| ()),
        ["build", "--profile", "desktop-dev", "--from", previous] => {
            session::refresh::desktop(&mut run, previous).map(|_| ())
        }
        ["desktop", ..] => {
            // `--apps`: the P18 apps volume (`init-apps-volume`) instead of
            // the owner's.
            let apps = arguments[1..].iter().any(|a| a == "--apps");
            let rest: Vec<_> = arguments[1..]
                .iter()
                .filter(|a| *a != "--apps")
                .cloned()
                .collect();
            let options = session::DesktopOptions::parse(&rest)?;
            let options = if apps {
                options.with_two_cpus()
            } else {
                options
            };
            let image = image::read_json(&run.root.join("out/latest-desktop-image.json"))?["image"]
                .as_str()
                .ok_or("Build desktop image first")?
                .to_owned();
            let disk = run.root.join(if apps {
                session::apps_volume::PATH
            } else {
                "out/session/user.raw"
            });
            if apps && !disk.exists() {
                return Err(
                    "Apps volume absent. Create it with cargo xtask init-apps-volume".into(),
                );
            }
            if !disk.exists() {
                return Err("Owner volume absent. Create a NEW persistent volume explicitly with cargo xtask init-user-volume; existing data is never overwritten".into());
            }
            options.launch(&mut run, &image, &disk).map(|_| ())
        }
        ["build", "--profile", "text-apps"] => {
            let target =
                paths::fresh_directory(&run.root, &format!("out/builds/{}/applications", run.id))?;
            session::applications::build(&mut run, &target).map(|_| ())
        }
        ["session"] => session::start(&mut run, None, true).map(|_| ()),
        ["session", "--headless"] => session::start(&mut run, None, false).map(|_| ()),
        ["session", "send", line] => session::send(&mut run, line),
        ["session", "stop"] => session::stop(&mut run),
        ["demo"] => session::start(&mut run, None, true).map(|_| ()),
        [
            "demo",
            "--profile",
            "services",
            "--image",
            image,
            "--disk",
            disk,
        ] => services::demo::launch(&mut run, image, disk, true, false).map(|_| ()),
        ["plan", "--profile", "dev-qemu-x64"] => build::write_plan(&mut run),
        ["build", "--profile", "host-dev"] => build::host_build(&mut run, false),
        ["build", "--profile", "dev-qemu-x64"] => build::guest_build(&mut run, false).map(|_| ()),
        ["build", "--profile", "dev-qemu-x64", "--through", phase] => {
            let selected = phase::Phase::parse(phase)?;
            build::guest_build_for(&mut run, false, selected).map(|_| ())
        }
        ["inspect", "--class", class, path] => artifact::command(&run, class, path),
        ["image", "--manifest", path] => image::stage(&mut run, path),
        ["run", "--profile", profile, "--image", image] => {
            emulator::command(&mut run, profile, image)
        }
        ["disk", "create"] => storage::create(&run).map(|_| ()),
        ["disk", "create", "--profile", "p07"] => persistence::create(&run).map(|_| ()),
        ["disk", "create", "--profile", "p07-small"] => {
            persistence::create_size(&run, true).map(|_| ())
        }
        ["disk", "create", "--profile", "p08"] => services::create(&run).map(|_| ()),
        ["disk", "inspect", "--state", "p08", path] => services::record_audit(&mut run, path),
        ["disk", "inspect", "--state", "p07", path] => {
            let checked = persistence::inspect(&paths::existing(&run.root, path)?)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&checked).map_err(|e| e.to_string())?
            );
            run.check("p07-persistence-audit", checked);
            Ok(())
        }
        ["disk", "inspect", "--state", state, path]
            if matches!(*state, "initial" | "persisted") =>
        {
            let checked =
                storage::inspect(&paths::existing(&run.root, path)?, *state == "persisted")?;
            println!(
                "{}",
                serde_json::to_string_pretty(&checked).map_err(|e| e.to_string())?
            );
            run.check("p06-storage-audit", checked);
            Ok(())
        }
        [
            "run",
            "--profile",
            profile,
            "--image",
            image,
            "--disk",
            disk,
            "--mode",
            mode,
        ] if matches!(*mode, "storage-write" | "storage-reopen")
            || persistence::is_mode(mode)
            || services::is_mode(mode)
            || windows::is_mode(mode) =>
        {
            let outcome = emulator::scenario_with_disk(&mut run, profile, image, mode, Some(disk))?;
            if matches!(
                outcome,
                emulator::Outcome::Passed | emulator::Outcome::Interrupted
            ) {
                Ok(())
            } else {
                Err(format!("storage guest {outcome:?}"))
            }
        }
        _ => Err("unknown command; use cargo xtask --help".into()),
    };
    let maintenance = if budgeted {
        cache::maintain(&mut run, true).and_then(|_| cache::within_limit(&run.root))
    } else {
        Ok(())
    };
    let result = result.and(maintenance);
    run.finish(result.as_ref().err().map(String::as_str))?;
    result
}

fn host_tests(run: &mut Run) -> Result<(), String> {
    run.required(
        "cargo",
        &["test", "--workspace", "--locked", "--all-targets"],
    )?;
    Ok(())
}

/// Current-source regressions through the implemented foundation plus the active
/// desktop development batch. This deliberately does not certify/close P12.
fn integrated_desktop(run: &mut Run) -> Result<(), String> {
    spec::check(&run.root)?;
    for argv in phase_commands("P11") {
        run.required(argv[0], &argv[1..])?;
    }
    boot::verify(run, "P11")?;
    let text = session::build(run)?;
    session::verify::run(run, &text)?;
    assets::export(&run.root, true)?;
    let image = session::build_desktop(run)?;
    let build = image::read_json(&run.directory.join("desktop-build.json"))?;
    desktop::verify_all(
        run,
        &image,
        build["test_image"].as_str().ok_or("desktop test image")?,
    )?;
    desktop::validate(
        &run.root,
        &serde_json::json!({"id":run.id,"checks":run.checks(),"commands":run.commands(),"source_snapshot":run.build_inputs()}),
    )?;
    println!("Current desktop integration regressions passed; P12 qualification remains partial.");
    Ok(())
}

/// Rebuild and exercise the focused private 4K motion workflow only.
fn current_motion(run: &mut Run) -> Result<(), String> {
    assets::export(&run.root, true)?;
    let image = session::build_desktop(run)?;
    desktop::motion_test(run, &image)
}

/// Reproducible current desktop build and all native consumer workflows, without
/// representing the separately recorded foundation or historical phases as rerun.
fn current_desktop(run: &mut Run) -> Result<(), String> {
    assets::export(&run.root, true)?;
    let image = session::build_desktop(run)?;
    let build = image::read_json(&run.directory.join("desktop-build.json"))?;
    desktop::verify_all(
        run,
        &image,
        build["test_image"].as_str().ok_or("desktop test image")?,
    )?;
    desktop::validate(
        &run.root,
        &serde_json::json!({"id":run.id,"output_layout":run.output_layout.declaration(&run.id),"checks":run.checks(),
        "commands":run.commands(),"source_snapshot":run.build_inputs()}),
    )?;
    run.check("desktop-complete-evidence-validation",serde_json::json!({"scope":"All ten current desktop workflows reopened: actual launches, serials, native captures, rebuilds and private persistent bytes","new_foundation_execution":false,"phase_closure":false}));
    Ok(())
}

/// One declared acceptance suite on the current desktop build (spec section 86:
/// `test ui --suite component-gallery`, `test compat --suite win32-gui...`).
/// An unregistered suite fails before anything is built.
/// With `--image`, the suite reuses that staged desktop build (its run's
/// `desktop-build.json` names the test and diagnostics images) instead of
/// rebuilding.
fn acceptance_command(
    run: &mut Run,
    family: &str,
    name: &str,
    prebuilt: Option<&str>,
) -> Result<(), String> {
    let suite = scenarios::acceptance_suite(family, name)?;
    let runner = scenarios::runner(suite)?;
    let (image, build) = match prebuilt {
        Some(image) => (
            image.to_owned(),
            session::desktop_build_of(&run.root, image)?,
        ),
        None => {
            assets::export(&run.root, true)?;
            let image = session::build_desktop(run)?;
            (
                image,
                image::read_json(&run.directory.join("desktop-build.json"))?,
            )
        }
    };
    let test_image = build["test_image"].as_str().ok_or("desktop test image")?;
    // a11y-01 and the gallery boot the same build's diagnostics image.
    build["diagnostics_image"]
        .as_str()
        .ok_or("desktop diagnostics image (rebuild: this build predates it)")?;
    let result = runner(run, &image, test_image)?;
    run.record_suite(suite.name, scenarios::reference_comparison(&result));
    run.check(
        suite.check,
        scenarios::acceptance_record(suite, &format!("{:?}", suite.phase), result),
    );
    println!(
        "{} acceptance suite {} passed on the current desktop build; this does not close the phase gate.",
        suite.id, suite.name
    );
    Ok(())
}

/// `cargo xtask test guest-batch --phase <P> --image <image.json> --only
/// a,b,...`: the named desktop gate steps, desktop scenarios and acceptance
/// suites of a staged desktop build on the guest pool (`DOOR_GUEST_JOBS`),
/// recorded exactly as the gate records them. A developer re-run of part of
/// the gate's guest work; it certifies nothing.
fn guest_batch(run: &mut Run, phase: &str, image: &str, names: &str) -> Result<(), String> {
    let ledger = image::read_json(&run.root.join("project/phases.json"))?;
    let phase = spec::verification_phase(&ledger, Some(phase))?;
    let build = session::desktop_build_of(&run.root, image)?;
    let test_image = build["test_image"].as_str().ok_or("desktop test image")?;
    build["diagnostics_image"]
        .as_str()
        .ok_or("desktop diagnostics image (rebuild: this build predates it)")?;
    let only: Vec<String> = names
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    if only.is_empty() {
        return Err("--only names no gate step, scenario or suite".into());
    }
    let concurrency = parallel::concurrency(&run.root)?;
    println!(
        "Guest workers: {} ({})",
        concurrency.jobs, concurrency.record
    );
    run.record_scheduling(concurrency.record);
    let started = std::time::Instant::now();
    let result = scenarios::run_guest_gate(
        run,
        &phase,
        image,
        test_image,
        concurrency.jobs,
        Some(&only),
    );
    run.record_timing(
        "total",
        "guest batch",
        started.elapsed(),
        if result.is_ok() { "passed" } else { "failed" },
    );
    println!("{}", run.timing_summary(30));
    result
}

/// Runs every cumulative acceptance suite of the selected phase in order.
fn acceptance(run: &mut Run, phase: &str, image: &str, test_image: &str) -> Result<(), String> {
    for suite in scenarios::acceptance_suites(phase)? {
        let runner = scenarios::runner(suite)?;
        let result = evidence::timed(run, "acceptance-suite", suite.name, |run| {
            runner(run, image, test_image)
        })?;
        run.record_suite(suite.name, scenarios::reference_comparison(&result));
        run.check(
            suite.check,
            scenarios::acceptance_record(suite, phase, result),
        );
    }
    scenarios::validate_acceptance(&serde_json::json!({"checks":run.checks()}), phase)
}

/// `cargo xtask verify --list`: the selected cumulative gate's composition, read
/// only from the ledger and the declared inventories. It qualifies nothing.
fn verify_plan(root: &std::path::Path, requested: Option<&str>) -> Result<(), String> {
    let ledger = image::read_json(&root.join("project/phases.json"))?;
    let phase = spec::verification_phase(&ledger, requested)?;
    let selected = phase::Phase::parse(&phase)?;
    println!("{phase} cumulative gate (dry listing; nothing is built or run):");
    println!(
        "  doctor and check-spec (P00-P11 historical reports owner-confirmed unavailable, ADR 0017)"
    );
    let commands = phase_commands(&phase);
    println!("  {} mandatory commands:", commands.len());
    for argv in &commands {
        println!("    {}", argv.join(" "));
    }
    if selected >= phase::Phase::P02 {
        println!(
            "  {} inherited diagnostic guest scenarios and the demo ({})",
            scenarios::cases(&phase)?.len(),
            scenarios::check_id(&phase)?
        );
    }
    if selected >= phase::Phase::P10 {
        println!("  ordinary text session");
    }
    if selected >= phase::Phase::P11 {
        println!(
            "  P11 desktop gate: desktop workflows, recovery, wide lifecycle, resolutions, native default"
        );
    }
    let desktop = scenarios::desktop_scenarios(&phase)?;
    if !desktop.is_empty() {
        let names: Vec<_> = desktop.iter().map(|s| s.name).collect();
        println!(
            "  {} desktop scenarios: {}",
            desktop.len(),
            names.join(", ")
        );
    }
    let suites = scenarios::acceptance_suites(&phase)?;
    if !suites.is_empty() {
        println!("  {} acceptance suites:", suites.len());
        for suite in suites {
            let state = if scenarios::runner(suite).is_ok() {
                "runner registered"
            } else {
                "MISSING: verify fails before any build"
            };
            println!(
                "    {} ({} --suite {}): {state}",
                suite.id, suite.family, suite.name
            );
        }
    }
    Ok(())
}

/// The cumulative gate, with the elapsed time of every stage, image build,
/// guest case, scenario and suite recorded in the report (`timings`) and the
/// longest printed at the end, whether the gate passed or failed.
fn verify(run: &mut Run, requested: Option<&str>) -> Result<(), String> {
    // A gate builds every artifact from scratch.
    if evidence::dev_cache_enabled() {
        return Err("DOOR_XTASK_DEV_CACHE=1 is for development builds; unset it to verify".into());
    }
    let started = std::time::Instant::now();
    let result = verify_stages(run, requested);
    run.record_timing(
        "total",
        "verify",
        started.elapsed(),
        if result.is_ok() { "passed" } else { "failed" },
    );
    println!("{}", run.timing_summary(30));
    result
}

fn verify_stages(run: &mut Run, requested: Option<&str>) -> Result<(), String> {
    use evidence::timed;
    timed(run, "stage", "doctor", doctor::discover)?;
    timed(run, "stage", "check-spec", |run| spec::check(&run.root))?;
    let ledger = image::read_json(&run.root.join("project/phases.json"))?;
    // The ledger's selected phase, or an explicitly requested owner-authorized
    // parallel phase (P13 or P14 alongside P12). Each gate includes all of
    // the previous phase's.
    let phase = spec::verification_phase(&ledger, requested)?;
    let phase = phase.as_str();
    let selected = phase::Phase::parse(phase)?;
    if selected >= phase::Phase::P12 {
        // Fail before any build or guest run when a named suite has no runner.
        scenarios::acceptance_preflight(phase)?;
    }
    // Guest worker count (DOOR_GUEST_JOBS or the resource default), fixed
    // before anything runs so an invalid override fails first.
    let concurrency = if selected >= phase::Phase::P11 {
        let concurrency = parallel::concurrency(&run.root)?;
        println!(
            "Guest workers: {} ({})",
            concurrency.jobs, concurrency.record
        );
        run.record_scheduling(concurrency.record.clone());
        Some(concurrency.jobs)
    } else {
        None
    };
    run.certified_phase = Some(phase.to_owned());
    timed(run, "stage", "mandatory host commands", |run| {
        for argv in phase_commands(phase) {
            run.required(argv[0], &argv[1..])?;
        }
        Ok(())
    })?;
    if phase == "P01" {
        build::write_plan(run)?;
        build::host_build(run, true)?;
        spec::verify_current_rebuild(run)?;
        let hardware =
            image::read_json(&run.root.join("profiles/hardware/qemu-diagnostic-x64.json"))?;
        if hardware["runnable"] == true {
            let (profile, _, _, _) =
                emulator::preflight(run, "profiles/hardware/qemu-diagnostic-x64.json")?;
            run.check("p01-emulator-prerequisites", profile);
        }
    }
    if selected >= phase::Phase::P02 {
        timed(run, "stage", "inherited guest scenarios and demos", |run| {
            boot::verify(run, phase)
        })?;
        timed(run, "stage", "guest gate validation", |run| {
            boot_gate::validate_current(run)
        })?;
    }
    if selected >= phase::Phase::P10 {
        let image = timed(run, "build", "text session image", session::build)?;
        timed(run, "stage", "ordinary text session", |run| {
            session::verify::run(run, &image)
        })?;
    }
    if selected >= phase::Phase::P11 {
        timed(run, "stage", "asset check", |run| {
            assets::export(&run.root, true)
        })?;
        let image = timed(run, "build", "desktop images", session::build_desktop)?;
        let build = image::read_json(&run.directory.join("desktop-build.json"))?;
        let test_image = build["test_image"]
            .as_str()
            .ok_or("desktop test image")?
            .to_owned();
        let jobs = concurrency.unwrap_or(1);
        if jobs == 1 {
            // The sequential gate, unchanged.
            timed(run, "stage", "P11 desktop gate", |run| {
                desktop::verify_all(run, &image, &test_image)
            })?;
            desktop::validate(
                &run.root,
                &serde_json::json!({"id":run.id,"checks":run.checks(),"commands":run.commands(),"source_snapshot":run.build_inputs()}),
            )?;
            if selected >= phase::Phase::P12 {
                // Every other existing desktop scenario, each in its own child run.
                timed(run, "stage", "desktop scenarios", |run| {
                    scenarios::run_desktop_scenarios(run, phase, &image)
                })?;
                timed(run, "stage", "acceptance suites", |run| {
                    acceptance(run, phase, &image, &test_image)
                })?;
            }
        } else {
            // The same gate steps, scenarios and suites on `jobs` workers.
            timed(
                run,
                "stage",
                &format!("guest pool ({jobs} workers)"),
                |run| scenarios::run_guest_gate(run, phase, &image, &test_image, jobs, None),
            )?;
        }
    }
    if selected >= phase::Phase::P12 {
        println!(
            "{phase} cumulative gate passed: inherited commands, guest scenarios, desktop workflows, {} desktop scenarios and {} acceptance suites on current source.",
            scenarios::desktop_scenarios(phase)?.len(),
            scenarios::acceptance_suites(phase)?.len()
        );
    } else if selected >= phase::Phase::P02 {
        println!(
            "{phase} actual guest gate passed on acquired QEMU/OVMF. Later userland and desktop gates remain separate."
        );
    } else {
        println!(
            "{phase} host checks passed. Guest boot, Windows compatibility and desktop execution remain not_run."
        );
    }
    Ok(())
}

pub(crate) fn phase_commands(phase: &str) -> Vec<Vec<&'static str>> {
    let mut commands = gate_commands();
    let Ok(selected) = phase::Phase::parse(phase) else {
        return commands;
    };
    if selected >= phase::Phase::P01 {
        for target in ["x86_64-unknown-none", "x86_64-unknown-uefi"] {
            commands.push(vec![
                "cargo",
                "check",
                "--locked",
                "--package",
                "door-image-format",
                "--target",
                target,
            ]);
        }
    }
    if selected >= phase::Phase::P02 {
        for target in ["x86_64-unknown-none", "x86_64-unknown-uefi"] {
            commands.push(vec![
                "cargo",
                "check",
                "--locked",
                "--package",
                "door-initfs",
                "--target",
                target,
            ]);
        }
        commands.push(vec![
            "cargo",
            "clippy",
            "--locked",
            "--package",
            "door-uefi",
            "--features",
            "firmware",
            "--target",
            "x86_64-unknown-uefi",
            "--",
            "-D",
            "warnings",
        ]);
        commands.push(vec![
            "cargo",
            "clippy",
            "--locked",
            "--package",
            "door-kernel",
            "--features",
            "baremetal",
            "--target",
            "x86_64-unknown-none",
            "--",
            "-D",
            "warnings",
        ]);
    }
    if selected >= phase::Phase::P05 {
        for package in ["door-abi", "door-sys", "door-rt"] {
            for target in ["x86_64-unknown-none", "x86_64-unknown-uefi"] {
                commands.push(vec![
                    "cargo",
                    "check",
                    "--locked",
                    "--package",
                    package,
                    "--target",
                    target,
                ]);
            }
        }
        commands.push(vec![
            "cargo",
            "clippy",
            "--locked",
            "--package",
            "door-native-fixtures",
            "--features",
            "native",
            "--target",
            "x86_64-unknown-none",
            "--",
            "-D",
            "warnings",
        ]);
    }
    if selected >= phase::Phase::P08 {
        for package in [
            "door-storage-core",
            "door-service-protocol",
            "door-service-client",
        ] {
            for target in ["x86_64-unknown-none", "x86_64-unknown-uefi"] {
                commands.push(vec![
                    "cargo",
                    "check",
                    "--locked",
                    "--package",
                    package,
                    "--target",
                    target,
                ]);
            }
        }
        for package in ["door-system-services", "door-native-workload"] {
            commands.push(vec![
                "cargo",
                "clippy",
                "--locked",
                "--package",
                package,
                "--features",
                "native",
                "--target",
                "x86_64-unknown-none",
                "--",
                "-D",
                "warnings",
            ]);
        }
    }
    if selected >= phase::Phase::P09 {
        for package in ["door-windows-abi", "door-pe-loader"] {
            for target in ["x86_64-unknown-none", "x86_64-unknown-uefi"] {
                commands.push(vec![
                    "cargo",
                    "check",
                    "--locked",
                    "--package",
                    package,
                    "--target",
                    target,
                ]);
            }
        }
        for package in ["door-ntdll", "door-kernel32"] {
            commands.push(vec![
                "cargo",
                "clippy",
                "--locked",
                "--package",
                package,
                "--features",
                "guest-pe",
                "--target",
                "x86_64-pc-windows-msvc",
                "--",
                "-D",
                "warnings",
            ]);
        }
    }
    if selected >= phase::Phase::P10 {
        for package in ["door-shell", "door-tools"] {
            commands.push(vec![
                "cargo",
                "clippy",
                "--locked",
                "--package",
                package,
                "--features",
                "native",
                "--target",
                "x86_64-unknown-none",
                "--",
                "-D",
                "warnings",
            ]);
        }

        commands.push(vec![
            "cargo",
            "clippy",
            "--locked",
            "--package",
            "door-msvcrt",
            "--features",
            "guest-pe",
            "--target",
            "x86_64-pc-windows-msvc",
            "--",
            "-D",
            "warnings",
        ]);
    }
    if selected >= phase::Phase::P11 {
        for package in ["door-window", "door-workspace"] {
            commands.push(vec![
                "cargo",
                "clippy",
                "--locked",
                "--package",
                package,
                "--features",
                "native",
                "--target",
                "x86_64-unknown-none",
                "--",
                "-D",
                "warnings",
            ]);
        }
        for package in ["door-graphics-core", "door-graphics-text"] {
            commands.push(vec![
                "cargo",
                "check",
                "--locked",
                "--package",
                package,
                "--target",
                "x86_64-unknown-none",
            ]);
        }
    }
    if selected >= phase::Phase::P12 {
        // The shared native UI facade and the P12 applications in the desktop image.
        // Toolkit, accessibility, keyboard-layout and gallery packages join here
        // when their workstreams create them (docs/design/p12-p13-plan-20261023.md).
        commands.push(vec![
            "cargo",
            "clippy",
            "--locked",
            "--package",
            "door-app-ui",
            "--target",
            "x86_64-unknown-none",
            "--",
            "-D",
            "warnings",
        ]);
        for package in ["door-preview", "door-monitor"] {
            commands.push(vec![
                "cargo",
                "clippy",
                "--locked",
                "--package",
                package,
                "--features",
                "native",
                "--target",
                "x86_64-unknown-none",
                "--",
                "-D",
                "warnings",
            ]);
        }
        // Door UI toolkit, accessibility, gallery, keyboard layouts, IME and
        // locale crates on the guest target, with their widest guest features.
        for (package, features) in [
            ("door-ui", ""),
            ("door-ui-native", "accessibility"),
            ("door-accessibility", ""),
            ("door-keyboard-layouts", ""),
            ("door-locale", ""),
            ("door-gallery", "native,accessibility"),
            ("door-ime", "native"),
        ] {
            let mut argv = vec!["cargo", "clippy", "--locked", "--package", package];
            if !features.is_empty() {
                argv.extend(["--features", features]);
            }
            argv.extend(["--target", "x86_64-unknown-none", "--", "-D", "warnings"]);
            commands.push(argv);
        }
        // Named host suites (also inside the workspace run) so each P12 area has
        // its own mandatory result: toolkit, accessibility, gallery, Unicode text,
        // layouts, IME (with its conversion corpus) and locale.
        for package in [
            "door-ui",
            "door-ui-native",
            "door-accessibility",
            "door-gallery",
            "door-graphics-text",
            "door-keyboard-layouts",
            "door-ime",
            "door-locale",
        ] {
            commands.push(vec![
                "cargo",
                "test",
                "--locked",
                "--package",
                package,
                "--all-targets",
            ]);
        }
        commands.push(vec![
            "cargo",
            "test",
            "--locked",
            "--package",
            "door-ime",
            "--test",
            "corpus",
        ]);
        // Window-server text input, clipboard and drag-and-drop brokers.
        for filter in ["text_input", "clipboard", "dnd"] {
            commands.push(vec![
                "cargo",
                "test",
                "--locked",
                "--package",
                "door-window",
                "--lib",
                filter,
            ]);
        }
    }
    if selected >= phase::Phase::P13 {
        // P13 inherits every P12 command, then: the portable Win32 GUI core
        // (P13-B), the application-loader integration test (P13-A) and the
        // original user32/gdi32/comdlg32/comctl32 DLLs on the Windows guest
        // target (P13-B). They fail until those workstreams land; the five
        // Windows GUI acceptance suites keep the gate closed meanwhile.
        commands.push(vec![
            "cargo",
            "test",
            "--locked",
            "--package",
            "door-windows-gui",
        ]);
        commands.push(vec![
            "cargo",
            "test",
            "--locked",
            "--package",
            "door-pe-loader",
            "--test",
            "p13_application",
        ]);
        for package in [
            "door-user32",
            "door-gdi32",
            "door-comdlg32",
            "door-comctl32",
        ] {
            commands.push(vec![
                "cargo",
                "clippy",
                "--locked",
                "--package",
                package,
                "--features",
                "guest-pe",
                "--target",
                "x86_64-pc-windows-msvc",
                "--",
                "-D",
                "warnings",
            ]);
        }
    }
    if selected >= phase::Phase::P14 {
        // P14 inherits every P13 command, then: the restartable shell (WS-A),
        // the allocation-free desktop crates (tokens, file operations, typed
        // settings, settings routes) on the guest target, their host suites,
        // and named host suites for the sign-in chain (kernel entropy,
        // credential store, trusted sign-in surface).
        commands.push(vec![
            "cargo",
            "clippy",
            "--locked",
            "--package",
            "door-desktop-shell",
            "--features",
            "native",
            "--target",
            "x86_64-unknown-none",
            "--",
            "-D",
            "warnings",
        ]);
        for package in [
            "door-design-tokens",
            "door-file-ops",
            "door-settings-registry",
            "door-settings-map",
        ] {
            commands.push(vec![
                "cargo",
                "clippy",
                "--locked",
                "--package",
                package,
                "--target",
                "x86_64-unknown-none",
                "--",
                "-D",
                "warnings",
            ]);
            commands.push(vec![
                "cargo",
                "test",
                "--locked",
                "--package",
                package,
                "--all-targets",
            ]);
        }
        commands.push(vec![
            "cargo",
            "test",
            "--locked",
            "--package",
            "door-desktop-shell",
            "--all-targets",
        ]);
        // The inbox apps' image (Calculator) on the guest target and its
        // host suite (engine, layout and hit testing).
        commands.push(vec![
            "cargo",
            "clippy",
            "--locked",
            "--package",
            "door-inbox",
            "--features",
            "native",
            "--target",
            "x86_64-unknown-none",
            "--",
            "-D",
            "warnings",
        ]);
        commands.push(vec![
            "cargo",
            "test",
            "--locked",
            "--package",
            "door-inbox",
            "--all-targets",
        ]);
        for (package, filter) in [
            ("door-kernel", "entropy"),
            ("door-system-services", "credentials"),
            ("door-window", "login"),
        ] {
            commands.push(vec![
                "cargo",
                "test",
                "--locked",
                "--package",
                package,
                "--lib",
                filter,
            ]);
        }
    }
    if selected >= phase::Phase::P15 {
        // P15 inherits every P14 command, then: the network and sound
        // services on the guest target, the Windows network and multimedia
        // DLLs on the PE target, and the host suites of the protocol and
        // transport crates and of each DLL's host-testable core.
        for package in ["door-network", "door-audio"] {
            commands.push(vec![
                "cargo",
                "clippy",
                "--locked",
                "--package",
                package,
                "--bins",
                "--features",
                "native",
                "--target",
                "x86_64-unknown-none",
                "--",
                "-D",
                "warnings",
            ]);
        }
        for package in [
            "door-ws2_32",
            "door-iphlpapi",
            "door-dnsapi",
            "door-winhttp",
            "door-wininet",
            "door-winmm",
            "door-dsound",
            "door-ole32",
            "door-shell32",
            "door-mmdevapi",
        ] {
            commands.push(vec![
                "cargo",
                "clippy",
                "--locked",
                "--package",
                package,
                "--features",
                "guest-pe",
                "--target",
                "x86_64-pc-windows-msvc",
                "--",
                "-D",
                "warnings",
            ]);
        }
        for package in [
            "door-network",
            "door-audio",
            "door-tls",
            "door-http",
            "door-windows-inet",
            "door-ws2_32",
            "door-iphlpapi",
            "door-dnsapi",
            "door-winhttp",
            "door-wininet",
            "door-winmm",
            "door-dsound",
            "door-ole32",
            "door-shell32",
            "door-mmdevapi",
            "door-firewall",
            "door-wireguard",
            "door-pac",
        ] {
            commands.push(vec!["cargo", "test", "--locked", "--package", package]);
        }
    }
    if selected >= phase::Phase::P16 {
        // P16 inherits every P15 command, then: Windows Installer's msi.dll
        // and advapi32 on the PE target (msiexec.exe is linked by the image
        // build) and the host suites of the cabinet, package and engine
        // crates, cmd and Doorsh; oleaut32 and the COM wire formats (M6).
        for package in [
            "door-msi-dll",
            "door-advapi32",
            "door-oleaut32",
            "door-taskschd",
            "door-wintrust",
        ] {
            commands.push(vec![
                "cargo",
                "clippy",
                "--locked",
                "--package",
                package,
                "--features",
                "guest-pe",
                "--target",
                "x86_64-pc-windows-msvc",
                "--",
                "-D",
                "warnings",
            ]);
        }
        for package in [
            "door-cab",
            "door-msi",
            "door-msi-dll",
            "door-shells",
            "door-advapi32",
            "door-oleaut32",
            "door-com-wire",
            "door-taskschd",
            "door-wintrust",
            "door-authenticode",
        ] {
            commands.push(vec!["cargo", "test", "--locked", "--package", package]);
        }
    }
    if selected >= phase::Phase::P17 {
        // P17 inherits every P16 command, then the 32-bit (WOW64) builds of
        // every compatibility DLL linted on the i686 PE target, and the host
        // suites of the structure layouts and WOW64 views
        // (door-windows-abi) and of the PE32 loader.
        for package in [
            "door-ntdll",
            "door-kernel32",
            "door-msvcrt",
            "door-gdi32",
            "door-user32",
            "door-comctl32",
            "door-comdlg32",
            "door-ws2_32",
            "door-iphlpapi",
            "door-dnsapi",
            "door-advapi32",
            "door-winhttp",
            "door-wininet",
            "door-winmm",
            "door-dsound",
            "door-ole32",
            "door-shell32",
            "door-msi-dll",
            "door-oleaut32",
            "door-mmdevapi",
            "door-taskschd",
            "door-wintrust",
        ] {
            commands.push(vec![
                "cargo",
                "clippy",
                "--locked",
                "--package",
                package,
                "--features",
                "guest-pe",
                "--target",
                "i686-pc-windows-msvc",
                "--",
                "-D",
                "warnings",
            ]);
        }
        for package in ["door-windows-abi", "door-pe-loader"] {
            commands.push(vec!["cargo", "test", "--locked", "--package", package]);
        }
    }
    commands
}

pub(crate) fn gate_commands() -> Vec<Vec<&'static str>> {
    vec![
        vec!["cargo", "fmt", "--all", "--", "--check"],
        vec![
            "cargo",
            "clippy",
            "--workspace",
            "--locked",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
        vec!["cargo", "test", "--workspace", "--locked", "--all-targets"],
        vec![
            "cargo",
            "check",
            "--locked",
            "--package",
            "door-boot-protocol",
            "--target",
            "x86_64-unknown-none",
        ],
        vec![
            "cargo",
            "check",
            "--locked",
            "--package",
            "door-boot-protocol",
            "--target",
            "x86_64-unknown-uefi",
        ],
    ]
}

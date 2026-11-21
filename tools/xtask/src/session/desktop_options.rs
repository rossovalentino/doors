//! Interactive launch policy, separate from the pinned software regressions.
use super::*;
pub(crate) const NATIVE_PIXEL_ENV: &[(&str, &str)] = &[
    ("SDL_WINDOWS_DPI_AWARENESS", "permonitorv2"),
    ("SDL_WINDOWS_DPI_SCALING", "0"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Accelerator {
    Whpx,
    Tcg,
}
impl Accelerator {
    fn name(self) -> &'static str {
        match self {
            Self::Whpx => "whpx",
            Self::Tcg => "tcg",
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub struct DesktopOptions {
    visible: bool,
    resolution: String,
    accelerator: Accelerator,
    /// Host display scale hint (percent) for `--resolution auto`, resolved
    /// read-only at launch and passed as fw_cfg `opt/door/scale`.
    scale_hint: Option<u32>,
    /// P18 `--apps`: the 2-CPU WHPX desktop profile.
    two_cpus: bool,
}
impl DesktopOptions {
    /// Options after the desktop command. Reject duplicates and missing values
    /// before reading the image, opening storage or spawning a guest.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut options = Self {
            visible: true,
            resolution: "auto".into(),
            accelerator: Accelerator::Whpx,
            scale_hint: None,
            two_cpus: false,
        };
        let (mut headless, mut accelerator, mut resolution) = (false, false, false);
        let mut at = 0;
        while at < args.len() {
            match args[at].as_str() {
                "--headless" if !headless => {
                    headless = true;
                    options.visible = false;
                }
                "--accelerator" if !accelerator => {
                    accelerator = true;
                    at += 1;
                    options.accelerator = match args.get(at).map(String::as_str) {
                        Some("whpx") => Accelerator::Whpx,
                        Some("tcg") => Accelerator::Tcg,
                        _ => return Err("Desktop accelerator must be whpx or tcg".into()),
                    };
                }
                "--resolution" if !resolution => {
                    resolution = true;
                    at += 1;
                    let mode = args.get(at).ok_or("Desktop resolution value missing")?;
                    if mode != "auto" && !supported_resolution(mode) {
                        return Err("Unsupported desktop resolution".into());
                    }
                    options.resolution.clone_from(mode);
                }
                _ => return Err(format!("Unknown or duplicate desktop option: {}", args[at])),
            }
            at += 1;
        }
        Ok(options)
    }
    /// Run on the 2-CPU WHPX desktop profile (P18 `desktop --apps`).
    pub fn with_two_cpus(mut self) -> Self {
        self.two_cpus = true;
        self
    }
    pub(super) fn scale_hint(&self) -> Option<u32> {
        self.scale_hint
    }
    pub(super) fn hardware_profile(&self) -> &'static str {
        match self.accelerator {
            Accelerator::Whpx if self.two_cpus => {
                "profiles/hardware/qemu-desktop-whpx-smp2-x64.json"
            }
            Accelerator::Whpx => "profiles/hardware/qemu-desktop-whpx-x64.json",
            Accelerator::Tcg => "profiles/hardware/qemu-services-x64.json",
        }
    }
    pub(super) fn validate_argv(&self, args: &[String]) -> Result<(), String> {
        let (mut accelerators, mut displays, mut video, mut modes) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut seen = std::collections::BTreeSet::new();
        let mut at = 0;
        while at < args.len() {
            let key = args[at].as_str();
            if key == "-nodefaults" {
                if !seen.insert(key) {
                    return Err("Duplicate desktop QEMU flag".into());
                }
                at += 1;
                continue;
            }
            if !matches!(
                key,
                "-machine"
                    | "-accel"
                    | "-cpu"
                    | "-smp"
                    | "-m"
                    | "-device"
                    | "-display"
                    | "-monitor"
                    | "-nic"
                    | "-netdev"
                    | "-audiodev"
                    | "-serial"
                    | "-drive"
                    | "-fw_cfg"
                    | "-name"
                    | "-uuid"
                    | "-qmp"
            ) {
                return Err("Unknown desktop QEMU flag".into());
            }
            if !matches!(key, "-device" | "-drive" | "-fw_cfg") && !seen.insert(key) {
                return Err("Duplicate desktop QEMU option".into());
            }
            let value = args
                .get(at + 1)
                .ok_or("Missing desktop QEMU option value")?;
            if value.is_empty() || value.starts_with('-') {
                return Err("Invalid desktop QEMU option value".into());
            }
            match key {
                "-accel" => accelerators.push(value.as_str()),
                "-display" => displays.push(value.as_str()),
                "-device"
                    if !value.starts_with("virtio-blk-pci,")
                        && !value.starts_with("virtio-net-pci,")
                        && !value.starts_with("virtio-sound-pci,") =>
                {
                    video.push(value.as_str())
                }
                "-fw_cfg" => modes.push(value.as_str()),
                _ => (),
            }
            at += 2;
        }
        let expected_mode = format!("name=opt/door/display,string={}", self.resolution);
        let expected_scale = self
            .scale_hint
            .map(|p| format!("name=opt/door/scale,string={p}"));
        let expected_modes: Vec<&str> = core::iter::once(expected_mode.as_str())
            .chain(expected_scale.as_deref())
            .collect();
        let expected_video = match self.accelerator {
            Accelerator::Whpx => "ramfb".into(),
            Accelerator::Tcg => {
                let (width, height) = self.resolution.split_once('x').ok_or("Desktop mode")?;
                format!("VGA,vgamem_mb=64,xres={width},yres={height}")
            }
        };
        if accelerators != [self.accelerator.name()]
            || displays != [if self.visible { "sdl" } else { "none" }]
            || video != [expected_video.as_str()]
            || modes != expected_modes
        {
            return Err(
                "Constructed desktop arguments differ from requested backend/display/mode".into(),
            );
        }
        Ok(())
    }
    pub fn launch(&self, run: &mut Run, image: &str, disk: &Path) -> Result<Value, String> {
        let resolved = Self {
            visible: self.visible,
            accelerator: self.accelerator,
            resolution: if self.resolution == "auto" {
                native_display::resolve(run)?
            } else {
                self.resolution.clone()
            },
            scale_hint: if self.resolution == "auto" {
                native_display::resolve_scale(run)?
            } else {
                None
            },
            two_cpus: self.two_cpus,
        };
        println!(
            "Desktop backend: {} / {}; output {}",
            self.accelerator.name(),
            if self.accelerator == Accelerator::Whpx {
                "RAMFB"
            } else {
                "VGA"
            },
            resolved.resolution
        );
        let result = launch_profile_hardware(
            run,
            image,
            disk,
            self.visible,
            true,
            Some(&resolved.resolution),
            LaunchProfile::Interactive(&resolved),
        );
        let record = result.map_err(|error| {
            if self.accelerator == Accelerator::Whpx {
                format!("{error}; selected WHPX/RAMFB requires Windows Hypervisor Platform. Explicit software mode: cargo xtask desktop --accelerator tcg")
            } else {
                error
            }
        })?;
        run.check(
            "desktop-launch-selection",
            json!({
                "requested_accelerator": self.accelerator.name(),
                "requested_resolution": self.resolution,
                "resolution": resolved.resolution,
                "scale_hint": resolved.scale_hint,
                "visible": self.visible,
                "actual_argv": record["argv"],
                "viewer_environment": record["viewer_environment"],
                "automatic_fallback": false,
                "actual_arguments_validated_before_spawn": true,
                "scope": "Shared interactive CLI policy; no host feature change or FPS claim"
            }),
        );
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<DesktopOptions, String> {
        DesktopOptions::parse(&args.iter().map(|s| (*s).into()).collect::<Vec<_>>())
    }
    #[test]
    fn default_is_windowed_native_hardware_profile_and_software_is_explicit() {
        let default = parse(&[]).unwrap();
        assert!(default.visible);
        assert_eq!(default.resolution, "auto");
        assert_eq!(default.accelerator, Accelerator::Whpx);
        assert_eq!(
            parse(&["--accelerator", "tcg"]).unwrap().accelerator,
            Accelerator::Tcg
        );
        assert_eq!(parse(&["--accelerator", "whpx"]).unwrap(), default);
        assert_eq!(parse(&["--resolution", "auto"]).unwrap(), default);
    }
    #[test]
    fn option_order_preserves_headless_accelerator_and_all_supported_modes() {
        for mode in [
            "1024x768",
            "1280x720",
            "1920x1080",
            "1920x1200",
            "2560x1440",
            "2560x1600",
            "3840x2160",
        ] {
            for accelerator in ["whpx", "tcg"] {
                let first = parse(&[
                    "--headless",
                    "--accelerator",
                    accelerator,
                    "--resolution",
                    mode,
                ])
                .unwrap();
                for args in [
                    vec![
                        "--resolution",
                        mode,
                        "--headless",
                        "--accelerator",
                        accelerator,
                    ],
                    vec![
                        "--accelerator",
                        accelerator,
                        "--resolution",
                        mode,
                        "--headless",
                    ],
                    vec![
                        "--resolution",
                        mode,
                        "--accelerator",
                        accelerator,
                        "--headless",
                    ],
                    vec![
                        "--accelerator",
                        accelerator,
                        "--headless",
                        "--resolution",
                        mode,
                    ],
                    vec![
                        "--headless",
                        "--resolution",
                        mode,
                        "--accelerator",
                        accelerator,
                    ],
                ] {
                    assert_eq!(parse(&args).unwrap(), first);
                }
                assert!(!first.visible);
                assert_eq!(first.resolution, mode);
                assert_eq!(first.accelerator.name(), accelerator);
            }
        }
    }
    #[test]
    fn malformed_or_duplicate_options_never_reach_a_launch() {
        for args in [
            vec!["--accelerator"],
            vec!["--resolution"],
            vec!["--accelerator", "auto"],
            vec!["--accelerator", "whpx,tcg"],
            vec!["--resolution", "3840x2160,extra"],
            vec!["--resolution", "0x0"],
            vec!["--headless", "--headless"],
            vec!["--resolution", "1280x720", "--resolution", "3840x2160"],
            vec!["--accelerator", "whpx", "--accelerator", "tcg"],
            vec!["--accelerator", "--headless"],
            vec!["--unknown"],
            vec!["extra"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
    }
    #[test]
    fn selected_backend_display_and_mode_are_bound_to_actual_arguments() {
        let hardware = parse(&["--headless", "--resolution", "3840x2160"]).unwrap();
        let args = [
            "-accel",
            "whpx",
            "-device",
            "ramfb",
            "-display",
            "none",
            "-fw_cfg",
            "name=opt/door/display,string=3840x2160",
        ]
        .map(String::from);
        assert!(hardware.validate_argv(&args).is_ok());
        // A host scale hint is bound exactly like the display mode.
        let mut hinted = parse(&["--headless", "--resolution", "3840x2160"]).unwrap();
        hinted.scale_hint = Some(150);
        let mut with_scale = args.to_vec();
        with_scale.extend(["-fw_cfg", "name=opt/door/scale,string=150"].map(String::from));
        assert!(hinted.validate_argv(&with_scale).is_ok());
        assert!(hardware.validate_argv(&with_scale).is_err());
        assert!(hinted.validate_argv(&args).is_err());
        let software = parse(&["--accelerator", "tcg", "--resolution", "1280x720"]).unwrap();
        let soft_args = [
            "-accel",
            "tcg",
            "-device",
            "VGA,vgamem_mb=64,xres=1280,yres=720",
            "-display",
            "sdl",
            "-fw_cfg",
            "name=opt/door/display,string=1280x720",
        ]
        .map(String::from);
        assert!(software.validate_argv(&soft_args).is_ok());
        assert!(hardware.validate_argv(&soft_args).is_err());
        assert!(software.validate_argv(&args).is_err());
    }
    #[test]
    fn wrong_duplicate_mixed_or_missing_runtime_options_are_rejected() {
        let options = parse(&["--resolution", "1280x720"]).unwrap();
        let good = [
            "-accel",
            "whpx",
            "-device",
            "ramfb",
            "-display",
            "sdl",
            "-fw_cfg",
            "name=opt/door/display,string=1280x720",
        ]
        .map(String::from)
        .to_vec();
        for (at, wrong) in [
            (1, "tcg"),
            (1, "whpx,ssd=off"),
            (3, "VGA"),
            (5, "none"),
            (5, "sdl,full-screen=on"),
            (7, "name=opt/door/display,string=3840x2160"),
        ] {
            let mut args = good.clone();
            args[at] = wrong.into();
            assert!(options.validate_argv(&args).is_err());
        }
        for extra in [
            ["-accel", "whpx"],
            ["-device", "VGA"],
            ["-device", "ramfb"],
            ["-display", "sdl"],
            ["-fw_cfg", "name=opt/door/display,string=1280x720"],
            ["-vga", "none"],
        ] {
            let mut args = good.clone();
            args.extend(extra.map(String::from));
            assert!(options.validate_argv(&args).is_err());
        }
        for length in 0..good.len() {
            assert!(options.validate_argv(&good[..length]).is_err());
        }
        let mut trailing = good.clone();
        trailing.push("-machine".into());
        assert!(options.validate_argv(&trailing).is_err());
    }
}

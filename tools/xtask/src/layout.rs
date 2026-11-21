//! Versioned placement for new evidence. Never infer a layout from file existence.
use crate::paths;
use serde_json::{Value, json};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum Layout {
    #[default]
    Legacy,
    RetainedDesktop,
}

pub fn canonical_owner(name: &str) -> bool {
    let Some((stamp, pid)) = name.strip_prefix("host-").and_then(|s| s.split_once('-')) else {
        return false;
    };
    stamp
        .parse::<u128>()
        .is_ok_and(|n| n != 0 && n.to_string() == stamp)
        && pid
            .parse::<u32>()
            .is_ok_and(|n| n != 0 && n.to_string() == pid)
}

impl Layout {
    /// Audited producer graphs only. Standalone app consumers create their raw
    /// disks/captures under the run and use the same retained QEMU launcher.
    pub fn for_command(arguments: &[impl AsRef<str>]) -> Self {
        let args: Vec<_> = arguments.iter().map(AsRef::as_ref).collect();
        match args.as_slice() {
            ["build", "--profile", "desktop-qemu-x64"]
            | [
                "test",
                "desktop-current"
                | "desktop-motion-current"
                | "settings-reference-current"
                | "settings-reference-development"
                | "terminal-current"
                | "explorer-current"
                | "explorer-development",
            ]
            | [
                "test",
                "desktop-notes"
                | "desktop-notes-accelerated"
                | "desktop-files"
                | "desktop-files-accelerated"
                | "desktop-remove"
                | "desktop-remove-accelerated"
                | "desktop-default"
                | "desktop-motion"
                | "desktop-software"
                | "explorer"
                | "settings-reference"
                | "preview"
                | "monitor"
                | "multiwindow"
                | "multiwindow-pressure"
                | "desktop-recovery",
                "--image",
                _,
            ] => Self::RetainedDesktop,
            ["desktop", options @ ..]
                if crate::session::DesktopOptions::parse(
                    &options.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
                )
                .is_ok() =>
            {
                Self::RetainedDesktop
            }
            _ => Self::Legacy,
        }
    }
    pub fn declaration(self, owner: &str) -> Value {
        match self {
            Self::Legacy => json!({"version":1,"kind":"legacy-out","owner":owner}),
            Self::RetainedDesktop => json!({"version":2,"kind":"retained-desktop","owner":owner}),
        }
    }

    pub fn from_report(report: &Value) -> Result<Self, String> {
        let declaration = &report["output_layout"];
        // Historical v1 reports predate the declaration. No existence fallback.
        if declaration.is_null() {
            return Ok(Self::Legacy);
        }
        let owner = report["id"].as_str().ok_or("layout report identity")?;
        if !canonical_owner(owner) {
            return Err("noncanonical layout owner".into());
        }
        for candidate in [Self::Legacy, Self::RetainedDesktop] {
            if *declaration == candidate.declaration(owner) {
                return Ok(candidate);
            }
        }
        Err("unknown or inconsistent output layout declaration".into())
    }

    pub fn path(self, owner: &str, legacy: &str) -> Result<String, String> {
        paths::relative_path(legacy)?;
        if self == Self::Legacy {
            return Ok(legacy.to_owned());
        }
        if !canonical_owner(owner) {
            return Err("noncanonical generated-output owner".into());
        }
        let parts: Vec<_> = legacy.split('/').collect();
        if parts.len() < 3
            || parts[0] != "out"
            || !matches!(parts[1], "builds" | "images" | "qemu")
            || !(parts[2] == owner
                || parts[2]
                    .strip_prefix(owner)
                    .is_some_and(|s| s.starts_with('-')))
        {
            return Err("retained desktop output must belong to its declared run".into());
        }
        if parts[1] == "builds" && parts[2] != owner {
            return Err("build tree requires the exact declared owner".into());
        }
        let variant = parts[2].strip_prefix(owner).ok_or("output owner")?;
        let variant = variant.strip_prefix('-').unwrap_or(variant);
        if (parts[2] != owner && variant.is_empty())
            || !variant
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || (parts[1] == "images"
                && !matches!(
                    variant,
                    "" | "desktop" | "desktop-test" | "desktop-diagnostics"
                ))
            || (parts[1] == "qemu"
                && !variant.strip_prefix("session-").is_some_and(|s| {
                    s.parse::<u128>()
                        .is_ok_and(|n| n != 0 && n.to_string() == s)
                }))
        {
            return Err("invalid generated instance variant".into());
        }
        // Keep owner once in the root: repetition exceeds MSVC linker limits.
        let mut result = format!("reports/runs/{owner}/generated/{}", parts[1]);
        if parts[1] != "builds" {
            result.push('/');
            result.push_str(if variant.is_empty() { "core" } else { variant });
        }
        for part in &parts[3..] {
            result.push('/');
            result.push_str(part);
        }
        Ok(result)
    }
}

pub fn retained_directory(name: &str) -> bool {
    let parts: Vec<_> = name.split('/').collect();
    parts.len() >= 5
        && parts[..2] == ["reports", "runs"]
        && canonical_owner(parts[2])
        && parts[3] == "generated"
        && matches!(parts[4], "builds" | "images" | "qemu")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retained_command_selection_is_exact_and_excludes_other_producer_graphs() {
        assert_eq!(
            Layout::for_command(&["test", "desktop-current"]),
            Layout::RetainedDesktop
        );
        assert_eq!(
            Layout::for_command(&["test", "desktop-motion-current"]),
            Layout::RetainedDesktop
        );
        assert_eq!(
            Layout::for_command(&["test", "settings-reference-current"]),
            Layout::RetainedDesktop
        );
        for command in [
            "desktop-notes",
            "desktop-notes-accelerated",
            "desktop-files",
            "desktop-files-accelerated",
            "desktop-remove",
            "desktop-remove-accelerated",
            "desktop-default",
            "desktop-motion",
            "desktop-software",
            "preview",
            "monitor",
            "multiwindow",
            "multiwindow-pressure",
            "desktop-recovery",
        ] {
            assert_eq!(
                Layout::for_command(&["test", command, "--image", "saved-image.json"]),
                Layout::RetainedDesktop
            );
            assert_eq!(
                Layout::for_command(&["test", command, "--image", "saved-image.json", "extra"]),
                Layout::Legacy
            );
        }
        for args in [
            vec!["verify"],
            vec!["test", "boot"],
            vec!["test", "desktop-current", "extra"],
            vec!["test", "desktop-motion-current", "extra"],
            vec!["test", "desktop-motion-current", "--image", "image.json"],
            vec!["test", "desktop-notes"],
            vec!["desktop", "--image", "image.json"],
            vec!["desktop", "--resolution", "0x0"],
            vec!["desktop", "--headless", "--headless"],
            vec!["init-user-volume"],
        ] {
            assert_eq!(Layout::for_command(&args), Layout::Legacy);
        }
    }
    #[test]
    fn desktop_build_retention_is_the_exact_already_audited_graph() {
        assert_eq!(
            Layout::for_command(&["build", "--profile", "desktop-qemu-x64"]),
            Layout::RetainedDesktop
        );
        for args in [
            vec!["build", "--profile", "desktop-qemu-x64", "extra"],
            vec!["build", "--profile", "text-qemu-x64"],
            vec!["build", "--profile", "desktop-dev", "--from", "image.json"],
        ] {
            assert_eq!(Layout::for_command(&args), Layout::Legacy);
        }
    }
    #[test]
    fn interactive_retention_requires_the_actual_validated_launch_options() {
        for args in [
            vec!["desktop"],
            vec!["desktop", "--headless", "--resolution", "3840x2160"],
            vec!["desktop", "--accelerator", "tcg"],
        ] {
            assert_eq!(Layout::for_command(&args), Layout::RetainedDesktop);
        }
    }
    #[test]
    fn explicit_layout_does_not_fallback_or_accept_foreign_owners() {
        let owner = "host-123-456";
        let v2 = Layout::RetainedDesktop;
        assert_eq!(
            v2.path(owner, "out/qemu/host-123-456-session-7").unwrap(),
            "reports/runs/host-123-456/generated/qemu/session-7"
        );
        for invalid in [
            "out/session/host-123-456",
            "out/builds/host-123-4567/guest",
            "out/builds/host-123-456-other/guest",
            "out/images/host-123-456-",
            "out/images/host-123-456-other",
            "out/qemu/host-123-456-session-07",
            "out/builds/host-7-8/guest",
            "out/builds/../guest",
        ] {
            assert!(v2.path(owner, invalid).is_err());
        }
        for declaration in [
            json!(3),
            json!({"version":3}),
            json!({"version":2,"kind":"retained-desktop","owner":"host-7-8"}),
        ] {
            assert!(Layout::from_report(&json!({"id":owner,"output_layout":declaration})).is_err());
        }
        assert_eq!(
            Layout::from_report(&json!({"id":owner})).unwrap(),
            Layout::Legacy
        );
        assert_eq!(
            Layout::from_report(&json!({"id":owner,"output_layout":v2.declaration(owner)}))
                .unwrap(),
            v2
        );
        assert!(retained_directory(
            "reports/runs/host-123-456/generated/builds/guest"
        ));
        assert_eq!(
            v2.path(owner, "out/builds/host-123-456/guest/pass-0")
                .unwrap(),
            "reports/runs/host-123-456/generated/builds/guest/pass-0"
        );
        assert!(!retained_directory(
            "reports/runs/host-01-2/generated/builds/host-01-2"
        ));
        assert!(!retained_directory(
            "reports/runs/host-123-456/generated/storage/host-123-456"
        ));
    }
}

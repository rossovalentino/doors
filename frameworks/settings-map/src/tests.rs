use super::*;
extern crate std;
use std::vec::Vec;

#[test]
fn route_identities_never_move_and_every_page_sits_under_a_rail_category() {
    for (index, page) in Page::ALL.iter().enumerate() {
        assert_eq!(*page as usize, index);
        assert_eq!(Page::from_index(index), Some(*page));
        assert!(Page::NAV.contains(&page.category()), "{page:?}");
        assert!(page.depth() <= 2, "{page:?} is nested too deeply");
        let crumbs: Vec<_> = page.crumbs().collect();
        assert_eq!(crumbs.len(), page.depth() + 1);
        assert_eq!(*crumbs.last().unwrap(), *page);
        assert_eq!(crumbs[0], page.category());
        // Names are unique among a page's siblings (Windows reuses some, such
        // as Keyboard under Accessibility and under Bluetooth & devices; the
        // breadcrumb and search results show where each one is).
        assert!(
            Page::ALL[..index]
                .iter()
                .all(|p| p.nav_title() != page.nav_title() || p.parent() != page.parent()),
            "{page:?}"
        );
    }
    assert_eq!(Page::from_index(Page::COUNT), None);
    // Identities reported to existing guest scenarios.
    assert_eq!(Page::About as usize, 12);
    assert_eq!(Page::ContextMenu as usize, 16);
    assert_eq!(Page::Registry as usize, 17);
    assert_eq!(Page::DevicesDrivers as usize, 18);
    assert_eq!(Page::DeveloperTools as usize, 32);
    assert_eq!(Page::SystemAdvanced as usize, 44);
    assert_eq!(Page::Typing as usize, 48);
    for tool in Page::TOOLS {
        assert_eq!(tool.parent(), Page::SystemAdvanced);
    }
    assert_eq!(
        Page::DiskManagement.crumbs().collect::<Vec<_>>(),
        [Page::System, Page::Storage, Page::DiskManagement]
    );
}
#[test]
fn every_redirect_key_is_reachable_and_names_a_real_section() {
    for (i, r) in REDIRECTS.iter().enumerate() {
        assert!(
            REDIRECTS[..i].iter().all(|o| o.key != r.key),
            "duplicate key {}",
            r.key
        );
        assert_eq!(
            resolve(r.key),
            Some((r.page, r.section)),
            "{} does not resolve to its own row",
            r.key
        );
        if let Some(anchor) = r.section {
            assert!(section(r.page, anchor).is_some(), "{} -> {anchor}", r.key);
        }
        assert!(!r.name.is_empty());
    }
    for (i, s) in SECTIONS.iter().enumerate() {
        assert!(
            SECTIONS[..i]
                .iter()
                .all(|o| !(o.page == s.page && o.anchor == s.anchor)),
            "{}",
            s.anchor
        );
        assert!(!s.title.is_empty());
    }
}
/// The legacy surfaces of docs/design/settings-unification.md, written the
/// ways people type them (any case, paths, quotes, extensions, arguments).
const INVENTORY: &[(&str, Page, Option<&str>)] = &[
    ("control", Page::Home, None),
    ("CONTROL.EXE", Page::Home, None),
    (r"C:\Windows\System32\control.exe", Page::Home, None),
    ("control /name Microsoft.System", Page::About, None),
    ("control.exe -name microsoft.system", Page::About, None),
    ("control system", Page::About, None),
    ("sysdm.cpl", Page::About, None),
    ("control sysdm.cpl", Page::About, None),
    ("control sysdm.cpl,,1", Page::DevicesDrivers, None),
    (
        "control sysdm.cpl,,3",
        Page::SystemAdvanced,
        Some("administrative-tools"),
    ),
    (
        "sysdm.cpl ,4",
        Page::BackupRestore,
        Some("point-in-time-restore"),
    ),
    (
        "control sysdm.cpl,,5",
        Page::SystemAdvanced,
        Some("remote-desktop"),
    ),
    (
        "rundll32 shell32.dll,Control_RunDLL sysdm.cpl,,3",
        Page::SystemAdvanced,
        Some("administrative-tools"),
    ),
    ("SystemPropertiesComputerName", Page::About, None),
    ("SystemPropertiesHardware.exe", Page::DevicesDrivers, None),
    (
        "SystemPropertiesAdvanced",
        Page::SystemAdvanced,
        Some("administrative-tools"),
    ),
    ("SystemPropertiesPerformance", Page::VisualEffects, None),
    (
        "SystemPropertiesProtection",
        Page::BackupRestore,
        Some("point-in-time-restore"),
    ),
    (
        "SystemPropertiesRemote",
        Page::SystemAdvanced,
        Some("remote-desktop"),
    ),
    (
        "rundll32.exe sysdm.cpl,EditEnvironmentVariables",
        Page::Environment,
        None,
    ),
    ("powercfg.cpl", Page::PowerManagement, None),
    (
        "control /name Microsoft.PowerOptions",
        Page::PowerManagement,
        None,
    ),
    ("mmsys.cpl", Page::Sound, None),
    ("control mmsys.cpl,,1", Page::Sound, None),
    (
        "ncpa.cpl",
        Page::NetworkAdministration,
        Some("network-adapters"),
    ),
    (
        "control netconnections",
        Page::NetworkAdministration,
        Some("network-adapters"),
    ),
    (
        "control /name Microsoft.NetworkAndSharingCenter",
        Page::NetworkAdministration,
        None,
    ),
    ("appwiz.cpl", Page::InstalledApps, None),
    (
        "control /name Microsoft.ProgramsAndFeatures",
        Page::InstalledApps,
        None,
    ),
    ("control userpasswords", Page::UsersGroups, None),
    (
        "control userpasswords2",
        Page::Accounts,
        Some("other-users"),
    ),
    ("netplwiz", Page::Accounts, Some("other-users")),
    (
        "control /name Microsoft.CredentialManager",
        Page::Accounts,
        Some("saved-credentials"),
    ),
    (
        "control folders",
        Page::SystemAdvanced,
        Some("file-explorer"),
    ),
    ("control fonts", Page::Fonts, None),
    ("explorer shell:Fonts", Page::Fonts, None),
    ("intl.cpl", Page::LanguageRegion, Some("regional-format")),
    (
        "control international",
        Page::LanguageRegion,
        Some("regional-format"),
    ),
    ("timedate.cpl", Page::DateTime, None),
    ("control date/time", Page::DateTime, None),
    ("main.cpl", Page::Mouse, None),
    ("control mouse", Page::Mouse, None),
    ("control keyboard", Page::Bluetooth, Some("keyboard")),
    (
        "control /name Microsoft.IndexingOptions",
        Page::Search,
        Some("find-files"),
    ),
    (
        "inetcpl.cpl",
        Page::NetworkAdministration,
        Some("internet-options"),
    ),
    ("firewall.cpl", Page::Firewall, None),
    ("wf.msc", Page::NetworkAdministration, Some("firewall")),
    ("control printers", Page::Bluetooth, Some("printers")),
    (
        "control /name Microsoft.DevicesAndPrinters",
        Page::Bluetooth,
        Some("printers"),
    ),
    ("access.cpl", Page::Accessibility, None),
    (
        "control /name Microsoft.EaseOfAccessCenter",
        Page::Accessibility,
        None,
    ),
    (
        "control admintools",
        Page::SystemAdvanced,
        Some("administrative-tools"),
    ),
    (
        "control /name Microsoft.Recovery",
        Page::BackupRestore,
        None,
    ),
    ("wscui.cpl", Page::Privacy, Some("doors-security")),
    (
        "control /name Microsoft.ActionCenter",
        Page::Privacy,
        Some("doors-security"),
    ),
    ("desk.cpl", Page::Display, None),
    ("control desktop", Page::Personalization, None),
    ("control color", Page::Colors, None),
    (
        "rundll32 shell32.dll,Control_RunDLL desk.cpl,,0",
        Page::Themes,
        Some("desktop-icons"),
    ),
    ("bthprops.cpl", Page::Bluetooth, None),
    ("joy.cpl", Page::DevicesDrivers, Some("game-controllers")),
    (
        "compmgmt.msc",
        Page::SystemAdvanced,
        Some("administrative-tools"),
    ),
    ("devmgmt.msc", Page::DevicesDrivers, None),
    ("mmc devmgmt.msc", Page::DevicesDrivers, None),
    (
        r"mmc.exe C:\Windows\system32\devmgmt.msc",
        Page::DevicesDrivers,
        None,
    ),
    ("hdwwiz.cpl", Page::DevicesDrivers, None),
    ("diskmgmt.msc", Page::DiskManagement, None),
    ("services.msc", Page::Services, None),
    ("eventvwr", Page::EventViewer, None),
    ("eventvwr.msc /s", Page::EventViewer, None),
    ("taskschd.msc", Page::TaskScheduler, None),
    ("control schedtasks", Page::TaskScheduler, None),
    ("lusrmgr.msc", Page::UsersGroups, None),
    (
        "secpol.msc",
        Page::GroupPolicy,
        Some("local-security-policy"),
    ),
    ("gpedit.msc", Page::GroupPolicy, None),
    ("perfmon", Page::Performance, Some("performance")),
    ("perfmon.exe /res", Page::Performance, Some("performance")),
    ("resmon", Page::Performance, Some("performance")),
    ("regedit", Page::Registry, None),
    ("REGEDIT.EXE", Page::Registry, None),
    ("regedt32", Page::Registry, None),
    ("msconfig", Page::SystemConfiguration, None),
    ("dxdiag", Page::About, Some("device-specifications")),
    ("winver", Page::About, Some("doors-specifications")),
    ("msinfo32", Page::About, Some("device-specifications")),
    ("optionalfeatures", Page::Apps, Some("optional-features")),
    ("rstrui", Page::BackupRestore, Some("point-in-time-restore")),
    (
        "\"control\" \"/name\" \"Microsoft.Fonts\"",
        Page::Fonts,
        None,
    ),
    (
        "shell:::{26EE0668-A00A-44D7-9371-BEB064C98683}",
        Page::Home,
        None,
    ),
    (
        "explorer.exe shell:::{BB06C0E4-D293-4F75-8A90-CB05B6477EEE}",
        Page::About,
        None,
    ),
    ("::{ED7BA470-8E54-465E-825C-99712043E01C}", Page::Home, None),
    ("ms-settings:display", Page::Display, None),
    (
        "ms-settings:privacy-location?activationSource=SMC-IA",
        Page::Location,
        None,
    ),
    ("MS-SETTINGS:DISKSANDVOLUMES", Page::DiskManagement, None),
    ("ms-settings:startupapps", Page::Startup, None),
    ("ms-settings:not-a-real-page", Page::Home, None),
];
#[test]
fn every_inventory_surface_redirects_to_its_settings_page() {
    for (command, page, section) in INVENTORY {
        assert_eq!(resolve(command), Some((*page, *section)), "{command}");
    }
    for other in [
        "notepad",
        "cmd",
        "",
        "   ",
        "control nonsense",
        "foo.msc",
        "taskmgr",
        "explorer",
    ] {
        assert_eq!(resolve(other), None, "{other}");
    }
    // Non-ASCII or overlong text is never mistaken for a command.
    assert_eq!(resolve("régedit"), None);
    assert_eq!(resolve(&"x".repeat(400)), None);
}
#[test]
fn navigation_arguments_round_trip_and_reject_foreign_sections() {
    for page in Page::ALL {
        assert_eq!(navigate_arg(page, None), page as u32);
        assert_eq!(decode_navigate(page as u32), Some((page, None)));
    }
    for s in SECTIONS {
        let a = navigate_arg(s.page, Some(s.anchor));
        assert!(a >> 16 != 0);
        assert_eq!(decode_navigate(a), Some((s.page, Some(s))));
        // The same section under another page is rejected.
        let other = if s.page == Page::Home {
            Page::System
        } else {
            Page::Home
        };
        assert_eq!(decode_navigate(other as u32 | (a & !0xffff)), None);
    }
    // An unknown anchor is dropped, not mapped elsewhere.
    assert_eq!(
        navigate_arg(Page::Display, Some("nope")),
        Page::Display as u32
    );
    assert_eq!(decode_navigate(Page::COUNT as u32), None);
    assert_eq!(decode_navigate(1 | (SECTIONS.len() as u32 + 1) << 16), None);
}
#[test]
fn search_finds_pages_sections_keywords_and_legacy_names_once_each() {
    let first = |q: &str| search(q).next().map(Hit::target);
    // Queries guest scenarios type: the first result never moved.
    for (query, page) in [
        ("colors", Page::Colors),
        ("visual effects", Page::VisualEffects),
        ("typing", Page::Typing),
        ("installed", Page::InstalledApps),
        ("installed apps", Page::InstalledApps),
        ("context menu", Page::ContextMenu),
        ("personalization", Page::Personalization),
        ("taskbar", Page::Taskbar),
        ("display", Page::Display),
        ("registry", Page::Registry),
        ("drivers", Page::DevicesDrivers),
        ("services", Page::Services),
        ("text size", Page::TextSize),
        ("contrast themes", Page::ContrastThemes),
    ] {
        assert_eq!(first(query), Some((page, None)), "{query}");
    }
    // Legacy names find their replacements.
    for (query, target) in [
        ("Device Manager", (Page::DevicesDrivers, None)),
        ("sysdm.cpl", (Page::About, None)),
        ("disk management", (Page::DiskManagement, None)),
        ("Event Viewer", (Page::EventViewer, None)),
        ("Task Scheduler", (Page::TaskScheduler, None)),
        ("group policy", (Page::GroupPolicy, None)),
        ("Programs and Features", (Page::InstalledApps, None)),
        ("Control Panel", (Page::Home, None)),
        (
            "Credential Manager",
            (Page::Accounts, Some("saved-credentials")),
        ),
        ("regedit", (Page::Registry, None)),
        ("msconfig", (Page::SystemConfiguration, None)),
    ] {
        assert!(
            search(query).any(|h| h.target() == target),
            "{query} -> {target:?}"
        );
    }
    // Sections are results of their own.
    assert!(search("environment").any(|h| h.target() == (Page::Environment, None)));
    assert!(search("night light").any(|h| h.target() == (Page::Display, Some("night-light"))));
    // One result per destination.
    let all: Vec<_> = search("e").map(Hit::target).collect();
    for (i, t) in all.iter().enumerate() {
        assert!(!all[..i].contains(t), "{t:?}");
    }
    assert_eq!(search("").count(), 0);
    assert_eq!(search("   ").count(), 0);
    assert_eq!(search("zz-no-such-setting").count(), 0);
}

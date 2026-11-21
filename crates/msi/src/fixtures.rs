//! INSTALL-A's packages, built with `write` (no WiX, no Windows SDK): Doors
//! Tools 1.0 and 2.0 (a major upgrade of it) and Doors Companion, which
//! shares a component with Doors Tools. Each has its files in an embedded
//! MSZIP cabinet, registry values, a Start menu shortcut, a file
//! association, an optional service, custom actions of every kind the
//! engine runs (property, error, EXE deferred with its rollback action,
//! DLL from the Binary table), the Upgrade table, launch conditions and
//! the standard sequences; `ui` adds the full-UI dialogs.
use crate::Value;
use crate::summary::{Property, pid};
use crate::write::{Builder, kind};
use alloc::{format, string::String, vec, vec::Vec};

pub const TOOLS_UPGRADE: &str = "{3D1C9A57-8E2B-4F60-9C14-7A5B2E0D6F11}";
pub const TOOLS_1: &str = "{3D1C9A57-0001-4F60-9C14-7A5B2E0D6F11}";
pub const TOOLS_2: &str = "{3D1C9A57-0002-4F60-9C14-7A5B2E0D6F11}";
pub const COMPANION: &str = "{5E2B7C19-0001-4A3D-8F6E-2C9D1B0A7E33}";
pub const COMPANION_UPGRADE: &str = "{5E2B7C19-8E2B-4A3D-8F6E-2C9D1B0A7E33}";
/// The component both products install (C:\Program Files\Common
/// Files\Doors\doors-shared.txt).
pub const SHARED_COMPONENT: &str = "{9A4F2E61-3B7C-4D18-A5E0-6C2B9F1D8E44}";

/// The programs a package carries (the host tests use stand-ins).
#[derive(Clone, Debug, Default)]
pub struct Payload {
    /// toolprobe.exe: the product's program, the association's handler
    /// and the EXE custom actions' program.
    pub tool: Vec<u8>,
    /// A service program (InstallServices); None leaves the service out.
    pub service: Option<Vec<u8>>,
    /// caprobe.dll: the DLL custom action (Binary table); None leaves it
    /// out.
    pub custom_action_dll: Option<Vec<u8>>,
}

struct File {
    key: &'static str,
    component: &'static str,
    name: &'static str,
    data: Vec<u8>,
    version: Option<&'static str>,
}

fn declare(b: &mut Builder) {
    use kind::*;
    b.table(
        "Property",
        &[("Property", key(string(72))), ("Value", text(0))],
    );
    b.table(
        "Directory",
        &[
            ("Directory", key(string(72))),
            ("Directory_Parent", null(string(72))),
            ("DefaultDir", text(255)),
        ],
    );
    b.table(
        "Component",
        &[
            ("Component", key(string(72))),
            ("ComponentId", null(string(38))),
            ("Directory_", string(72)),
            ("Attributes", I2),
            ("Condition", null(string(255))),
            ("KeyPath", null(string(72))),
        ],
    );
    b.table(
        "File",
        &[
            ("File", key(string(72))),
            ("Component_", string(72)),
            ("FileName", text(255)),
            ("FileSize", I4),
            ("Version", null(string(72))),
            ("Language", null(string(20))),
            ("Attributes", null(I2)),
            ("Sequence", I4),
        ],
    );
    b.table(
        "Media",
        &[
            ("DiskId", key(I2)),
            ("LastSequence", I4),
            ("DiskPrompt", null(text(64))),
            ("Cabinet", null(string(255))),
            ("VolumeLabel", null(string(32))),
            ("Source", null(string(72))),
        ],
    );
    b.table(
        "Feature",
        &[
            ("Feature", key(string(38))),
            ("Feature_Parent", null(string(38))),
            ("Title", null(text(64))),
            ("Description", null(text(255))),
            ("Display", null(I2)),
            ("Level", I2),
            ("Directory_", null(string(72))),
            ("Attributes", I2),
        ],
    );
    b.table(
        "FeatureComponents",
        &[
            ("Feature_", key(string(38))),
            ("Component_", key(string(72))),
        ],
    );
    b.table(
        "Registry",
        &[
            ("Registry", key(string(72))),
            ("Root", I2),
            ("Key", text(255)),
            ("Name", null(text(255))),
            ("Value", null(text(0))),
            ("Component_", string(72)),
        ],
    );
    b.table(
        "Shortcut",
        &[
            ("Shortcut", key(string(72))),
            ("Directory_", string(72)),
            ("Name", text(128)),
            ("Component_", string(72)),
            ("Target", string(72)),
            ("Arguments", null(string(255))),
            ("Description", null(text(255))),
            ("Hotkey", null(I2)),
            ("Icon_", null(string(72))),
            ("IconIndex", null(I2)),
            ("ShowCmd", null(I2)),
            ("WkDir", null(string(72))),
        ],
    );
    b.table("Binary", &[("Name", key(string(72))), ("Data", BINARY)]);
    b.table(
        "CustomAction",
        &[
            ("Action", key(string(72))),
            ("Type", I2),
            ("Source", null(string(72))),
            ("Target", null(string(255))),
        ],
    );
    for t in ["InstallExecuteSequence", "InstallUISequence"] {
        b.table(
            t,
            &[
                ("Action", key(string(72))),
                ("Condition", null(string(255))),
                ("Sequence", null(I2)),
            ],
        );
    }
    b.table(
        "Upgrade",
        &[
            ("UpgradeCode", key(string(38))),
            ("VersionMin", key(null(string(20)))),
            ("VersionMax", key(null(string(20)))),
            ("Language", key(null(string(255)))),
            ("Attributes", key(I4)),
            ("Remove", null(text(255))),
            ("ActionProperty", string(72)),
        ],
    );
    b.table(
        "LaunchCondition",
        &[("Condition", key(string(255))), ("Description", text(255))],
    );
    b.table(
        "Extension",
        &[
            ("Extension", key(string(255))),
            ("Component_", key(string(72))),
            ("ProgId_", null(string(255))),
            ("MIME_", null(string(64))),
            ("Feature_", string(38)),
        ],
    );
    b.table(
        "ProgId",
        &[
            ("ProgId", key(string(255))),
            ("ProgId_Parent", null(string(255))),
            ("Class_", null(string(38))),
            ("Description", null(text(255))),
            ("Icon_", null(string(72))),
            ("IconIndex", null(I2)),
        ],
    );
    b.table(
        "Verb",
        &[
            ("Extension_", key(string(255))),
            ("Verb", key(string(32))),
            ("Sequence", null(I2)),
            ("Command", null(text(255))),
            ("Argument", null(text(255))),
        ],
    );
    b.table(
        "ServiceInstall",
        &[
            ("ServiceInstall", key(string(72))),
            ("Name", string(255)),
            ("DisplayName", null(text(255))),
            ("ServiceType", I4),
            ("StartType", I4),
            ("ErrorControl", I4),
            ("LoadOrderGroup", null(string(255))),
            ("Dependencies", null(string(255))),
            ("StartName", null(string(255))),
            ("Password", null(string(255))),
            ("Arguments", null(string(255))),
            ("Component_", string(72)),
            ("Description", null(text(255))),
        ],
    );
    b.table(
        "ServiceControl",
        &[
            ("ServiceControl", key(string(72))),
            ("Name", text(255)),
            ("Event", I2),
            ("Arguments", null(text(255))),
            ("Wait", null(I2)),
            ("Component_", string(72)),
        ],
    );
    b.table(
        "RemoveFile",
        &[
            ("FileKey", key(string(72))),
            ("Component_", string(72)),
            ("FileName", null(text(255))),
            ("DirProperty", string(72)),
            ("InstallMode", I2),
        ],
    );
    b.table(
        "CreateFolder",
        &[
            ("Directory_", key(string(72))),
            ("Component_", key(string(72))),
        ],
    );
    b.table("Error", &[("Error", key(I2)), ("Message", null(text(0)))]);
}

/// The standard InstallExecuteSequence (Windows Installer's recommended
/// numbers), plus `extra` rows.
fn execute_sequence(b: &mut Builder, extra: &[(&str, &str, i32)]) {
    let rows: [(&str, &str, i32); 51] = [
        ("FindRelatedProducts", "", 25),
        ("AppSearch", "", 50),
        ("LaunchConditions", "", 100),
        ("ValidateProductID", "", 700),
        ("CostInitialize", "", 800),
        ("FileCost", "", 900),
        ("CostFinalize", "", 1000),
        ("InstallValidate", "", 1400),
        ("RemoveExistingProducts", "", 1450),
        ("InstallInitialize", "", 1500),
        ("ProcessComponents", "", 1600),
        ("UnpublishComponents", "", 1700),
        ("UnpublishFeatures", "", 1800),
        ("StopServices", "VersionNT", 1900),
        ("DeleteServices", "VersionNT", 2000),
        ("SelfUnregModules", "", 2200),
        ("UnregisterTypeLibraries", "", 2300),
        ("UnregisterFonts", "", 2500),
        ("RemoveRegistryValues", "", 2600),
        ("UnregisterClassInfo", "", 2700),
        ("UnregisterExtensionInfo", "", 2800),
        ("UnregisterProgIdInfo", "", 2900),
        ("UnregisterMIMEInfo", "", 3000),
        ("RemoveIniValues", "", 3100),
        ("RemoveShortcuts", "", 3200),
        ("RemoveEnvironmentStrings", "", 3300),
        ("RemoveDuplicateFiles", "", 3400),
        ("RemoveFiles", "", 3500),
        ("RemoveFolders", "", 3600),
        ("CreateFolders", "", 3700),
        ("MoveFiles", "", 3800),
        ("InstallFiles", "", 4000),
        ("DuplicateFiles", "", 4210),
        ("CreateShortcuts", "", 4500),
        ("RegisterClassInfo", "", 4600),
        ("RegisterExtensionInfo", "", 4700),
        ("RegisterProgIdInfo", "", 4800),
        ("RegisterMIMEInfo", "", 4900),
        ("WriteRegistryValues", "", 5000),
        ("WriteIniValues", "", 5100),
        ("WriteEnvironmentStrings", "", 5200),
        ("RegisterFonts", "", 5300),
        ("RegisterTypeLibraries", "", 5500),
        ("SelfRegModules", "", 5600),
        ("InstallServices", "VersionNT", 5800),
        ("StartServices", "VersionNT", 5900),
        ("RegisterUser", "", 6000),
        ("RegisterProduct", "", 6100),
        ("PublishComponents", "", 6200),
        ("PublishFeatures", "", 6300),
        ("PublishProduct", "", 6400),
    ];
    for (a, c, s) in rows
        .iter()
        .chain(extra.iter())
        .chain([("InstallFinalize", "", 6600)].iter())
    {
        b.row(
            "InstallExecuteSequence",
            vec![
                Value::Str(String::from(*a)),
                if c.is_empty() {
                    Value::Null
                } else {
                    Value::Str(String::from(*c))
                },
                Value::Int(*s),
            ],
        );
    }
}

fn ui_sequence_base(b: &mut Builder, extra: &[(&str, &str, i32)]) {
    let rows: [(&str, &str, i32); 7] = [
        ("FindRelatedProducts", "", 25),
        ("AppSearch", "", 50),
        ("LaunchConditions", "", 100),
        ("CostInitialize", "", 800),
        ("FileCost", "", 900),
        ("CostFinalize", "", 1000),
        ("ExecuteAction", "", 1300),
    ];
    for (a, c, s) in rows.iter().chain(extra.iter()) {
        b.row(
            "InstallUISequence",
            vec![
                Value::Str(String::from(*a)),
                if c.is_empty() {
                    Value::Null
                } else {
                    Value::Str(String::from(*c))
                },
                Value::Int(*s),
            ],
        );
    }
}

fn summary(b: &mut Builder, name: &str, package: &str) {
    summary_for(b, name, package, "x64;1033");
}
fn summary_for(b: &mut Builder, name: &str, package: &str, template: &str) {
    b.summary(pid::CODEPAGE, Property::Int(1252));
    b.summary(
        pid::TITLE,
        Property::Text(String::from("Installation Database")),
    );
    b.summary(pid::SUBJECT, Property::Text(String::from(name)));
    b.summary(pid::AUTHOR, Property::Text(String::from("Macrohard Test")));
    b.summary(pid::KEYWORDS, Property::Text(String::from("Installer")));
    b.summary(
        pid::COMMENTS,
        Property::Text(format!(
            "This installer database contains the logic and data required to install {name}."
        )),
    );
    b.summary(pid::TEMPLATE, Property::Text(String::from(template)));
    b.summary(pid::REVISION, Property::Text(String::from(package)));
    // 2026-11-01 00:00 UTC.
    let filetime: u64 = 134_379_648_000_000_000;
    b.summary(pid::CREATED, Property::Time(filetime));
    b.summary(pid::SAVED, Property::Time(filetime));
    b.summary(pid::PAGE_COUNT, Property::Int(500));
    b.summary(pid::WORD_COUNT, Property::Int(2));
    b.summary(
        pid::APPLICATION,
        Property::Text(String::from("Doors xtask")),
    );
    b.summary(pid::SECURITY, Property::Int(2));
}

fn cabinet(b: &mut Builder, files: &[File]) {
    let mut cab = door_cab::Writer::new(door_cab::Compression::MsZip);
    // 2026-11-01 12:00.
    let date: u16 = ((2026 - 1980) << 9) | (11 << 5) | 1;
    let time: u16 = 12 << 11;
    for f in files {
        cab.add(f.key, &f.data, date, time, 0x20);
    }
    b.stream("tools.cab", cab.finish());
    b.row(
        "Media",
        vec![
            Value::Int(1),
            Value::Int(files.len() as i32),
            Value::Null,
            Value::Str(String::from("#tools.cab")),
            Value::Null,
            Value::Null,
        ],
    );
}

fn add_files(b: &mut Builder, files: &[File]) {
    for (i, f) in files.iter().enumerate() {
        b.row(
            "File",
            vec![
                Value::Str(String::from(f.key)),
                Value::Str(String::from(f.component)),
                Value::Str(String::from(f.name)),
                Value::Int(f.data.len() as i32),
                f.version
                    .map_or(Value::Null, |v| Value::Str(String::from(v))),
                Value::Null,
                Value::Int(0x4000),
                Value::Int(i as i32 + 1),
            ],
        );
    }
}

/// A 24-bit bottom-up BMP of `width` x `height` from `pixel(x, y)` (RGB).
fn bitmap(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let row = (width * 3).div_ceil(4) * 4;
    let image = row * height;
    let mut out = Vec::with_capacity((54 + image) as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + image).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&image.to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for y in (0..height).rev() {
        let start = out.len();
        for x in 0..width {
            let [r, g, b] = pixel(x, y);
            out.extend_from_slice(&[b, g, r]);
        }
        out.resize(start + row as usize, 0);
    }
    out
}

/// The package's own dialogs (the full UI): welcome with the license,
/// progress, completion, interruption, failure, the cancel question and
/// maintenance (repair or remove), in installer units of a 370 x 270
/// dialog, with Doors' banner and side artwork.
fn ui(b: &mut Builder) {
    use kind::*;
    b.table(
        "Dialog",
        &[
            ("Dialog", key(string(72))),
            ("HCentering", I2),
            ("VCentering", I2),
            ("Width", I2),
            ("Height", I2),
            ("Attributes", null(I4)),
            ("Title", null(text(128))),
            ("Control_First", string(50)),
            ("Control_Default", null(string(50))),
            ("Control_Cancel", null(string(50))),
        ],
    );
    b.table(
        "Control",
        &[
            ("Dialog_", key(string(72))),
            ("Control", key(string(50))),
            ("Type", string(20)),
            ("X", I2),
            ("Y", I2),
            ("Width", I2),
            ("Height", I2),
            ("Attributes", null(I4)),
            ("Property", null(string(72))),
            ("Text", null(text(0))),
            ("Control_Next", null(string(50))),
            ("Help", null(text(50))),
        ],
    );
    b.table(
        "ControlEvent",
        &[
            ("Dialog_", key(string(72))),
            ("Control_", key(string(50))),
            ("Event", key(string(50))),
            ("Argument", key(string(255))),
            ("Condition", key(null(string(255)))),
            ("Ordering", null(I2)),
        ],
    );
    b.table(
        "ControlCondition",
        &[
            ("Dialog_", key(string(72))),
            ("Control_", key(string(50))),
            ("Action", key(string(50))),
            ("Condition", key(string(255))),
        ],
    );
    b.table(
        "EventMapping",
        &[
            ("Dialog_", key(string(72))),
            ("Control_", key(string(50))),
            ("Event", key(string(50))),
            ("Attribute", string(50)),
        ],
    );
    b.table(
        "TextStyle",
        &[
            ("TextStyle", key(string(72))),
            ("FaceName", string(32)),
            ("Size", I2),
            ("Color", null(I4)),
            ("StyleBits", null(I2)),
        ],
    );
    b.table(
        "CheckBox",
        &[("Property", key(string(72))), ("Value", null(string(64)))],
    );
    b.table(
        "UIText",
        &[("Key", key(string(72))), ("Text", null(text(255)))],
    );
    for row in [
        ["DoorsFontNormal", "Segoe UI", "#9", "", "#0"],
        ["DoorsFontTitle", "Segoe UI", "#9", "", "#1"],
        ["DoorsFontBigger", "Segoe UI", "#12", "", "#0"],
    ] {
        b.row_text("TextStyle", &row);
    }
    b.row_text("CheckBox", &["LicenseAccepted", "1"]);
    for (k, v) in [
        ("bytes", "bytes"),
        ("KB", "KB"),
        ("MB", "MB"),
        ("GB", "GB"),
        (
            "TimeRemaining",
            "Time remaining: {[1] minutes }{[2] seconds}",
        ),
    ] {
        b.row_text("UIText", &[k, v]);
    }
    // Doors' artwork: the side panel and the banner.
    let side = bitmap(493, 312, |x, y| {
        if x < 164 {
            let t = (y * 60 / 312) as u8;
            [0, 95 + t / 2, 184 + t]
        } else {
            [255, 255, 255]
        }
    });
    let banner = bitmap(493, 58, |x, y| {
        let dx = x as i32 - 452;
        let dy = y as i32 - 29;
        if dx * dx + dy * dy < 17 * 17 {
            [0, 103, 192]
        } else {
            [255, 255, 255]
        }
    });
    for (name, data) in [("DoorsDialog", side), ("DoorsBanner", banner)] {
        b.stream(&format!("Binary.{name}"), data);
        b.row(
            "Binary",
            vec![
                Value::Str(String::from(name)),
                Value::Stream(format!("Binary.{name}")),
            ],
        );
    }
    let license = "{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0\\fnil Segoe UI;}}\\viewkind4\\uc1\\pard\\f0\\fs18 DOORS TOOLS LICENSE\\par\\par This test package installs Doors Tools to check Windows Installer on Doors. You may install, repair and remove it as often as you like.\\par\\par It comes with no warranty.\\par}";
    let dialogs: [[&str; 10]; 7] = [
        [
            "WelcomeEulaDlg",
            "#50",
            "#50",
            "#370",
            "#270",
            "#3",
            "[ProductName] Setup",
            "Install",
            "Install",
            "Cancel",
        ],
        [
            "ProgressDlg",
            "#50",
            "#50",
            "#370",
            "#270",
            "#1",
            "[ProductName] Setup",
            "Cancel",
            "Cancel",
            "Cancel",
        ],
        [
            "ExitDialog",
            "#50",
            "#50",
            "#370",
            "#270",
            "#3",
            "[ProductName] Setup",
            "Finish",
            "Finish",
            "Finish",
        ],
        [
            "UserExit",
            "#50",
            "#50",
            "#370",
            "#270",
            "#3",
            "[ProductName] Setup",
            "Finish",
            "Finish",
            "Finish",
        ],
        [
            "FatalError",
            "#50",
            "#50",
            "#370",
            "#270",
            "#3",
            "[ProductName] Setup",
            "Finish",
            "Finish",
            "Finish",
        ],
        [
            "CancelDlg",
            "#50",
            "#10",
            "#260",
            "#85",
            "#3",
            "[ProductName] Setup",
            "No",
            "No",
            "No",
        ],
        [
            "MaintenanceTypeDlg",
            "#50",
            "#50",
            "#370",
            "#270",
            "#3",
            "[ProductName] Setup",
            "RepairButton",
            "RepairButton",
            "Cancel",
        ],
    ];
    for d in dialogs {
        b.row_text("Dialog", &d);
    }
    let text = "#196611"; // visible, enabled, transparent, no prefix
    let hidden_text = "#196610";
    let controls: Vec<[&str; 12]> = vec![
        [
            "WelcomeEulaDlg",
            "Bitmap",
            "Bitmap",
            "#0",
            "#0",
            "#370",
            "#234",
            "#1",
            "",
            "DoorsDialog",
            "",
            "",
        ],
        [
            "WelcomeEulaDlg",
            "Title",
            "Text",
            "#135",
            "#20",
            "#220",
            "#40",
            text,
            "",
            "{\\DoorsFontBigger}Welcome to the [ProductName] Setup Wizard",
            "",
            "",
        ],
        [
            "WelcomeEulaDlg",
            "Description",
            "Text",
            "#135",
            "#62",
            "#220",
            "#20",
            text,
            "",
            "Read the license terms, then choose Install.",
            "",
            "",
        ],
        [
            "WelcomeEulaDlg",
            "LicenseText",
            "ScrollableText",
            "#135",
            "#86",
            "#220",
            "#100",
            "#7",
            "",
            license,
            "LicenseAcceptedCheckBox",
            "",
        ],
        [
            "WelcomeEulaDlg",
            "LicenseAcceptedCheckBox",
            "CheckBox",
            "#135",
            "#192",
            "#220",
            "#18",
            "#3",
            "LicenseAccepted",
            "I &accept the terms in the License Agreement",
            "Install",
            "",
        ],
        [
            "WelcomeEulaDlg",
            "BottomLine",
            "Line",
            "#0",
            "#234",
            "#370",
            "#0",
            "#1",
            "",
            "",
            "",
            "",
        ],
        [
            "WelcomeEulaDlg",
            "Back",
            "PushButton",
            "#156",
            "#243",
            "#56",
            "#17",
            "#1",
            "",
            "&Back",
            "",
            "",
        ],
        [
            "WelcomeEulaDlg",
            "Install",
            "PushButton",
            "#212",
            "#243",
            "#80",
            "#17",
            "#8388611",
            "",
            "&Install",
            "Cancel",
            "",
        ],
        [
            "WelcomeEulaDlg",
            "Cancel",
            "PushButton",
            "#304",
            "#243",
            "#56",
            "#17",
            "#3",
            "",
            "Cancel",
            "LicenseText",
            "",
        ],
        [
            "ProgressDlg",
            "BannerBitmap",
            "Bitmap",
            "#0",
            "#0",
            "#370",
            "#44",
            "#1",
            "",
            "DoorsBanner",
            "",
            "",
        ],
        [
            "ProgressDlg",
            "BannerLine",
            "Line",
            "#0",
            "#44",
            "#370",
            "#0",
            "#1",
            "",
            "",
            "",
            "",
        ],
        [
            "ProgressDlg",
            "Title",
            "Text",
            "#20",
            "#15",
            "#300",
            "#15",
            text,
            "",
            "{\\DoorsFontTitle}Installing [ProductName]",
            "",
            "",
        ],
        [
            "ProgressDlg",
            "TitleRemove",
            "Text",
            "#20",
            "#15",
            "#300",
            "#15",
            hidden_text,
            "",
            "{\\DoorsFontTitle}Removing [ProductName]",
            "",
            "",
        ],
        [
            "ProgressDlg",
            "Text",
            "Text",
            "#35",
            "#65",
            "#300",
            "#30",
            text,
            "",
            "Please wait while the Setup Wizard sets up [ProductName].",
            "",
            "",
        ],
        [
            "ProgressDlg",
            "StatusLabel",
            "Text",
            "#35",
            "#100",
            "#35",
            "#10",
            text,
            "",
            "Status:",
            "",
            "",
        ],
        [
            "ProgressDlg",
            "ActionText",
            "Text",
            "#70",
            "#100",
            "#265",
            "#10",
            "#3",
            "",
            "",
            "",
            "",
        ],
        [
            "ProgressDlg",
            "ProgressBar",
            "ProgressBar",
            "#35",
            "#115",
            "#300",
            "#10",
            "#65537",
            "",
            "Progress done",
            "",
            "",
        ],
        [
            "ProgressDlg",
            "BottomLine",
            "Line",
            "#0",
            "#234",
            "#370",
            "#0",
            "#1",
            "",
            "",
            "",
            "",
        ],
        [
            "ProgressDlg",
            "Back",
            "PushButton",
            "#180",
            "#243",
            "#56",
            "#17",
            "#1",
            "",
            "&Back",
            "",
            "",
        ],
        [
            "ProgressDlg",
            "Next",
            "PushButton",
            "#236",
            "#243",
            "#56",
            "#17",
            "#1",
            "",
            "&Next",
            "",
            "",
        ],
        [
            "ProgressDlg",
            "Cancel",
            "PushButton",
            "#304",
            "#243",
            "#56",
            "#17",
            "#3",
            "",
            "Cancel",
            "",
            "",
        ],
        [
            "ExitDialog",
            "Bitmap",
            "Bitmap",
            "#0",
            "#0",
            "#370",
            "#234",
            "#1",
            "",
            "DoorsDialog",
            "",
            "",
        ],
        [
            "ExitDialog",
            "Title",
            "Text",
            "#135",
            "#20",
            "#220",
            "#40",
            text,
            "",
            "{\\DoorsFontBigger}Completed the [ProductName] Setup Wizard",
            "",
            "",
        ],
        [
            "ExitDialog",
            "Description",
            "Text",
            "#135",
            "#70",
            "#220",
            "#40",
            text,
            "",
            "Click Finish to close the Setup Wizard.",
            "",
            "",
        ],
        [
            "ExitDialog",
            "BottomLine",
            "Line",
            "#0",
            "#234",
            "#370",
            "#0",
            "#1",
            "",
            "",
            "",
            "",
        ],
        [
            "ExitDialog",
            "Back",
            "PushButton",
            "#180",
            "#243",
            "#56",
            "#17",
            "#1",
            "",
            "&Back",
            "",
            "",
        ],
        [
            "ExitDialog",
            "Finish",
            "PushButton",
            "#236",
            "#243",
            "#56",
            "#17",
            "#3",
            "",
            "&Finish",
            "",
            "",
        ],
        [
            "ExitDialog",
            "Cancel",
            "PushButton",
            "#304",
            "#243",
            "#56",
            "#17",
            "#1",
            "",
            "Cancel",
            "",
            "",
        ],
        [
            "UserExit",
            "Bitmap",
            "Bitmap",
            "#0",
            "#0",
            "#370",
            "#234",
            "#1",
            "",
            "DoorsDialog",
            "",
            "",
        ],
        [
            "UserExit",
            "Title",
            "Text",
            "#135",
            "#20",
            "#220",
            "#40",
            text,
            "",
            "{\\DoorsFontBigger}[ProductName] Setup Wizard was interrupted",
            "",
            "",
        ],
        [
            "UserExit",
            "Description",
            "Text",
            "#135",
            "#70",
            "#220",
            "#60",
            text,
            "",
            "[ProductName] Setup stopped before it finished. Nothing on this computer was changed. Run the Setup Wizard again to install it later.",
            "",
            "",
        ],
        [
            "UserExit",
            "BottomLine",
            "Line",
            "#0",
            "#234",
            "#370",
            "#0",
            "#1",
            "",
            "",
            "",
            "",
        ],
        [
            "UserExit",
            "Finish",
            "PushButton",
            "#236",
            "#243",
            "#56",
            "#17",
            "#3",
            "",
            "&Finish",
            "",
            "",
        ],
        [
            "FatalError",
            "Bitmap",
            "Bitmap",
            "#0",
            "#0",
            "#370",
            "#234",
            "#1",
            "",
            "DoorsDialog",
            "",
            "",
        ],
        [
            "FatalError",
            "Title",
            "Text",
            "#135",
            "#20",
            "#220",
            "#40",
            text,
            "",
            "{\\DoorsFontBigger}[ProductName] Setup Wizard ended prematurely",
            "",
            "",
        ],
        [
            "FatalError",
            "Description",
            "Text",
            "#135",
            "#70",
            "#220",
            "#60",
            text,
            "",
            "[ProductName] Setup stopped because of an error. Nothing on this computer was changed. Run the Setup Wizard again to install it later.",
            "",
            "",
        ],
        [
            "FatalError",
            "BottomLine",
            "Line",
            "#0",
            "#234",
            "#370",
            "#0",
            "#1",
            "",
            "",
            "",
            "",
        ],
        [
            "FatalError",
            "Finish",
            "PushButton",
            "#236",
            "#243",
            "#56",
            "#17",
            "#3",
            "",
            "&Finish",
            "",
            "",
        ],
        [
            "CancelDlg",
            "Text",
            "Text",
            "#30",
            "#15",
            "#200",
            "#30",
            text,
            "",
            "Are you sure you want to cancel [ProductName] installation?",
            "",
            "",
        ],
        [
            "CancelDlg",
            "No",
            "PushButton",
            "#132",
            "#57",
            "#56",
            "#17",
            "#3",
            "",
            "&No",
            "Yes",
            "",
        ],
        [
            "CancelDlg",
            "Yes",
            "PushButton",
            "#72",
            "#57",
            "#56",
            "#17",
            "#3",
            "",
            "&Yes",
            "No",
            "",
        ],
        [
            "MaintenanceTypeDlg",
            "BannerBitmap",
            "Bitmap",
            "#0",
            "#0",
            "#370",
            "#44",
            "#1",
            "",
            "DoorsBanner",
            "",
            "",
        ],
        [
            "MaintenanceTypeDlg",
            "BannerLine",
            "Line",
            "#0",
            "#44",
            "#370",
            "#0",
            "#1",
            "",
            "",
            "",
            "",
        ],
        [
            "MaintenanceTypeDlg",
            "Title",
            "Text",
            "#15",
            "#6",
            "#340",
            "#15",
            text,
            "",
            "{\\DoorsFontTitle}Change, repair, or remove installation",
            "",
            "",
        ],
        [
            "MaintenanceTypeDlg",
            "Description",
            "Text",
            "#25",
            "#23",
            "#340",
            "#15",
            text,
            "",
            "Choose what to do with [ProductName].",
            "",
            "",
        ],
        [
            "MaintenanceTypeDlg",
            "RepairButton",
            "PushButton",
            "#40",
            "#65",
            "#80",
            "#17",
            "#8388611",
            "",
            "Re&pair",
            "RemoveButton",
            "",
        ],
        [
            "MaintenanceTypeDlg",
            "RepairText",
            "Text",
            "#60",
            "#85",
            "#280",
            "#30",
            text,
            "",
            "Fixes missing or damaged files, shortcuts and registry entries of the installed version.",
            "",
            "",
        ],
        [
            "MaintenanceTypeDlg",
            "RemoveButton",
            "PushButton",
            "#40",
            "#118",
            "#80",
            "#17",
            "#8388611",
            "",
            "&Remove",
            "Cancel",
            "",
        ],
        [
            "MaintenanceTypeDlg",
            "RemoveText",
            "Text",
            "#60",
            "#138",
            "#280",
            "#20",
            text,
            "",
            "Removes [ProductName] from this computer.",
            "",
            "",
        ],
        [
            "MaintenanceTypeDlg",
            "BottomLine",
            "Line",
            "#0",
            "#234",
            "#370",
            "#0",
            "#1",
            "",
            "",
            "",
            "",
        ],
        [
            "MaintenanceTypeDlg",
            "Back",
            "PushButton",
            "#180",
            "#243",
            "#56",
            "#17",
            "#1",
            "",
            "&Back",
            "",
            "",
        ],
        [
            "MaintenanceTypeDlg",
            "Cancel",
            "PushButton",
            "#304",
            "#243",
            "#56",
            "#17",
            "#3",
            "",
            "Cancel",
            "RepairButton",
            "",
        ],
    ];
    for c in &controls {
        b.row_text("Control", c);
    }
    let events: [[&str; 6]; 15] = [
        [
            "WelcomeEulaDlg",
            "Install",
            "EndDialog",
            "Return",
            "LicenseAccepted = \"1\"",
            "#1",
        ],
        [
            "WelcomeEulaDlg",
            "Cancel",
            "SpawnDialog",
            "CancelDlg",
            "1",
            "#1",
        ],
        [
            "ProgressDlg",
            "Cancel",
            "SpawnDialog",
            "CancelDlg",
            "1",
            "#1",
        ],
        ["ExitDialog", "Finish", "EndDialog", "Return", "1", "#1"],
        ["UserExit", "Finish", "EndDialog", "Exit", "1", "#1"],
        ["FatalError", "Finish", "EndDialog", "Exit", "1", "#1"],
        ["CancelDlg", "Yes", "EndDialog", "Exit", "1", "#1"],
        ["CancelDlg", "No", "EndDialog", "Return", "1", "#1"],
        [
            "MaintenanceTypeDlg",
            "RepairButton",
            "[REINSTALL]",
            "ALL",
            "1",
            "#1",
        ],
        [
            "MaintenanceTypeDlg",
            "RepairButton",
            "[REINSTALLMODE]",
            "omus",
            "1",
            "#2",
        ],
        [
            "MaintenanceTypeDlg",
            "RepairButton",
            "EndDialog",
            "Return",
            "1",
            "#3",
        ],
        [
            "MaintenanceTypeDlg",
            "RemoveButton",
            "[REMOVE]",
            "ALL",
            "1",
            "#1",
        ],
        [
            "MaintenanceTypeDlg",
            "RemoveButton",
            "EndDialog",
            "Return",
            "1",
            "#2",
        ],
        [
            "MaintenanceTypeDlg",
            "Cancel",
            "SpawnDialog",
            "CancelDlg",
            "1",
            "#1",
        ],
        [
            "WelcomeEulaDlg",
            "Install",
            "DoAction",
            "SetLicenseRecorded",
            "LicenseAccepted = \"1\"",
            "#0",
        ],
    ];
    for e in events {
        b.row_text("ControlEvent", &e);
    }
    b.row_text(
        "CustomAction",
        &["SetLicenseRecorded", "#51", "DOORS_LICENSE", "accepted"],
    );
    for c in [
        [
            "WelcomeEulaDlg",
            "Install",
            "Disable",
            "LicenseAccepted <> \"1\"",
        ],
        [
            "WelcomeEulaDlg",
            "Install",
            "Enable",
            "LicenseAccepted = \"1\"",
        ],
        ["ProgressDlg", "Title", "Hide", "REMOVE = \"ALL\""],
        ["ProgressDlg", "TitleRemove", "Show", "REMOVE = \"ALL\""],
    ] {
        b.row_text("ControlCondition", &c);
    }
    for m in [
        ["ProgressDlg", "ActionText", "ActionText", "Text"],
        ["ProgressDlg", "ProgressBar", "SetProgress", "Progress"],
    ] {
        b.row_text("EventMapping", &m);
    }
}

/// Doors Tools `version` 1 or 2 (a major upgrade: new product code, same
/// upgrade code).
pub fn tools(version: u32, payload: &Payload) -> Vec<u8> {
    let mut b = Builder::new();
    declare(&mut b);
    let (product, package, text_version) = if version >= 2 {
        (TOOLS_2, "{7B0E4D22-0002-4C1A-B8F3-5D6E7A8B9C02}", "2.0.0")
    } else {
        (TOOLS_1, "{7B0E4D22-0001-4C1A-B8F3-5D6E7A8B9C01}", "1.0.0")
    };
    let readme = if version >= 2 {
        "Doors Tools 2.0\r\nThe second release.\r\n"
    } else {
        "Doors Tools 1.0\r\nThe first release.\r\n"
    };
    for (k, v) in [
        ("ProductName", "Doors Tools"),
        ("ProductVersion", text_version),
        ("Manufacturer", "Macrohard Test"),
        ("ProductCode", product),
        ("UpgradeCode", TOOLS_UPGRADE),
        ("ProductLanguage", "1033"),
        ("ALLUSERS", "1"),
        (
            "ARPCOMMENTS",
            "Tools for testing Windows Installer on Doors",
        ),
        ("ARPPRODUCTICON", "[#fil_tool]"),
        (
            "SecureCustomProperties",
            "WIX_UPGRADE_DETECTED;WIX_DOWNGRADE_DETECTED",
        ),
        ("DefaultUIFont", "DoorsFontNormal"),
    ] {
        b.row_text("Property", &[k, v]);
    }
    for row in [
        ["TARGETDIR", "", "SourceDir"],
        ["ProgramFiles64Folder", "TARGETDIR", "."],
        [
            "INSTALLFOLDER",
            "ProgramFiles64Folder",
            "DOORST~1|Doors Tools",
        ],
        ["CommonFiles64Folder", "TARGETDIR", "."],
        ["SharedFolder", "CommonFiles64Folder", "Doors"],
        ["ProgramMenuFolder", "TARGETDIR", "."],
        ["CommonAppDataFolder", "TARGETDIR", "."],
    ] {
        b.row_text("Directory", &row);
    }
    let mut files = vec![
        File {
            key: "fil_tool",
            component: "cmp_tool",
            name: "TOOLPR~1.EXE|toolprobe.exe",
            data: payload.tool.clone(),
            version: Some(if version >= 2 { "2.0.0.0" } else { "1.0.0.0" }),
        },
        File {
            key: "fil_readme",
            component: "cmp_readme",
            name: "readme.txt",
            data: readme.as_bytes().to_vec(),
            version: None,
        },
        File {
            key: "fil_settings",
            component: "cmp_settings",
            name: "SETTIN~1.INI|settings.ini",
            data: b"[Tools]\r\nTheme=Light\r\n".to_vec(),
            version: None,
        },
        File {
            key: "fil_shared",
            component: "cmp_shared",
            name: "DOORS-~1.TXT|doors-shared.txt",
            data: b"Shared by Doors Tools and Doors Companion\r\n".to_vec(),
            version: None,
        },
    ];
    if let Some(s) = &payload.service {
        files.push(File {
            key: "fil_service",
            component: "cmp_service",
            name: "TOOLSV~1.EXE|toolsvc.exe",
            data: s.clone(),
            version: Some("1.0.0.0"),
        });
    }
    let mut components: Vec<[&str; 6]> = vec![
        [
            "cmp_tool",
            "{2C6B1A00-1111-4A2B-9C3D-4E5F60718293}",
            "INSTALLFOLDER",
            "#256",
            "",
            "fil_tool",
        ],
        [
            "cmp_readme",
            "{2C6B1A00-2222-4A2B-9C3D-4E5F60718293}",
            "INSTALLFOLDER",
            "#256",
            "",
            "fil_readme",
        ],
        [
            "cmp_settings",
            "{2C6B1A00-3333-4A2B-9C3D-4E5F60718293}",
            "INSTALLFOLDER",
            "#256",
            "",
            "fil_settings",
        ],
        [
            "cmp_shared",
            SHARED_COMPONENT,
            "SharedFolder",
            "#264",
            "",
            "fil_shared",
        ],
        [
            "cmp_registry",
            "{2C6B1A00-4444-4A2B-9C3D-4E5F60718293}",
            "INSTALLFOLDER",
            "#260",
            "",
            "reg_version",
        ],
        [
            "cmp_shortcut",
            "{2C6B1A00-5555-4A2B-9C3D-4E5F60718293}",
            "ProgramMenuFolder",
            "#260",
            "",
            "reg_shortcut",
        ],
    ];
    if payload.service.is_some() {
        components.push([
            "cmp_service",
            "{2C6B1A00-6666-4A2B-9C3D-4E5F60718293}",
            "INSTALLFOLDER",
            "#256",
            "",
            "fil_service",
        ]);
    }
    for c in &components {
        b.row_text("Component", c);
        b.row_text("FeatureComponents", &["Main", c[0]]);
    }
    b.row_text(
        "Feature",
        &[
            "Main",
            "",
            "Doors Tools",
            "The tools, their shortcut and file type.",
            "#1",
            "#1",
            "INSTALLFOLDER",
            "#0",
        ],
    );
    add_files(&mut b, &files);
    cabinet(&mut b, &files);
    for r in [
        [
            "reg_version",
            "#2",
            "Software\\Macrohard Test\\Doors Tools",
            "Version",
            "[ProductVersion]",
            "cmp_registry",
        ],
        [
            "reg_path",
            "#2",
            "Software\\Macrohard Test\\Doors Tools",
            "InstallPath",
            "[INSTALLFOLDER]",
            "cmp_registry",
        ],
        [
            "reg_runs",
            "#2",
            "Software\\Macrohard Test\\Doors Tools",
            "Runs",
            "#42",
            "cmp_registry",
        ],
        [
            "reg_key",
            "#2",
            "Software\\Macrohard Test\\Doors Tools",
            "-",
            "",
            "cmp_registry",
        ],
        [
            "reg_shortcut",
            "#2",
            "Software\\Macrohard Test\\Doors Tools",
            "StartMenu",
            "#1",
            "cmp_shortcut",
        ],
    ] {
        b.row_text("Registry", &r);
    }
    if payload.custom_action_dll.is_some() {
        // What the DLL custom action found, written later in the same
        // installation.
        b.row_text(
            "Registry",
            &[
                "reg_cacheck",
                "#2",
                "Software\\Macrohard Test\\Doors Tools",
                "CaCheck",
                "[DOORS_CA_CHECK]",
                "cmp_registry",
            ],
        );
    }
    b.row(
        "Shortcut",
        vec![
            Value::Str(String::from("sc_tool")),
            Value::Str(String::from("ProgramMenuFolder")),
            Value::Str(String::from("DOORST~1|Doors Tools")),
            Value::Str(String::from("cmp_shortcut")),
            Value::Str(String::from("[#fil_tool]")),
            Value::Null,
            Value::Str(String::from("Run Doors Tools")),
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Int(1),
            Value::Str(String::from("INSTALLFOLDER")),
        ],
    );
    // The .doorsnote file type opens with toolprobe.exe.
    b.row_text(
        "Extension",
        &[
            "doorsnote",
            "cmp_tool",
            "DoorsTools.Note",
            "text/plain",
            "Main",
        ],
    );
    b.row_text(
        "ProgId",
        &["DoorsTools.Note", "", "", "Doors Tools note", "", "#0"],
    );
    b.row_text("Verb", &["doorsnote", "open", "#1", "&Open", "open \"%1\""]);
    if payload.service.is_some() {
        b.row_text(
            "ServiceInstall",
            &[
                "svc_tools",
                "DoorsToolsSvc",
                "Doors Tools Service",
                "#16",
                "#3",
                "#1",
                "",
                "",
                "LocalSystem",
                "",
                "",
                "cmp_service",
                "Keeps the Doors Tools state",
            ],
        );
        b.row_text(
            "ServiceControl",
            &["sc_tools", "DoorsToolsSvc", "#163", "", "#1", "cmp_service"],
        );
    }
    // A user's log file in the install folder goes on removal (the
    // package says so); anything else the user put there stays.
    b.row_text(
        "RemoveFile",
        &["rf_log", "cmp_tool", "*.log", "INSTALLFOLDER", "#2"],
    );
    // Upgrades: older versions are removed, a newer one blocks this one.
    b.row_text(
        "Upgrade",
        &[
            TOOLS_UPGRADE,
            "",
            text_version,
            "",
            "#1",
            "",
            "WIX_UPGRADE_DETECTED",
        ],
    );
    b.row_text(
        "Upgrade",
        &[
            TOOLS_UPGRADE,
            text_version,
            "",
            "",
            "#2",
            "",
            "WIX_DOWNGRADE_DETECTED",
        ],
    );
    b.row_text(
        "LaunchCondition",
        &[
            "VersionNT64",
            "Doors Tools requires a 64-bit edition of Windows.",
        ],
    );
    // Custom actions.
    let marker = "\"[CommonAppDataFolder]Doors Tools Marker.txt\"";
    for row in [
        [
            "DowngradeError",
            "#19",
            "",
            "A newer version of [ProductName] is already installed.",
        ],
        ["SetMarkerData", "#51", "RecordInstall", marker],
        [
            "SetMarkerRollbackData",
            "#51",
            "RecordInstallRollback",
            marker,
        ],
        // Deferred EXE from the installed program: writes a marker file
        // outside the script (nontransactional); its rollback action
        // removes it.
        [
            "RecordInstallRollback",
            "#1298",
            "fil_tool",
            "unmark [CustomActionData]",
        ],
        [
            "RecordInstall",
            "#3090",
            "fil_tool",
            "mark [CustomActionData]",
        ],
        // INSTALL-A's interrupted installation: the program sleeps until
        // the machine is reset.
        ["HangInstall", "#3090", "fil_tool", "sleep 600"],
        [
            "FailInstall",
            "#19",
            "",
            "Doors Tools stopped the installation as asked (FAIL_AT=[FAIL_AT]).",
        ],
    ] {
        b.row_text("CustomAction", &row);
    }
    let mut execute_extra: Vec<(&str, &str, i32)> = vec![
        ("DowngradeError", "WIX_DOWNGRADE_DETECTED", 30),
        ("SetMarkerData", "NOT Installed", 1510),
        ("SetMarkerRollbackData", "NOT Installed", 1511),
        ("RecordInstallRollback", "NOT Installed", 4010),
        ("RecordInstall", "NOT Installed", 4020),
        ("HangInstall", "HANG=1", 4030),
        ("FailInstall", "FAIL_AT=\"files\"", 4040),
        ("ScheduleReboot", "NEEDREBOOT=1", 6500),
    ];
    if let Some(dll) = &payload.custom_action_dll {
        b.stream("Binary.CaProbe", dll.clone());
        b.row(
            "Binary",
            vec![
                Value::Str(String::from("CaProbe")),
                Value::Stream(String::from("Binary.CaProbe")),
            ],
        );
        b.row_text(
            "CustomAction",
            &["CheckMachine", "#1", "CaProbe", "CheckMachine"],
        );
        execute_extra.push(("CheckMachine", "NOT Installed", 1010));
    }
    execute_sequence(&mut b, &execute_extra);
    ui_sequence_base(
        &mut b,
        &[
            ("DowngradeError", "WIX_DOWNGRADE_DETECTED", 30),
            ("WelcomeEulaDlg", "NOT Installed", 1297),
            (
                "MaintenanceTypeDlg",
                "Installed AND NOT UPGRADINGPRODUCTCODE",
                1298,
            ),
            ("ProgressDlg", "", 1299),
            ("ExitDialog", "", -1),
            ("UserExit", "", -2),
            ("FatalError", "", -3),
        ],
    );
    ui(&mut b);
    summary(&mut b, "Doors Tools", package);
    b.build()
}

/// Doors Companion: one more file and the component Doors Tools shares.
pub fn companion() -> Vec<u8> {
    let mut b = Builder::new();
    declare(&mut b);
    for (k, v) in [
        ("ProductName", "Doors Companion"),
        ("ProductVersion", "1.0.0"),
        ("Manufacturer", "Macrohard Test"),
        ("ProductCode", COMPANION),
        ("UpgradeCode", COMPANION_UPGRADE),
        ("ProductLanguage", "1033"),
        ("ALLUSERS", "1"),
    ] {
        b.row_text("Property", &[k, v]);
    }
    for row in [
        ["TARGETDIR", "", "SourceDir"],
        ["ProgramFiles64Folder", "TARGETDIR", "."],
        [
            "INSTALLFOLDER",
            "ProgramFiles64Folder",
            "DOORSC~1|Doors Companion",
        ],
        ["CommonFiles64Folder", "TARGETDIR", "."],
        ["SharedFolder", "CommonFiles64Folder", "Doors"],
    ] {
        b.row_text("Directory", &row);
    }
    let files = vec![
        File {
            key: "fil_companion",
            component: "cmp_companion",
            name: "COMPAN~1.TXT|companion.txt",
            data: b"Doors Companion\r\n".to_vec(),
            version: None,
        },
        File {
            key: "fil_shared",
            component: "cmp_shared",
            name: "DOORS-~1.TXT|doors-shared.txt",
            data: b"Shared by Doors Tools and Doors Companion\r\n".to_vec(),
            version: None,
        },
    ];
    for c in [
        [
            "cmp_companion",
            "{4D2E8F10-1111-4B3C-8D4E-5F60718293A4}",
            "INSTALLFOLDER",
            "#256",
            "",
            "fil_companion",
        ],
        [
            "cmp_shared",
            SHARED_COMPONENT,
            "SharedFolder",
            "#264",
            "",
            "fil_shared",
        ],
    ] {
        b.row_text("Component", &c);
        b.row_text("FeatureComponents", &["Main", c[0]]);
    }
    b.row_text(
        "Feature",
        &[
            "Main",
            "",
            "Doors Companion",
            "",
            "#1",
            "#1",
            "INSTALLFOLDER",
            "#0",
        ],
    );
    add_files(&mut b, &files);
    cabinet(&mut b, &files);
    execute_sequence(&mut b, &[]);
    ui_sequence_base(&mut b, &[]);
    summary(
        &mut b,
        "Doors Companion",
        "{7B0E4D22-1001-4C1A-B8F3-5D6E7A8B9C11}",
    );
    b.build()
}

/// A package needing more space than any test volume has (its file table
/// says so): costing reports OutOfDiskSpace before anything is copied.
pub fn oversized() -> Vec<u8> {
    let mut b = Builder::new();
    declare(&mut b);
    for (k, v) in [
        ("ProductName", "Doors Oversized"),
        ("ProductVersion", "1.0.0"),
        ("Manufacturer", "Macrohard Test"),
        ("ProductCode", "{6A3F9B28-0001-4E4D-9F70-3D0E2C1B8F55}"),
        ("UpgradeCode", "{6A3F9B28-8E2B-4E4D-9F70-3D0E2C1B8F55}"),
        ("ProductLanguage", "1033"),
        ("ALLUSERS", "1"),
    ] {
        b.row_text("Property", &[k, v]);
    }
    for row in [
        ["TARGETDIR", "", "SourceDir"],
        ["ProgramFiles64Folder", "TARGETDIR", "."],
        [
            "INSTALLFOLDER",
            "ProgramFiles64Folder",
            "DOORSO~1|Doors Oversized",
        ],
    ] {
        b.row_text("Directory", &row);
    }
    let files = vec![File {
        key: "fil_big",
        component: "cmp_big",
        name: "big.bin",
        data: b"stand-in for a 1.9 GB file".to_vec(),
        version: None,
    }];
    b.row_text(
        "Component",
        &[
            "cmp_big",
            "{6A3F9B28-1111-4E4D-9F70-3D0E2C1B8F55}",
            "INSTALLFOLDER",
            "#256",
            "",
            "fil_big",
        ],
    );
    b.row_text("FeatureComponents", &["Main", "cmp_big"]);
    b.row_text(
        "Feature",
        &[
            "Main",
            "",
            "Doors Oversized",
            "",
            "#1",
            "#1",
            "INSTALLFOLDER",
            "#0",
        ],
    );
    // The file table claims 1.9 GB.
    b.row(
        "File",
        vec![
            Value::Str(String::from("fil_big")),
            Value::Str(String::from("cmp_big")),
            Value::Str(String::from("big.bin")),
            Value::Int(1_900_000_000),
            Value::Null,
            Value::Null,
            Value::Int(0x4000),
            Value::Int(1),
        ],
    );
    cabinet(&mut b, &files);
    execute_sequence(&mut b, &[]);
    ui_sequence_base(&mut b, &[]);
    summary(
        &mut b,
        "Doors Oversized",
        "{7B0E4D22-2001-4C1A-B8F3-5D6E7A8B9C21}",
    );
    b.build()
}

pub const REGISTRATIONS: &str = "{D0025A00-0000-4C11-8A00-5E7A1B2C3D01}";
/// Doors Registrations' COM class, AppId and published category.
pub const REG_CLSID: &str = "{D0025A00-0001-4C11-8A00-5E7A1B2C3D01}";
pub const REG_APPID: &str = "{D0025A00-0002-4C11-8A00-5E7A1B2C3D01}";
pub const REG_CATEGORY: &str = "{D0025A00-0003-4C11-8A00-5E7A1B2C3D01}";
/// The title in Doors Registrations' font file.
pub const REG_FONT: &str = "Doors Registration Sans";

/// A TrueType file's bytes as far as a title goes: the sfnt header and a
/// `name` table with the Windows English full name.
pub fn font_file(full_name: &str) -> Vec<u8> {
    let full: Vec<u8> = full_name
        .encode_utf16()
        .flat_map(u16::to_be_bytes)
        .collect();
    let mut name = Vec::new();
    for v in [0u16, 1, 18, 3, 1, 0x409, 4, full.len() as u16, 0] {
        name.extend(v.to_be_bytes());
    }
    name.extend(&full);
    let mut font = Vec::new();
    font.extend(0x0001_0000u32.to_be_bytes());
    font.extend(1u16.to_be_bytes());
    font.extend([0u8; 6]);
    font.extend(b"name");
    font.extend(0u32.to_be_bytes());
    font.extend(28u32.to_be_bytes());
    font.extend((name.len() as u32).to_be_bytes());
    font.extend(&name);
    font
}

/// Doors Registrations: a 32-bit (Intel) per-machine package with a COM
/// local server (Class, ProgId with its version-independent child,
/// AppId), a MIME type, a font with no title in the Font table, system
/// environment variables (one set, one appended to Path), INI lines (a
/// value and a tag), a duplicated file and a published (qualified)
/// component.
pub fn registrations() -> Vec<u8> {
    use kind::*;
    let mut b = Builder::new();
    declare(&mut b);
    b.table(
        "Class",
        &[
            ("CLSID", key(string(38))),
            ("Context", key(string(32))),
            ("Component_", key(string(72))),
            ("ProgId_Default", null(string(255))),
            ("Description", null(text(255))),
            ("AppId_", null(string(38))),
            ("FileTypeMask", null(string(255))),
            ("Icon_", null(string(72))),
            ("IconIndex", null(I2)),
            ("DefInprocHandler", null(string(32))),
            ("Argument", null(string(255))),
            ("Feature_", string(38)),
            ("Attributes", null(I2)),
        ],
    );
    b.table(
        "AppId",
        &[
            ("AppId", key(string(38))),
            ("RemoteServerName", null(string(255))),
            ("LocalService", null(string(255))),
            ("ServiceParameters", null(string(255))),
            ("DllSurrogate", null(string(255))),
            ("ActivateAtStorage", null(I2)),
            ("RunAsInteractiveUser", null(I2)),
        ],
    );
    b.table(
        "MIME",
        &[
            ("ContentType", key(string(64))),
            ("Extension_", string(255)),
            ("CLSID", null(string(38))),
        ],
    );
    b.table(
        "Font",
        &[("File_", key(string(72))), ("FontTitle", null(string(128)))],
    );
    b.table(
        "Environment",
        &[
            ("Environment", key(string(72))),
            ("Name", text(255)),
            ("Value", null(text(255))),
            ("Component_", string(72)),
        ],
    );
    b.table(
        "IniFile",
        &[
            ("IniFile", key(string(72))),
            ("FileName", text(255)),
            ("DirProperty", null(string(72))),
            ("Section", text(96)),
            ("Key", text(128)),
            ("Value", text(255)),
            ("Action", I2),
            ("Component_", string(72)),
        ],
    );
    b.table(
        "DuplicateFile",
        &[
            ("FileKey", key(string(72))),
            ("Component_", string(72)),
            ("File_", string(72)),
            ("DestName", null(text(255))),
            ("DestFolder", null(string(72))),
        ],
    );
    b.table(
        "PublishComponent",
        &[
            ("ComponentId", key(string(38))),
            ("Qualifier", key(text(255))),
            ("Component_", key(string(72))),
            ("AppData", null(text(255))),
            ("Feature_", string(38)),
        ],
    );
    for (k, v) in [
        ("ProductName", "Doors Registrations"),
        ("ProductVersion", "1.0.0"),
        ("Manufacturer", "Macrohard Test"),
        ("ProductCode", REGISTRATIONS),
        ("UpgradeCode", "{D0025A00-8E2B-4C11-8A00-5E7A1B2C3D01}"),
        ("ProductLanguage", "1033"),
        ("ALLUSERS", "1"),
    ] {
        b.row_text("Property", &[k, v]);
    }
    for row in [
        ["TARGETDIR", "", "SourceDir"],
        ["ProgramFilesFolder", "TARGETDIR", "."],
        [
            "INSTALLFOLDER",
            "ProgramFilesFolder",
            "DOORSR~1|Doors Registrations",
        ],
        ["DataFolder", "INSTALLFOLDER", "Data"],
        ["FontsFolder", "TARGETDIR", "."],
    ] {
        b.row_text("Directory", &row);
    }
    let files = vec![
        File {
            key: "fil_server",
            component: "cmp_server",
            name: "server.exe",
            data: b"MZ stand-in for the Doors Registrations server".to_vec(),
            version: None,
        },
        File {
            key: "fil_font",
            component: "cmp_font",
            name: "DOORSR~1.TTF|doorsreg.ttf",
            data: font_file(REG_FONT),
            version: None,
        },
        File {
            key: "fil_readme",
            component: "cmp_config",
            name: "readme.txt",
            data: b"Doors Registrations\r\n".to_vec(),
            version: None,
        },
    ];
    // 32-bit components (no msidbComponentAttributes64bit).
    for c in [
        [
            "cmp_server",
            "{D0025A00-1001-4C11-8A00-5E7A1B2C3D01}",
            "INSTALLFOLDER",
            "#0",
            "",
            "fil_server",
        ],
        [
            "cmp_font",
            "{D0025A00-1002-4C11-8A00-5E7A1B2C3D01}",
            "FontsFolder",
            "#0",
            "",
            "fil_font",
        ],
        [
            "cmp_config",
            "{D0025A00-1003-4C11-8A00-5E7A1B2C3D01}",
            "INSTALLFOLDER",
            "#0",
            "",
            "fil_readme",
        ],
    ] {
        b.row_text("Component", &c);
        b.row_text("FeatureComponents", &["Main", c[0]]);
    }
    b.row_text(
        "Feature",
        &[
            "Main",
            "",
            "Doors Registrations",
            "",
            "#1",
            "#1",
            "INSTALLFOLDER",
            "#0",
        ],
    );
    let n = Value::Null;
    let s = |v: &str| Value::Str(String::from(v));
    b.row(
        "Class",
        vec![
            s(REG_CLSID),
            s("LocalServer32"),
            s("cmp_server"),
            s("Doors.Server.1"),
            s("Doors Registrations Server"),
            s(REG_APPID),
            n.clone(),
            n.clone(),
            n.clone(),
            n.clone(),
            s("/automation"),
            s("Main"),
            n.clone(),
        ],
    );
    b.row(
        "ProgId",
        vec![
            s("Doors.Server.1"),
            n.clone(),
            s(REG_CLSID),
            s("Doors Registrations Server 1"),
            n.clone(),
            n.clone(),
        ],
    );
    b.row(
        "ProgId",
        vec![
            s("Doors.Server"),
            s("Doors.Server.1"),
            n.clone(),
            s("Doors Registrations Server"),
            n.clone(),
            n.clone(),
        ],
    );
    b.row(
        "AppId",
        vec![
            s(REG_APPID),
            n.clone(),
            n.clone(),
            n.clone(),
            n.clone(),
            Value::Int(1),
            n.clone(),
        ],
    );
    b.row(
        "Extension",
        vec![
            s("doorsreg"),
            s("cmp_server"),
            n.clone(),
            s("application/x-doors-reg"),
            s("Main"),
        ],
    );
    b.row(
        "MIME",
        vec![s("application/x-doors-reg"), s("doorsreg"), s(REG_CLSID)],
    );
    b.row("Font", vec![s("fil_font"), n.clone()]);
    b.row_text(
        "Environment",
        &[
            "env_home",
            "=-*DOORSREG_HOME",
            "[INSTALLFOLDER]",
            "cmp_config",
        ],
    );
    b.row_text(
        "Environment",
        &["env_path", "=-*Path", "[~];[INSTALLFOLDER]", "cmp_config"],
    );
    for (key, name, value, action) in [
        ("ini_home", "Home", "[INSTALLFOLDER]", 0),
        ("ini_plugin", "Plugins", "core", 3),
    ] {
        b.row(
            "IniFile",
            vec![
                s(key),
                s("doorsreg.ini"),
                s("INSTALLFOLDER"),
                s("Settings"),
                s(name),
                s(value),
                Value::Int(action),
                s("cmp_config"),
            ],
        );
    }
    b.row(
        "DuplicateFile",
        vec![
            s("dup_readme"),
            s("cmp_config"),
            s("fil_readme"),
            s("README~1.TXT|readme-copy.txt"),
            s("DataFolder"),
        ],
    );
    b.row(
        "PublishComponent",
        vec![
            s(REG_CATEGORY),
            s("en-us"),
            s("cmp_server"),
            s("Doors data"),
            s("Main"),
        ],
    );
    add_files(&mut b, &files);
    cabinet(&mut b, &files);
    execute_sequence(&mut b, &[]);
    ui_sequence_base(&mut b, &[]);
    summary_for(
        &mut b,
        "Doors Registrations",
        "{D0025A00-1000-4C11-8A00-5E7A1B2C3D01}",
        "Intel;1033",
    );
    b.build()
}

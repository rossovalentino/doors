extern crate std;
use super::*;
use alloc::string::ToString;
use install::{Cabinet, Machine, RegValue, Root, Source};

// Built by Windows Installer's own database engine (WindowsInstaller.Installer
// OpenDatabase/OpenView/SummaryInformation on Windows 11): a product with
// two components, a cabinet stream, a Binary row and the summary.
const SAMPLE: &[u8] = include_bytes!("../testdata/sample.msi");

#[test]
fn stream_names_round_trip() {
    for (name, table) in [
        ("_StringPool", true),
        ("Property", true),
        ("product.cab", false),
        ("Binary.readme.bin", false),
    ] {
        let encoded = encode_name(name, table);
        assert_eq!(decode_name(&encoded), (name.to_string(), table));
    }
    // Pairs pack into one unit.
    assert_eq!(encode_name("ab", false), [0x3800 + 36 + (37 << 6)]);
    assert_eq!(encode_name("a", true), [TABLE_PREFIX, 0x4800 + 36]);
}

#[test]
fn reads_the_windows_built_database() {
    let db = Database::open(SAMPLE).unwrap();
    let names: Vec<&str> = db.tables.iter().map(|t| t.0.as_str()).collect();
    for t in [
        "Property",
        "Directory",
        "Component",
        "File",
        "Media",
        "Feature",
        "FeatureComponents",
        "Registry",
        "Shortcut",
        "Binary",
    ] {
        assert!(names.contains(&t), "{t} missing from {names:?}");
    }
    assert_eq!(db.property("ProductName").as_deref(), Some("Doors Sample"));
    assert_eq!(
        db.property("ProductCode").as_deref(),
        Some("{6F1E2C3B-8A7D-4E5F-9A0B-1C2D3E4F5A6B}")
    );
    let file = db.table("File").unwrap();
    assert_eq!(file.rows.len(), 2);
    let app = file.find("fil_app").unwrap();
    assert_eq!(file.get(app, "FileSize").int(), Some(50000));
    assert_eq!(file.get(app, "Sequence").int(), Some(2));
    assert_eq!(file.get(app, "FileName").str(), Some("APP~1.BIN|app.bin"));
    assert!(file.get(app, "Version").is_null());
    // Keys are flagged; a nullable string column too.
    assert!(file.columns[0].is_key() && !file.columns[1].is_key());
    let registry = db.table("Registry").unwrap();
    let count = registry.find("reg_count").unwrap();
    assert_eq!(registry.get(count, "Root").int(), Some(2));
    // A Binary row names its stream after the table and key.
    let binary = db.table("Binary").unwrap();
    assert!(binary.columns[1].is_binary());
    assert_eq!(
        binary.get(0, "Data"),
        &Value::Stream("Binary.readme.bin".to_string())
    );
    assert_eq!(
        db.stream("Binary.readme.bin").unwrap(),
        b"Doors sample product\r\n"
    );
    // The cabinet stream.
    let cab = db.stream("product.cab").unwrap();
    assert_eq!(&cab[..4], b"MSCF");
    let streams = db.streams();
    assert!(streams.contains(&("product.cab".to_string(), false)));
    assert!(streams.contains(&("File".to_string(), true)));
    // Summary information.
    let s = db.summary().unwrap();
    assert_eq!(s.text(summary::pid::SUBJECT), Some("Doors Sample"));
    assert_eq!(s.text(summary::pid::AUTHOR), Some("Macrohard Test"));
    assert_eq!(s.platform(), Some("x64"));
    assert_eq!(
        s.text(summary::pid::REVISION),
        Some("{ABCDEF01-2345-6789-ABCD-EF0123456789}")
    );
    assert_eq!(s.int(summary::pid::PAGE_COUNT), Some(500));
    assert!(s.compressed());
}

#[test]
fn plans_the_installation() {
    let db = Database::open(SAMPLE).unwrap();
    let machine = Machine::doors("C:\\Users\\Ada\\", "Ada", "DOORS-PC", true);
    let plan = install::plan(&db, &machine, &[]).unwrap();
    assert_eq!(plan.product_name, "Doors Sample");
    assert_eq!(plan.version, "1.2.3");
    assert!(plan.all_users);
    assert_eq!(
        plan.directory("INSTALLFOLDER"),
        Some("C:\\Program Files\\Doors Sample\\")
    );
    assert_eq!(
        plan.directory("DataFolder"),
        Some("C:\\Program Files\\Doors Sample\\data\\")
    );
    assert_eq!(plan.files.len(), 2);
    let readme = plan.files.iter().find(|f| f.key == "fil_readme").unwrap();
    assert_eq!(readme.target, "C:\\Program Files\\Doors Sample\\readme.txt");
    assert_eq!(
        readme.source,
        Source::Cabinet {
            cabinet: Cabinet::Embedded("product.cab".to_string()),
            name: "fil_readme".to_string()
        }
    );
    let app = plan.files.iter().find(|f| f.key == "fil_app").unwrap();
    assert_eq!(app.target, "C:\\Program Files\\Doors Sample\\data\\app.bin");
    // Registry: a formatted string and a DWORD under HKLM.
    let path = plan
        .registry
        .iter()
        .find(|r| r.name.as_deref() == Some("InstallPath"))
        .unwrap();
    assert_eq!(path.root, Root::LocalMachine);
    assert_eq!(path.key, "Software\\Macrohard Test\\Doors Sample");
    assert_eq!(
        path.value,
        Some(RegValue::String(
            "C:\\Program Files\\Doors Sample\\".to_string()
        ))
    );
    let runs = plan
        .registry
        .iter()
        .find(|r| r.name.as_deref() == Some("Runs"))
        .unwrap();
    assert_eq!(runs.value, Some(RegValue::Dword(42)));
    // The Start menu shortcut to the readme.
    assert_eq!(plan.shortcuts.len(), 1);
    let s = &plan.shortcuts[0];
    assert_eq!(
        s.path,
        "C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\Doors Sample readme.lnk"
    );
    assert_eq!(s.target, readme.target);
    assert_eq!(s.description, "Read me");
    // Add/Remove Programs.
    assert_eq!(
        plan.uninstall_key,
        "HKEY_LOCAL_MACHINE\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{6F1E2C3B-8A7D-4E5F-9A0B-1C2D3E4F5A6B}"
    );
    let value = |n: &str| {
        plan.uninstall
            .iter()
            .find(|v| v.0 == n)
            .map(|v| v.1.clone())
    };
    assert_eq!(
        value("DisplayName"),
        Some(RegValue::String("Doors Sample".to_string()))
    );
    assert_eq!(
        value("Publisher"),
        Some(RegValue::String("Macrohard Test".to_string()))
    );
    assert_eq!(value("EstimatedSize"), Some(RegValue::Dword(49)));
    assert_eq!(value("Version"), Some(RegValue::Dword(0x0102_0003)));
    assert_eq!(
        value("UninstallString"),
        Some(RegValue::Expand(
            "MsiExec.exe /X{6F1E2C3B-8A7D-4E5F-9A0B-1C2D3E4F5A6B}".to_string()
        ))
    );
    // The command line moves the install folder.
    let moved = install::plan(
        &db,
        &machine,
        &[("INSTALLFOLDER".to_string(), "D:\\Apps\\Sample".to_string())],
    )
    .unwrap();
    assert_eq!(
        moved.directory("DataFolder"),
        Some("D:\\Apps\\Sample\\data\\")
    );
    // Per-user: HKCU and the user's Start menu.
    let user = install::plan(
        &db,
        &Machine::doors("C:\\Users\\Ada\\", "Ada", "DOORS-PC", false),
        &[("ALLUSERS".to_string(), String::new())],
    )
    .unwrap();
    assert!(user.uninstall_key.starts_with("HKEY_CURRENT_USER\\"));
    assert!(user.shortcuts[0].path.starts_with(
        "C:\\Users\\Ada\\AppData\\Roaming\\Microsoft\\Windows\\Start Menu\\Programs\\"
    ));
}

#[test]
fn conditions_follow_windows_installer() {
    use install::condition::evaluate;
    let p = |list: &[(&str, &str)]| -> Vec<(String, String)> {
        list.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    let props = p(&[("VersionNT64", "603"), ("ALLUSERS", "1"), ("Name", "Doors")]);
    assert!(evaluate("", &props));
    assert!(evaluate("VersionNT64", &props));
    assert!(!evaluate("Installed", &props));
    assert!(evaluate("NOT Installed", &props));
    assert!(evaluate("VersionNT64 >= 601 AND ALLUSERS = 1", &props));
    assert!(evaluate("Name = \"Doors\"", &props));
    assert!(evaluate("Name ~= \"doors\"", &props));
    assert!(!evaluate("Name = \"doors\"", &props));
    assert!(evaluate(
        "(Installed OR VersionNT64 < 700) AND NOT Missing",
        &props
    ));
    assert!(evaluate("Installed IMP Missing", &props));
    assert!(!evaluate("VersionNT64 <", &props));
    assert!(!evaluate("\"unterminated", &props));
}

#[test]
fn damaged_databases_are_refused() {
    assert!(Database::open(b"not a compound file").is_err());
    assert!(Database::open(&SAMPLE[..1024]).is_err());
    // Corrupt bytes across the file: errors, never a panic.
    for at in (0..SAMPLE.len()).step_by(97) {
        let mut damaged = SAMPLE.to_vec();
        damaged[at] ^= 0x5a;
        if let Ok(db) = Database::open(&damaged) {
            for t in db.tables.iter().map(|t| t.0.clone()).collect::<Vec<_>>() {
                let _ = db.table(&t);
            }
            let _ = db.summary();
        }
    }
}
